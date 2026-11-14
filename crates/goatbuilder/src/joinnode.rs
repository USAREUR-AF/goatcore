//! `join-node`: one more machine of a class at a live site, declared →
//! converged → verified. This verb is the node-addition sequence (the
//! executed order, the joiner's three passes, "done means proven") as
//! numbered phases with their gates, with each step's trap in the phase text;
//! docs/tutorials/02-first-federated-node.md walks through it. `--dry-run` prints every phase and
//! command and runs nothing; `--from`/`--to` resume; every phase is idempotent.
//!
//!   1  MANUAL reach the box on the SITE LAN (not the USB bridge address)
//!   2  T      seed-ssh-key.sh — key auth + NOPASSWD sudo, password from an env var;
//!             skipped when `ssh -o BatchMode=yes <user>@<addr> sudo -n true` already
//!             passes (a box installed unattended with the key and NOPASSWD sudo)
//!   3  GATE   repo work done first: standup-config-preflight.py passes
//!   4  T      quiet apt on the node (mask the daily timers; unmasked at 14)
//!   5  A      mgmt-host-baseline.yml, narrowed to the node
//!   6  A      wireguard-kmod.yml, first pass — its wt0 check fails before
//!             enrolment (wt0 absent); expected, and the only failure let pass
//!   7  GATE   the goatnet CA this site trusts is the site's own, not the shared dev root
//!   8  MANUAL mint a setup key carrying the group the site's other nodes hold
//!   9  GATE   preflight-enrollment.sh on the node (clock, stale wt0, apex reach)
//!  10  MANUAL netbird up with the goatnet's declared PQC posture; the gate is
//!             `table ip netbird`, not "Connected"
//!  11  A      wireguard-kmod.yml again — changed=0 proves the kmod took
//!  12  GATE   mesh_ip + the mesh inventory filled: preflight passes with no TBD_
//!  13  A      the class's converge: core (founder pass, or a joiner's three
//!             passes + the founder-side mirror, then signal federation: a
//!             gate on both sides' config, the relay pins and DNS, the token
//!             and proxy on each side, the routers, the relay pin, the
//!             delivery proof), obs (the three obs legs; at
//!             a joiner site the utility leg is skipped — the pub/sub state of
//!             record stays with the founder), breakglass, network-jump
//!  14  T      unmask the apt timers
//!  15  GATE   acceptance: every applicable row PASS, from the new node's context
//!
//! `--via mesh` (default) reaches the goatnet's hosts over this machine's mesh
//! (`mgmt-hosts.ini` + `mgmt-hosts-mesh.ini`); `--via door` over its env door
//! (`mgmt-hosts.ini` + `doors-groupvars.ini`, docs/design/env-doors.md), for a
//! machine not on that goatnet's mesh. The two transports are never combined.
//!
//! Status: built from the runbook; unmeasured on hardware (the runbook's own
//! run of is the record it transcribes).

use std::path::Path;
use crate::runner::Cmd;

use anyhow::{Context, Result};

use crate::config;
use crate::standup::{sh_quote, sh_word};
use crate::{Check, Report, EXIT_OK, EXIT_REFUSED};

pub struct Options<'a> {
    pub env: &'a str,
    pub site: &'a str,
    pub node: &'a str,
    pub class: &'a str,
    pub address: Option<&'a str>,
    pub pass_var: Option<&'a str>,
    /// How this machine reaches the goatnet's hosts: `mesh` or `door`.
    pub via: &'a str,
    pub from: u32,
    pub to: u32,
    pub dry_run: bool,
}

pub const CLASSES: [&str; 4] = ["core", "obs", "breakglass", "network-jump"];
pub const VIAS: [&str; 2] = ["mesh", "door"];

/// The inventories a play is given for each transport. Base inventory first
/// (group membership, site facts, router_role); then exactly one transport
/// override. Passing both overrides lets whichever loads last win silently.
fn inventories(via: &str) -> &'static str {
    match via {
        "door" => "-i inventory/mgmt-hosts.ini -i inventory/doors-groupvars.ini",
        _ => "-i inventory/mgmt-hosts.ini -i inventory/mgmt-hosts-mesh.ini",
    }
}

/// The step that opens the env door, printed first in a `--via door` plan and
/// run before the first phase (door-open.sh reuses a door already open).
fn door_open_cmd(repo: &Path, env: &str) -> String {
    format!("'{}/ops/scripts/door-open.sh' {env}", repo.display())
}

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

/// Exits 0 only when every `fatal:` in an ansible log falls under the
/// wireguard-kmod task that verifies wt0 (the check that cannot pass before the
/// node is enrolled, roles/wireguard-kmod/tasks/main.yml), and there is one.
const WT0_ABSENT_ONLY: &str = "awk '/^TASK \\[/{t=$0} /^fatal:/{n++; if (t !~ /verify wt0 is kernelspace wireguard/) bad=1} END{exit !(n>0 && !bad)}'";

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

