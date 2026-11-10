// SPDX-License-Identifier: Apache-2.0
//
// `goat admin members` — the membership transfer engine (ADR 1088).
//
// One audited, coverage-gated primitive: move a set of entities from group A
// to group B — bulk, individual, or selected set; shed and absorb are the same
// operation with source/destination swapped. Callers: this CLI directly, the
// exercise stamp/absorb/teardown tooling (Stage 2), the probation dormancy
// cron (Stage 3, ADR 1089), and the quarantine lane (ADR 1091).
//
// Invariants (ADR 1088 — do not weaken):
//   - DRY-RUN DEFAULT: without --apply the command prints the plan and who
//     moves, touches nothing, and emits no audit event.
//   - MIRROR-FIRST: every mover is ADDED to the destination and the addition
//     is re-fetched + VERIFIED before anything is removed from the source. A
//     failure mid-run leaves peers dual-homed (safe), never uncovered.
//   - FAIL-CLOSED COVERAGE GATE: the destination group must be referenced by
//     at least one enabled policy rule, or the move is refused — parking peers
//     in a rule-less group would cut them off silently under the enforced
//     default-deny posture.
//   - IDEMPOTENT: a re-run after success (or a partial run) finds nothing to
//     do / only the remaining work. Entities already at the destination are
//     skipped.
//   - AUDITED: one OperatorActionV1 per applied run (action
//     `admin.members.move`), from the `goat admin` chokepoint.
//
// The quarantine entry recipe (ADR 1091) composes from the same primitive:
//   goat admin members move --from DEVELOPER --to QUARANTINED \
//     --handle <who> --strip-others --set-marker quarantine --apply

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use clap::Subcommand;
use panoscope_core::netbird::{NbGroup, NbPeer, NbPolicy};

#[derive(Subcommand, Debug)]
pub enum MembersCmd {
    /// Move members between mesh groups (shed / absorb) — dry-run by default.
    ///
    /// Selects the peers currently in --from (all of them, or the subset named
    /// by --handle / --list / --filter), mirror-adds them to --to, verifies,
    /// then removes them from --from. Group names are suffix-resolved against
    /// the target goatnet's prefix (`DEVELOPER` → `ZAGROS-DEVELOPER`); a full
    /// group name is accepted verbatim.
    Move {
        /// Source group (suffix or full name).
        #[arg(long)]
        from: String,
        /// Destination group (suffix or full name). Must be covered by at
        /// least one enabled policy (fail-closed) — stamp the matrix rule
        /// first if it isn't.
        #[arg(long)]
        to: String,
        /// Select a single member: peer id, mesh IP, or a name/hostname
        /// substring (same matching as `goat admin peer cut`). Also the
        /// handle used for the Ory-plane leg when --planes includes ory
        /// or a marker flag is set.
        #[arg(long)]
        handle: Option<String>,
        /// File with one selector per line (blank lines + `#` comments ok).
        #[arg(long)]
        list: Option<PathBuf>,
        /// Comma-joined predicates over the --from members:
        /// `last_seen>14d` (dormant longer than N days; never-seen counts as
        /// dormant), `connected=true|false`.
        #[arg(long)]
        filter: Option<String>,
        /// Which plane(s) to touch: `mesh` (default), `ory`, `both`. The ory
        /// plane swaps --from/--to VERBATIM inside the identity's Ory
        /// `traits.groups` — Ory vocabulary only; mesh role names never enter
        /// Ory traits.
        #[arg(long, default_value = "mesh")]
        planes: String,
        /// After the mirror-add verifies, also remove the movers from EVERY
        /// other group (except the auto `All` and the destination). The
        /// quarantine-entry posture (ADR 1091): strip roles, P2P, labels.
        #[arg(long)]
        strip_others: bool,
        /// Set a boolean Ory trait marker (e.g. `quarantine`, `probation`) on
        /// the selected identities. Requires the Kratos schema to carry the
        /// trait — fails loudly naming the schema if it doesn't.
        #[arg(long)]
        set_marker: Option<String>,
        /// Clear (remove) a boolean Ory trait marker from the selected
        /// identities.
        #[arg(long)]
        clear_marker: Option<String>,
        /// Execute the plan. Without this flag the run is a dry-run: plan
        /// printed, nothing touched, no audit emitted.
        #[arg(long)]
        apply: bool,
        /// Target goatnet (defaults to the active profile context, e.g.
        /// `zagros`). Site prefix for suffix resolution = uppercased env.
        #[arg(long)]
        env: Option<String>,
        /// Override the NetBird mgmt URL (else the profile's).
        #[arg(long)]
        mgmt_url: Option<String>,
        /// Emit the plan/result as JSON.
        #[arg(long)]
        json: bool,
        /// On a scope governed by warrant (its
        /// `ops/warrant/membership/<scope>/_scope.yaml` says `converge: enforce`), add the
        /// entry that expresses this move to the scope directory instead of changing the
        /// mesh: edited in place where the peer already has an entry, else into
        /// `holds.yaml` / `services.yaml` / `people.yaml` by kind. Commit it and open a PR;
        /// the converger applies it.
        #[arg(long)]
        write_entry: bool,
        /// `owner:` of the written/printed entry (default: your operator handle).
        #[arg(long)]
        owner: Option<String>,
        /// `reason:` of the written/printed entry (≤200 chars).
        #[arg(long)]
        reason: Option<String>,
        /// `expires:` (YYYY-MM-DD) of the written/printed entry. Default: 7
        /// days for a quarantine hold, 90 days for P2P / PLATFORM-ADMIN /
        /// BREAKGLASS / EXTERNAL-INFRA, none otherwise.
        #[arg(long)]
        expires: Option<String>,
    },
}

pub async fn run(ctx: &super::Ctx, op: MembersCmd) -> anyhow::Result<()> {
    match op {
        MembersCmd::Move {
            from,
            to,
            handle,
            list,
            filter,
            planes,
            strip_others,
            set_marker,
            clear_marker,
            apply,
            env,
            mgmt_url,
            json,
            write_entry,
            owner,
            reason,
            expires,
        } => {
            run_move(
                ctx,
                MoveArgs {
                    from,
                    to,
                    handle,
                    list,
                    filter,
                    planes,
                    strip_others,
                    set_marker,
                    clear_marker,
                    apply,
                    env,
                    mgmt_url,
                    json,
                    write_entry,
                    owner,
                    reason,
                    expires,
                },
            )
            .await
        }
    }
}

struct MoveArgs {
    from: String,
    to: String,
    handle: Option<String>,
    list: Option<PathBuf>,
    filter: Option<String>,
    planes: String,
    strip_others: bool,
    set_marker: Option<String>,
    clear_marker: Option<String>,
    apply: bool,
    env: Option<String>,
    mgmt_url: Option<String>,
    json: bool,
    write_entry: bool,
    owner: Option<String>,
    reason: Option<String>,
    expires: Option<String>,
}

fn quarantine_transitions(
    from_group: &str,
    to_group: &str,
    prefix: &str,
    ory_plane: bool,
    set_marker: Option<&str>,
    clear_marker: Option<&str>,
) -> (bool, bool) {
    let quarantine = format!("{prefix}-QUARANTINED");
    let mesh = from_group == quarantine || to_group == quarantine;
    let marker = set_marker
        .into_iter()
        .chain(clear_marker)
        .any(|value| value.eq_ignore_ascii_case("quarantine"));
    (mesh, (ory_plane && mesh) || marker)
}

fn resolve_move_ory_config(
    env: &str,
    profile: &crate::profile::Profile,
    required: bool,
) -> anyhow::Result<Option<super::admin::OryEnvConfig>> {
    let cfg = super::admin::native_ory_config_for(Some(env), profile);
    if required && cfg.is_none() {
        anyhow::bail!(
            "no Ory facts for goatnet `{env}` — run `goat context refresh` in that goatnet's context"
        );
    }
    Ok(cfg)
}

fn move_people_and_baselines(
    decl: Option<&super::warrant::MembershipDecl>,
    manifest_people: BTreeMap<String, super::warrant::PersonBaseline>,
    env: &str,
    live_peers: &[super::warrant::LivePeer],
    joins: &BTreeMap<String, super::warrant::KeyJoin>,
) -> (
    BTreeMap<String, super::warrant::PersonBaseline>,
    BTreeMap<String, super::warrant::Baseline>,
) {
    let empty = BTreeMap::new();
    let baseline_people = decl.map(|d| &d.people).unwrap_or(&empty);
    let baselines = super::warrant::resolve_baselines(env, baseline_people, live_peers, joins);
    let people = super::warrant::merge_people(baseline_people.clone(), manifest_people);
    (people, baselines)
}

// ---------------------------------------------------------------------------
// Pure planning core — unit-tested without a mgmt server.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
enum Pred {
    /// `last_seen>Nd` — dormant longer than N days (never-seen ⇒ dormant).
    LastSeenOlderDays(i64),
    /// `connected=true|false`.
    Connected(bool),
}

