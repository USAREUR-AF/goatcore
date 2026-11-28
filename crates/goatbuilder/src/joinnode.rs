//! `join-node`: one more machine of a class at a live site, declared →
//! converged → verified. This verb is the node-addition sequence (the
//! executed order, the joiner's three passes, "done means proven") as
//! numbered phases with their gates, with each step's trap in the phase text;
//! docs/tutorials/02-first-federated-node.md walks through it. `--dry-run` prints every phase and
//! command and runs nothing; `--from`/`--to` resume; every phase is idempotent.
//!
//!   1  GATE   the node answers (over SSH at --address, or a phone over USB); stops only when it does not (GB-314)
//!   2  T      seed-ssh-key.sh — key auth + NOPASSWD sudo, password from an env var;
//!             skipped when `ssh -o BatchMode=yes <user>@<addr> sudo -n true` already
//!             passes (a box installed unattended with the key and NOPASSWD sudo)
//!   3  GATE   repo work done first: standup-config-preflight.py passes; then the
//!             node's clock, read-only (join-node-clock-check.py, GB-297)
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
//!  14  T      unmask and start the apt timers, over the mesh: at the node's mesh address,
//!             the one phase 10 recorded (through an env door: at --address)
//!  15  GATE   acceptance: every applicable row PASS, from the new node's context
//!
//! A phone node (`--class phone --via usb`) differs in two phases: 1 is
//! attaching the phone by USB; 2 is T + GATE, phone-node-provision.sh preparing
//! the phone's Linux VM and then its integrity checks.
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

use anyhow::Result;

use crate::config;
use crate::standup::{sh_quote, sh_word};
use crate::targets::{self, TargetType, Targets, Values};
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
    /// Who accepts the risk of a listed service whose rollout is not cleanly
    /// reversible (ADR 1211): its deploy takes it, and its ledger row records it.
    pub ack: &'a str,
}

pub const VIAS: [&str; 3] = ["mesh", "door", "usb"];