/// Phase 13's signal-federation steps for a joiner's core node, after the Spock
/// wiring (ADR 1085 Amendment 1). A peer is present on exactly one signal
/// server, so a second management node that can take signal load needs its
/// signal and the founder's to forward offers to each other, over the mesh
/// with a relay-tunneled backup.
///
/// The gate comes first and checks what the operator's repository must hold
/// before anything changes: both sites' federation group_vars, complete and
/// crossed (each side's peer address and name are the other's), the founder's
/// relay inventory pinning each federation hostname to its own node's tunnel,
/// the hostnames resolving to the relays, and the shared token in the
/// environment. Then, in the order that never exposes an unauthenticated
/// surface: the token and proxy on the joiner and on the founder, the
/// federation routers and certificates, the joiner's relay tunnel, the relay
/// pins, and the delivery proof. A joiner that declares
/// `signal_federation_absent_reason` gets the gate and a SKIP line instead:
/// the gate passes when the founder is off too, or when the founder federates
/// with another joiner and does not name this one (a third site).
fn signal_federation(ev: &mut Vec<Phase>, repo: &Path, gv: &Path, site: &str, founder: &str, pre: &str, inv: &str) {
    let r = repo.display().to_string();
    let site_group = format!("{}_hosts", site.replace('-', "_"));
    let founder_group = format!("{}_hosts", founder.replace('-', "_"));
    let p = |kind: &'static str, label: String, cmd: String| Phase { n: 13, kind, label, cmd };
    // The founder site is read from vars.yml: one shell word wherever it is used.
    let w = |v: String| sh_word(&v);
    ev.push(p("GATE", format!("[signal federation] the repo declares it on both sides: {site_group} and {founder_group} crossed and complete, {founder}'s relay inventory pins each federation hostname to its own node, the names resolve to the relays, the shared token is in the environment"), format!("python3 '{r}/ops/scripts/signal-federation-check.py' {site} --peer {} --resolve --require-env", w(founder.to_string()))));
    if !gv_first(gv, "signal_federation_absent_reason").is_empty() {
        ev.push(p("SKIP", format!("[signal federation] declared off at {site} (signal_federation_absent_reason in group_vars/{site_group}); the gate above proved no side expects it: {founder} is off too, or federates with another site"), format!("echo '>> signal federation: declared off for {site} (signal_federation_absent_reason); nothing to converge'")));
        return;
    }
    let rathole = crate::standup::site_play(repo, founder, "rathole-client.yml");
    let relay_inv = w(format!("inventory/hosts-{founder}.ini"));
    let rathole = w(rathole);
    let missing = sh_quote(&format!("{founder} relay inventory inventory/hosts-{founder}.ini missing — generate it: inventory/generate-relays.sh {founder} terraform-{founder}"));
    let need_relay_inv = format!("{{ [ -f {relay_inv} ] || {{ echo {missing}; exit 1; }}; }}");
    ev.push(p("A", format!("[signal federation] token + dual-path proxy on the joiner {site} first (the token is enforced before any public path reaches it)"), format!("cd '{r}/ops/ansible' && {pre} {inv} signal-federation-transport.yml -e federation_site={site}")));
    ev.push(p("A", format!("[signal federation] token + dual-path proxy on the founder {founder} (the same change, not a follow-up: one direction alone leaves offers undeliverable)"), format!("cd '{r}/ops/ansible' && {pre} {inv} signal-federation-transport.yml -e {}", w(format!("federation_site={founder}")))));
    ev.push(p("A", "[signal federation] each node's federation router and its certificate (Traefik, DNS-01)".to_string(), format!("cd '{r}/ops/ansible' && {pre} {inv} traefik-acme.yml -e target={site_group} && {pre} {inv} traefik-acme.yml -e {}", w(format!("target={founder_group}")))));
    ev.push(p("A", format!("[signal federation] the joiner's tunnel to every {founder} relay ({rathole}, service rathole_client_service_name)"), format!("cd '{r}/ops/ansible' && {need_relay_inv} && {pre} -i {relay_inv} {inv} {rathole} -e rathole_client_target={site_group}")));
    ev.push(p("A", format!("[signal federation] {founder}'s relays: the {site} backend and its federation-hostname pin, re-rendered and post-checked (relay-ingress-add-site.yml)"), format!("cd '{r}/ops/ansible' && {need_relay_inv} && {pre} -i {relay_inv} -i {} {inv} relay-ingress-add-site.yml -e target=goat_relay_hosts -e {} -e join_site={site}", w(format!("inventory/{founder}-goat-relay-alias.ini")), w(format!("founder_site={founder}")))));
    ev.push(p("GATE", "[signal federation] proven from both nodes: transport up both ways, an offer forwarded and delivered".to_string(), format!("cd '{r}/ops/ansible' && {pre} {inv} signal-federation-verify.yml -e federation_site={site} -e {}", w(format!("federation_peer_site={founder}")))));
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
    let declared_user = crate::settings::ssh_user(repo);
    let user = declared_user.clone().unwrap_or_else(|| "<deploy.ssh_user>".to_string());
    // The declared SSH user as one shell word; the placeholder a plan prints
    // when none is declared stays as it reads.
    let user_w = declared_user.as_deref().map(sh_word).unwrap_or_else(|| user.clone());
    let pre = format!("OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES ../scripts/with-aws {} -- ../scripts/ansible-preflight.sh", sh_word(&aws));
    let inv = inventories(o.via);
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
        p(2, "T", "seed the SSH key (key auth + NOPASSWD sudo; password from the env var, never the command line) — skipped when key auth and NOPASSWD sudo already work", format!("if ssh -o BatchMode=yes -o ConnectTimeout=10 {user_w}@{addr} sudo -n true; then echo {}; else '{r}/ops/scripts/seed-ssh-key.sh' {user_w}@{addr} {pass_var}; fi", sh_quote(&format!(">> {user}@{addr}: key auth and NOPASSWD sudo already work — seed skipped")))),
        // Phases 3 and 7 run before enrolment: the mesh-address placeholder
        // phase 12 fills is a warning there (--before-enrolment), not a failure.
        p(3, "GATE", "repo work first: group, inventory, pins, exclusions — the config preflight passes (the mesh-address placeholder is allowed until phase 12)", format!("python3 '{r}/ops/scripts/standup-config-preflight.py' {preflight_target} --before-enrolment")),
        p(4, "T", "quiet apt on the node (the baseline stalls on the dpkg lock otherwise; unmasked in phase 14)", format!("ssh {user_w}@{addr} 'sudo systemctl mask --now apt-daily.timer apt-daily-upgrade.timer'")),
        p(5, "A", "host baseline, narrowed to the node — run detached if the session may drop: it apt-upgrades and reboot-waves", format!("cd '{r}/ops/ansible' && {pre} {inv} mgmt-host-baseline.yml -e target={group} -e site_id={site} -l {node}")),
        p(6, "A", "WireGuard kernel modules, first pass — its wt0 check fails before enrolment (wt0 absent); expected, phase 11 re-runs it. Any other failure stops the run", format!("cd '{r}/ops/ansible' && log=$(mktemp) && if {pre} {inv} wireguard-kmod.yml -e target={group} -l {node} 2>&1 | tee \"$log\"; then rm -f \"$log\"; elif {WT0_ABSENT_ONLY} \"$log\" && grep -q 'wt0.*does not exist' \"$log\"; then rm -f \"$log\"; echo '>> wireguard-kmod: the wt0 check failed before enrolment (wt0 absent) — expected; phase 11 re-runs it for changed=0'; else echo \"!! wireguard-kmod failed on something other than the pre-enrolment wt0 check — output kept in $log\"; exit 1; fi")),
        p(7, "GATE", "the CA this site trusts is its own, not the shared dev root (silence puts a production node on the dev CA)", format!("python3 '{r}/ops/scripts/standup-config-preflight.py' {preflight_target} --before-enrolment && ! grep -rqE 'ca_id:[[:space:]]*\"?[a-z]+-goat-ca-' '{r}/ops/ansible/group_vars/{group}/' || {{ echo 'group_vars/{group} names the unnamed default CA (<mode>-goat-ca-<date>); a site node trusts its goatnet CA'; exit 1; }}")),
        p(8, "MANUAL", "mint a setup key carrying the group the site's other mgmt hosts hold — read it off a live peer, do not guess", format!("in the NetBird dashboard of {enrol_at} (or `goat admin` on it): Setup Keys → new key, auto_groups = the group a live mgmt host of {enrol_at} holds; keep it for phase 10")),
        p(9, "GATE", "enrollment preflight on the node (clock skew, a stale wt0, an unreachable apex — each turns enrolment into a hang)", format!("{{ printf 'export NETBIRD_SETUP_KEY=%q NETBIRD_MANAGEMENT_URL=%q\\n' \"$(printenv {upper}_NB_SETUP_KEY)\" {}; cat '{r}/ops/preflight-enrollment.sh'; }} | ssh {user_w}@{addr} 'bash -s'", sh_quote(&format!("https://{enrol_dns}")))),
        p(10, "MANUAL", "enrol the node — the gate is the kernel ACL plane, not \"Connected\"", "on the node: sudo netbird up <the line phase 9 printed> --enable-rosenpass=false (match the goatnet's declared PQC posture; never also pass --rosenpass-permissive=false). Gate: `netbird status` shows Interface type: Kernel AND `sudo nft list tables` shows `table ip netbird` — Connected alone is an unfiltered node".to_string()),
        p(11, "A", "WireGuard kernel modules again — changed=0 proves the kmod took, now that wt0 exists", format!("cd '{r}/ops/ansible' && {pre} {inv} wireguard-kmod.yml -e target={group} -l {node}")),
        p(12, "GATE", "mesh_ip + the mesh inventory filled in one change (mesh IPs are not stable across re-enrolment) — preflight passes with no TBD_", format!("python3 '{r}/ops/scripts/standup-config-preflight.py' {preflight_target} && ! grep -rqn 'TBD_' '{r}/ops/ansible/group_vars/{group}/' && ! grep -n 'TBD_' '{r}/ops/ansible/inventory/mgmt-hosts-mesh.ini' | grep -qw {node}")),
    ];
    let ev_secrets = format!("-e bootstrap_owner_email=\"$OWNER_EMAIL\" -e 'postgres_superuser_password={{{{ lookup(\"env\", \"{upper}_PG_PASS\") }}}}' -e 'netbird_encryption_key={{{{ lookup(\"env\", \"{upper}_NB_ENCKEY\") }}}}' -e 'pgbackrest_cipher_pass={{{{ lookup(\"env\", \"{upper}_BKP_KEY\") }}}}'");
    let ev_bootstrap = format!("-e site_id={site} {ev_secrets}");
    match o.class {
        "core" if joiner => {
            if founder_site.is_empty() {
                return Err(goat_errors::coded("GB-250", format!("group_vars/{group}/vars.yml declares goatnet_founder: false and no join_founder_site — a joiner must name the founder it seeds from")).into());
            }
            // site-seed-from-peer.yml is the generic play; a site that keeps its
            // own <site>-seed-from-peer.yml gets that one instead.
            let seed_play = crate::standup::site_play(repo, site, "seed-from-peer.yml");
            // site_id reaches the joiner's passes as a PLAY variable, never as
            // `-e site_id`: an extra var sets every host's site_id to this site,
            // and the Spock roles' inventory walk then finds no peer.
            let bs = crate::standup::bootstrap_play(repo, site);
            let p1 = if intra_pair { format!("{ev_secrets} -e participates_in_spock=false") } else { format!("{ev_secrets} -e participates_in_spock=false -e has_durable_independent_state=true") };
            // Pass 1 runs site-bootstrap's `stack` plays only, as standup's
            // phase 5 does: the `after-ory` plays wait for the management
            // service, which waits for Hydra, which the next step deploys.
            // Pass 3 runs the whole file once Ory is up.
            ev.push(p(13, "A", "[core, joiner] pass 1 — bootstrap without Spock, the stack plays only (AutoMigrate creates the schema once, on the empty DB; Postgres initialises with the founder's encoding; the after-ory plays wait for Ory, deployed next)", format!("cd '{r}/ops/ansible' && {pre} {inv} -l {node} {bs} {p1} --skip-tags after-ory")));
            ev.push(p(13, "A", "[core, joiner] the service deploys the seed needs (kratos/hydra schemas): ory", format!("cd '{r}/ops/ansible' && {pre} {inv} ory-deploy.yml --limit {node} -e 'postgres_superuser_password={{{{ lookup(\"env\", \"{upper}_PG_PASS\") }}}}'")));
            ev.push(p(13, "A", &format!("[core, joiner] pass 2 — seed-from-peer: data-only copy from founder {founder_site} (fails loud if the account is populated)"), format!("cd '{r}/ops/ansible' && {pre} {inv} -l {node} {seed_play} -e site_id={site}")));
            // Ory (Phase 7D) subscribes each side only to a peer that already has
            // its ory_narrow node, and its history reconcile checks the reverse
            // subscription on the peer, so the three steps below are staged:
            // joiner node+repset, founder node+repset+subscription, joiner
            // subscription+reconcile (which copies history both ways).
            ev.push(p(13, "A", "[core, joiner] pass 3 — bootstrap with Spock: netbird subscriptions on both sides, polled to replicating; Ory node + repset only", format!("cd '{r}/ops/ansible' && {pre} {inv} -l {node} {bs} {ev_secrets} -e ory_spock_wire_subscriptions=false")));
            ev.push(p(13, "A", &format!("[core, joiner] the founder-side mirror on {founder_group}, peer={site} only (the same change, not a follow-up): netbird + Ory node, repset, subscription"), format!("cd '{r}/ops/ansible' && {pre} {inv} spock-mesh-wire.yml --limit {} -e '{{\"inter_site_spock_peers\":[\"{site}\"],\"ory_spock_reconcile_history\":false}}'", sh_word(&founder_group))));
            ev.push(p(13, "A", "[core, joiner] the joiner's Spock wiring again, now that the founder's half exists: Ory subscription + history reconcile", format!("cd '{r}/ops/ansible' && {pre} {inv} spock-mesh-wire.yml --limit {node}")));
            signal_federation(&mut ev, repo, &gv, site, &founder_site, &pre, inv);
        }
        "core" => {
            ev.push(p(13, "A", &format!("[core, founder site] site-bootstrap on the node{}", if intra_pair { " (intra_site_pair: the A→B streaming standby is phase 6A of the play)" } else { "" }), format!("cd '{r}/ops/ansible' && {pre} {inv} -l {node} site-bootstrap.yml {ev_bootstrap}")));
        }
        "obs" => {
            if let Some(missing) = crate::standup::obs_play_missing(repo).filter(|_| !joiner) {
                return Err(goat_errors::coded("GB-248", missing).into());
            }
            let og = &group;
            ev.push(p(13, "A", "[obs] observability host: baseline + LUKS (own nodes only)", format!("cd '{r}/ops/ansible' && {pre} {inv} mgmt-host-baseline.yml -e target={og} -e site_id={site} -l {node} && {pre} {inv} tang-server.yml -e target={og} -l {node} && {pre} {inv} mgmt-host-luks.yml -e target={og} -e site_id={site} -l {node}")));
            if joiner {
                // The pub/sub state of record has one primary per goatnet, at
                // the founder; a joiner's obs host does not start a second one.
                let at = if founder_site.is_empty() { "the founder".to_string() } else { format!("the founder {founder_site}") };
                ev.push(p(13, "SKIP", &format!("[obs, joiner] utility leg (Forgejo, the pub/sub state-of-record primary) not run: {site} joins a goatnet, and its state of record stays with {at}"), format!("echo '>> utility-host-bootstrap.yml skipped: {site} is a joiner site (goatnet_founder: false); the state of record stays with {at}'")));
            } else {
                ev.push(p(13, "A", "[obs] observability host: utility (Forgejo, the pub/sub state-of-record primary; docker data-root on the NVMe first, F-504)", format!("cd '{r}/ops/ansible' && {pre} {inv} utility-host-bootstrap.yml -e target={og} -l {node}")));
            }
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
        other => return Err(goat_errors::coded("GB-247", format!("class '{other}' is not one this verb converges; known: {}", CLASSES.join(", "))).into()),
    }
    ev.push(p(14, "T", "unmask the apt timers now that the node is finished", format!("ssh {user_w}@{addr} 'sudo systemctl unmask apt-daily.timer apt-daily-upgrade.timer'")));
    ev.push(p(15, "GATE", "done means proven: every applicable acceptance row PASS, never UNKNOWN (a joiner adds the replicating-both-ways evidence)", format!("python3 '{r}/ops/validation/generate-values.py' {site} && python3 '{r}/ops/validation/run-acceptance.py' --env {site} --values '{r}/ops/validation/values-{site}.yaml' --all-eligible --by-layer")));
    Ok(ev)
}