fn parse_filter(s: &str) -> anyhow::Result<Vec<Pred>> {
    let mut preds = Vec::new();
    for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        if let Some(rest) = part.strip_prefix("last_seen>") {
            let days: i64 = rest
                .trim_end_matches(['d', 'D'])
                .parse()
                .map_err(|_| anyhow::anyhow!("bad filter `{part}` — want e.g. last_seen>14d"))?;
            preds.push(Pred::LastSeenOlderDays(days));
        } else if let Some(rest) = part.strip_prefix("connected=") {
            match rest {
                "true" => preds.push(Pred::Connected(true)),
                "false" => preds.push(Pred::Connected(false)),
                _ => anyhow::bail!("bad filter `{part}` — want connected=true|false"),
            }
        } else {
            anyhow::bail!(
                "unknown filter predicate `{part}` — supported: last_seen>Nd, connected=true|false"
            );
        }
    }
    Ok(preds)
}

fn peer_passes(p: &NbPeer, preds: &[Pred], now: chrono::DateTime<chrono::Utc>) -> bool {
    preds.iter().all(|pred| match *pred {
        Pred::Connected(want) => p.connected == want,
        Pred::LastSeenOlderDays(days) => {
            // A currently-connected peer is active regardless of the recorded
            // last_seen timestamp (NetBird updates it on disconnect).
            if p.connected {
                return false;
            }
            match chrono::DateTime::parse_from_rfc3339(&p.last_seen) {
                Ok(ts) => now.signed_duration_since(ts.with_timezone(&chrono::Utc))
                    > chrono::Duration::days(days),
                // Unparseable / absent last_seen: never seen ⇒ dormant.
                Err(_) => true,
            }
        }
    })
}

/// Resolve a group argument against the live group list: exact full-name match
/// wins; else expand `<PREFIX>-<arg>`. Fail-loud with the names tried.
pub(crate) fn resolve_group<'a>(
    groups: &'a [NbGroup],
    prefix: &str,
    arg: &str,
) -> anyhow::Result<&'a NbGroup> {
    if let Some(g) = groups.iter().find(|g| g.name == arg) {
        return Ok(g);
    }
    let full = format!("{prefix}-{arg}");
    groups.iter().find(|g| g.name == full).ok_or_else(|| {
        anyhow::anyhow!(
            "no group named `{arg}` or `{full}` on this mgmt server — \
             `goat admin peer ls` shows live groups; declare new groups in \
             ops/ansible/netbird_acl_taxonomy.yml + run netbird-acl-bootstrap.yml"
        )
    })
}

/// Group ids referenced by any enabled rule of any enabled policy — the set a
/// destination must intersect to count as covered (fail-closed gate).
pub(crate) fn covered_group_ids(policies: &[NbPolicy]) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for p in policies.iter().filter(|p| p.enabled) {
        for r in p.rules.iter().filter(|r| r.enabled) {
            for g in r.sources.iter().chain(r.destinations.iter()) {
                if !g.id.is_empty() {
                    ids.insert(g.id.clone());
                }
            }
        }
    }
    ids
}

#[derive(Debug, Clone, serde::Serialize)]
struct Mover {
    peer_id: String,
    name: String,
    mesh_ip: String,
    connected: bool,
    last_seen: String,
    /// Other group names this peer holds (informational; the strip set when
    /// --strip-others).
    other_groups: Vec<String>,
}

#[derive(Debug, Default, serde::Serialize)]
struct Plan {
    from_group: String,
    to_group: String,
    movers: Vec<Mover>,
    /// Selected peers already at the destination and out of the source —
    /// skipped (the idempotency count).
    already_at_destination: usize,
}

struct PrivilegedQuarantinePreflight<'a> {
    movers: &'a [Mover],
    peers: &'a [super::warrant::LivePeer],
    from_group: &'a str,
    to_group: &'a str,
    prefix: &'a str,
    env: &'a str,
    strip_others: bool,
    ory_plane: bool,
    set_marker: Option<&'a str>,
    clear_marker: Option<&'a str>,
    people: &'a BTreeMap<String, super::warrant::PersonBaseline>,
    baselines: &'a BTreeMap<String, super::warrant::Baseline>,
    ory: Result<&'a super::warrant::OryPrivilegeIndex, &'a str>,
    selector_emails: &'a [String],
    missing_owner_reason: Option<&'a str>,
}

fn privileged_quarantine_preflight(input: PrivilegedQuarantinePreflight<'_>) -> Vec<String> {
    let (_mesh_transition, identity_transition) = quarantine_transitions(
        input.from_group,
        input.to_group,
        input.prefix,
        input.ory_plane,
        input.set_marker,
        input.clear_marker,
    );
    let mut refusals = Vec::new();
    for mover in input.movers {
        let Some(peer) = input.peers.iter().find(|p| p.id == mover.peer_id) else {
            refusals.push(format!(
                "{}: {}",
                mover.name,
                super::warrant::unproven_owner_refusal(
                    "the selected peer disappeared before privilege evaluation",
                )
            ));
            continue;
        };
        let mut desired: BTreeSet<String> = peer.groups.iter().cloned().collect();
        desired.remove(input.from_group);
        desired.insert(input.to_group.to_string());
        if input.strip_others {
            desired.retain(|g| g == "All" || g == input.to_group);
        }
        if let Some(reason) = super::warrant::quarantine_refusal_with_owner_reason(
            peer,
            &desired,
            input.env,
            input.prefix,
            super::warrant::OwnerJoin {
                people: input.people,
                missing_reason: input.missing_owner_reason,
            },
            input.baselines.get(&peer.id),
            input.ory,
        ) {
            refusals.push(format!("{}: {reason}", mover.name));
        }
    }
    if identity_transition {
        for email in input.selector_emails {
            if let Some(reason) = super::warrant::ory_identity_quarantine_refusal(
                email,
                input.env,
                input.prefix,
                input.people,
                input.peers,
                input.baselines,
                input.ory,
            ) {
                refusals.push(reason);
            }
        }
    }
    refusals
}

enum Selection {
    Bulk,
    Selectors(Vec<String>),
    Filter(Vec<Pred>),
}

fn build_plan(
    peers: &[NbPeer],
    from_name: &str,
    to_name: &str,
    sel: &Selection,
    now: chrono::DateTime<chrono::Utc>,
) -> Plan {
    let mut plan = Plan {
        from_group: from_name.to_string(),
        to_group: to_name.to_string(),
        ..Default::default()
    };
    for p in peers {
        let names = p.group_names();
        let in_from = names.contains(&from_name);
        let in_to = names.contains(&to_name);
        let selected = match sel {
            Selection::Bulk => true,
            Selection::Selectors(sels) => sels.iter().any(|s| p.matches(s)),
            Selection::Filter(preds) => peer_passes(p, preds, now),
        };
        if !selected {
            continue;
        }
        if in_from {
            plan.movers.push(Mover {
                peer_id: p.id.clone(),
                name: p.name.clone(),
                mesh_ip: p.ip.clone(),
                connected: p.connected,
                last_seen: p.last_seen.clone(),
                other_groups: names
                    .iter()
                    .filter(|n| **n != from_name && **n != to_name && **n != "All")
                    .map(|n| n.to_string())
                    .collect(),
            });
        } else if in_to {
            plan.already_at_destination += 1;
        }
        // Selected, in neither group: not a member of the source — ignored.
    }
    plan
}

/// JSON-Patch ops for the Ory-plane leg: swap `from`→`to` inside the login
/// groups (only when `from` is present), plus optional boolean marker
/// set/clear. Returns None when there is nothing to change.
///
/// Login groups live in admin-managed `metadata_public.groups` (F-572 / ADR
/// 1188), not self-editable traits. `current_meta` is the identity's
/// `metadata_public`: when it already carries `groups` the swap is a replace on
/// that path; an identity not yet backfilled gets the whole object added (its
/// other keys kept), since a replace or child add on a missing path fails.
/// Non-authz boolean markers stay in traits.
fn identity_patch_ops(
    current_groups: &[String],
    current_meta: Option<&serde_json::Map<String, serde_json::Value>>,
    from: Option<&str>,
    to: Option<&str>,
    set_marker: Option<&str>,
    clear_marker: Option<&str>,
    marker_currently_set: bool,
) -> Option<serde_json::Value> {
    let mut ops = Vec::new();
    if let (Some(from), Some(to)) = (from, to) {
        if current_groups.iter().any(|g| g == from) {
            let new_groups: Vec<&str> = current_groups
                .iter()
                .map(|g| if g == from { to } else { g.as_str() })
                .collect();
            // Dedup while preserving order (target may already be present).
            let mut seen = BTreeSet::new();
            let deduped: Vec<&str> = new_groups
                .into_iter()
                .filter(|g| seen.insert(g.to_string()))
                .collect();
            let mut meta = current_meta.cloned().unwrap_or_default();
            if meta.contains_key("groups") {
                ops.push(serde_json::json!({
                    "op": "replace", "path": "/metadata_public/groups", "value": deduped
                }));
            } else {
                meta.insert("groups".into(), serde_json::json!(deduped));
                ops.push(serde_json::json!({
                    "op": "add", "path": "/metadata_public", "value": meta
                }));
            }
        }
    }
    if let Some(m) = set_marker {
        if !marker_currently_set {
            ops.push(serde_json::json!({
                "op": "add", "path": format!("/traits/{m}"), "value": true
            }));
        }
    }
    if let Some(m) = clear_marker {
        if marker_currently_set {
            ops.push(serde_json::json!({
                "op": "remove", "path": format!("/traits/{m}")
            }));
        }
    }
    if ops.is_empty() {
        None
    } else {
        Some(serde_json::Value::Array(ops))
    }
}

