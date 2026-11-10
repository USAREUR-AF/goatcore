// `goat status` — operator-facing health synthesis. The checks run in
// parallel via tokio::join: router port reachability (ztol: TCP connect
// to the router, same pattern as doctor step 1), then a real mTLS Zenoh
// session open against that router (session, only when ztol passed;
// same probe as doctor step 3), portal liveness (GET /api/health), local
// cert expiry parsed from the profile, name-plane source, WireGuard PQC
// state, and device-wide name protection.
//
// Pretty form is one glyph line per check; --json is one object with
// every verdict so wrapper scripts can detect partial-down cases.

mod session;

use crate::i18n::tr2;
use crate::profile::Profile;
use clap::Args;
use serde::Serialize;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::timeout;

const TCP_PROBE_TIMEOUT: Duration = Duration::from_millis(1500);
const PORTAL_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Args, Debug)]
pub struct StatusArgs {
    /// Emit one JSON object instead of pretty glyph lines.
    #[arg(long)]
    pub json: bool,
}

// Shared with `goat heal` (heal = these probes → actuate → re-probe),
// hence the pub(crate) surface on the verdict types + probe helpers.
#[derive(Debug, Serialize)]
pub(crate) struct Verdicts {
    pub(crate) ztol: Verdict,
    pub(crate) portal: Verdict,
    pub(crate) cert: Verdict,
    /// Name-plane verdict (ADR 1082): live mesh DNS vs signed-snapshot
    /// fallback (labeled with serial + age) vs unavailable. ok=true ONLY
    /// when no fallback was needed — running on snapshot names is a real
    /// degradation the operator must see (never a false green).
    pub(crate) names: Verdict,
    /// WireGuard PQC (Rosenpass) state read from the local netbird
    /// daemon. A state line, not a health gate: OFF renders neutral
    /// (sites without rosenpass must not exit non-zero), and an
    /// unreachable netbird daemon reads as unknown (the ztol line
    /// already owns mesh-down).
    pub(crate) pqc: Verdict,
    /// Device-wide name-fallback protection (ADR 1082 unification):
    /// goat-clientd store present → forwarder answering → OS fronted.
    /// ok=false ONLY for the actionable broken tier (store present but
    /// the forwarder is not answering); absent-daemon and not-fronted
    /// are neutral states the text carries.
    pub(crate) device: Verdict,
    /// Real mTLS Zenoh session open + close against the router (#1399).
    /// ztol only proves the router port accepts TCP; a wedged router
    /// or a rejected cert passes ztol and fails here. Neutral
    /// ("not attempted") when ztol failed, so off-mesh stays one red line.
    pub(crate) session: Verdict,
}

