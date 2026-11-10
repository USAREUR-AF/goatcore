// `goat admin lan` — LAN troubleshooting toolkit for goat mgmt hosts
// that have been physically relocated and "do not appear on the WAN."
//
// A relocated host can fail several ways at once: the laptop sits on a
// different subnet from the host, or both share a LAN and sshd accepts TCP
// connects but sends NO banner for a long time after cold boot while every
// container runs fine. This module turns that dig into two commands:
//
//   goat admin lan scan              find goat hosts even when DHCP
//                                    moved their IP (MAC-match the ARP
//                                    table after a TCP sweep warms it)
//   goat admin lan doctor [host|ip]  layered L0–L4 triage, each layer
//                                    PASS/FAIL/SKIP + one suggested
//                                    next action
//
// Deliberately depends on NO goatnet/mesh service: no profile cert, no
// portal, no Zenoh, no netbird. Everything here is plain TCP probes,
// `ping`/`arp`/`ssh` child processes, and a host registry read from
// ~/.goat/lan-hosts.yaml). The whole point
// is to work when the mesh does NOT.
//
// Portability: runtime `std::env::consts::OS` dispatch (linux reads
// /proc/net/arp + `ip -4 addr`; macOS parses `ifconfig` + `arp -a`;
// Windows parses `ipconfig` + `arp -a`). No unix-only APIs.

use clap::Subcommand;
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::time::timeout;

/// ARP-warming sweep: per-connect timeout. Even a refused connect (RST)
/// resolves the target's MAC into the ARP cache — we only need the
/// IP-level exchange, not an open port.
const SWEEP_TIMEOUT: Duration = Duration::from_millis(200);
/// Sweep fan-out. 254 hosts × 2 ports ≈ 508 connects; at 128 in flight
/// a /24 finishes in well under two seconds.
const SWEEP_CONCURRENCY: usize = 128;
/// Ports swept to warm ARP + reported per found host.
const PROBE_PORTS: [u16; 2] = [22, 443];
/// L1/L3 single TCP probe timeout (mirrors `goat status`'s ZTOL probe).
const TCP_PROBE_TIMEOUT: Duration = Duration::from_millis(1500);
/// L2: total budget to TCP-connect to 22 AND read the SSH banner. The
/// wedged-sshd failure mode accepts the connect and then says nothing.
const BANNER_TIMEOUT: Duration = Duration::from_secs(10);
/// L3: HTTP GET of the netbird mgmt Prometheus endpoint.
const METRICS_TIMEOUT: Duration = Duration::from_secs(5);
/// L4: overall budget for the one ssh round-trip (ConnectTimeout=15 +
/// the remote script's own 5 s curl ceiling + margin).
const SSH_TOTAL_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Subcommand, Debug)]
pub enum LanCmd {
    /// Find goat mgmt hosts on the local LAN even when DHCP moved their
    /// IP: sweep each local /24 (TCP 22+443, 200 ms) to warm the ARP
    /// cache, then match ARP-table MACs against the host registry
    /// (~/.goat/lan-hosts.yaml).
    Scan {
        /// Emit one JSON object instead of the pretty table.
        #[arg(long)]
        json: bool,
    },
    /// Layered triage of a relocated host: L0 local subnet plausibility,
    /// L1 ping + TCP reachability, L2 sshd banner health (wedged-sshd
    /// detection), L3 mesh-free service-plane probes (443 + :9090
    /// /metrics), L4 on-host checks over one BatchMode ssh (clock /
    /// chrony-presync / default route / WAN / netbird / docker).
    Doctor {
        /// Registry host name (e.g. `mgmt-a-01`) or a literal IPv4
        /// address. Omit to triage every registry host `scan` finds.
        target: Option<String>,
        /// SSH user for the L4 on-host probe (default: the registry
        /// entry's ssh_user, else the context's `mgmt_ssh_user`).
        #[arg(long)]
        ssh_user: Option<String>,
        /// Skip the L4 on-host ssh probe even when the banner is healthy.
        #[arg(long)]
        no_ssh: bool,
        /// Emit one JSON object instead of pretty layer lines.
        #[arg(long)]
        json: bool,
    },
}

pub async fn run(ctx: &super::Ctx, op: LanCmd) -> anyhow::Result<()> {
    let registry = load_registry(&ctx.profile.dir)?;
    match op {
        LanCmd::Scan { json } => scan(&registry, json).await,
        LanCmd::Doctor {
            target,
            ssh_user,
            no_ssh,
            json,
        } => {
            let fallback = ctx.profile.profile_field("mgmt_ssh_user");
            let users = SshUsers {
                flag: ssh_user.as_deref(),
                fallback: fallback.as_deref(),
            };
            doctor(&registry, target.as_deref(), users, no_ssh, json).await
        }
    }
}

// ---- host registry ------------------------------------------------------

/// One known goat mgmt host. The MAC is the durable identity here — the
/// LAN IP is whatever the travel router's DHCP handed out this time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanHost {
    pub name: String,
    /// Normalized lowercase colon-separated (`02:00:00:aa:e5:26`). (opsec-allow: synthetic test MAC)
    pub mac: String,
    /// Last known LAN IP — a hint, not truth (DHCP may have moved it).
    pub last_lan_ip: Ipv4Addr,
    /// Mesh (netbird) IP — informational; this toolkit never needs it up.
    pub mesh_ip: Ipv4Addr,
    /// Empty = not declared for this host; the context's `mgmt_ssh_user` applies.
    #[serde(default)]
    pub ssh_user: String,
    #[serde(default)]
    pub goatnet: String,
}

/// Where the L4 probe's SSH user comes from, in order: the `--ssh-user` flag,
/// the registry entry, then the context's `mgmt_ssh_user` (goatnet.toml).
#[derive(Clone, Copy, Default)]
struct SshUsers<'a> {
    flag: Option<&'a str>,
    fallback: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
struct RegistryFile {
    #[serde(default)]
    hosts: Vec<LanHost>,
}

/// Compiled-in hosts: none. A goatnet's hosts are local facts, listed in
/// `~/.goat/lan-hosts.yaml`.
fn embedded_registry() -> Vec<LanHost> {
    Vec::new()
}

/// The compiled-in hosts (none) merged with the host file at
/// `<profile-dir>/lan-hosts.yaml` (`~/.goat/lan-hosts.yaml` by default).
/// Override entries replace same-name embedded entries; new names append.
fn load_registry(profile_dir: &std::path::Path) -> anyhow::Result<Vec<LanHost>> {
    let mut hosts = embedded_registry();
    let path = profile_dir.join("lan-hosts.yaml");
    if let Ok(body) = std::fs::read_to_string(&path) {
        let file: RegistryFile = serde_yaml::from_str(&body)
            .map_err(|e| anyhow::anyhow!("parse {}: {e}", path.display()))?;
        for entry in file.hosts {
            let mac = normalize_mac(&entry.mac).ok_or_else(|| {
                anyhow::anyhow!(
                    "{}: host `{}` has malformed mac `{}` (want six hex octets)",
                    path.display(),
                    entry.name,
                    entry.mac
                )
            })?;
            let entry = LanHost { mac, ..entry };
            match hosts.iter_mut().find(|h| h.name == entry.name) {
                Some(existing) => *existing = entry,
                None => hosts.push(entry),
            }
        }
    }
    Ok(hosts)
}

// ---- local interface / subnet discovery ----------------------------------

#[derive(Debug, Clone, Serialize)]
struct LocalNet {
    iface: String,
    ip: Ipv4Addr,
    prefix: u8,
}

impl LocalNet {
    fn contains(&self, ip: Ipv4Addr) -> bool {
        ip_in_net(ip, self.ip, self.prefix)
    }
}

fn ip_in_net(ip: Ipv4Addr, net_ip: Ipv4Addr, prefix: u8) -> bool {
    if prefix == 0 {
        return true;
    }
    if prefix > 32 {
        return false;
    }
    let mask: u32 = u32::MAX << (32 - u32::from(prefix));
    (u32::from(ip) & mask) == (u32::from(net_ip) & mask)
}

/// The /24 containing `ip`, as its network base (`a.b.c.0`).
fn slash24(ip: Ipv4Addr) -> Ipv4Addr {
    let o = ip.octets();
    Ipv4Addr::new(o[0], o[1], o[2], 0)
}

fn is_sweepable(ip: Ipv4Addr) -> bool {
    // CGNAT 100.64.0.0/10 is the netbird mesh overlay (utun/wt0) —
    // point-to-point, no ARP, not the physical LAN this toolkit hunts.
    let o = ip.octets();
    let cgnat = o[0] == 100 && (64..128).contains(&o[1]);
    !ip.is_loopback() && !ip.is_link_local() && !cgnat
}