/// Every command the plan runs, in order: the door when `--via door`, then
/// each phase that is not a manual step (whose text is for a person). What
/// `goatbuilder inputs join-node` must give a source for.
#[cfg(test)]
pub(crate) fn plan_commands(repo: &Path, o: &Options) -> Result<Vec<String>> {
    let mut v: Vec<String> = Vec::new();
    if o.via == "door" {
        v.push(door_open_cmd(repo, o.env));
    }
    v.extend(plan(repo, o)?.into_iter().filter(|p| p.kind != "MANUAL").map(|p| p.cmd));
    Ok(v)
}

/// The class's inventory group (`class_group`) and the transport's
/// inventories (`inventories`), for `goatbuilder inputs`.
pub(crate) fn group_of(site: &str, class: &str) -> String {
    class_group(&site.replace('-', "_"), class)
}

pub(crate) fn inventories_of(via: &str) -> &'static str {
    inventories(via)
}

pub fn join_node(repo: &Path, o: &Options) -> Result<Report> {
    let cfg = config::load(&repo.join("ops/goatbuilder/goatbuilder.yaml"))?;
    let Some(env) = cfg.environments.iter().find(|e| e.name == o.env) else {
        return Err(goat_errors::coded("GB-241", format!("unknown environment \"{}\"; declared: {:?}", o.env, cfg.environments.iter().map(|e| e.name.as_str()).collect::<Vec<_>>())).into());
    };
    if !env.sites.is_empty() {
        let Some(site) = env.sites.iter().find(|s| s.name == o.site) else {
            return Err(goat_errors::coded("GB-247", format!("site '{}' is not declared for environment '{}' (declared: {})", o.site, o.env, env.sites.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", "))).into());
        };
        if !site.classes.is_empty() && !site.classes.contains_key(o.class) {
            return Err(goat_errors::coded("GB-247", format!("class '{}' is not declared at site '{}' (declared: {})", o.class, o.site, site.classes.keys().cloned().collect::<Vec<_>>().join(", "))).into());
        }
    }
    if !CLASSES.contains(&o.class) {
        return Err(goat_errors::coded("GB-247", format!("class '{}' is not one this verb converges; known: {}", o.class, CLASSES.join(", "))).into());
    }
    execute(repo, o)
}

