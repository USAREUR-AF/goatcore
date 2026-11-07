//! `join-node`: one more machine of a class at a live site, declared →
//! converged → verified. This verb is the node-addition sequence (the
//! executed order, the joiner's three passes, "done means proven") as
//! numbered phases with their gates, with each step's trap in the phase text;
//! docs/tutorials/02-first-federated-node.md walks through it. `--dry-run` prints every phase and
//! command and runs nothing; `--from`/`--to` resume; every phase is idempotent.
//!
//!   1  MANUAL reach the box on the SITE LAN (not the USB bridge address)
//!   2  T      seed-ssh-key.sh — key auth + NOPASSWD sudo, password from an env var
//!   3  GATE   repo work done first: standup-config-preflight.py passes
//!   4  T      quiet apt on the node (mask the daily timers; unmasked at 14)
//!   5  A      mgmt-host-baseline.yml, narrowed to the node
//!   6  A      wireguard-kmod.yml, first pass — its last check fails before
//!             enrolment (wt0 absent); expected
//!   7  GATE   the goatnet CA this site trusts is the site's own, not the shared dev root
//!   8  MANUAL mint a setup key carrying the group the site's other nodes hold
//!   9  GATE   preflight-enrollment.sh on the node (clock, stale wt0, apex reach)
//!  10  MANUAL netbird up with the goatnet's declared PQC posture; the gate is
//!             `table ip netbird`, not "Connected"
//!  11  A      wireguard-kmod.yml again — changed=0 proves the kmod took
//!  12  GATE   mesh_ip + the mesh inventory filled: preflight passes with no TBD_
//!  13  A      the class's converge: core (founder pass, or a joiner's three
//!             passes + the founder-side mirror), obs (the three obs legs),
//!             breakglass, network-jump
//!  14  T      unmask the apt timers
//!  15  GATE   acceptance: every applicable row PASS, from the new node's context
//!
//! Status: built from the runbook; unmeasured on hardware (the runbook's own
//! run of is the record it transcribes).

use std::path::Path;
use std::process::Command;

use anyhow::{anyhow, Context, Result};

use crate::config;
use crate::{Check, Report, EXIT_OK, EXIT_REFUSED};

pub struct Options<'a> {
    pub env: &'a str,
    pub site: &'a str,
    pub node: &'a str,
    pub class: &'a str,
    pub address: Option<&'a str>,
    pub pass_var: Option<&'a str>,
    pub from: u32,
    pub to: u32,
    pub dry_run: bool,
}

pub const CLASSES: [&str; 4] = ["core", "obs", "breakglass", "network-jump"];

struct Phase {
    n: u32,
    kind: &'static str,
    label: String,
    cmd: String,
}

fn row(name: &str, detail: String) -> Check {
    Check { name: name.into(), ok: true, detail, fix: None }
}

/// The inventory group a node of `class` sits in at a site: the core group for
/// `core`, `<site>_<class>_hosts` otherwise (obs, breakglass, network-jump).
fn class_group(site_us: &str, class: &str) -> String {
    match class {
        "core" => format!("{site_us}_hosts"),
        other => format!("{site_us}_{}_hosts", other.replace('-', "_")),
    }
}

fn gv_matches(file: &Path, key: &str, value: &str) -> bool {
    std::fs::read_to_string(file)
        .map(|t| t.lines().any(|l| l.trim_start().strip_prefix(key).and_then(|r| r.strip_prefix(':')).map(|v| v.trim_start().starts_with(value)).unwrap_or(false)))
        .unwrap_or(false)
}

fn gv_first(file: &Path, key: &str) -> String {
    std::fs::read_to_string(file)
        .ok()
        .and_then(|t| t.lines().find_map(|l| l.trim_start().strip_prefix(key).and_then(|r| r.strip_prefix(':')).map(|v| v.split_whitespace().next().unwrap_or("").replace(['"', '\''], ""))))
        .unwrap_or_default()
}