/// Shell out to the platform's interface lister and parse IPv4 addrs.
/// linux: `ip -4 addr` (fallback `ifconfig`); windows: `ipconfig`;
/// everything else (macOS/BSD): `ifconfig`.
fn discover_local_nets() -> anyhow::Result<Vec<LocalNet>> {
    let nets = match std::env::consts::OS {
        "linux" => match run_capture("ip", &["-4", "addr"]) {
            Some(out) => parse_ip_addr_output(&out),
            None => parse_ifconfig_output(&run_capture("ifconfig", &[]).unwrap_or_default()),
        },
        "windows" => parse_ipconfig_output(&run_capture("ipconfig", &[]).unwrap_or_default()),
        _ => parse_ifconfig_output(&run_capture("ifconfig", &[]).unwrap_or_default()),
    };
    let nets: Vec<LocalNet> = nets.into_iter().filter(|n| is_sweepable(n.ip)).collect();
    if nets.is_empty() {
        anyhow::bail!(
            "could not determine any local IPv4 subnet (is the laptop on a network at all?)"
        );
    }
    Ok(nets)
}

/// Run a command and capture stdout as a lossy string. `None` if the
/// binary is missing or exits non-zero — callers fall back or surface
/// an empty parse.
fn run_capture(bin: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(bin).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Parse `ip -4 addr` (linux). Interface header lines look like
/// `2: enp1s0: <BROADCAST,...> mtu 1500 ...`; address lines are
/// indented `inet 192.168.8.171/24 brd ... scope global ...`.
fn parse_ip_addr_output(out: &str) -> Vec<LocalNet> {
    let mut nets = Vec::new();
    let mut iface = String::new();
    for line in out.lines() {
        if !line.starts_with(char::is_whitespace) {
            // `2: enp1s0: <...>` → `enp1s0`
            let mut parts = line.splitn(3, ':');
            let _idx = parts.next();
            if let Some(name) = parts.next() {
                iface = name.trim().to_string();
            }
            continue;
        }
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("inet ") {
            let cidr = rest.split_whitespace().next().unwrap_or_default();
            if let Some((ip_s, prefix_s)) = cidr.split_once('/') {
                if let (Ok(ip), Ok(prefix)) = (ip_s.parse::<Ipv4Addr>(), prefix_s.parse::<u8>()) {
                    nets.push(LocalNet {
                        iface: iface.clone(),
                        ip,
                        prefix,
                    });
                }
            }
        }
    }
    nets
}

/// Parse `ifconfig` (macOS/BSD and the linux net-tools variant).
/// Interface headers are `en0: flags=8863<...>`; address lines are
/// `inet 192.168.8.50 netmask 0xffffff00 ...` (BSD hex mask) or
/// `inet 192.168.8.171  netmask 255.255.255.0 ...` (net-tools).
fn parse_ifconfig_output(out: &str) -> Vec<LocalNet> {
    let mut nets = Vec::new();
    let mut iface = String::new();
    for line in out.lines() {
        if !line.starts_with(char::is_whitespace) {
            if let Some((name, _)) = line.split_once(':') {
                iface = name.trim().to_string();
            }
            continue;
        }
        let t = line.trim();
        let Some(rest) = t.strip_prefix("inet ") else {
            continue;
        };
        let mut ip = None;
        let mut prefix = 24u8; // sensible sweep default when no mask shown
        let mut words = rest.split_whitespace();
        if let Some(ip_s) = words.next() {
            ip = ip_s.parse::<Ipv4Addr>().ok();
        }
        let mut words = rest.split_whitespace().peekable();
        while let Some(w) = words.next() {
            if w == "netmask" {
                if let Some(mask_s) = words.peek() {
                    if let Some(p) = parse_netmask(mask_s) {
                        prefix = p;
                    }
                }
            }
        }
        if let Some(ip) = ip {
            nets.push(LocalNet {
                iface: iface.clone(),
                ip,
                prefix,
            });
        }
    }
    nets
}

/// `0xffffff00` (BSD) or `255.255.255.0` (net-tools / Windows) → prefix.
fn parse_netmask(s: &str) -> Option<u8> {
    let bits: u32 = if let Some(hex) = s.strip_prefix("0x") {
        u32::from_str_radix(hex, 16).ok()?
    } else {
        u32::from(s.parse::<Ipv4Addr>().ok()?)
    };
    // Must be contiguous high bits to be a valid mask.
    let prefix = bits.count_ones();
    let expected = if prefix == 0 { 0 } else { u32::MAX << (32 - prefix) };
    (bits == expected).then_some(prefix as u8)
}

/// Parse `ipconfig` (Windows). Adapter headers end in `adapter <name>:`;
/// the values we want are `IPv4 Address. . . : 192.168.8.50(Preferred)`
/// and `Subnet Mask . . . : 255.255.255.0`.
fn parse_ipconfig_output(out: &str) -> Vec<LocalNet> {
    let mut nets = Vec::new();
    let mut iface = String::new();
    let mut pending_ip: Option<Ipv4Addr> = None;
    for line in out.lines() {
        let t = line.trim();
        if t.ends_with(':') && t.to_ascii_lowercase().contains("adapter ") {
            // Flush an IP that never saw a mask (assume /24 for the sweep).
            if let Some(ip) = pending_ip.take() {
                nets.push(LocalNet {
                    iface: iface.clone(),
                    ip,
                    prefix: 24,
                });
            }
            iface = t.trim_end_matches(':').to_string();
            continue;
        }
        let Some((label, value)) = t.split_once(':') else {
            continue;
        };
        let label = label.trim_end_matches(['.', ' ']).trim();
        let value = value.trim();
        if label.starts_with("IPv4 Address") {
            // `(Preferred)` and similar suffixes ride after the address.
            let ip_s = value.split('(').next().unwrap_or_default().trim();
            pending_ip = ip_s.parse::<Ipv4Addr>().ok();
        } else if label.starts_with("Subnet Mask") {
            if let Some(ip) = pending_ip.take() {
                let prefix = parse_netmask(value).unwrap_or(24);
                nets.push(LocalNet {
                    iface: iface.clone(),
                    ip,
                    prefix,
                });
            }
        }
    }
    if let Some(ip) = pending_ip {
        nets.push(LocalNet {
            iface,
            ip,
            prefix: 24,
        });
    }
    nets
}

// ---- ARP table ------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
struct ArpEntry {
    ip: Ipv4Addr,
    /// Normalized lowercase colon-separated.
    mac: String,
}

/// Read the OS ARP table. linux: /proc/net/arp (no child process);
/// everywhere else (and as a linux fallback): parse `arp -a`, whose
/// macOS and Windows formats both yield to one parser.
fn read_arp_table() -> Vec<ArpEntry> {
    if std::env::consts::OS == "linux" {
        if let Ok(body) = std::fs::read_to_string("/proc/net/arp") {
            return parse_proc_net_arp(&body);
        }
    }
    parse_arp_a_output(&run_capture("arp", &["-a"]).unwrap_or_default())
}

/// `/proc/net/arp`: header line then
/// `192.168.8.171  0x1  0x2  02:00:00:aa:e5:26  *  enp1s0`. (opsec-allow: synthetic test MAC)
/// Flags 0x0 = incomplete entry; skip those and all-zero MACs.
fn parse_proc_net_arp(body: &str) -> Vec<ArpEntry> {
    body.lines()
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 4 || f[2] == "0x0" {
                return None;
            }
            let ip = f[0].parse::<Ipv4Addr>().ok()?;
            let mac = normalize_mac(f[3])?;
            (mac != "00:00:00:00:00:00").then_some(ArpEntry { ip, mac })
        })
        .collect()
}

/// `arp -a` — both observed dialects:
///   macOS:   `? (192.168.8.171) at 02:00:00:aa:e5:26 on en0 ifscope [ethernet]` (opsec-allow: synthetic test MAC)
///   Windows: `  192.168.8.171         02-00-00-aa-e5-26     dynamic` (opsec-allow: synthetic test MAC)
/// macOS prints `(incomplete)` for unresolved entries; Windows headers
/// and `Interface:` lines fall out because their fields don't parse.
fn parse_arp_a_output(out: &str) -> Vec<ArpEntry> {
    let mut entries = Vec::new();
    for line in out.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if let Some(open) = t.find('(') {
            // macOS/BSD dialect.
            let Some(close) = t[open..].find(')') else {
                continue;
            };
            let Ok(ip) = t[open + 1..open + close].parse::<Ipv4Addr>() else {
                continue;
            };
            let Some(at) = t.find(" at ") else { continue };
            let mac_tok = t[at + 4..].split_whitespace().next().unwrap_or_default();
            if let Some(mac) = normalize_mac(mac_tok) {
                entries.push(ArpEntry { ip, mac });
            }
            continue;
        }
        // Windows dialect: `<ip> <mac> <type>`.
        let f: Vec<&str> = t.split_whitespace().collect();
        if f.len() >= 2 {
            if let (Ok(ip), Some(mac)) = (f[0].parse::<Ipv4Addr>(), normalize_mac(f[1])) {
                entries.push(ArpEntry { ip, mac });
            }
        }
    }
    entries
}