/// The end-state model's script, relative to the checkout.
const MODEL: &str = "ops/goatbuilder/endstate/endstate.py";

/// One service the site does not run, as the plan prints it.
fn not_run_line(service: &str, reason: &str) -> String {
    format!("{service} \u{2014} {reason}")
}

/// What the site does not run (docs/design/goatbuilder.md, `join-node` (d)):
/// every catalog service the end-state model marks absent here, each with the
/// model's own reason, under one heading. The model is read, never re-derived,
/// so this list and `goatbuilder endstate`'s N/A rows cannot disagree. A model
/// that cannot be read is one `not read` line naming what is missing; the run
/// does not stop for it, and no list is guessed.
fn not_run(repo: &Path, site: &str) -> Vec<Check> {
    let shown = format!("{MODEL} model {site}");
    let mut rows = vec![row("not-run-heading", format!("What {site} does not run: every catalog service the end-state model marks absent here, with its reason ({shown})"))];
    let not_read = |why: String| row("not-run", format!("not read: {why}; this list of what the site does not run is left out"));
    let script = repo.join(MODEL);
    if !script.is_file() {
        rows.push(not_read(format!("{MODEL} is not in this checkout")));
        return rows;
    }
    let python = crate::endstate::python();
    let out = match Cmd::new(&python).arg(&script).args(["model", site, "--root"]).arg(repo).current_dir(repo).output() {
        Ok(o) => o,
        Err(e) => {
            rows.push(not_read(format!("cannot run {python} for {shown}: {e}")));
            return rows;
        }
    };
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
        let how = out.status.code().map_or("it was killed".to_string(), |c| format!("exit {c}"));
        rows.push(not_read(format!("{shown} failed ({how}){}", if last.is_empty() { String::new() } else { format!(": {last}") })));
        return rows;
    }
    let Ok(model) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else {
        rows.push(not_read(format!("{shown} printed no model")));
        return rows;
    };
    let Some(services) = model.get("services").and_then(|s| s.as_object()) else {
        rows.push(not_read(format!("{shown} printed no services")));
        return rows;
    };
    let before = rows.len();
    for (name, s) in services {
        if s.get("present") != Some(&serde_json::Value::Bool(false)) {
            continue;
        }
        let reason = s.get("reason").and_then(|r| r.as_str()).map(str::to_string).unwrap_or_else(|| {
            // A model from before the reason field: its declarations, verbatim.
            let decls = s.get("declarations").and_then(|d| d.as_array()).cloned().unwrap_or_default();
            decls.iter().filter_map(|d| d.get("why").and_then(|w| w.as_str())).collect::<Vec<_>>().join("; ")
        });
        rows.push(row("not-run", not_run_line(name, &reason)));
    }
    if rows.len() == before {
        rows.push(row("not-run", "nothing: the model marks no catalog service absent here".into()));
    }
    rows
}