// ---------------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------------

async fn run_move(ctx: &super::Ctx, a: MoveArgs) -> anyhow::Result<()> {
    match a.planes.as_str() {
        "mesh" | "ory" | "both" => {}
        other => anyhow::bail!("--planes must be mesh|ory|both, got `{other}`"),
    }
    let mesh_plane = a.planes == "mesh" || a.planes == "both";
    let ory_plane = a.planes == "ory" || a.planes == "both";
    let markers = a.set_marker.is_some() || a.clear_marker.is_some();

    let env = a
        .env
        .clone()
        .unwrap_or_else(|| ctx.profile.context.clone());
    let prefix = env.to_uppercase();

    // Selection: exactly one of handle/list/filter, or none (bulk).
    let mut selectors: Vec<String> = Vec::new();
    if let Some(h) = &a.handle {
        selectors.push(h.clone());
    }
    if let Some(path) = &a.list {
        let body = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("read --list {}: {e}", path.display()))?;
        selectors.extend(
            body.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(String::from),
        );
    }
    let preds = a.filter.as_deref().map(parse_filter).transpose()?;
    let sel = match (&selectors[..], &preds) {
        ([], None) => Selection::Bulk,
        ([], Some(p)) => Selection::Filter(p.clone()),
        (_, None) => Selection::Selectors(selectors.clone()),
        (_, Some(_)) => {
            anyhow::bail!("--filter cannot be combined with --handle/--list — pick one selection")
        }
    };
    if (ory_plane || markers) && matches!(sel, Selection::Bulk | Selection::Filter(_)) {
        anyhow::bail!(
            "the Ory-plane / marker legs need identity-shaped selectors: use --handle or --list \
             with handles or emails (the mesh plane alone supports bulk/--filter)"
        );
    }

    // -------------------------------------------------------------------
    // Mesh plane: fetch, resolve, gate, plan.
    // -------------------------------------------------------------------
    let nb = ctx.profile.netbird_client(a.mgmt_url.as_deref())?;
    let groups = nb.list_groups().await.map_err(map_nb_err)?;
    let peers = nb.list_peers().await.map_err(map_nb_err)?;

    let from_g = resolve_group(&groups, &prefix, &a.from)?.clone();
    let to_g = resolve_group(&groups, &prefix, &a.to)?.clone();
    if from_g.id == to_g.id {
        anyhow::bail!("--from and --to resolve to the same group ({})", from_g.name);
    }

    if mesh_plane {
        let policies = nb.list_policies().await.map_err(map_nb_err)?;
        let covered = covered_group_ids(&policies);
        if !covered.contains(&to_g.id) {
            anyhow::bail!(
                "COVERAGE GATE: destination `{}` is not referenced by any enabled policy — \
                 moving peers there would leave them unreachable under default-deny. \
                 Stamp its matrix rule (ops/ansible/netbird_acl_matrix.yml) and run the \
                 cutover playbook mode=apply first.",
                to_g.name
            );
        }
    }

    let now = chrono::Utc::now();
    let plan = build_plan(&peers, &from_g.name, &to_g.name, &sel, now);

    // ADR 1091: a security-quarantine transition involving a privileged
    // account is not a reversible membership move. Prove the peer owner and
    // read the owner's live Ory roles before printing, writing a declaration,
    // or touching either plane. Machines have no Ory owner and are graded from
    // their live/declared mesh baseline instead.
    let (mesh_quarantine_transition, identity_quarantine_transition) = quarantine_transitions(
        &from_g.name,
        &to_g.name,
        &prefix,
        ory_plane,
        a.set_marker.as_deref(),
        a.clear_marker.as_deref(),
    );
    let quarantine = format!("{prefix}-QUARANTINED");
    let strip_quarantine_transition = a.strip_others
        && to_g.name != quarantine
        && plan.movers.iter().any(|mover| {
            peers
                .iter()
                .find(|peer| peer.id == mover.peer_id)
                .map(|peer| peer.group_names().contains(&quarantine.as_str()))
                .unwrap_or(false)
        });
    let quarantine_preflight =
        ((mesh_quarantine_transition || strip_quarantine_transition) && !plan.movers.is_empty())
            || (identity_quarantine_transition && !selectors.is_empty());
    let needs_ory = quarantine_preflight
        && (identity_quarantine_transition
            || plan
                .movers
                .iter()
                .any(|m| panoscope_core::peername::rest(&m.name).is_some()));
    let ory_config = resolve_move_ory_config(&env, &ctx.profile, needs_ory || ory_plane || markers)
        .map_err(|error| {
            if needs_ory {
                anyhow::anyhow!(super::warrant::unproven_owner_refusal(
                    "the live Ory owner lookup has no resolved configuration"
                ))
            } else {
                error
            }
        })?;
    if quarantine_preflight {
        use super::warrant::{
            key_joins, load_decl, membership_path, people_from_manifests, unproven_owner_refusal,
            LivePeer,
        };

        let decl = match membership_path(&env) {
            Some(path) if path.is_dir() => Some(load_decl(&path, &env).map_err(|_| {
                anyhow::anyhow!(unproven_owner_refusal(
                    "the local membership declaration is malformed or unreadable"
                ))
            })?),
            _ => None,
        };
        let cwd = std::env::current_dir().ok();
        let repo_root = cwd.as_deref().and_then(super::admin::discover_repo_root);
        let manifest_dir = repo_root
            .as_ref()
            .map(|root| root.join("ops/operators").join(&env));
        let manifest_people = match manifest_dir.as_deref() {
            Some(dir) if dir.is_dir() => people_from_manifests(dir).map_err(|_| {
                anyhow::anyhow!(unproven_owner_refusal(
                    "a local operator manifest is malformed, unreadable, or missing required \
                     identity fields"
                ))
            })?,
            _ => BTreeMap::new(),
        };
        let missing_owner_reason = repo_root.is_none().then_some(
            "no owner join is available because the command is not running from a repo checkout",
        );
        let live_peers: Vec<LivePeer> = peers.iter().map(LivePeer::from).collect();
        let events = nb.list_events().await.map_err(map_nb_err)?;
        let keys = nb.list_setup_keys().await.map_err(map_nb_err)?;
        let joins = key_joins(&events, &keys, &groups);
        let (people, baselines) =
            move_people_and_baselines(decl.as_ref(), manifest_people, &env, &live_peers, &joins);
        let ory = if needs_ory {
            let cfg = ory_config.as_ref().ok_or_else(|| {
                anyhow::anyhow!(unproven_owner_refusal(
                    "the live Ory owner lookup has no resolved configuration"
                ))
            })?;
            Some(super::warrant::read_ory_privileges(cfg).await)
        } else {
            None
        };
        let selector_emails = if identity_quarantine_transition {
            selectors
                .iter()
                .map(|selector| {
                    resolve_selector_email(ctx, &env, selector).map_err(|_| {
                        anyhow::anyhow!(unproven_owner_refusal(
                            "the selected Ory identity could not be joined to a known owner"
                        ))
                    })
                })
                .collect::<anyhow::Result<Vec<_>>>()?
        } else {
            Vec::new()
        };
        let ory_view = match ory.as_ref() {
            Some(Ok(index)) => Ok(index),
            _ => Err("the live Ory owner lookup was unavailable or ambiguous"),
        };
        // Load-bearing order: this pure decision runs before plan output,
        // warrant-file authoring, or any NetBird/Ory mutation.
        let refusals = privileged_quarantine_preflight(PrivilegedQuarantinePreflight {
            movers: &plan.movers,
            peers: &live_peers,
            from_group: &from_g.name,
            to_group: &to_g.name,
            prefix: &prefix,
            env: &env,
            strip_others: a.strip_others,
            ory_plane,
            set_marker: a.set_marker.as_deref(),
            clear_marker: a.clear_marker.as_deref(),
            people: &people,
            baselines: &baselines,
            ory: ory_view,
            selector_emails: &selector_emails,
            missing_owner_reason,
        });
        if !refusals.is_empty() {
            anyhow::bail!(refusals.join("\n"));
        }
    }

    if a.json {
        println!("{}", serde_json::to_string_pretty(&plan)?);
    } else {
        eprintln!(
            "plan: {} → {}   movers={}  already-at-destination={}",
            plan.from_group,
            plan.to_group,
            plan.movers.len(),
            plan.already_at_destination
        );
        for m in &plan.movers {
            eprintln!(
                "  {}  {:<15}  {:<28}  {}{}",
                m.peer_id,
                m.mesh_ip,
                m.name,
                if m.connected { "up" } else { "down" },
                if a.strip_others && !m.other_groups.is_empty() {
                    format!("  strips: {}", m.other_groups.join(","))
                } else {
                    String::new()
                }
            );
        }
    }

    if plan.movers.is_empty() && !((ory_plane || markers) && !selectors.is_empty()) {
        eprintln!("nothing to do (idempotent: selection already at destination or empty)");
        return Ok(());
    }

    // Warrant owns membership on a governed scope (mesh-change-control.md
    // §4.5): the mesh leg becomes an entry in the declaration, never a live
    // write. An ungoverned scope never enters this block.
    let governance = if mesh_plane {
        Some(super::warrant::governance(&env, &ctx.profile, &nb).await)
    } else {
        None
    };
    let governed = governance.is_some_and(|g| g.governed);
    if let Some(g) = governance.filter(|g| g.governed) {
        eprintln!("`{env}` membership is governed by warrant — {}", g.because());
    }
    if a.write_entry && !governed {
        anyhow::bail!(
            "--write-entry applies only on a scope governed by warrant \
             (its `_scope.yaml` says `converge: enforce`); `{env}` is not governed{}",
            governance
                .map(|g| format!(" — {}", g.because()))
                .unwrap_or_default()
        );
    }
    if governed && (a.apply || a.write_entry) {
        return governed_move(ctx, &a, &env, &prefix, &from_g.name, &to_g.name, &plan);
    }

    if !a.apply {
        eprintln!("dry-run — nothing touched. Re-run with --apply to execute.");
        return Ok(());
    }

    // -------------------------------------------------------------------
    // Apply — mesh plane, mirror-first.
    // -------------------------------------------------------------------
    let mut outcome_ok = true;
    let mut error_msg = String::new();

    if mesh_plane && !plan.movers.is_empty() {
        let mover_ids: BTreeSet<String> =
            plan.movers.iter().map(|m| m.peer_id.clone()).collect();

        // 1. Mirror-add: destination gets the union.
        let mut to_ids: BTreeSet<String> =
            to_g.peers.iter().map(|p| p.id.clone()).collect();
        to_ids.extend(mover_ids.iter().cloned());
        let to_vec: Vec<String> = to_ids.iter().cloned().collect();
        nb.update_group_peers(&to_g.id, &to_g.name, &to_vec)
            .await
            .map_err(map_nb_err)?;

        // 2. Verify the addition landed before shrinking anything.
        let verify = nb.list_groups().await.map_err(map_nb_err)?;
        let live_to: BTreeSet<String> = verify
            .iter()
            .find(|g| g.id == to_g.id)
            .map(|g| g.peers.iter().map(|p| p.id.clone()).collect())
            .unwrap_or_default();
        let missing: Vec<&String> =
            mover_ids.iter().filter(|id| !live_to.contains(*id)).collect();
        if !missing.is_empty() {
            anyhow::bail!(
                "mirror-add verify FAILED: {} mover(s) not in `{}` after PUT — aborting before \
                 any removal (peers are at worst dual-homed; re-run to resume): {:?}",
                missing.len(),
                to_g.name,
                missing
            );
        }
        eprintln!("mirror-add verified: {} peer(s) in {}", mover_ids.len(), to_g.name);

        // 3. Shrink the source.
        let from_remaining: Vec<String> = from_g
            .peers
            .iter()
            .map(|p| p.id.clone())
            .filter(|id| !mover_ids.contains(id))
            .collect();
        nb.update_group_peers(&from_g.id, &from_g.name, &from_remaining)
            .await
            .map_err(map_nb_err)?;

        // 4. Optional strip of every other group (quarantine posture).
        if a.strip_others {
            // Map group name → group (with live membership) once.
            let live: BTreeMap<String, &NbGroup> =
                verify.iter().map(|g| (g.name.clone(), g)).collect();
            let mut strip_names: BTreeSet<String> = BTreeSet::new();
            for m in &plan.movers {
                for g in &m.other_groups {
                    strip_names.insert(g.clone());
                }
            }
            for name in &strip_names {
                let Some(g) = live.get(name) else { continue };
                let remaining: Vec<String> = g
                    .peers
                    .iter()
                    .map(|p| p.id.clone())
                    .filter(|id| !mover_ids.contains(id))
                    .collect();
                if remaining.len() != g.peers.len() {
                    nb.update_group_peers(&g.id, &g.name, &remaining)
                        .await
                        .map_err(map_nb_err)?;
                    eprintln!("stripped {} mover(s) from {}", g.peers.len() - remaining.len(), name);
                }
            }
        }

        // 5. Final verify: every mover in TO, none in FROM.
        let after = nb.list_groups().await.map_err(map_nb_err)?;
        let final_from: BTreeSet<String> = after
            .iter()
            .find(|g| g.id == from_g.id)
            .map(|g| g.peers.iter().map(|p| p.id.clone()).collect())
            .unwrap_or_default();
        let leftover: Vec<&String> =
            mover_ids.iter().filter(|id| final_from.contains(*id)).collect();
        if leftover.is_empty() {
            eprintln!(
                "mesh move complete: {} peer(s) {} → {}",
                mover_ids.len(),
                from_g.name,
                to_g.name
            );
        } else {
            outcome_ok = false;
            error_msg = format!("{} mover(s) still in source after removal", leftover.len());
            eprintln!("WARNING: {error_msg} — re-run to converge");
        }
    }

    // -------------------------------------------------------------------
    // Apply — Ory plane / markers (identity-shaped selectors only).
    // -------------------------------------------------------------------
    if ory_plane || markers {
        let cfg = ory_config.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "no Ory facts for goatnet `{env}` — run `goat context refresh` in that goatnet's context"
            )
        })?;
        for sel in &selectors {
            let email = resolve_selector_email(ctx, &env, sel)?;
            match apply_identity_leg(
                cfg,
                &email,
                if ory_plane { Some((a.from.as_str(), a.to.as_str())) } else { None },
                a.set_marker.as_deref(),
                a.clear_marker.as_deref(),
            )
            .await
            {
                Ok(desc) => eprintln!("ory[{sel}]: {desc}"),
                Err(e) => {
                    outcome_ok = false;
                    error_msg = format!("ory[{sel}]: {e}");
                    eprintln!("ory[{sel}] FAILED: {e}");
                }
            }
        }
    }

    // -------------------------------------------------------------------
    // One OperatorActionV1 per applied run.
    // -------------------------------------------------------------------
    let actor = super::audit::actor_handle(&ctx.profile);
    let target = format!("{}→{} n={}", from_g.name, to_g.name, plan.movers.len());
    let summary = format!(
        "members move from={} to={} movers={} planes={} strip={} markers={}/{}",
        from_g.name,
        to_g.name,
        plan.movers.len(),
        a.planes,
        a.strip_others,
        a.set_marker.as_deref().unwrap_or("-"),
        a.clear_marker.as_deref().unwrap_or("-"),
    );
    let mut act = super::audit::OperatorActionV1::new(
        &actor,
        "goat-cli",
        "admin.members.move",
        target,
        env.clone(),
        if outcome_ok { "ok" } else { "failed" },
    )
    .with_args_digest(&summary);
    if !outcome_ok {
        act = act.with_error(error_msg.clone());
    }
    super::audit::emit(&ctx.profile, act).await;

    if outcome_ok {
        Ok(())
    } else {
        anyhow::bail!("members move finished with errors: {error_msg}")
    }
}