impl Verdicts {
    pub(crate) fn any_fail(&self) -> bool {
        !self.ztol.ok
            || !self.portal.ok
            || !self.cert.ok
            || !self.names.ok
            || !self.pqc.ok
            || !self.device.ok
            || !self.session.ok
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct Verdict {
    #[serde(rename = "ok")]
    pub(crate) ok: bool,
    /// One-line summary — canonical English, serialized into `--json` so
    /// the machine surface stays byte-stable across locales.
    pub(crate) summary: String,
    /// Active-locale render of `summary` for the pretty glyph line; not
    /// serialized (the `--json` surface stays English by rule).
    #[serde(skip)]
    pub(crate) summary_loc: String,
    /// Neutral state (rendered `[–]`): true for lines that report a
    /// state rather than pass/fail — PQC off, no daemon installed. ok
    /// stays true so wrapper scripts keying on the exit code are
    /// unaffected. Not serialized; the JSON summary text carries it.
    #[serde(skip)]
    pub(crate) neutral: bool,
}

/// Build a `Verdict` from a `tr2!` `(english, localized)` summary pair:
/// the English half lands in the serialized `--json` field, the localized
/// half drives the pretty render. Keeps the render boundary explicit at
/// every probe call site.
fn verdict(ok: bool, summary: (String, String)) -> Verdict {
    Verdict {
        ok,
        summary: summary.0,
        summary_loc: summary.1,
        neutral: false,
    }
}

/// A neutral state line: `[–]` glyph, ok=true (never fails the exit code).
fn verdict_neutral(summary: (String, String)) -> Verdict {
    Verdict {
        ok: true,
        summary: summary.0,
        summary_loc: summary.1,
        neutral: true,
    }
}

/// The profile's `router_endpoint` (the ZTOL probe target). Shared with
/// `goat heal`'s recovery poll loop.
pub(crate) fn router_endpoint(profile: &Profile) -> anyhow::Result<String> {
    let toml = std::fs::read_to_string(profile.profile_toml_path())?;
    extract_toml_string(&toml, "router_endpoint")
        .ok_or_else(|| anyhow::anyhow!("profile.toml missing router_endpoint"))
}

/// Run the status probes in parallel and return the verdicts.
/// Errors only on a missing/unreadable profile — probe failures land in
/// the verdicts, never short-circuit.
pub(crate) async fn gather(ctx: &super::Ctx) -> anyhow::Result<Verdicts> {
    ctx.profile.require_initialized()?;

    // Read endpoints once for the ZTOL probe; portal client builds
    // separately via portal_client().
    let router = router_endpoint(&ctx.profile)?;
    // A context without a portal key file still gets every other row;
    // the portal row then reads the endpoint itself (probe_portal_any).
    let mut portal = ctx
        .profile
        .portal_client()
        .unwrap_or_else(|_| panoscope_core::portal::PortalClient::new(String::new(), String::new()));

    // Name-resolution pass (ADR 1082): live DNS first, signed-snapshot
    // fallback second, per target. The outcome feeds (a) the `names`
    // verdict and (b) the probes — a snapshot-resolved target is probed
    // at its resolved IP so the fabric stays reachable when mesh DNS is
    // down, honestly labeled.
    let router_hp = parse_zenoh_locator(&router);
    let portal_hp = portal_host_port(&ctx.profile);
    let (router_res, portal_res) = tokio::join!(
        resolve_opt(&ctx.profile, router_hp.as_ref()),
        resolve_opt(&ctx.profile, portal_hp.as_ref()),
    );
    let names = names_verdict(&[&router_res, &portal_res]);
    // Pre-resolved addr ONLY on fallback (snapshot or observed) — live
    // resolution keeps the probe's own OS-resolver behavior byte-identical.
    let router_addr = match &router_res {
        Some(Ok(r)) if r.source.fallback_note().is_some() => Some(r.addr),
        _ => None,
    };
    let mut portal_fallback: Option<(String, std::net::SocketAddr)> = None;
    if let (Some((host, _)), Some(Ok(res))) = (portal_hp.as_ref(), portal_res.as_ref()) {
        // Any fallback source (snapshot OR observed) needs the socket
        // override — live answers resolve normally inside reqwest.
        if res.source.fallback_note().is_some() {
            portal = portal.with_resolve(host, res.addr);
            portal_fallback = Some((host.clone(), res.addr));
        }
    }

    let (cert_path, key_path, ca_path) = (
        ctx.profile.cert_path(),
        ctx.profile.key_path(),
        ctx.profile.ca_roots_path(),
    );
    let material = crate::probe::SessionMaterial {
        cert: &cert_path,
        key: &key_path,
        ca_roots: &ca_path,
    };
    // Session runs after ztol in the same join arm: it needs the ztol
    // verdict, and off mesh it must not burn its timeout.
    let reach = async {
        let ztol = probe_ztol_at(&router, router_addr).await;
        let session = session::probe_session(&router, &material, router_addr, ztol.ok).await;
        (ztol, session)
    };
    let ((ztol, session), portal_v, cert, pqc, device) = tokio::join!(
        reach,
        probe_portal_any(&ctx.profile, &portal, portal_fallback),
        probe_cert(&ctx.profile),
        probe_pqc(),
        probe_device(),
    );

    Ok(Verdicts {
        ztol,
        portal: portal_v,
        cert,
        names,
        pqc,
        device,
        session,
    })
}

// --- WireGuard PQC (Rosenpass) state — read from the local netbird daemon ---

/// Budget for `netbird status --json` (local IPC round-trip).
const PQC_PROBE_TIMEOUT: Duration = Duration::from_secs(3);

async fn probe_pqc() -> Verdict {
    let out = timeout(
        PQC_PROBE_TIMEOUT,
        tokio::process::Command::new("netbird")
            .args(["status", "--json"])
            .output(),
    )
    .await;
    let raw = match out {
        Ok(Ok(o)) if o.status.success() => o.stdout,
        Ok(Ok(_)) | Ok(Err(_)) => {
            return verdict_neutral(tr2!(
                "status-pqc-unknown",
                reason = "netbird daemon unreachable".to_string()
            ))
        }
        Err(_) => {
            return verdict_neutral(tr2!(
                "status-pqc-unknown",
                reason = format!("netbird status gave no answer in {}s", PQC_PROBE_TIMEOUT.as_secs())
            ))
        }
    };
    match parse_pqc(&raw) {
        Some(p) if p.on => verdict(
            true,
            tr2!(
                "status-pqc-on",
                qr = p.qr_peers.to_string(),
                connected = p.connected_peers.to_string()
            ),
        ),
        Some(_) => verdict_neutral(tr2!("status-pqc-off")),
        None => verdict_neutral(tr2!(
            "status-pqc-unknown",
            reason = "netbird status output unparseable".to_string()
        )),
    }
}

#[derive(Debug, PartialEq)]
pub(crate) struct PqcState {
    pub(crate) on: bool,
    pub(crate) qr_peers: usize,
    pub(crate) connected_peers: usize,
}

/// Parse the PQC-relevant slice of `netbird status --json`: the global
/// `quantumResistance` flag plus per-connected-peer counts. Pure for
/// testability.
pub(crate) fn parse_pqc(raw: &[u8]) -> Option<PqcState> {
    let v: serde_json::Value = serde_json::from_slice(raw).ok()?;
    let on = v.get("quantumResistance")?.as_bool()?;
    let details = v
        .get("peers")
        .and_then(|p| p.get("details"))
        .and_then(|d| d.as_array());
    let (mut qr, mut connected) = (0usize, 0usize);
    if let Some(details) = details {
        for peer in details {
            let is_connected = peer
                .get("status")
                .and_then(|s| s.as_str())
                .map(|s| s.eq_ignore_ascii_case("connected"))
                .unwrap_or(false);
            if !is_connected {
                continue;
            }
            connected += 1;
            if peer
                .get("quantumResistance")
                .and_then(|q| q.as_bool())
                .unwrap_or(false)
            {
                qr += 1;
            }
        }
    }
    Some(PqcState {
        on,
        qr_peers: qr,
        connected_peers: connected,
    })
}

// --- Device-wide name-fallback protection (ADR 1082 unification) ---

/// Budget for the forwarder UDP round-trip (loopback).
const FORWARDER_PROBE_TIMEOUT: Duration = Duration::from_millis(800);

/// The forwarder address `goat status` probes — the same default +
/// env override goat-clientd binds (`GOAT_NAMES_LISTEN`).
fn forwarder_addr() -> String {
    std::env::var("GOAT_NAMES_LISTEN").unwrap_or_else(|_| "127.0.0.1:53530".to_string())
}

async fn probe_device() -> Verdict {
    let now = crate::names::now_unix();
    let Some((meta, zone)) = crate::names::daemon_store_summary(now) else {
        return verdict_neutral(tr2!("status-device-none"));
    };
    let addr = forwarder_addr();
    let serial = meta.serial.to_string();
    let grade = format!("{:?}", meta.grade).to_lowercase();

    if !forwarder_answers(&addr, &zone).await {
        // The actionable broken tier: a daemon owns the store but its
        // forwarder is not answering — daemon down or names subsystem
        // wedged. Browsers may be pointed at a dead resolver.
        return verdict(
            false,
            tr2!(
                "status-device-broken",
                serial = serial.clone(),
                addr = addr.clone()
            ),
        );
    }
    match os_fronted(&zone, &addr).await {
        Some(true) => verdict(
            true,
            tr2!(
                "status-device-active",
                addr = addr.clone(),
                serial = serial.clone(),
                grade = grade.clone()
            ),
        ),
        Some(false) => verdict_neutral(tr2!(
            "status-device-not-fronted",
            addr = addr.clone(),
            serial = serial.clone(),
            grade = grade.clone()
        )),
        None => verdict_neutral(tr2!(
            "status-device-fronting-unknown",
            addr = addr.clone(),
            serial = serial.clone(),
            grade = grade.clone()
        )),
    }
}

/// True when a DNS server answers on `addr`: send a minimal A query for
/// a name under the zone and accept ANY response with a matching ID
/// (NOERROR, NXDOMAIN, SERVFAIL all mean "listening"). Dependency-free —
/// 12-byte header + one QNAME.
async fn forwarder_answers(addr: &str, zone: &str) -> bool {
    let name = format!("goat-status-probe.{zone}");
    let id: u16 = (crate::names::now_unix() as u16) | 1;
    let mut q: Vec<u8> = Vec::with_capacity(64);
    q.extend_from_slice(&id.to_be_bytes());
    q.extend_from_slice(&[0x01, 0x00]); // RD
    q.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 0]); // QD=1
    for label in name.trim_end_matches('.').split('.') {
        if label.is_empty() || label.len() > 63 {
            return false;
        }
        q.push(label.len() as u8);
        q.extend_from_slice(label.as_bytes());
    }
    q.push(0);
    q.extend_from_slice(&[0, 1, 0, 1]); // QTYPE=A QCLASS=IN