fn plan(repo: &Path, o: &Options) -> Result<Vec<Phase>> {
    let r = repo.display().to_string();
    let site = o.site;
    let site_us = site.replace('-', "_");
    let upper = site_us.to_uppercase();
    // The site's core group carries the site-level facts (founder or joiner,
    // the founder it joins); the node's own group is its class's group, and the
    // gates and plays of phases 3-12 run against that one. A class whose node
    // sits in another group would otherwise match no host (`-e target=` ∩ `-l`
    // is empty) and the play would pass having done nothing.
    let site_group = format!("{site_us}_hosts");
    let group = class_group(&site_us, o.class);
    // The config preflight judges a site from its core group; an obs node's
    // facts are checked through `--obs <group>` (the mgmt-stack rows do not
    // apply to an obs host), and a rescue node's site is its core group's.
    let preflight_target = match o.class {
        "core" => group.clone(),
        "obs" => format!("{site_group} --obs {group}"),
        _ => site_group.clone(),
    };
    let node = o.node;
    let aws = crate::settings::aws_profile(repo);
    let user = crate::settings::ssh_user(repo).unwrap_or_else(|| "<deploy.ssh_user>".to_string());
    let pre = format!("OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES ../scripts/with-aws {aws} -- ../scripts/ansible-preflight.sh");
    let inv = "-i inventory/mgmt-hosts.ini -i inventory/mgmt-hosts-mesh.ini";
    let addr = o.address.unwrap_or("<the node's LAN address>");
    let pass_var = o.pass_var.map(String::from).unwrap_or_else(|| format!("{upper}_NODE_PASS"));
    let gv = repo.join("ops/ansible/group_vars").join(&site_group).join("vars.yml");
    let joiner = gv_matches(&gv, "goatnet_founder", "false");
    let founder_site = gv_first(&gv, "join_founder_site");
    let founder_group = if founder_site.is_empty() { String::new() } else { format!("{}_hosts", founder_site.replace('-', "_")) };
    let intra_pair = gv_matches(&gv, "intra_site_pair", "true");
    let p = |n: u32, kind: &'static str, label: &str, cmd: String| Phase { n, kind, label: label.to_string(), cmd };

    // A joiner enrols into the goatnet it joins: the founder's control plane.
    let enrol_at = if joiner && !founder_site.is_empty() { founder_site.as_str() } else { site };
    // The enrolment apex the preflight checks: the goatnet's, declared in the
    // site's core group (a joiner's names the goatnet it joins).
    let enrol_dns = { let d = gv_first(&gv, "site_enrollment_dns"); if d.is_empty() { format!("<site_enrollment_dns of {site_group}>") } else { d } };
    let mut ev = vec![
        p(1, "MANUAL", "reach the box on the SITE LAN", format!("ssh {user}@{addr} — find the box's real LAN address before anything else and pass it as --address. On a Jetson the address handed over may be 192.168.55.1: that is l4tbr0, the USB device-mode bridge on the box itself, reachable only over a direct USB-C cable.")),
        p(2, "T", "seed the SSH key (key auth + NOPASSWD sudo; password from the env var, never the command line)", format!("'{r}/ops/scripts/seed-ssh-key.sh' {user}@{addr} {pass_var}")),
        p(3, "GATE", "repo work first: group, inventory, pins, exclusions — the config preflight passes", format!("python3 '{r}/ops/scripts/standup-config-preflight.py' {preflight_target}")),
        p(4, "T", "quiet apt on the node (the baseline stalls on the dpkg lock otherwise; unmasked in phase 14)", format!("ssh {user}@{addr} 'sudo systemctl mask --now apt-daily.timer apt-daily-upgrade.timer'")),
        p(5, "A", "host baseline, narrowed to the node — run detached if the session may drop: it apt-upgrades and reboot-waves", format!("cd '{r}/ops/ansible' && {pre} {inv} mgmt-host-baseline.yml -e target={group} -e site_id={site} -l {node}")),
        p(6, "A", "WireGuard kernel modules, first pass — the final check fails before enrolment (wt0 absent); expected, phase 11 re-runs it", format!("cd '{r}/ops/ansible' && ( {pre} {inv} wireguard-kmod.yml -e target={group} -l {node} || echo '>> wireguard-kmod: final check failed before enrolment (wt0 absent) — expected; phase 11 re-runs it for changed=0' )")),
        p(7, "GATE", "the CA this site trusts is its own, not the shared dev root (silence puts a production node on the dev CA)", format!("python3 '{r}/ops/scripts/standup-config-preflight.py' {preflight_target} && ! grep -rqE 'ca_id:[[:space:]]*\"?[a-z]+-goat-ca-' '{r}/ops/ansible/group_vars/{group}/' || {{ echo 'group_vars/{group} names the unnamed default CA (<mode>-goat-ca-<date>); a site node trusts its goatnet CA'; exit 1; }}")),
        p(8, "MANUAL", "mint a setup key carrying the group the site's other mgmt hosts hold — read it off a live peer, do not guess", format!("in the NetBird dashboard of {enrol_at} (or `goat admin` on it): Setup Keys → new key, auto_groups = the group a live mgmt host of {enrol_at} holds; keep it for phase 10")),
        p(9, "GATE", "enrollment preflight on the node (clock skew, a stale wt0, an unreachable apex — each turns enrolment into a hang)", format!("{{ printf 'export NETBIRD_SETUP_KEY=%s NETBIRD_MANAGEMENT_URL=%s\\n' \"$(printenv {upper}_NB_SETUP_KEY)\" 'https://{enrol_dns}'; cat '{r}/ops/preflight-enrollment.sh'; }} | ssh {user}@{addr} 'bash -s'")),
        p(10, "MANUAL", "enrol the node — the gate is the kernel ACL plane, not \"Connected\"", "on the node: sudo netbird up <the line phase 9 printed> --enable-rosenpass=false (match the goatnet's declared PQC posture; never also pass --rosenpass-permissive=false). Gate: `netbird status` shows Interface type: Kernel AND `sudo nft list tables` shows `table ip netbird` — Connected alone is an unfiltered node".to_string()),
        p(11, "A", "WireGuard kernel modules again — changed=0 proves the kmod took, now that wt0 exists", format!("cd '{r}/ops/ansible' && {pre} {inv} wireguard-kmod.yml -e target={group} -l {node}")),
        p(12, "GATE", "mesh_ip + the mesh inventory filled in one change (mesh IPs are not stable across re-enrolment) — preflight passes with no TBD_", format!("python3 '{r}/ops/scripts/standup-config-preflight.py' {preflight_target} && ! grep -rqn 'TBD_' '{r}/ops/ansible/group_vars/{group}/' && ! grep -n 'TBD_' '{r}/ops/ansible/inventory/mgmt-hosts-mesh.ini' | grep -qw {node}")),
    ];
    let ev_secrets = format!("-e bootstrap_owner_email=\"$OWNER_EMAIL\" -e 'postgres_superuser_password={{{{ lookup(\"env\", \"{upper}_PG_PASS\") }}}}' -e 'netbird_encryption_key={{{{ lookup(\"env\", \"{upper}_NB_ENCKEY\") }}}}' -e 'pgbackrest_cipher_pass={{{{ lookup(\"env\", \"{upper}_BKP_KEY\") }}}}'");
    let ev_bootstrap = format!("-e site_id={site} {ev_secrets}");
    match o.class {
        "core" if joiner => {
            if founder_site.is_empty() {
                return Err(anyhow!("group_vars/{group}/vars.yml declares goatnet_founder: false and no join_founder_site — a joiner must name the founder it seeds from"));
            }
            // site-seed-from-peer.yml is the generic play; a site that keeps its
            // own <site>-seed-from-peer.yml gets that one instead.
            let seed_play = crate::standup::site_play(repo, site, "seed-from-peer.yml");
            // site_id reaches the joiner's passes as a PLAY variable, never as
            // `-e site_id`: an extra var sets every host's site_id to this site,
            // and the Spock roles' inventory walk then finds no peer.
            let bs = crate::standup::bootstrap_play(repo, site);
            let p1 = if intra_pair { format!("{ev_secrets} -e participates_in_spock=false") } else { format!("{ev_secrets} -e participates_in_spock=false -e has_durable_independent_state=true") };
            ev.push(p(13, "A", "[core, joiner] pass 1 — bootstrap without Spock (AutoMigrate creates the schema once, on the empty DB; Postgres initialises with the founder's encoding)", format!("cd '{r}/ops/ansible' && {pre} {inv} -l {node} {bs} {p1}")));
            ev.push(p(13, "A", "[core, joiner] the service deploys the seed needs (kratos/hydra schemas): ory", format!("cd '{r}/ops/ansible' && {pre} {inv} ory-deploy.yml --limit {node} -e 'postgres_superuser_password={{{{ lookup(\"env\", \"{upper}_PG_PASS\") }}}}'")));
            ev.push(p(13, "A", &format!("[core, joiner] pass 2 — seed-from-peer: data-only copy from founder {founder_site} (fails loud if the account is populated)"), format!("cd '{r}/ops/ansible' && {pre} {inv} -l {node} {seed_play} -e site_id={site}")));
            // Ory (Phase 7D) subscribes each side only to a peer that already has
            // its ory_narrow node, and its history reconcile checks the reverse
            // subscription on the peer, so the three steps below are staged:
            // joiner node+repset, founder node+repset+subscription, joiner
            // subscription+reconcile (which copies history both ways).
            ev.push(p(13, "A", "[core, joiner] pass 3 — bootstrap with Spock: netbird subscriptions on both sides, polled to replicating; Ory node + repset only", format!("cd '{r}/ops/ansible' && {pre} {inv} -l {node} {bs} {ev_secrets} -e ory_spock_wire_subscriptions=false")));
            ev.push(p(13, "A", &format!("[core, joiner] the founder-side mirror on {founder_group}, peer={site} only (the same change, not a follow-up): netbird + Ory node, repset, subscription"), format!("cd '{r}/ops/ansible' && {pre} {inv} spock-mesh-wire.yml --limit {founder_group} -e '{{\"inter_site_spock_peers\":[\"{site}\"],\"ory_spock_reconcile_history\":false}}'")));
            ev.push(p(13, "A", "[core, joiner] the joiner's Spock wiring again, now that the founder's half exists: Ory subscription + history reconcile", format!("cd '{r}/ops/ansible' && {pre} {inv} spock-mesh-wire.yml --limit {node}")));
        }
        "core" => {
            ev.push(p(13, "A", &format!("[core, founder site] site-bootstrap on the node{}", if intra_pair { " (intra_site_pair: the A→B streaming standby is phase 6A of the play)" } else { "" }), format!("cd '{r}/ops/ansible' && {pre} {inv} -l {node} site-bootstrap.yml {ev_bootstrap}")));
        }
        "obs" => {
            if let Some(missing) = crate::standup::obs_play_missing(repo) {
                return Err(anyhow!("{missing}"));
            }
            let og = &group;
            ev.push(p(13, "A", "[obs] observability host: baseline + LUKS (own nodes only)", format!("cd '{r}/ops/ansible' && {pre} {inv} mgmt-host-baseline.yml -e target={og} -e site_id={site} -l {node} && {pre} {inv} tang-server.yml -e target={og} -l {node} && {pre} {inv} mgmt-host-luks.yml -e target={og} -e site_id={site} -l {node}")));
            ev.push(p(13, "A", "[obs] observability host: utility (Forgejo, the pub/sub state-of-record primary; docker data-root on the NVMe first, F-504)", format!("cd '{r}/ops/ansible' && {pre} {inv} utility-host-bootstrap.yml -e target={og} -l {node}")));
            ev.push(p(13, "A", "[obs] observability host: stack (Prometheus/Alertmanager/Grafana) — needs obs_scrape_targets", format!("cd '{r}/ops/ansible' && {pre} {inv} obs-host-bootstrap.yml -e target={og} -l {node}")));
        }
        "breakglass" => {
            let rg = format!("{site_us}_breakglass_hosts");
            let extra = format!("inventory/{site}-breakglass-nodes.ini");
            let inv_extra = if repo.join("ops/ansible").join(&extra).is_file() { format!(" -i {extra}") } else { String::new() };
            ev.push(p(13, "A", "[breakglass] break-glass node (ADR 1114 — the way in when mesh + control plane are both down)", format!("cd '{r}/ops/ansible' && {pre} {inv}{inv_extra} breakglass-nodes.yml -e target={rg} -l {node}")));
        }
        "network-jump" => {
            let rg = format!("{site_us}_network_jump_hosts");
            let extra = format!("inventory/{site}-network-jump.ini");
            let inv_extra = if repo.join("ops/ansible").join(&extra).is_file() { format!(" -i {extra}") } else { String::new() };
            ev.push(p(13, "A", "[network-jump] network-jump host (ADR 1131 — reach to switch, router and out-of-band gear; herd-connector has no public image: set herd_connector_image in group_vars to an image you supply, or the play refuses)", format!("cd '{r}/ops/ansible' && {pre} {inv}{inv_extra} herd-connector-deploy.yml -e herd_connector_target_hosts={rg} -e target={rg} -l {node}")));
        }
        other => return Err(anyhow!("class '{other}' is not one this verb converges; known: {}", CLASSES.join(", "))),
    }
    ev.push(p(14, "T", "unmask the apt timers now that the node is finished", format!("ssh {user}@{addr} 'sudo systemctl unmask apt-daily.timer apt-daily-upgrade.timer'")));
    ev.push(p(15, "GATE", "done means proven: every applicable acceptance row PASS, never UNKNOWN (a joiner adds the replicating-both-ways evidence)", format!("python3 '{r}/ops/validation/generate-values.py' {site} && python3 '{r}/ops/validation/run-acceptance.py' --env {site} --values '{r}/ops/validation/values-{site}.yaml' --all-eligible --by-layer")));
    Ok(ev)
}

