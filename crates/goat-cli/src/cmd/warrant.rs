// `goat warrant why-blocked` — ADR 1129 C2.
//
// Most reports of "I can't reach X" are not ACL gaps. They are DNS, a
// membership the peer does not hold, or a service that is not listening. This
// returns that verdict in seconds without creating anything, and when it IS an
// ACL gap it prints the minimal port-set that closes it and the diff to put in
// a PR.
//
// It exists because of a specific failure: two ACL findings were
// manufactured by string-matching group prefixes in a one-off script, and the
// proposed remediation would have stripped the one correct group off two
// machines. The method was the defect — a hand-rolled read of a declaration
// nobody had checked. So this answers the question with the SAME evaluator the
// assertions and netvis use (`panoscope_core::aclcheck`), and an operator
// never has to write that script.
//
// WHAT IT READS, AND WHAT IT DOES NOT. This is the DECLARED plane — the YAML
// in git, compiled into the binary. It needs no network, no context and no
// credentials, because a check that requires a working profile is a check
// people skip. It therefore cannot tell you whether the peer actually holds
// the group, or whether anything is listening on the port. It says so, every
// time, rather than implying a completeness it does not have.

use clap::{Args, Subcommand};
use panoscope_core::aclcheck::{Model, Permit};
use panoscope_core::portsets::{PortSet, Vocabulary};

/// The vocabulary, so a blocked verdict can propose a NAMED set rather than
/// asking the operator for a port list. Asking for ports is how the vocabulary
/// re-fragments (docs/design/warrant.md §2).
const PORT_SETS_YAML: &str = include_str!("../../../../ops/warrant/port-sets.yaml");

#[derive(Subcommand, Debug)]
pub enum WarrantCmd {
    /// Why can't this source reach that destination on that port?
    WhyBlocked(WhyBlockedArgs),
    /// The port-set vocabulary: seven names, what each is for, who owns it.
    PortSets(PortSetsArgs),
    /// Converge a scope's live peer membership to its declaration in git
    /// (the directory `ops/warrant/membership/<scope>/`) — report by default.
    ///
    /// Reads peers, groups, policies, setup keys and the activity log from the
    /// NetBird management API and computes, per peer, the managed groups it
    /// should hold (docs/design/mesh-change-control.md §4.2). `--mode enforce`
    /// adds before it removes and re-reads to verify; the file's `converge:`
    /// is the ceiling. Token: $GOAT_NB_API_TOKEN or $<SCOPE>_NB_API_TOKEN.
    Apply(ApplyArgs),
    /// Hold a scope's membership converger in report mode for a while — the
    /// audited, expiring way to make an emergency membership edit
    /// (docs/design/mesh-change-control.md §2).
    ///
    /// Writes `<state-dir>/pause-<scope>.json`; run it on the scope's
    /// management host, as the user the converger runs as (root). At most
    /// 24h, a reason is required, and one `warrant.membership.pause`
    /// operator-action event is recorded. It ends on its own.
    Pause(PauseArgs),
    /// End a `goat warrant pause` early: removes the pause file and records
    /// one `warrant.membership.resume` operator-action event.
    Resume(ResumeArgs),
}

pub async fn run(ctx: &super::Ctx, cmd: WarrantCmd) -> anyhow::Result<()> {
    match cmd {
        WarrantCmd::WhyBlocked(a) => run_why_blocked(a),
        WarrantCmd::PortSets(a) => run_port_sets(a),
        WarrantCmd::Apply(a) => membership::run_apply(ctx, a).await,
        WarrantCmd::Pause(a) => membership::run_pause(ctx, a).await,
        WarrantCmd::Resume(a) => membership::run_resume(ctx, a).await,
    }
}