    let fut = async {
        let sock = tokio::net::UdpSocket::bind("127.0.0.1:0").await.ok()?;
        sock.connect(addr).await.ok()?;
        sock.send(&q).await.ok()?;
        let mut buf = [0u8; 512];
        let n = sock.recv(&mut buf).await.ok()?;
        Some(n >= 2 && buf[..2] == id.to_be_bytes())
    };
    matches!(timeout(FORWARDER_PROBE_TIMEOUT, fut).await, Ok(Some(true)))
}

/// Is the OS's mesh-zone resolution pointed at the forwarder? macOS:
/// parse `scutil --dns`; Linux: look for the ported server in
/// `resolvectl status`. None = cannot determine on this platform.
async fn os_fronted(zone: &str, fwd_addr: &str) -> Option<bool> {
    let (ip, port) = fwd_addr.rsplit_once(':')?;
    if cfg!(target_os = "macos") {
        let out = timeout(
            Duration::from_secs(3),
            tokio::process::Command::new("/usr/sbin/scutil")
                .arg("--dns")
                .output(),
        )
        .await
        .ok()?
        .ok()?;
        return Some(scutil_zone_fronted(
            &String::from_utf8_lossy(&out.stdout),
            zone,
            ip,
            port,
        ));
    }
    if cfg!(target_os = "linux") {
        let out = timeout(
            Duration::from_secs(3),
            tokio::process::Command::new("resolvectl").arg("status").output(),
        )
        .await
        .ok()?
        .ok()?;
        // SetDNSEx-configured servers render as ip:port in resolvectl.
        return Some(String::from_utf8_lossy(&out.stdout).contains(&format!("{ip}:{port}")));
    }
    None
}