/// Normalize a MAC to lowercase colon-separated with zero-padded octets.
/// Accepts `:` or `-` separators and unpadded octets (macOS `arp -a`
/// prints `0:11:22:...` for `00:11:22:...`). `None` if it isn't six hex
/// octets.
fn normalize_mac(s: &str) -> Option<String> {
    let parts: Vec<&str> = s.split([':', '-']).collect();
    if parts.len() != 6 {
        return None;
    }
    let mut octets = Vec::with_capacity(6);
    for p in parts {
        if p.is_empty() || p.len() > 2 || !p.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        octets.push(format!("{:0>2}", p.to_ascii_lowercase()));
    }
    Some(octets.join(":"))
}

// ---- scan -----------------------------------------------------------------

#[derive(Debug, Serialize)]
struct FoundHost {
    name: String,
    goatnet: String,
    ip: Ipv4Addr,
    mac: String,
    /// Probe ports that completed a TCP connect during the sweep.
    ports: Vec<u16>,
    /// e.g. "IP changed from 192.168.8.171".
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

#[derive(Debug, Serialize)]
struct ScanReport {
    nets: Vec<LocalNet>,
    found: Vec<FoundHost>,
    /// Registry hosts whose last LAN IP is inside a local subnet but
    /// whose MAC never showed up — expected here, not answering.
    missing_expected: Vec<String>,
    /// False = nothing found AND no registry IP is plausible here: the
    /// operator is probably not on the goat host LAN at all.
    on_goat_lan: bool,
}

/// Sweep + ARP + registry-match. Shared by `scan` and `doctor`'s
/// target-locating pass.
async fn scan_lan(registry: &[LanHost], nets: &[LocalNet]) -> ScanReport {
    // Sweep the /24 around every local address, plus the /24 of any
    // registry hint that falls inside a wider-than-/24 local net (a /16
    // laptop subnet shouldn't hide a Thor sitting three /24s away).
    let mut bases: Vec<Ipv4Addr> = Vec::new();
    for n in nets {
        let b = slash24(n.ip);
        if !bases.contains(&b) {
            bases.push(b);
        }
        for h in registry {
            let b = slash24(h.last_lan_ip);
            if n.contains(h.last_lan_ip) && !bases.contains(&b) {
                bases.push(b);
            }
        }
    }

    let mut answered: BTreeMap<Ipv4Addr, Vec<u16>> = BTreeMap::new();
    for base in &bases {
        for (ip, port) in sweep_slash24(*base).await {
            answered.entry(ip).or_default().push(port);
        }
    }

    let arp = read_arp_table();
    let mut found = Vec::new();
    for host in registry {
        if let Some(entry) = arp.iter().find(|e| e.mac == host.mac) {
            let note = (entry.ip != host.last_lan_ip)
                .then(|| format!("IP changed from {}", host.last_lan_ip));
            found.push(FoundHost {
                name: host.name.clone(),
                goatnet: host.goatnet.clone(),
                ip: entry.ip,
                mac: host.mac.clone(),
                ports: answered.get(&entry.ip).cloned().unwrap_or_default(),
                note,
            });
        }
    }

    let missing_expected: Vec<String> = registry
        .iter()
        .filter(|h| {
            nets.iter().any(|n| n.contains(h.last_lan_ip))
                && !found.iter().any(|f| f.name == h.name)
        })
        .map(|h| h.name.clone())
        .collect();

    // "Plausibly the goat LAN" = we found a host, or at least one
    // registry last-LAN-IP lives inside a local subnet.
    let on_goat_lan = !found.is_empty()
        || registry
            .iter()
            .any(|h| nets.iter().any(|n| n.contains(h.last_lan_ip)));

    ScanReport {
        nets: nets.to_vec(),
        found,
        missing_expected,
        on_goat_lan,
    }
}

/// Parallel TCP-connect sweep of one /24 on `PROBE_PORTS`. Returns the
/// (ip, port) pairs that connected. The side effect is the point: every
/// attempt — success, refusal, or timeout-after-ARP — populates the OS
/// ARP cache for the MAC match that follows.
async fn sweep_slash24(base: Ipv4Addr) -> Vec<(Ipv4Addr, u16)> {
    let o = base.octets();
    let targets: Vec<(Ipv4Addr, u16)> = (1u8..=254)
        .flat_map(|h| {
            let ip = Ipv4Addr::new(o[0], o[1], o[2], h);
            PROBE_PORTS.iter().map(move |&p| (ip, p))
        })
        .collect();
    stream::iter(targets)
        .map(|(ip, port)| async move {
            let ok = matches!(
                timeout(SWEEP_TIMEOUT, TcpStream::connect((IpAddr::V4(ip), port))).await,
                Ok(Ok(_))
            );
            (ip, port, ok)
        })
        .buffer_unordered(SWEEP_CONCURRENCY)
        .filter_map(|(ip, port, ok)| async move { ok.then_some((ip, port)) })
        .collect()
        .await
}

const NOT_ON_GOAT_LAN: &str = "you may not be on the goat host LAN — no registry host answered \
     and no registry last-LAN-IP is inside a local subnet. Join the hosts' Wi-Fi or LAN and re-run; \
     hosts are listed in ~/.goat/lan-hosts.yaml. A laptop on a phone hotspot is the usual cause.";

async fn scan(registry: &[LanHost], json: bool) -> anyhow::Result<()> {
    let nets = discover_local_nets()?;
    for n in &nets {
        eprintln!("local subnet: {} {}/{}", n.iface, n.ip, n.prefix);
    }
    eprintln!(
        "sweeping {} /24(s) on ports {} (≈2 s each) ...",
        nets.iter()
            .map(|n| slash24(n.ip))
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        PROBE_PORTS.map(|p| p.to_string()).join("+"),
    );
    let report = scan_lan(registry, &nets).await;

    if json {
        println!("{}", serde_json::to_string(&report)?);
    } else if report.found.is_empty() {
        println!("no goat mgmt host found on the local LAN.");
        if report.on_goat_lan {
            println!(
                "expected here but not answering: {} — check power/cabling, then \
                 `goat admin lan doctor <host>`",
                report.missing_expected.join(", ")
            );
        } else {
            println!("{NOT_ON_GOAT_LAN}");
        }
    } else {
        let name_w = report
            .found
            .iter()
            .map(|f| f.name.len())
            .max()
            .unwrap_or(4)
            .max("name".len());
        println!(
            "{:<name_w$}  {:<15}  {:<17}  {:<7}  goatnet",
            "name", "ip", "mac", "ports"
        );
        for f in &report.found {
            let ports = if f.ports.is_empty() {
                "-".to_string()
            } else {
                f.ports
                    .iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            };
            println!(
                "{:<name_w$}  {:<15}  {:<17}  {:<7}  {}{}",
                f.name,
                f.ip.to_string(),
                f.mac,
                ports,
                f.goatnet,
                f.note
                    .as_deref()
                    .map(|n| format!("   ({n})"))
                    .unwrap_or_default(),
            );
        }
        for name in &report.missing_expected {
            println!("not found (expected on this LAN): {name}");
        }
    }

    if report.found.is_empty() {
        std::process::exit(crate::exit_codes::EXIT_GENERIC);
    }
    Ok(())
}

// ---- doctor ---------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum CheckStatus {
    Pass,
    Fail,
    /// Informational miss that must not fail the run — e.g. L0 "not on the
    /// host LAN" while the target is reachable anyway (mesh/VPN path).
    Warn,
    Skip,
}

impl CheckStatus {
    fn glyph(self) -> &'static str {
        match self {
            CheckStatus::Pass => "[✓]",
            CheckStatus::Fail => "[✗]",
            CheckStatus::Warn => "[!]",
            CheckStatus::Skip => "[-]",
        }
    }
}

#[derive(Debug, Serialize)]
struct LayerOutcome {
    layer: &'static str,
    name: &'static str,
    status: CheckStatus,
    detail: String,
    /// One-line suggested next action (FAILs carry one).
    #[serde(skip_serializing_if = "Option::is_none")]
    next: Option<String>,
    /// L4 per-check breakdown.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    checks: Vec<RemoteCheck>,
}

impl LayerOutcome {
    fn new(layer: &'static str, name: &'static str, status: CheckStatus, detail: String) -> Self {
        Self {
            layer,
            name,
            status,
            detail,
            next: None,
            checks: Vec::new(),
        }
    }
    fn with_next(mut self, next: String) -> Self {
        self.next = Some(next);
        self
    }
}