#[cfg(test)]
pub(crate) use membership::{email_hash, BaselineSource};
pub(crate) use membership::{
    entry_yaml, governance, key_joins, load_decl, membership_path, merge_people,
    ory_identity_quarantine_refusal, people_from_manifests, quarantine_refusal_with_owner_reason,
    read_ory_privileges, resolve_baselines, target_fragment, unproven_owner_refusal, write_entry,
    Baseline, EntryForm, KeyJoin, LivePeer, MembershipDecl, MembershipEntry, OryPrivilegeIndex,
    OwnerJoin, PersonBaseline, PEOPLE_FILE, RUNBOOK,
};
pub use membership::{ApplyArgs, PauseArgs, ResumeArgs, WarrantExit};
pub(crate) use membership::CONTEXT_CONVERGE_KEY;

#[path = "warrant_membership.rs"]
mod membership;

#[derive(Args, Debug)]
pub struct WhyBlockedArgs {
    /// Scope — a site (`zagros`, `zagros-prod`) or an exercise scope.
    #[arg(long)]
    pub scope: String,
    /// Source group. Bare suffix (`DEVELOPER`) or fully qualified.
    #[arg(long)]
    pub from: String,
    /// Destination group. Bare suffix (`INFRA`) or fully qualified.
    #[arg(long)]
    pub to: String,
    /// Destination port.
    #[arg(long)]
    pub port: String,
    /// `tcp` | `udp` | `icmp` | `any`.
    #[arg(long, default_value = "tcp")]
    pub proto: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct PortSetsArgs {
    #[arg(long)]
    pub json: bool,
}

// ---------------------------------------------------------------------------
// the vocabulary
// ---------------------------------------------------------------------------

fn port_sets() -> anyhow::Result<Vocabulary> {
    Ok(Vocabulary::parse(PORT_SETS_YAML)?)
}

// ---------------------------------------------------------------------------
// the verdict
// ---------------------------------------------------------------------------

fn qualify(model: &Model, name: &str) -> String {
    let upper = name.to_uppercase();
    if upper.starts_with(&format!("{}-", model.prefix)) {
        upper
    } else {
        format!("{}-{}", model.prefix, upper)
    }
}

fn run_why_blocked(args: WhyBlockedArgs) -> anyhow::Result<()> {
    // `{e:?}` (Debug) discarded DeclError's own hand-authored Display prose;
    // acl.rs's identical case just uses `?` (Display, via anyhow's blanket
    // conversion), which is what this should have done too.
    let decl = super::acl::declaration("warrant: reading the ACL declaration in this checkout")?;
    let model = decl.model(&args.scope)?;

    let from = qualify(&model, &args.from);
    let to = qualify(&model, &args.to);

    // A group the scope cannot name is a malformed question, not a blocked
    // path. Answering "blocked" to a typo is how a checker teaches people to
    // distrust it — and it is exactly how the findings were made.
    for (label, g) in [("--from", &from), ("--to", &to)] {
        if !model.groups.contains(g) {
            let near: Vec<&str> = model
                .groups
                .iter()
                .filter(|k| {
                    k.contains(args.from.to_uppercase().as_str())
                        || k.contains(args.to.to_uppercase().as_str())
                })
                .map(String::as_str)
                .collect();
            anyhow::bail!(
                "{label} {g} is not a group in scope '{}'.\n  \
                 This is a malformed question, not a blocked path.{}",
                args.scope,
                if near.is_empty() {
                    String::new()
                } else {
                    format!("\n  Did you mean: {}", near.join(", "))
                }
            );
        }
    }

    let between: Vec<&Permit> =
        model.permits.iter().filter(|p| p.src == from && p.dst == to).collect();
    let granting: Vec<&&Permit> = between
        .iter()
        .filter(|p| p.covers_proto(&args.proto) && p.covers_port(&args.port))
        .collect();

    if args.json {
        let out = serde_json::json!({
            "scope": args.scope,
            "from": from,
            "to": to,
            "protocol": args.proto,
            "port": args.port,
            "verdict": if granting.is_empty() { "blocked" } else { "permitted" },
            "granted_by": granting.iter().map(|p| p.rule.clone()).collect::<Vec<_>>(),
            "other_rules_between_the_pair":
                between.iter().map(|p| p.rule.clone()).collect::<Vec<_>>(),
            "plane": "declared",
            "not_consulted": ["live group membership", "listening surface", "DNS"],
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    println!("{from} -> {to}  {}/{}  [scope: {}]\n", args.proto, args.port, args.scope);

    if !granting.is_empty() {
        println!("PERMITTED by the declared ACL. Granted by:");
        for p in &granting {
            println!(
                "  - {}  ({} {})",
                p.rule,
                p.protocol,
                if p.ports.is_empty() { "any port".into() } else { p.ports.join(", ") }
            );
        }
        println!(
            "\nSo this is not an ACL gap. The remaining candidates, in the order\n\
             they are usually the answer:\n  \
             1. the peer does not hold {to} (check `goat who`)\n  \
             2. nothing is listening on {}/{} at the far end\n  \
             3. DNS — the name resolves somewhere else, or not at all",
            args.proto, args.port
        );
        return Ok(());
    }

    println!("BLOCKED by the declared ACL — no rule grants this pair that port.");
    if between.is_empty() {
        println!("\nNo rule connects {from} to {to} at all, on any port.");
    } else {
        println!("\nWhat DOES exist between the pair:");
        for p in &between {
            println!(
                "  - {}  ({} {})",
                p.rule,
                p.protocol,
                if p.ports.is_empty() { "any port".into() } else { p.ports.join(", ") }
            );
        }
    }

    match port_sets()?.minimal_for(&args.proto, &args.port).cloned() {
        Some(set) => {
            let arm = set
                .members
                .iter()
                .find(|m| m.protocol.eq_ignore_ascii_case(&args.proto))
                .expect("minimal_set only returns sets with a matching arm");
            println!(
                "\nThe minimal port-set that closes it: {}\n  \
                 {} {}\n  owner: {}\n  {}",
                set.name,
                arm.protocol,
                arm.ports.join(", "),
                set.owner,
                set.purpose.trim().replace('\n', "\n  ")
            );
            println!(
                "\nThe diff to put in a PR — ops/warrant/rule-intents/netbird-acl.yaml:\n\
                 \n      - name: \"{} → {} ({})\"\n        \
                 description: \"<why this pair needs this set — one sentence a reviewer can check>\"\n        \
                 sources: [{}]\n        destinations: [{}]\n        \
                 protocol: {}\n        port_set: {}\n        \
                 action: accept\n        bidirectional: false\n",
                args.from.to_lowercase(),
                args.to.to_lowercase(),
                set.name,
                args.from.to_uppercase(),
                args.to.to_uppercase(),
                arm.protocol,
                set.name,
            );
            println!(
                "Then: ops/warrant/generate.py && git commit. CODEOWNERS routes it to\n\
                 {} for review; CI runs the assertions on the proposal, before effect.\n\
                 If the grant is temporary, add `expires: YYYY-MM-DD` and an `owner:`\n\
                 and it recertifies itself by going red.",
                set.owner
            );
        }
        None => {
            println!(
                "\nNo port-set carries {}/{}. That is a vocabulary decision, not a\n\
                 rule change: adding a port-set affects every rule that would use it,\n\
                 so it goes to the platform admins on its own PR against\n\
                 ops/warrant/port-sets.yaml. `goat warrant port-sets` lists the seven.",
                args.proto, args.port
            );
        }
    }

    println!(
        "\n--\nThis read the DECLARED plane (the YAML in git). It did not consult live\n\
         group membership, the listening surface, or DNS — so it can tell you a path\n\
         is not permitted, and it cannot tell you a permitted path works."
    );
    Ok(())
}

fn run_port_sets(args: PortSetsArgs) -> anyhow::Result<()> {
    let vocab = port_sets()?;
    let sets: Vec<PortSet> = vocab.all().to_vec();
    if args.json {
        println!("{}", serde_json::to_string_pretty(&sets)?);
        return Ok(());
    }
    println!("The port-set vocabulary — ops/warrant/port-sets.yaml\n");
    for s in &sets {
        println!("{}  (owner: {})", s.name, s.owner);
        for m in &s.members {
            println!("  {:<5} {}", m.protocol, m.ports.join(", "));
        }
        println!("  {}\n", s.purpose.trim().replace('\n', "\n  "));
    }
    Ok(())
}