// ---------------------------------------------------------------------------
// Governed scopes — the move becomes a warrant membership entry
// ---------------------------------------------------------------------------

/// Default `expires:` for an entry the CI requires one on (§4.3): a hold is a
/// review date a week out; a grant or exact naming a sensitive group gets 90
/// days. Anything else carries none.
pub(crate) fn default_expires(form: &super::warrant::EntryForm) -> Option<String> {
    use super::warrant::EntryForm;
    const SENSITIVE: &[&str] = &["-EXTERNAL-INFRA", "-PLATFORM-ADMIN", "-BREAKGLASS", "-P2P"];
    let days = match form {
        EntryForm::Hold => 7,
        EntryForm::Exact(g) | EntryForm::GrantAbsent { grant: g, .. }
            if g.iter().any(|x| SENSITIVE.iter().any(|s| x.ends_with(s))) =>
        {
            90
        }
        _ => return None,
    };
    Some(
        (chrono::Utc::now().date_naive() + chrono::Duration::days(days))
            .format("%Y-%m-%d")
            .to_string(),
    )
}

/// The entry form that expresses moving a peer from `from` to `to`. Pure.
///
/// - `--to QUARANTINED --strip-others` → `hold: quarantine`;
/// - `--to EXTERNAL-INFRA` → `exact: [<P>-EXTERNAL-INFRA]` (§4.3: a service
///   host holds it alone);
/// - any other `--strip-others` → `exact: [<to>]`;
/// - otherwise `grant: [<to>]`, `absent: [<from>]`.
fn move_form(
    prefix: &str,
    from: &str,
    to: &str,
    strip_others: bool,
) -> anyhow::Result<super::warrant::EntryForm> {
    use super::warrant::EntryForm;
    let managed = |g: &str| g.starts_with(&format!("{prefix}-"));
    if !managed(to) {
        anyhow::bail!(
            "`{to}` is not a managed group of this scope (managed groups start with `{prefix}-`) — \
             a warrant membership entry cannot express this move"
        );
    }
    if to == format!("{prefix}-QUARANTINED") && strip_others {
        return Ok(EntryForm::Hold);
    }
    if to == format!("{prefix}-EXTERNAL-INFRA") || strip_others {
        return Ok(EntryForm::Exact(BTreeSet::from([to.to_string()])));
    }
    let absent = if managed(from) {
        BTreeSet::from([from.to_string()])
    } else {
        BTreeSet::new()
    };
    Ok(EntryForm::GrantAbsent {
        grant: BTreeSet::from([to.to_string()]),
        absent,
    })
}