/// Pure scutil --dns parse: does any resolver block claim `zone` with
/// nameserver `ip` on `port`?
pub(crate) fn scutil_zone_fronted(scutil: &str, zone: &str, ip: &str, port: &str) -> bool {
    for block in scutil.split("resolver #") {
        let (mut has_zone, mut has_ip, mut has_port) = (false, false, false);
        for line in block.lines() {
            let mut parts = line.splitn(2, ':');
            let key = parts.next().unwrap_or("").trim();
            let val = parts.next().unwrap_or("").trim();
            match key {
                "domain" if val == zone => has_zone = true,
                k if k.starts_with("nameserver") && val == ip => has_ip = true,
                "port" if val == port => has_port = true,
                _ => {}
            }
        }
        if has_zone && has_ip && has_port {
            return true;
        }
    }
    false
}

/// Resolve one optional (host, port) target through the fallback resolver.
/// `None` target (malformed locator / missing endpoint) stays `None` — it
/// doesn't exercise the name plane.
async fn resolve_opt(
    profile: &Profile,
    hp: Option<&(String, u16)>,
) -> Option<Result<crate::names::Resolved, crate::names::ResolveFailure>> {
    match hp {
        Some((host, port)) => Some(crate::names::resolve_host_port(profile, host, *port).await),
        None => None,
    }
}

/// The profile's portal host + port (443 default) for the resolution pass.
fn portal_host_port(profile: &Profile) -> Option<(String, u16)> {
    let toml = std::fs::read_to_string(profile.profile_toml_path()).ok()?;
    let endpoint = extract_toml_string(&toml, "portal_endpoint")?;
    parse_https_host_port(&endpoint)
}

// pub(crate): doctor reuses this for its own portal-host resolution pass.
pub(crate) fn parse_https_host_port(endpoint: &str) -> Option<(String, u16)> {
    let (default_port, rest) = if let Some(r) = endpoint.trim().strip_prefix("https://") {
        (443u16, r)
    } else if let Some(r) = endpoint.trim().strip_prefix("http://") {
        (80u16, r)
    } else {
        (443u16, endpoint.trim())
    };
    let hostport = rest.split('/').next()?.trim();
    if hostport.is_empty() {
        return None;
    }
    match hostport.rsplit_once(':') {
        Some((h, p)) if p.parse::<u16>().is_ok() => Some((h.to_string(), p.parse().unwrap())),
        _ => Some((hostport.to_string(), default_port)),
    }
}