#[derive(Debug, Serialize)]
struct RemoteCheck {
    name: &'static str,
    status: CheckStatus,
    detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    next: Option<String>,
}

#[derive(Debug, Serialize)]
struct DoctorReport {
    target: String,
    ip: Ipv4Addr,
    ssh_user: String,
    layers: Vec<LayerOutcome>,
    all_pass: bool,
}

struct DoctorTarget {
    label: String,
    ip: Ipv4Addr,
    ssh_user: String,
}

async fn doctor(
    registry: &[LanHost],
    target: Option<&str>,
    users: SshUsers<'_>,
    no_ssh: bool,
    json: bool,
) -> anyhow::Result<()> {
    // Resolve the target list BEFORE touching the network so a typo'd
    // host name fails fast and deterministically.
    if let Some(t) = target {
        if t.parse::<Ipv4Addr>().is_err() && !registry.iter().any(|h| h.name == t) {
            anyhow::bail!(
                "no registry host named `{t}` and it is not an IPv4 address — known hosts: {} \
                 (list hosts in ~/.goat/lan-hosts.yaml)",
                registry
                    .iter()
                    .map(|h| h.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }

    let nets = discover_local_nets()?;
    let targets = resolve_targets(registry, &nets, target, users).await?;

    let mut reports = Vec::new();
    for t in &targets {
        eprintln!("triaging {} ({}) ...", t.label, t.ip);
        reports.push(run_layers(t, &nets, registry, no_ssh).await);
    }
    let all_pass = reports.iter().all(|r| r.all_pass);

    if json {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "targets": reports,
                "all_pass": all_pass,
            }))?
        );
    } else {
        for r in &reports {
            println!("── {} ({}) ──", r.target, r.ip);
            for l in &r.layers {
                println!("{} {} {:<13} {}", l.status.glyph(), l.layer, l.name, l.detail);
                if let Some(next) = &l.next {
                    println!("                      → {next}");
                }
                for c in &l.checks {
                    println!("      {} {:<15} {}", c.status.glyph(), c.name, c.detail);
                    if let Some(next) = &c.next {
                        println!("                        → {next}");
                    }
                }
            }
        }
    }

    if !all_pass {
        std::process::exit(crate::exit_codes::EXIT_GENERIC);
    }
    Ok(())
}

/// Turn `[host|ip]` into concrete probe targets. Named hosts get located
/// by MAC via the scan machinery (DHCP may have moved them); a bare IP
/// is taken literally; no argument means "every registry host scan finds."
async fn resolve_targets(
    registry: &[LanHost],
    nets: &[LocalNet],
    target: Option<&str>,
    users: SshUsers<'_>,
) -> anyhow::Result<Vec<DoctorTarget>> {
    let user_for = |h: Option<&LanHost>| -> String {
        users
            .flag
            .map(str::to_owned)
            .or_else(|| h.map(|h| h.ssh_user.clone()).filter(|u| !u.is_empty()))
            .or_else(|| users.fallback.map(str::to_owned))
            .unwrap_or_default()
    };

    match target {
        Some(t) => {
            if let Ok(ip) = t.parse::<Ipv4Addr>() {
                // Literal IP. Attach registry metadata if the IP matches
                // a known hint so L4 gets the right ssh user.
                let host = registry
                    .iter()
                    .find(|h| h.last_lan_ip == ip || h.mesh_ip == ip);
                return Ok(vec![DoctorTarget {
                    label: host.map(|h| h.name.clone()).unwrap_or_else(|| t.to_string()),
                    ip,
                    ssh_user: user_for(host),
                }]);
            }
            // Registry name (existence pre-checked in doctor()).
            let host = registry
                .iter()
                .find(|h| h.name == t)
                .expect("pre-checked registry name");
            let report = scan_lan(registry, nets).await;
            let ip = match report.found.iter().find(|f| f.name == host.name) {
                Some(f) => {
                    if let Some(note) = &f.note {
                        eprintln!("{}: {note}", host.name);
                    }
                    f.ip
                }
                None => {
                    eprintln!(
                        "{}: MAC {} not in the ARP table after sweep — falling back to last \
                         known IP {}",
                        host.name, host.mac, host.last_lan_ip
                    );
                    host.last_lan_ip
                }
            };
            Ok(vec![DoctorTarget {
                label: host.name.clone(),
                ip,
                ssh_user: user_for(Some(host)),
            }])
        }
        None => {
            let report = scan_lan(registry, nets).await;
            if report.found.is_empty() {
                anyhow::bail!(
                    "no goat mgmt host found on the local LAN to triage. {}",
                    if report.on_goat_lan {
                        format!(
                            "Expected here but not answering: {} — name one explicitly \
                             (`goat admin lan doctor <host>`) to probe its last known IP.",
                            report.missing_expected.join(", ")
                        )
                    } else {
                        NOT_ON_GOAT_LAN.to_string()
                    }
                );
            }
            Ok(report
                .found
                .iter()
                .map(|f| DoctorTarget {
                    label: f.name.clone(),
                    ip: f.ip,
                    ssh_user: user_for(registry.iter().find(|h| h.name == f.name)),
                })
                .collect())
        }
    }
}

async fn run_layers(
    t: &DoctorTarget,
    nets: &[LocalNet],
    registry: &[LanHost],
    no_ssh: bool,
) -> DoctorReport {
    let mut layers = Vec::new();

    // L0 — am I plausibly on the host LAN at all?
    layers.push(layer_l0(t.ip, nets, registry));

    // L1 — ping + TCP 22/443.
    layers.push(layer_l1(t.ip).await);

    layers[0].status = reconcile_l0(layers[0].status, layers[1].status);

    // L2 — sshd banner health (the wedged-sshd detector).
    let banner = probe_ssh_banner(t.ip).await;
    layers.push(layer_l2(&banner));

    // L3 — service-plane probes, no SSH needed.
    layers.push(layer_l3(t.ip).await);

    // L4 — on-host checks over one BatchMode ssh, only when the banner
    // is healthy (a wedged sshd would just eat the 10–15 s timeouts).
    let l4 = if no_ssh {
        LayerOutcome::new(
            "L4",
            "on-host",
            CheckStatus::Skip,
            "skipped (--no-ssh)".into(),
        )
    } else if matches!(banner, BannerState::Banner(_)) && t.ssh_user.is_empty() {
        LayerOutcome::new(
            "L4",
            "on-host",
            CheckStatus::Skip,
            "skipped: no SSH user (pass --ssh-user, or run `goat context refresh`)".into(),
        )
    } else if matches!(banner, BannerState::Banner(_)) {
        layer_l4(t.ip, &t.ssh_user).await
    } else {
        LayerOutcome::new(
            "L4",
            "on-host",
            CheckStatus::Skip,
            "skipped — L2 banner not healthy, ssh would hang".into(),
        )
    };
    layers.push(l4);

    let all_pass = layers.iter().all(|l| l.status != CheckStatus::Fail)
        && layers
            .iter()
            .flat_map(|l| &l.checks)
            .all(|c| c.status != CheckStatus::Fail);

    DoctorReport {
        target: t.label.clone(),
        ip: t.ip,
        ssh_user: t.ssh_user.clone(),
        layers,
        all_pass,
    }
}

/// L0 is hard-FAIL only when the target is ALSO unreachable — that's the
/// genuine "can't reach it and you're not on its LAN" case. A reachable
/// target (mesh/VPN path from off-LAN, e.g. hotspot laptop
/// → mesh IP → fully healthy Thor) downgrades L0 to WARN so a healthy host
/// exits 0; the printed context stays.
fn reconcile_l0(l0: CheckStatus, l1: CheckStatus) -> CheckStatus {
    if l0 == CheckStatus::Fail && l1 == CheckStatus::Pass {
        CheckStatus::Warn
    } else {
        l0
    }
}

fn layer_l0(target: Ipv4Addr, nets: &[LocalNet], registry: &[LanHost]) -> LayerOutcome {
    let nets_s = nets
        .iter()
        .map(|n| format!("{} {}/{}", n.iface, n.ip, n.prefix))
        .collect::<Vec<_>>()
        .join(", ");
    let target_local = nets
        .iter()
        .any(|n| n.contains(target) || slash24(n.ip) == slash24(target));
    if target_local {
        LayerOutcome::new(
            "L0",
            "local",
            CheckStatus::Pass,
            format!("on {nets_s} — same subnet as target {target}"),
        )
    } else {
        let registry_local = registry
            .iter()
            .any(|h| nets.iter().any(|n| n.contains(h.last_lan_ip)));
        let mut o = LayerOutcome::new(
            "L0",
            "local",
            CheckStatus::Fail,
            format!("on {nets_s} — target {target} is not in any local subnet"),
        );
        o = o.with_next(if registry_local {
            "another registry host IS local — the target may have moved LANs; re-run \
             `goat admin lan scan`"
                .into()
        } else {
            NOT_ON_GOAT_LAN.into()
        });
        o
    }
}