fn governed_move(
    ctx: &super::Ctx,
    a: &MoveArgs,
    env: &str,
    prefix: &str,
    from: &str,
    to: &str,
    plan: &Plan,
) -> anyhow::Result<()> {
    use super::warrant::{
        entry_yaml, membership_path, target_fragment, write_entry, MembershipEntry,
    };
    let form = move_form(prefix, from, to, a.strip_others)?;
    let owner = a
        .owner
        .clone()
        .unwrap_or_else(|| super::audit::actor_handle(&ctx.profile));
    let reason = a
        .reason
        .clone()
        .unwrap_or_else(|| format!("members move {from} -> {to}"));
    let expires = a.expires.clone().or_else(|| default_expires(&form));
    let entries: Vec<MembershipEntry> = plan
        .movers
        .iter()
        .map(|m| {
            MembershipEntry::with_form(&m.name, form.clone(), &owner, &reason, expires.clone())
        })
        .collect();
    let path = membership_path(env);
    let shown = path
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| format!("ops/warrant/membership/{env}"));
    let target = target_fragment(&form, prefix);

    eprintln!(
        "\nscope `{env}` is governed by warrant: mesh membership changes are made in\n  {shown}\n\
         (a new entry goes in {target}; a peer that already has one is edited where it is)\n\
         and applied by the converger, not written live. The entry for this move:\n"
    );
    let yaml: String = entries.iter().map(|e| entry_yaml(e, "  ")).collect();
    if a.json {
        eprint!("{yaml}");
    } else {
        print!("{yaml}");
    }

    if a.write_entry {
        let p = path.filter(|p| p.is_dir()).ok_or_else(|| {
            anyhow::anyhow!(
                "--write-entry needs a checkout holding the scope directory {shown} — run it \
                 from inside the repo"
            )
        })?;
        let mut written = BTreeSet::new();
        for e in entries {
            let (_, file) = write_entry(&p, env, target, e)?;
            written.insert(file.display().to_string());
        }
        eprintln!(
            "\nwrote {} entr{} to {} — commit it and open a PR; the converger applies it \
             (runbook: {}).",
            plan.movers.len(),
            if plan.movers.len() == 1 { "y" } else { "ies" },
            written.into_iter().collect::<Vec<_>>().join(", "),
            super::warrant::RUNBOOK
        );
    }
    if a.apply {
        anyhow::bail!(
            "REFUSED --apply: `{env}` is governed by warrant, so this command does not write mesh \
             membership. Put the entry above in {shown} (--write-entry does it), open a PR, and \
             the converger applies it. Runbook: {}. For the Ory leg alone, re-run with --planes ory.",
            super::warrant::RUNBOOK
        );
    }
    Ok(())
}

/// Selector → Kratos identifier email. An email-shaped selector passes
/// through; a kebab handle resolves via the per-env operator manifest
/// (`ops/operators/<env>/<handle>.yml`, `identity.email`).
fn resolve_selector_email(ctx: &super::Ctx, env: &str, sel: &str) -> anyhow::Result<String> {
    if sel.contains('@') {
        return Ok(sel.to_string());
    }
    let _ = ctx; // profile carries no repo hint; manifests resolve from cwd
    let start = std::env::current_dir()
        .map_err(|e| anyhow::anyhow!("cannot resolve cwd for manifest lookup: {e}"))?;
    let root = super::admin::discover_repo_root(&start).ok_or_else(|| {
        anyhow::anyhow!(
            "cannot resolve handle `{sel}` to an email: not inside the repo (manifests live at \
             ops/operators/<env>/<handle>.yml) — pass the email directly instead"
        )
    })?;
    let path = root.join("ops/operators").join(env).join(format!("{sel}.yml"));
    let body = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("read manifest {}: {e}", path.display()))?;
    let manifest: serde_yaml::Value = serde_yaml::from_str(&body)
        .map_err(|e| anyhow::anyhow!("parse manifest {}: {e}", path.display()))?;
    let email = manifest
        .get("identity")
        .and_then(|identity| identity.get("email"))
        .and_then(serde_yaml::Value::as_str)
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("manifest {} has no identity.email", path.display()))?;
    Ok(email)
}

/// GET the identity by email, compute the JSON-Patch (groups swap + markers),
/// PATCH it. Returns a short human description of what changed.
async fn apply_identity_leg(
    cfg: &super::admin::OryEnvConfig,
    email: &str,
    swap: Option<(&str, &str)>,
    set_marker: Option<&str>,
    clear_marker: Option<&str>,
) -> anyhow::Result<String> {
    let get = format!(
        "curl -sS --max-time 30 -w '\\n%{{http_code}}' '{}/admin/identities?credentials_identifier={}'",
        cfg.kratos_admin_url,
        super::admin::pct_encode_query(email)
    );
    let (code, body) = super::admin::run_ssh_curl(cfg, &get, None).await?;
    if code != 200 {
        anyhow::bail!("Kratos GET identities HTTP {code}");
    }
    let v: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| anyhow::anyhow!("decode identities: {e}"))?;
    let ident = v
        .as_array()
        .and_then(|a| a.first())
        .ok_or_else(|| anyhow::anyhow!("no Kratos identity for {email}"))?;
    let id = ident
        .get("id")
        .and_then(|i| i.as_str())
        .ok_or_else(|| anyhow::anyhow!("identity for {email} has no id"))?;
    let traits = ident.get("traits").cloned().unwrap_or_default();
    let meta = ident.get("metadata_public").and_then(|m| m.as_object());
    // Login groups: admin-managed metadata_public first,
    // traits only for an identity the backfill has not reached yet.
    let current_groups: Vec<String> = meta
        .and_then(|m| m.get("groups"))
        .or_else(|| traits.get("groups"))
        .and_then(|g| g.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let marker_key = set_marker.or(clear_marker);
    let marker_currently_set = marker_key
        .and_then(|m| traits.get(m))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let Some(ops) = identity_patch_ops(
        &current_groups,
        meta,
        swap.map(|(f, _)| f),
        swap.map(|(_, t)| t),
        set_marker,
        clear_marker,
        marker_currently_set,
    ) else {
        return Ok("no change (idempotent)".into());
    };
    let n_ops = ops.as_array().map(|a| a.len()).unwrap_or(0);
    let patch = format!(
        "curl -sS --max-time 30 -X PATCH -H 'Content-Type: application/json' \
         --data-binary @- -w '\\n%{{http_code}}' '{}/admin/identities/{id}'",
        cfg.kratos_admin_url
    );
    let (c, b) = super::admin::run_ssh_curl(cfg, &patch, Some(&ops.to_string())).await?;
    if c != 200 {
        anyhow::bail!(
            "Kratos PATCH HTTP {c} (a 4xx on a marker usually means the Kratos identity \
             schema does not carry the trait yet — the Stage-3 schema dependency): {}",
            b.chars().take(200).collect::<String>()
        );
    }
    Ok(format!("patched ({n_ops} op(s))"))
}

pub(crate) fn map_nb_err(e: panoscope_core::netbird::NetbirdError) -> anyhow::Error {
    use panoscope_core::netbird::NetbirdError as E;
    match e {
        E::Disabled => anyhow::anyhow!(
            "NetBird mgmt client not configured — set the profile mgmt URL or pass --mgmt-url, \
             and export the site's NB API token"
        ),
        E::NotFound => anyhow::anyhow!("group/peer not found"),
        // Was missing entirely here (unlike admin.rs's copy of this same
        // function) — a 403 fell into the generic `other` arm below and
        // read as a wrong-scope/bad-URL fault instead of the actual cause:
        // a valid-but-unauthorized admin token, the operator-only path.
        E::Http { status } if status.as_u16() == 403 => crate::errors::GoatError::NotAdmin {
            hint: format!(
                "NetBird mgmt API denied this request (HTTP {status}). The admin token is valid \
                 but not authorized to manage groups/peers. Nothing was changed. Use an \
                 admin-scoped NetBird token, or contact your goat administrator."
            ),
        }
        .into(),
        E::Http { status } => anyhow::anyhow!(
            "NetBird mgmt API returned HTTP {status} — token wrong scope, or URL incorrect"
        ),
        E::Transient(s) => anyhow::anyhow!("NetBird mgmt API unreachable: {s}"),
    }
}