/// Synthesize the `names` verdict from the per-target resolution outcomes.
/// Failure > snapshot > live > ip-literal: the worst outcome wins the line,
/// so degradation is never masked by a healthier sibling target.
fn names_verdict(
    outcomes: &[&Option<Result<crate::names::Resolved, crate::names::ResolveFailure>>],
) -> Verdict {
    use crate::names::{Grade, NameSource};
    let mut worst_snapshot: Option<crate::names::SnapshotMeta> = None;
    let mut worst_observed_age: Option<i64> = None;
    let mut worst_claim_age: Option<i64> = None;
    let mut any_live = false;
    for o in outcomes.iter().filter_map(|o| o.as_ref()) {
        match o {
            Err(f) => {
                return verdict(false, tr2!("status-names-failed", reason = f.to_string()));
            }
            Ok(r) => match &r.source {
                NameSource::SelfClaim { age_secs } => {
                    if worst_claim_age.map(|w| *age_secs > w).unwrap_or(true) {
                        worst_claim_age = Some(*age_secs);
                    }
                }
                NameSource::Snapshot(meta) => {
                    let stale = worst_snapshot
                        .as_ref()
                        .map(|w| meta.age_secs > w.age_secs)
                        .unwrap_or(true);
                    if stale {
                        worst_snapshot = Some(meta.clone());
                    }
                }
                NameSource::Observed { age_secs } => {
                    if worst_observed_age.map(|w| *age_secs > w).unwrap_or(true) {
                        worst_observed_age = Some(*age_secs);
                    }
                }
                NameSource::Live => any_live = true,
                NameSource::IpLiteral => {}
            },
        }
    }
    // Precedence: failure > observed (noncanonical) > snapshot > live —
    // the least-trusted source in play owns the line.
    if let Some(age) = worst_observed_age {
        return verdict(
            false,
            tr2!("status-names-observed", age = crate::names::age_human(age)),
        );
    }
    if let Some(meta) = worst_snapshot {
        // The store marker makes the §4.1 one-store-per-device contract
        // visible: "daemon store" = read from goat-clientd's shared
        // store, plain = goat-cli's own per-context cache.
        let key_pair = match (meta.grade == Grade::Aging, meta.from_daemon) {
            (true, true) => tr2!(
                "status-names-snapshot-daemon-aging",
                serial = meta.serial.to_string(),
                age = meta.age_human()
            ),
            (true, false) => tr2!(
                "status-names-snapshot-aging",
                serial = meta.serial.to_string(),
                age = meta.age_human()
            ),
            (false, true) => tr2!(
                "status-names-snapshot-daemon",
                serial = meta.serial.to_string(),
                age = meta.age_human()
            ),
            (false, false) => tr2!(
                "status-names-snapshot",
                serial = meta.serial.to_string(),
                age = meta.age_human()
            ),
        };
        return verdict(false, key_pair);
    }
    if let Some(age) = worst_claim_age {
        // Signed and authorized, but still a fallback — the operator sees
        // names are not live.
        return verdict(
            false,
            tr2!("status-names-selfclaim", age = crate::names::age_human(age)),
        );
    }
    if any_live {
        verdict(true, tr2!("status-names-live"))
    } else {
        verdict(true, tr2!("status-names-ip-only"))
    }
}

/// Pretty render: one glyph line per verdict (the localized summary).
/// `[–]` marks neutral state lines (PQC off, no daemon) — reported, not
/// judged.
pub(crate) fn print_glyph_lines(verdicts: &Verdicts) {
    for (name, v) in [
        ("ztol", &verdicts.ztol),
        ("session", &verdicts.session),
        ("portal", &verdicts.portal),
        ("cert", &verdicts.cert),
        ("names", &verdicts.names),
        ("pqc", &verdicts.pqc),
        ("device", &verdicts.device),
    ] {
        let glyph = if v.neutral {
            "[–]"
        } else if v.ok {
            "[✓]"
        } else {
            "[✗]"
        };
        println!("{glyph} {name:<7}  {}", v.summary_loc);
    }
}

pub async fn run(ctx: &super::Ctx, args: StatusArgs) -> anyhow::Result<()> {
    let verdicts = gather(ctx).await?;

    // Opportunistic name-snapshot cache refresh (ADR 1082): only at a
    // connectivity moment — names answered live AND the mesh is reachable.
    // When DNS is already down the refresh can't work; don't burn its
    // timeout on the failure path.
    if verdicts.names.ok && verdicts.ztol.ok {
        crate::names::refresh(&ctx.profile).await;
    }

    if args.json {
        println!("{}", serde_json::to_string(&verdicts).unwrap_or_default());
    } else {
        print_glyph_lines(&verdicts);
    }

    // Exit non-zero if any probe failed — wrapper scripts can branch
    // on the exit code without parsing the body. Use the generic
    // bail since this isn't a single canonical GoatError variant.
    let any_fail = verdicts.any_fail();

    // All-green flourish: pretty mode + full success only, suppressed by
    // GOAT_NO_FUN / non-TTY (the Three Rules — never on the --json surface).
    if !args.json && !any_fail {
        if let Some(line) = crate::fun::all_green_flourish() {
            println!("{line}");
        }
    }

    if any_fail {
        std::process::exit(crate::exit_codes::EXIT_GENERIC);
    }
    Ok(())
}

pub(crate) async fn probe_ztol(router_locator: &str) -> Verdict {
    probe_ztol_at(router_locator, None).await
}