async fn layer_l1(ip: Ipv4Addr) -> LayerOutcome {
    let (ping_ok, t22, t443) = tokio::join!(ping_once(ip), tcp_probe(ip, 22), tcp_probe(ip, 443));
    let detail = format!(
        "ping {} · tcp/22 {} · tcp/443 {}",
        if ping_ok { "ok" } else { "FAIL" },
        if t22 { "ok" } else { "FAIL" },
        if t443 { "ok" } else { "FAIL" },
    );
    if ping_ok || t22 || t443 {
        LayerOutcome::new("L1", "reachability", CheckStatus::Pass, detail)
    } else {
        LayerOutcome::new("L1", "reachability", CheckStatus::Fail, detail).with_next(
            "host not answering at this IP — check power/cabling/DHCP, then re-run \
             `goat admin lan scan` (its IP may have changed)"
                .into(),
        )
    }
}

/// One ping, OS-correct flags, ~2 s budget. Shelling out keeps us off
/// raw sockets (which would need privileges and unix-only APIs).
async fn ping_once(ip: Ipv4Addr) -> bool {
    let ip_s = ip.to_string();
    let args: Vec<&str> = match std::env::consts::OS {
        "windows" => vec!["-n", "1", "-w", "2000", &ip_s],
        "macos" => vec!["-c", "1", "-t", "2", &ip_s],
        // linux + other unix: -W is the per-reply timeout in seconds.
        _ => vec!["-c", "1", "-W", "2", &ip_s],
    };
    match timeout(
        Duration::from_secs(4),
        tokio::process::Command::new("ping").args(&args).output(),
    )
    .await
    {
        Ok(Ok(out)) => out.status.success(),
        _ => false,
    }
}

async fn tcp_probe(ip: Ipv4Addr, port: u16) -> bool {
    matches!(
        timeout(TCP_PROBE_TIMEOUT, TcpStream::connect((IpAddr::V4(ip), port))).await,
        Ok(Ok(_))
    )
}

#[derive(Debug)]
enum BannerState {
    /// TCP connect itself failed — host or sshd down.
    NoConnect(String),
    /// TCP connected but nothing arrived within the budget — the
    /// wedged-sshd state: accepts without a banner, possibly for ~90 min after
    /// cold boot, while every container runs fine.
    NoBanner,
    /// Got a banner line.
    Banner(String),
}

async fn probe_ssh_banner(ip: Ipv4Addr) -> BannerState {
    let mut stream = match timeout(
        Duration::from_secs(5),
        TcpStream::connect((IpAddr::V4(ip), 22)),
    )
    .await
    {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => return BannerState::NoConnect(e.to_string()),
        Err(_) => return BannerState::NoConnect("connect timeout".into()),
    };
    let mut buf = [0u8; 256];
    match timeout(BANNER_TIMEOUT, stream.read(&mut buf)).await {
        Ok(Ok(n)) if n > 0 => {
            let line = String::from_utf8_lossy(&buf[..n]);
            BannerState::Banner(line.lines().next().unwrap_or_default().trim().to_string())
        }
        // 0-byte read (peer closed silently), read error, or timeout —
        // all "connected but no banner."
        _ => BannerState::NoBanner,
    }
}

fn layer_l2(banner: &BannerState) -> LayerOutcome {
    match banner {
        BannerState::Banner(b) => LayerOutcome::new(
            "L2",
            "sshd-banner",
            CheckStatus::Pass,
            format!("banner: {b}"),
        ),
        BannerState::NoBanner => LayerOutcome::new(
            "L2",
            "sshd-banner",
            CheckStatus::Fail,
            format!(
                "TCP 22 connects but no SSH banner within {}s — sshd wedged \
                 (observed 2026-06-12: accepts-without-banner ~90 min after cold boot \
                 while every container ran fine)",
                BANNER_TIMEOUT.as_secs()
            ),
        )
        .with_next(
            "service plane is likely fine — check L3; wait it out or console in".into(),
        ),
        BannerState::NoConnect(e) => LayerOutcome::new(
            "L2",
            "sshd-banner",
            CheckStatus::Fail,
            format!("no TCP connect on 22 ({e}) — host or sshd down"),
        )
        .with_next("if L3 passes, sshd alone is down (console in); if not, the host is likely down".into()),
    }
}

async fn layer_l3(ip: Ipv4Addr) -> LayerOutcome {
    let (t443, metrics) = tokio::join!(tcp_probe(ip, 443), probe_netbird_metrics(ip));
    let detail = format!(
        "tcp/443 {} (Traefik) · :9090/metrics {} (netbird mgmt)",
        if t443 { "ok" } else { "FAIL" },
        match &metrics {
            Ok(lines) => format!("ok, {lines} lines"),
            Err(e) => format!("FAIL ({e})"),
        }
    );
    // Either probe answering is proof of life for the service plane —
    // the triage question this layer answers, even while sshd is wedged.
    if t443 || metrics.is_ok() {
        LayerOutcome::new("L3", "service-plane", CheckStatus::Pass, detail)
    } else {
        LayerOutcome::new("L3", "service-plane", CheckStatus::Fail, detail).with_next(
            "no service-plane signal — containers may not be up (cold boot + LUKS still \
             locked?); if L2 is healthy, ssh in and check `docker ps`"
                .into(),
        )
    }
}

/// GET http://ip:9090/metrics and sanity-check the body is Prometheus
/// exposition format. This is the netbird mgmt metrics listener — it
/// answers on the LAN IP even while sshd is wedged, which makes it the
/// best no-SSH liveness signal.
async fn probe_netbird_metrics(ip: Ipv4Addr) -> Result<usize, String> {
    let client = reqwest::Client::builder()
        .timeout(METRICS_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(format!("http://{ip}:9090/metrics"))
        .send()
        .await
        .map_err(|e| brief_reqwest_err(&e))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status().as_u16()));
    }
    let body = resp.text().await.map_err(|e| brief_reqwest_err(&e))?;
    if looks_like_prometheus(&body) {
        Ok(body.lines().count())
    } else {
        Err("body is not Prometheus exposition format".into())
    }
}

/// reqwest errors chain verbosely; keep the layer line one line.
fn brief_reqwest_err(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "timeout".into()
    } else if e.is_connect() {
        "connect failed".into()
    } else {
        e.to_string()
    }
}

fn looks_like_prometheus(body: &str) -> bool {
    body.lines().take(50).any(|l| {
        l.starts_with("# HELP")
            || l.starts_with("# TYPE")
            || (l
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && l.contains(' '))
    })
}

// ---- L4: one BatchMode ssh, compact remote script -------------------------

/// Remote probe script. Plain POSIX sh where possible; the /dev/tcp WAN
/// fallback is bash-only and degrades to "no" elsewhere. `sudo -n`
/// (NOPASSWD is the mgmt-host day-1 baseline) with a non-sudo fallback
/// for netbird/docker. Every value rides a GOAT:<KEY>: line so the
/// parser needs no positional assumptions.
const REMOTE_SCRIPT: &str = r#"
echo "GOAT:DATE:$(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "GOAT:UPTIME:$(uptime 2>/dev/null | tr -s ' ' | sed 's/^ //')"
if [ -e /run/chrony-presync.done ]; then echo "GOAT:PRESYNC:yes"; else echo "GOAT:PRESYNC:no"; fi
echo "GOAT:CHRONY:$( (sudo -n chronyc tracking 2>/dev/null || chronyc tracking 2>/dev/null) | awk -F': *' '/^Leap status/{print $2}')"
echo "GOAT:DEFROUTE:$(ip route show default 2>/dev/null | head -1)"
if curl -sm5 https://1.1.1.1 -o /dev/null 2>/dev/null; then echo "GOAT:WAN:yes"; elif (exec 3<>/dev/tcp/1.1.1.1/443) 2>/dev/null; then echo "GOAT:WAN:yes"; else echo "GOAT:WAN:no"; fi
NB="$(sudo -n netbird status 2>/dev/null || netbird status 2>/dev/null)"
echo "GOAT:NETBIRD_MGMT:$(printf '%s\n' "$NB" | awk -F': *' '/^Management:/{print $2}')"
echo "GOAT:NETBIRD_SIGNAL:$(printf '%s\n' "$NB" | awk -F': *' '/^Signal:/{print $2}')"
DPS="$(sudo -n docker ps --format '{{.Names}} {{.Status}}' 2>/dev/null || docker ps --format '{{.Names}} {{.Status}}' 2>/dev/null)"
echo "GOAT:DOCKER_RUNNING:$(printf '%s\n' "$DPS" | sed '/^$/d' | wc -l | tr -d ' ')"
echo "GOAT:DOCKER_BAD:$(printf '%s\n' "$DPS" | grep -Ei 'restarting|unhealthy' | awk '{print $1}' | paste -sd, -)"
"#;