// ---------------------------------------------------------------------------
// Tests — pure planning core.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use panoscope_core::netbird::{NbGroupPeer, NbPolicyRule};

    fn grp(id: &str, name: &str, peer_ids: &[&str]) -> NbGroup {
        NbGroup {
            id: id.into(),
            name: name.into(),
            peers: peer_ids
                .iter()
                .map(|p| NbGroupPeer { id: (*p).into(), name: String::new() })
                .collect(),
        }
    }

    fn peer(id: &str, name: &str, groups: &[&str], connected: bool, last_seen: &str) -> NbPeer {
        NbPeer {
            id: id.into(),
            name: name.into(),
            hostname: name.into(),
            ip: format!("100.64.0.{}", id.len()),
            connected,
            last_seen: last_seen.into(),
            os: String::new(),
            groups: groups.iter().map(|g| grp("", g, &[])).collect(),
            // Spread the rest. This fixture names only the fields the members
            // tests actually assert on; every other field takes its default.
            //
            // Without this, adding ANY field to NbPeer breaks this test even
            // though the test does not care about the new field — which is
            // what happened when netvis needed richer peer data, and it is the
            // second time in this codebase that a fixture's exhaustive struct
            // literal turned an additive change into a cross-crate CI failure.
            // A fixture should pin what it asserts and stay silent about the
            // rest.
            ..Default::default()
        }
    }

    fn now() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339("2026-07-17T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc)
    }

    fn person(email: &str, groups: &[&str]) -> crate::cmd::warrant::PersonBaseline {
        crate::cmd::warrant::PersonBaseline {
            email_hash: crate::cmd::warrant::email_hash(email),
            device: groups.iter().map(|group| (*group).to_string()).collect(),
            handset: Vec::new(),
        }
    }

    struct PreflightCase<'a> {
        from: &'a str,
        to: &'a str,
        strip_others: bool,
        ory_plane: bool,
        marker: Option<&'a str>,
        selector_emails: &'a [String],
    }

    fn preflight(
        case: PreflightCase<'_>,
        peers: &[NbPeer],
        people: &BTreeMap<String, crate::cmd::warrant::PersonBaseline>,
        ory: Result<&crate::cmd::warrant::OryPrivilegeIndex, &str>,
    ) -> Vec<String> {
        preflight_with_baselines(case, peers, people, &BTreeMap::new(), ory)
    }

    fn preflight_with_baselines(
        case: PreflightCase<'_>,
        peers: &[NbPeer],
        people: &BTreeMap<String, crate::cmd::warrant::PersonBaseline>,
        baselines: &BTreeMap<String, crate::cmd::warrant::Baseline>,
        ory: Result<&crate::cmd::warrant::OryPrivilegeIndex, &str>,
    ) -> Vec<String> {
        let plan = build_plan(peers, case.from, case.to, &Selection::Bulk, now());
        let live_peers = peers
            .iter()
            .map(crate::cmd::warrant::LivePeer::from)
            .collect::<Vec<_>>();
        privileged_quarantine_preflight(PrivilegedQuarantinePreflight {
            movers: &plan.movers,
            peers: &live_peers,
            from_group: case.from,
            to_group: case.to,
            prefix: "ZAGROS",
            env: "zagros",
            strip_others: case.strip_others,
            ory_plane: case.ory_plane,
            set_marker: case.marker,
            clear_marker: None,
            people,
            baselines,
            ory,
            selector_emails: case.selector_emails,
            missing_owner_reason: None,
        })
    }

    #[test]
    fn filter_parses_and_rejects() {
        assert_eq!(
            parse_filter("last_seen>14d, connected=false").unwrap(),
            vec![Pred::LastSeenOlderDays(14), Pred::Connected(false)]
        );
        assert!(parse_filter("role=dev").is_err());
        assert!(parse_filter("last_seen>abc").is_err());
        assert!(parse_filter("connected=maybe").is_err());
    }

    #[test]
    fn dormancy_filter_semantics() {
        let preds = parse_filter("last_seen>14d").unwrap();
        // 30 days quiet → dormant.
        let quiet = peer("a", "quiet", &[], false, "2026-06-17T12:00:00Z");
        assert!(peer_passes(&quiet, &preds, now()));
        // 2 days quiet → active.
        let recent = peer("b", "recent", &[], false, "2026-07-15T12:00:00Z");
        assert!(!peer_passes(&recent, &preds, now()));
        // Connected NOW → active regardless of stale last_seen.
        let live = peer("c", "live", &[], true, "2026-01-01T00:00:00Z");
        assert!(!peer_passes(&live, &preds, now()));
        // Never seen (empty) → dormant.
        let never = peer("d", "never", &[], false, "");
        assert!(peer_passes(&never, &preds, now()));
    }

    #[test]
    fn resolve_group_exact_then_prefixed() {
        let groups = vec![grp("g1", "ZAGROS-DEVELOPER", &[]), grp("g2", "ZAGROS-QUARANTINED", &[])];
        assert_eq!(resolve_group(&groups, "ZAGROS", "DEVELOPER").unwrap().id, "g1");
        assert_eq!(resolve_group(&groups, "ZAGROS", "ZAGROS-QUARANTINED").unwrap().id, "g2");
        let err = resolve_group(&groups, "ZAGROS", "NOPE").unwrap_err().to_string();
        assert!(err.contains("ZAGROS-NOPE"));
    }

    #[test]
    fn coverage_gate_reads_only_enabled() {
        let pols = vec![
            NbPolicy {
                id: "p1".into(),
                name: "live".into(),
                enabled: true,
                rules: vec![NbPolicyRule {
                    name: "r".into(),
                    enabled: true,
                    sources: vec![grp("gq", "ZAGROS-QUARANTINED", &[])],
                    destinations: vec![grp("gi", "ZAGROS-INFRA", &[])],
                    ..Default::default()
                }],
            },
            NbPolicy {
                id: "p2".into(),
                name: "Default".into(),
                enabled: false,
                rules: vec![NbPolicyRule {
                    name: "r".into(),
                    enabled: true,
                    sources: vec![grp("gall", "All", &[])],
                    destinations: vec![grp("gall", "All", &[])],
                    ..Default::default()
                }],
            },
        ];
        let covered = covered_group_ids(&pols);
        assert!(covered.contains("gq"));
        assert!(covered.contains("gi"));
        // The disabled Default policy covers nothing.
        assert!(!covered.contains("gall"));
    }

    #[test]
    fn plan_selects_movers_and_counts_idempotent_skips() {
        let peers = vec![
            peer("p1", "bob-laptop", &["All", "ZAGROS-DEVELOPER"], true, ""),
            peer("p2", "eve-box", &["All", "ZAGROS-DEVELOPER", "ZAGROS-P2P"], false, ""),
            peer("p3", "done-already", &["All", "ZAGROS-QUARANTINED"], false, ""),
            peer("p4", "unrelated", &["All", "ZAGROS-OBSERVER"], true, ""),
        ];
        // Bulk: both DEVELOPER members move; the already-moved one is counted.
        let plan = build_plan(&peers, "ZAGROS-DEVELOPER", "ZAGROS-QUARANTINED", &Selection::Bulk, now());
        assert_eq!(plan.movers.len(), 2);
        assert_eq!(plan.already_at_destination, 1);
        // The strip set excludes All / from / to.
        let eve = plan.movers.iter().find(|m| m.name == "eve-box").unwrap();
        assert_eq!(eve.other_groups, vec!["ZAGROS-P2P"]);

        // Selector: only eve.
        let sel = Selection::Selectors(vec!["eve".into()]);
        let plan = build_plan(&peers, "ZAGROS-DEVELOPER", "ZAGROS-QUARANTINED", &sel, now());
        assert_eq!(plan.movers.len(), 1);
        assert_eq!(plan.movers[0].peer_id, "p2");

        // Idempotency: post-move state plans nothing.
        let after = vec![
            peer("p1", "bob-laptop", &["All", "ZAGROS-QUARANTINED"], true, ""),
            peer("p2", "eve-box", &["All", "ZAGROS-QUARANTINED"], false, ""),
        ];
        let plan = build_plan(&after, "ZAGROS-DEVELOPER", "ZAGROS-QUARANTINED", &Selection::Bulk, now());
        assert!(plan.movers.is_empty());
        assert_eq!(plan.already_at_destination, 2);
    }

    #[test]
    fn quarantine_preflight_covers_mesh_ory_and_marker_paths() {
        assert_eq!(
            quarantine_transitions(
                "ZAGROS-DEVELOPER",
                "ZAGROS-QUARANTINED",
                "ZAGROS",
                false,
                None,
                None,
            ),
            (true, false)
        );
        assert_eq!(
            quarantine_transitions(
                "ZAGROS-DEVELOPER",
                "ZAGROS-QUARANTINED",
                "ZAGROS",
                true,
                None,
                None,
            ),
            (true, true)
        );
        assert_eq!(
            quarantine_transitions(
                "ZAGROS-DEVELOPER",
                "ZAGROS-OBSERVER",
                "ZAGROS",
                false,
                Some("quarantine"),
                None,
            ),
            (false, true)
        );
    }

    #[test]
    fn privileged_quarantine_preflight_blocks_entry_and_exit_before_any_move_mode() {
        let people = BTreeMap::from([(
            "alice".to_string(),
            person("alice@example.test", &["ZAGROS-DEVELOPER"]),
        )]);
        let idx =
            crate::cmd::warrant::OryPrivilegeIndex::one("alice@example.test", &["PLATFORM-ADMIN"]);
        let entering = vec![
            peer(
                "p1",
                "peer-alice-zagros-2026-09-01-laptop",
                &["ZAGROS-DEVELOPER"],
                true,
                "",
            ),
            peer(
                "p2",
                "peer-alice-zagros-2026-09-02-laptop",
                &["ZAGROS-DEVELOPER"],
                true,
                "",
            ),
        ];
        let refusals = preflight(
            PreflightCase {
                from: "ZAGROS-DEVELOPER",
                to: "ZAGROS-QUARANTINED",
                strip_others: false,
                ory_plane: false,
                marker: None,
                selector_emails: &[],
            },
            &entering,
            &people,
            Ok(&idx),
        );
        assert_eq!(refusals.len(), 2);
        assert!(refusals[0].starts_with("peer-alice-zagros-2026-09-01-laptop: "));
        assert!(refusals[1].starts_with("peer-alice-zagros-2026-09-02-laptop: "));
        assert!(refusals.iter().all(|r| !r.contains("alice@example.test")));
        let leaving = vec![peer(
            "p1",
            "peer-alice-zagros-2026-09-01-laptop",
            &["ZAGROS-QUARANTINED"],
            true,
            "",
        )];
        assert!(!preflight(
            PreflightCase {
                from: "ZAGROS-QUARANTINED",
                to: "ZAGROS-DEVELOPER",
                strip_others: false,
                ory_plane: false,
                marker: None,
                selector_emails: &[],
            },
            &leaving,
            &people,
            Ok(&idx),
        )
        .is_empty());
    }

    #[test]
    fn preflight_allows_non_privileged_person_machine_and_admin_dormancy_release() {
        let people = BTreeMap::from([(
            "alice".to_string(),
            person("alice@example.test", &["ZAGROS-DEVELOPER"]),
        )]);
        let idx = crate::cmd::warrant::OryPrivilegeIndex::one("alice@example.test", &["DEVELOPER"]);
        let peers = vec![
            peer(
                "p1",
                "peer-alice-zagros-2026-09-01-laptop",
                &["ZAGROS-DEVELOPER"],
                true,
                "",
            ),
            peer("p2", "sensor-1", &["ZAGROS-DEVELOPER"], true, ""),
        ];
        assert!(preflight(
            PreflightCase {
                from: "ZAGROS-DEVELOPER",
                to: "ZAGROS-QUARANTINED",
                strip_others: false,
                ory_plane: false,
                marker: None,
                selector_emails: &[],
            },
            &peers,
            &people,
            Ok(&idx),
        )
        .is_empty());

        let dormant = vec![peer(
            "p1",
            "peer-alice-zagros-2026-09-01-laptop",
            &["ZAGROS-PROBATION-PLATFORM-ADMIN"],
            true,
            "",
        )];
        assert!(preflight(
            PreflightCase {
                from: "ZAGROS-PROBATION-PLATFORM-ADMIN",
                to: "ZAGROS-PLATFORM-ADMIN",
                strip_others: false,
                ory_plane: false,
                marker: None,
                selector_emails: &[],
            },
            &dormant,
            &people,
            Err("Ory is not needed"),
        )
        .is_empty());
    }

    #[test]
    fn preflight_allows_ambiguous_manifest_handles_when_all_candidates_are_ordinary() {
        let people = BTreeMap::from([
            (
                "maintainer".to_string(),
                person("shared@example.test", &["ZAGROS-DEVELOPER"]),
            ),
            (
                "maintainer-smith".to_string(),
                person("shared@example.test", &["ZAGROS-DEVELOPER"]),
            ),
        ]);
        let idx =
            crate::cmd::warrant::OryPrivilegeIndex::one("shared@example.test", &["DEVELOPER"]);
        let peers = vec![peer(
            "p1",
            "peer-maintainer-smith-zagros-2026-09-01-laptop",
            &["ZAGROS-DEVELOPER"],
            true,
            "",
        )];

        assert!(preflight(
            PreflightCase {
                from: "ZAGROS-DEVELOPER",
                to: "ZAGROS-QUARANTINED",
                strip_others: false,
                ory_plane: false,
                marker: None,
                selector_emails: &[],
            },
            &peers,
            &people,
            Ok(&idx),
        )
        .is_empty());
    }

    #[test]
    fn preflight_handles_doubled_legacy_owner() {
        let people = BTreeMap::from([(
            "user371x".to_string(),
            person("jonas@example.test", &["ZAGROS-DEVELOPER"]),
        )]);
        let idx = crate::cmd::warrant::OryPrivilegeIndex::one("jonas@example.test", &["DEVELOPER"]);
        let peers = vec![peer(
            "p1",
            "peer-user371x-user371x-zagros-2026-06-16-JL-2.local",
            &["ZAGROS-DEVELOPER"],
            true,
            "",
        )];

        assert!(preflight(
            PreflightCase {
                from: "ZAGROS-DEVELOPER",
                to: "ZAGROS-QUARANTINED",
                strip_others: false,
                ory_plane: false,
                marker: None,
                selector_emails: &[],
            },
            &peers,
            &people,
            Ok(&idx),
        )
        .is_empty());
    }

    #[test]
    fn manifest_owner_with_privileged_key_baseline_cannot_leave_quarantine() {
        let people = BTreeMap::from([("alice".to_string(), person("alice@example.test", &[]))]);
        let baselines = BTreeMap::from([(
            "p1".to_string(),
            crate::cmd::warrant::Baseline {
                source: crate::cmd::warrant::BaselineSource::Key,
                groups: BTreeSet::from(["ZAGROS-PLATFORM-ADMIN".to_string()]),
            },
        )]);
        let idx = crate::cmd::warrant::OryPrivilegeIndex::one(
            "alice@example.test",
            &["DEVELOPER"],
        );
        let peers = vec![peer(
            "p1",
            "peer-alice-zagros-2026-09-01-laptop",
            &["ZAGROS-QUARANTINED"],
            true,
            "",
        )];

        let refusals = preflight_with_baselines(
            PreflightCase {
                from: "ZAGROS-QUARANTINED",
                to: "ZAGROS-DEVELOPER",
                strip_others: false,
                ory_plane: false,
                marker: None,
                selector_emails: &[],
            },
            &peers,
            &people,
            &baselines,
            Ok(&idx),
        );

        assert_eq!(refusals.len(), 1);
        assert!(refusals[0].starts_with("peer-alice-zagros-2026-09-01-laptop: "));
        assert!(refusals[0].contains("the subject is a privileged person"));
    }

    #[test]
    fn manifest_owner_with_ordinary_key_baseline_can_leave_quarantine() {
        let people = BTreeMap::from([("alice".to_string(), person("alice@example.test", &[]))]);
        let baselines = BTreeMap::from([(
            "p1".to_string(),
            crate::cmd::warrant::Baseline {
                source: crate::cmd::warrant::BaselineSource::Key,
                groups: BTreeSet::from(["ZAGROS-DEVELOPER".to_string()]),
            },
        )]);
        let idx = crate::cmd::warrant::OryPrivilegeIndex::one(
            "alice@example.test",
            &["DEVELOPER"],
        );
        let peers = vec![peer(
            "p1",
            "peer-alice-zagros-2026-09-01-laptop",
            &["ZAGROS-QUARANTINED"],
            true,
            "",
        )];

        assert!(preflight_with_baselines(
            PreflightCase {
                from: "ZAGROS-QUARANTINED",
                to: "ZAGROS-DEVELOPER",
                strip_others: false,
                ory_plane: false,
                marker: None,
                selector_emails: &[],
            },
            &peers,
            &people,
            &baselines,
            Ok(&idx),
        )
        .is_empty());
    }

    #[test]
    fn manifest_only_owner_keeps_the_setup_key_baseline() {
        let manifest_people =
            BTreeMap::from([("alice".to_string(), person("alice@example.test", &[]))]);
        let live_peers = vec![crate::cmd::warrant::LivePeer {
            id: "p1".to_string(),
            name: "peer-alice-zagros-2026-09-01-laptop".to_string(),
            groups: vec!["ZAGROS-QUARANTINED".to_string()],
        }];
        let joins = BTreeMap::from([(
            "p1".to_string(),
            crate::cmd::warrant::KeyJoin {
                key_name: Some("alice-device".to_string()),
                groups: Some(BTreeSet::from(["ZAGROS-PLATFORM-ADMIN".to_string()])),
            },
        )]);

        let (people, baselines) =
            move_people_and_baselines(None, manifest_people, "zagros", &live_peers, &joins);

        assert!(people.contains_key("alice"));
        assert_eq!(
            baselines.get("p1").map(|baseline| baseline.source),
            Some(crate::cmd::warrant::BaselineSource::Key)
        );
    }

    fn profile_with(body: &str) -> (tempfile::TempDir, crate::profile::Profile) {
        let temp = tempfile::tempdir().expect("tempdir");
        let context = temp.path().join("contexts").join("default");
        std::fs::create_dir_all(&context).expect("mkdir");
        std::fs::write(context.join("profile.toml"), body).expect("write");
        let profile = crate::profile::Profile {
            dir: temp.path().to_path_buf(),
            context: "default".to_string(),
        };
        (temp, profile)
    }

    #[test]
    fn move_ory_config_uses_the_profile_override_for_check_and_write() {
        let (_temp, profile) = profile_with(
            r#"
ory_auth_url = "https://auth.override.example"
ory_ssh_host = "100.127.0.44"
ory_ssh_user = "operator"
"#,
        );

        let cfg = resolve_move_ory_config("zagros", &profile, true)
            .unwrap()
            .expect("required config");

        assert_eq!(cfg.ssh_host, "100.127.0.44");
    }

    #[test]
    fn move_ory_config_is_required_only_when_a_leg_needs_it() {
        let (_temp, profile) = profile_with("portal_endpoint = \"http://portal:8090\"\n");

        assert!(resolve_move_ory_config("unknown-goatnet", &profile, true).is_err());
        assert!(resolve_move_ory_config("unknown-goatnet", &profile, false)
            .unwrap()
            .is_none());
    }

    #[test]
    fn identity_only_quarantine_checks_setup_key_baseline_without_mesh_mover() {
        let people = BTreeMap::from([("alice".to_string(), person("alice@example.test", &[]))]);
        let baselines = BTreeMap::from([(
            "p1".to_string(),
            crate::cmd::warrant::Baseline {
                source: crate::cmd::warrant::BaselineSource::Key,
                groups: BTreeSet::from(["ZAGROS-PLATFORM-ADMIN".to_string()]),
            },
        )]);
        let idx =
            crate::cmd::warrant::OryPrivilegeIndex::one("alice@example.test", &["DEVELOPER"]);
        let peers = vec![peer(
            "p1",
            "peer-alice-zagros-2026-09-01-laptop",
            &["ZAGROS-DEVELOPER"],
            true,
            "",
        )];
        let emails = vec!["alice@example.test".to_string()];
        assert!(build_plan(
            &peers,
            "ZAGROS-QUARANTINED",
            "ZAGROS-DEVELOPER",
            &Selection::Bulk,
            now(),
        )
        .movers
        .is_empty());

        let refusals = preflight_with_baselines(
            PreflightCase {
                from: "ZAGROS-QUARANTINED",
                to: "ZAGROS-DEVELOPER",
                strip_others: false,
                ory_plane: true,
                marker: None,
                selector_emails: &emails,
            },
            &peers,
            &people,
            &baselines,
            Ok(&idx),
        );

        assert_eq!(refusals.len(), 1);
        assert!(refusals[0].contains("the subject is a privileged person"));
    }

    #[test]
    fn preflight_catches_ory_only_marker_and_strip_others_transitions() {
        let people = BTreeMap::from([(
            "alice".to_string(),
            person("alice@example.test", &["ZAGROS-DEVELOPER"]),
        )]);
        let idx = crate::cmd::warrant::OryPrivilegeIndex::one("alice@example.test", &["IAM-ADMIN"]);
        let emails = vec!["alice@example.test".to_string()];
        let refusals = preflight(
            PreflightCase {
                from: "ZAGROS-DEVELOPER",
                to: "ZAGROS-OBSERVER",
                strip_others: false,
                ory_plane: true,
                marker: Some("quarantine"),
                selector_emails: &emails,
            },
            &[],
            &people,
            Ok(&idx),
        );
        assert!(!refusals.is_empty());
        assert!(refusals.iter().all(|r| !r.contains("alice@example.test")));

        let peers = vec![peer(
            "p1",
            "peer-alice-zagros-2026-09-01-laptop",
            &["ZAGROS-DEVELOPER", "ZAGROS-QUARANTINED"],
            true,
            "",
        )];
        assert!(!preflight(
            PreflightCase {
                from: "ZAGROS-DEVELOPER",
                to: "ZAGROS-OBSERVER",
                strip_others: true,
                ory_plane: false,
                marker: None,
                selector_emails: &[],
            },
            &peers,
            &people,
            Ok(&idx),
        )
        .is_empty());
    }

    #[test]
    fn move_form_expresses_each_move() {
        use crate::cmd::warrant::EntryForm;
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<BTreeSet<_>>();
        assert_eq!(
            move_form(
                "ZAGROS-DEV",
                "ZAGROS-DEV-DEVELOPER",
                "ZAGROS-DEV-QUARANTINED",
                true
            )
            .unwrap(),
            EntryForm::Hold
        );
        assert_eq!(
            move_form("ZAGROS-DEV", "ZAGROS-DEV-DEVELOPER", "ZAGROS-DEV-OBSERVER", false).unwrap(),
            EntryForm::GrantAbsent {
                grant: s(&["ZAGROS-DEV-OBSERVER"]),
                absent: s(&["ZAGROS-DEV-DEVELOPER"])
            }
        );
        assert_eq!(
            move_form(
                "ZAGROS-DEV",
                "ZAGROS-DEV-DEVELOPER",
                "ZAGROS-DEV-EXTERNAL-INFRA",
                false
            )
            .unwrap(),
            EntryForm::Exact(s(&["ZAGROS-DEV-EXTERNAL-INFRA"]))
        );
        assert_eq!(
            move_form("ZAGROS-DEV", "ZAGROS-DEV-DEVELOPER", "ZAGROS-DEV-OBSERVER", true).unwrap(),
            EntryForm::Exact(s(&["ZAGROS-DEV-OBSERVER"]))
        );
        // Quarantine without --strip-others is an ordinary grant/absent.
        assert!(matches!(
            move_form(
                "ZAGROS-DEV",
                "ZAGROS-DEV-DEVELOPER",
                "ZAGROS-DEV-QUARANTINED",
                false
            )
            .unwrap(),
            EntryForm::GrantAbsent { .. }
        ));
        // An unmanaged source is not listed as absent.
        assert_eq!(
            move_form("ZAGROS-DEV", "All", "ZAGROS-DEV-OBSERVER", false).unwrap(),
            EntryForm::GrantAbsent {
                grant: s(&["ZAGROS-DEV-OBSERVER"]),
                absent: BTreeSet::new()
            }
        );
        assert!(move_form("ZAGROS-DEV", "ZAGROS-DEV-DEVELOPER", "ZAGROS-OBSERVER", false).is_err());
        assert_eq!(
            default_expires(&EntryForm::GrantAbsent {
                grant: s(&["ZAGROS-DEV-OBSERVER"]),
                absent: BTreeSet::new()
            }),
            None
        );
        assert!(default_expires(&EntryForm::Hold).is_some());
        assert!(default_expires(&EntryForm::Exact(s(&["ZAGROS-DEV-EXTERNAL-INFRA"]))).is_some());
    }

    #[test]
    fn identity_patch_swaps_dedups_and_markers() {
        // Swap ADMINS→PLATFORM-ADMIN where PLATFORM-ADMIN already present: dedup.
        let cur = vec!["ADMINS".to_string(), "PLATFORM-ADMIN".to_string()];
        let migrated: serde_json::Map<String, serde_json::Value> =
            serde_json::from_value(serde_json::json!({"groups": cur})).unwrap();
        let ops = identity_patch_ops(&cur, Some(&migrated), Some("ADMINS"), Some("PLATFORM-ADMIN"), None, None, false)
            .unwrap();
        let arr = ops.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["path"], "/metadata_public/groups");
        assert_eq!(arr[0]["value"], serde_json::json!(["PLATFORM-ADMIN"]));

        // No-op when from absent and marker already in desired state.
        assert!(identity_patch_ops(&cur, Some(&migrated), Some("NOPE"), Some("X"), None, None, false).is_none());
        assert!(identity_patch_ops(&cur, Some(&migrated), None, None, Some("quarantine"), None, true).is_none());
        assert!(identity_patch_ops(&cur, Some(&migrated), None, None, None, Some("quarantine"), false).is_none());

        // Marker set + clear produce the right ops.
        let set = identity_patch_ops(&cur, Some(&migrated), None, None, Some("quarantine"), None, false).unwrap();
        assert_eq!(set.as_array().unwrap()[0]["op"], "add");
        let clear = identity_patch_ops(&cur, Some(&migrated), None, None, None, Some("probation"), true).unwrap();
        assert_eq!(clear.as_array().unwrap()[0]["op"], "remove");
    }

    #[test]
    fn identity_patch_adds_metadata_object_for_unmigrated_identity() {
        // F-572 / ADR 1188: an identity the backfill has not reached has no
        // metadata_public.groups, so a replace there would 4xx. The swap adds
        // the whole object instead, keeping any other admin-managed key.
        let cur = vec!["ADMINS".to_string()];
        let meta: serde_json::Map<String, serde_json::Value> =
            serde_json::from_value(serde_json::json!({"org": {"short_tag": "acme"}})).unwrap();
        for m in [None, Some(&meta)] {
            let ops = identity_patch_ops(&cur, m, Some("ADMINS"), Some("PLATFORM-ADMIN"), None, None, false)
                .unwrap();
            let op = &ops.as_array().unwrap()[0];
            assert_eq!(op["op"], "add");
            assert_eq!(op["path"], "/metadata_public");
            assert_eq!(op["value"]["groups"], serde_json::json!(["PLATFORM-ADMIN"]));
            if m.is_some() {
                assert_eq!(op["value"]["org"]["short_tag"], "acme");
            }
        }
    }
}
