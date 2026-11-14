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
//!   8  GATE   the site declares mesh_enrollment_url, where its nodes enrol
//!   9  GATE   preflight-enrollment.sh on the node (clock, stale wt0, apex reach);
//!             no setup key is sent (--key-at-enrolment)
//!  10  A      mesh-enroll-host.yml: a single-use, one-hour key minted where the
//!             NetBird database is (a joiner: the founder's first host), with the
//!             groups that host's peer holds, handed to `netbird up` through a
//!             0600 file on the node; never on this machine. GATE: the kernel
//!             interface and `table ip netbird`, not "Connected". T: the mesh
//!             address into the checkout (standup-mesh-record.py --no-mint)
//!  11  A      wireguard-kmod.yml again — changed=0 proves the kmod took
//!  12  GATE   mesh_ip + the mesh inventory filled: preflight passes with no TBD_
//!  13  A      the class's converge: core (founder pass, or a joiner's three
//!             passes + the founder-side mirror, then signal federation: a
//!             gate on both sides' config, the relay pins and DNS, the token
//!             and proxy on each side, the routers, the relay pin, the
//!             delivery proof, then the site's data router), obs (the three obs legs; at
//!             a joiner site the utility leg is skipped — the pub/sub state of
//!             record stays with the founder), breakglass, network-jump
//!  14  T      unmask the apt timers
//!  15  GATE   acceptance: every applicable row PASS, from the new node's context
//!
//! A phone node (`--class phone --via usb`) differs in three phases: 1 is
//! attaching the phone by USB; 2 is T + GATE, phone-node-provision.sh preparing
//! the phone's Linux VM and then its integrity checks; 14 reaches the node at
//! the mesh address phase 10 recorded.
//!
//! `--via mesh` (default) reaches the goatnet's hosts over this machine's mesh
//! (`mgmt-hosts.ini` + `mgmt-hosts-mesh.ini`); `--via door` over its env door
//! (`mgmt-hosts.ini` + `doors-groupvars.ini`, docs/design/env-doors.md), for a
//! machine not on that goatnet's mesh. The two transports are never combined.
//!
//! `--class phone --via usb` (docs/design/phone-node.md; gate rows H3 to H5):
//! the node is the Linux VM on an Android phone and converges as a `core`
//! joiner. Until it has a mesh address (phases 1 to 11) it is reached over the
//! Android debug bridge on a USB cable: before the first phase the run writes
//! an SSH configuration and an Ansible inventory line for the node into the
//! state directory (phone-node-ssh.sh), so none is written by hand and
//! ~/.ssh/config is not read. From phase 12 it is reached at its mesh address,
//! as every other node, and the goatnet's other hosts over this machine's
//! mesh throughout. Phase 1 is attaching the phone; phase 2 prepares its VM
//! and refuses unless the integrity checks pass (phone-node-provision.sh);
//! the config preflight judges the site with the class's declarations
//! (`--class phone`, ops/ansible/group_vars/class_phone.yml).
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
    /// How this machine reaches the goatnet's hosts: `mesh` or `door`; `usb`
    /// for a phone node, over the Android debug bridge until it is enrolled.
    pub via: &'a str,
    pub from: u32,
    pub to: u32,
    pub dry_run: bool,
}

pub const CLASSES: [&str; 5] = ["core", "obs", "breakglass", "network-jump", "phone"];
pub const VIAS: [&str; 3] = ["mesh", "door", "usb"];

/// What a class converges as: a phone node is a `core` joiner whose class
/// adds declared defaults (ops/ansible/group_vars/class_phone.yml).
fn converges_as(class: &str) -> &str {
    if class == "phone" {
        "core"
    } else {
        class
    }
}

/// The inventories a play is given for each transport. Base inventory first
/// (group membership, site facts, router_role); then exactly one transport
/// override. Passing both overrides lets whichever loads last win silently.
/// `usb` reaches the goatnet's hosts over the mesh; the node's own line, over
/// the Android debug bridge, is added for the phases before it has a mesh
/// address (`usb_base`).
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

/// Where a `--via usb` run keeps what it writes for the node, in the state
/// directory: `<base>.ssh` (the SSH configuration), `<base>.ini` (the Ansible
/// inventory line) and `<base>.known_hosts`.
pub(crate) fn usb_base(repo: &Path, site: &str, node: &str) -> String {
    format!("{}/join-node-{site}-{node}-usb", crate::settings::state_dir_abs(repo))
}

/// The step that writes the node's SSH configuration and inventory line,
/// printed first in a `--via usb` plan and run before the first phase (it
/// writes the same two files every time).
fn usb_setup_cmd(repo: &Path, o: &Options) -> String {
    format!("'{}/ops/scripts/phone-node-ssh.sh' --write {} --node {}", repo.display(), sh_quote(&usb_base(repo, o.site, o.node)), o.node)
}

struct Phase {
    n: u32,
    kind: &'static str,
    label: String,
    cmd: String,
    /// Files in the checkout the phase may write, relative to it: the run
    /// names each one it changed in its summary and its run record.
    writes: Vec<String>,
}

fn row(name: &str, detail: String) -> Check {
    Check { name: name.into(), ok: true, detail, fix: None }
}