async fn layer_l4(ip: Ipv4Addr, ssh_user: &str) -> LayerOutcome {
    let dest = format!("{ssh_user}@{ip}");
    let result = timeout(
        SSH_TOTAL_TIMEOUT,
        tokio::process::Command::new("ssh")
            .args([
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=15",
                "-o",
                "StrictHostKeyChecking=accept-new",
                &dest,
                REMOTE_SCRIPT,
            ])
            .kill_on_drop(true)
            .output(),
    )
    .await;

    let out = match result {
        Err(_) => {
            return LayerOutcome::new(
                "L4",
                "on-host",
                CheckStatus::Fail,
                format!("ssh {dest} hit the {}s overall budget", SSH_TOTAL_TIMEOUT.as_secs()),
            )
            .with_next("sshd may be wedged after all — re-run and watch L2".into());
        }
        Ok(Err(e)) => {
            return LayerOutcome::new(
                "L4",
                "on-host",
                CheckStatus::Fail,
                format!("could not spawn ssh: {e}"),
            )
            .with_next("install an OpenSSH client on this laptop".into());
        }
        Ok(Ok(out)) => out,
    };

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let next = if stderr.contains("IDENTIFICATION HAS CHANGED")
            || stderr.contains("Host key verification failed")
        {
            format!(
                "stale known_hosts for a moved IP — `ssh-keygen -R {ip}` then re-run \
                 (DHCP reuse after relocation makes this common)"
            )
        } else if stderr.contains("Permission denied") {
            format!("key auth refused for {dest} — pass --ssh-user or check the laptop's key")
        } else {
            "run the same ssh by hand to see the full error".into()
        };
        let first_err = stderr.lines().last().unwrap_or_default().trim().to_string();
        return LayerOutcome::new(
            "L4",
            "on-host",
            CheckStatus::Fail,
            format!("ssh exited {}: {first_err}", out.status.code().unwrap_or(-1)),
        )
        .with_next(next);
    }

    let checks = parse_remote_report(&String::from_utf8_lossy(&out.stdout));
    let failed = checks.iter().filter(|c| c.status == CheckStatus::Fail).count();
    let mut layer = LayerOutcome::new(
        "L4",
        "on-host",
        if failed == 0 {
            CheckStatus::Pass
        } else {
            CheckStatus::Fail
        },
        format!("ssh ok — {} checks, {failed} failing", checks.len()),
    );
    layer.checks = checks;
    layer
}

/// Parse the GOAT:<KEY>: lines from the remote script into per-check
/// verdicts. Pure — unit-tested against captured output shapes.
fn parse_remote_report(stdout: &str) -> Vec<RemoteCheck> {
    let mut kv: BTreeMap<&str, &str> = BTreeMap::new();
    for line in stdout.lines() {
        if let Some(rest) = line.trim().strip_prefix("GOAT:") {
            if let Some((k, v)) = rest.split_once(':') {
                kv.insert(k, v.trim());
            }
        }
    }
    let get = |k: &str| kv.get(k).copied().unwrap_or_default();
    let mut checks = Vec::new();

    // clock — these hosts have NO RTC battery: the clock starts at 1970
    // and chrony-presync must step it before NTS/TLS can work at all.
    let date = get("DATE");
    let year: u16 = date.get(..4).and_then(|y| y.parse().ok()).unwrap_or(0);
    let uptime = get("UPTIME");
    if year >= 2020 {
        checks.push(RemoteCheck {
            name: "clock",
            status: CheckStatus::Pass,
            detail: format!("{date} · {uptime}"),
            next: None,
        });
    } else {
        checks.push(RemoteCheck {
            name: "clock",
            status: CheckStatus::Fail,
            detail: format!("remote clock reads `{date}` — never stepped (no RTC battery)"),
            next: Some(
                "chrony-presync must step the clock before NTS/TLS works — \
                 `systemctl status chrony-presync chronyd`"
                    .into(),
            ),
        });
    }

    // chrony-presync sentinel.
    if get("PRESYNC") == "yes" {
        checks.push(RemoteCheck {
            name: "chrony-presync",
            status: CheckStatus::Pass,
            detail: "/run/chrony-presync.done present".into(),
            next: None,
        });
    } else {
        checks.push(RemoteCheck {
            name: "chrony-presync",
            status: CheckStatus::Fail,
            detail: "/run/chrony-presync.done missing".into(),
            next: Some(
                "pre-step never completed this boot — `systemctl status chrony-presync` \
                 (TLS-dependent services will fail until the clock steps)"
                    .into(),
            ),
        });
    }

    // chrony tracking.
    let leap = get("CHRONY");
    checks.push(match leap {
        "Normal" => RemoteCheck {
            name: "chrony",
            status: CheckStatus::Pass,
            detail: "leap status Normal".into(),
            next: None,
        },
        "" => RemoteCheck {
            name: "chrony",
            status: CheckStatus::Fail,
            detail: "chronyc tracking unavailable".into(),
            next: Some("`systemctl status chronyd` on the host".into()),
        },
        other => RemoteCheck {
            name: "chrony",
            status: CheckStatus::Fail,
            detail: format!("leap status `{other}`"),
            next: Some("clock not synchronised — check NTS sources once WAN is up".into()),
        },
    });

    // default route.
    let defroute = get("DEFROUTE");
    checks.push(if defroute.is_empty() {
        RemoteCheck {
            name: "default-route",
            status: CheckStatus::Fail,
            detail: "no default route".into(),
            next: Some("travel-router uplink/DHCP on the host — `ip route` / `networkctl`".into()),
        }
    } else {
        RemoteCheck {
            name: "default-route",
            status: CheckStatus::Pass,
            detail: defroute.to_string(),
            next: None,
        }
    });

    // WAN reachability.
    checks.push(if get("WAN") == "yes" {
        RemoteCheck {
            name: "wan",
            status: CheckStatus::Pass,
            detail: "https://1.1.1.1 reachable".into(),
            next: None,
        }
    } else {
        RemoteCheck {
            name: "wan",
            status: CheckStatus::Fail,
            detail: "cannot reach https://1.1.1.1 from the host".into(),
            next: Some(
                "host LAN is up but its uplink isn't — check the travel router's WAN side"
                    .into(),
            ),
        }
    });

    // netbird.
    let mgmt = get("NETBIRD_MGMT");
    let signal = get("NETBIRD_SIGNAL");
    let nb_ok = mgmt.contains("Connected") && signal.contains("Connected");
    checks.push(if nb_ok {
        RemoteCheck {
            name: "netbird",
            status: CheckStatus::Pass,
            detail: format!("management {mgmt} · signal {signal}"),
            next: None,
        }
    } else {
        RemoteCheck {
            name: "netbird",
            status: CheckStatus::Fail,
            detail: if mgmt.is_empty() && signal.is_empty() {
                "netbird status unavailable".into()
            } else {
                format!("management `{mgmt}` · signal `{signal}`")
            },
            next: Some(
                "mesh control plane down (existing P2P tunnels survive this) — usually \
                 follows a WAN or clock failure above"
                    .into(),
            ),
        }
    });

    // docker.
    let running: u32 = get("DOCKER_RUNNING").parse().unwrap_or(0);
    let bad = get("DOCKER_BAD");
    checks.push(if running > 0 && bad.is_empty() {
        RemoteCheck {
            name: "docker",
            status: CheckStatus::Pass,
            detail: format!("{running} containers running, none restarting/unhealthy"),
            next: None,
        }
    } else if running == 0 {
        RemoteCheck {
            name: "docker",
            status: CheckStatus::Fail,
            detail: "no containers running".into(),
            next: Some(
                "docker may be blocked on the LUKS-encrypted volume after cold boot — \
                 unlock, then `systemctl status docker`"
                    .into(),
            ),
        }
    } else {
        RemoteCheck {
            name: "docker",
            status: CheckStatus::Fail,
            detail: format!("{running} running; restarting/unhealthy: {bad}"),
            next: Some("`docker logs <name>` on the flagged containers".into()),
        }
    });

    checks
}