/// ZTOL probe with an optional pre-resolved address (the ADR 1082 fallback
/// path: when mesh DNS is down and the snapshot answered, `addr` carries the
/// snapshot-resolved IP; the summary keeps the hostname for display).
pub(crate) async fn probe_ztol_at(router_locator: &str, addr: Option<std::net::SocketAddr>) -> Verdict {
    let (host, port) = match parse_zenoh_locator(router_locator) {
        Some(hp) => hp,
        None => {
            return verdict(
                false,
                tr2!("status-ztol-malformed", endpoint = router_locator.to_string()),
            )
        }
    };
    let connect = async {
        match addr {
            Some(a) => TcpStream::connect(a).await,
            None => TcpStream::connect((host.as_str(), port)).await,
        }
    };
    match timeout(TCP_PROBE_TIMEOUT, connect).await {
        Ok(Ok(_)) => verdict(
            true,
            tr2!("status-ztol-tcp-ok", host = host.clone(), port = port.to_string()),
        ),
        Ok(Err(e)) => verdict(
            false,
            tr2!(
                "status-ztol-refused",
                host = host.clone(),
                port = port.to_string(),
                err = e.to_string()
            ),
        ),
        Err(_) => verdict(
            false,
            tr2!(
                "status-ztol-unreachable",
                host = host.clone(),
                port = port.to_string(),
                ms = TCP_PROBE_TIMEOUT.as_millis().to_string()
            ),
        ),
    }
}

/// The portal row. A context with a portal key probes through the keyed
/// client, as every other portal call does. A context with none (a
/// goatlite founder's, which holds no portal key) still gets a real
/// liveness verdict: `/api/health` takes no bearer token, so the probe
/// runs without one and trusts the context's own CA roots beside the
/// public ones, because a goatlite's portal certificate comes from the
/// goatnet's own authority. Neutral only when the context names no portal.
async fn probe_portal_any(
    profile: &Profile,
    portal: &panoscope_core::portal::PortalClient,
    fallback: Option<(String, std::net::SocketAddr)>,
) -> Verdict {
    if portal.is_enabled() {
        return probe_portal(portal).await;
    }
    let endpoint = std::fs::read_to_string(profile.profile_toml_path())
        .ok()
        .and_then(|t| extract_toml_string(&t, "portal_endpoint"))
        .map(|e| e.trim().trim_end_matches('/').to_string())
        .filter(|e| !e.is_empty());
    let Some(endpoint) = endpoint else {
        return verdict_neutral(tr2!("status-portal-no-endpoint"));
    };
    let ca_pem = std::fs::read(profile.ca_roots_path()).ok();
    match probe_portal_keyless(&endpoint, ca_pem.as_deref(), fallback).await {
        Ok(()) => verdict(true, tr2!("status-portal-ok-no-key")),
        Err(e) => verdict(false, tr2!("status-portal-failed", err = e.clone())),
    }
}

/// `GET <endpoint>/api/health` with no bearer token. `ca_pem` adds the
/// context's CA roots (every PEM certificate in it) to the public ones;
/// `fallback` pins the host to a fallback-resolved address (ADR 1082).
async fn probe_portal_keyless(
    endpoint: &str,
    ca_pem: Option<&[u8]>,
    fallback: Option<(String, std::net::SocketAddr)>,
) -> Result<(), String> {
    let mut builder = reqwest::Client::builder()
        .timeout(PORTAL_PROBE_TIMEOUT)
        .connect_timeout(PORTAL_PROBE_TIMEOUT);
    if let Some(pem) = ca_pem {
        if let Ok(certs) = reqwest::Certificate::from_pem_bundle(pem) {
            for c in certs {
                builder = builder.add_root_certificate(c);
            }
        }
    }
    if let Some((host, addr)) = fallback.as_ref() {
        builder = builder.resolve(host, *addr);
    }
    let http = builder.build().map_err(|e| e.to_string())?;
    let url = format!("{endpoint}/api/health");
    let resp = match timeout(PORTAL_PROBE_TIMEOUT, http.get(&url).send()).await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => return Err(e.to_string()),
        Err(_) => {
            return Err(format!(
                "timed out in {}s",
                PORTAL_PROBE_TIMEOUT.as_secs()
            ))
        }
    };
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("portal returned {}", resp.status()))
    }
}

async fn probe_portal(portal: &panoscope_core::portal::PortalClient) -> Verdict {
    match timeout(PORTAL_PROBE_TIMEOUT, portal.health()).await {
        Ok(Ok(())) => verdict(true, tr2!("status-portal-ok")),
        Ok(Err(e)) => verdict(false, tr2!("status-portal-failed", err = e.to_string())),
        Err(_) => verdict(
            false,
            tr2!(
                "status-portal-timeout",
                secs = PORTAL_PROBE_TIMEOUT.as_secs().to_string()
            ),
        ),
    }
}