/// The inventory group a node of `class` sits in at a site: the core group for
/// `core`, `<site>_<class>_hosts` otherwise (obs, breakglass, network-jump).
fn class_group(site_us: &str, class: &str) -> String {
    match class {
        "core" | "phone" => format!("{site_us}_hosts"),
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

/// The services stage-service-leaves.yml stages a leaf for (its
/// `service_leaf_map`). A test holds this list in step with the play.
const STAGED_SERVICES: [&str; 9] = ["fabric-health", "mesh-health", "presence-health", "time-health", "peer-health", "consistency-health", "fabric-crawler", "goat-topology", "audit-subscriber"];

/// A joining site's leaves, from its manifest
/// (`ops/enrollment/manifests/<site>-certs.yaml`): where mint-site-certs.sh
/// writes them (the manifest's `out_dir`, else `<state dir>/<site>-leaves`),
/// and the service leaves it carries that the staging play knows, in the
/// manifest's order.
pub(crate) struct Leaves {
    pub(crate) out_dir: String,
    pub(crate) staged: Vec<String>,
}

pub(crate) fn leaves(repo: &Path, site: &str) -> Result<Leaves> {
    let rel = format!("ops/enrollment/manifests/{site}-certs.yaml");
    let Ok(text) = std::fs::read_to_string(repo.join(&rel)) else {
        return Err(goat_errors::coded("GB-286", format!("{site} has no leaf-cert manifest at {rel}: join-node mints the site's router and service leaves from it before the data router. Copy ops/enrollment/manifests/TEMPLATE-certs.yaml to {rel} and re-thread every value for {site} (its URI SANs name this goatnet)")).into());
    };
    let doc: serde_yaml::Value = serde_yaml::from_str(&text).map_err(|e| goat_errors::coded("GB-286", format!("{rel} is not readable YAML: {e}")))?;
    let names: Vec<String> = doc.get("leaves").and_then(|l| l.as_sequence()).map(|l| l.iter().filter_map(|x| x.get("name")?.as_str().map(String::from)).collect()).unwrap_or_default();
    if !names.iter().any(|n| n == "router") {
        return Err(goat_errors::coded("GB-287", format!("{rel} has no router leaf: the data router's play (zenoh-deploy.yml) reads it. Add a leaf named router, as ops/enrollment/manifests/TEMPLATE-certs.yaml shows")).into());
    }
    let out_dir = doc.get("out_dir").and_then(|v| v.as_str()).map(String::from).unwrap_or_else(|| format!("{}/{site}-leaves", crate::settings::state_dir_abs(repo)));
    let staged = names.into_iter().filter(|n| STAGED_SERVICES.contains(&n.as_str())).collect();
    Ok(Leaves { out_dir, staged })
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
///
/// Returns whether the federation steps run, and with them the joiner's
/// front door (traefik-acme.yml); false when the site declares it off.
fn signal_federation(ev: &mut Vec<Phase>, repo: &Path, gv: &Path, site: &str, founder: &str, pre: &str, inv: &str) -> bool {
    let r = repo.display().to_string();
    let site_group = format!("{}_hosts", site.replace('-', "_"));
    let founder_group = format!("{}_hosts", founder.replace('-', "_"));
    let p = |kind: &'static str, label: String, cmd: String| Phase { n: 13, kind, label, cmd, writes: Vec::new() };
    // The founder site is read from vars.yml: one shell word wherever it is used.
    let w = |v: String| sh_word(&v);
    ev.push(p("GATE", format!("[signal federation] the repo declares it on both sides: {site_group} and {founder_group} crossed and complete, {founder}'s relay inventory pins each federation hostname to its own node, the names resolve to the relays, the shared token is in the environment"), format!("python3 '{r}/ops/scripts/signal-federation-check.py' {site} --peer {} --resolve --require-env", w(founder.to_string()))));
    if !gv_first(gv, "signal_federation_absent_reason").is_empty() {
        ev.push(p("SKIP", format!("[signal federation] declared off at {site} (signal_federation_absent_reason in group_vars/{site_group}); the gate above proved no side expects it: {founder} is off too, or federates with another site"), format!("echo '>> signal federation: declared off for {site} (signal_federation_absent_reason); nothing to converge'")));
        return false;
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
    true
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
    // A phone node's site is judged with its class's declarations under its
    // own (`--class phone`, gate row H5).
    let preflight_target = match o.class {
        "core" => group.clone(),
        "phone" => format!("{group} --class phone"),
        "obs" => format!("{site_group} --obs {group}"),
        _ => site_group.clone(),
    };
    let converge = converges_as(o.class);
    let phone = o.class == "phone";
    let usb = o.via == "usb";
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
    // `--via usb`: until the node has a mesh address (phases 1 to 11) ssh uses
    // the configuration the run wrote, and the plays the inventory line that
    // names it, loaded after the goatnet's inventories. From phase 12 the
    // node is reached at its mesh address, as every other node.
    let base = usb_base(repo, site, node);
    let node_ssh = if usb { format!("ssh -F {} {user_w}@{node}", sh_quote(&format!("{base}.ssh"))) } else { format!("ssh {user_w}@{addr}") };
    let inv_before = if usb { format!("{inv} -i {}", sh_quote(&format!("{base}.ini"))) } else { inv.to_string() };
    let pass_var = o.pass_var.map(String::from).unwrap_or_else(|| format!("{upper}_NODE_PASS"));
    let gv = repo.join("ops/ansible/group_vars").join(&site_group).join("vars.yml");
    let joiner = gv_matches(&gv, "goatnet_founder", "false");
    let founder_site = gv_first(&gv, "join_founder_site");
    let founder_group = if founder_site.is_empty() { String::new() } else { format!("{}_hosts", founder_site.replace('-', "_")) };
    let intra_pair = gv_matches(&gv, "intra_site_pair", "true");
    let p = |n: u32, kind: &'static str, label: &str, cmd: String| Phase { n, kind, label: label.to_string(), cmd, writes: Vec::new() };

    // A joiner enrols into the goatnet it joins: the founder's control plane.
    let enrol_at = if joiner && !founder_site.is_empty() { founder_site.as_str() } else { site };
    // The enrolment apex the preflight checks: the goatnet's, declared in the
    // site's core group (a joiner's names the goatnet it joins).
    let enrol_dns = { let d = gv_first(&gv, "site_enrollment_dns"); if d.is_empty() { format!("<site_enrollment_dns of {site_group}>") } else { d } };
    // Phase 10 mints the setup key where the NetBird database is: a joiner
    // site has none yet, so the key is minted on the founder's first host.
    let mint_group = if joiner && !founder_group.is_empty() { founder_group.clone() } else { site_group.clone() };
    let mesh_ips = format!("{}/join-node-{site}-{node}-mesh-ips.json", crate::settings::state_dir_abs(repo));
    // The node's mesh address, into the checkout: the one edit a person made
    // between phases 10 and 12. A node of another class sets only its mesh
    // inventory line; the core pair's addresses are the core node's.
    let record = {
        let group_arg = if converge == "core" { String::new() } else { format!(" --group {group}") };
        let mut writes = vec!["ops/ansible/inventory/mgmt-hosts-mesh.ini".to_string()];
        if converge == "core" {
            writes.insert(0, format!("ops/ansible/group_vars/{site_group}/vars.yml"));
            writes.push(format!("ops/enrollment/manifests/{site}-certs.yaml"));
        }
        Phase { n: 10, kind: "T", label: "record the node's mesh address in the checkout (group_vars, mesh inventory) for phase 12; the router certificate is left as it is".into(), cmd: format!("python3 '{r}/ops/scripts/standup-mesh-record.py' {site} --ips {}{group_arg} --no-mint", sh_quote(&mesh_ips)), writes }
    };
    // Phase 1 is the one step for a person: reaching the node. A node on a
    // USB cable is reached by attaching the phone.
    let reach = if usb {
        p(1, "MANUAL", "attach the phone by USB", "attach the phone to this machine by USB, allow USB debugging on it, accept this machine's key on its screen, and unlock it — `adb devices` then lists it as `device`. With more than one device attached, set ANDROID_SERIAL. A phone node has no LAN address: pass no --address.".to_string())
    } else {
        p(1, "MANUAL", "reach the box on the SITE LAN", format!("ssh {user}@{addr} — find the box's real LAN address before anything else and pass it as --address. On a Jetson the address handed over may be 192.168.55.1: that is l4tbr0, the USB device-mode bridge on the box itself, reachable only over a direct USB-C cable."))
    };
    // Phase 2 gives the plays key auth and NOPASSWD sudo. For a phone it
    // prepares the VM, which creates the deploy user with both, and refuses
    // unless the integrity checks pass (gate row H4).
    let prepare = format!("'{r}/ops/scripts/phone-node-provision.sh' --node {node} --user {user_w} --base {}", sh_quote(&base));
    let key = if phone {
        vec![
            p(2, "T", "prepare the phone's Linux VM: Debian's kernel with the boot settings, Docker Engine, the deploy user with key auth and NOPASSWD sudo — a VM already prepared goes straight to the checks", format!("{prepare} --prepare")),
            p(2, "GATE", "the VM's integrity checks: Debian's kernel booted, namespaces, the same bytes on every read, a write read back, installed packages verified, no I/O error in the kernel log, WireGuard, a container run — nothing is built on a VM that fails one (GB-300)", format!("{prepare} --check")),
        ]
    } else {
        vec![p(2, "T", "seed the SSH key (key auth + NOPASSWD sudo; password from the env var, never the command line) — skipped when key auth and NOPASSWD sudo already work", format!("if ssh -o BatchMode=yes -o ConnectTimeout=10 {user_w}@{addr} sudo -n true; then echo {}; else '{r}/ops/scripts/with-secrets' '{r}/ops/scripts/seed-ssh-key.sh' {user_w}@{addr} {pass_var}; fi", sh_quote(&format!(">> {user}@{addr}: key auth and NOPASSWD sudo already work — seed skipped"))))]
    };
    let mut ev = vec![reach];
    ev.extend(key);
    ev.extend([
        // Phases 3 and 7 run before enrolment: the mesh-address placeholder
        // phase 12 fills is a warning there (--before-enrolment), not a failure.
        p(3, "GATE", "repo work first: group, inventory, pins, exclusions — the config preflight passes (the mesh-address placeholder is allowed until phase 12)", format!("python3 '{r}/ops/scripts/standup-config-preflight.py' {preflight_target} --before-enrolment")),
        p(4, "T", "quiet apt on the node (the baseline stalls on the dpkg lock otherwise; unmasked in phase 14)", format!("{node_ssh} 'sudo systemctl mask --now apt-daily.timer apt-daily-upgrade.timer'")),
        p(5, "A", "host baseline, narrowed to the node — run detached if the session may drop: it apt-upgrades and reboot-waves", format!("cd '{r}/ops/ansible' && {pre} {inv_before} mgmt-host-baseline.yml -e target={group} -e site_id={site} -l {node}")),
        p(6, "A", "WireGuard kernel modules, first pass — its wt0 check fails before enrolment (wt0 absent); expected, phase 11 re-runs it. Any other failure stops the run", format!("cd '{r}/ops/ansible' && log=$(mktemp) && if {pre} {inv_before} wireguard-kmod.yml -e target={group} -l {node} 2>&1 | tee \"$log\"; then rm -f \"$log\"; elif {WT0_ABSENT_ONLY} \"$log\" && grep -q 'wt0.*does not exist' \"$log\"; then rm -f \"$log\"; echo '>> wireguard-kmod: the wt0 check failed before enrolment (wt0 absent) — expected; phase 11 re-runs it for changed=0'; else echo \"!! wireguard-kmod failed on something other than the pre-enrolment wt0 check — output kept in $log\"; exit 1; fi")),
        p(7, "GATE", "the CA this site trusts is its own, not the shared dev root (silence puts a production node on the dev CA)", format!("python3 '{r}/ops/scripts/standup-config-preflight.py' {preflight_target} --before-enrolment && ! grep -rqE 'ca_id:[[:space:]]*\"?[a-z]+-goat-ca-' '{r}/ops/ansible/group_vars/{group}/' || {{ echo 'group_vars/{group} names the unnamed default CA (<mode>-goat-ca-<date>); a site node trusts its goatnet CA'; exit 1; }}")),
        p(8, "GATE", "the site declares where its nodes enrol (mesh_enrollment_url): phase 10 mints the setup key and enrols the node, and refuses before any key exists without it", format!("grep -rqsE '^mesh_enrollment_url:[[:space:]]*\\\"?https://' '{r}/ops/ansible/group_vars/{group}/' '{r}/ops/ansible/group_vars/{site_group}/' || {{ echo {}; exit 1; }}", sh_quote(&format!("set mesh_enrollment_url in group_vars/{site_group}/vars.yml: https://<the enrolment name of {enrol_at}>:443 (it resolves to the relays) or :8443 (it resolves to the host itself)")))),
        p(9, "GATE", "enrollment preflight on the node (clock skew, a stale wt0, an unreachable apex — each turns enrolment into a hang); no setup key is sent: phase 10 mints it where it is used", format!("{{ printf 'export NETBIRD_MANAGEMENT_URL=%q\\n' {}; cat '{r}/ops/preflight-enrollment.sh'; }} | {node_ssh} 'bash -s -- --key-at-enrolment'", sh_quote(&format!("https://{enrol_dns}")))),
        p(10, "A", &format!("enrol the node: a single-use, one-hour setup key minted on the first host of {mint_group} (where the NetBird database is), with the groups that host's peer holds, handed to netbird up through a 0600 file on the node and removed; the key never reaches this machine"), format!("cd '{r}/ops/ansible' && {pre} {inv_before} mesh-enroll-host.yml -e target={group} -l {node} -e mesh_enroll_mint_group={} -e mesh_enroll_groups_from_peer=true -e mesh_ips_out={}", sh_word(&mint_group), sh_quote(&mesh_ips))),
        p(10, "GATE", "the node is on the kernel ACL plane, not only \"Connected\": netbird's interface is the kernel's, and `table ip netbird` is loaded", format!("{node_ssh} 'sudo netbird status | grep -q \"Interface type: Kernel\" && sudo nft list tables | grep -qx \"table ip netbird\"'")),
        record,
        p(11, "A", "WireGuard kernel modules again — changed=0 proves the kmod took, now that wt0 exists", format!("cd '{r}/ops/ansible' && {pre} {inv_before} wireguard-kmod.yml -e target={group} -l {node}")),
        p(12, "GATE", "mesh_ip + the mesh inventory filled in one change (mesh IPs are not stable across re-enrolment) — preflight passes with no TBD_", format!("python3 '{r}/ops/scripts/standup-config-preflight.py' {preflight_target} && ! grep -rqn 'TBD_' '{r}/ops/ansible/group_vars/{group}/' && ! grep -n 'TBD_' '{r}/ops/ansible/inventory/mgmt-hosts-mesh.ini' | grep -qw {node}")),
    ]);
    let ev_secrets = format!("-e bootstrap_owner_email=\"$OWNER_EMAIL\" -e 'postgres_superuser_password={{{{ lookup(\"env\", \"{upper}_PG_PASS\") }}}}' -e 'netbird_encryption_key={{{{ lookup(\"env\", \"{upper}_NB_ENCKEY\") }}}}' -e 'pgbackrest_cipher_pass={{{{ lookup(\"env\", \"{upper}_BKP_KEY\") }}}}'");
    let ev_bootstrap = format!("-e site_id={site} {ev_secrets}");
    match converge {
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
            let federated = signal_federation(&mut ev, repo, &gv, site, &founder_site, &pre, inv);
            // The site's router and service leaves (gate row I3), before the
            // router, whose play reads the router leaf.
            let l = leaves(repo, site)?;
            ev.push(p(13, "T", &format!("[certificates] mint the site's leaves from ops/enrollment/manifests/{site}-certs.yaml (idempotent: a leaf already minted is kept)"), format!("'{r}/ops/scripts/mint-site-certs.sh' {}", sh_word(site))));
            if !l.staged.is_empty() {
                let only = format!("{{\"leaf_only\": [{}]}}", l.staged.iter().map(|n| format!("\"{n}\"")).collect::<Vec<_>>().join(", "));
                ev.push(p(13, "A", &format!("[certificates] stage the service leaves the manifest carries onto the node ({})", l.staged.join(", ")), format!("cd '{r}/ops/ansible' && {pre} {inv} stage-service-leaves.yml -e target={site_group} -l {node} -e leaf_src_dir={} -e {}", sh_word(&l.out_dir), sh_quote(&only))));
            }
            // The site's own data router (gate row I3): its fabric does not
            // depend on reaching the founder's. The play's pre-flight refuses
            // a node without its router leaf certificate.
            ev.push(p(13, "A", "[data router] the site's own Zenoh router (zenoh-deploy.yml), limited to the node: the site's fabric runs on its own router, not the founder's", format!("cd '{r}/ops/ansible' && {pre} {inv} zenoh-deploy.yml -e target={site_group} -l {node}")));
            // The site's front door (gate row I3). Signal federation sets it up
            // with the federation routers; a site that declares federation off
            // gets it here, for its profile: an internet site (ACME delivery,
            // as standup reads it) its public certificate and routes, a LAN
            // site its internal-CA routes, whose leaf is minted above.
            if !federated {
                if gv_matches(&gv, "dashboard_cert_delivery_mode", "acme") {
                    ev.push(p(13, "A", "[front door] the site's public certificate and routes (traefik-acme.yml, ACME DNS-01), limited to the node: an internet site (dashboard_cert_delivery_mode: acme) whose signal federation, which sets the door up otherwise, is declared off", format!("cd '{r}/ops/ansible' && {pre} {inv} traefik-acme.yml -e target={site_group} -l {node}")));
                } else {
                    ev.push(p(13, "A", "[front door] the site's LAN TLS routes (lan-traefik-routes.yml, the internal-CA mgmt-identity leaf), limited to the node: a LAN site (no dashboard_cert_delivery_mode: acme) whose signal federation is declared off", format!("cd '{r}/ops/ansible' && {pre} {inv} lan-traefik-routes.yml -e target={site_group} -e site_id={site} -l {node}")));
                }
            }
            // The health publishers the site declares (gate row I3), on the
            // router, with the service leaves staged above.
            match declared_publishers(repo, site)? {
                Ok(pubs) if pubs.is_empty() => {
                    let why = format!("[health publishers] none deployed: the end-state model marks no health publisher present at {site}");
                    ev.push(p(13, "SKIP", &why, format!("echo {}", sh_quote(&format!(">> {why}")))));
                }
                Ok(pubs) => {
                    for (name, play) in pubs {
                        ev.push(p(13, "A", &format!("[health publishers] {name} ({play}), limited to the node: the end-state model marks it present at {site}"), format!("cd '{r}/ops/ansible' && {pre} {inv} {play} -e target={site_group} -l {node}")));
                    }
                }
                Err(why) => {
                    let label = format!("[health publishers] not deployed: the end-state model was not read ({why}), so which publishers {site} declares is not known; fix that and resume with --from 13");
                    ev.push(p(13, "SKIP", &label, format!("echo {}", sh_quote(&format!(">> {label}")))));
                }
            }
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
    let unmask = "'sudo systemctl unmask apt-daily.timer apt-daily-upgrade.timer'";
    if usb {
        // The node has a mesh address by now; phase 10 recorded it in the mesh inventory.
        let missing = sh_quote(&format!("inventory/mgmt-hosts-mesh.ini gives {node} no mesh address under [{group}]; phase 10 records it"));
        ev.push(p(14, "T", "unmask the apt timers now that the node is finished (over the mesh: the node's address is the one phase 10 recorded)", format!("cd '{r}/ops/ansible' && t=$(python3 ../scripts/inventory-hosts.py {group} --inventory inventory/mgmt-hosts-mesh.ini --ssh-target --user {user_w} --host {node}) && [ -n \"$t\" ] || {{ echo {missing}; exit 1; }}; ssh \"$t\" {unmask}")));
    } else {
        ev.push(p(14, "T", "unmask the apt timers now that the node is finished", format!("ssh {user_w}@{addr} {unmask}")));
    }
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
    if o.via == "usb" {
        v.push(usb_setup_cmd(repo, o));
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

/// The end-state model's services at `site` (`endstate.py model <site>`),
/// or why the model could not be read: the script is not in the checkout,
/// would not start, failed, or printed no services.
fn model_services(repo: &Path, site: &str) -> std::result::Result<serde_json::Map<String, serde_json::Value>, String> {
    let shown = format!("{MODEL} model {site}");
    let script = repo.join(MODEL);
    if !script.is_file() {
        return Err(format!("{MODEL} is not in this checkout"));
    }
    let python = crate::endstate::python();
    let out = Cmd::new(&python).arg(&script).args(["model", site, "--root"]).arg(repo).current_dir(repo).output().map_err(|e| format!("cannot run {python} for {shown}: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
        let how = out.status.code().map_or("it was killed".to_string(), |c| format!("exit {c}"));
        return Err(format!("{shown} failed ({how}){}", if last.is_empty() { String::new() } else { format!(": {last}") }));
    }
    let model = serde_json::from_slice::<serde_json::Value>(&out.stdout).map_err(|_| format!("{shown} printed no model"))?;
    match model.get("services").and_then(|s| s.as_object()) {
        Some(services) => Ok(services.clone()),
        None => Err(format!("{shown} printed no services")),
    }
}

/// The health publishers join-node deploys at a joining site's core node, in
/// the order it runs them, each with its play. The service names are the
/// end-state catalog's (ops/goatbuilder/endstate/catalog.yaml).
const PUBLISHERS: [(&str, &str); 6] = [
    ("fabric-health", "fabric-health-deploy.yml"),
    ("mesh-health", "mesh-health-deploy.yml"),
    ("presence-health", "presence-health-deploy.yml"),
    ("time-health", "time-health-deploy.yml"),
    ("consistency-health", "consistency-health-deploy.yml"),
    ("fabric-crawler", "fabric-crawler-deploy.yml"),
];

/// The health publishers a joining site declares (gate row I3): each one the
/// end-state model marks present at the site, never one marked absent or left
/// undecided. A present one whose play is not in the checkout is refused here
/// (GB-288). `Ok(Err(why))` is a model that could not be read.
fn declared_publishers(repo: &Path, site: &str) -> Result<std::result::Result<Vec<(&'static str, &'static str)>, String>> {
    let services = match model_services(repo, site) {
        Ok(s) => s,
        Err(why) => return Ok(Err(why)),
    };
    let present: Vec<(&str, &str)> = PUBLISHERS.iter().copied().filter(|(name, _)| services.get(*name).and_then(|s| s.get("present")) == Some(&serde_json::Value::Bool(true))).collect();
    let missing: Vec<String> = present.iter().filter(|(_, play)| !repo.join("ops/ansible").join(play).is_file()).map(|(name, play)| format!("{name} (ops/ansible/{play})")).collect();
    if !missing.is_empty() {
        return Err(goat_errors::coded("GB-288", format!("the end-state model marks {} present at {site}, and this checkout has no play for {}: join-node deploys each health publisher the site declares. Bring the play into the checkout, or declare the publisher absent at {site} in ops/pins/fleet-pins.yaml with its reason", missing.join(", "), if missing.len() == 1 { "it" } else { "them" })).into());
    }
    Ok(Ok(present))
}

/// One service the site does not run yet, as the plan prints it.
fn not_run_line(service: &str, reason: &str) -> String {
    format!("{service} \u{2014} {reason}")
}

/// What the site does not run yet (docs/design/goatbuilder.md, `join-node`
/// (d)), under one heading: each gap the end-state model names here ("declared,
/// not yet built at this site", with the gate row that closes it), then each
/// service the site turns off, with the model's reason. Every site is declared
/// to run every service (ADR 1165, gate group I), so a service a joining site
/// does not run yet is a gap, never a design. The model is read, never
/// re-derived, so this list and `goatbuilder endstate`'s GAP and N/A rows
/// cannot disagree. A model that cannot be read is one `not read` line naming
/// what is missing; the run does not stop for it, and no list is guessed.
fn not_run(repo: &Path, site: &str) -> Vec<Check> {
    let shown = format!("{MODEL} model {site}");
    let mut rows = vec![row("not-run-heading", format!("What {site} does not run yet: each gap the end-state model names, with the gate row that closes it, and each service the site turns off ({shown})"))];
    let services = match model_services(repo, site) {
        Ok(s) => s,
        Err(why) => {
            rows.push(row("not-run", format!("not read: {why}; this list of what the site does not run yet is left out")));
            return rows;
        }
    };
    let before = rows.len();
    // The gaps first: declared here, not yet built at this site.
    for (name, s) in &services {
        let Some(g) = s.get("gap").filter(|g| g.is_object()) else {
            continue;
        };
        let field = |k: &str| g.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        rows.push(row(
            "not-run",
            not_run_line(
                name,
                &format!(
                    "declared, not yet built at this site: {} (gate row {})",
                    field("why"),
                    field("row")
                ),
            ),
        ));
    }
    // Then what the site turns off itself.
    for (name, s) in &services {
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
        rows.push(row("not-run", "nothing: the model names no gap here, and the site turns nothing off".into()));
    }
    rows
}

/// The plan and the run, once the environment, site and class are known.
fn execute(repo: &Path, o: &Options) -> Result<Report> {
    if !VIAS.contains(&o.via) {
        return Err(goat_errors::coded("GB-242", format!("--via '{}' is not a transport; known: {} (mesh = this machine is on the goatnet's mesh; door = through its env door, docs/design/env-doors.md)", o.via, VIAS.join(", "))).into());
    }
    // A phone node and the USB transport go together: the VM has no address
    // another machine can reach until it is enrolled, and only a phone's VM
    // answers over the Android debug bridge.
    if (o.class == "phone") != (o.via == "usb") {
        return Err(goat_errors::coded("GB-242", format!("--class {} with --via {}: a phone node is reached over USB until it is enrolled, and --via usb reaches nothing else. Pass --class phone --via usb together (docs/design/phone-node.md)", o.class, o.via)).into());
    }
    if o.via == "usb" && o.address.is_some() {
        return Err(goat_errors::coded("GB-242", "--address with --via usb: a phone node has no LAN address; the run finds its VM over the Android debug bridge. Leave --address out").into());
    }
    if o.class == "phone" {
        let gv = repo.join("ops/ansible/group_vars").join(class_group(&o.site.replace('-', "_"), o.class)).join("vars.yml");
        if !gv_matches(&gv, "goatnet_founder", "false") {
            return Err(goat_errors::coded("GB-301", format!("--class phone at {}, which does not declare goatnet_founder: false: a phone node joins a goatnet. It takes no public ingress, so it is not the site other nodes enrol at (docs/design/phone-node.md)", o.site)).into());
        }
    }
    if !o.dry_run && o.address.is_none() && o.via != "usb" {
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
    let resume = format!("goatbuilder join-node {} --site {} --node {} --class {}{}{}", o.env, o.site, o.node, o.class, o.address.map(|a| format!(" --address {a}")).unwrap_or_default(), match o.via {
        "door" => " --via door",
        "usb" => " --via usb",
        _ => "",
    });
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
    // The node's SSH configuration and inventory line, for the phases before
    // it has a mesh address. Nothing is written when none of them runs.
    if o.via == "usb" && o.from <= 11 {
        let setup = usb_setup_cmd(repo, o);
        checks.push(row("usb", format!("=== Before phase 1: write the SSH configuration and the inventory line that reach {} over the Android debug bridge (phases 1 to 11; from phase 12 it is reached at its mesh address) ===", o.node)));
        checks.push(row("cmd", setup.clone()));
        if !o.dry_run {
            crate::runner::phase("usb", &format!("write the SSH configuration for {} over USB", o.node), "");
            let status = Cmd::new("bash").args(["-euo", "pipefail", "-c", &setup]).current_dir(repo).status().context("bash")?;
            if !status.success() {
                checks.push(Check { name: "failed".into(), ok: false, detail: format!("!! the SSH configuration for {} could not be written — no phase has run", o.node), fix: Some(format!("check the state directory ({}) is writable; then re-run: {resume} --from {}", crate::settings::state_dir_abs(repo), o.from.max(1))) });
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
        let before: Vec<Option<Vec<u8>>> = ph.writes.iter().map(|f| std::fs::read(repo.join(f)).ok()).collect();
        let status = Cmd::new("bash").args(["-euo", "pipefail", "-c", &ph.cmd]).current_dir(repo).status().context("bash")?;
        // Each checkout file the phase changed, in the summary and the run
        // record, so a person sees the checkout changed and commits it.
        for (f, was) in ph.writes.iter().zip(before) {
            if std::fs::read(repo.join(f)).ok() != was {
                checks.push(row("wrote", format!("wrote {f} — review it and commit it")));
                crate::runner::wrote(f);
            }
        }
        if !status.success() {
            let rc = status.code().unwrap_or(1);
            checks.push(Check { name: "failed".into(), ok: false, detail: format!("!! Phase {} (\"{}\") FAILED — exit {rc}. No later phase has run; the node is left partway through phase {}. `--dry-run` prints this phase's command and the gate that proves it.", ph.n, ph.label, ph.n), fix: Some(format!("fix the cause, then resume — every phase is idempotent: {resume} --from {}", ph.n)) });
            // F5: the phase's output, as the runner kept it, matched against
            // the error catalog (explain_phase::interpreted), as standup does.
            let id = ph.n.to_string();
            checks.extend(crate::explain_phase::interpreted("join-node", &id, crate::explain_phase::runner_log(&id).as_deref()));
            exit = EXIT_REFUSED;
            break;
        }
    }
    // What the site does not run yet, in the plan and in a real run's summary alike.
    checks.extend(not_run(repo, o.site));
    if o.dry_run {
        checks.extend(crate::inputs::rows(&listed));
        let needs = if o.via == "usb" { "takes no --address: the phone is found over USB" } else { "needs --address" };
        checks.push(row("footer", format!("PLAN complete. Remove --dry-run to execute ({needs}). Phase 1, reaching the node, is the one manual step: it pauses the run; resume with --from 2. docs/tutorials/02-first-federated-node.md walks through it.")));
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

    /// Gate row H2: only phase 1 is a person's step. Phase 9 sends no setup
    /// key; phase 10 mints it where the NetBird database is (the founder's
    /// first host for a joiner, the site's own for a founder), enrols the node,
    /// proves the kernel ACL plane, and records the mesh address, the core
    /// pair's for a core node and only the mesh inventory line for another class.
    #[test]
    fn only_phase_1_is_manual_and_the_key_is_minted_where_it_is_used() {
        let t = joiner_checkout();
        let ph = plan(t.path(), &opts("zagros-b", "core", "zagros")).unwrap();
        let manual: Vec<u32> = ph.iter().filter(|p| p.kind == "MANUAL").map(|p| p.n).collect();
        assert_eq!(manual, vec![1]);
        assert!(ph.iter().all(|p| !p.cmd.contains("NB_SETUP_KEY")), "no phase reads a setup key");
        let p9 = ph.iter().find(|p| p.n == 9).unwrap();
        assert!(p9.cmd.ends_with("'bash -s -- --key-at-enrolment'") && !p9.cmd.contains("SETUP_KEY"), "{}", p9.cmd);
        let p10: Vec<&Phase> = ph.iter().filter(|p| p.n == 10).collect();
        assert_eq!(p10.iter().map(|p| p.kind).collect::<Vec<_>>(), vec!["A", "GATE", "T"]);
        assert!(p10[0].cmd.contains("mesh-enroll-host.yml -e target=zagros_b_hosts -l node-x -e mesh_enroll_mint_group=zagros_hosts -e mesh_enroll_groups_from_peer=true -e mesh_ips_out="), "{}", p10[0].cmd);
        assert!(p10[1].cmd.contains("Interface type: Kernel") && p10[1].cmd.contains("table ip netbird"), "{}", p10[1].cmd);
        assert!(p10[2].cmd.contains("standup-mesh-record.py' zagros-b --ips ") && p10[2].cmd.ends_with(" --no-mint") && !p10[2].cmd.contains("--group"), "{}", p10[2].cmd);
        assert_eq!(p10[2].writes[0], "ops/ansible/group_vars/zagros_b_hosts/vars.yml");
        let p2 = ph.iter().find(|p| p.n == 2).unwrap();
        assert!(p2.cmd.contains("/ops/scripts/with-secrets' '") && p2.cmd.contains("seed-ssh-key.sh' "), "the password resolves through the injector: {}", p2.cmd);
        // An obs node records only its mesh inventory line.
        let ph = plan(t.path(), &opts("zagros-b", "obs", "zagros")).unwrap();
        let rec = ph.iter().find(|p| p.n == 10 && p.kind == "T").unwrap();
        assert!(rec.cmd.contains(" --group zagros_b_obs_hosts --no-mint"), "{}", rec.cmd);
        assert_eq!(rec.writes, vec!["ops/ansible/inventory/mgmt-hosts-mesh.ini".to_string()]);
        // A founder site's node mints on its own site's first host.
        let gvf = t.path().join("ops/ansible/group_vars/zagros_hosts");
        std::fs::create_dir_all(&gvf).unwrap();
        std::fs::write(gvf.join("vars.yml"), "goatnet_founder: true\n").unwrap();
        let ph = plan(t.path(), &opts("zagros", "core", "zagros")).unwrap();
        assert!(ph.iter().any(|p| p.cmd.contains("-e mesh_enroll_mint_group=zagros_hosts ")));
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
        for s in ["ops/scripts/with-aws", "ops/scripts/ansible-preflight.sh", "ops/scripts/seed-ssh-key.sh", "ops/scripts/door-open.sh", "ops/scripts/phone-node-provision.sh", "ops/scripts/phone-node-ssh.sh"] {
            exe(&repo.join(s));
        }
        std::fs::write(repo.join("ops/preflight-enrollment.sh"), "true\n").unwrap();
        // The site's leaf-cert manifest, which the certificate steps read.
        std::fs::create_dir_all(repo.join("ops/enrollment/manifests")).unwrap();
        std::fs::write(repo.join("ops/enrollment/manifests/zagros-b-certs.yaml"), "site_id: zagros-b\nleaves:\n  - name: router\n    cn: goat-zenoh-router\n  - name: fabric-health\n    cn: fabric-health-trial-peer\n").unwrap();
        exe(&repo.join("ops/scripts/mint-site-certs.sh"));
        std::fs::create_dir_all(repo.join("ops/ansible/inventory")).unwrap();
        let path = format!("{}:/usr/bin:/bin", bin.display());
        let mut ran = 0;
        for (class, vias) in [("core", &["mesh", "door"][..]), ("obs", &["mesh", "door"]), ("phone", &["usb"])] {
            for via in vias.iter().copied() {
                let o = Options { env: "zagros", site: "zagros-b", node: "node-x", class, address: (via != "usb").then_some("192.0.2.30"), pass_var: None, via, from: 1, to: 15, dry_run: true };
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

    /// Gate rows H3 to H5, the plan of a phone node over USB. Phase 1 is
    /// attaching the phone and is the one manual step; phase 2 prepares the VM
    /// and then gates on its integrity checks, and reads no password; the
    /// config preflight judges the site with the class's declarations at
    /// phases 3, 7 and 12; the node is in the site's core group and converges
    /// as a core joiner; until phase 12 ssh names the configuration the run
    /// wrote and the plays its inventory line, after the goatnet's own; from
    /// phase 12 the plays use the mesh inventories alone.
    #[test]
    fn a_phone_node_is_prepared_checked_and_reached_over_usb_until_enrolled() {
        let t = joiner_checkout();
        let o = Options { via: "usb", ..opts("zagros-b", "phone", "zagros") };
        let ph = plan(t.path(), &o).unwrap();
        let base = usb_base(t.path(), "zagros-b", "node-x");
        assert_eq!(ph.iter().filter(|p| p.kind == "MANUAL").map(|p| p.n).collect::<Vec<_>>(), vec![1]);
        assert!(ph[0].label.contains("attach the phone by USB") && ph[0].cmd.contains("pass no --address"), "{}", ph[0].cmd);
        let p2: Vec<&Phase> = ph.iter().filter(|p| p.n == 2).collect();
        assert_eq!(p2.iter().map(|p| p.kind).collect::<Vec<_>>(), vec!["T", "GATE"]);
        let script = format!("/ops/scripts/phone-node-provision.sh' --node node-x --user <deploy.ssh_user> --base '{base}' ");
        assert!(p2[0].cmd.contains(&script) && p2[0].cmd.ends_with("--prepare"), "{}", p2[0].cmd);
        assert!(p2[1].cmd.contains(&script) && p2[1].cmd.ends_with("--check"), "{}", p2[1].cmd);
        assert!(ph.iter().all(|p| !p.cmd.contains("seed-ssh-key.sh") && !p.cmd.contains("_NODE_PASS")), "a phone node reads no first-login password");
        for n in [3, 7, 12] {
            let gate = &ph.iter().find(|p| p.n == n).unwrap().cmd;
            assert!(gate.contains("standup-config-preflight.py' zagros_b_hosts --class phone"), "phase {n}: {gate}");
        }
        // A core joiner's converge, in the site's core group.
        assert_eq!(ph.iter().filter(|p| p.label.contains("[core, joiner]")).count(), 6);
        assert_eq!(group_of("zagros-b", "phone"), "zagros_b_hosts");
        let ssh = format!("ssh -F '{base}.ssh' <deploy.ssh_user>@node-x ");
        let usb_inv = format!("-i inventory/mgmt-hosts.ini -i inventory/mgmt-hosts-mesh.ini -i '{base}.ini' ");
        for p in ph.iter().filter(|p| (4..=11).contains(&p.n) && p.kind != "T" || p.n == 4) {
            if p.cmd.contains("ansible-preflight.sh") {
                assert!(p.cmd.contains(&usb_inv), "phase {}: {}", p.n, p.cmd);
            } else if p.cmd.contains("ssh ") {
                assert!(p.cmd.contains(&ssh), "phase {}: {}", p.n, p.cmd);
            }
        }
        assert!(ph.iter().filter(|p| p.cmd.contains(&ssh)).count() >= 3, "phases 4, 9 and 10 reach the node over USB");
        for p in ph.iter().filter(|p| p.n >= 12) {
            assert!(!p.cmd.contains("-usb."), "phase {} is after enrolment: {}", p.n, p.cmd);
        }
        let p14 = ph.iter().find(|p| p.n == 14).unwrap();
        assert!(p14.cmd.contains("inventory-hosts.py zagros_b_hosts --inventory inventory/mgmt-hosts-mesh.ini --ssh-target --user <deploy.ssh_user> --host node-x") && p14.cmd.contains("ssh \"$t\" 'sudo systemctl unmask"), "{}", p14.cmd);
        // Every other class and transport plans as before: no phone script, no USB file.
        for class in ["core", "obs"] {
            for via in ["mesh", "door"] {
                let ph = plan(t.path(), &Options { via, ..opts("zagros-b", class, "zagros") }).unwrap();
                assert!(ph.iter().all(|p| !p.cmd.contains("phone-node-") && !p.cmd.contains("-usb.") && !p.cmd.contains("--class phone")), "{class} {via}");
            }
        }
    }

    /// A phone node and the USB transport are taken together, a node on a USB
    /// cable takes no --address, and a phone node joins: a site that is not a
    /// joiner is refused (GB-301). Each is refused before any phase is planned.
    #[test]
    fn a_phone_node_is_refused_without_usb_with_an_address_or_at_a_founder() {
        let t = joiner_checkout();
        let refusal = |o: &Options| execute(t.path(), o).err().map(|e| e.to_string()).unwrap_or_default();
        assert!(refusal(&opts("zagros-b", "phone", "zagros")).contains("--class phone with --via mesh"));
        assert!(refusal(&Options { via: "usb", ..opts("zagros-b", "core", "zagros") }).contains("--class core with --via usb"));
        assert!(refusal(&Options { via: "usb", address: Some("192.0.2.9"), ..opts("zagros-b", "phone", "zagros") }).contains("a phone node has no LAN address"));
        let gvf = t.path().join("ops/ansible/group_vars/zagros_hosts");
        std::fs::create_dir_all(&gvf).unwrap();
        std::fs::write(gvf.join("vars.yml"), "goatnet_founder: true\n").unwrap();
        let founder = refusal(&Options { via: "usb", ..opts("zagros", "phone", "zagros") });
        assert!(founder.contains("GB-301") || founder.contains("a phone node joins a goatnet"), "{founder}");
    }

    /// One line per service the site does not run yet: its name, a dash, and the
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
        assert!(rows[0].detail.starts_with("What zagros-b does not run yet:") && rows[0].detail.contains("ops/goatbuilder/endstate/endstate.py model zagros-b"), "{}", rows[0].detail);
        assert_eq!(rows[1].name, "not-run");
        assert_eq!(rows[1].detail, "not read: ops/goatbuilder/endstate/endstate.py is not in this checkout; this list of what the site does not run yet is left out");
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
    /// obs node, a breakglass and a network-jump node, a run through the env
    /// door, and a phone node over USB. Every phase the plan holds has an entry and every entry is a phase;
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
            (
                off.path(),
                Options {
                    via: "usb",
                    ..opts("zagros-b", "phone", "zagros")
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
        // The site's leaf-cert manifest: the router and one service leaf.
        let m = t.path().join("ops/enrollment/manifests");
        std::fs::create_dir_all(&m).unwrap();
        std::fs::write(m.join("zagros-b-certs.yaml"), "site_id: zagros-b\nout_dir: /s/zagros-b-leaves\nleaves:\n  - name: router\n    cn: goat-zenoh-router\n  - name: fabric-health\n    cn: fabric-health-trial-peer\n  - name: portal\n    cn: portal\n").unwrap();
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

    /// A stand-in for the end-state model: prints `services` (JSON) as the
    /// model's services, whatever site it is asked about.
    fn fake_model(repo: &Path, services: &str) {
        let d = repo.join("ops/goatbuilder/endstate");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("endstate.py"), format!("import json\nprint(json.dumps({{\"services\": json.loads(r'''{services}''')}}))\n")).unwrap();
    }

    /// Gate row I3, the health publishers: after the data router, a joiner's
    /// core node runs each health publisher the end-state model marks present
    /// at the site, in a fixed order, for the site's core group limited to the
    /// node; one marked absent or undecided does not run. A publisher marked
    /// present whose play is not in the checkout is refused at plan time
    /// (GB-288). With no model to read, the plan says so on a SKIP line.
    #[test]
    fn a_joiners_core_node_runs_the_publishers_the_model_marks_present() {
        let t = joiner_checkout();
        // No model: a SKIP line says the publishers are not deployed, and why.
        let ph = plan(t.path(), &opts("zagros-b", "core", "zagros")).unwrap();
        let skip = ph.iter().find(|p| p.label.starts_with("[health publishers]")).expect("a line for the publishers");
        assert_eq!(skip.kind, "SKIP");
        assert!(skip.label.contains("not read"), "{}", skip.label);
        // The model marks fabric-crawler and fabric-health present, mesh-health absent, time-health undecided.
        fake_model(t.path(), r#"{"fabric-crawler": {"present": true}, "fabric-health": {"present": true}, "mesh-health": {"present": false}, "time-health": {"present": null}}"#);
        for play in ["fabric-health-deploy.yml", "fabric-crawler-deploy.yml"] {
            std::fs::write(t.path().join("ops/ansible").join(play), "---\n").unwrap();
        }
        let ph = plan(t.path(), &opts("zagros-b", "core", "zagros")).unwrap();
        let pubs: Vec<&Phase> = ph.iter().filter(|p| p.label.starts_with("[health publishers]")).collect();
        assert_eq!(pubs.len(), 2, "{:?}", pubs.iter().map(|p| &p.label).collect::<Vec<_>>());
        assert!(pubs[0].cmd.ends_with("fabric-health-deploy.yml -e target=zagros_b_hosts -l node-x"), "{}", pubs[0].cmd);
        assert!(pubs[1].cmd.ends_with("fabric-crawler-deploy.yml -e target=zagros_b_hosts -l node-x"), "{}", pubs[1].cmd);
        assert!(pubs.iter().all(|p| p.n == 13 && p.kind == "A"));
        let router = ph.iter().position(|p| p.cmd.contains("zenoh-deploy.yml")).unwrap();
        assert!(ph.iter().position(|p| p.cmd.contains("fabric-health-deploy.yml")).unwrap() > router);
        // Present, and no play in the checkout: refused when the plan is made.
        std::fs::remove_file(t.path().join("ops/ansible/fabric-crawler-deploy.yml")).unwrap();
        let e = plan(t.path(), &opts("zagros-b", "core", "zagros")).err().expect("refused");
        assert_eq!(e.downcast_ref::<goat_errors::Coded>().map(|c| c.code.as_str()), Some("GB-288"), "{e}");
        assert!(e.to_string().contains("fabric-crawler"), "{e}");
    }

    /// STAGED_SERVICES is stage-service-leaves.yml's service_leaf_map.
    #[test]
    fn the_staged_services_are_the_staging_plays() {
        let text = std::fs::read_to_string(repo().join("ops/ansible/stage-service-leaves.yml")).unwrap();
        let plays: Vec<serde_yaml::Value> = serde_yaml::from_str(&text).unwrap();
        let map = plays[0]["vars"]["service_leaf_map"].as_mapping().unwrap();
        let mut keys: Vec<&str> = map.keys().filter_map(|k| k.as_str()).collect();
        let mut ours = STAGED_SERVICES.to_vec();
        keys.sort_unstable();
        ours.sort_unstable();
        assert_eq!(keys, ours);
    }

    /// Each publisher join-node deploys is an end-state catalog service whose
    /// leaf the staging play stages, and its play is in this repository.
    #[test]
    fn each_publisher_is_a_catalog_service_with_its_play() {
        let text = std::fs::read_to_string(repo().join("ops/goatbuilder/endstate/catalog.yaml")).unwrap();
        let catalog: serde_yaml::Value = serde_yaml::from_str(&text).unwrap();
        for (name, play) in PUBLISHERS {
            assert!(catalog["services"].get(name).is_some(), "{name} is not an end-state catalog service");
            assert!(STAGED_SERVICES.contains(&name), "{name}'s leaf is not staged");
            assert!(repo().join("ops/ansible").join(play).is_file(), "no ops/ansible/{play}");
        }
    }

    /// Gate row I3, the certificates: before the data router, a joiner's core
    /// node mints the site's leaves from its manifest and stages the service
    /// leaves the manifest carries onto the node. A site with no manifest is
    /// refused when the plan is made (GB-286), and so is a manifest with no
    /// router leaf, which the router's play reads (GB-287).
    #[test]
    fn a_joiners_core_node_mints_and_stages_its_leaves_before_the_router() {
        let t = joiner_checkout();
        let ph = plan(t.path(), &opts("zagros-b", "core", "zagros")).unwrap();
        let at = |needle: &str| ph.iter().position(|p| p.cmd.contains(needle)).unwrap_or_else(|| panic!("no phase runs {needle}"));
        let (mint, stage, router) = (at("mint-site-certs.sh"), at("stage-service-leaves.yml"), at("zenoh-deploy.yml"));
        assert!(at("signal-federation-verify.yml") < mint && mint < stage && stage + 1 == router, "{mint} {stage} {router}");
        assert_eq!((ph[mint].n, ph[mint].kind, ph[stage].n, ph[stage].kind), (13, "T", 13, "A"));
        assert!(ph[mint].cmd.ends_with("/ops/scripts/mint-site-certs.sh' zagros-b"), "{}", ph[mint].cmd);
        assert!(ph[stage].cmd.contains("stage-service-leaves.yml -e target=zagros_b_hosts -l node-x -e leaf_src_dir=/s/zagros-b-leaves "), "{}", ph[stage].cmd);
        assert!(ph[stage].cmd.ends_with(r#"-e '{"leaf_only": ["fabric-health"]}'"#), "{}", ph[stage].cmd);
        // No router leaf in the manifest: refused at plan time.
        let m = t.path().join("ops/enrollment/manifests/zagros-b-certs.yaml");
        std::fs::write(&m, "site_id: zagros-b\nout_dir: /s/zagros-b-leaves\nleaves:\n  - name: fabric-health\n    cn: fabric-health-trial-peer\n").unwrap();
        let e = plan(t.path(), &opts("zagros-b", "core", "zagros")).err().expect("refused");
        assert_eq!(e.downcast_ref::<goat_errors::Coded>().map(|c| c.code.as_str()), Some("GB-287"), "{e}");
        // No manifest at all: refused at plan time.
        std::fs::remove_file(&m).unwrap();
        let e = plan(t.path(), &opts("zagros-b", "core", "zagros")).err().expect("refused");
        assert_eq!(e.downcast_ref::<goat_errors::Coded>().map(|c| c.code.as_str()), Some("GB-286"), "{e}");
        assert!(e.to_string().contains("ops/enrollment/manifests/zagros-b-certs.yaml"), "{e}");
    }

    /// A site that declares its own audit-subscriber mints its leaf from the
    /// manifest like any other service leaf, so the staging step stages it
    /// too, into the directory the audit-subscriber role reads.
    #[test]
    fn a_manifests_audit_subscriber_leaf_is_staged() {
        let t = joiner_checkout();
        let m = t.path().join("ops/enrollment/manifests/zagros-b-certs.yaml");
        std::fs::write(&m, "site_id: zagros-b\nout_dir: /s/zagros-b-leaves\nleaves:\n  - name: router\n    cn: goat-zenoh-router\n  - name: fabric-health\n    cn: fabric-health-trial-peer\n  - name: audit-subscriber\n    cn: audit-subscriber\n").unwrap();
        let ph = plan(t.path(), &opts("zagros-b", "core", "zagros")).unwrap();
        let stage = ph.iter().find(|p| p.cmd.contains("stage-service-leaves.yml")).expect("a staging step");
        assert!(stage.cmd.ends_with(r#"-e '{"leaf_only": ["fabric-health", "audit-subscriber"]}'"#), "{}", stage.cmd);
        assert!(stage.label.contains("audit-subscriber"), "{}", stage.label);
    }

    /// Gate row I3, the data router: a joiner's core node runs zenoh-deploy.yml
    /// for its site's core group, limited to the node, in phase 13 after the
    /// Spock wiring and signal federation (on, or declared off). A founder's
    /// core node and the other classes run no router step of join-node's.
    #[test]
    fn a_joiners_core_node_deploys_its_data_router_after_signal_federation() {
        let t = joiner_checkout();
        let ph = plan(t.path(), &opts("zagros-b", "core", "zagros")).unwrap();
        let router: Vec<&Phase> = ph.iter().filter(|p| p.cmd.contains("zenoh-deploy.yml")).collect();
        assert_eq!(router.len(), 1, "{:?}", router.iter().map(|p| &p.label).collect::<Vec<_>>());
        assert_eq!((router[0].n, router[0].kind), (13, "A"));
        assert!(router[0].cmd.ends_with("zenoh-deploy.yml -e target=zagros_b_hosts -l node-x"), "{}", router[0].cmd);
        let at = |needle: &str| ph.iter().position(|p| p.cmd.contains(needle)).unwrap();
        assert!(at("signal-federation-verify.yml") < at("zenoh-deploy.yml"), "after the federation proof");
        assert!(ph[at("zenoh-deploy.yml") + 1..].iter().all(|p| p.n > 13 || !p.cmd.contains("signal-federation")));
        // Federation declared off: the router still runs, after the SKIP.
        let gv = t.path().join("ops/ansible/group_vars/zagros_b_hosts/vars.yml");
        let mut text = std::fs::read_to_string(&gv).unwrap();
        text.push_str("signal_federation_absent_reason: one node holds every peer\n");
        std::fs::write(&gv, text).unwrap();
        let ph = plan(t.path(), &opts("zagros-b", "core", "zagros")).unwrap();
        let skip = ph.iter().position(|p| p.kind == "SKIP" && p.label.starts_with("[signal federation]")).unwrap();
        let router = ph.iter().position(|p| p.cmd.contains("zenoh-deploy.yml")).expect("the router runs with federation off");
        assert!(skip < router);
        // Other classes, and a founder's core node, run no router step of join-node's.
        for class in ["obs", "breakglass", "network-jump"] {
            assert!(plan(t.path(), &opts("zagros-b", class, "zagros")).unwrap().iter().all(|p| !p.cmd.contains("zenoh-deploy.yml")), "{class}");
        }
        let gvf = t.path().join("ops/ansible/group_vars/zagros_hosts");
        std::fs::create_dir_all(&gvf).unwrap();
        std::fs::write(gvf.join("vars.yml"), "goatnet_founder: true\n").unwrap();
        assert!(plan(t.path(), &opts("zagros", "core", "zagros")).unwrap().iter().all(|p| !p.cmd.contains("zenoh-deploy.yml")));
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

    /// Gate row I3, the front door: a joiner's core node that declares signal
    /// federation off still gets its front door, after the data router, for
    /// the site's core group limited to the node. Its profile is the site's
    /// declaration: `dashboard_cert_delivery_mode: acme` is an internet site,
    /// whose door is traefik-acme.yml; anything else is a LAN site, whose door
    /// is lan-traefik-routes.yml. With federation on, the federation steps set
    /// up the door (traefik-acme.yml), and no second door step runs.
    #[test]
    fn a_joiner_gets_its_front_door_with_signal_federation_off() {
        let t = joiner_checkout();
        let gv = t.path().join("ops/ansible/group_vars/zagros_b_hosts/vars.yml");
        let base = std::fs::read_to_string(&gv).unwrap();
        fn door(ph: &[Phase]) -> Vec<&Phase> {
            ph.iter().filter(|p| p.label.starts_with("[front door]")).collect()
        }
        // Federation on: the federation steps own the door.
        let ph = plan(t.path(), &opts("zagros-b", "core", "zagros")).unwrap();
        assert!(door(&ph).is_empty(), "{:?}", door(&ph).iter().map(|p| &p.label).collect::<Vec<_>>());
        assert_eq!(ph.iter().filter(|p| p.cmd.contains("traefik-acme.yml")).count(), 1);
        // Federation off, a LAN site: the LAN routes, after the router.
        std::fs::write(&gv, format!("{base}signal_federation_absent_reason: one node holds every peer\n")).unwrap();
        let ph = plan(t.path(), &opts("zagros-b", "core", "zagros")).unwrap();
        let d = door(&ph);
        assert_eq!(d.len(), 1, "{:?}", d.iter().map(|p| &p.label).collect::<Vec<_>>());
        assert_eq!((d[0].n, d[0].kind), (13, "A"));
        assert!(d[0].cmd.ends_with("lan-traefik-routes.yml -e target=zagros_b_hosts -e site_id=zagros-b -l node-x"), "{}", d[0].cmd);
        let at = |needle: &str| ph.iter().position(|p| p.cmd.contains(needle)).unwrap();
        assert!(at("zenoh-deploy.yml") < at("lan-traefik-routes.yml"), "after the router");
        // Federation off, an internet site: traefik-acme.yml, limited to the node.
        std::fs::write(&gv, format!("{base}signal_federation_absent_reason: one node holds every peer\ndashboard_cert_delivery_mode: acme\n")).unwrap();
        let ph = plan(t.path(), &opts("zagros-b", "core", "zagros")).unwrap();
        let d = door(&ph);
        assert_eq!(d.len(), 1);
        assert!(d[0].cmd.ends_with("traefik-acme.yml -e target=zagros_b_hosts -l node-x"), "{}", d[0].cmd);
        assert!(ph.iter().all(|p| !p.cmd.contains("lan-traefik-routes.yml")));
        // Other classes, and a founder's core node, get no door step here.
        for class in ["obs", "breakglass", "network-jump"] {
            assert!(door(&plan(t.path(), &opts("zagros-b", class, "zagros")).unwrap()).is_empty(), "{class}");
        }
        let gvf = t.path().join("ops/ansible/group_vars/zagros_hosts");
        std::fs::create_dir_all(&gvf).unwrap();
        std::fs::write(gvf.join("vars.yml"), "goatnet_founder: true\n").unwrap();
        assert!(door(&plan(t.path(), &opts("zagros", "core", "zagros")).unwrap()).is_empty());
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
