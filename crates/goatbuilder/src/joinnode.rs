//! `join-node`: one more machine of a class at a live site, declared →
//! converged → verified. Nothing drives this today but the runbook
//! (docs/operations/node-addition.md §5.4 "The sequence as actually
//! executed", §4/§5.4 the three passes, §6 "Done means proven"); this verb is
//! that sequence as numbered phases with the runbook's gates, in its order,
//! with its traps as the phase text. `--dry-run` prints every phase and
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
    let group = format!("{site_us}_hosts");
    let node = o.node;
    let aws = crate::settings::aws_profile(repo);
    let user = crate::settings::ssh_user(repo).unwrap_or_else(|| "<deploy.ssh_user>".to_string());
    let pre = format!("OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES ../scripts/with-aws {aws} -- ../scripts/ansible-preflight.sh");
    let inv = "-i inventory/mgmt-hosts.ini -i inventory/mgmt-hosts-mesh.ini";
    let addr = o.address.unwrap_or("<the node's LAN address>");
    let pass_var = o.pass_var.map(String::from).unwrap_or_else(|| format!("{upper}_NODE_PASS"));
    let gv = repo.join("ops/ansible/group_vars").join(&group).join("vars.yml");
    let joiner = gv_matches(&gv, "goatnet_founder", "false");
    let founder_site = gv_first(&gv, "join_founder_site");
    let founder_group = if founder_site.is_empty() { String::new() } else { format!("{}_hosts", founder_site.replace('-', "_")) };
    let intra_pair = gv_matches(&gv, "intra_site_pair", "true");
    let p = |n: u32, kind: &'static str, label: &str, cmd: String| Phase { n, kind, label: label.to_string(), cmd };

    let mut ev = vec![
        p(1, "MANUAL", "reach the box on the SITE LAN", format!("ssh {user}@{addr} — the address handed over may be 192.168.55.1: that is l4tbr0, the Jetson USB device-mode bridge on the box itself, reachable only over a direct USB-C cable. Find the real LAN address before anything else, then pass it as --address.")),
        p(2, "T", "seed the SSH key (key auth + NOPASSWD sudo; password from the env var, never the command line)", format!("'{r}/ops/scripts/seed-ssh-key.sh' {user}@{addr} {pass_var}")),
        p(3, "GATE", "repo work first: group, inventory, pins, exclusions (node-addition.md §5.5) — the config preflight passes", format!("python3 '{r}/ops/scripts/standup-config-preflight.py' {group}")),
        p(4, "T", "quiet apt on the node (the baseline stalls on the dpkg lock otherwise; unmasked in phase 14)", format!("ssh {user}@{addr} 'sudo systemctl mask --now apt-daily.timer apt-daily-upgrade.timer'")),
        p(5, "A", "host baseline, narrowed to the node — run detached if the session may drop: it apt-upgrades and reboot-waves", format!("cd '{r}/ops/ansible' && {pre} {inv} mgmt-host-baseline.yml -e target={group} -e site_id={site} -l {node}")),
        p(6, "A", "WireGuard kernel modules, first pass — the final check fails before enrolment (wt0 absent); expected, phase 11 re-runs it", format!("cd '{r}/ops/ansible' && ( {pre} {inv} wireguard-kmod.yml -e target={group} -l {node} || echo '>> wireguard-kmod: final check failed before enrolment (wt0 absent) — expected; phase 11 re-runs it for changed=0' )")),
        p(7, "GATE", "the CA this site trusts is its own, not the shared dev root (silence puts a production node on the dev CA)", format!("python3 '{r}/ops/scripts/standup-config-preflight.py' {group} && ! grep -rqE 'ca_id:[[:space:]]*\"?dev-' '{r}/ops/ansible/group_vars/{group}/' || {{ echo 'group_vars/{group} names a dev- CA; a site node trusts its goatnet CA'; exit 1; }}")),
        p(8, "MANUAL", "mint a setup key carrying the group the site's other mgmt hosts hold — read it off a live peer, do not guess", format!("in the NetBird dashboard of {site} (or `goat admin` on it): Setup Keys → new key, auto_groups = the group a live mgmt host of {site} holds; keep it for phase 10")),
        p(9, "GATE", "enrollment preflight on the node (clock skew, a stale wt0, an unreachable apex — each turns enrolment into a hang)", format!("ssh {user}@{addr} 'bash -s' < '{r}/ops/preflight-enrollment.sh")),
        p(10, "MANUAL", "enrol the node — the gate is the kernel ACL plane, not \"Connected\"", "on the node: sudo netbird up <the line phase 9 printed> --enable-rosenpass=false (match the goatnet's declared PQC posture; never also pass --rosenpass-permissive=false). Gate: `netbird status` shows Interface type: Kernel AND `sudo nft list tables` shows `table ip netbird` — Connected alone is an unfiltered node".to_string()),
        p(11, "A", "WireGuard kernel modules again — changed=0 proves the kmod took, now that wt0 exists", format!("cd '{r}/ops/ansible' && {pre} {inv} wireguard-kmod.yml -e target={group} -l {node}")),
        p(12, "GATE", "mesh_ip + the mesh inventory filled in one change (mesh IPs are not stable across re-enrolment) — preflight passes with no TBD_", format!("python3 '{r}/ops/scripts/standup-config-preflight.py' {group} && ! grep -rn 'TBD_' '{r}/ops/ansible/inventory/mgmt-hosts-mesh.ini' '{r}/ops/ansible/group_vars/{group}/' | grep -q {node}")),
    ];
    let ev_bootstrap = format!("-e site_id={site} -e bootstrap_owner_email=\"$OWNER_EMAIL\" -e postgres_superuser_password=\"$(printenv {upper}_PG_PASS)\" -e netbird_encryption_key=\"$(printenv {upper}_NB_ENCKEY)\" -e pgbackrest_cipher_pass=\"$(printenv {upper}_BKP_KEY)\"");
    match o.class {
        "core" if joiner => {
            if founder_site.is_empty() {
                return Err(anyhow!("group_vars/{group}/vars.yml declares goatnet_founder: false and no join_founder_site — a joiner must name the founder it seeds from (node-addition.md §3)"));
            }
            // site-seed-from-peer.yml is the generic play; a site that keeps its
            // own <site>-seed-from-peer.yml gets that one instead.
            let seed_play = crate::standup::site_play(repo, site, "seed-from-peer.yml");
            let p1 = if intra_pair { format!("{ev_bootstrap} -e participates_in_spock=false") } else { format!("{ev_bootstrap} -e participates_in_spock=false -e has_durable_independent_state=true") };
            ev.push(p(13, "A", "[core, joiner] pass 1 — bootstrap without Spock (AutoMigrate creates the schema once, on the empty DB)", format!("cd '{r}/ops/ansible' && {pre} {inv} -l {node} site-bootstrap.yml {p1}")));
            ev.push(p(13, "A", "[core, joiner] the service deploys the seed needs (kratos/hydra schemas): ory", format!("cd '{r}/ops/ansible' && {pre} {inv} ory-deploy.yml --limit {node} -e postgres_superuser_password=\"$(printenv {upper}_PG_PASS)\"")));
            ev.push(p(13, "A", &format!("[core, joiner] pass 2 — seed-from-peer: data-only copy from founder {founder_site} (fails loud if the account is populated)"), format!("cd '{r}/ops/ansible' && {pre} {inv} -l {node} {seed_play} -e site_id={site}")));
            ev.push(p(13, "A", "[core, joiner] pass 3 — bootstrap with Spock: subscriptions on both sides, polled to replicating", format!("cd '{r}/ops/ansible' && {pre} {inv} -l {node} site-bootstrap.yml {ev_bootstrap}")));
            ev.push(p(13, "A", &format!("[core, joiner] the founder-side mirror on {founder_group}, peer={site} only (§5.4a: the same change, not a follow-up)"), format!("cd '{r}/ops/ansible' && {pre} {inv} spock-mesh-wire.yml --limit {founder_group} -e '{{\"inter_site_spock_peers\":[\"{site}\"]}}'")));
        }
        "core" => {
            ev.push(p(13, "A", &format!("[core, founder site] site-bootstrap on the node{}", if intra_pair { " (intra_site_pair: the A→B streaming standby is phase 6A of the play)" } else { "" }), format!("cd '{r}/ops/ansible' && {pre} {inv} -l {node} site-bootstrap.yml {ev_bootstrap}")));
        }
        "obs" => {
            if let Some(missing) = crate::standup::obs_play_missing(repo) {
                return Err(anyhow!("{missing}"));
            }
            let og = format!("{site_us}_obs_hosts");
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
            ev.push(p(13, "A", "[network-jump] network-jump host (ADR 1131 — reach to switch, router and out-of-band gear)", format!("cd '{r}/ops/ansible' && {pre} {inv}{inv_extra} herd-connector-deploy.yml -e herd_connector_target_hosts={rg} -e target={rg} -l {node}")));
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
            checks.push(Check { name: "failed".into(), ok: false, detail: format!("!! Phase {} (\"{}\") FAILED — exit {rc}. No later phase has run; the node is left partway through phase {}. docs/operations/node-addition.md §5.4 names this step's proof and its trap.", ph.n, ph.label, ph.n), fix: Some(format!("fix the cause, then resume — every phase is idempotent: {resume} --from {}", ph.n)) });
            exit = EXIT_REFUSED;
            break;
        }
    }
    if o.dry_run {
        checks.push(row("footer", "PLAN complete. Remove --dry-run to execute (needs --address). Manual steps (1, 8, 10) pause the run; resume with --from <next>. Read docs/operations/node-addition.md §5.4 first.".into()));
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
        assert_eq!(core.len(), 5, "{phases:?}");
        assert!(core[0].contains("pass 1") && core[2].contains("pass 2") && core[3].contains("pass 3") && core[4].contains("founder-side mirror on zagros_hosts"));
        assert!(phases.last().unwrap().starts_with("=== Phase 15 [GATE] done means proven"));
        let cmds: Vec<&str> = r.checks.iter().filter(|c| c.name == "cmd").map(|c| c.detail.as_str()).collect();
        assert!(cmds.iter().any(|c| c.contains("-e participates_in_spock=false")));
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