/// The plan and the run, once the environment, site and class are known.
fn execute(repo: &Path, o: &Options) -> Result<Report> {
    if !VIAS.contains(&o.via) {
        return Err(goat_errors::coded("GB-242", format!("--via '{}' is not a transport; known: {} (mesh = this machine is on the goatnet's mesh; door = through its env door, docs/design/env-doors.md)", o.via, VIAS.join(", "))).into());
    }
    if !o.dry_run && o.address.is_none() {
        return Err(goat_errors::coded("GB-242", "--address <the node's LAN address> is required to run (phase 1 says how to find it); --dry-run plans without it").into());
    }
    if !o.dry_run && crate::settings::ssh_user(repo).is_none() {
        return Err(goat_errors::coded("GB-251", "this checkout declares no SSH user for a new machine: set deploy.ssh_user in ops/goatbuilder/goatbuilder.yaml; --dry-run plans without it").into());
    }
    let phases = plan(repo, o)?;
    // Every value the run uses, with its source (gate row F2): the run record
    // carries it, and a dry run prints it. A secret by its source only.
    // Listing reads the secrets posture; a dry run prints those probes too.
    let (listed, listing_probes) = crate::secrets::probes_of(|| crate::inputs::joinnode::inputs(repo, o, &|k: &str| std::env::var(k).ok()));
    crate::inputs::record(&listed);
    let resume = format!("goatbuilder join-node {} --site {} --node {} --class {}{}{}", o.env, o.site, o.node, o.class, o.address.map(|a| format!(" --address {a}")).unwrap_or_default(), if o.via == "door" { " --via door" } else { "" });
    let mut checks = vec![row("banner", format!("join-node: site={}  node={}  class={}  via={}  {}", o.site, o.node, o.class, o.via, if o.dry_run { "(DRY RUN — nothing executes)" } else { "" }))];
    if o.dry_run {
        checks.extend(listing_probes.iter().map(|l| row("probe-ran", l.clone())));
    }
    let mut exit = EXIT_OK;
    if o.via == "door" {
        let open = door_open_cmd(repo, o.env);
        checks.push(row("door", format!("=== Before phase 1: open the env door to {} — every play below goes through it (mgmt-hosts.ini + doors-groupvars.ini, never the mesh inventory too) ===", o.env)));
        checks.push(row("cmd", open.clone()));
        if !o.dry_run {
            crate::runner::phase("door", &format!("open the env door to {}", o.env), "");
            let status = Cmd::new("bash").args(["-euo", "pipefail", "-c", &open]).current_dir(repo).status().context("bash")?;
            if !status.success() {
                checks.push(Check { name: "failed".into(), ok: false, detail: format!("!! the env door to {} did not open — no phase has run", o.env), fix: Some(format!("{open} --status says why; then re-run: {resume} --from {}", o.from.max(1))) });
                let mut r = Report::new("join-node", checks, EXIT_REFUSED);
                r.exit = EXIT_REFUSED;
                r.ok = false;
                return Ok(r);
            }
        }
    }
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
        crate::runner::phase(&ph.n.to_string(), &ph.label, "");
        let status = Cmd::new("bash").args(["-euo", "pipefail", "-c", &ph.cmd]).current_dir(repo).status().context("bash")?;
        if !status.success() {
            let rc = status.code().unwrap_or(1);
            checks.push(Check { name: "failed".into(), ok: false, detail: format!("!! Phase {} (\"{}\") FAILED — exit {rc}. No later phase has run; the node is left partway through phase {}. `--dry-run` prints this phase's command and the gate that proves it.", ph.n, ph.label, ph.n), fix: Some(format!("fix the cause, then resume — every phase is idempotent: {resume} --from {}", ph.n)) });
            exit = EXIT_REFUSED;
            break;
        }
    }
    // What the site does not run, in the plan and in a real run's summary alike.
    checks.extend(not_run(repo, o.site));
    if o.dry_run {
        checks.extend(crate::inputs::rows(&listed));
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
    use std::process::Command;

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
        Options { env, site, node: "node-x", class, address: None, pass_var: None, via: "mesh", from: 0, to: 99, dry_run: true }
    }

    /// Phases 3 and 7 run before the node enrols, so they pass the config
    /// preflight `--before-enrolment` (the mesh-address placeholder is a
    /// warning); phase 12, after enrolment, runs it without, so the
    /// placeholder fails there.
    #[test]
    fn only_the_gates_before_enrolment_allow_the_mesh_placeholder() {
        let t = joiner_checkout();
        for class in ["core", "obs"] {
            let ph = plan(t.path(), &opts("zagros-b", class, "zagros")).unwrap();
            let gate = |n: u32| ph.iter().find(|p| p.n == n).unwrap().cmd.clone();
            for n in [3, 7] {
                assert!(gate(n).contains("standup-config-preflight.py' ") && gate(n).contains(" --before-enrolment"), "{class} phase {n}: {}", gate(n));
            }
            assert!(gate(12).contains("standup-config-preflight.py' ") && !gate(12).contains("--before-enrolment"), "{class} phase 12: {}", gate(12));
        }
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

    /// A value a site's vars.yml or the project's goatbuilder.yaml declares
    /// reaches the phases as written, never as shell: every planned command
    /// runs through bash, with a hostile founder site, enrolment name, SSH user
    /// and AWS profile and stand-ins for every program it starts, and nothing
    /// the values hold is executed. The stand-ins record what they were given,
    /// and the founder reaches the signal-federation gate as written.
    #[test]
    fn declared_values_reach_join_node_phases_without_shell_interpolation() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        let marker = root.join("pwned");
        let m = marker.display().to_string();
        assert!(!m.contains(char::is_whitespace), "{m}");
        let founder = format!("za$(touch${{IFS}}{m})b`touch${{IFS}}{m}`");
        let repo = root.join("repo");
        let gv = repo.join("ops/ansible/group_vars/zagros_b_hosts");
        std::fs::create_dir_all(&gv).unwrap();
        std::fs::write(
            gv.join("vars.yml"),
            format!("goatnet_founder: false\njoin_founder_site: {founder}\nsite_enrollment_dns: e$(touch${{IFS}}{m})x\n"),
        )
        .unwrap();
        std::fs::create_dir_all(repo.join("ops/goatbuilder")).unwrap();
        std::fs::write(
            repo.join("ops/goatbuilder/goatbuilder.yaml"),
            format!("deploy:\n  ssh_user: \"u$(touch${{IFS}}{m})\"\n  aws_profile: \"p$(touch${{IFS}}{m})\"\n"),
        )
        .unwrap();
        // Stand-ins for every program the phases start, each recording its argv.
        let log = root.join("argv.log");
        let stub = format!("#!/bin/sh\nfor a; do printf '%s\\n' \"$a\"; done >> '{}'\nprintf -- '--\\n' >> '{}'\nexit 0\n", log.display(), log.display());
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let exe = |p: &Path| crate::test_support::executable(p, &stub);
        for b in ["python3", "ansible-playbook", "ansible", "go", "netbird", "sudo"] {
            exe(&bin.join(b));
        }
        // ssh runs what it is piped (`... | ssh <node> 'bash -s'`) as the node would.
        let ssh = bin.join("ssh");
        crate::test_support::executable(&ssh, &format!("#!/bin/sh\nfor a; do printf '%s\\n' \"$a\"; done >> '{}'\nfor a; do [ \"$a\" = 'bash -s' ] && exec bash -s; done\nexit 0\n", log.display()));
        for s in ["ops/scripts/with-aws", "ops/scripts/ansible-preflight.sh", "ops/scripts/seed-ssh-key.sh", "ops/scripts/door-open.sh"] {
            exe(&repo.join(s));
        }
        std::fs::write(repo.join("ops/preflight-enrollment.sh"), "true\n").unwrap();
        std::fs::create_dir_all(repo.join("ops/ansible/inventory")).unwrap();
        let path = format!("{}:/usr/bin:/bin", bin.display());
        let mut ran = 0;
        for class in ["core", "obs"] {
            for via in ["mesh", "door"] {
                let o = Options { env: "zagros", site: "zagros-b", node: "node-x", class, address: Some("192.0.2.30"), pass_var: None, via, from: 1, to: 15, dry_run: true };
                for ph in plan(&repo, &o).unwrap().into_iter().filter(|p| p.kind != "MANUAL") {
                    let out = Command::new("bash")
                        .args(["-c", &ph.cmd])
                        .current_dir(&repo)
                        .env("PATH", &path)
                        .env("HOME", root)
                        .output()
                        .unwrap();
                    let _ = out;
                    ran += 1;
                    assert!(!marker.exists(), "phase {} ({}) ran a declared value as shell: {}", ph.n, ph.label, ph.cmd);
                }
            }
        }
        assert!(ran > 20, "{ran} commands");
        let argv = std::fs::read_to_string(&log).unwrap_or_default();
        assert!(argv.lines().any(|l| l == founder), "the founder reaches the gate as written:\n{argv}");
    }

    /// One line per service the site does not run: its name, a dash, and the
    /// model's reason verbatim. A checkout with no model gets one `not read`
    /// line under the heading, naming the missing script, and no list.
    #[test]
    fn what_the_site_does_not_run_reads_as_the_models_reason() {
        assert_eq!(
            not_run_line("isitup", "`isitup_enabled` is false in group_vars"),
            "isitup \u{2014} `isitup_enabled` is false in group_vars"
        );
        let t = tempfile::tempdir().unwrap();
        let rows = not_run(t.path(), "zagros-b");
        assert_eq!(rows.len(), 2, "{:?}", rows.iter().map(|r| &r.detail).collect::<Vec<_>>());
        assert_eq!(rows[0].name, "not-run-heading");
        assert!(rows[0].detail.starts_with("What zagros-b does not run:") && rows[0].detail.contains("ops/goatbuilder/endstate/endstate.py model zagros-b"), "{}", rows[0].detail);
        assert_eq!(rows[1].name, "not-run");
        assert_eq!(rows[1].detail, "not read: ops/goatbuilder/endstate/endstate.py is not in this checkout; this list of what the site does not run is left out");
        assert!(rows.iter().all(|r| r.ok), "a missing list is a stated line, not a refusal");
    }

    /// The scripts and plays a planned command runs: every `*.yml`, `*.py` and
    /// `*.sh` word it names, by file name, except the group_vars files a step
    /// only reads.
    fn command_runs(cmd: &str) -> std::collections::BTreeSet<String> {
        let re = regex::Regex::new(r"[A-Za-z0-9_./-]+\.(?:yml|py|sh)\b").unwrap();
        re.find_iter(cmd)
            .map(|m| m.as_str())
            .filter(|w| !w.contains("group_vars/"))
            .map(|w| w.rsplit('/').next().unwrap_or(w).to_string())
            .collect()
    }

    /// The scripts and plays an entry's `runs` names: each comma-separated
    /// part's first word, by its file name.
    fn runs_names(runs: &str) -> Vec<String> {
        runs.split(", ")
            .filter_map(|part| part.split_whitespace().next())
            .filter(|w| {
                w.contains('/') || w.ends_with(".yml") || w.ends_with(".py") || w.ends_with(".sh")
            })
            .map(|w| w.rsplit('/').next().unwrap_or(w).to_string())
            .collect()
    }

    /// The phase catalog (ops/phases/catalog.yaml) describes join-node as its
    /// plan is, for every class and site shape: a founder's and a joiner's core
    /// node (signal federation on, and declared off), a founder's and a joiner's
    /// obs node, a breakglass and a network-jump node, and a run through the env
    /// door. Every phase the plan holds has an entry and every entry is a phase;
    /// a phase's `kind` is the kinds of its steps; and its `runs` names exactly
    /// the scripts and plays its steps run.
    #[test]
    fn the_phase_catalog_describes_join_nodes_plan() {
        let t = joiner_checkout();
        let gv = t.path().join("ops/ansible/group_vars/zagros_hosts");
        std::fs::create_dir_all(&gv).unwrap();
        std::fs::write(gv.join("vars.yml"), "goatnet_founder: true\n").unwrap();
        std::fs::write(
            t.path().join("ops/ansible/utility-host-bootstrap.yml"),
            "---\n",
        )
        .unwrap();
        let off = joiner_checkout();
        std::fs::write(
            off.path().join("ops/ansible/group_vars/zagros_b_hosts/vars.yml"),
            "goatnet_founder: false\njoin_founder_site: zagros\nsite_enrollment_dns: netbird.example.net\nsignal_federation_absent_reason: not yet\n",
        )
        .unwrap();
        let shapes: Vec<(&Path, Options)> = vec![
            (t.path(), opts("zagros", "core", "zagros")),
            (t.path(), opts("zagros-b", "core", "zagros")),
            (off.path(), opts("zagros-b", "core", "zagros")),
            (t.path(), opts("zagros", "obs", "zagros")),
            (t.path(), opts("zagros-b", "obs", "zagros")),
            (t.path(), opts("zagros", "breakglass", "zagros")),
            (t.path(), opts("zagros", "network-jump", "zagros")),
            (
                t.path(),
                Options {
                    via: "door",
                    ..opts("zagros", "core", "zagros")
                },
            ),
        ];
        let mut planned: std::collections::BTreeMap<
            u32,
            (std::collections::BTreeSet<String>, Vec<String>),
        > = Default::default();
        for (r, o) in &shapes {
            for p in plan(r, o).unwrap() {
                let slot = planned.entry(p.n).or_default();
                slot.0.insert(p.kind.to_string());
                if p.kind != "MANUAL" {
                    slot.1.push(p.cmd);
                }
            }
        }
        let entries = goat_errors::phases::phases().of("join-node");
        let keys: std::collections::BTreeSet<String> =
            entries.iter().map(|e| e.phase.to_string()).collect();
        let want: std::collections::BTreeSet<String> =
            planned.keys().map(|n| n.to_string()).collect();
        assert_eq!(
            keys, want,
            "the catalog's join-node entries are the phases the plan can hold"
        );
        let source = include_str!("joinnode.rs");
        let mut wrong = Vec::new();
        for e in &entries {
            let n: u32 = e.phase.to_string().parse().unwrap();
            let (kinds, cmds) = &planned[&n];
            let said: std::collections::BTreeSet<String> =
                e.kind.split(" + ").map(str::to_string).collect();
            if &said != kinds {
                wrong.push(format!(
                    "phase {n}: kind `{}`, the plan's steps are {kinds:?}",
                    e.kind
                ));
            }
            let names = runs_names(&e.runs);
            for c in cmds {
                for r in command_runs(c) {
                    if !names.contains(&r) {
                        wrong.push(format!(
                            "phase {n}: a planned step runs {r}, which its `runs` does not name"
                        ));
                    }
                }
            }
            for name in &names {
                if !cmds.iter().any(|c| c.contains(name.as_str()))
                    && !source.contains(name.as_str())
                {
                    wrong.push(format!(
                        "phase {n}: `runs` names {name}, which no planned step runs"
                    ));
                }
            }
        }
        wrong.dedup();
        assert!(
            wrong.is_empty(),
            "the catalog does not match join-node's plan:\n  {}",
            wrong.join("\n  ")
        );
        let p13 =
            goat_errors::phases::explain_phase("join-node", "converge").expect("phase 13 by id");
        assert_eq!(p13.phase.to_string(), "13");
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

    /// Pass 1 runs site-bootstrap's `stack` plays only: the `after-ory` plays
    /// end on the management service, which waits for Hydra, and Ory is the
    /// next step. Pass 3, after Ory, runs the whole file, as standup's
    /// joiner does.
    #[test]
    fn pass_1_runs_the_stack_plays_only_and_pass_3_the_whole_file() {
        let t = joiner_checkout();
        let ph = plan(t.path(), &opts("zagros-b", "core", "zagros")).unwrap();
        let core: Vec<&Phase> = ph.iter().filter(|p| p.label.contains("[core, joiner]")).collect();
        assert!(core[0].label.contains("pass 1") && core[0].cmd.ends_with(" --skip-tags after-ory"), "{}", core[0].cmd);
        assert!(core[1].cmd.contains("ory-deploy.yml"), "Ory follows pass 1: {}", core[1].cmd);
        assert!(core[3].label.contains("pass 3") && !core[3].cmd.contains("--skip-tags"), "{}", core[3].cmd);
    }

    /// After the Spock wiring, a joiner's core node runs signal federation:
    /// the repo gate first, then the joiner's token and proxy before the
    /// founder's, the routers, the joiner's tunnel, the founder's relay pins,
    /// and the delivery proof last. Nothing in it names a site but the two
    /// the checkout declares.
    #[test]
    fn a_joiner_core_node_federates_signal_after_the_spock_wiring() {
        let t = joiner_checkout();
        let ph = plan(t.path(), &opts("zagros-b", "core", "zagros")).unwrap();
        let rewire = ph.iter().position(|p| p.label.contains("Spock wiring again")).unwrap();
        let fed: Vec<&Phase> = ph.iter().filter(|p| p.label.starts_with("[signal federation]")).collect();
        assert_eq!(fed.len(), 7, "{:?}", fed.iter().map(|p| &p.label).collect::<Vec<_>>());
        let first = ph.iter().position(|p| p.label.starts_with("[signal federation]")).unwrap();
        assert_eq!(first, rewire + 1, "federation follows the Spock wiring directly");
        assert!(fed.iter().all(|p| p.n == 13));
        assert_eq!(fed[0].kind, "GATE");
        assert!(fed[0].cmd.ends_with("signal-federation-check.py' zagros-b --peer zagros --resolve --require-env"), "{}", fed[0].cmd);
        assert!(fed[1].cmd.ends_with("signal-federation-transport.yml -e federation_site=zagros-b"), "joiner first: {}", fed[1].cmd);
        assert!(fed[2].cmd.ends_with("signal-federation-transport.yml -e federation_site=zagros"), "{}", fed[2].cmd);
        assert!(fed[3].cmd.contains("traefik-acme.yml -e target=zagros_b_hosts") && fed[3].cmd.contains("traefik-acme.yml -e target=zagros_hosts"), "{}", fed[3].cmd);
        assert!(fed[4].cmd.contains("-i inventory/hosts-zagros.ini") && fed[4].cmd.contains("site-rathole-client.yml -e rathole_client_target=zagros_b_hosts"), "{}", fed[4].cmd);
        assert!(fed[5].cmd.contains("-i inventory/zagros-goat-relay-alias.ini") && fed[5].cmd.contains("relay-ingress-add-site.yml -e target=goat_relay_hosts -e founder_site=zagros -e join_site=zagros-b"), "{}", fed[5].cmd);
        assert_eq!(fed[6].kind, "GATE");
        assert!(fed[6].cmd.ends_with("signal-federation-verify.yml -e federation_site=zagros-b -e federation_peer_site=zagros"), "{}", fed[6].cmd);
        assert!(fed.iter().all(|p| !p.cmd.contains("-e site_id")));
        // A founder keeping its own rathole-client play gets it.
        std::fs::write(t.path().join("ops/ansible/zagros-rathole-client.yml"), "---\n").unwrap();
        let ph = plan(t.path(), &opts("zagros-b", "core", "zagros")).unwrap();
        assert!(ph.iter().any(|p| p.cmd.contains(" zagros-rathole-client.yml -e rathole_client_target=zagros_b_hosts")));
        // The missing relay inventory stops the step with how to make it.
        let step = ph.iter().find(|p| p.cmd.contains("relay-ingress-add-site.yml")).unwrap();
        let o = Command::new("bash").args(["-euo", "pipefail", "-c", &step.cmd]).output().unwrap();
        assert!(!o.status.success() && String::from_utf8_lossy(&o.stdout).contains("generate-relays.sh zagros terraform-zagros"));
    }

    /// A joiner that declares federation off with a reason gets the gate (the
    /// two sides must agree) and a SKIP, and no federation play runs.
    #[test]
    fn a_joiner_that_declares_federation_off_gets_the_gate_and_a_skip() {
        let t = joiner_checkout();
        let gv = t.path().join("ops/ansible/group_vars/zagros_b_hosts/vars.yml");
        let mut text = std::fs::read_to_string(&gv).unwrap();
        text.push_str("signal_federation_absent_reason: one node holds every peer\n");
        std::fs::write(&gv, text).unwrap();
        let ph = plan(t.path(), &opts("zagros-b", "core", "zagros")).unwrap();
        let fed: Vec<&Phase> = ph.iter().filter(|p| p.label.starts_with("[signal federation]")).collect();
        assert_eq!(fed.iter().map(|p| p.kind).collect::<Vec<_>>(), vec!["GATE", "SKIP"]);
        assert!(ph.iter().all(|p| !p.cmd.contains("signal-federation-transport.yml") && !p.cmd.contains("relay-ingress-add-site.yml")));
        // A founder site's core node has no federation phase of its own.
        let gvf = t.path().join("ops/ansible/group_vars/zagros_hosts");
        std::fs::create_dir_all(&gvf).unwrap();
        std::fs::write(gvf.join("vars.yml"), "goatnet_founder: true\n").unwrap();
        assert!(plan(t.path(), &opts("zagros", "core", "zagros")).unwrap().iter().all(|p| !p.label.starts_with("[signal federation]")));
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

    /// `--via door` gives every play the base inventory and the door override,
    /// never the mesh override too, and the plan opens with the door step.
    #[test]
    fn via_door_uses_the_door_inventory_and_opens_the_door_first() {
        let t = joiner_checkout();
        let o = Options { via: "door", ..opts("zagros-b", "core", "zagros") };
        let r = execute(t.path(), &o).unwrap();
        // After the banner and the local probes that read the run's inputs, the
        // door is the first step: before any phase reaches a host.
        let first = r.checks.iter().position(|c| c.name != "banner" && c.name != "probe-ran").unwrap();
        assert_eq!(r.checks[first].name, "door", "{:?}", r.checks.iter().map(|c| &c.name).collect::<Vec<_>>());
        assert!(r.checks[first].detail.contains("open the env door to zagros"), "{}", r.checks[first].detail);
        assert!(r.checks[first + 1].detail.ends_with("/ops/scripts/door-open.sh' zagros"), "{}", r.checks[first + 1].detail);
        // (Phase 12 still reads the mesh inventory for TBD_: the node's mesh row
        // is owed whichever way this machine reaches it. No play is given it.)
        let plays: Vec<Phase> = plan(t.path(), &o).unwrap().into_iter().filter(|p| p.cmd.contains("ansible-preflight.sh")).collect();
        assert!(plays.len() >= 6);
        for ph in plays {
            assert!(!ph.cmd.contains("mgmt-hosts-mesh.ini"), "phase {}: {}", ph.n, ph.cmd);
            assert!(ph.cmd.contains("-i inventory/mgmt-hosts.ini -i inventory/doors-groupvars.ini"), "phase {}: {}", ph.n, ph.cmd);
        }
        // The default stays the mesh, with no door step.
        let mesh = opts("zagros-b", "core", "zagros");
        let r = execute(t.path(), &mesh).unwrap();
        assert!(r.checks.iter().all(|c| c.name != "door"));
        for ph in plan(t.path(), &mesh).unwrap() {
            assert!(!ph.cmd.contains("doors-groupvars.ini"), "phase {}: {}", ph.n, ph.cmd);
        }
        let bad = Options { via: "tunnel", ..opts("zagros-b", "core", "zagros") };
        assert!(execute(t.path(), &bad).unwrap_err().to_string().contains("--via 'tunnel'"));
    }

    /// At a joiner site an obs node skips the utility leg (the state of record
    /// stays with the founder) and says why; a founder site's obs node runs it.
    #[test]
    fn a_joiner_obs_node_skips_the_state_of_record_leg() {
        let t = joiner_checkout();
        let ph = plan(t.path(), &opts("zagros-b", "obs", "zagros")).unwrap();
        assert!(ph.iter().all(|p| !p.cmd.contains("ansible-preflight.sh") || !p.cmd.contains("utility-host-bootstrap.yml")), "a joiner never runs the utility play");
        let skip = ph.iter().find(|p| p.kind == "SKIP").expect("a SKIP line for the utility leg");
        assert!(skip.label.contains("state of record stays with the founder zagros"), "{}", skip.label);
        assert!(ph.iter().any(|p| p.cmd.contains("obs-host-bootstrap.yml -e target=zagros_b_obs_hosts")));

        let gv = t.path().join("ops/ansible/group_vars/zagros_hosts");
        std::fs::create_dir_all(&gv).unwrap();
        std::fs::write(gv.join("vars.yml"), "goatnet_founder: true\n").unwrap();
        std::fs::write(t.path().join("ops/ansible/utility-host-bootstrap.yml"), "---\n").unwrap();
        let ph = plan(t.path(), &opts("zagros", "obs", "zagros")).unwrap();
        assert!(ph.iter().all(|p| p.kind != "SKIP"));
        assert!(ph.iter().any(|p| p.cmd.contains("utility-host-bootstrap.yml -e target=zagros_obs_hosts")));
    }

    /// Phase 2 probes key auth + NOPASSWD sudo first and seeds only when the
    /// probe fails; the dry-run prints the probe.
    #[test]
    fn the_key_seed_is_skipped_when_the_box_already_takes_the_key() {
        let t = joiner_checkout();
        let o = Options { address: Some("192.0.2.9"), ..opts("zagros-b", "core", "zagros") };
        let p2 = plan(t.path(), &o).unwrap().into_iter().find(|p| p.n == 2).unwrap();
        assert!(p2.cmd.starts_with("if ssh -o BatchMode=yes -o ConnectTimeout=10 <deploy.ssh_user>@192.0.2.9 sudo -n true; then echo"), "{}", p2.cmd);
        assert!(p2.cmd.contains("seed skipped") && p2.cmd.contains("else '") && p2.cmd.contains("seed-ssh-key.sh' <deploy.ssh_user>@192.0.2.9 ZAGROS_B_NODE_PASS; fi"), "{}", p2.cmd);
        let r = execute(t.path(), &o).unwrap();
        assert!(r.checks.iter().any(|c| c.name == "cmd" && c.detail.contains("sudo -n true")));
    }

    /// Phase 6 lets through only the pre-enrolment wt0 check: a clean run and a
    /// run whose sole failure is that check pass; any other failure stops.
    #[test]
    fn phase_six_masks_only_the_wt0_absent_check() {
        let t = joiner_checkout();
        std::fs::create_dir_all(t.path().join("ops/scripts")).unwrap();
        // Stands in for with-aws + ansible: prints FAKE_LOG, exits FAKE_RC.
        let stub = t.path().join("ops/scripts/with-aws");
        crate::test_support::executable(&stub, "#!/bin/bash\nprintf '%s\\n' \"$FAKE_LOG\"\nexit \"$FAKE_RC\"\n");
        let p6 = plan(t.path(), &opts("zagros-b", "core", "zagros")).unwrap().into_iter().find(|p| p.n == 6).unwrap();
        // A failing run keeps its log (mktemp); TMPDIR puts it in the fixture,
        // so the test leaves nothing in the system temp directory.
        let run = |log: &str, rc: &str| {
            Command::new("bash")
                .args(["-euo", "pipefail", "-c", &p6.cmd])
                .env("FAKE_LOG", log)
                .env("FAKE_RC", rc)
                .env("TMPDIR", t.path())
                .output()
                .unwrap()
        };
        let wt0 = "TASK [wireguard-kmod : check current wt0 driver] ***\nok: [node-x]\nTASK [wireguard-kmod : verify wt0 is kernelspace wireguard] ***\nFAILED - RETRYING: [node-x]: verify wt0 is kernelspace wireguard (12 retries left).\nfatal: [node-x]: FAILED! => {\"attempts\": 12, \"cmd\": \"ip -d link show wt0\", \"stderr\": \"Device \\\"wt0\\\" does not exist.\"}";
        let other = "TASK [wireguard-kmod : install the module] ***\nfatal: [node-x]: FAILED! => {\"msg\": \"modprobe: FATAL: Module wireguard not found\"}";
        let unreachable = "TASK [Gathering Facts] ***\nfatal: [node-x]: UNREACHABLE! => {\"msg\": \"ssh: connect to host timed out\"}";
        assert!(run("PLAY RECAP ok=9 failed=0", "0").status.success());
        let o = run(wt0, "2");
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stdout));
        assert!(String::from_utf8_lossy(&o.stdout).contains("expected; phase 11 re-runs it"));
        for log in [other, unreachable, &format!("{wt0}\n{other}")] {
            let o = run(log, "2");
            assert!(!o.status.success(), "must surface: {log}");
            assert!(String::from_utf8_lossy(&o.stdout).contains("something other than the pre-enrolment wt0 check"));
        }
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