async fn probe_cert(profile: &Profile) -> Verdict {
    let path = profile.cert_path();
    let pem_bytes = match tokio::fs::read(&path).await {
        Ok(b) => b,
        Err(e) => {
            return verdict(false, tr2!("status-cert-read-failed", err = e.to_string()));
        }
    };
    let blob = match pem::parse(&pem_bytes) {
        Ok(b) => b,
        Err(e) => {
            return verdict(
                false,
                tr2!("status-cert-pem-parse-failed", err = e.to_string()),
            );
        }
    };
    if blob.tag() != "CERTIFICATE" {
        return verdict(
            false,
            tr2!("status-cert-wrong-tag", tag = blob.tag().to_string()),
        );
    }
    let (_, cert) = match x509_parser::parse_x509_certificate(blob.contents()) {
        Ok(c) => c,
        Err(e) => {
            return verdict(
                false,
                tr2!("status-cert-x509-parse-failed", err = e.to_string()),
            );
        }
    };
    let not_after = cert.validity().not_after.timestamp();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    if now > not_after {
        verdict(
            false,
            tr2!(
                "status-cert-expired",
                not_after = not_after.to_string(),
                now = now.to_string()
            ),
        )
    } else {
        let days_left = (not_after - now) / 86400;
        verdict(
            true,
            tr2!(
                "status-cert-valid",
                days_left = days_left.to_string(),
                not_after = not_after.to_string()
            ),
        )
    }
}

fn parse_zenoh_locator(s: &str) -> Option<(String, u16)> {
    let after_proto = s.split_once('/')?.1;
    let (host, port) = after_proto.rsplit_once(':')?;
    let port = port.parse::<u16>().ok()?;
    Some((host.to_string(), port))
}