// ---- tests -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    const SYN_MAC: &str = "02:00:00:aa:e5:26"; // opsec-allow: synthetic test MAC
    const SYN_MAC_2: &str = "02:00:00:00:00:01"; // opsec-allow: synthetic test MAC

    #[test]
    fn normalize_mac_handles_separators_and_padding() {
        assert_eq!(
            normalize_mac("02:00:00:AA:E5:26").as_deref(), // opsec-allow: synthetic test MAC
            Some("02:00:00:aa:e5:26") // opsec-allow: synthetic test MAC
        );
        assert_eq!(
            normalize_mac("02-00-00-aa-e4-85").as_deref(), // opsec-allow: synthetic test MAC
            Some("02:00:00:aa:e4:85") // opsec-allow: synthetic test MAC
        );
        // macOS `arp -a` drops leading zeros.
        assert_eq!(
            normalize_mac("0:1a:2:b:5:6").as_deref(),
            Some("00:1a:02:0b:05:06") // opsec-allow: synthetic test MAC
        );
        assert_eq!(normalize_mac("(incomplete)"), None);
        assert_eq!(normalize_mac("02:00:00:aa:e5"), None);
        assert_eq!(normalize_mac("zz:3a:e2:a2:e5:26"), None);
    }

    #[test]
    fn parse_ip_addr_output_linux() {
        let out = "\
1: lo: <LOOPBACK,UP,LOWER_UP> mtu 65536 qdisc noqueue state UNKNOWN group default qlen 1000
    inet 127.0.0.1/8 scope host lo
       valid_lft forever preferred_lft forever
2: enp1s0: <BROADCAST,MULTICAST,UP,LOWER_UP> mtu 1500 qdisc fq_codel state UP group default qlen 1000
    inet 192.168.8.171/24 brd 192.168.8.255 scope global dynamic noprefixroute enp1s0
       valid_lft 85090sec preferred_lft 85090sec
";
        let nets = parse_ip_addr_output(out);
        assert_eq!(nets.len(), 2);
        assert_eq!(nets[1].iface, "enp1s0");
        assert_eq!(nets[1].ip, Ipv4Addr::new(192, 168, 8, 171));
        assert_eq!(nets[1].prefix, 24);
        // The loopback entry parses but is filtered by is_sweepable.
        assert!(!is_sweepable(nets[0].ip));
        assert!(is_sweepable(nets[1].ip));
    }

    #[test]
    fn is_sweepable_excludes_mesh_overlay_and_link_local() {
        // netbird mesh (CGNAT 100.64/10) — no ARP, not the physical LAN.
        assert!(!is_sweepable(Ipv4Addr::new(100, 64, 85, 149)));
        assert!(!is_sweepable(Ipv4Addr::new(100, 127, 0, 1)));
        assert!(is_sweepable(Ipv4Addr::new(100, 63, 0, 1)));
        assert!(is_sweepable(Ipv4Addr::new(100, 128, 0, 1)));
        assert!(!is_sweepable(Ipv4Addr::new(169, 254, 1, 1)));
        assert!(is_sweepable(Ipv4Addr::new(192, 168, 8, 50)));
        assert!(is_sweepable(Ipv4Addr::new(172, 20, 10, 2)));
    }

    #[test]
    fn parse_ifconfig_output_macos_hex_mask() {
        let out = "\
lo0: flags=8049<UP,LOOPBACK,RUNNING,MULTICAST> mtu 16384
\tinet 127.0.0.1 netmask 0xff000000
en10: flags=8863<UP,BROADCAST,SMART,RUNNING,SIMPLEX,MULTICAST> mtu 1500
\tinet 192.168.8.50 netmask 0xffffff00 broadcast 192.168.8.255
";
        let nets = parse_ifconfig_output(out);
        assert_eq!(nets.len(), 2);
        assert_eq!(nets[1].iface, "en10");
        assert_eq!(nets[1].ip, Ipv4Addr::new(192, 168, 8, 50));
        assert_eq!(nets[1].prefix, 24);
        assert_eq!(nets[0].prefix, 8);
    }

    #[test]
    fn parse_ifconfig_output_net_tools_dotted_mask() {
        let out = "\
enp1s0: flags=4163<UP,BROADCAST,RUNNING,MULTICAST>  mtu 1500
        inet 192.168.8.171  netmask 255.255.255.0  broadcast 192.168.8.255
";
        let nets = parse_ifconfig_output(out);
        assert_eq!(nets.len(), 1);
        assert_eq!(nets[0].prefix, 24);
    }

    #[test]
    fn parse_ipconfig_output_windows() {
        let out = "\
Windows IP Configuration

Ethernet adapter Ethernet:

   Connection-specific DNS Suffix  . : lan
   IPv4 Address. . . . . . . . . . . : 192.168.8.99(Preferred)
   Subnet Mask . . . . . . . . . . . : 255.255.255.0
   Default Gateway . . . . . . . . . : 192.168.8.1
";
        let nets = parse_ipconfig_output(out);
        assert_eq!(nets.len(), 1);
        assert_eq!(nets[0].ip, Ipv4Addr::new(192, 168, 8, 99));
        assert_eq!(nets[0].prefix, 24);
        assert!(nets[0].iface.contains("Ethernet"));
    }

    #[test]
    fn parse_proc_net_arp_skips_incomplete() {
        let body = "\
IP address       HW type     Flags       HW address            Mask     Device
192.168.8.171    0x1         0x2         {M}     *        enp1s0
192.168.8.40     0x1         0x0         00:00:00:00:00:00     *        enp1s0
"
        .replace("{M}", SYN_MAC);
        let entries = parse_proc_net_arp(&body);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].ip, Ipv4Addr::new(192, 168, 8, 171));
        assert_eq!(entries[0].mac, "02:00:00:aa:e5:26"); // opsec-allow: synthetic test MAC
    }

    #[test]
    fn parse_arp_a_output_macos() {
        let out = "\
? (192.168.8.1) at 30:de:4b:a:b:1 on en10 ifscope [ethernet]
? (192.168.8.171) at {M} on en10 ifscope [ethernet]
? (192.168.8.40) at (incomplete) on en10 ifscope [ethernet]
"
        .replace("{M}", SYN_MAC);
        let entries = parse_arp_a_output(&out);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].mac, "30:de:4b:0a:0b:01"); // opsec-allow: synthetic test MAC
        assert_eq!(entries[1].ip, Ipv4Addr::new(192, 168, 8, 171));
    }

    #[test]
    fn parse_arp_a_output_windows() {
        let out = "\
Interface: 192.168.8.99 --- 0x10
  Internet Address      Physical Address      Type
  192.168.8.171         {M}     dynamic
  192.168.8.255         ff-ff-ff-ff-ff-ff     static
"
        .replace("{M}", &SYN_MAC.replace(':', "-"));
        let entries = parse_arp_a_output(&out);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].ip, Ipv4Addr::new(192, 168, 8, 171));
        assert_eq!(entries[0].mac, "02:00:00:aa:e5:26"); // opsec-allow: synthetic test MAC
    }

    #[test]
    fn ip_in_net_prefix_math() {
        let net = Ipv4Addr::new(192, 168, 8, 50);
        assert!(ip_in_net(Ipv4Addr::new(192, 168, 8, 171), net, 24));
        assert!(!ip_in_net(Ipv4Addr::new(192, 168, 9, 171), net, 24));
        assert!(ip_in_net(Ipv4Addr::new(192, 168, 9, 171), net, 16));
        // The real failure mode: hotspot vs travel-router LAN.
        assert!(!ip_in_net(
            Ipv4Addr::new(192, 168, 8, 171),
            Ipv4Addr::new(172, 20, 10, 3),
            28
        ));
    }

    #[test]
    fn parse_netmask_variants() {
        assert_eq!(parse_netmask("0xffffff00"), Some(24));
        assert_eq!(parse_netmask("255.255.255.0"), Some(24));
        assert_eq!(parse_netmask("255.255.0.0"), Some(16));
        assert_eq!(parse_netmask("255.0.255.0"), None); // non-contiguous
        assert_eq!(parse_netmask("garbage"), None);
    }

    #[test]
    fn embedded_registry_macs_are_normalized() {
        for h in embedded_registry() {
            assert_eq!(normalize_mac(&h.mac).as_deref(), Some(h.mac.as_str()));
        }
    }

    #[test]
    fn load_registry_reads_override_file() {
        let td = tempfile::TempDir::new().unwrap();
        let body = "hosts:\n\
             \x20 - name: mgmt-a-01\n\
             \x20   mac: \"{U}\"\n\
             \x20   last_lan_ip: 10.0.0.5\n\
             \x20   mesh_ip: 100.127.1.5\n\
             \x20 - name: mgmt-b-01\n\
             \x20   mac: {D}\n\
             \x20   last_lan_ip: 192.168.8.9\n\
             \x20   mesh_ip: 100.127.1.9\n\
             \x20   ssh_user: ops\n\
             \x20   goatnet: lab\n"
            .replace("{U}", &SYN_MAC.to_uppercase())
            .replace("{D}", &SYN_MAC_2.replace(':', "-"));
        std::fs::write(td.path().join("lan-hosts.yaml"), body).unwrap();
        let hosts = load_registry(td.path()).unwrap();
        assert_eq!(hosts.len(), 2);
        let a = hosts.iter().find(|h| h.name == "mgmt-a-01").unwrap();
        assert_eq!(a.last_lan_ip, Ipv4Addr::new(10, 0, 0, 5));
        assert_eq!(a.mac, SYN_MAC); // normalized at load
        assert_eq!(a.ssh_user, ""); // undeclared: the context's mgmt_ssh_user applies
        let b = hosts.iter().find(|h| h.name == "mgmt-b-01").unwrap();
        assert_eq!(b.mac, SYN_MAC_2);
        assert_eq!(b.ssh_user, "ops");
    }

    #[test]
    fn load_registry_rejects_malformed_mac() {
        let td = tempfile::TempDir::new().unwrap();
        std::fs::write(
            td.path().join("lan-hosts.yaml"),
            "hosts:\n\
             \x20 - name: bad\n\
             \x20   mac: not-a-mac\n\
             \x20   last_lan_ip: 10.0.0.5\n\
             \x20   mesh_ip: 100.127.0.5\n",
        )
        .unwrap();
        let err = load_registry(td.path()).unwrap_err().to_string();
        assert!(err.contains("malformed mac"), "{err}");
    }

    #[test]
    fn load_registry_without_override_is_empty() {
        let td = tempfile::TempDir::new().unwrap();
        let hosts = load_registry(td.path()).unwrap();
        assert!(hosts.is_empty());
    }

    #[test]
    fn looks_like_prometheus_accepts_exposition() {
        assert!(looks_like_prometheus(
            "# HELP process_cpu_seconds_total Total CPU\n\
             # TYPE process_cpu_seconds_total counter\n\
             process_cpu_seconds_total 12.3\n"
        ));
        assert!(looks_like_prometheus("up 1\n"));
        assert!(!looks_like_prometheus("<html><body>nope</body></html>"));
        assert!(!looks_like_prometheus(""));
    }

    #[test]
    fn parse_remote_report_all_healthy() {
        let stdout = "\
GOAT:DATE:2026-06-12T19:02:11Z
GOAT:UPTIME:19:02:11 up 2:10, 1 user, load average: 0.31, 0.40, 0.38
GOAT:PRESYNC:yes
GOAT:CHRONY:Normal
GOAT:DEFROUTE:default via 192.168.8.1 dev enp1s0 proto dhcp metric 100
GOAT:WAN:yes
GOAT:NETBIRD_MGMT:Connected
GOAT:NETBIRD_SIGNAL:Connected
GOAT:DOCKER_RUNNING:24
GOAT:DOCKER_BAD:
";
        let checks = parse_remote_report(stdout);
        assert_eq!(checks.len(), 7);
        assert!(checks.iter().all(|c| c.status == CheckStatus::Pass), "{checks:?}");
    }

    #[test]
    fn parse_remote_report_1970_clock_and_no_presync() {
        let stdout = "\
GOAT:DATE:1970-01-01T00:03:41Z
GOAT:UPTIME:00:03:41 up 3 min
GOAT:PRESYNC:no
GOAT:CHRONY:Not synchronised
GOAT:DEFROUTE:default via 192.168.8.1 dev enp1s0
GOAT:WAN:no
GOAT:NETBIRD_MGMT:Disconnected
GOAT:NETBIRD_SIGNAL:Disconnected
GOAT:DOCKER_RUNNING:0
GOAT:DOCKER_BAD:
";
        let checks = parse_remote_report(stdout);
        let by_name = |n: &str| checks.iter().find(|c| c.name == n).unwrap();
        assert_eq!(by_name("clock").status, CheckStatus::Fail);
        assert!(by_name("clock").detail.contains("1970"));
        assert_eq!(by_name("chrony-presync").status, CheckStatus::Fail);
        assert_eq!(by_name("chrony").status, CheckStatus::Fail);
        assert_eq!(by_name("default-route").status, CheckStatus::Pass);
        assert_eq!(by_name("wan").status, CheckStatus::Fail);
        assert_eq!(by_name("netbird").status, CheckStatus::Fail);
        assert_eq!(by_name("docker").status, CheckStatus::Fail);
        assert!(by_name("docker").next.as_deref().unwrap().contains("LUKS"));
    }

    #[test]
    fn parse_remote_report_unhealthy_container_named() {
        let stdout = "\
GOAT:DATE:2026-06-12T19:02:11Z
GOAT:UPTIME:up 2 hours
GOAT:PRESYNC:yes
GOAT:CHRONY:Normal
GOAT:DEFROUTE:default via 192.168.8.1
GOAT:WAN:yes
GOAT:NETBIRD_MGMT:Connected
GOAT:NETBIRD_SIGNAL:Connected
GOAT:DOCKER_RUNNING:24
GOAT:DOCKER_BAD:zenoh-router,traefik
";
        let checks = parse_remote_report(stdout);
        let docker = checks.iter().find(|c| c.name == "docker").unwrap();
        assert_eq!(docker.status, CheckStatus::Fail);
        assert!(docker.detail.contains("zenoh-router,traefik"));
    }

    #[test]
    fn parse_remote_report_missing_keys_fail_closed() {
        // Empty stdout (e.g. remote shell choked) → every check FAILs,
        // none panics.
        let checks = parse_remote_report("");
        assert_eq!(checks.len(), 7);
        assert!(checks.iter().all(|c| c.status == CheckStatus::Fail));
    }

    #[test]
    fn layer_l2_states_render_distinctly() {
        let down = layer_l2(&BannerState::NoConnect("connection refused".into()));
        assert_eq!(down.status, CheckStatus::Fail);
        assert!(down.detail.contains("no TCP connect"));

        let wedged = layer_l2(&BannerState::NoBanner);
        assert_eq!(wedged.status, CheckStatus::Fail);
        assert!(wedged.detail.contains("no SSH banner"));
        assert!(wedged.next.as_deref().unwrap().contains("check L3"));

        let ok = layer_l2(&BannerState::Banner("SSH-2.0-OpenSSH_9.6p1".into()));
        assert_eq!(ok.status, CheckStatus::Pass);
        assert!(ok.detail.contains("OpenSSH"));
    }

    #[test]
    fn layer_l0_off_lan_names_the_hotspot_failure_mode() {
        // Laptop on a phone hotspot; target on the travel-router LAN.
        let nets = vec![LocalNet {
            iface: "en0".into(),
            ip: Ipv4Addr::new(172, 20, 10, 3),
            prefix: 28,
        }];
        let o = layer_l0(Ipv4Addr::new(192, 168, 8, 171), &nets, &embedded_registry());
        assert_eq!(o.status, CheckStatus::Fail);
        assert!(o.next.as_deref().unwrap().contains("phone hotspot"));
    }

    #[test]
    fn reconcile_l0_warns_when_reachable_fails_when_not() {
        // Off-LAN but reachable (mesh path) → WARN, run can still exit 0.
        assert_eq!(
            reconcile_l0(CheckStatus::Fail, CheckStatus::Pass),
            CheckStatus::Warn
        );
        // Off-LAN AND unreachable → the genuine hard failure.
        assert_eq!(
            reconcile_l0(CheckStatus::Fail, CheckStatus::Fail),
            CheckStatus::Fail
        );
        // On-LAN stays PASS regardless.
        assert_eq!(
            reconcile_l0(CheckStatus::Pass, CheckStatus::Fail),
            CheckStatus::Pass
        );
    }

    #[test]
    fn layer_l0_same_slash24_passes() {
        let nets = vec![LocalNet {
            iface: "en10".into(),
            ip: Ipv4Addr::new(192, 168, 8, 50),
            prefix: 24,
        }];
        let o = layer_l0(Ipv4Addr::new(192, 168, 8, 171), &nets, &embedded_registry());
        assert_eq!(o.status, CheckStatus::Pass);
    }

    #[test]
    fn remote_script_quotes_survive_as_one_argv() {
        // The script rides as ONE ssh argv element; it must not contain
        // an unbalanced double-quote that would break the remote shell.
        let dq = REMOTE_SCRIPT.matches('"').count();
        assert_eq!(dq % 2, 0, "unbalanced double quotes in REMOTE_SCRIPT");
        // Every output line is GOAT:-prefixed so the parser sees it.
        for key in [
            "GOAT:DATE:", "GOAT:UPTIME:", "GOAT:PRESYNC:", "GOAT:CHRONY:",
            "GOAT:DEFROUTE:", "GOAT:WAN:", "GOAT:NETBIRD_MGMT:",
            "GOAT:NETBIRD_SIGNAL:", "GOAT:DOCKER_RUNNING:", "GOAT:DOCKER_BAD:",
        ] {
            assert!(REMOTE_SCRIPT.contains(key), "script missing {key}");
        }
    }
}