pub fn join_node(repo: &Path, o: &Options) -> Result<Report> {
    let cfg = config::load(&repo.join("ops/goatbuilder/goatbuilder.yaml"))?;
    let Some(env) = cfg.environments.iter().find(|e| e.name == o.env) else {
        return Err(anyhow!("unknown environment \"{}\"; declared: {:?}", o.env, cfg.environments.iter().map(|e| e.name.as_str()).collect::<Vec<_>>()));
    };
    if !env.sites.is_empty() {
        let Some(site) = env.sites.iter().find(|s| s.name == o.site) else {
            return Err(anyhow!("site '{}' is not declared for environment '{}' (declared: {})", o.site, o.env, env.sites.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", ")));
        };
        if !site.classes.is_empty() && !site.classes.contains_key(o.class) {
            return Err(anyhow!("class '{}' is not declared at site '{}' (declared: {})", o.class, o.site, site.classes.keys().cloned().collect::<Vec<_>>().join(", ")));
        }
    }
    if !CLASSES.contains(&o.class) {
        return Err(anyhow!("class '{}' is not one this verb converges; known: {}", o.class, CLASSES.join(", ")));
    }
    if !o.dry_run && o.address.is_none() {
        return Err(anyhow!("--address <the node's LAN address> is required to run (phase 1 says how to find it); --dry-run plans without it"));
    }
    if !o.dry_run && crate::settings::ssh_user(repo).is_none() {
        return Err(anyhow!("this checkout declares no SSH user for a new machine: set deploy.ssh_user in ops/goatbuilder/goatbuilder.yaml; --dry-run plans without it"));
    }
    let phases = plan(repo, o)?;
    let resume = format!("goatbuilder join-node {} --site {} --node {} --class {}{}", o.env, o.site, o.node, o.class, o.address.map(|a| format!(" --address {a}")).unwrap_or_default());
    let mut checks = vec![row("banner", format!("join-node: site={}  node={}  class={}  {}", o.site, o.node, o.class, if o.dry_run { "(DRY RUN — nothing executes)" } else { "" }))];
    let mut exit = EXIT_OK;
    for ph in &phases {
        if ph.n < o.from || ph.n > o.to {
            continue;
        }
        checks.push(row("phase", format!("=== Phase {} [{}] {} ===", ph.n, ph.kind, ph.label)));
        checks.push(row("cmd", ph.cmd.clone()));
        if o.dry_run {
            continue;
        }
        if ph.kind == "MANUAL" {
            checks.push(row("note", format!("^ OPERATOR STEP — do the above, then re-run with --from {}", ph.n + 1)));
            break;
        }
        let status = Command::new("bash").args(["-euo", "pipefail", "-c", &ph.cmd]).current_dir(repo).status().context("bash")?;
        if !status.success() {
            let rc = status.code().unwrap_or(1);
            checks.push(Check { name: "failed".into(), ok: false, detail: format!("!! Phase {} (\"{}\") FAILED — exit {rc}. No later phase has run; the node is left partway through phase {}. `--dry-run` prints this phase's command and the gate that proves it.", ph.n, ph.label, ph.n), fix: Some(format!("fix the cause, then resume — every phase is idempotent: {resume} --from {}", ph.n)) });
            exit = EXIT_REFUSED;
            break;
        }
    }
    if o.dry_run {
        checks.push(row("footer", "PLAN complete. Remove --dry-run to execute (needs --address). Manual steps (1, 8, 10) pause the run; resume with --from <next>. docs/tutorials/02-first-federated-node.md walks through it.".into()));
    }
    let mut r = Report::new("join-node", checks, EXIT_REFUSED);
    r.exit = exit;
    r.ok = exit == EXIT_OK;
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// A project that declares no SSH user plans with a placeholder and cannot run.
    #[test]
    fn the_ssh_user_comes_from_the_checkout() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        let plan_text = |o: &Options, r: &Path| plan(r, o).unwrap().iter().map(|p| p.cmd.clone()).collect::<Vec<_>>().join("\n");
        let o = Options { address: Some("192.0.2.9"), ..opts("alpha", "core", "alpha") };
        assert!(plan_text(&o, &repo()).contains(&format!("{}@192.0.2.9", crate::settings::ssh_user(&repo()).unwrap())));
        let t = tempfile::tempdir().unwrap();
        let text = plan_text(&o, t.path());
        assert!(text.contains("<deploy.ssh_user>@192.0.2.9"), "{text}");
    }

    fn opts<'a>(site: &'a str, class: &'a str, env: &'a str) -> Options<'a> {
        Options { env, site, node: "node-x", class, address: None, pass_var: None, from: 0, to: 99, dry_run: true }
    }

    /// A joiner site's core node gets the three passes and the founder-side
    /// mirror, in that order, after the twelve host phases; phase 15 is last.
    #[test]
    fn a_joiner_core_node_runs_the_three_passes_and_the_mirror() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        let r = join_node(&repo(), &opts("uk", "core", "zagros")).unwrap();
        assert_eq!(r.exit, EXIT_OK);
        let phases: Vec<&str> = r.checks.iter().filter(|c| c.name == "phase").map(|c| c.detail.as_str()).collect();
        assert!(phases[0].starts_with("=== Phase 1 [MANUAL] reach the box"));
        let core: Vec<&&str> = phases.iter().filter(|p| p.contains("[core, joiner]")).collect();
        assert_eq!(core.len(), 6, "{phases:?}");
        assert!(core[0].contains("pass 1") && core[2].contains("pass 2") && core[3].contains("pass 3") && core[4].contains("founder-side mirror on zagros_hosts") && core[5].contains("Spock wiring again"));
        assert!(phases.last().unwrap().starts_with("=== Phase 15 [GATE] done means proven"));
        let cmds: Vec<&str> = r.checks.iter().filter(|c| c.name == "cmd").map(|c| c.detail.as_str()).collect();
        assert!(cmds.iter().any(|c| c.contains("-e participates_in_spock=false")));
    }

    /// A checkout with one founder and one joiner site, for the plan tests
    /// that must also run in a public clone.
    fn joiner_checkout() -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        let gv = t.path().join("ops/ansible/group_vars");
        std::fs::create_dir_all(gv.join("zagros_b_hosts")).unwrap();
        std::fs::write(gv.join("zagros_b_hosts/vars.yml"), "goatnet_founder: false\njoin_founder_site: zagros\nsite_enrollment_dns: netbird.example.net\n").unwrap();
        t
    }

    /// The joiner's bootstrap passes carry site_id as a play variable (the
    /// generic wrapper, or the site's own wrapper when it keeps one), never as
    /// `-e site_id`, which would reach every host and leave the Spock roles
    /// with no peer. Ory is staged across both sides in the order that lets
    /// each step pass: joiner node + repset, founder subscription, joiner
    /// subscription + reconcile.
    #[test]
    fn a_joiner_bootstraps_with_a_play_var_and_stages_ory() {
        let t = joiner_checkout();
        let o = opts("zagros-b", "core", "zagros");
        let ph = plan(t.path(), &o).unwrap();
        let core: Vec<&Phase> = ph.iter().filter(|p| p.label.contains("[core, joiner]")).collect();
        assert_eq!(core.len(), 6, "{:?}", core.iter().map(|p| &p.label).collect::<Vec<_>>());
        for i in [0, 3] {
            assert!(core[i].cmd.contains("-l node-x site-bootstrap-for.yml -e bootstrap_site=zagros-b "), "{}", core[i].cmd);
            assert!(!core[i].cmd.contains("-e site_id="), "{}", core[i].cmd);
            assert!(core[i].cmd.contains("_PG_PASS\")"), "secrets still reach the play: {}", core[i].cmd);
        }
        assert!(core[0].cmd.contains("-e participates_in_spock=false"), "{}", core[0].cmd);
        assert!(core[3].cmd.contains("-e ory_spock_wire_subscriptions=false"), "{}", core[3].cmd);
        assert!(core[4].cmd.contains("spock-mesh-wire.yml --limit zagros_hosts") && core[4].cmd.contains("\"inter_site_spock_peers\":[\"zagros-b\"]") && core[4].cmd.contains("\"ory_spock_reconcile_history\":false"), "{}", core[4].cmd);
        assert!(core[5].cmd.ends_with("spock-mesh-wire.yml --limit node-x"), "{}", core[5].cmd);
        assert!(!core[4].cmd.contains("-e site_id") && !core[5].cmd.contains("-e site_id"));

        // A site that keeps its own wrapper gets it.
        std::fs::write(t.path().join("ops/ansible/zagros-b-bootstrap.yml"), "---\n").unwrap();
        let ph = plan(t.path(), &o).unwrap();
        let core: Vec<&Phase> = ph.iter().filter(|p| p.label.contains("[core, joiner]")).collect();
        assert!(core[0].cmd.contains("-l node-x zagros-b-bootstrap.yml -e bootstrap_owner_email"), "{}", core[0].cmd);
    }

    /// The generic wrapper exists and sets site_id from bootstrap_site as a
    /// play variable on the import, which is what keeps other hosts' site_id.
    #[test]
    fn the_bootstrap_wrapper_sets_site_id_as_a_play_var() {
        let text = std::fs::read_to_string(repo().join("ops/ansible/site-bootstrap-for.yml")).unwrap();
        let docs: Vec<serde_yaml::Value> = serde_yaml::from_str(&text).unwrap();
        let import = docs.iter().find(|d| d.get("ansible.builtin.import_playbook").is_some()).expect("an import_playbook entry");
        assert_eq!(import["ansible.builtin.import_playbook"].as_str(), Some("site-bootstrap.yml"));
        assert_eq!(import["vars"]["site_id"].as_str(), Some("{{ bootstrap_site }}"));
    }

    #[test]
    fn a_founder_breakglass_node_converges_its_class_only() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        let r = join_node(&repo(), &opts("zagros-prod", "breakglass", "zagros_prod")).unwrap();
        let phases: Vec<&str> = r.checks.iter().filter(|c| c.name == "phase").map(|c| c.detail.as_str()).collect();
        assert_eq!(phases.iter().filter(|p| p.contains("=== Phase 13 ")).count(), 1);
        assert!(phases.iter().any(|p| p.contains("[breakglass] break-glass node (ADR 1114")));
        let cmd = r.checks.iter().filter(|c| c.name == "cmd").find(|c| c.detail.contains("breakglass-nodes.yml")).unwrap();
        assert!(cmd.detail.contains("-e target=zagros_prod_breakglass_hosts -l node-x"), "{}", cmd.detail);
    }

    /// Each class's node sits in its own group; the core group only for core.
    #[test]
    fn each_class_resolves_to_its_own_group() {
        assert_eq!(class_group("zagros_b", "core"), "zagros_b_hosts");
        assert_eq!(class_group("zagros_b", "obs"), "zagros_b_obs_hosts");
        assert_eq!(class_group("zagros_prod", "breakglass"), "zagros_prod_breakglass_hosts");
        assert_eq!(class_group("zagros_prod", "network-jump"), "zagros_prod_network_jump_hosts");
    }

    /// An obs node's baseline, kmod and gates name the obs group, never the
    /// core group: `-e target=<core> -l <obs node>` matches no host and the
    /// play passes having done nothing.
    #[test]
    fn an_obs_node_runs_every_host_phase_against_the_obs_group() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        let r = join_node(&repo(), &opts("zagros-prod", "obs", "zagros_prod")).unwrap();
        let cmds: Vec<&str> = r.checks.iter().filter(|c| c.name == "cmd").map(|c| c.detail.as_str()).collect();
        assert!(cmds.iter().all(|c| !c.contains("target=zagros_prod_hosts ")), "{cmds:?}");
        assert!(cmds.iter().any(|c| c.contains("mgmt-host-baseline.yml -e target=zagros_prod_obs_hosts")), "{cmds:?}");
        assert!(cmds.iter().any(|c| c.contains("standup-config-preflight.py' zagros_prod_hosts --obs zagros_prod_obs_hosts")), "{cmds:?}");
    }

    /// Phase 12 refuses any TBD_ left in the node's group_vars (a TBD mesh_ip_a
    /// carries no node name), and phase 7 refuses only the unnamed default CA,
    /// not the `dev-` key-mode prefix every goatnet's CA id carries.
    #[test]
    fn the_tbd_and_ca_gates_test_what_they_name() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        let r = join_node(&repo(), &opts("uk", "core", "zagros")).unwrap();
        let cmds: Vec<&str> = r.checks.iter().filter(|c| c.name == "cmd").map(|c| c.detail.as_str()).collect();
        let p12 = cmds.iter().find(|c| c.contains("TBD_")).unwrap();
        assert!(p12.contains("! grep -rqn 'TBD_'") && p12.contains("_hosts/' && ! grep -n 'TBD_'"), "{p12}");
        let p7 = cmds.iter().find(|c| c.contains("ca_id:")).unwrap();
        assert!(p7.contains("-goat-ca-") && !p7.contains("\"?dev-'"), "{p7}");
    }

    /// No secret is expanded onto a command line: phase 13 reads them inside
    /// the play (ansible's env lookup, after with-secrets has resolved them),
    /// and the phase 9 preflight gets the key over stdin with the real apex.
    #[test]
    fn secrets_stay_off_argv_and_the_preflight_checks_the_real_apex() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        let r = join_node(&repo(), &opts("uk", "core", "zagros")).unwrap();
        let cmds: Vec<&str> = r.checks.iter().filter(|c| c.name == "cmd").map(|c| c.detail.as_str()).collect();
        assert!(cmds.iter().all(|c| !c.contains("_PG_PASS)") && !c.contains("_NB_ENCKEY)") && !c.contains("_BKP_KEY)")), "{cmds:?}");
        assert!(cmds.iter().any(|c| c.contains("lookup(\"env\", \"") && c.contains("_PG_PASS\")")), "{cmds:?}");
        let p9 = cmds.iter().find(|c| c.contains("preflight-enrollment.sh")).unwrap();
        assert!(p9.contains("NETBIRD_MANAGEMENT_URL=") && p9.contains("| ssh ") && p9.ends_with("'bash -s'"), "{p9}");
    }

    #[test]
    fn an_unknown_class_and_a_run_without_an_address_are_refused() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        assert!(join_node(&repo(), &opts("zagros-prod", "router", "zagros_prod")).is_err());
        let o = Options { dry_run: false, ..opts("zagros-prod", "core", "zagros_prod") };
        assert!(join_node(&repo(), &o).unwrap_err().to_string().contains("--address"));
    }
}