// extract_toml_string used to be a locally hand-rolled parser with a
// prefix-collision bug (`.trim_start_matches('=')` didn't require the `=` to
// actually be present) — see crate::profile::extract_toml_field's doc
// comment. Now an alias onto the one shared, correct implementation.
use crate::profile::extract_toml_field as extract_toml_string;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locator_parses() {
        assert_eq!(
            parse_zenoh_locator("tls/100.127.0.193:7447"),
            Some(("100.127.0.193".into(), 7447))
        );
    }

    #[test]
    fn verdict_serializes_ok_false_summary() {
        let v = Verdict {
            ok: false,
            summary: "off-mesh".into(),
            // Deliberately different so the test proves it is NOT
            // serialized — the `--json` surface stays English by rule.
            summary_loc: "hors maillage".into(),
            neutral: false,
        };
        let s = serde_json::to_string(&v).unwrap();
        assert!(s.contains("\"ok\":false"));
        assert!(s.contains("\"summary\":\"off-mesh\""));
        // The localized half must not leak into `--json`.
        assert!(!s.contains("summary_loc"));
        assert!(!s.contains("hors maillage"));
    }

    #[test]
    fn en_catalog_reproduces_status_summaries_verbatim() {
        // Locks the externalized English to the pre-i18n literals, so the
        // serialized `--json` `summary` stays byte-identical across locales
        // (the EN_LOADER half of every `tr2!` pair feeds `--json`).
        use i18n_embed_fl::fl;
        let l = &*crate::i18n::EN_LOADER;
        assert_eq!(
            fl!(l, "status-ztol-tcp-ok", host = "100.127.0.1", port = "7447"),
            "router port 100.127.0.1:7447 accepts TCP (port only — see session)"
        );
        assert_eq!(
            fl!(l, "status-session-ok", ms = "42"),
            "zenoh mTLS session opened in 42ms (router accepted this cert)"
        );
        assert_eq!(
            fl!(l, "status-session-skipped"),
            "session not attempted (off mesh; see ztol)"
        );
        assert_eq!(
            fl!(l, "status-ztol-unreachable", host = "h", port = "7447", ms = "1500"),
            "off-mesh (router h:7447 unreachable in 1500ms)"
        );
        assert_eq!(fl!(l, "status-portal-ok"), "portal /api/health 2xx");
        assert_eq!(
            fl!(l, "status-cert-valid", days_left = "30", not_after = "1779969600"),
            "cert valid; expires in 30d (unix=1779969600)"
        );
        assert_eq!(
            fl!(l, "status-cert-wrong-tag", tag = "PRIVATE KEY"),
            "first PEM block is PRIVATE KEY, want CERTIFICATE"
        );
    }

    #[test]
    fn verdicts_json_carries_the_session_line_and_fails_on_it() {
        let v = |ok: bool| verdict(ok, tr2!("status-portal-ok"));
        let mut all = Verdicts {
            ztol: v(true),
            portal: v(true),
            cert: v(true),
            names: v(true),
            pqc: v(true),
            device: v(true),
            session: verdict(false, tr2!("status-session-timeout", secs = "5".to_string())),
        };
        let s = serde_json::to_string(&all).unwrap();
        assert!(
            s.contains("\"session\":{\"ok\":false,\"summary\":\"zenoh session did not open in 5s"),
            "session field missing or reshaped: {s}"
        );
        // A red session line alone fails the exit code (#1399).
        assert!(all.any_fail());
        // Off mesh: neutral skip never adds a failure of its own.
        all.session = verdict_neutral(tr2!("status-session-skipped"));
        assert!(!all.any_fail());
    }

    /// One-shot HTTP server: answers the first request with `status_line`
    /// and hands back the request head it read.
    async fn one_shot(status_line: &'static str) -> (String, tokio::task::JoinHandle<String>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        let h = tokio::spawn(async move {
            let (mut sock, _) = l.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let n = sock.read(&mut buf).await.unwrap();
            let body = "{}";
            let resp = format!(
                "{status_line}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            String::from_utf8_lossy(&buf[..n]).to_string()
        });
        (url, h)
    }

    #[tokio::test]
    async fn keyless_portal_probe_reads_health_without_a_bearer() {
        let (url, h) = one_shot("HTTP/1.1 200 OK").await;
        assert_eq!(probe_portal_keyless(&url, None, None).await, Ok(()));
        let head = h.await.unwrap();
        assert!(head.starts_with("GET /api/health "), "request: {head}");
        assert!(!head.to_ascii_lowercase().contains("authorization"), "sent a bearer: {head}");
    }

    #[tokio::test]
    async fn keyless_portal_probe_reports_a_non_2xx() {
        let (url, h) = one_shot("HTTP/1.1 503 Service Unavailable").await;
        let err = probe_portal_keyless(&url, None, None).await.unwrap_err();
        assert!(err.contains("503"), "{err}");
        h.await.unwrap();
    }

    #[test]
    fn keyless_portal_strings() {
        let v = verdict(true, tr2!("status-portal-ok-no-key"));
        assert_eq!(
            v.summary,
            "portal /api/health 2xx (this context holds no portal key: liveness only)"
        );
        let n = verdict_neutral(tr2!("status-portal-no-endpoint"));
        assert!(n.ok && n.neutral);
        assert_eq!(n.summary, "portal not probed (this context names no portal)");
    }

    #[test]
    fn verdict_pins_english_in_json_half() {
        // The `tr2!` pair behind `verdict()`: the serialized `summary`
        // (the EN_LOADER half) is invariant regardless of the active
        // locale, so `--json` stays byte-stable. The human half lives in
        // `summary_loc` and is skip-serialized.
        let v = verdict(true, tr2!("status-portal-ok"));
        assert_eq!(v.summary, "portal /api/health 2xx");
        let s = serde_json::to_string(&v).unwrap();
        assert!(s.contains("\"summary\":\"portal /api/health 2xx\""));
        assert!(!s.contains("summary_loc"));
    }

    #[test]
    fn pqc_parse_counts_connected_quantum_resistant_peers() {
        let raw = serde_json::json!({
            "quantumResistance": true,
            "peers": {"total": 3, "connected": 2, "details": [
                {"status": "Connected", "quantumResistance": true},
                {"status": "Connected", "quantumResistance": false},
                {"status": "Connecting", "quantumResistance": true},
            ]}
        })
        .to_string();
        let p = parse_pqc(raw.as_bytes()).unwrap();
        assert!(p.on);
        assert_eq!((p.qr_peers, p.connected_peers), (1, 2));

        // Global off + no details still parses (state line says OFF).
        let off = serde_json::json!({"quantumResistance": false, "peers": {}}).to_string();
        assert!(!parse_pqc(off.as_bytes()).unwrap().on);
        // Garbage → None (verdict renders unknown, never a false state).
        assert!(parse_pqc(b"not json").is_none());
        assert!(parse_pqc(b"{}").is_none());
    }

    #[test]
    fn scutil_fronting_detection() {
        // Real shape from a live capture (browser gate).
        let fronted = "resolver #5\n  domain   : netbird.zagros.example\n  nameserver[0] : 127.0.0.1\n  port     : 53531\n  flags    : Supplemental\n";
        assert!(scutil_zone_fronted(fronted, "netbird.zagros.example", "127.0.0.1", "53531"));
        // Same zone pointed at the netbird proxy → not fronted.
        let direct = "resolver #5\n  domain   : netbird.zagros.example\n  nameserver[0] : 100.127.0.252\n  port     : 53\n";
        assert!(!scutil_zone_fronted(direct, "netbird.zagros.example", "127.0.0.1", "53531"));
        // The ip+port must be claimed by the ZONE's block, not another's.
        let split = "resolver #4\n  domain   : other.zone\n  nameserver[0] : 127.0.0.1\n  port     : 53531\nresolver #5\n  domain   : netbird.zagros.example\n  nameserver[0] : 100.127.0.252\n";
        assert!(!scutil_zone_fronted(split, "netbird.zagros.example", "127.0.0.1", "53531"));
    }
}