/// The inventories a play is given for each transport. Base inventory first
/// (group membership, site facts, router_role): the goatnet's, then the one
/// the node's type declares (`typed`, ` -i <file>` or empty), which names the
/// type's group and its machines; then exactly one transport override, last,
/// so the node is reached by it. Passing both overrides lets whichever loads
/// last win silently. `usb` reaches the goatnet's hosts over the mesh; the
/// node's own line, over the Android debug bridge, is added for the phases
/// before it has a mesh address (`usb_base`).
fn inventories(via: &str, typed: &str) -> String {
    let over = match via {
        "door" => "inventory/doors-groupvars.ini",
        _ => "inventory/mgmt-hosts-mesh.ini",
    };
    format!("-i inventory/mgmt-hosts.ini{typed} -i {over}")
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

/// GB-369 for the first phase in `from..=to` whose command reads, as `$NAME`
/// or `${NAME}`, a variable `get` does not hold: bash runs each phase with
/// `-u`, so that phase would stop on `NAME: unbound variable` before its
/// program starts, partway through the node. A manual phase runs nothing.
fn unset_variable_refusal(phases: &[Phase], from: u32, to: u32, get: &dyn Fn(&str) -> Option<String>) -> Option<anyhow::Error> {
    let (ph, names) = phases.iter().filter(|p| p.n >= from && p.n <= to && p.kind != "MANUAL").find_map(|p| {
        let names = crate::envrefs::unset(&p.cmd, get);
        (!names.is_empty()).then_some((p, names))
    })?;
    let (verb, them) = if names.len() == 1 { ("is", "it") } else { ("are", "each") };
    Some(
        goat_errors::coded(
            "GB-369",
            format!(
                "phase {} (\"{}\") reads {}, which {verb} not set in this environment: bash would stop on {them} before the phase's command starts, so no phase has run. export {} (or keep {them} under your secrets posture and run goatbuilder through ops/scripts/with-secrets), then re-run",
                ph.n,
                ph.label,
                names.join(", "),
                names.iter().map(|n| format!("{n}=<value>")).collect::<Vec<_>>().join(" "),
            ),
        )
        .into(),
    )
}

fn row(name: &str, detail: String) -> Check {
    Check { name: name.into(), ok: true, detail, fix: None }
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

/// Whether the group's vars.yml declares a joiner, as site-bootstrap reads it:
/// `goatnet_founder | default(true) | bool`, from the top-level key only.
/// Ansible's `bool` is true only for its true words (y, yes, on, 1, true, t,
/// any case) and the number 1; anything else set is false. A file that does
/// not parse, or no top-level key, is a founder, as the default is.
fn declares_joiner(file: &Path) -> bool {
    let Some(doc) = std::fs::read_to_string(file).ok().and_then(|t| serde_yaml::from_str::<serde_yaml::Value>(&t).ok()) else {
        return false;
    };
    let Some(v) = doc.get("goatnet_founder") else {
        return false;
    };
    !ansible_bool(v)
}

/// Ansible's `bool` filter on a YAML value.
fn ansible_bool(v: &serde_yaml::Value) -> bool {
    match v {
        serde_yaml::Value::Bool(b) => *b,
        serde_yaml::Value::Number(n) => n.as_f64() == Some(1.0),
        serde_yaml::Value::String(s) => matches!(s.trim().to_ascii_lowercase().as_str(), "y" | "yes" | "on" | "1" | "true" | "t"),
        _ => false,
    }
}

fn gv_first(file: &Path, key: &str) -> String {
    std::fs::read_to_string(file)
        .ok()
        .and_then(|t| t.lines().find_map(|l| l.trim_start().strip_prefix(key).and_then(|r| r.strip_prefix(':')).map(|v| v.split_whitespace().next().unwrap_or("").replace(['"', '\''], ""))))
        .unwrap_or_default()
}

/// The services stage-service-leaves.yml stages a leaf for (its
/// `service_leaf_map`). A test holds this list in step with the play.
const STAGED_SERVICES: [&str; 11] = ["fabric-health", "mesh-health", "presence-health", "time-health", "peer-health", "consistency-health", "fabric-crawler", "goat-topology", "audit-subscriber", "node-net-health", "ntp-health-pub"];

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
    let (rows, on) = signal_federation_rows(repo, &repo.display().to_string(), gv, site, founder, pre, inv);
    ev.extend(rows.into_iter().map(|(kind, label, cmd)| Phase { n: 13, kind, label, cmd, writes: Vec::new() }));
    on
}

/// The signal-federation rows themselves, as (kind, label, command), for
/// whichever verb runs them: join-node's phase 13 and a joiner's standup
/// (phase 9). `repo` is where plays and group_vars are read; `r` is the
/// checkout path the commands carry. Returns whether the federation steps run.
pub(crate) fn signal_federation_rows(repo: &Path, r: &str, gv: &Path, site: &str, founder: &str, pre: &str, inv: &str) -> (Vec<(&'static str, String, String)>, bool) {
    let mut ev: Vec<(&'static str, String, String)> = Vec::new();
    let site_group = format!("{}_hosts", site.replace('-', "_"));
    let founder_group = format!("{}_hosts", founder.replace('-', "_"));
    let p = |kind: &'static str, label: String, cmd: String| (kind, label, cmd);
    // The founder site is read from vars.yml: one shell word wherever it is used.
    let w = |v: String| sh_word(&v);
    ev.push(p("GATE", format!("[signal federation] the repo declares it on both sides: {site_group} and {founder_group} crossed and complete, {founder}'s relay inventory pins each federation hostname to its own node, the names resolve to the relays, the shared token is in the environment"), format!("python3 '{r}/ops/scripts/signal-federation-check.py' {site} --peer {} --resolve --require-env", w(founder.to_string()))));
    if !gv_first(gv, "signal_federation_absent_reason").is_empty() {
        ev.push(p("SKIP", format!("[signal federation] declared off at {site} (signal_federation_absent_reason in group_vars/{site_group}); the gate above proved no side expects it: {founder} is off too, or federates with another site"), format!("echo '>> signal federation: declared off for {site} (signal_federation_absent_reason); nothing to converge'")));
        return (ev, false);
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
    (ev, true)
}

/// The mesh DNS zone file a site serves: the `ops/dns/<dir>/mesh-hosts` whose
/// records carry `zone`, as ops/dns/generate-mesh-hosts.py finds it. Returns
/// the directory name and the first name the file answers.
fn zone_file(repo: &Path, zone: &str) -> Option<(String, String)> {
    let mut dirs: Vec<_> = std::fs::read_dir(repo.join("ops/dns")).ok()?.flatten().map(|e| e.path()).collect();
    dirs.sort();
    dirs.into_iter().find_map(|d| {
        let text = std::fs::read_to_string(d.join("mesh-hosts")).ok()?;
        let name = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .filter_map(|l| l.split_whitespace().last())
            .find(|n| n.ends_with(zone))?
            .to_string();
        Some((d.file_name()?.to_string_lossy().into_owned(), name))
    })
}

/// What a joining site's mesh DNS step runs with, derived from the
/// checkout: the zone, its file's directory and a name it answers (the play's
/// smoke test), the site's nameserver group, the founder's management API and
/// the variable holding its token, and the founder's core group, whose first
/// host is the second replica. `goatbuilder inputs join-node` lists the same.
#[derive(Debug, Clone)]
pub(crate) struct MeshDns {
    pub zone: String,
    pub dir: String,
    pub first_name: String,
    pub group_name: String,
    pub api: String,
    pub token: String,
    pub peer_group: String,
}

pub(crate) fn mesh_dns_values(repo: &Path, gv: &Path, site: &str, founder: &str) -> Result<MeshDns> {
    let founder_us = founder.replace('-', "_");
    let founder_gv = repo.join("ops/ansible/group_vars").join(format!("{founder_us}_hosts")).join("vars.yml");
    let founder_zone = gv_first(&founder_gv, "site_dns_zone");
    let zone = Some(gv_first(gv, "site_dns_zone")).filter(|z| !z.is_empty()).unwrap_or_else(|| founder_zone.clone());
    if zone.is_empty() {
        return Err(goat_errors::coded("GB-280", format!("{site} has no mesh DNS zone to serve: neither group_vars/{}_hosts nor group_vars/{founder_us}_hosts declares site_dns_zone. Declare it, then generate the zone: python3 ops/dns/generate-mesh-hosts.py", site.replace('-', "_"))).into());
    }
    let Some((dir, first_name)) = zone_file(repo, &zone) else {
        return Err(goat_errors::coded("GB-280", format!("{site} has no mesh DNS zone to serve: no ops/dns/*/mesh-hosts carries {zone}. Generate it from the goatnet's registry (python3 ops/dns/generate-mesh-hosts.py, or --init <site_id> for a zone that has none yet) and commit it; without it the site, cut off from {founder}, resolves none of the goatnet's names")).into());
    };
    // The founder's management: the name it declares for it (public_mgmt_fqdn),
    // which on a LAN goatlite is netbird.<zone>, not the zone; the zone where it
    // declares none (a site whose management is fronted at the zone's apex).
    let api_host = Some(gv_first(&founder_gv, "public_mgmt_fqdn")).filter(|v| !v.is_empty() && !v.contains("{{"));
    let api_zone = api_host.unwrap_or_else(|| if founder_zone.is_empty() { zone.clone() } else { founder_zone });
    Ok(MeshDns {
        zone,
        dir,
        first_name,
        group_name: format!("{site}-mesh-dns"),
        api: format!("https://{api_zone}/api"),
        token: format!("{}_NB_API_TOKEN", founder_us.to_uppercase()),
        peer_group: format!("{founder_us}_hosts"),
    })
}

/// What the founder's management API token is for, as the gate before the
/// mesh DNS step says it (join-node and standup alike).
pub(crate) const MESH_DNS_TOKEN_FOR: &str = "the mesh DNS step registers this site's nameserver group through the founder's management API with it";

/// The gate before the mesh DNS step: the founder's API token resolves
/// through the injector, else GB-281 names the variable (never its value).
pub(crate) fn mesh_dns_token_gate(r: &str, token: &str) -> String {
    format!("'{r}/ops/scripts/with-secrets' '{r}/ops/scripts/require-secret.sh' GB-281 {} {}", sh_word(token), sh_quote(MESH_DNS_TOKEN_FOR))
}

/// Phase 13's mesh DNS step for a joiner's core node (gate row I3, ADR 1235).
/// A site cut off from its founder still resolves the goatnet's names only if
/// it runs a resolver of its own. goat-coredns.yml puts a CoreDNS replica of
/// the goatnet's zone on the node, and registers the site's own nameserver
/// group, `<site>-mesh-dns`, which lists this replica first and the founder's
/// second. The group is written through the founder's management API, the
/// one writer, as standup writes the founder's. A gate before it refuses an
/// unset or unresolved founder API token (GB-281); a site with no zone file to
/// serve is refused when the plan is made (GB-280).
#[allow(clippy::too_many_arguments)]
fn mesh_dns(ev: &mut Vec<Phase>, repo: &Path, gv: &Path, site: &str, founder: &str, node: &str, pre: &str, inv: &str) -> Result<()> {
    let r = repo.display().to_string();
    let MeshDns { zone, dir, first_name, group_name, api, token, peer_group } = mesh_dns_values(repo, gv, site, founder)?;
    let p = |kind: &'static str, label: String, cmd: String| Phase { n: 13, kind, label, cmd, writes: Vec::new() };
    // The token is resolved through the injector, as the play reads it; its
    // value is never printed or put on a command line.
    ev.push(p("GATE", format!("[mesh DNS] the founder's management API token ({token}) resolves, before anything registers the site's nameserver group"), mesh_dns_token_gate(&r, &token)));
    ev.push(p("A", format!("[mesh DNS] a replica of the goatnet's zone ({zone}, ops/dns/{dir}/mesh-hosts) on the node, and the site's own nameserver group {group_name}, this replica first and {founder}'s second, written through {founder}'s management: a site cut off from {founder} still resolves the goatnet's names"), format!("cd '{r}/ops/ansible' && {pre} {inv} goat-coredns.yml -l {node} -e mesh_dns_zone_dir_name={} -e mesh_coredns_smoke_test_name={} -e netbird_nameserver_group_name={} -e netbird_api={} -e netbird_api_token_envvar={} -e mesh_dns_peer_replica_group={}", sh_word(&dir), sh_word(&first_name), sh_word(&group_name), sh_word(&api), sh_word(&token), sh_word(&peer_group))));
    Ok(())
}

/// A single-host joining site's observability stack (gate row I3). A site
/// with no obs host of its own (no `has_obs_host: true`) runs Prometheus,
/// Alertmanager and the obs exporters on its core node, from `obs.yml`, the
/// play for such a site. Grafana is left off (`grafana_enabled=false`): its
/// OAuth2 client is written to the site's Hydra, a write to the identity store
/// at a second site, which gate rows I1 and I4 hold back. The play takes the
/// site as one of its own only when `obs_colocated_site_zones` in the site's
/// group_vars maps it to its zone; without that, the step is skipped and says
/// what to declare.
#[allow(clippy::too_many_arguments)]
fn single_host_obs(ev: &mut Vec<Phase>, repo: &Path, gv: &Path, site: &str, site_group: &str, node: &str, upper: &str, pre: &str, inv: &str) {
    if gv_matches(gv, "has_obs_host", "true") {
        return;
    }
    let r = repo.display().to_string();
    let p = |kind: &'static str, label: String, cmd: String| Phase { n: 13, kind, label, cmd, writes: Vec::new() };
    if !gv_matches(gv, "obs_colocated_site_zones", "") {
        let why = format!("[obs, single-host] not deployed: {site} has no obs host of its own, and group_vars/{site_group}/vars.yml declares no obs_colocated_site_zones (its site_id mapped to its DNS zone), which obs.yml needs; declare it and resume with --from 13");
        ev.push(p("SKIP", why.clone(), format!("echo {}", sh_quote(&format!(">> {why}")))));
        return;
    }
    ev.push(p("A", format!("[obs, single-host] Prometheus, Alertmanager and the obs exporters on the core node (obs.yml), limited to the node: {site} has no obs host of its own. Grafana is left off: its OAuth2 client would be written to the site's Hydra, an identity-store write at a second site (gate rows I1, I4). Needs {upper}_POSTGRES_EXPORTER_PASS"), format!("cd '{r}/ops/ansible' && {pre} {inv} obs.yml -e target={site_group} -l {node} -e grafana_enabled=false -e 'postgres_superuser_password={{{{ lookup(\"env\", \"{upper}_PG_PASS\") }}}}'")));
}

/// What phase 13's built-in steps work with: the steps of a type file whose
/// logic is goatbuilder's own (targets::BUILTINS).
struct Converge<'a> {
    repo: &'a Path,
    r: &'a str,
    o: &'a Options<'a>,
    gv: &'a Path,
    site_group: &'a str,
    group: &'a str,
    upper: &'a str,
    founder_site: &'a str,
    founder_group: &'a str,
    pre: &'a str,
    inv: &'a str,
}

/// One of phase 13's steps.
fn p13(kind: &'static str, label: &str, cmd: String) -> Phase {
    Phase { n: 13, kind, label: label.to_string(), cmd, writes: Vec::new() }
}

impl Converge<'_> {
    /// The steps a type file names `name`, in their order.
    fn builtin(&self, name: &str, ev: &mut Vec<Phase>) -> Result<()> {
        let Converge { repo, gv, site_group, founder_site, upper, pre, inv, .. } = *self;
        let (site, node) = (self.o.site, self.o.node);
        match name {
            "seed-and-replicate" => self.seed_and_replicate(ev)?,
            "signal-federation" => {
                signal_federation(ev, repo, gv, site, founder_site, pre, inv);
            }
            "site-leaves" => self.site_leaves(ev)?,
            "front-door" => self.front_door(ev),
            "health-publishers" => self.health_publishers(ev)?,
            // The site's own mesh DNS replica (ADR 1235).
            "mesh-dns" => mesh_dns(ev, repo, gv, site, founder_site, node, pre, inv)?,
            // A single-host site's observability stack (gate row I3).
            "single-host-obs" => single_host_obs(ev, repo, gv, site, site_group, node, upper, pre, inv),
            "founder-bootstrap" => self.founder_bootstrap(ev),
            // An obs node of a founder site runs the utility play, which is
            // not in a public clone: refused when the plan is made.
            "utility-play-present" => {
                if let Some(missing) = crate::standup::obs_play_missing(repo) {
                    return Err(goat_errors::coded("GB-248", missing).into());
                }
            }
            // The loader refuses a name that is not in targets::BUILTINS.
            other => return Err(goat_errors::coded("GB-374", format!("the built-in step '{other}' is not one join-node runs")).into()),
        }
        Ok(())
    }

    fn ev_secrets(&self) -> String {
        let upper = self.upper;
        format!("-e 'postgres_superuser_password={{{{ lookup(\"env\", \"{upper}_PG_PASS\") }}}}' -e 'netbird_encryption_key={{{{ lookup(\"env\", \"{upper}_NB_ENCKEY\") }}}}' -e 'pgbackrest_cipher_pass={{{{ lookup(\"env\", \"{upper}_BKP_KEY\") }}}}'")
    }

    /// A joiner's core node seeded from its founder and replicating both ways.
    fn seed_and_replicate(&self, ev: &mut Vec<Phase>) -> Result<()> {
        let Converge { repo, r, gv, group, upper, founder_site, founder_group, pre, inv, .. } = *self;
        let (site, node) = (self.o.site, self.o.node);
        let ev_secrets = self.ev_secrets();
        let intra_pair = gv_matches(gv, "intra_site_pair", "true");
        if founder_site.is_empty() {
            return Err(goat_errors::coded("GB-250", format!("group_vars/{group}/vars.yml declares goatnet_founder: false and no join_founder_site — a joiner must name the founder it seeds from")).into());
        }
        // site-seed-from-peer.yml is the generic play; a site that keeps its
        // own <site>-seed-from-peer.yml gets that one instead.
        let seed_play = crate::standup::site_play(repo, site, "seed-from-peer.yml");
        // A site's own copy of the seed play is checked against the generic
        // play it was derived from before phase 13 changes anything (GB-362).
        if let Some((label, cmd)) = crate::standup::site_play_gate(repo, r, site, "seed-from-peer.yml") {
            ev.push(p13("GATE", &label, cmd));
        }
        // The wiring below runs its Spock roles only where the host's own
        // variables declare replication (spock-mesh-wire.yml's `when:`), on the
        // node and on the founder alike; elsewhere it is a no-op that reports
        // done. A goatlite founder declares none (goatlite-growth.md section 5).
        // The management tables replicate only where spock_mesh_databases pulls
        // netbird_mgmt (ADR 1084 D1), and the seed and its checks compare exactly
        // those, so without it the sites drift. Both sides are asked before
        // anything changes (GB-417).
        ev.push(p13("GATE", &format!("[core, joiner] replication is declared on both sides: the node and {founder_group}'s A host each declare participates_in_spock, a topology, ory_spock_enabled and the netbird_mgmt replication set the seed compares (read-only; GB-417 names each missing value)"), format!("cd '{r}/ops/ansible' && {pre} {inv} spock-join-replication-declared.yml -e join_check_node={} -e join_check_site={site} -e join_check_founder_group={}", sh_word(node), sh_word(founder_group))));
        // site_id reaches the joiner's passes as a PLAY variable, never as
        // `-e site_id`: an extra var sets every host's site_id to this site,
        // and the Spock roles' inventory walk then finds no peer.
        let bs = crate::standup::bootstrap_play(repo, site);
        let p1 = if intra_pair { format!("{ev_secrets} -e participates_in_spock=false") } else { format!("{ev_secrets} -e participates_in_spock=false -e has_durable_independent_state=true") };
        // Pass 1 runs site-bootstrap's `stack` plays only, as standup's
        // phase 5 does: the `after-ory` plays wait for the management
        // service, which waits for Hydra, which the next step deploys.
        // Pass 3 runs the whole file once Ory is up.
        ev.push(p13("A", "[core, joiner] pass 1 — bootstrap without Spock, the stack plays only (AutoMigrate creates the schema once, on the empty DB; Postgres initialises with the founder's encoding; the after-ory plays wait for Ory, deployed next)", format!("cd '{r}/ops/ansible' && {pre} {inv} -l {node} {bs} {p1} --skip-tags after-ory")));
        ev.push(p13("A", "[core, joiner] the service deploys the seed needs (kratos/hydra schemas): ory", format!("cd '{r}/ops/ansible' && {pre} {inv} ory-deploy.yml --limit {node} -e 'postgres_superuser_password={{{{ lookup(\"env\", \"{upper}_PG_PASS\") }}}}'")));
        ev.push(p13("A", &format!("[core, joiner] pass 2 — seed-from-peer: data-only copy from founder {founder_site} (fails loud if the account is populated)"), format!("cd '{r}/ops/ansible' && {pre} {inv} -l {node} {seed_play} -e site_id={site}")));
        // Row 9: the seed carries the founder's data and the subscriptions
        // only later writes (synchronize_data false), so a founder write
        // between the two is never sent. Before Spock is wired, every seed
        // table must still equal the founder's (GB-367).
        ev.push(p13("GATE", "[core, joiner] the seed is still current: every seed table equals the founder's, before Spock is wired (row counts and checksums, read-only; GB-367 names each table that differs: re-seed)", format!("cd '{r}/ops/ansible' && {pre} {inv} -l {node} spock-seed-current-check.yml -e site_id={site} -e seed_check_phase=before")));
        // Ory (Phase 7D) subscribes each side only to a peer that already has
        // its ory_narrow node, and its history reconcile checks the reverse
        // subscription on the peer, so the three steps below are staged:
        // joiner node+repset, founder node+repset+subscription, joiner
        // subscription+reconcile (which copies history both ways).
        ev.push(p13("A", "[core, joiner] pass 3 — bootstrap with Spock: netbird subscriptions on both sides, polled to replicating; Ory node + repset only", format!("cd '{r}/ops/ansible' && {pre} {inv} -l {node} {bs} {ev_secrets} -e ory_spock_wire_subscriptions=false")));
        ev.push(p13("A", &format!("[core, joiner] the founder-side mirror on {founder_group}, peer={site} only (the same change, not a follow-up): netbird + Ory node, repset, subscription"), format!("cd '{r}/ops/ansible' && {pre} {inv} spock-mesh-wire.yml --limit {} -e '{{\"inter_site_spock_peers\":[\"{site}\"],\"ory_spock_reconcile_history\":false}}'", sh_word(founder_group))));
        ev.push(p13("A", "[core, joiner] the joiner's Spock wiring again, now that the founder's half exists: Ory subscription + history reconcile", format!("cd '{r}/ops/ansible' && {pre} {inv} spock-mesh-wire.yml --limit {node}")));
        // And once the joiner's subscriptions from the founder have caught
        // up: a write made before they existed never arrives (GB-367).
        ev.push(p13("GATE", "[core, joiner] the seed reached the wire: once the subscriptions from the founder have caught up, every seed table equals the founder's (GB-367 names each table that differs)", format!("cd '{r}/ops/ansible' && {pre} {inv} -l {node} spock-seed-current-check.yml -e site_id={site} -e seed_check_phase=after")));
        Ok(())
    }

    /// The site's router and service leaves, minted and staged.
    fn site_leaves(&self, ev: &mut Vec<Phase>) -> Result<()> {
        let Converge { repo, r, site_group, pre, inv, .. } = *self;
        let (site, node) = (self.o.site, self.o.node);
        // The site's router and service leaves (gate row I3), before the
        // router, whose play reads the router leaf.
        let l = leaves(repo, site)?;
        ev.push(p13("T", &format!("[certificates] mint the site's leaves from ops/enrollment/manifests/{site}-certs.yaml (idempotent: a leaf already minted is kept)"), format!("'{r}/ops/scripts/mint-site-certs.sh' {}", sh_word(site))));
        if !l.staged.is_empty() {
            let only = format!("{{\"leaf_only\": [{}]}}", l.staged.iter().map(|n| format!("\"{n}\"")).collect::<Vec<_>>().join(", "));
            ev.push(p13("A", &format!("[certificates] stage the service leaves the manifest carries onto the node ({})", l.staged.join(", ")), format!("cd '{r}/ops/ansible' && {pre} {inv} stage-service-leaves.yml -e target={site_group} -l {node} -e leaf_src_dir={} -e {}", sh_word(&l.out_dir), sh_quote(&only))));
        }
        Ok(())
    }

    /// The site's front door, when signal federation did not set it up.
    fn front_door(&self, ev: &mut Vec<Phase>) {
        let Converge { r, gv, site_group, pre, inv, .. } = *self;
        let (site, node) = (self.o.site, self.o.node);
        // The site's front door (gate row I3). Signal federation sets it up
        // with the federation routers; a site that declares federation off
        // gets it here, for its profile: an internet site (ACME delivery,
        // as standup reads it) its public certificate and routes, a LAN
        // site its internal-CA routes, whose leaf is minted above.
        if !gv_first(gv, "signal_federation_absent_reason").is_empty() {
            if gv_matches(gv, "dashboard_cert_delivery_mode", "acme") {
                ev.push(p13("A", "[front door] the site's public certificate and routes (traefik-acme.yml, ACME DNS-01), limited to the node: an internet site (dashboard_cert_delivery_mode: acme) whose signal federation, which sets the door up otherwise, is declared off", format!("cd '{r}/ops/ansible' && {pre} {inv} traefik-acme.yml -e target={site_group} -l {node}")));
            } else {
                ev.push(p13("A", "[front door] the site's LAN TLS routes (lan-traefik-routes.yml, the internal-CA mgmt-identity leaf), limited to the node: a LAN site (no dashboard_cert_delivery_mode: acme) whose signal federation is declared off", format!("cd '{r}/ops/ansible' && {pre} {inv} lan-traefik-routes.yml -e target={site_group} -e site_id={site} -l {node}")));
            }
        }
    }

    /// The health publishers the site declares.
    fn health_publishers(&self, ev: &mut Vec<Phase>) -> Result<()> {
        let Converge { repo, r, site_group, pre, inv, .. } = *self;
        let (site, node) = (self.o.site, self.o.node);
        let l = leaves(repo, site)?;
        // The health publishers the site declares (gate row I3), on the
        // router, with the service leaves staged above.
        match declared_publishers(repo, site)? {
            Ok(pubs) if pubs.is_empty() => {
                let why = format!("[health publishers] none deployed: the end-state model marks no health publisher present at {site}");
                ev.push(p13("SKIP", &why, format!("echo {}", sh_quote(&format!(">> {why}")))));
            }
            Ok(pubs) => {
                // Every publisher reads the router's stack name (GB-361).
                let site_id =
                    crate::federate::preflight::group_var(repo, site_group, "site_id")
                        .unwrap_or_else(|| site.to_string());
                let roles: Vec<&str> = pubs.iter().map(|(name, _)| *name).collect();
                crate::zenoh_stack::check(repo, site_group, &site_id, &roles)?;
                // Each publisher runs with its own leaf, minted from the
                // manifest and staged above: one the manifest does not
                // carry is refused now, not at staging or at deploy.
                let no_leaf: Vec<&str> = pubs.iter().map(|(name, _)| *name).filter(|name| !l.staged.iter().any(|s| s == name)).collect();
                if !no_leaf.is_empty() {
                    let (them, add) = if no_leaf.len() == 1 { ("it", format!("a leaf named {}", no_leaf[0])) } else { ("them", "a leaf named for each of them".to_string()) };
                    return Err(goat_errors::coded("GB-364", format!("the end-state model marks {} present at {site}, and ops/enrollment/manifests/{site}-certs.yaml carries no leaf for {them}: join-node deploys each health publisher the site declares with the leaf it mints from that manifest. Add {add} to the manifest, as ops/enrollment/manifests/TEMPLATE-certs.yaml shows, or declare the publisher absent at {site} in ops/pins/fleet-pins.yaml with its reason", no_leaf.join(", "))).into());
                }
                for (name, play) in pubs {
                    ev.push(p13("A", &format!("[health publishers] {name} ({play}), limited to the node: the end-state model marks it present at {site}"), format!("cd '{r}/ops/ansible' && {pre} {inv} {play} -e target={site_group} -l {node}")));
                }
            }
            Err(why) => {
                let label = format!("[health publishers] not deployed: the end-state model was not read ({why}), so which publishers {site} declares is not known; fix that and resume with --from 13");
                ev.push(p13("SKIP", &label, format!("echo {}", sh_quote(&format!(">> {label}")))));
            }
        }
        Ok(())
    }

    /// The founder site's core node. Standup builds the founder's first core,
    /// so a core joined here is a second one at the same site, and the only
    /// second core a site runs is its streaming standby (`intra_site_pair`).
    /// Without that declaration site-bootstrap skips the standby and the node
    /// comes up as a second primary: its own empty NetBird database, and a
    /// management service whose OIDC endpoint is a Hydra that runs only on the
    /// primary (`ory-deploy.yml`), so it restarts until the run gives up. The
    /// step is then a refusal (GB-440), and `plan` refuses the run at its
    /// first phase too, before the node changes.
    fn founder_bootstrap(&self, ev: &mut Vec<Phase>) {
        let Converge { r, gv, site_group, pre, inv, .. } = *self;
        let (site, node) = (self.o.site, self.o.node);
        if !gv_matches(gv, "intra_site_pair", "true") {
            ev.push(p13("GATE", &format!("[core, founder site] refused: a core joined to {site} is its streaming standby, which the site does not declare (GB-440)"), format!("echo {} >&2; exit 1", sh_quote(&standby_refusal(site, site_group, node)))));
            return;
        }
        // A joiner's account arrives from the founder (seed-from-peer, then
        // Spock), and site-bootstrap mints an account, so needs an owner, only on
        // a founder (goatnet_founder): the owner is a founder's word alone, and an
        // unattended joiner run has no OWNER_EMAIL to set.
        let ev_secrets = self.ev_secrets();
        let ev_bootstrap = format!("-e site_id={site} -e bootstrap_owner_email=\"$OWNER_EMAIL\" {ev_secrets}");
        ev.push(p13("A", "[core, founder site] site-bootstrap on the node (intra_site_pair: the A→B streaming standby is phase 6A of the play)", format!("cd '{r}/ops/ansible' && {pre} {inv} -l {node} site-bootstrap.yml {ev_bootstrap}")));
    }
}

/// GB-440: a core joined to the founder's own site, which declares no standby.
fn standby_refusal(site: &str, site_group: &str, node: &str) -> String {
    format!("GB-440: {site} is the founder's site, and its first core is standup's: a core joined to it is the site's streaming standby, which its group_vars (group_vars/{site_group}) do not declare: intra_site_pair: true, with the values the postgres-streaming-replication role's README lists. Without it {node} would come up as a second primary with its own NetBird database. Declare the pair, or join {node} as the first core of a new site, one that declares goatnet_founder: false. Nothing was changed.")
}

fn plan(repo: &Path, o: &Options) -> Result<Vec<Phase>> {
    let targets = targets::load(repo)?;
    let target = targets.require(o.class)?;
    let converged = targets.converged(target)?;
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
    let group = target.group(site);
    // The config preflight judges a site from its core group; an obs node's
    // facts are checked through `--obs <group>` (the mgmt-stack rows do not
    // apply to an obs host), and a rescue node's site is its core group's.
    // A phone node's site is judged with its class's declarations under its
    // own (`--class phone`, gate row H5). The type's file says which.
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
    // Every play from phase 5 on names the node by the type's group, which
    // the type's own inventory may be the one to declare.
    let inv = inventories(o.via, &converged.inventory_arg(repo, site));
    let addr = o.address.unwrap_or("<the node's LAN address>");
    // `--via usb`: until the node has a mesh address (phases 1 to 11) ssh uses
    // the configuration the run wrote, and the plays the inventory line that
    // names it, loaded after the goatnet's inventories. From phase 12 the
    // node is reached at its mesh address, as every other node.
    let base = usb_base(repo, site, node);
    let node_ssh = if usb { format!("ssh -F {} {user_w}@{node}", sh_quote(&format!("{base}.ssh"))) } else { format!("ssh {user_w}@{addr}") };
    // The clock check (phase 3) connects as the other steps before phase 12 do.
    let clock_to = if usb { format!("-F {} {} {user_w}@{node}", sh_quote(&format!("{base}.ssh")), sh_word(node)) } else { format!("{} {user_w}@{addr}", sh_word(node)) };
    let inv_before = if usb { format!("{inv} -i {}", sh_quote(&format!("{base}.ini"))) } else { inv.to_string() };
    let pass_var = o.pass_var.map(String::from).unwrap_or_else(|| format!("{upper}_NODE_PASS"));
    let gv = repo.join("ops/ansible/group_vars").join(&site_group).join("vars.yml");
    let joiner = declares_joiner(&gv);
    let founder_site = gv_first(&gv, "join_founder_site");
    let founder_group = if founder_site.is_empty() { String::new() } else { format!("{}_hosts", founder_site.replace('-', "_")) };
    let values = Values::new(target, site, &founder_site);
    let preflight_target = target.preflight(&values);
    let converge = target.converges_as();
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
    // Phase 1 checks the node answers before anything is done on it, and goes
    // on (gate row H1: one run from phase 1 to 15). It stops only when the node
    // does not answer, with the step for a person (GB-314). It cannot prove a
    // login: before phase 2 seeds this machine's key, a password-only node
    // refuses every batch login (ops/scripts/join-node-reach.sh).
    let reach = if usb {
        p(1, "GATE", "the phone answers over USB (`adb get-state`: device) — attached, USB debugging allowed from this machine, unlocked; ANDROID_SERIAL picks one of several", format!("'{r}/ops/scripts/join-node-reach.sh' usb"))
    } else {
        p(1, "GATE", "the node answers over SSH at --address (its SSH server is reached; the login is proven by phase 2, which seeds the key) — on the site LAN, not a USB bridge address", format!("'{r}/ops/scripts/join-node-reach.sh' ssh {user_w}@{addr}"))
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
        // Before phase 4 changes anything on the node: its clock, read-only
        // (ADR 1232). GB-297 refuses an unsynchronized node or one over budget.
        p(3, "GATE", "the node's clock, read-only, before anything on it changes: synchronized to an outside reference and within the offset budget (JOIN_NODE_CLOCK_BUDGET_MS, default 250 ms of the router's 500 ms limit)", format!("python3 '{r}/ops/scripts/join-node-clock-check.py' {clock_to}")),
    ]);
    // A joining site's core node (and a phone, which converges as one): its
    // secrets, before phase 4 changes the node. The values every site shares
    // are taken from the founder, the site's own are minted, and each shared
    // value is compared with the founder's; nothing is overwritten
    // (docs/design/join-node-site-secrets.md).
    if converge == "core" && joiner && !founder_site.is_empty() {
        ev.push(p(3, "T", &format!("the site's secrets: the values every site shares (newsite-secrets.sh --cluster-json: NetBird's datastore key, the replicator, the relay secret, Ory's network id, the Kratos cipher, the Hydra system secret) are taken from {founder_site}'s, as references or under a value store; newsite-secrets.sh mints the site's own; each shared value is compared with the founder's, by digest. Nothing is overwritten: a joiner value that differs is refused (GB-353)"), format!("python3 '{r}/ops/scripts/join-site-secrets.py' {} --founder {}", sh_word(site), sh_word(&founder_site))));
    }
    // The node's key carries the groups the founder's own peer holds: the
    // site's INFRA group, the management hosts' tier. A core node takes them,
    // and so does a phone node, which runs the full core joiner set and
    // converges as core (docs/design/phone-node.md, "Its NetBird groups").
    // No other class may: a break-glass node reaches only INFRA and NETWORK on
    // 22 (ADR 1114), a network-jump host is kept out of INFRA (ADR 1131), and
    // no record puts an obs host there. Until a class declares its own
    // groups, its enrolment is refused (GB-430), and so is its run, at its
    // first phase, before the node changes.
    let groups_from_founder = matches!(o.class, "core" | "phone");
    let refusal = format!("GB-430: join-node enrols a --class {} node only once its class declares the NetBird groups its node joins; today only a core node and a phone node take the groups of the founder's own peer (the site's INFRA group), and that is not this class's to take. Nothing was changed.", o.class);
    let enrol = if groups_from_founder {
        p(10, "A", &format!("enrol the node: a single-use, one-hour setup key minted on the first host of {mint_group} (where the NetBird database is), with the groups that host's peer holds, handed to netbird up through a 0600 file on the node and removed; the key never reaches this machine"), format!("cd '{r}/ops/ansible' && {pre} {inv_before} mesh-enroll-host.yml -e target={group} -l {node} -e mesh_enroll_mint_group={} -e mesh_enroll_groups_from_peer=true -e mesh_ips_out={}", sh_word(&mint_group), sh_quote(&mesh_ips)))
    } else {
        p(10, "GATE", &format!("enrol the node: refused for --class {} until its class declares its NetBird groups (GB-430)", o.class), format!("echo {} >&2; exit 1", sh_quote(&refusal)))
    };
    if !groups_from_founder && o.from <= 10 && o.to >= 10 {
        // At the run's first phase, so a resume (--from 4) is refused before the node changes too.
        ev.insert(0, p(o.from.max(1), "GATE", &format!("--class {} enrols with groups of its own, which no class file declares yet: refused before the node changes (GB-430)", o.class), format!("echo {} >&2; exit 1", sh_quote(&refusal))));
    }
    ev.extend([
        p(4, "T", "quiet apt on the node (the baseline stalls on the dpkg lock otherwise; unmasked in phase 14)", format!("{node_ssh} 'sudo systemctl mask --now apt-daily.timer apt-daily-upgrade.timer'")),
        p(5, "A", "host baseline, narrowed to the node — run detached if the session may drop: it apt-upgrades and reboot-waves", format!("cd '{r}/ops/ansible' && {pre} {inv_before} mgmt-host-baseline.yml -e target={group} -e site_id={site} -l {node}")),
        p(6, "A", "WireGuard kernel modules, first pass — its wt0 check fails before enrolment (wt0 absent); expected, phase 11 re-runs it. Any other failure stops the run", format!("cd '{r}/ops/ansible' && log=$(mktemp) && if {pre} {inv_before} wireguard-kmod.yml -e target={group} -l {node} 2>&1 | tee \"$log\"; then rm -f \"$log\"; elif {WT0_ABSENT_ONLY} \"$log\" && grep -q 'wt0.*does not exist' \"$log\"; then rm -f \"$log\"; echo '>> wireguard-kmod: the wt0 check failed before enrolment (wt0 absent) — expected; phase 11 re-runs it for changed=0'; else echo \"!! wireguard-kmod failed on something other than the pre-enrolment wt0 check — output kept in $log\"; exit 1; fi")),
        p(7, "GATE", "the CA this site trusts is its own, not the shared dev root (silence puts a production node on the dev CA)", format!("python3 '{r}/ops/scripts/standup-config-preflight.py' {preflight_target} --before-enrolment && ! grep -rqE 'ca_id:[[:space:]]*\"?[a-z]+-goat-ca-' '{r}/ops/ansible/group_vars/{group}/' || {{ echo 'group_vars/{group} names the unnamed default CA (<mode>-goat-ca-<date>); a site node trusts its goatnet CA'; exit 1; }}")),
        p(8, "GATE", "the site declares where its nodes enrol (mesh_enrollment_url): phase 10 mints the setup key and enrols the node, and refuses before any key exists without it", format!("grep -rqsE '^mesh_enrollment_url:[[:space:]]*\\\"?https://' '{r}/ops/ansible/group_vars/{group}/' '{r}/ops/ansible/group_vars/{site_group}/' || {{ echo {}; exit 1; }}", sh_quote(&format!("set mesh_enrollment_url in group_vars/{site_group}/vars.yml: https://<the enrolment name of {enrol_at}>:443 (it resolves to the relays) or :8443 (it resolves to the host itself)")))),
        p(9, "GATE", "enrollment preflight on the node (clock skew, a stale wt0, an unreachable apex — each turns enrolment into a hang); no setup key is sent: phase 10 mints it where it is used", format!("{{ printf 'export NETBIRD_MANAGEMENT_URL=%q\\n' {}; cat '{r}/ops/preflight-enrollment.sh'; }} | {node_ssh} 'bash -s -- --key-at-enrolment'", sh_quote(&format!("https://{enrol_dns}")))),
        enrol,
        p(10, "GATE", "the node is on the kernel ACL plane, not only \"Connected\": netbird's interface is the kernel's, and `table ip netbird` is loaded", format!("{node_ssh} 'sudo netbird status | grep -q \"Interface type: Kernel\" && sudo nft list tables | grep -qx \"table ip netbird\"'")),
        record,
        p(11, "A", "WireGuard kernel modules again — changed=0 proves the kmod took, now that wt0 exists", format!("cd '{r}/ops/ansible' && {pre} {inv_before} wireguard-kmod.yml -e target={group} -l {node}")),
        p(12, "GATE", "mesh_ip + the mesh inventory filled in one change (mesh IPs are not stable across re-enrolment) — preflight passes with no TBD_", format!("python3 '{r}/ops/scripts/standup-config-preflight.py' {preflight_target} && ! grep -rqn 'TBD_' '{r}/ops/ansible/group_vars/{group}/' && ! grep -n 'TBD_' '{r}/ops/ansible/inventory/mgmt-hosts-mesh.ini' | grep -qw {node}")),
    ]);
    // What the type converges as: its file's phases, in order.
    let cx = Converge { repo, r: &r, o, gv: &gv, site_group: &site_group, group: &group, upper: &upper, founder_site: &founder_site, founder_group: &founder_group, pre: &pre, inv: &inv };
    let v13 = Values::new(converged, site, &founder_site).sourced(repo, converged);
    for e in converged.phases.iter().filter(|e| e.applies(joiner)) {
        if let Some(name) = &e.builtin {
            cx.builtin(name, &mut ev)?;
        } else if let Some(reason) = &e.skip {
            let label = e.tagged(&converged.name, &v13);
            let skip = |why: String| p(13, "SKIP", &label, format!("echo {}", sh_quote(&format!(">> {why}"))));
            let Some(c) = &e.component else {
                ev.push(skip(v13.render(reason)));
                continue;
            };
            match crate::release_deploy::pinned(repo, c, o.env, &group)? {
                crate::release_deploy::Pinned::No(why) => ev.push(skip(format!("{}: {why}", v13.render(reason)))),
                crate::release_deploy::Pinned::Yes { pin, play, target_flag } => {
                    let gb = format!("\"${{GOATBUILDER_BIN:-goatbuilder}}\" -C {}", sh_quote(&r));
                    let fetch = format!("{gb} release-file vars {c} --pin {}", sh_quote(&pin));
                    // Before phase 4 changes the node, every check deploy makes
                    // before a host, by deploy itself: a dry run runs the fresh
                    // source, ladder, digest (GB-382), signed intent (GB-383),
                    // release (GB-381) and secrets checks, opens no door, and
                    // exits 1 on any refusal. What deploy would refuse leaves the
                    // node as it was; there is no install path with fewer checks.
                    let at = ev.iter().position(|x| x.n > 3).unwrap_or(ev.len());
                    ev.insert(at, p(3, "GATE", &format!("{c}'s release {pin} passes every check deploy makes before a host (fresh source, ladder, digest, signed intent, release, secrets), before anything on the node changes"), format!("{gb} deploy --env {} --component {c} --group {group} --dry-run", o.env)));
                    let play = play.file_name().unwrap_or_default().to_string_lossy().to_string();
                    ev.push(p(13, "A", &format!("{label}: {c} {pin}, the verified file of the node's architecture"), format!("vars=$({fetch}) && cd '{r}/ops/ansible' && {pre} {inv} {play} {target_flag}{group} -e @\"$vars\" -l {node}")));
                }
            }
        } else {
            ev.push(p(13, "A", &e.tagged(&converged.name, &v13), v13.command(&e.plays, &r, &pre, &inv, &format!(" -l {node}"))));
        }
    }
    // A core at the founder's site with no standby declared is refused at the
    // run's first phase too, so a run (or a resume) stops before the node changes.
    if o.from <= 13 && o.to >= 13 && ev.iter().any(|ph| ph.n == 13 && ph.cmd.contains("GB-440: ")) {
        ev.insert(0, p(o.from.max(1), "GATE", &format!("a core joined to {site} is its streaming standby, which the site does not declare: refused before the node changes (GB-440)"), format!("echo {} >&2; exit 1", sh_quote(&standby_refusal(site, &site_group, node)))));
    }
    // The services its type lists, through deploy itself, on this machine alone.
    for c in &target.services {
        let label = format!("[{}] service {c}", target.name);
        match crate::services::on(repo, &target.name, c, o.env, &group)? {
            crate::services::Service::Absent => ev.push(p(13, "SKIP", &label, format!("echo {}", sh_quote(&format!(">> ops/pins/fleet-pins.yaml declares {c} absent on {group}"))))),
            crate::services::Service::Pinned(pin) => {
                let (cmd, unacked) = crate::services::deploy_command(repo, &r, "join-node", c, &crate::services::Onto { goatnet: o.env, group: &group, node: Some(node) }, o.ack);
                let note = if unacked.is_some() { " (its rollout needs --ack: a run refuses before phase 1 without it)" } else { "" };
                ev.push(p(13, "A", &format!("{label}: {pin}, deployed to the node and read back from its ledger{note}"), cmd));
            }
        }
    }
    // Unmasking alone leaves the timers stopped until the next boot, so the node
    // took no automatic updates until then: they are enabled and started too.
    let unmask = "'sudo systemctl unmask apt-daily.timer apt-daily-upgrade.timer && sudo systemctl enable --now apt-daily.timer apt-daily-upgrade.timer'";
    if o.via == "door" {
        // Through the env door, this machine is not on the goatnet's mesh, so
        // the node's mesh address is not one it can reach directly.
        ev.push(p(14, "T", "unmask and start the apt timers now that the node is finished", format!("ssh {user_w}@{addr} {unmask}")));
    } else {
        // From phase 12 the node is reached at its mesh address, as the plays
        // of phase 13 reach it: the one phase 10 recorded in the mesh
        // inventory. The LAN address (--address) is for phases 1 to 11.
        let missing = sh_quote(&format!("inventory/mgmt-hosts-mesh.ini gives {node} no mesh address under [{group}]; phase 10 records it"));
        ev.push(p(14, "T", "unmask and start the apt timers now that the node is finished (over the mesh: the node's address is the one phase 10 recorded)", format!("cd '{r}/ops/ansible' && t=$(python3 ../scripts/inventory-hosts.py {group} --inventory inventory/mgmt-hosts-mesh.ini --ssh-target --user {user_w} --host {node}) && [ -n \"$t\" ] || {{ echo {missing}; exit 1; }}; ssh \"$t\" {unmask}")));
    }
    // Standup's acceptance line: the site's end-state model first, so a row
    // whose services the site declares absent reads N/A, not FAIL (ADR 1206).
    ev.push(p(15, "GATE", "done means proven: every applicable acceptance row PASS, never UNKNOWN (a joiner adds the replicating-both-ways evidence)", crate::assay::phase13(repo, site, &crate::settings::state_dir_abs(repo)).phase_line()));
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

/// The transport's inventories (`inventories`), for `goatbuilder inputs`: the
/// goatnet's base and the override; a type's own is its declaration's.
pub(crate) fn inventories_of(via: &str) -> String {
    inventories(via, "")
}

/// The environment, site and class are declared, the class is a type this
/// checkout has, it accepts the machine's family, where that is declared, and
/// the machine declares what its type requires (a tap's mode).
fn declared(repo: &Path, o: &Options) -> Result<()> {
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
    let targets = targets::load(repo)?;
    let t = targets.require(o.class)?;
    targets.check_family(repo, t, o.site, o.node)?;
    // A network tap states its mode (listen or inline); there is no default.
    // A type's inputs are set, and not the role's placeholder (GB-414).
    targets.declarations(repo, t, o.site, o.node).map(|_| ())
}

pub fn join_node(repo: &Path, o: &Options) -> Result<Report> {
    declared(repo, o)?;
    execute(repo, o)
}

/// `join-node ... --reseed`: the node, already joined, seeded again from its
/// founder (crate::reseed). Only a node that replicates has a slot to lose.
pub fn reseed(repo: &Path, o: &Options) -> Result<Report> {
    if targets::load(repo)?.get(o.class).is_some_and(|t| t.converges_as() != "core") {
        return Err(goat_errors::coded("GB-247", format!("--reseed is for a node that replicates (class core or phone); a {} node is not a Spock member and has no slot to lose", o.class)).into());
    }
    declared(repo, o)?;
    crate::reseed::execute(repo, o)
}

/// The end-state model's script, relative to the checkout.
const MODEL: &str = "ops/goatbuilder/endstate/endstate.py";

/// The end-state model's services at `site` (`endstate.py model <site>`),
/// or why the model could not be read: the script is not in the checkout,
/// would not start, failed, or printed no services.
pub(crate) fn model_services(repo: &Path, site: &str) -> std::result::Result<serde_json::Map<String, serde_json::Value>, String> {
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

/// What the plan's steps build, by the play each one runs, in the end-state
/// catalog's service names (gate row I3): the plan and the model are compared
/// name for name. The health publishers are `PUBLISHERS`; the site's
/// bootstrap play builds `BOOTSTRAP_BUILDS`. A test holds every name here to
/// the catalog and every play to the repository.
const BUILDERS: [(&str, &[&str]); 8] = [
    ("ory-deploy.yml", &["ory", "kratos-selfservice-ui"]),
    ("signal-federation-transport.yml", &["federation-proxy"]),
    ("traefik-acme.yml", &["traefik"]),
    ("lan-traefik-routes.yml", &["traefik"]),
    ("zenoh-deploy.yml", &["zenoh-router"]),
    ("goat-coredns.yml", &["mesh-coredns"]),
    ("obs-host-bootstrap.yml", &["prometheus", "alertmanager", "grafana", "obs-exporters"]),
    // A single-host joiner's core node, run with grafana_enabled=false.
    ("obs.yml", &["prometheus", "alertmanager", "obs-exporters"]),
];

/// What a site's bootstrap play (site-bootstrap.yml, through mgmt-host.yml)
/// builds: the replication core of passes 1 and 3, and admit-audit (its role,
/// on unless the site sets `admit_audit_enabled: false`).
const BOOTSTRAP_BUILDS: [&str; 6] = ["postgres-spock", "netbird-mgmt", "signal-server", "pgbackrest", "traefik", "admit-audit"];

/// The catalog services the plan's steps build. A gate, a SKIP line and a
/// person's step build nothing, so a step the site declares off (signal
/// federation, a publisher marked absent) is not counted.
fn built_by(repo: &Path, site: &str, phases: &[Phase]) -> std::collections::BTreeSet<&'static str> {
    let own = crate::standup::bootstrap_play(repo, site);
    let own = own.split_whitespace().next().unwrap_or("").to_string();
    let re = regex::Regex::new(r"[A-Za-z0-9_.-]+\.yml\b").expect("a valid pattern");
    let mut built = std::collections::BTreeSet::new();
    for ph in phases.iter().filter(|p| p.kind == "A" || p.kind == "T") {
        for m in re.find_iter(&ph.cmd) {
            let play = m.as_str();
            if play == own || play == "site-bootstrap.yml" {
                built.extend(BOOTSTRAP_BUILDS);
            }
            built.extend(BUILDERS.iter().filter(|(p, _)| *p == play).flat_map(|(_, s)| s.iter().copied()));
            built.extend(PUBLISHERS.iter().filter(|(_, p)| *p == play).map(|(s, _)| *s));
        }
    }
    built
}

/// Each service the model marks present on this node that no step of the
/// run's plan builds and the catalog does not already name as a gap (gate row
/// I3): at a site with its own obs host (`has_obs_host: true`), one the same
/// verb builds for the node's other class (core, obs) names that run; one no
/// step builds names gate row I3. A single-host site's obs stack, which the
/// model places on its core node, is never sent to an obs run: that run would
/// re-enrol a node already built; the core run's own `obs.yml` step builds it
/// (`single_host_obs`), Grafana aside. A node the model's hosts do not
/// list is read by class: a core run answers for the mgmt-class services, an
/// obs run for the obs-class ones. Only a joining site's core and obs nodes;
/// a founder's is standup's to build.
fn unbuilt(repo: &Path, o: &Options, phases: &[Phase], services: &serde_json::Map<String, serde_json::Value>) -> Vec<Check> {
    let gv = repo.join("ops/ansible/group_vars").join(format!("{}_hosts", o.site.replace('-', "_"))).join("vars.yml");
    let targets = targets::load(repo).ok();
    let (here, other) = match targets.as_ref().and_then(|t| t.get(o.class)).map(TargetType::converges_as) {
        Some("core") => ("mgmt", "obs"),
        Some("obs") => ("obs", "core"),
        _ => return Vec::new(),
    };
    if !gv_matches(&gv, "goatnet_founder", "false") {
        return Vec::new();
    }
    let built = built_by(repo, o.site, phases);
    let by_other = if gv_matches(&gv, "has_obs_host", "true") { plan(repo, &Options { class: other, ..*o }).map(|p| built_by(repo, o.site, &p)).unwrap_or_default() } else { Default::default() };
    let on = |s: &serde_json::Value| s.get("hosts").and_then(|h| h.as_array()).is_some_and(|h| h.iter().any(|n| n.as_str() == Some(o.node)));
    let placed = services.values().any(on);
    let mut rows = Vec::new();
    for (name, s) in services {
        if s.get("present") != Some(&serde_json::Value::Bool(true)) || s.get("gap").is_some_and(|g| g.is_object()) || built.contains(name.as_str()) {
            continue;
        }
        let here_too = if placed { on(s) } else { s.get("class").and_then(|c| c.as_str()) == Some(here) };
        if !here_too {
            continue;
        }
        let why = if by_other.contains(name.as_str()) {
            format!("declared at {}, built by `join-node --class {other}`, not by this run", o.node)
        } else {
            format!("declared at {}, and no join-node step builds it yet (gate row I3)", o.node)
        };
        rows.push(row("not-run", not_run_line(name, &why)));
    }
    rows
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
/// With the run's plan, each declared service it does not build follows
/// (`unbuilt`).
fn not_run(repo: &Path, o: &Options, phases: Option<&[Phase]>) -> Vec<Check> {
    let site = o.site;
    let shown = format!("{MODEL} model {site}");
    let mut rows = vec![row("not-run-heading", format!("What {site} does not run yet: each gap the end-state model names, with the gate row that closes it, each service the site turns off, and each declared service this run does not build ({shown})"))];
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
        // What closes it: its gate rows, or the decision it waits on
        // (endstate.py, `gap_closes`).
        let closes = if field("row").is_empty() { field("decision") } else { format!("gate row {}", field("row")) };
        rows.push(row(
            "not-run",
            not_run_line(
                name,
                &format!(
                    "declared, not yet built at this site: {} ({closes})",
                    field("why"),
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
    // Last, what the site declares on this node that this run does not build.
    if let Some(phases) = phases {
        rows.extend(unbuilt(repo, o, phases, &services));
    }
    if rows.len() == before {
        rows.push(row("not-run", "nothing: the model names no gap here, and the site turns nothing off".into()));
    }
    rows
}

/// The refusal for a play phase 13 runs that this tree does not have (GB-377).
fn missing_play(repo: &Path, o: &Options) -> Result<Option<anyhow::Error>> {
    let targets: Targets = targets::load(repo)?;
    let t = targets.converged(targets.require(o.class)?)?;
    let joiner = declares_joiner(&repo.join("ops/ansible/group_vars").join(format!("{}_hosts", o.site.replace('-', "_"))).join("vars.yml"));
    let plays = || t.phases.iter().filter(|e| e.applies(joiner)).flat_map(|e| &e.plays);
    if let Some(e) = targets::missing_play_refusal(repo, &t.name, plays(), ". Phases 1 to 12 run without it: --to 12") {
        return Ok(Some(e));
    }
    Ok(targets::source_refusal(repo, t, plays()))
}

/// GB-421 for a service the type lists whose rollout needs an acknowledgement
/// that this run was not given: before phase 1, not at phase 13 once the node
/// has changed.
fn unacked_service(repo: &Path, o: &Options) -> Result<Option<anyhow::Error>> {
    let targets: Targets = targets::load(repo)?;
    let t = targets.require(o.class)?;
    let group = t.group(o.site);
    for c in &t.services {
        if let crate::services::Service::Pinned(_) = crate::services::on(repo, &t.name, c, o.env, &group)? {
            if let (_, Some(why)) = crate::services::deploy_command(repo, &repo.display().to_string(), "join-node", c, &crate::services::Onto { goatnet: o.env, group: &group, node: Some(o.node) }, o.ack) {
                return Ok(Some(crate::services::ack_refusal(why)));
            }
        }
    }
    Ok(None)
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
        let gv = repo.join("ops/ansible/group_vars").join(format!("{}_hosts", o.site.replace('-', "_"))).join("vars.yml");
        if !declares_joiner(&gv) {
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
    // Before any host: a play the type converges with, from another repository
    // or an overlay, that this tree was assembled without.
    if !o.dry_run {
        targets::unverified_refusal(repo)?;
    }
    if !o.dry_run && (o.from..=o.to).contains(&13) {
        if let Some(e) = missing_play(repo, o)? {
            return Err(e);
        }
        if let Some(e) = unacked_service(repo, o)? {
            return Err(e);
        }
    }
    // Before any host: a phase whose command reads a variable that is not set
    // would stop on bash's own `unbound variable`, partway through the node.
    if !o.dry_run {
        if let Some(e) = unset_variable_refusal(&phases, o.from, o.to, &|k: &str| std::env::var(k).ok()) {
            return Err(e);
        }
    }
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
    // What the machine declares for its type, as the run record keeps it: a
    // tap's mode, and "inline, fail-closed" when the link depends on it.
    if let Ok(t) = targets::load(repo) {
        if let Ok(ty) = t.require(o.class) {
            for (line, at) in t.declarations(repo, ty, o.site, o.node)? {
                checks.push(row("declared", format!("{}: {line} ({at})", o.node)));
            }
        }
    }
    if o.dry_run {
        checks.extend(listing_probes.iter().map(|l| row("probe-ran", l.clone())));
    }
    let mut exit = EXIT_OK;
    // Each executed step is a row in <state_dir>/join-node-<site>-<node>-timings.tsv,
    // with standup's columns, and the summary gets standup's timing line.
    let (state_dir, timing_file) = (crate::settings::state_dir_abs(repo), format!("join-node-{}-{}-timings.tsv", o.site, o.node));
    let mut timings: Vec<(u32, String, u64)> = Vec::new();
    let run_started = std::time::Instant::now();
    if o.via == "door" {
        let open = door_open_cmd(repo, o.env);
        checks.push(row("door", format!("=== Before phase 1: open the env door to {} — every play below goes through it (mgmt-hosts.ini + doors-groupvars.ini, never the mesh inventory too) ===", o.env)));
        checks.push(row("cmd", open.clone()));
        if !o.dry_run {
            crate::runner::phase("door", &format!("open the env door to {}", o.env), "");
            let status = Cmd::new("bash").args(["-euo", "pipefail", "-c", &open]).current_dir(repo).status().map_err(|e| goat_errors::coded("GB-320", format!("could not start bash: {e}")))?;
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
            let status = Cmd::new("bash").args(["-euo", "pipefail", "-c", &setup]).current_dir(repo).status().map_err(|e| goat_errors::coded("GB-320", format!("could not start bash: {e}")))?;
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
        let t0 = std::time::Instant::now();
        let mut bash = Cmd::new("bash");
        bash.args(["-euo", "pipefail", "-c", &ph.cmd]).current_dir(repo);
        // A phase that calls goatbuilder calls this one, on PATH or not.
        if let (None, Ok(exe)) = (std::env::var_os("GOATBUILDER_BIN"), std::env::current_exe()) {
            bash.env("GOATBUILDER_BIN", exe);
        }
        let status = bash.status().map_err(|e| goat_errors::coded("GB-320", format!("could not start bash: {e}")))?;
        // Timed as standup times its phases (standup::record_timing).
        let (secs, rc_now) = (t0.elapsed().as_secs(), status.code().unwrap_or(1));
        checks.push(row("time", format!("phase {} took {secs}s (exit {rc_now})", ph.n)));
        timings.push((ph.n, ph.label.clone(), secs));
        let cause = if status.success() { "ok".to_string() } else { crate::explain_phase::cause("join-node", &ph.n.to_string(), crate::explain_phase::runner_log(&ph.n.to_string()).as_deref()) };
        crate::standup::record_timing(&state_dir, &timing_file, ph.n, &ph.label, secs, rc_now, &cause);
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
            checks.push(Check { name: "failed".into(), ok: false, detail: format!("!! Phase {} (\"{}\") FAILED — exit {rc} (GB-332). No later phase has run; the node is left partway through phase {}. `--dry-run` prints this phase's command and the gate that proves it.", ph.n, ph.label, ph.n), fix: Some(format!("fix the cause, then resume — every phase is idempotent: {resume} --from {}", ph.n)) });
            // F5: the phase's output, as the runner kept it, matched against
            // the error catalog (explain_phase::interpreted), as standup does.
            let id = ph.n.to_string();
            checks.extend(crate::explain_phase::interpreted("join-node", &id, crate::explain_phase::runner_log(&id).as_deref()));
            exit = EXIT_REFUSED;
            break;
        }
    }
    if !o.dry_run && !timings.is_empty() {
        checks.push(row("timings", crate::standup::timing_summary(&timings, run_started.elapsed().as_secs())));
    }
    // What the site does not run yet, in the plan and in a real run's summary alike.
    checks.extend(not_run(repo, o, Some(&phases)));
    if o.dry_run {
        checks.extend(crate::inputs::rows(&listed));
        let (needs, answers) = if o.via == "usb" {
            ("takes no --address: the phone is found over USB", "the phone answers over USB")
        } else {
            ("needs --address", "the node answers over SSH at --address")
        };
        checks.push(row("footer", format!("PLAN complete. Remove --dry-run to execute ({needs}). Phase 1 checks that {answers}, and the run goes on to phase 15 with no operator step; a node that does not answer stops the run at phase 1 with GB-314: resume with --from 1. docs/tutorials/02-first-federated-node.md walks through it.")));
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

    /// C5 drill log: phase 13 died on `bash: line 1: OWNER_EMAIL: unbound
    /// variable`, with no catalogued error. A run now refuses before any phase,
    /// with GB-369, naming the first phase in range that reads a variable that
    /// is not set, and each such variable; a phase outside the range, or a
    /// manual one, is not read.
    #[test]
    fn a_phase_that_reads_an_unset_variable_is_refused_before_any_phase_runs() {
        let ph = |n: u32, kind: &'static str, cmd: &str| Phase { n, kind, label: format!("phase {n}"), cmd: cmd.into(), writes: vec![] };
        let phases = vec![
            ph(12, "AUTO", "echo ready"),
            ph(13, "AUTO", r#"ansible-playbook site-bootstrap.yml -e bootstrap_owner_email="$OWNER_EMAIL" -e t=$OTHER_TOKEN"#),
            ph(14, "MANUAL", r#"echo "$NOT_RUN_HERE""#),
        ];
        let none = |_: &str| None;
        let e = unset_variable_refusal(&phases, 0, 99, &none).expect("refused");
        assert_eq!(e.downcast_ref::<goat_errors::Coded>().map(|c| c.code.as_str()), Some("GB-369"), "{e}");
        let text = e.to_string();
        assert!(text.contains("phase 13") && text.contains("OWNER_EMAIL") && text.contains("OTHER_TOKEN"), "{text}");
        assert!(!text.contains("NOT_RUN_HERE"), "a manual phase runs nothing: {text}");
        assert!(text.contains("export OTHER_TOKEN=<value> OWNER_EMAIL=<value>") && text.contains("no phase has run"), "{text}");
        assert!(unset_variable_refusal(&phases, 14, 99, &none).is_none(), "phase 13 is outside --from 14");
        let set = |n: &str| (n == "OWNER_EMAIL" || n == "OTHER_TOKEN").then(|| "v".to_string());
        assert!(unset_variable_refusal(&phases, 0, 99, &set).is_none());

        // A joiner's real plan reads no variable as $NAME: its secrets reach
        // the plays through lookup("env") and with-secrets, so the check never
        // stops a joiner that a founder's OWNER_EMAIL would not stop.
        let t = joiner_checkout();
        for class in ["core", "obs"] {
            let plan = plan(t.path(), &opts("zagros-b", class, "zagros")).unwrap();
            let reads: Vec<String> = plan.iter().flat_map(|p| crate::envrefs::unset(&p.cmd, &none)).collect();
            assert!(reads.is_empty(), "{class}: {reads:?}");
        }
    }

    fn opts<'a>(site: &'a str, class: &'a str, env: &'a str) -> Options<'a> {
        Options { ack: "", env, site, node: "node-x", class, address: None, pass_var: None, via: "mesh", from: 0, to: 99, dry_run: true }
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

    /// Only a core node, and a phone node (which converges as core), takes the
    /// groups of the founder's own peer (the site's INFRA group). Every other
    /// class is refused (GB-430): at phase 1, before
    /// the node changes, when the run reaches phase 10, and at phase 10 itself.
    /// No class gains a group it could not get before.
    #[test]
    fn only_a_core_node_enrols_with_the_founder_peers_groups() {
        let t = joiner_checkout();
        let ph = plan(t.path(), &opts("zagros-b", "core", "zagros")).unwrap();
        assert!(ph.iter().any(|p| p.n == 10 && p.cmd.contains("-e mesh_enroll_groups_from_peer=true")));
        assert!(!ph.iter().any(|p| p.cmd.contains("GB-430")));
        let phone = plan(t.path(), &Options { via: "usb", ..opts("zagros-b", "phone", "zagros") }).unwrap();
        assert!(phone.iter().any(|p| p.n == 10 && p.cmd.contains("-e mesh_enroll_groups_from_peer=true")), "a phone node takes core's groups");
        assert!(!phone.iter().any(|p| p.cmd.contains("GB-430")));
        for class in ["obs", "breakglass", "network-jump"] {
            let ph = match plan(t.path(), &opts("zagros-b", class, "zagros")) { Ok(ph) => ph, Err(_) => continue };
            assert!(!ph.iter().any(|p| p.cmd.contains("mesh_enroll_groups_from_peer")), "{class}");
            assert!(ph[0].n == 1 && ph[0].kind == "GATE" && ph[0].cmd.contains("GB-430: join-node enrols a --class") && ph[0].cmd.ends_with("exit 1"), "{class}: {} {}", ph[0].label, ph[0].cmd);
            assert!(ph.iter().any(|p| p.n == 10 && p.kind == "GATE" && p.cmd.contains("GB-430")), "{class}");
            let mut early = opts("zagros-b", class, "zagros");
            early.to = 9;
            let ph = plan(t.path(), &early).unwrap();
            assert!(!ph.iter().any(|p| p.n == 1 && p.cmd.contains("GB-430")), "{class}: a run that stops before enrolment is not refused at phase 1");
            let mut resume = opts("zagros-b", class, "zagros");
            resume.from = 5;
            let ph = plan(t.path(), &resume).unwrap();
            assert!(ph[0].n == 5 && ph[0].cmd.contains("GB-430"), "{class}: a resume from 5 is refused at 5, before the baseline");
        }
    }

    /// The mesh DNS step writes the site's nameserver group through the
    /// founder's management API, at the name the founder declares for its
    /// management (public_mgmt_fqdn): on a LAN goatlite that is netbird.<zone>,
    /// and the zone itself answers nothing. A founder that declares none is
    /// reached at its zone's apex, as before.
    #[test]
    fn mesh_dns_writes_at_the_founders_declared_management_name() {
        let t = joiner_checkout();
        let gvf = t.path().join("ops/ansible/group_vars/zagros_hosts");
        std::fs::create_dir_all(&gvf).unwrap();
        let api = |t: &tempfile::TempDir| mesh_dns_values(t.path(), &t.path().join("ops/ansible/group_vars/zagros_b_hosts/vars.yml"), "zagros-b", "zagros").unwrap().api;
        std::fs::write(gvf.join("vars.yml"), "goatnet_founder: true\nsite_dns_zone: zagros.example.org\nsite_enrollment_dns: \"netbird.zagros.example.org\"\n").unwrap();
        assert_eq!(api(&t), "https://zagros.example.org/api", "no management name declared: the zone's apex");
        std::fs::write(gvf.join("vars.yml"), "goatnet_founder: true\nsite_dns_zone: zagros.lan\npublic_mgmt_fqdn: netbird.zagros.lan\n").unwrap();
        assert_eq!(api(&t), "https://netbird.zagros.lan/api", "a LAN goatlite's management");
        std::fs::write(gvf.join("vars.yml"), "goatnet_founder: true\nsite_dns_zone: zagros.lan\npublic_mgmt_fqdn: \"{{ site_enrollment_dns }}\"\n").unwrap();
        assert_eq!(api(&t), "https://zagros.lan/api", "a templated name is not read as one");
    }


    /// Gate row H2: only phase 1 is a person's step. Phase 9 sends no setup
    /// key; phase 10 mints it where the NetBird database is (the founder's
    /// first host for a joiner, the site's own for a founder), enrols the node,
    /// proves the kernel ACL plane, and records the mesh address, the core
    /// pair's for a core node and only the mesh inventory line for another class.
    #[test]
    fn no_phase_is_manual_and_the_key_is_minted_where_it_is_used() {
        let t = joiner_checkout();
        let ph = plan(t.path(), &opts("zagros-b", "core", "zagros")).unwrap();
        let manual: Vec<u32> = ph.iter().filter(|p| p.kind == "MANUAL").map(|p| p.n).collect();
        assert_eq!(manual, Vec::<u32>::new(), "phase 1 checks the node answers; no phase stops for a person");
        assert_eq!((ph[0].n, ph[0].kind), (1, "GATE"));
        assert!(ph[0].cmd.ends_with("/ops/scripts/join-node-reach.sh' ssh <deploy.ssh_user>@<the node's LAN address>"), "{}", ph[0].cmd);
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

    /// Standup builds a founder site's first core, so a core joined to that
    /// site is its streaming standby. A site that does not declare the pair
    /// (a goatlite) is refused when the plan is made, naming the key: run
    /// anyway, the node came up as a second primary whose management service
    /// waited for a Hydra that runs only on the primary.
    #[test]
    fn a_core_at_the_founders_site_needs_the_standby_pair_declared() {
        let t = joiner_checkout();
        let gvf = t.path().join("ops/ansible/group_vars/zagros_hosts");
        std::fs::create_dir_all(&gvf).unwrap();
        std::fs::write(gvf.join("vars.yml"), "goatnet_founder: true\n").unwrap();
        let ph = plan(t.path(), &opts("zagros", "core", "zagros")).unwrap();
        let refused = |ph: &Phase| ph.kind == "GATE" && ph.cmd.contains("GB-440: zagros is the founder") && ph.cmd.contains("intra_site_pair: true") && ph.cmd.contains("(group_vars/zagros_hosts)") && ph.cmd.ends_with("exit 1");
        assert!(ph[0].n == 1 && refused(&ph[0]), "phase 1 refuses: {} {}", ph[0].label, ph[0].cmd);
        assert!(ph.iter().any(|p| p.n == 13 && refused(p)), "phase 13's step is the refusal");
        assert!(!ph.iter().any(|p| p.cmd.contains("site-bootstrap.yml")), "site-bootstrap is never planned");
        let mut resume = opts("zagros", "core", "zagros");
        resume.from = 13;
        let ph = plan(t.path(), &resume).unwrap();
        assert!(ph[0].n == 13 && refused(&ph[0]), "a resume from 13 is refused first");
        let mut early = opts("zagros", "core", "zagros");
        early.to = 12;
        assert!(!plan(t.path(), &early).unwrap().iter().any(|p| p.n < 13 && p.cmd.contains("GB-440")), "a run that stops before 13 is not refused");
        std::fs::write(gvf.join("vars.yml"), "goatnet_founder: true\nintra_site_pair: true\n").unwrap();
        let ph = plan(t.path(), &opts("zagros", "core", "zagros")).unwrap();
        assert!(!ph.iter().any(|p| p.cmd.contains("GB-440")));
        assert!(ph.iter().any(|p| p.n == 13 && p.label.starts_with("[core, founder site] site-bootstrap") && p.cmd.contains("site-bootstrap.yml")));
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
        assert!(phases[0].starts_with("=== Phase 1 [GATE] the node answers over SSH"));
        let core: Vec<&&str> = phases.iter().filter(|p| p.contains("[core, joiner]")).collect();
        assert_eq!(core.len(), 8, "{phases:?}");
        assert!(core[0].contains("pass 1") && core[2].contains("pass 2") && core[3].contains("the seed is still current") && core[4].contains("pass 3") && core[5].contains("founder-side mirror on zagros_hosts") && core[6].contains("Spock wiring again") && core[7].contains("the seed reached the wire"));
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
            format!("goatnet_founder: false\njoin_founder_site: {founder}\nsite_enrollment_dns: e$(touch${{IFS}}{m})x\nsite_dns_zone: zagros.example.org\n"),
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
        for s in ["ops/scripts/with-aws", "ops/scripts/ansible-preflight.sh", "ops/scripts/seed-ssh-key.sh", "ops/scripts/door-open.sh", "ops/scripts/phone-node-provision.sh", "ops/scripts/phone-node-ssh.sh", "ops/scripts/join-node-reach.sh"] {
            exe(&repo.join(s));
        }
        std::fs::write(repo.join("ops/preflight-enrollment.sh"), "true\n").unwrap();
        // The site's leaf-cert manifest, which the certificate steps read.
        std::fs::create_dir_all(repo.join("ops/enrollment/manifests")).unwrap();
        std::fs::write(repo.join("ops/enrollment/manifests/zagros-b-certs.yaml"), "site_id: zagros-b\nleaves:\n  - name: router\n    cn: goat-zenoh-router\n  - name: fabric-health\n    cn: fabric-health-trial-peer\n").unwrap();
        exe(&repo.join("ops/scripts/mint-site-certs.sh"));
        // The goatnet's mesh zone, which the joiner's mesh DNS step serves.
        std::fs::create_dir_all(repo.join("ops/dns/zagros")).unwrap();
        std::fs::write(repo.join("ops/dns/zagros/mesh-hosts"), "100.127.0.1 isitup.zagros.zagros.example.org\n").unwrap();
        std::fs::create_dir_all(repo.join("ops/ansible/inventory")).unwrap();
        let path = format!("{}:/usr/bin:/bin", bin.display());
        let mut ran = 0;
        for (class, vias) in [("core", &["mesh", "door"][..]), ("obs", &["mesh", "door"]), ("phone", &["usb"])] {
            for via in vias.iter().copied() {
                let o = Options { ack: "", env: "zagros", site: "zagros-b", node: "node-x", class, address: (via != "usb").then_some("192.0.2.30"), pass_var: None, via, from: 1, to: 15, dry_run: true };
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
        assert_eq!(ph.iter().filter(|p| p.kind == "MANUAL").count(), 0);
        assert!(ph[0].kind == "GATE" && ph[0].label.contains("the phone answers over USB") && ph[0].cmd.ends_with("/ops/scripts/join-node-reach.sh' usb"), "{}", ph[0].cmd);
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
        assert_eq!(ph.iter().filter(|p| p.label.contains("[core, joiner]") && p.kind != "GATE").count(), 6);
        assert_eq!(ph.iter().filter(|p| p.cmd.contains("spock-seed-current-check.yml")).count(), 2, "a phone joiner's seed is checked too");
        assert_eq!(targets::load(t.path()).unwrap().require("phone").unwrap().group("zagros-b"), "zagros_b_hosts");
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
        let rows = not_run(t.path(), &opts("zagros-b", "core", "zagros"), None);
        assert_eq!(rows.len(), 2, "{:?}", rows.iter().map(|r| &r.detail).collect::<Vec<_>>());
        assert_eq!(rows[0].name, "not-run-heading");
        assert!(rows[0].detail.starts_with("What zagros-b does not run yet:") && rows[0].detail.contains("ops/goatbuilder/endstate/endstate.py model zagros-b"), "{}", rows[0].detail);
        assert_eq!(rows[1].name, "not-run");
        assert_eq!(rows[1].detail, "not read: ops/goatbuilder/endstate/endstate.py is not in this checkout; this list of what the site does not run yet is left out");
        assert!(rows.iter().all(|r| r.ok), "a missing list is a stated line, not a refusal");
    }

    /// A gap names what closes it: its gate rows, or, when no gate row does,
    /// the decision it waits on (a site's own SPIRE server: ADR 1164).
    #[test]
    fn a_gap_names_its_gate_row_or_the_decision_it_waits_on() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path().join("ops/goatbuilder/endstate");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("endstate.py"), "import json\nprint(json.dumps({'services': {'portal': {'present': True, 'gap': {'row': 'I1, I4', 'why': 'w1'}}, 'spire': {'present': True, 'gap': {'row': '', 'why': 'w2', 'decision': 'ADR 1164'}}}}))\n").unwrap();
        let rows: Vec<String> = not_run(t.path(), &opts("zagros-b", "core", "zagros"), None).into_iter().filter(|r| r.name == "not-run").map(|r| r.detail).collect();
        assert_eq!(
            rows,
            vec![
                "portal \u{2014} declared, not yet built at this site: w1 (gate row I1, I4)".to_string(),
                "spire \u{2014} declared, not yet built at this site: w2 (ADR 1164)".to_string(),
            ]
        );
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
            "goatnet_founder: false\njoin_founder_site: zagros\nsite_enrollment_dns: netbird.example.net\nsite_dns_zone: zagros.example.org\nsignal_federation_absent_reason: not yet\n",
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
        std::fs::write(gv.join("zagros_b_hosts/vars.yml"), "goatnet_founder: false\njoin_founder_site: zagros\nsite_enrollment_dns: netbird.example.net\nsite_dns_zone: zagros.example.org\n").unwrap();
        // The site's leaf-cert manifest: the router and one service leaf.
        let m = t.path().join("ops/enrollment/manifests");
        std::fs::create_dir_all(&m).unwrap();
        std::fs::write(m.join("zagros-b-certs.yaml"), "site_id: zagros-b\nout_dir: /s/zagros-b-leaves\nleaves:\n  - name: router\n    cn: goat-zenoh-router\n  - name: fabric-health\n    cn: fabric-health-trial-peer\n  - name: portal\n    cn: portal\n").unwrap();
        // The goatnet's mesh zone, as generate-mesh-hosts.py writes it.
        std::fs::create_dir_all(t.path().join("ops/dns/zagros")).unwrap();
        std::fs::write(t.path().join("ops/dns/zagros/mesh-hosts"), "# zagros mesh DNS\n100.127.0.1 isitup.zagros.zagros.example.org\n").unwrap();
        t
    }

    /// Gate row I3, a joining site's secrets: phase 3 of a joiner's core node
    /// runs join-site-secrets.py with the site and its founder, after the
    /// read-only checks and before phase 4 changes the node. An obs node and a
    /// founder site run no such step.
    #[test]
    fn a_joiners_core_node_takes_its_shared_secrets_from_the_founder_in_phase_3() {
        let t = joiner_checkout();
        let ph = plan(t.path(), &opts("zagros-b", "core", "zagros")).unwrap();
        let at = ph.iter().position(|p| p.cmd.contains("join-site-secrets.py")).expect("a secrets step");
        assert_eq!((ph[at].n, ph[at].kind), (3, "T"));
        assert!(ph[at].cmd.ends_with("/ops/scripts/join-site-secrets.py' zagros-b --founder zagros"), "{}", ph[at].cmd);
        assert!(ph[at].label.contains("taken from zagros's") && ph[at].label.contains("GB-353"), "{}", ph[at].label);
        assert!(ph[..at].iter().all(|p| p.n <= 3) && ph[at + 1].n == 4, "the last step of phase 3");
        assert!(plan(t.path(), &opts("zagros-b", "obs", "zagros")).unwrap().iter().all(|p| !p.cmd.contains("join-site-secrets.py")));
        std::fs::write(t.path().join("ops/ansible/group_vars/zagros_b_hosts/vars.yml"), "goatnet_founder: true\nsite_enrollment_dns: netbird.example.net\nsite_dns_zone: zagros.example.org\n").unwrap();
        assert!(plan(t.path(), &opts("zagros-b", "core", "zagros")).unwrap().iter().all(|p| !p.cmd.contains("join-site-secrets.py")), "a founder's come from standup");
    }

    /// `goatnet_founder` is read as site-bootstrap reads it
    /// (`goatnet_founder | default(true) | bool`): only the top-level key, and
    /// Ansible's booleans, so `False`, `"false"`, `no` and `off` declare a
    /// joiner, and a nested or commented-out key, or none, a founder.
    #[test]
    fn goatnet_founder_is_read_as_ansible_reads_it() {
        for (line, joiner) in [
            ("goatnet_founder: false", true),
            ("goatnet_founder: False", true),
            ("goatnet_founder: \"false\"", true),
            ("goatnet_founder: no", true),
            ("goatnet_founder: 'no'", true),
            ("goatnet_founder: off", true),
            ("goatnet_founder: true", false),
            ("goatnet_founder: yes", false),
            ("goatnet_founder: \"True\"", false),
            ("", false),
            ("site:\n  goatnet_founder: false", false),
            ("# goatnet_founder: false", false),
        ] {
            let t = joiner_checkout();
            let vars = t.path().join("ops/ansible/group_vars/zagros_b_hosts/vars.yml");
            let text = std::fs::read_to_string(&vars).unwrap().replacen("goatnet_founder: false\n", &format!("{line}\n"), 1);
            std::fs::write(&vars, text).unwrap();
            let ph = plan(t.path(), &opts("zagros-b", "core", "zagros")).unwrap_or_else(|e| panic!("{line:?}: {e:#}"));
            let is_joiner = ph.iter().any(|p| p.label.contains("[core, joiner]"));
            assert_eq!(is_joiner, joiner, "{line:?}");
        }
    }

    /// Gate row I3: a joiner's core node runs its own mesh DNS replica in
    /// phase 13, after Ory and signal federation (ADR 1235). A gate first
    /// refuses an unset or unresolved founder API token (GB-281); the play
    /// then serves the goatnet's zone on the node, registers the site's own
    /// nameserver group through the founder's management, and lists the
    /// node's replica first and the founder's second. A site with no zone to
    /// serve is refused when the plan is made (GB-280). Other classes run no
    /// mesh DNS.
    #[test]
    fn a_joiner_core_node_runs_its_mesh_dns_replica_after_ory() {
        let t = joiner_checkout();
        let ph = plan(t.path(), &opts("zagros-b", "core", "zagros")).unwrap();
        let at = |needle: &str| ph.iter().position(|p| p.cmd.contains(needle)).unwrap_or_else(|| panic!("no phase runs {needle}"));
        let (ory, fed, gate, dns) = (at("ory-deploy.yml"), at("signal-federation-check.py"), at("GB-281"), at("goat-coredns.yml"));
        assert!(ory < fed && fed < gate && gate + 1 == dns, "{ory} {fed} {gate} {dns}");
        assert_eq!((ph[gate].n, ph[gate].kind, ph[dns].n, ph[dns].kind), (13, "GATE", 13, "A"));
        assert!(ph[gate].cmd.contains("with-secrets") && ph[gate].cmd.contains("ZAGROS_NB_API_TOKEN"), "{}", ph[gate].cmd);
        let c = &ph[dns].cmd;
        for want in [" -l node-x ", "-e mesh_dns_zone_dir_name=zagros ", "-e netbird_nameserver_group_name=zagros-b-mesh-dns ", "-e netbird_api=https://zagros.example.org/api ", "-e netbird_api_token_envvar=ZAGROS_NB_API_TOKEN ", "-e mesh_dns_peer_replica_group=zagros_hosts"] {
            assert!(c.contains(want), "{want:?} in {c}");
        }
        // After mesh DNS, phase 13 holds only the single-host obs step.
        assert!(ph[dns + 1..].iter().all(|p| p.n > 13 || p.label.starts_with("[obs, single-host]")), "mesh DNS ends phase 13 but for the obs step");
        // Other classes run no mesh DNS.
        assert!(plan(t.path(), &opts("zagros-b", "obs", "zagros")).unwrap().iter().all(|p| !p.cmd.contains("goat-coredns.yml")));
        // No zone to serve: refused when the plan is made.
        std::fs::remove_file(t.path().join("ops/dns/zagros/mesh-hosts")).unwrap();
        let e = plan(t.path(), &opts("zagros-b", "core", "zagros")).err().expect("refused");
        assert_eq!(e.downcast_ref::<goat_errors::Coded>().map(|c| c.code.as_str()), Some("GB-280"), "{e}");
        assert!(e.to_string().contains("zagros.example.org"), "{e}");
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
        // The bootstrap passes and Ory's staging; the seed checks between them are tested on their own.
        let core: Vec<&Phase> = ph.iter().filter(|p| p.label.contains("[core, joiner]") && !p.cmd.contains("spock-seed-current-check.yml") && !p.cmd.contains("spock-join-replication-declared.yml")).collect();
        assert_eq!(core.len(), 6, "{:?}", core.iter().map(|p| &p.label).collect::<Vec<_>>());
        for i in [0, 3] {
            assert!(core[i].cmd.contains("-l node-x site-bootstrap-for.yml -e bootstrap_site=zagros-b "), "{}", core[i].cmd);
            assert!(!core[i].cmd.contains("-e site_id="), "{}", core[i].cmd);
            assert!(core[i].cmd.contains("_PG_PASS\")"), "secrets still reach the play: {}", core[i].cmd);
            // A joiner mints no account: no bootstrap owner, so no OWNER_EMAIL to set.
            assert!(!core[i].cmd.contains("bootstrap_owner_email") && !core[i].cmd.contains("OWNER_EMAIL"), "{}", core[i].cmd);
        }
        assert!(core[0].cmd.contains("-e participates_in_spock=false"), "{}", core[0].cmd);
        assert!(core[3].cmd.contains("-e ory_spock_wire_subscriptions=false"), "{}", core[3].cmd);
        assert!(core[4].cmd.contains("spock-mesh-wire.yml --limit zagros_hosts") && core[4].cmd.contains("\"inter_site_spock_peers\":[\"zagros-b\"]") && core[4].cmd.contains("\"ory_spock_reconcile_history\":false"), "{}", core[4].cmd);
        assert!(core[5].cmd.ends_with("spock-mesh-wire.yml --limit node-x"), "{}", core[5].cmd);
        assert!(!core[4].cmd.contains("-e site_id") && !core[5].cmd.contains("-e site_id"));

        // A site that keeps its own wrapper gets it.
        std::fs::write(t.path().join("ops/ansible/zagros-b-bootstrap.yml"), "---\n").unwrap();
        let ph = plan(t.path(), &o).unwrap();
        let core: Vec<&Phase> = ph.iter().filter(|p| p.label.contains("[core, joiner]") && p.kind != "GATE").collect();
        assert!(core[0].cmd.contains("-l node-x zagros-b-bootstrap.yml -e 'postgres_superuser_password="), "{}", core[0].cmd);
    }

    /// Phase 15 is standup's acceptance line: the end-state model into the
    /// state directory, the values, then the rows with `--endstate`.
    #[test]
    fn phase_15_derives_the_end_state_and_passes_it_to_the_rows() {
        let t = joiner_checkout();
        let ph = plan(t.path(), &opts("zagros-b", "core", "zagros")).unwrap();
        let last = ph.last().unwrap();
        assert_eq!(last.n, 15);
        let model = format!("{}/endstate-zagros-b.json", crate::settings::state_dir_abs(t.path()));
        assert!(last.cmd.contains(&format!("endstate.py' model zagros-b > '{model}' && ")), "{}", last.cmd);
        assert!(last.cmd.contains(&format!("--endstate '{model}' --all-eligible --by-layer")), "{}", last.cmd);
    }

    /// Pass 1 runs site-bootstrap's `stack` plays only: the `after-ory` plays
    /// end on the management service, which waits for Hydra, and Ory is the
    /// next step. Pass 3, after Ory, runs the whole file, as standup's
    /// joiner does.
    #[test]
    fn pass_1_runs_the_stack_plays_only_and_pass_3_the_whole_file() {
        let t = joiner_checkout();
        let ph = plan(t.path(), &opts("zagros-b", "core", "zagros")).unwrap();
        let core: Vec<&Phase> = ph.iter().filter(|p| p.label.contains("[core, joiner]") && !p.cmd.contains("spock-join-replication-declared.yml")).collect();
        assert!(core[0].label.contains("pass 1") && core[0].cmd.ends_with(" --skip-tags after-ory"), "{}", core[0].cmd);
        assert!(core[1].cmd.contains("ory-deploy.yml"), "Ory follows pass 1: {}", core[1].cmd);
        assert!(core[4].label.contains("pass 3") && !core[4].cmd.contains("--skip-tags"), "{}", core[4].cmd);
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
        // Each present publisher's leaf is in the manifest (GB-364 otherwise).
        std::fs::write(t.path().join("ops/enrollment/manifests/zagros-b-certs.yaml"), "site_id: zagros-b\nout_dir: /s/zagros-b-leaves\nleaves:\n  - name: router\n    cn: goat-zenoh-router\n  - name: fabric-health\n    cn: fabric-health-trial-peer\n  - name: fabric-crawler\n    cn: fabric-crawler\n").unwrap();
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

    /// A health publisher join-node deploys runs with its own leaf, which
    /// phase 13 mints from the site's leaf manifest and stages onto the node.
    /// A publisher the model marks present whose leaf the manifest does not
    /// carry is refused when the plan is made (GB-364), naming the publisher
    /// and the manifest, not at staging or at deploy. A publisher marked absent
    /// needs no leaf.
    #[test]
    fn a_publisher_it_deploys_with_no_leaf_in_the_manifest_is_refused_at_plan_time() {
        let t = joiner_checkout();
        for play in ["fabric-health-deploy.yml", "mesh-health-deploy.yml"] {
            std::fs::write(t.path().join("ops/ansible").join(play), "---\n").unwrap();
        }
        // mesh-health present, and the manifest carries only fabric-health's leaf.
        fake_model(t.path(), r#"{"fabric-health": {"present": true}, "mesh-health": {"present": true}}"#);
        let e = plan(t.path(), &opts("zagros-b", "core", "zagros")).err().expect("refused");
        assert_eq!(e.downcast_ref::<goat_errors::Coded>().map(|c| c.code.as_str()), Some("GB-364"), "{e}");
        let text = e.to_string();
        assert!(text.contains("mesh-health") && !text.contains("fabric-health,"), "names the publisher with no leaf: {text}");
        assert!(text.contains("ops/enrollment/manifests/zagros-b-certs.yaml"), "names the manifest: {text}");
        // Marked absent: no leaf needed.
        fake_model(t.path(), r#"{"fabric-health": {"present": true}, "mesh-health": {"present": false}}"#);
        assert!(plan(t.path(), &opts("zagros-b", "core", "zagros")).is_ok());
        // The manifest gains the leaf: the plan is made, and stages it.
        fake_model(t.path(), r#"{"fabric-health": {"present": true}, "mesh-health": {"present": true}}"#);
        std::fs::write(t.path().join("ops/enrollment/manifests/zagros-b-certs.yaml"), "site_id: zagros-b\nout_dir: /s/zagros-b-leaves\nleaves:\n  - name: router\n    cn: goat-zenoh-router\n  - name: fabric-health\n    cn: fabric-health-trial-peer\n  - name: mesh-health\n    cn: mesh-health\n").unwrap();
        let ph = plan(t.path(), &opts("zagros-b", "core", "zagros")).unwrap();
        let stage = ph.iter().find(|p| p.cmd.contains("stage-service-leaves.yml")).expect("a staging step");
        assert!(stage.cmd.ends_with(r#"-e '{"leaf_only": ["fabric-health", "mesh-health"]}'"#), "{}", stage.cmd);
    }

    /// Row 9: the seed carries the founder's data, and the subscriptions
    /// from the founder carry only later writes (synchronize_data false). A
    /// founder write after the seed and before the subscriptions exist is never
    /// sent, so a joiner's core node checks its seed tables against the
    /// founder's twice: before pass 3 wires Spock, and after the joiner's Spock
    /// wiring, once the new subscriptions have caught up. Each check is a
    /// phase-13 GATE, limited to the node. A founder site seeds nothing, so its
    /// plan has neither.
    #[test]
    fn a_joiners_seed_is_checked_current_before_and_after_spock_is_wired() {
        let t = joiner_checkout();
        let ph = plan(t.path(), &opts("zagros-b", "core", "zagros")).unwrap();
        let at = |needle: &str| ph.iter().position(|p| p.cmd.contains(needle)).unwrap_or_else(|| panic!("no phase runs {needle}"));
        let (seed, before, pass3, again, after) = (
            at("site-seed-from-peer.yml"),
            at("spock-seed-current-check.yml -e site_id=zagros-b -e seed_check_phase=before"),
            ph.iter().position(|p| p.label.contains("pass 3")).expect("pass 3"),
            ph.iter().position(|p| p.label.contains("Spock wiring again")).expect("the wiring again"),
            at("spock-seed-current-check.yml -e site_id=zagros-b -e seed_check_phase=after"),
        );
        assert!(seed < before && before + 1 == pass3 && again + 1 == after, "{seed} {before} {pass3} {again} {after}");
        for i in [before, after] {
            assert_eq!((ph[i].n, ph[i].kind), (13, "GATE"), "{}", ph[i].label);
            assert!(ph[i].cmd.contains(" -l node-x "), "{}", ph[i].cmd);
            assert!(ph[i].label.contains("GB-367"), "{}", ph[i].label);
        }
        // A founder seeds nothing.
        std::fs::write(t.path().join("ops/ansible/group_vars/zagros_b_hosts/vars.yml"), "goatnet_founder: true\nsite_enrollment_dns: netbird.example.net\nsite_dns_zone: zagros.example.org\n").unwrap();
        assert!(plan(t.path(), &opts("zagros-b", "core", "zagros")).unwrap().iter().all(|p| !p.cmd.contains("spock-seed-current-check.yml")));
    }

    /// Phase 13's wiring runs its Spock roles only where a host's own
    /// variables declare replication, so before it changes anything (before
    /// pass 1, after the check of a site's copy of the seed play) the node and
    /// its founder are asked (GB-417), read-only, with the node and the
    /// founder's group named as the play's own variables. A founder site wires nothing and is not asked.
    #[test]
    fn a_joiner_and_its_founder_are_asked_to_declare_replication_before_phase_13_changes_anything() {
        let t = joiner_checkout();
        let ph = plan(t.path(), &opts("zagros-b", "core", "zagros")).unwrap();
        let gate = ph.iter().position(|p| p.cmd.contains("spock-join-replication-declared.yml")).expect("a check that replication is declared");
        let pass1 = ph.iter().position(|p| p.label.contains("pass 1")).expect("pass 1");
        assert_eq!(gate, ph.iter().position(|p| p.n == 13).unwrap(), "phase 13's first step");
        assert_eq!(gate + 1, pass1, "right before pass 1");
        assert_eq!((ph[gate].n, ph[gate].kind), (13, "GATE"));
        assert!(ph[gate].cmd.ends_with("spock-join-replication-declared.yml -e join_check_node=node-x -e join_check_site=zagros-b -e join_check_founder_group=zagros_hosts"), "{}", ph[gate].cmd);
        assert!(!ph[gate].cmd.contains("-e site_id"), "{}", ph[gate].cmd);
        assert!(ph[gate].label.contains("GB-417"), "{}", ph[gate].label);
        // After the check of a site's own copy of the seed play.
        std::fs::write(t.path().join("ops/ansible/zagros-b-seed-from-peer.yml"), "---\n").unwrap();
        let ph = plan(t.path(), &opts("zagros-b", "core", "zagros")).unwrap();
        let copy = ph.iter().position(|p| p.cmd.contains("check-site-play-current.py")).unwrap();
        assert_eq!(ph.iter().position(|p| p.cmd.contains("spock-join-replication-declared.yml")), Some(copy + 1));
        // A founder wires nothing.
        std::fs::write(t.path().join("ops/ansible/group_vars/zagros_b_hosts/vars.yml"), "goatnet_founder: true\nsite_enrollment_dns: netbird.example.net\nsite_dns_zone: zagros.example.org\n").unwrap();
        assert!(plan(t.path(), &opts("zagros-b", "core", "zagros")).unwrap().iter().all(|p| !p.cmd.contains("spock-join-replication-declared.yml")));
    }

    /// The same gate holds both sides to the management tables' replication
    /// (ADR 1084 D1): spock_mesh_databases pulling netbird_mgmt, whose repset is
    /// the seed's own table list. The check reads that list from the file the
    /// seed play loads it from, so the two cannot name different tables, and
    /// the plan's line says what it asks.
    #[test]
    fn the_replication_gate_asks_for_the_management_tables_the_seed_compares() {
        let t = joiner_checkout();
        let ph = plan(t.path(), &opts("zagros-b", "core", "zagros")).unwrap();
        let gate = ph.iter().find(|p| p.cmd.contains("spock-join-replication-declared.yml")).expect("the gate");
        assert!(gate.label.contains("netbird_mgmt"), "{}", gate.label);
        let defaults = "roles/spock-mesh-peering/defaults/main.yml";
        let seed: Vec<serde_yaml::Value> = serde_yaml::from_str(&std::fs::read_to_string(repo().join("ops/ansible/site-seed-from-peer.yml")).unwrap()).unwrap();
        assert!(seed[0]["vars_files"].as_sequence().unwrap().iter().any(|f| f.as_str() == Some(defaults)), "the seed reads its tables from {defaults}");
        let check: Vec<serde_yaml::Value> = serde_yaml::from_str(&std::fs::read_to_string(repo().join("ops/ansible/spock-join-replication-declared.yml")).unwrap()).unwrap();
        let tables = check[0]["vars"]["spock_mesh_netbird_mgmt_tables"].as_str().expect("the check names the seed's table list");
        assert!(tables.contains(&format!("'/{defaults}'")) && tables.ends_with(".spock_mesh_netbird_mgmt_tables }}"), "{tables}");
        // Only that variable: the whole file as play vars would set spock_mesh_databases above the hosts' own.
        assert!(check[0].get("vars_files").is_none());
    }

    /// A joining site that keeps its own copy of the seed play
    /// (`<site>-seed-from-peer.yml`) has it checked against the generic play
    /// it was derived from (GB-362, ops/scripts/check-site-play-current.py),
    /// as a phase-13 GATE before phase 13 changes anything. A site that keeps
    /// no copy gets no such step, so its plan is unchanged.
    #[test]
    fn a_joiners_copy_of_the_seed_play_is_checked_before_phase_13_changes_anything() {
        let t = joiner_checkout();
        let none = plan(t.path(), &opts("zagros-b", "core", "zagros")).unwrap();
        assert!(!none.iter().any(|p| p.cmd.contains("check-site-play-current.py")), "no copy, no check");
        std::fs::write(t.path().join("ops/ansible/zagros-b-seed-from-peer.yml"), "---\n").unwrap();
        let ph = plan(t.path(), &opts("zagros-b", "core", "zagros")).unwrap();
        let gate = ph.iter().position(|p| p.cmd.contains("check-site-play-current.py")).expect("a check of the site's copy");
        let first13 = ph.iter().position(|p| p.n == 13).unwrap();
        assert_eq!(gate, first13, "the check is phase 13's first step");
        assert_eq!((ph[gate].n, ph[gate].kind), (13, "GATE"));
        assert!(ph[gate].cmd.ends_with(&format!("check-site-play-current.py' --repo '{}' zagros-b seed-from-peer.yml", t.path().display())), "{}", ph[gate].cmd);
        assert!(ph[gate].label.contains("GB-362"), "{}", ph[gate].label);
        assert!(ph.iter().any(|p| p.cmd.contains(" zagros-b-seed-from-peer.yml")), "the seed runs the site's copy");
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

    /// The not-run rows a joiner's plan prints for `node`, the heading left out.
    fn not_run_rows(repo: &Path, o: &Options) -> Vec<String> {
        let ph = plan(repo, o).unwrap();
        not_run(repo, o, Some(&ph)).into_iter().filter(|r| r.name == "not-run").map(|r| r.detail).collect()
    }

    /// Gate row I3, the declared set: every service the end-state model marks
    /// present on the node is either built by a step of the plan or named under
    /// the not-run heading. A catalog gap keeps its own line with its gate rows
    /// (the admin surfaces: I1, I4). A present service no step builds names gate
    /// row I3: on a single-host joiner (no `has_obs_host: true`) whose
    /// group_vars declare no `obs_colocated_site_zones`, that is also the obs
    /// stack the model places on the core node, whose step is skipped. A
    /// service the plan builds, one on another node, and one the site does not
    /// declare are not named.
    #[test]
    fn a_joiner_names_each_declared_service_its_plan_does_not_build() {
        let t = joiner_checkout();
        std::fs::write(t.path().join("ops/ansible/fabric-health-deploy.yml"), "---\n").unwrap();
        fake_model(
            t.path(),
            r#"{
                "zenoh-router": {"present": true, "class": "mgmt", "hosts": ["node-x"]},
                "mesh-coredns": {"present": true, "class": "mgmt", "hosts": ["node-x"]},
                "ory": {"present": true, "class": "mgmt", "hosts": ["node-x"]},
                "netbird-mgmt": {"present": true, "class": "mgmt", "hosts": ["node-x"]},
                "traefik": {"present": true, "class": "mgmt", "hosts": ["node-x"]},
                "fabric-health": {"present": true, "class": "mgmt", "hosts": ["node-x"]},
                "portal": {"present": true, "class": "mgmt", "hosts": ["node-x"], "gap": {"row": "I1, I4", "why": "an admin surface waits"}},
                "peer-health": {"present": true, "class": "mgmt", "hosts": ["node-x"]},
                "prometheus": {"present": true, "class": "obs", "hosts": ["node-x"]},
                "goat-docs": {"present": true, "class": "mgmt", "hosts": ["node-y"]},
                "isitup": {"present": false, "class": "mgmt", "hosts": ["node-x"], "reason": "`isitup_enabled` is false in group_vars"},
                "spire": {"present": null, "class": "mgmt", "hosts": ["node-x"]}
            }"#,
        );
        let rows = not_run_rows(t.path(), &opts("zagros-b", "core", "zagros"));
        assert_eq!(
            rows,
            vec![
                "portal \u{2014} declared, not yet built at this site: an admin surface waits (gate row I1, I4)".to_string(),
                "isitup \u{2014} `isitup_enabled` is false in group_vars".to_string(),
                "peer-health \u{2014} declared at node-x, and no join-node step builds it yet (gate row I3)".to_string(),
                "prometheus \u{2014} declared at node-x, and no join-node step builds it yet (gate row I3)".to_string(),
            ]
        );
    }

    /// A site with its own obs host (`has_obs_host: true`): a service the
    /// node's other class builds names that run (`join-node --class obs` from
    /// a core run, `--class core` from an obs run); one neither builds names
    /// gate row I3. Without `has_obs_host: true` the other class is never
    /// named, since its run would re-enrol a node already built.
    #[test]
    fn a_site_with_an_obs_host_names_the_other_class_run() {
        let t = joiner_checkout();
        let gv = t.path().join("ops/ansible/group_vars/zagros_b_hosts/vars.yml");
        let text = std::fs::read_to_string(&gv).unwrap();
        std::fs::write(&gv, format!("{text}has_obs_host: true\n")).unwrap();
        fake_model(
            t.path(),
            r#"{
                "zenoh-router": {"present": true, "class": "mgmt", "hosts": ["node-x"]},
                "peer-health": {"present": true, "class": "mgmt", "hosts": ["node-x"]},
                "prometheus": {"present": true, "class": "obs", "hosts": ["node-x"]}
            }"#,
        );
        let core = not_run_rows(t.path(), &opts("zagros-b", "core", "zagros"));
        assert_eq!(
            core,
            vec![
                "peer-health \u{2014} declared at node-x, and no join-node step builds it yet (gate row I3)".to_string(),
                "prometheus \u{2014} declared at node-x, built by `join-node --class obs`, not by this run".to_string(),
            ]
        );
        let obs = not_run_rows(t.path(), &opts("zagros-b", "obs", "zagros"));
        assert!(!obs.iter().any(|d| d.starts_with("prometheus ")), "{obs:#?}");
        assert!(obs.iter().any(|d| d == "zenoh-router \u{2014} declared at node-x, built by `join-node --class core`, not by this run"), "{obs:#?}");
        assert!(obs.iter().any(|d| d == "peer-health \u{2014} declared at node-x, and no join-node step builds it yet (gate row I3)"), "{obs:#?}");
    }

    /// The site's bootstrap play builds admit-audit (site-bootstrap.yml,
    /// role admit-audit), so a joiner that declares it is not told it is
    /// unbuilt.
    #[test]
    fn the_bootstrap_play_builds_admit_audit() {
        let t = joiner_checkout();
        fake_model(t.path(), r#"{"admit-audit": {"present": true, "class": "mgmt", "hosts": ["node-x"]}}"#);
        assert_eq!(not_run_rows(t.path(), &opts("zagros-b", "core", "zagros")), vec!["nothing: the model names no gap here, and the site turns nothing off".to_string()]);
    }

    /// Gate row I3, a single-host joiner's obs stack: with no `has_obs_host:
    /// true`, the core run's last phase-13 step runs `obs.yml` on the node with
    /// Grafana off, once `obs_colocated_site_zones` is declared. Prometheus,
    /// Alertmanager and the exporters then count as built; Grafana, whose
    /// OAuth2 client is an identity-store write at a second site, is named
    /// with gate row I3. Without the declaration the step is a SKIP that names
    /// it, and the four are named. A site with its own obs host gets no such
    /// step.
    #[test]
    fn a_single_host_joiner_runs_its_obs_stack_with_grafana_off() {
        let t = joiner_checkout();
        let gv = t.path().join("ops/ansible/group_vars/zagros_b_hosts/vars.yml");
        let text = std::fs::read_to_string(&gv).unwrap();
        fake_model(
            t.path(),
            r#"{
                "prometheus": {"present": true, "class": "obs", "hosts": ["node-x"]},
                "alertmanager": {"present": true, "class": "obs", "hosts": ["node-x"]},
                "grafana": {"present": true, "class": "obs", "hosts": ["node-x"]},
                "obs-exporters": {"present": true, "class": "obs", "hosts": ["node-x"]}
            }"#,
        );
        let o = opts("zagros-b", "core", "zagros");
        let i3 = |svc: &str| format!("{svc} \u{2014} declared at node-x, and no join-node step builds it yet (gate row I3)");

        // Not declared: a SKIP that names the declaration, and all four named.
        let ph = plan(t.path(), &o).unwrap();
        let skip = ph.iter().find(|p| p.label.starts_with("[obs, single-host]")).unwrap();
        assert_eq!(skip.kind, "SKIP");
        assert!(skip.label.contains("obs_colocated_site_zones") && skip.label.contains("--from 13"), "{}", skip.label);
        assert!(!ph.iter().any(|p| p.kind == "A" && p.cmd.contains(" obs.yml ")), "no obs.yml step without the declaration");
        assert_eq!(not_run_rows(t.path(), &o), ["prometheus", "alertmanager", "grafana", "obs-exporters"].map(i3).to_vec());

        // Declared: the step runs obs.yml on the node, Grafana off, last in phase 13.
        std::fs::write(&gv, format!("{text}obs_colocated_site_zones:\n  zagros-b: zagros.example.org\n")).unwrap();
        let ph = plan(t.path(), &o).unwrap();
        let last13 = ph.iter().rfind(|p| p.n == 13).unwrap();
        assert_eq!(last13.kind, "A");
        assert!(last13.label.starts_with("[obs, single-host]") && last13.label.contains("ZAGROS_B_POSTGRES_EXPORTER_PASS"), "{}", last13.label);
        assert!(last13.cmd.contains("obs.yml -e target=zagros_b_hosts -l node-x -e grafana_enabled=false"), "{}", last13.cmd);
        assert!(last13.cmd.contains("lookup(\"env\", \"ZAGROS_B_PG_PASS\")"), "{}", last13.cmd);
        assert_eq!(not_run_rows(t.path(), &o), vec![i3("grafana")]);

        // A site with its own obs host: no obs.yml step on the core node.
        std::fs::write(&gv, format!("{text}has_obs_host: true\nobs_colocated_site_zones:\n  zagros-b: zagros.example.org\n")).unwrap();
        let ph = plan(t.path(), &o).unwrap();
        assert!(!ph.iter().any(|p| p.label.starts_with("[obs, single-host]")), "a site with an obs host gets no single-host obs step");
    }

    /// A model that does not place the node on any host (an inventory that does
    /// not list it yet) is read by class: a core run is answerable for the
    /// mgmt-class services, an obs run for the obs-class ones.
    #[test]
    fn a_node_the_model_does_not_place_is_read_by_its_class() {
        let t = joiner_checkout();
        fake_model(t.path(), r#"{"zenoh-router": {"present": true, "class": "mgmt", "hosts": []}, "peer-health": {"present": true, "class": "mgmt"}, "grafana": {"present": true, "class": "obs"}}"#);
        let core = not_run_rows(t.path(), &opts("zagros-b", "core", "zagros"));
        assert_eq!(core, vec!["peer-health \u{2014} declared at node-x, and no join-node step builds it yet (gate row I3)".to_string()]);
        let obs = not_run_rows(t.path(), &opts("zagros-b", "obs", "zagros"));
        assert!(!obs.iter().any(|d| d.starts_with("grafana ") || d.starts_with("peer-health ")), "{obs:#?}");
    }

    /// A founder site's node is standup's to build: join-node names no
    /// declared service of its own there, only the model's gaps and what the
    /// site turns off.
    #[test]
    fn a_founder_sites_node_names_no_unbuilt_service() {
        let t = joiner_checkout();
        std::fs::write(t.path().join("ops/ansible/group_vars/zagros_b_hosts/vars.yml"), "goatnet_founder: true\nsite_enrollment_dns: netbird.example.net\n").unwrap();
        fake_model(t.path(), r#"{"peer-health": {"present": true, "class": "mgmt", "hosts": ["node-x"]}}"#);
        assert_eq!(not_run_rows(t.path(), &opts("zagros-b", "core", "zagros")), vec!["nothing: the model names no gap here, and the site turns nothing off".to_string()]);
    }

    /// Each service a plan step is held to build is an end-state catalog
    /// service, and each play named is in this repository.
    #[test]
    fn each_built_service_is_a_catalog_service_with_its_play() {
        let text = std::fs::read_to_string(repo().join("ops/goatbuilder/endstate/catalog.yaml")).unwrap();
        let catalog: serde_yaml::Value = serde_yaml::from_str(&text).unwrap();
        for (play, services) in BUILDERS.iter().chain([("site-bootstrap.yml", &BOOTSTRAP_BUILDS[..])].iter()) {
            assert!(repo().join("ops/ansible").join(play).is_file(), "no ops/ansible/{play}");
            for s in *services {
                assert!(catalog["services"].get(*s).is_some(), "{s} is not an end-state catalog service");
            }
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
        assert!(router[0].cmd.ends_with("zenoh-deploy.yml -e target=zagros_b_hosts -e zenoh_slot_fragment_bootstrap_if_unassigned=true -l node-x"), "the first-render check, as standup phase 9 asks for it: {}", router[0].cmd);
        assert!(!router[0].cmd.contains("zenoh_slot_fragment_bootstrap="), "never the role's switch itself: {}", router[0].cmd);
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
        assert!(ph[rewire + 1].cmd.contains("seed_check_phase=after"), "the seed check follows the Spock wiring: {}", ph[rewire + 1].label);
        assert_eq!(first, rewire + 2, "federation follows the Spock wiring and its seed check directly");
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

    /// Phase 14 leaves the node taking its updates again: it unmasks the apt
    /// timers phase 4 masked and starts them, which unmasking alone does not do
    /// until the next boot. Over the mesh and through a door alike.
    #[test]
    fn phase_14_unmasks_the_apt_timers_and_starts_them() {
        let t = joiner_checkout();
        for via in ["mesh", "door"] {
            let o = Options { via, ..opts("zagros-b", "core", "zagros") };
            let ph = plan(t.path(), &o).unwrap();
            let p4 = ph.iter().find(|p| p.n == 4).unwrap();
            assert!(p4.cmd.contains("systemctl mask --now apt-daily.timer apt-daily-upgrade.timer"), "{via}: {}", p4.cmd);
            let p14 = ph.iter().find(|p| p.n == 14).unwrap();
            assert!(
                p14.cmd.contains("'sudo systemctl unmask apt-daily.timer apt-daily-upgrade.timer && sudo systemctl enable --now apt-daily.timer apt-daily-upgrade.timer'"),
                "{via}: {}",
                p14.cmd
            );
            assert!(p14.label.starts_with("unmask and start the apt timers"), "{via}: {}", p14.label);
        }
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

    /// `chronyc -c tracking` of a node following 192.0.2.123 (or `refid`),
    /// with the given system time offset in seconds and leap status.
    fn chrony_csv(refid: &str, offset: &str, leap: &str) -> String {
        format!("{refid},192.0.2.123,3,1700000000.0,{offset},0.0,0.0,0.0,0.0,0.0,0.010,0.002,64.0,{leap}")
    }

    /// Phase 3's clock step, run as join-node runs it, on a checkout that
    /// carries the real script and the federation preflight's clock code. ssh
    /// runs the remote line here; chronyc and timedatectl print `chrony` and
    /// `timedatectl`, or answer nothing (exit 1) when they are empty.
    fn run_clock_step(chrony: &str, timedatectl: &str, budget: Option<&str>) -> (i32, String) {
        let t = joiner_checkout();
        for f in ["ops/scripts/join-node-clock-check.py", "ops/federation/standup/preflight_live.py"] {
            std::fs::create_dir_all(t.path().join(f).parent().unwrap()).unwrap();
            std::fs::copy(repo().join(f), t.path().join(f)).unwrap();
        }
        std::fs::create_dir_all(t.path().join("ops/goatbuilder")).unwrap();
        std::fs::write(t.path().join("ops/goatbuilder/goatbuilder.yaml"), "deploy:\n  ssh_user: operator\n").unwrap();
        let bin = t.path().join("bin");
        crate::test_support::executable(&bin.join("ssh"), "#!/bin/sh\nfor a; do last=\"$a\"; done\nexec sh -c \"$last\"\n");
        crate::test_support::executable(&bin.join("chronyc"), "#!/bin/sh\n[ -n \"$FAKE_CHRONY\" ] || { echo '506 Cannot talk to daemon' >&2; exit 1; }\nprintf '%s\\n' \"$FAKE_CHRONY\"\n");
        crate::test_support::executable(&bin.join("timedatectl"), "#!/bin/sh\n[ -n \"$FAKE_TIMEDATECTL\" ] || exit 1\nprintf '%s\\n' \"$FAKE_TIMEDATECTL\"\n");
        let o = Options { address: Some("192.0.2.9"), ..opts("zagros-b", "core", "zagros") };
        let step = plan(t.path(), &o).unwrap().into_iter().find(|p| p.cmd.contains("join-node-clock-check.py")).expect("a clock step");
        assert_eq!((step.n, step.kind), (3, "GATE"));
        assert!(step.cmd.ends_with("join-node-clock-check.py' node-x operator@192.0.2.9"), "{}", step.cmd);
        let mut c = Command::new("bash");
        c.args(["-euo", "pipefail", "-c", &step.cmd])
            .current_dir(t.path())
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .env("FAKE_CHRONY", chrony)
            .env("FAKE_TIMEDATECTL", timedatectl)
            .env_remove("JOIN_NODE_CLOCK_BUDGET_MS");
        if let Some(b) = budget {
            c.env("JOIN_NODE_CLOCK_BUDGET_MS", b);
        }
        let out = c.output().unwrap();
        (out.status.code().unwrap_or(-1), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
    }

    /// ADR 1232: before phase 4 changes anything on the node, phase 3 reads
    /// its clock. A node synchronized to an outside reference and within the
    /// budget passes, and the line prints its offset; the budget can be
    /// raised, never past the router's 500 ms limit.
    #[test]
    fn a_synchronized_clock_within_budget_passes_and_prints_its_offset() {
        let (rc, out) = run_clock_step(&chrony_csv("C0000201", "-0.012345678", "Normal"), "", None);
        assert_eq!(rc, 0, "{out}");
        assert!(out.contains(">> clock: node-x offset 12.3 ms (chrony, stratum 3, reference 192.0.2.123); budget 250 ms of the 500 ms router limit"), "{out}");
        let (rc, out) = run_clock_step(&chrony_csv("C0000201", "0.312300000", "Normal"), "", Some("400"));
        assert_eq!(rc, 0, "{out}");
        assert!(out.contains("offset 312.3 ms") && out.contains("budget 400 ms"), "{out}");
        let (rc, out) = run_clock_step(&chrony_csv("C0000201", "0.0", "Normal"), "", Some("900"));
        assert_eq!(rc, 2, "{out}");
        assert!(out.contains("JOIN_NODE_CLOCK_BUDGET_MS: 900 is not a whole number of milliseconds from 1 to 500"), "{out}");
    }

    /// Over the budget, unsynchronized, or following only its own clock: the
    /// node is refused with GB-297, which names the node as the run names it,
    /// what was measured, the budget and the router limit, and says to repair
    /// time synchronization on that node. The failed phase reads as GB-297.
    #[test]
    fn a_clock_over_budget_or_unsynchronized_is_refused_with_gb_297() {
        for (chrony, measured) in [
            (chrony_csv("C0000201", "0.312300000", "Normal"), "offset 312.3 ms from UTC, over the budget"),
            (chrony_csv("00000000", "0.0", "Not synchronised"), "not synchronized (chrony leap status Not synchronised, no reference)"),
            (chrony_csv("7F7F0101", "0.0", "Normal"), "not synchronized (chrony follows only this host's own local clock)"),
        ] {
            let (rc, out) = run_clock_step(&chrony, "NTPSynchronized=yes", None);
            assert_eq!(rc, 1, "{out}");
            assert!(out.starts_with(&format!("GB-297: node-x's clock is {measured}; budget 250 ms, router limit 500 ms")), "{out}");
            assert!(out.contains("fix: repair time synchronization on node-x"), "{out}");
            let f = crate::explain_phase::failed_phase("join-node", "3", &out, None).expect("phase 3 is in the catalog");
            assert_eq!(f.code.as_deref(), Some("GB-297"), "{out}");
        }
    }

    /// When chronyd does not answer, systemd's flag decides; it gives no
    /// offset, and the line says so. Not synchronized there is refused too.
    #[test]
    fn timedatectl_decides_when_chrony_does_not_answer() {
        let (rc, out) = run_clock_step("", "NTPSynchronized=yes", None);
        assert_eq!(rc, 0, "{out}");
        assert!(out.contains(">> clock: node-x synchronized (timedatectl; it reports no offset)"), "{out}");
        let (rc, out) = run_clock_step("", "NTPSynchronized=no", None);
        assert_eq!(rc, 1, "{out}");
        assert!(out.starts_with("GB-297: node-x's clock is not synchronized (timedatectl NTPSynchronized=no)"), "{out}");
    }

    /// A clock no time service answers for is never a pass: the run goes on,
    /// as join-node does for any fact it could not read, and the line says
    /// NOT verified and which commands it tried.
    #[test]
    fn an_unreadable_clock_is_not_verified_never_passed() {
        let (rc, out) = run_clock_step("", "", None);
        assert_eq!(rc, 0, "{out}");
        assert!(out.contains(">> clock: node-x: NOT verified, not read: no time service answered (tried chronyc -c tracking and timedatectl show -p NTPSynchronized -p TimeUSec)"), "{out}");
        assert!(!out.contains("budget"), "{out}");
    }

    /// A dry run prints the clock step in phase 3, after the config preflight
    /// and before phase 4 masks the apt timers, and runs nothing.
    #[test]
    fn a_dry_run_prints_the_clock_step_before_anything_on_the_node_changes() {
        let t = joiner_checkout();
        let r = execute(t.path(), &opts("zagros-b", "core", "zagros")).unwrap();
        let rows: Vec<&str> = r.checks.iter().map(|c| c.detail.as_str()).collect();
        let at = |needle: &str| rows.iter().position(|d| d.contains(needle)).unwrap_or_else(|| panic!("no row {needle}"));
        assert!(at("standup-config-preflight.py") < at("join-node-clock-check.py") && at("join-node-clock-check.py") < at("=== Phase 4 "));
        assert!(rows[at("join-node-clock-check.py") - 1].starts_with("=== Phase 3 [GATE] the node's clock"), "{rows:?}");
        assert!(rows.iter().all(|d| !d.contains(">> clock:") && !d.contains("GB-297")), "a dry run reads no clock");
    }
}
