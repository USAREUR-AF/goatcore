// `goat health` — read the environment-health surfaces (ADR 1034/1036/1042).
// Subscribes the on-bus health signal and renders it for any participant:
// fabric weather (per-class load vs target + green/amber/red) for everyone,
// plus a producer's own footprint with `--me` / `--peer`, and the three
// `EnvHealthV1` board domains — mesh (`--domain mesh`), presence
// (`--domain presence`) and time (`--domain time`) — each rendered at the
// board-glance altitude with the honesty color plus the "why" spelled out in
// words: mesh says which links ride a relay; presence says how many peers are
// alive and whether any dropped this window; time says which host clocks are
// healthy/degraded/unattested. The richer per-token presence view stays the
// standalone `goat presence` (one-shot/`--watch`/`--subscribers-of`); this is
// the board glance of the same domain (ADR 1036 §5.1, three altitudes).
// `--domain peer` (ADR 1232) reads the faults the router we own measured on
// peers we do not own: the site grade, and per peer its name or Zenoh ID,
// clock offset, rejections per minute, whether it holds a session directly,
// and the peering. On a machine whose session carries its own Zenoh identity
// (a slot-anchor certificate pins the ZID), every domain also listens on
// ops/peer/verdict/<own zid> and prints the router's verdict sentence when one
// arrives.
//
// Reads only — the consumer half of the QoS "Reveal" pillar. Decodes the
// FabricHealthV1 / ProducerFootprintV1 protobuf with hand-written
// prost-derive structs matching
// ops/zenoh-trial-peers/fabric-health/proto/fabric_health_v1.proto, and the
// EnvHealthV1{mesh|presence|time|net|consistency|peer} envelope matching
// ops/zenoh-trial-peers/time-health/proto/env_health_v1.proto.

use crate::health_wire::{
    fmt_age, short_name, ConsistencyDetailV1, CryptoPostureV1, EnvDetail, EnvHealthV1,
    FabricHealthV1, MeshDetailV1, MeshLinkHealthV1, PeerFaultDetailV1, PeerFaultKindV1,
    PeerFaultV1, PeerVerdictV1, PresenceDetailV1, ProducerFootprintV1, TimeDetailV1,
    CONSISTENCY_KEY, CRYPTO_KEY, FOOTPRINT_PREFIX, HEALTH_KEY, HEALTH_PREFIX, MESH_KEY, NET_KEY,
    PEER_KEY, PEER_VERDICT_PREFIX, PEER_VERDICT_SCHEMA, PRESENCE_KEY, TIME_KEY,
};
use clap::Args;
use prost::Message as _;
use std::io::IsTerminal;
use std::time::{Duration, Instant};
use tokio::time::timeout;
/// One-shot wait: the meter publishes every few seconds, so a single
/// `goat health` waits at most this long for the next beat.
const ONESHOT_WAIT: Duration = Duration::from_secs(12);
/// The mesh/presence/time meters publish every ~30s — a one-shot must outlast
/// one beat.
const ENV_ONESHOT_WAIT: Duration = Duration::from_secs(35);

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum HealthDomain {
    /// Fabric load weather (ADR 1034) — the default.
    Fabric,
    /// WireGuard mesh-link health from the site's vantage host (ADR 1042).
    Mesh,
    /// Native liveliness presence — who is alive, who dropped (ADR 1040 gap 3).
    Presence,
    /// Per-host clock health rolled from the signed time attestation (ADR 1037).
    Time,
    /// Per-node connection inventory — mesh/fabric/baseline/undeclared
    /// classification against the node-class egress contract (node-net-health.md).
    Net,
    /// Evidence-based crypto posture — live-probed key-exchange group + FIPS
    /// self-attestation of the fabric router (ADR 1079).
    Crypto,
    /// Convergence: which generation of its goatnet's intent each node
    /// converged against, and what it is running (ADR 1110).
    Consistency,
    /// Faults on peers we do not own, measured at our router: clock offset,
    /// rejections per minute, direct session, peering (ADR 1232).
    Peer,
}

#[derive(Args, Debug)]
pub struct HealthArgs {
    /// Which environment-health domain to read.
    #[arg(long, value_enum, default_value = "fabric")]
    pub domain: HealthDomain,

    /// Live-refresh: keep printing each health beat as it arrives.
    #[arg(long)]
    pub watch: bool,

    /// Also show this producer's own footprint (its rate, class shares,
    /// rank, near-limit). Subscribes ops/fabric/footprint/<peer>. Fabric only
    /// (rejected with a non-fabric --domain).
    #[arg(long, value_name = "PEER")]
    pub peer: Option<String>,

    /// Shorthand for `--peer <your-own-id>`, resolved from the profile.
    #[arg(long, conflicts_with = "peer")]
    pub me: bool,

    /// One-shot only: print one JSON object for the beat — domain, scope,
    /// colour, age, sample count — or `"beat": false` with the reason when
    /// none arrived in the window. The machine contract the acceptance
    /// assay grades (docs/design/goatnet-acceptance-suite.md). Not for
    /// `--domain crypto`, whose beat has no colour. `--domain peer` adds
    /// `detail` (the site verdict and every peer row). On a machine with its
    /// own Zenoh identity, `own_zid` and `own_verdict` (the router's verdict
    /// document on ops/peer/verdict/<own zid>, or null) are added.
    #[arg(long, conflicts_with_all = ["watch", "peer", "me"])]
    pub json: bool,
}

// Wire types live in `crate::health_wire` (shared with `goat top`, ADR 1044).

pub async fn run(ctx: &super::Ctx, args: HealthArgs) -> anyhow::Result<()> {
    if args.domain != HealthDomain::Fabric && (args.peer.is_some() || args.me) {
        anyhow::bail!(
            "--peer/--me are fabric-footprint options; not applicable to a non-fabric --domain"
        );
    }

    // Resolve which producer footprint to show, if any.
    let want_peer: Option<String> = if let Some(p) = &args.peer {
        Some(p.clone())
    } else if args.me {
        Some(resolve_me(&ctx.profile)?)
    } else {
        None
    };

    if args.json && args.domain == HealthDomain::Crypto {
        anyhow::bail!("--json covers fabric, mesh, presence, time, net, consistency and peer; read crypto with `goat health --domain crypto`");
    }

    let session = super::pubsub::open_session_for(&ctx.profile).await?;
    // The router's verdict on this machine, when it has a Zenoh identity of
    // its own; listened for during whatever wait the domain does anyway.
    let own = OwnVerdict::listen(&session, own_identity(&ctx.profile, &session)).await;

    if args.json {
        // The fabric beat only: HEALTH_KEY also carries footprints and crypto.
        let fabric_key = format!("{HEALTH_PREFIX}**");
        let (key, domain) = match args.domain {
            HealthDomain::Fabric => (fabric_key.as_str(), "fabric"),
            HealthDomain::Mesh => (MESH_KEY, "mesh"),
            HealthDomain::Presence => (PRESENCE_KEY, "presence"),
            HealthDomain::Time => (TIME_KEY, "time"),
            HealthDomain::Net => (NET_KEY, "net"),
            HealthDomain::Consistency => (CONSISTENCY_KEY, "consistency"),
            HealthDomain::Peer => (PEER_KEY, "peer"),
            HealthDomain::Crypto => unreachable!("refused above"),
        };
        let out = json_one_shot(&session, key, domain).await;
        let mut out = out?;
        if let Some(own) = &own {
            let verdict = own.latest(domain == "peer" && out["beat"] == true).await;
            out["own_zid"] = serde_json::Value::String(own.zid.clone());
            out["own_verdict"] = verdict
                .map(|v| serde_json::to_value(v).unwrap_or_default())
                .unwrap_or(serde_json::Value::Null);
        }
        let _ = session.close().await;
        println!("{}", serde_json::to_string(&out).unwrap_or_default());
        return Ok(());
    }

    if args.watch {
        if let Some(own) = &own {
            own.print_as_they_arrive(&session);
        }
    }

    match args.domain {
        HealthDomain::Fabric => {
            if args.watch {
                watch_loop(&session, want_peer.as_deref()).await?;
            } else {
                one_shot(&session, want_peer.as_deref()).await?;
            }
        }
        HealthDomain::Mesh => {
            env_dispatch(
                &session,
                args.watch,
                MESH_KEY,
                "mesh",
                print_mesh,
                "no mesh-health beat seen — is goat-mesh-health running on this fabric's vantage host?",
            )
            .await?;
        }
        HealthDomain::Presence => {
            env_dispatch(
                &session,
                args.watch,
                PRESENCE_KEY,
                "presence",
                print_presence,
                "no presence-health beat seen — is goat-presence-health running on this fabric?",
            )
            .await?;
        }
        HealthDomain::Time => {
            env_dispatch(
                &session,
                args.watch,
                TIME_KEY,
                "time",
                print_time,
                "no time-health beat seen — is goat-time-health running on this fabric?",
            )
            .await?;
        }
        HealthDomain::Net => {
            env_dispatch(
                &session,
                args.watch,
                NET_KEY,
                "net",
                print_net,
                "no net-health beat seen — is goat-node-net-health running on a node of this fabric?",
            )
            .await?;
        }
        HealthDomain::Consistency => {
            env_dispatch(
                &session,
                args.watch,
                CONSISTENCY_KEY,
                "consistency",
                print_consistency,
                "no consistency beat seen — is goat-consistency-health running in --mode rollup on this goatnet? (a denied write is SILENT, F-177: check with `goat sub 'ops/consistency/report/**' --take 2`)",
            )
            .await?;
        }
        HealthDomain::Peer => {
            env_dispatch(
                &session,
                args.watch,
                PEER_KEY,
                "peer",
                print_peer,
                "no peer-health beat seen — is goat-peer-health running beside a router of this fabric?",
            )
            .await?;
        }
        HealthDomain::Crypto => {
            crypto_dispatch(
                &session,
                args.watch,
                "no crypto-posture beat seen — is goat-crypto-posture running against this fabric's router?",
            )
            .await?;
        }
    }

    if !args.watch {
        match &own {
            Some(own) => match own.latest(args.domain == HealthDomain::Peer).await {
                Some(v) => println!("\n{}", verdict_line(&own.zid, &v)),
                None if args.domain == HealthDomain::Peer => println!(
                    "\n  this machine ({}): no router verdict on {}{} in this window",
                    own.zid, PEER_VERDICT_PREFIX, own.zid
                ),
                None => {}
            },
            None if args.domain == HealthDomain::Peer => println!(
                "\n  this machine has no Zenoh identity of its own (its certificate pins no session ID),\n  so no router verdict is addressed to it"
            ),
            None => {}
        }
    }

    let _ = session.close().await;
    Ok(())
}

/// `--json`: wait one beat window for the domain's beat and summarize it.
/// Nothing heard is `"beat": false`, never a colour.
async fn json_one_shot(
    session: &zenoh::Session,
    key: &str,
    domain: &str,
) -> anyhow::Result<serde_json::Value> {
    let subscriber = session
        .declare_subscriber(key)
        .await
        .map_err(|e| anyhow::anyhow!("declare subscriber on `{key}`: {e}"))?;
    let wait = if domain == "fabric" { ONESHOT_WAIT } else { ENV_ONESHOT_WAIT };
    let deadline = Instant::now() + wait;
    let mut undecoded = 0u32;
    let mut detail: Option<serde_json::Value> = None;
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let sample = match timeout(remaining, subscriber.recv_async()).await {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => anyhow::bail!("subscription on `{key}` ended early (connection lost, not a timeout) — {e}"),
            Err(_) => break,
        };
        let (scope, ts, color, samples, window) = if domain == "fabric" {
            match FabricHealthV1::decode(sample.payload().to_bytes().as_ref()) {
                Ok(h) => (h.site, h.ts_unix_ms, h.overall_color, h.active_producers as u64, h.window_secs),
                Err(_) => {
                    undecoded += 1;
                    continue;
                }
            }
        } else {
            match decode_env(&sample, domain) {
                Some(e) => {
                    detail = env_detail_json(&e);
                    (
                        e.scope,
                        e.ts_unix_ms,
                        e.overall_color,
                        e.sample_count,
                        e.window_secs,
                    )
                }
                None => {
                    undecoded += 1;
                    continue;
                }
            }
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let mut out = serde_json::json!({
            "domain": domain,
            "beat": true,
            "key": sample.key_expr().as_str(),
            "scope": scope,
            "overall_color": color,
            "age_secs": ((now - ts).max(0)) / 1000,
            "samples": samples,
            "window_secs": window,
        });
        if let Some(d) = detail {
            out["detail"] = d;
        }
        return Ok(out);
    }
    let reason = if undecoded > 0 {
        format!("{undecoded} sample(s) arrived on `{key}` but none decoded as the {domain} beat")
    } else {
        format!("no {domain}-health beat on `{key}` in {}s", wait.as_secs())
    };
    Ok(serde_json::json!({ "domain": domain, "beat": false, "key": key, "reason": reason }))
}

async fn one_shot(session: &zenoh::Session, want_peer: Option<&str>) -> anyhow::Result<()> {
    // One subscriber over both health + footprint; dispatch by key prefix.
    let subscriber = session
        .declare_subscriber(HEALTH_KEY)
        .await
        .map_err(|e| anyhow::anyhow!("declare subscriber on `{HEALTH_KEY}`: {e}"))?;
    println!("waiting for the next fabric-health beat (up to {}s)…", ONESHOT_WAIT.as_secs());
    let deadline = Instant::now() + ONESHOT_WAIT;
    let mut health: Option<FabricHealthV1> = None;
    let mut footprint: Option<ProducerFootprintV1> = None;
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let sample = match timeout(remaining, subscriber.recv_async()).await {
            Ok(Ok(s)) => s,
            _ => break,
        };
        ingest(&sample, want_peer, &mut health, &mut footprint);
        if health.is_some() && (footprint.is_some() || want_peer.is_none()) {
            break;
        }
    }
    match health {
        Some(h) => print_health(&h),
        None => println!("no fabric-health beat seen — is goat-fabric-health running on this fabric?"),
    }
    if let Some(p) = want_peer {
        match footprint {
            Some(f) => print_footprint(&f),
            None => println!("\n(no footprint seen for `{p}` in the window — it may be idle)"),
        }
    }
    Ok(())
}

async fn watch_loop(session: &zenoh::Session, want_peer: Option<&str>) -> anyhow::Result<()> {
    let subscriber = session
        .declare_subscriber(HEALTH_KEY)
        .await
        .map_err(|e| anyhow::anyhow!("declare subscriber on `{HEALTH_KEY}`: {e}"))?;
    println!("watching fabric health (Ctrl-C to stop)…\n");
    let mut last_footprint: Option<ProducerFootprintV1> = None;
    // Distinguishes a clean Ctrl-C stop from the subscription dying —
    // before this, both broke the same loop and this function always
    // returned Ok(()), so a script driving `--watch` could not tell "the
    // operator stopped it" from "the connection died" via the exit code.
    let mut recv_error: Option<String> = None;
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            recv = subscriber.recv_async() => {
                let sample = match recv {
                    Ok(s) => s,
                    Err(e) => {
                        recv_error = Some(e.to_string());
                        break;
                    }
                };
                let mut health: Option<FabricHealthV1> = None;
                ingest(&sample, want_peer, &mut health, &mut last_footprint);
                if let Some(h) = health {
                    print_health(&h);
                    if want_peer.is_some() {
                        if let Some(f) = &last_footprint { print_footprint(f); }
                    }
                    println!();
                }
            }
        }
    }
    if let Some(e) = recv_error {
        anyhow::bail!("fabric-health watch ended (connection lost, not Ctrl-C) — {e}");
    }
    Ok(())
}

/// Decode one sample into the health or footprint slot by key prefix.
fn ingest(
    sample: &zenoh::sample::Sample,
    want_peer: Option<&str>,
    health: &mut Option<FabricHealthV1>,
    footprint: &mut Option<ProducerFootprintV1>,
) {
    let key = sample.key_expr().as_str().to_string();
    let bytes = sample.payload().to_bytes();
    if key.starts_with(HEALTH_PREFIX) {
        match FabricHealthV1::decode(bytes.as_ref()) {
            Ok(h) => *health = Some(h),
            Err(e) => eprintln!("decode FabricHealthV1 from `{key}`: {e}"),
        }
    } else if key.starts_with(FOOTPRINT_PREFIX) {
        if let Some(want) = want_peer {
            // ops/fabric/footprint/<peer>
            let peer = key.rsplit('/').next().unwrap_or("");
            if peer == want {
                match ProducerFootprintV1::decode(bytes.as_ref()) {
                    Ok(f) => *footprint = Some(f),
                    Err(e) => eprintln!("decode ProducerFootprintV1 from `{key}`: {e}"),
                }
            }
        }
    }
}

fn print_health(h: &FabricHealthV1) {
    let total_n: u64 = h.classes.iter().map(|c| c.sample_count).sum();
    // Dipstick headline — one glance, with its grounding (the counterfactual
    // lives in `n` + window: a reading is only as good as the traffic behind it).
    println!(
        "FABRIC: {}   (site {}, {} producers, {}s window, n={})",
        colorize(&h.overall_color),
        h.site,
        h.active_producers,
        h.window_secs,
        total_n,
    );
    if h.overall_color == "idle" {
        println!("  ⓘ idle = no traffic to confirm health this window — UNCONFIRMED, not \"healthy\".");
    }
    println!(
        "  {:<4} {:>10} {:>13} {:>9} {:>8}  state",
        "cls", "msgs/s", "of target", "headroom", "n"
    );
    for c in &h.classes {
        let of_target = if c.target_sample_rate > 0.0 {
            format!("{:.0}/{:.0}", c.sample_rate, c.target_sample_rate)
        } else {
            "—".to_string()
        };
        // headroom_pct < 0 is the meter's sentinel for "unknown" (idle/unmetered).
        let headroom = if c.headroom_pct < 0.0 {
            "—".to_string()
        } else {
            format!("{:.0}%", (c.headroom_pct * 100.0).max(0.0))
        };
        println!(
            "  {:<4} {:>10.0} {:>13} {:>9} {:>8}  {}",
            c.class,
            c.sample_rate,
            of_target,
            headroom,
            c.sample_count,
            colorize(&c.color),
        );
    }
}

fn print_footprint(f: &ProducerFootprintV1) {
    println!(
        "\nyour footprint — peer {} — rank #{} — {:.0} msgs/s, {:.0} B/s{}",
        f.peer,
        f.rank,
        f.sample_rate,
        f.byte_rate,
        if f.near_limit { "  ⚠ near-limit" } else { "" },
    );
    for s in &f.shares {
        println!("  {:<4} {:>12.0} msgs/s {:>14.0} B/s", s.class, s.sample_rate, s.byte_rate);
    }
}

/// Color the state word when writing to a TTY; plain otherwise (pipe-safe).
fn colorize(state: &str) -> String {
    let upper = state.to_uppercase();
    if !std::io::stdout().is_terminal() {
        return upper;
    }
    let code = match state {
        "red" => "31",
        "amber" => "33",
        "green" => "32",
        "idle" | "unmetered" => "2", // dim — honestly unconfirmed, not a false green
        // crypto-posture states (ADR 1079): PQ-on-validated-module is the only
        // green; PQ-without-FIPS is cyan (good, not the claim); classical amber;
        // unknown is dim, never a false green on missing/stale evidence.
        "fips-pq" => "32",
        "pq" => "36",
        "classical" => "33",
        "unknown" => "2",
        _ => return upper,
    };
    format!("\x1b[{code}m{upper}\x1b[0m")
}

/// Crypto-posture domain (ADR 1079): subscribe `ops/fabric/crypto/**`, decode
/// the evidence-carrying `CryptoPostureV1`, render it with freshness. Unlike
/// the mesh/time/presence domains this is not an `EnvHealthV1`, so it has its
/// own tiny dispatch rather than going through `env_dispatch`.
async fn crypto_dispatch(
    session: &zenoh::Session,
    watch: bool,
    absent_msg: &str,
) -> anyhow::Result<()> {
    let subscriber = session
        .declare_subscriber(CRYPTO_KEY)
        .await
        .map_err(|e| anyhow::anyhow!("declare subscriber on `{CRYPTO_KEY}`: {e}"))?;
    if watch {
        println!("watching crypto posture (Ctrl-C to stop)…\n");
        loop {
            tokio::select! {
                s = subscriber.recv_async() => {
                    if let Ok(sample) = s {
                        if let Some(p) = decode_crypto(&sample) {
                            print_crypto(&p);
                        }
                    } else {
                        break;
                    }
                }
                _ = tokio::signal::ctrl_c() => break,
            }
        }
        return Ok(());
    }
    println!(
        "waiting for the next crypto-posture beat (the publisher probes every ~30s; up to {}s)…",
        ENV_ONESHOT_WAIT.as_secs()
    );
    let deadline = Instant::now() + ENV_ONESHOT_WAIT;
    let mut undecoded = 0u32;
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let sample = match timeout(remaining, subscriber.recv_async()).await {
            Ok(Ok(s)) => s,
            // The subscription died. Folding this into the deadline arm used
            // to print the "no publisher" message below and exit 0.
            Ok(Err(e)) => anyhow::bail!(
                "subscription on `{CRYPTO_KEY}` ended early (connection lost, not a timeout) \
                 after {undecoded} undecodable sample(s) — {e}"
            ),
            Err(_) => break,
        };
        if let Some(p) = decode_crypto(&sample) {
            print_crypto(&p);
            return Ok(());
        }
        undecoded += 1;
    }
    if undecoded > 0 {
        // Data DID arrive — it just never decoded (see decode_crypto's
        // stderr line(s) above for why). "is the service running?" would be
        // the wrong diagnosis here: the service is running and reaching us.
        println!(
            "{undecoded} sample(s) arrived on `{CRYPTO_KEY}` but failed to decode as \
             CryptoPostureV1 (see the decode error(s) above) — this is a payload/version \
             mismatch, not an absent publisher."
        );
    } else {
        println!("{absent_msg}");
    }
    Ok(())
}

fn decode_crypto(sample: &zenoh::sample::Sample) -> Option<CryptoPostureV1> {
    let bytes = sample.payload().to_bytes();
    match CryptoPostureV1::decode(bytes.as_ref()) {
        Ok(p) => Some(p),
        Err(e) => {
            // `.ok()` used to drop this silently — a watch mode that hit a
            // bad payload just never updated again, forever, with zero
            // output naming why.
            eprintln!("decode CryptoPostureV1 from `{}`: {e}", sample.key_expr().as_str());
            None
        }
    }
}

/// Render the crypto posture from evidence. Every line is a fact the
/// publisher observed, with its freshness; nothing here is asserted.
fn print_crypto(p: &CryptoPostureV1) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    println!(
        "CRYPTO: {} (site {}) — {}",
        colorize(&p.color),
        p.site,
        p.headline
    );
    // Live key-exchange evidence.
    if p.kx_ok {
        let fips = if p.kx_group_fips { " [fips]" } else { "" };
        let pq = if p.kx_pq { " · post-quantum" } else { "" };
        println!(
            "  key exchange:  {}{}{} — live-probed {} ago",
            p.kx_group,
            fips,
            pq,
            fmt_age(now - p.kx_probed_ms)
        );
    } else {
        println!("  key exchange:  probe failed — {}", if p.kx_error.is_empty() { "(no detail)" } else { &p.kx_error });
    }
    if !p.router_version.is_empty() {
        println!("  router:        {}", p.router_version);
    }
    // Host FIPS self-attestation evidence (freshness-graded; honest label).
    if p.attest_present && p.attest_fips_marker {
        let module = if p.attest_module.is_empty() { "validated FIPS module" } else { &p.attest_module };
        let digest = if p.attest_digest.is_empty() {
            String::new()
        } else {
            format!(" ({})", short_name(&p.attest_digest))
        };
        println!(
            "  FIPS attest:   {} — self-attested {} ago{}",
            module,
            fmt_age(now - p.attest_at_ms),
            digest
        );
    } else {
        println!("  FIPS attest:   none — no FIPS self-attestation on this router");
    }
}

/// Resolve the caller's own peer id for `--me`. Best-effort from the
/// profile.toml; if no identity field is present, ask the operator to
/// pass `--peer` explicitly (cert-CN auto-resolution is a later item).
fn resolve_me(profile: &crate::profile::Profile) -> anyhow::Result<String> {
    let toml = std::fs::read_to_string(profile.profile_toml_path()).map_err(|e| {
        anyhow::anyhow!("read {}: {e}", profile.profile_toml_path().display())
    })?;
    for key in ["vendor_prefix", "vendor", "handle", "name", "peer"] {
        if let Some(v) = extract_toml_string(&toml, key) {
            if !v.is_empty() {
                return Ok(v);
            }
        }
    }
    anyhow::bail!(
        "could not resolve your own peer id from {} — pass `--peer <id>` explicitly",
        profile.profile_toml_path().display()
    )
}

// This was the one already-correct copy of this parser; now an alias onto
// crate::profile::extract_toml_field, the single shared implementation the
// other (buggy) copies were consolidated into.
use crate::profile::extract_toml_field as extract_toml_string;

// ---------------------------------------------------------------------------
// Mesh domain (ADR 1042) — decode EnvHealthV1{mesh} and say WHY in words.
// The EnvHealthV1 envelope + per-domain detail wire types live in
// `crate::health_wire` (shared with `goat top`, ADR 1044).
// ---------------------------------------------------------------------------

/// One entry point for every `EnvHealthV1` board domain (mesh/presence/time):
/// subscribe `ops/<domain>/health/**`, decode the envelope, render it with the
/// domain's print fn. `--watch` streams every beat; the one-shot waits one
/// publish cycle and reports honestly if the meter isn't running.
async fn env_dispatch(
    session: &zenoh::Session,
    watch: bool,
    key: &str,
    domain: &str,
    print_fn: fn(&EnvHealthV1),
    absent_msg: &str,
) -> anyhow::Result<()> {
    if watch {
        env_watch(session, key, domain, print_fn).await
    } else {
        env_one_shot(session, key, domain, print_fn, absent_msg).await
    }
}

async fn env_one_shot(
    session: &zenoh::Session,
    key: &str,
    domain: &str,
    print_fn: fn(&EnvHealthV1),
    absent_msg: &str,
) -> anyhow::Result<()> {
    let subscriber = session
        .declare_subscriber(key)
        .await
        .map_err(|e| anyhow::anyhow!("declare subscriber on `{key}`: {e}"))?;
    println!(
        "waiting for the next {domain}-health beat (the meter publishes every ~30s; up to {}s)…",
        ENV_ONESHOT_WAIT.as_secs()
    );
    let deadline = Instant::now() + ENV_ONESHOT_WAIT;
    let mut undecoded = 0u32;
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let sample = match timeout(remaining, subscriber.recv_async()).await {
            Ok(Ok(s)) => s,
            // The subscription died. Folding this into the deadline arm used
            // to print the "no publisher" message below and exit 0.
            Ok(Err(e)) => anyhow::bail!(
                "subscription on `{key}` ended early (connection lost, not a timeout) after \
                 {undecoded} undecodable sample(s) — {e}"
            ),
            Err(_) => break,
        };
        if let Some(env) = decode_env(&sample, domain) {
            print_fn(&env);
            return Ok(());
        }
        undecoded += 1;
    }
    if undecoded > 0 {
        // decode_env eprintln's real decode failures above (but NOT a pure
        // domain mismatch — that's debug-only), so this must not claim
        // "see above" unconditionally; the diagnosis must also not say "is
        // the service running?" when data plainly DID arrive.
        println!(
            "{undecoded} sample(s) arrived on `{key}` but none decoded as this domain's \
             EnvHealthV1 — a payload mismatch (any decode error(s) print above) or a domain \
             mismatch (re-run with -v to see which), not an absent publisher."
        );
    } else {
        println!("{absent_msg}");
    }
    Ok(())
}

async fn env_watch(
    session: &zenoh::Session,
    key: &str,
    domain: &str,
    print_fn: fn(&EnvHealthV1),
) -> anyhow::Result<()> {
    let subscriber = session
        .declare_subscriber(key)
        .await
        .map_err(|e| anyhow::anyhow!("declare subscriber on `{key}`: {e}"))?;
    println!("watching {domain} health (Ctrl-C to stop)…\n");
    let mut recv_error: Option<String> = None;
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            recv = subscriber.recv_async() => {
                let sample = match recv {
                    Ok(s) => s,
                    Err(e) => {
                        recv_error = Some(e.to_string());
                        break;
                    }
                };
                if let Some(env) = decode_env(&sample, domain) {
                    print_fn(&env);
                    println!();
                }
            }
        }
    }
    if let Some(e) = recv_error {
        anyhow::bail!("{domain}-health watch ended (connection lost, not Ctrl-C) — {e}");
    }
    Ok(())
}

fn decode_env(sample: &zenoh::sample::Sample, domain: &str) -> Option<EnvHealthV1> {
    let bytes = sample.payload().to_bytes();
    match EnvHealthV1::decode(bytes.as_ref()) {
        Ok(env) if env.domain == domain => Some(env),
        // A domain mismatch used to print nothing at all, unlike the Err
        // arm below — so env_one_shot's "see the decode error(s) above"
        // could point at a line that was never printed, when every
        // received sample turned out to be for a different domain.
        Ok(env) => {
            tracing::debug!(got = %env.domain, want = domain, key = sample.key_expr().as_str(), "EnvHealthV1 domain mismatch");
            None
        }
        Err(e) => {
            eprintln!("decode EnvHealthV1 from `{}`: {e}", sample.key_expr().as_str());
            None
        }
    }
}

/// The headline reason, derived from the same rows the color came from —
/// an amber without its "why" is half a reading.
fn mesh_reason(detail: &MeshDetailV1) -> String {
    let connected: Vec<&MeshLinkHealthV1> = detail.links.iter().filter(|l| l.connected).collect();
    if connected.is_empty() {
        return "no connected links to grade".to_string();
    }
    let relayed = connected.iter().filter(|l| !l.direct).count();
    let direct = connected.len() - relayed;
    let slow = connected
        .iter()
        .filter(|l| l.direct && l.color != "green")
        .count();
    let mut parts: Vec<String> = Vec::new();
    if relayed > 0 {
        if relayed == connected.len() {
            parts.push(format!(
                "all {relayed} connected peer{} riding relay fallback",
                if relayed == 1 { " is" } else { "s are" }
            ));
        } else {
            parts.push(format!("{relayed} of {} riding relay fallback", connected.len()));
        }
    }
    if slow > 0 {
        parts.push(format!("{slow} direct link{} with high measured latency", if slow == 1 { "" } else { "s" }));
    }
    if parts.is_empty() {
        parts.push(format!("{direct} direct P2P link{}", if direct == 1 { "" } else { "s" }));
    }
    parts.join("; ")
}

// fmt_age + short_name live in `crate::health_wire` (shared with `goat top`).

/// Colorize a state word and left-pad to a VISIBLE width, so the ANSI escape
/// bytes added by colorize() don't break column alignment.
fn pad_state(state: &str, width: usize) -> String {
    let colored = colorize(state);
    let visible = state.chars().count(); // colorize uppercases 1:1, no width change
    let pad = width.saturating_sub(visible);
    format!("{colored}{}", " ".repeat(pad))
}

/// Relay short label: "via relay-dub" from "rels://relay-dub.example.net:33073".
fn relay_label(addr: &str) -> String {
    let host = addr.trim_start_matches("rels://").trim_start_matches("rel://");
    match host.split(['.', ':']).next() {
        Some(h) if !h.is_empty() => format!("via {h}"),
        _ => "via relay".to_string(),
    }
}

fn print_mesh(env: &EnvHealthV1) {
    let Some(EnvDetail::Mesh(detail)) = &env.detail else {
        println!("MESH: {} (site {}, n={}) — no per-link detail in this beat", colorize(&env.overall_color), env.scope, env.sample_count);
        return;
    };
    println!(
        "MESH: {} — {} of {} peers connected; {}",
        colorize(&env.overall_color),
        detail.peers_connected,
        detail.peers_total,
        mesh_reason(detail),
    );
    println!(
        "  vantage {} · site {} · {}s window · n={} connected links",
        short_name(&detail.vantage_host),
        env.scope,
        env.window_secs,
        env.sample_count,
    );
    if env.overall_color == "unmetered" {
        println!("  ⓘ unmetered = the meter has no fresh netbird snapshot on the vantage host — check mesh-health-snapshot.timer there. Absence of data is never reported green.");
        return;
    }
    println!();
    println!(
        "  {:<22} {:<7} {:<26} {:>10} {:>11} {:>6}",
        "PEER", "STATE", "PATH", "LATENCY", "HANDSHAKE", "FLIPS"
    );
    for l in &detail.links {
        let path = if !l.connected {
            "offline".to_string()
        } else if l.direct {
            "direct P2P".to_string()
        } else {
            format!("relayed {}", relay_label(&l.relay_address))
        };
        let latency = if !l.connected {
            "—".to_string()
        } else if l.latency_ms > 0.0 {
            format!("{:.0} ms", l.latency_ms)
        } else {
            "unmeasured".to_string()
        };
        let flips = if l.connected { l.transition_count.to_string() } else { "—".to_string() };
        println!(
            "  {:<22} {} {:<26} {:>10} {:>11} {:>6}",
            short_name(&l.peer_fqdn),
            // STATE column: colorize, then pad on VISIBLE width (the plain
            // state word) so ANSI escapes don't skew the layout.
            pad_state(&l.color, 7),
            path,
            latency,
            fmt_age(l.handshake_age_ms),
            flips,
        );
    }
    // The footer says what the colors MEAN and what to do — the reading is
    // for operators, not for people who memorized the design doc.
    println!();
    let relayed_any = detail.links.iter().any(|l| l.connected && !l.direct);
    if relayed_any {
        println!("  amber/relayed: the link works but rides a relay server instead of a direct");
        println!("  encrypted P2P path — expect higher latency + shared relay capacity. P2P here is");
        println!("  WAN hole-punching across separate networks (not a same-LAN thing). Common causes:");
        println!("  a symmetric NAT on one side (defeats hole-punching), or the remote peer not being");
        println!("  reachable on its WireGuard UDP port (firewall / cloud security-group). Drill in");
        println!("  with `netbird status -d` and check NAT type + peer reachability.");
    }
    if detail.links.iter().any(|l| !l.connected) {
        println!("  offline peers read idle and do not grade the mesh — presence lives in panoscope.");
    }
}

// ---------------------------------------------------------------------------
// Presence domain (ADR 1040 gap 3) — decode EnvHealthV1{presence}: how many
// peers are alive, who dropped this window. The board glance of `goat presence`.
// ---------------------------------------------------------------------------

/// Headline reason for the presence reading, from the same rows the color came
/// from — alive count + recent-drop churn.
fn presence_reason(detail: &PresenceDetailV1) -> String {
    if detail.alive_total == 0 {
        return "the declared presence fleet has vanished — every token dropped".to_string();
    }
    if detail.dropped_recent > 0 {
        format!(
            "{} peer{} dropped within the last window (churn — look here)",
            detail.dropped_recent,
            if detail.dropped_recent == 1 { "" } else { "s" }
        )
    } else {
        "no drops this window".to_string()
    }
}

/// Short slot-anchor form for the board: a 32-hex UUID anchor truncates to 8
/// chars + ellipsis; a shorter vendor prefix / ops path prints whole. This is a
/// display truncation, NOT the portal alias layer (that stays `goat presence`'s
/// job) — the ADR 1039 invariant that machine paths stay UUID is untouched.
fn short_root(root: &str) -> String {
    if root.len() == 32 && root.chars().all(|c| c.is_ascii_hexdigit()) {
        format!("{}…", &root[..8])
    } else {
        root.to_string()
    }
}

fn print_presence(env: &EnvHealthV1) {
    let Some(EnvDetail::Presence(detail)) = &env.detail else {
        println!(
            "PRESENCE: {} (site {}, n={}) — no per-peer detail in this beat",
            colorize(&env.overall_color),
            env.scope,
            env.sample_count
        );
        return;
    };
    println!(
        "PRESENCE: {} — {} peer{} alive; {}",
        colorize(&env.overall_color),
        detail.alive_total,
        if detail.alive_total == 1 { "" } else { "s" },
        presence_reason(detail),
    );
    println!(
        "  site {} · {}s window · n={} token{}",
        env.scope,
        env.window_secs,
        env.sample_count,
        if env.sample_count == 1 { "" } else { "s" },
    );
    if env.overall_color == "idle" {
        println!("  ⓘ idle = nothing declares presence yet — UNCONFIRMED, not \"healthy\". Peers");
        println!("    predating the presence rollout (or lacking the liveliness ACL grant) are");
        println!("    invisible here; `goat ui` still shows traffic-inferred liveness.");
        return;
    }
    println!();
    println!("  {:<24} {:<7} {:<14} {:>10} {:>6}", "PEER", "STATE", "ROOT", "SINCE", "DROPS");
    for p in &detail.peers {
        let state = if p.alive { "alive" } else { "dropped" };
        // Time-in-state from the reading's own clock (env.ts), not the local
        // machine's — avoids cross-host skew. since=0 (unknown) → "—".
        let age = if p.since_unix_ms > 0 { env.ts_unix_ms - p.since_unix_ms } else { -1 };
        println!(
            "  {:<24} {} {:<14} {:>10} {:>6}",
            p.label,
            pad_state(state, 7),
            short_root(&p.write_root),
            fmt_age(age),
            p.drop_count,
        );
    }
    println!();
    println!("  presence is native liveliness — a dropped token is a real session close/crash,");
    println!("  not just quiet traffic. The per-token alias-rendered view is `goat presence`");
    println!("  (one-shot, `--watch` for join/leave edges, `--subscribers-of <topic>`).");
}

// ---------------------------------------------------------------------------
// Time domain (ADR 1037) — decode EnvHealthV1{time}: which host clocks are
// healthy/degraded/unattested, with offset + attestation grounding. An
// unattested host reads idle (never a false green).
// ---------------------------------------------------------------------------

/// Headline reason for the time reading — what (if anything) pulled it off green.
fn time_reason(detail: &TimeDetailV1) -> String {
    if detail.hosts.is_empty() {
        return "no host clocks to grade".to_string();
    }
    let degraded = detail.hosts.iter().filter(|h| h.color == "red").count();
    let warning = detail.hosts.iter().filter(|h| h.color == "amber").count();
    let unattested = detail.hosts.iter().filter(|h| !h.attested).count();
    let diverged = detail.hosts.iter().filter(|h| h.divergence_detected).count();
    let mut parts: Vec<String> = Vec::new();
    if degraded > 0 {
        parts.push(format!("{degraded} degraded"));
    }
    if warning > 0 {
        parts.push(format!("{warning} warning"));
    }
    if unattested > 0 {
        parts.push(format!("{unattested} unattested (idle, never green)"));
    }
    if diverged > 0 {
        parts.push(format!("{diverged} diverging from peer consensus"));
    }
    if parts.is_empty() {
        let n = detail.hosts.len();
        format!("all {n} host clock{} healthy", if n == 1 { "" } else { "s" })
    } else {
        parts.join("; ")
    }
}

fn print_time(env: &EnvHealthV1) {
    let Some(EnvDetail::Time(detail)) = &env.detail else {
        println!(
            "TIME: {} (site {}, n={}) — no per-host detail in this beat",
            colorize(&env.overall_color),
            env.scope,
            env.sample_count
        );
        return;
    };
    let n = detail.hosts.len();
    println!(
        "TIME: {} — {n} host clock{}; {}",
        colorize(&env.overall_color),
        if n == 1 { "" } else { "s" },
        time_reason(detail),
    );
    println!(
        "  site {} · {}s window · n={} host{}",
        env.scope,
        env.window_secs,
        env.sample_count,
        if env.sample_count == 1 { "" } else { "s" },
    );
    if env.overall_color == "idle" || env.overall_color == "unmetered" {
        println!(
            "  ⓘ {} = no fresh attestation behind this reading — UNCONFIRMED, never a false green.",
            env.overall_color
        );
    }
    if detail.hosts.is_empty() {
        return;
    }
    println!();
    println!(
        "  {:<16} {:<7} {:<9} {:>10} {:>4} {:>5} {:>10}",
        "HOST", "STATE", "VERDICT", "OFFSET", "STR", "GNSS", "ATTEST"
    );
    for h in &detail.hosts {
        let offset = format!("{:.1} ms", h.rms_offset_ms);
        let gnss = if h.gnss_present { "yes" } else { "—" };
        // attested→handshake-style age; unattested→the honest "no" (idle driver).
        let attest = if h.attested { fmt_age(h.attestation_age_ms) } else { "no".to_string() };
        println!(
            "  {:<16} {} {:<9} {:>10} {:>4} {:>5} {:>10}",
            short_name(&h.host_id),
            pad_state(&h.color, 7),
            h.verdict,
            offset,
            h.stratum,
            gnss,
            attest,
        );
    }
    println!();
    println!("  two watches: internal coherence (ordering/fusion — holds at the denied edge) vs");
    println!("  UTC confidence (anchor freshness). An unattested host reads idle, never green —");
    println!("  absence of a signed attestation is never reported as healthy.");
}

// ---------------------------------------------------------------------------
// Peer domain (ADR 1232) — decode EnvHealthV1{peer}: faults the router we own
// measured on peers we do not own. The reading is graded before anyone is
// blamed: when our own clock is the likelier fault the site reads amber
// (local_clock_suspect) and no peer row is at fault; when the reader has no
// log to read it is unmetered, never green.
// ---------------------------------------------------------------------------

/// How long a peer one-shot waits for this machine's verdict after the beat
/// arrived: the reader publishes both on the same tick.
const VERDICT_GRACE: Duration = Duration::from_secs(3);

/// A peer row's display name: the attribution name, else its Zenoh ID.
fn peer_label(p: &PeerFaultV1) -> &str {
    match p.attribution_name.as_deref() {
        Some(n) if !n.trim().is_empty() => n,
        _ => &p.zid,
    }
}

/// `direct_session` in words: absent is unknown, never a guess.
fn fmt_direct(direct: Option<bool>) -> &'static str {
    match direct {
        Some(true) => "yes",
        Some(false) => "no",
        None => "unknown",
    }
}

/// Signed offset in seconds: peer time minus our router's time.
fn fmt_offset_secs(s: f64) -> String {
    format!("{s:+.2} s")
}

fn peer_kind(p: &PeerFaultV1) -> &'static str {
    match PeerFaultKindV1::try_from(p.kind) {
        Ok(PeerFaultKindV1::ClockAhead) => "clock_ahead",
        _ => "unspecified",
    }
}

/// Headline reason: the reader's verdict, with its own reason when it gave one.
fn peer_reason(detail: &PeerFaultDetailV1) -> String {
    let verdict = if detail.verdict.is_empty() {
        "no verdict"
    } else {
        detail.verdict.as_str()
    };
    if detail.verdict_reason.trim().is_empty() {
        verdict.to_string()
    } else {
        format!("{verdict}: {}", detail.verdict_reason.trim())
    }
}

/// The peer detail as JSON, for `--json`: the site grade and every row, with
/// `direct_session` as yes/no/unknown, as the table prints it.
fn peer_json(detail: &PeerFaultDetailV1) -> serde_json::Value {
    let peering = (!detail.peering.is_empty()).then(|| detail.peering.clone());
    serde_json::json!({
        "verdict": detail.verdict,
        "verdict_reason": detail.verdict_reason,
        "local_clock": detail.local_clock,
        "observed_by": detail.observed_by,
        "peers_known": detail.peers_known,
        "last_line_age_ms": detail.last_line_age_ms,
        "unparsed_lines_window": detail.unparsed_lines_window,
        "peering": peering,
        "peering_rejections_per_minute": detail.peering_rejections_per_minute,
        "peering_max_offset_seconds": detail.peering_max_offset_seconds,
        "peers": detail.peers.iter().map(|p| serde_json::json!({
            "label": peer_label(p),
            "zid": p.zid,
            "name": p.attribution_name,
            "owner": p.attribution_owner,
            "kind": peer_kind(p),
            "grade": p.grade,
            "color": p.color,
            "offset_seconds": p.offset_seconds,
            "min_offset_seconds": p.min_offset_seconds,
            "max_offset_seconds": p.max_offset_seconds,
            "limit_seconds": p.limit_seconds,
            "rejections_per_minute": p.rejections_per_minute,
            "rejections_in_window": p.rejections_in_window,
            "direct_session": fmt_direct(p.direct_session),
            "peering": peering,
            "observed_by": p.observed_by,
        })).collect::<Vec<_>>(),
    })
}

/// The per-domain detail `--json` carries beside the summary. Only the peer
/// domain has one today.
fn env_detail_json(env: &EnvHealthV1) -> Option<serde_json::Value> {
    match &env.detail {
        Some(EnvDetail::Peer(d)) => Some(peer_json(d)),
        _ => None,
    }
}

/// The board glance of one peer reading, as text. Pure, so it is tested.
fn render_peer(env: &EnvHealthV1) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let Some(EnvDetail::Peer(detail)) = &env.detail else {
        let _ = writeln!(
            out,
            "PEER: {} (site {}, n={}) — no per-peer detail in this beat",
            colorize(&env.overall_color),
            env.scope,
            env.sample_count
        );
        return out;
    };
    let _ = writeln!(
        out,
        "PEER: {} — {}",
        colorize(&env.overall_color),
        peer_reason(detail)
    );
    let mut context = format!(
        "  site {} · {}s window · our clock {} · {} peer{} known",
        env.scope,
        env.window_secs,
        if detail.local_clock.is_empty() {
            "unknown"
        } else {
            &detail.local_clock
        },
        detail.peers_known,
        if detail.peers_known == 1 { "" } else { "s" },
    );
    if !detail.observed_by.is_empty() {
        let _ = write!(context, " · observed by {}", detail.observed_by);
    }
    let _ = writeln!(out, "{context}");
    if !detail.peering.is_empty() {
        let _ = writeln!(
            out,
            "  peering {}: {:.1} rejections/min, largest offset {}",
            detail.peering,
            detail.peering_rejections_per_minute,
            fmt_offset_secs(detail.peering_max_offset_seconds),
        );
    }
    match env.overall_color.as_str() {
        "unmetered" | "idle" => {
            let _ = writeln!(
                out,
                "  ⓘ {} = the reader has no router log to judge — UNCONFIRMED, never a false green.",
                env.overall_color
            );
        }
        _ if detail.verdict == "local_clock_suspect" => {
            let _ = writeln!(
                out,
                "  ⓘ our own clock is the likelier fault, so no peer is blamed; check this router's time sync first."
            );
        }
        _ => {}
    }
    if detail.unparsed_lines_window > 0 {
        let _ = writeln!(
            out,
            "  ⓘ {} rejection line(s) in the window did not parse; this reading never grades green.",
            detail.unparsed_lines_window
        );
    }
    if detail.peers.is_empty() {
        return out;
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "  PEER                             STATE     GRADE                   OFFSET  REJ/MIN DIRECT   PEERING");
    for p in &detail.peers {
        let _ = writeln!(
            out,
            "  {:<32} {} {:<19} {:>10} {:>8.1} {:<8} {}",
            peer_label(p),
            pad_state(&p.color, 9),
            p.grade,
            fmt_offset_secs(p.offset_seconds),
            p.rejections_per_minute,
            fmt_direct(p.direct_session),
            if detail.peering.is_empty() {
                "—"
            } else {
                &detail.peering
            },
        );
    }
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "  OFFSET is the peer's time minus our router's; the router replaces a timestamp more"
    );
    let _ = writeln!(
        out,
        "  than its limit ahead. A Zenoh ID names the router that stamped the sample, which can"
    );
    let _ = writeln!(
        out,
        "  sit behind the one holding the session (DIRECT no, or unknown without a sessions file)."
    );
    out
}

fn print_peer(env: &EnvHealthV1) {
    print!("{}", render_peer(env));
}

/// This machine's own Zenoh ID, when it has one: the profile's certificate
/// pins the session ID to its slot anchor, and the open session carries it
/// (the pin can be refused at open, and then the ID is random and not ours).
fn own_identity(profile: &crate::profile::Profile, session: &zenoh::Session) -> Option<String> {
    let pinned = super::pubsub::pinned_zid(profile)?;
    let live = session.zid().to_string();
    (live == pinned).then_some(live)
}

/// A verdict document, when the payload is one (schema goat.peer.verdict.v1).
fn decode_verdict(bytes: &[u8]) -> Option<PeerVerdictV1> {
    let v: PeerVerdictV1 = serde_json::from_slice(bytes).ok()?;
    (v.schema == PEER_VERDICT_SCHEMA).then_some(v)
}

/// Text from the bus, safe to print: no control characters, bounded length.
fn clean_text(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .take(400)
        .collect::<String>()
        .trim()
        .to_string()
}

/// The line `goat health` prints for this machine's verdict.
fn verdict_line(zid: &str, v: &PeerVerdictV1) -> String {
    let mut sentence = clean_text(&v.sentence);
    if sentence.is_empty() {
        sentence = if v.state == "cleared" {
            "the router reports the clock fault cleared".to_string()
        } else {
            format!(
                "the router sees your clock {:.1} s ahead of its own; the limit is {} s",
                v.offset_seconds, v.limit_seconds
            )
        };
    }
    let state = match clean_text(&v.state) {
        s if s.is_empty() => "unknown".to_string(),
        s => s,
    };
    let mut line = format!("VERDICT on this machine ({zid}, {state}): {sentence}");
    let at = clean_text(&v.observed_at);
    if !at.is_empty() {
        line.push_str(&format!(" (observed {at})"));
    }
    line
}

/// A listener on ops/peer/verdict/<own zid>, open for the whole command.
struct OwnVerdict {
    zid: String,
    key: String,
    sub: zenoh::pubsub::Subscriber<zenoh::handlers::FifoChannelHandler<zenoh::sample::Sample>>,
}

impl OwnVerdict {
    async fn listen(session: &zenoh::Session, zid: Option<String>) -> Option<Self> {
        let zid = zid?;
        let key = format!("{PEER_VERDICT_PREFIX}{zid}");
        match session.declare_subscriber(&key).await {
            Ok(sub) => Some(Self { zid, key, sub }),
            Err(e) => {
                eprintln!("listen for this machine's verdict on `{key}`: {e}");
                None
            }
        }
    }

    /// The newest verdict that arrived so far; with `grace`, wait briefly for
    /// one when none has.
    async fn latest(&self, grace: bool) -> Option<PeerVerdictV1> {
        let mut last = None;
        while let Ok(Some(s)) = self.sub.try_recv() {
            if let Some(v) = decode_verdict(s.payload().to_bytes().as_ref()) {
                last = Some(v);
            }
        }
        if last.is_none() && grace {
            if let Ok(Ok(s)) = timeout(VERDICT_GRACE, self.sub.recv_async()).await {
                last = decode_verdict(s.payload().to_bytes().as_ref());
            }
        }
        last
    }

    /// `--watch`: print each verdict for this machine as it arrives.
    fn print_as_they_arrive(&self, session: &zenoh::Session) {
        let session = session.clone();
        let (zid, key) = (self.zid.clone(), self.key.clone());
        tokio::spawn(async move {
            let Ok(sub) = session.declare_subscriber(&key).await else {
                return;
            };
            while let Ok(s) = sub.recv_async().await {
                if let Some(v) = decode_verdict(s.payload().to_bytes().as_ref()) {
                    println!("{}\n", verdict_line(&zid, &v));
                }
            }
        });
    }
}

/// Board-glance renderer for one node's `net` envelope (node-net-health.md):
/// the connection-inventory roll-up — mesh active/stale split, the 4-bucket
/// counts, and the UNDECLARED risk list spelled out (process-attributed,
/// first-seen), because that list is the whole point of the domain.
fn print_net(env: &EnvHealthV1) {
    let Some(EnvDetail::Net(detail)) = &env.detail else {
        println!(
            "NET: {} (node {}, n={}) — no connection detail in this beat",
            colorize(&env.overall_color),
            env.scope,
            env.sample_count
        );
        return;
    };
    let buckets = |b: &str| detail.conns.iter().filter(|c| c.bucket == b).count();
    println!(
        "NET: {} — node {} ({}): {} undeclared · mesh {} active / {} stale · {} conns ({} mesh / {} fabric / {} baseline)",
        colorize(&env.overall_color),
        env.scope,
        detail.node_class,
        detail.undeclared_count,
        detail.mesh_active,
        detail.mesh_stale,
        detail.conns.len(),
        buckets("mesh"),
        buckets("fabric"),
        buckets("baseline"),
    );
    if env.overall_color == "unmetered" {
        println!("  ⓘ unmetered = no fresh connection snapshot on the node — check node-net-health-snapshot.timer there. Absence of data is never reported green.");
        return;
    }
    if detail.undeclared_count > 0 {
        println!();
        println!(
            "  {:<5} {:<6} {:<22} {:<24} {:<16} {:<14} FIRST SEEN",
            "PROTO", "STATE", "LOCAL", "REMOTE", "PROCESS", "CONTAINER"
        );
        for c in detail.conns.iter().filter(|c| c.bucket == "undeclared") {
            let age = if c.first_seen_unix_ms > 0 {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                fmt_age(now - c.first_seen_unix_ms)
            } else {
                "—".to_string()
            };
            println!(
                "  {:<5} {:<6} {:<22} {:<24} {:<16} {:<14} {}",
                c.proto,
                c.state,
                c.local_addr,
                if c.remote_addr.is_empty() { "—" } else { &c.remote_addr },
                format!("{}[{}]", c.comm, c.pid),
                if c.container.is_empty() { "host" } else { &c.container },
                age,
            );
        }
        println!();
        println!("  undeclared = outside the node-class egress contract (group_vars baseline");
        println!("  registry) and neither mesh nor fabric. Either declare it WITH ITS WHY, or");
        println!("  investigate it — this surface observes and reports; enforcement is the");
        println!("  egress-proxy design (Block 17), not this meter.");
    }
    let _ = detail.snapshot_gap_secs; // carried on the wire; rendered by the page
}


/// Board-glance renderer for one goatnet's `consistency` envelope
/// (consistency-foundation.md §5.3): which generation each node converged
/// against, the intra-goatnet skew band, and — on every non-matching row — the
/// remediation pointer, printed VERBATIM.
///
/// The three honesty rules this render exists to keep, all inherited rather
/// than invented here:
///   - Nothing ungrounded ever renders green, and no cell is ever blank. A
///     blank cell is read as "fine" by every operator who has ever looked at a
///     table (§7.5).
///   - Counts carry their denominator or they are not shown. No percentages:
///     "a percentage without a denominator is not a reading" (ADR 1109 P15).
///   - The command is printed as the library emitted it. This CLI never
///     composes, paraphrases, shortens, parameterises or executes it (§6.4);
///     where the artifact type refuses, the refusal is shown as a FACT rather
///     than as a command that fails when someone pastes it.
fn print_consistency(env: &EnvHealthV1) {
    let Some(EnvDetail::Consistency(detail)) = &env.detail else {
        println!(
            "CONSISTENCY: {} (goatnet {}, n={}) — no per-node detail in this beat",
            colorize(&env.overall_color),
            env.scope,
            env.sample_count
        );
        return;
    };

    println!(
        "CONSISTENCY: {} — goatnet {}: {}",
        colorize(&env.overall_color),
        if detail.goatnet.is_empty() { env.scope.as_str() } else { detail.goatnet.as_str() },
        consistency_reason(detail),
    );
    // The denominator and the generation serial both come from the intent
    // artifact. When the rollup could not read it, neither exists — and
    // printing "1 of 0 nodes reporting · generation serial 0" would state two
    // numbers that were never observed, in the exact place a reader looks for
    // grounding. Say what is known and no more.
    if detail.intent_readable {
        println!(
            "  site {} · {}s window · {} of {} node{} reporting · generation serial {}",
            env.scope,
            env.window_secs,
            detail.nodes_reporting,
            detail.nodes_expected,
            if detail.nodes_expected == 1 { "" } else { "s" },
            detail.current_serial,
        );
    } else {
        println!(
            "  site {} · {}s window · {} node{} reporting · expected count and current generation UNKNOWN (no readable intent)",
            env.scope,
            env.window_secs,
            detail.nodes_reporting,
            if detail.nodes_reporting == 1 { "" } else { "s" },
        );
    }

    // The rollup refused to grade. Say so loudly: a reader that showed the node
    // table here would be rendering a comparison against an intent nobody could
    // trust (ADR 1109 P14, fail-closed).
    if !detail.intent_readable {
        println!();
        println!("  ⓘ REFUSING TO GRADE — the rollup could not read or could not verify this");
        println!("    goatnet's intent artifact, so no node is compared against it:");
        println!("      {}", detail.intent_note);
        println!("    An unverifiable intent is refused, never rendered as fact.");
        return;
    }

    if detail.nodes.is_empty() {
        println!();
        println!(
            "  ⓘ {} = no node published a convergence record inside the window. Absence of",
            env.overall_color
        );
        println!("    data is never reported green, and a node that has not reported never counts");
        println!("    as compliant. Check the publishers before believing anything else here.");
        return;
    }

    println!();
    println!(
        "  {:<18} {:<7} {:<16} {:>7} {:>10} {:>11} {:>9}",
        "NODE", "STATE", "MEMBER", "SERIAL", "BEHIND", "INTENT AGE", "REPORTED"
    );
    for n in &detail.nodes {
        // A node that holds no generation shows "—", never a fabricated 0: it
        // has not been placed on the generation line at all.
        let serial = if n.intent_content_hash.is_empty() {
            "—".to_string()
        } else {
            n.intent_serial.to_string()
        };
        let behind = fmt_behind(&n.grade, &n.intent_content_hash, n.behind_by);
        println!(
            "  {:<18} {} {:<16} {:>7} {:>10} {:>11} {:>9}",
            short_name(&n.host_id),
            pad_state(&n.color, 7),
            if n.member_group.is_empty() { "—" } else { &n.member_group },
            serial,
            behind,
            fmt_intent_age(n.intent_age_seconds),
            fmt_age(n.report_age_ms),
        );
    }

    // The skew band answers the question no per-node grade can: how far apart
    // this goatnet's own nodes are. Zero-wide means they AGREE — which is not
    // the same as being right, and the per-node grades above say which.
    if detail.nodes_reporting > 1 {
        println!();
        if detail.skew_band_generations == 0 {
            println!("  skew band: 0 — every reporting node holds the SAME generation.");
            println!("  (that is agreement, not correctness: the states above say whether the");
            println!("  generation they agree on is the current one)");
        } else {
            println!(
                "  skew band: {} generation{} ({}) between the newest and oldest generation held",
                detail.skew_band_generations,
                if detail.skew_band_generations == 1 { "" } else { "s" },
                fmt_intent_age(detail.skew_band_seconds),
            );
            println!("  inside this goatnet — the nodes disagree with each other, not only with the tree.");
        }
    }

    // Remediation, verbatim, once per distinct pointer.
    let mut printed: Vec<&str> = Vec::new();
    for n in &detail.nodes {
        for r in n.rows.iter().filter(|r| !r.remediation.is_empty()) {
            if printed.contains(&r.remediation.as_str()) {
                continue;
            }
            printed.push(&r.remediation);
            if printed.len() == 1 {
                println!();
                println!("  WHAT TO RUN — emitted by the grading library, shown verbatim:");
            }
            println!();
            println!(
                "    {} {} ({})",
                pad_state(&r.color, 7),
                if r.name.is_empty() { "—" } else { &r.name },
                r.why
            );
            if r.remediation_refuses {
                println!("      REFUSES: {}", r.remediation);
            } else {
                println!("      {}", r.remediation);
            }
        }
    }

    println!();
    println!("  a node BEHIND is fixed by re-running its play; a node whose generation no");
    println!("  committed artifact explains is NOT — re-running erases the evidence first.");
    println!("  Drift is fixed by re-converging, regenerating, rolling back or declaring —");
    println!("  never by hand (engineering-baseline.md §2.2).");

    // Coverage, only where something is unobservable — a count with its
    // denominator, never a silently shorter list.
    let gaps: Vec<&crate::health_wire::NodeConsistencyV1> = detail
        .nodes
        .iter()
        .filter(|n| n.components_observed < n.components_governed)
        .collect();
    if !gaps.is_empty() {
        println!();
        for n in gaps {
            println!(
                "  ⓘ {}: {} of {} governed components observed — the rest have no deploy-ledger",
                short_name(&n.host_id),
                n.components_observed,
                n.components_governed
            );
            println!("    coverage on this node, so nothing can be said about them (ADR 1060 gap).");
        }
    }
}

/// The headline reason, derived from the same rows the colour came from — an
/// amber without its "why" is half a reading.
fn consistency_reason(detail: &ConsistencyDetailV1) -> String {
    if !detail.intent_readable {
        return "the intent artifact could not be read or verified".to_string();
    }
    if detail.nodes.is_empty() {
        return "no node reported".to_string();
    }
    let count = |g: &str| detail.nodes.iter().filter(|n| n.grade == g).count();
    let (unaccountable, skew) = (count("unaccountable"), count("SKEW"));
    let ungrounded = count("unreadable") + count("indeterminate");
    let matched = count("match") + count("declared");
    let mut parts: Vec<String> = Vec::new();
    if unaccountable > 0 {
        parts.push(format!(
            "{unaccountable} running a generation NO committed artifact explains"
        ));
    }
    if skew > 0 {
        parts.push(format!("{skew} behind the current generation"));
    }
    if ungrounded > 0 {
        parts.push(format!("{ungrounded} with nothing groundable to read"));
    }
    if parts.is_empty() {
        return format!(
            "all {matched} reporting node{} on the current generation",
            if matched == 1 { "" } else { "s" }
        );
    }
    if matched > 0 {
        parts.push(format!("{matched} current"));
    }
    parts.join("; ")
}

/// The BEHIND column. "current" is a CLAIM, and it is only true for a node
/// whose generation the artifact actually accounts for.
///
/// The case this exists for: an `unaccountable` node has a content hash and a
/// `behind_by` of 0 — it was never *placed* on the generation line at all — so
/// a naive "hash present and behind_by == 0 means current" would print
/// "current" beside a RED state. A table that contradicts itself in the one row
/// an operator most needs to read correctly is worse than a blank one.
fn fmt_behind(grade: &str, intent_content_hash: &str, behind_by: u32) -> String {
    if intent_content_hash.is_empty() {
        return "—".to_string();
    }
    match grade {
        "unaccountable" => "unexplained".to_string(),
        "unreadable" => "unverified".to_string(),
        _ if behind_by == 0 => "current".to_string(),
        _ => format!("{behind_by} gen"),
    }
}

/// Intent age in words. Negative means unstatable — a reader whose clock sits
/// before the generation reports no age rather than a negative one (ADR 1037
/// makes UTC confidence a thing to state, not assume).
fn fmt_intent_age(seconds: i64) -> String {
    if seconds < 0 {
        return "—".to_string();
    }
    if seconds >= 86_400 {
        return format!("{}d", seconds / 86_400);
    }
    if seconds >= 3_600 {
        return format!("{}h", seconds / 3_600);
    }
    format!("{}m", seconds / 60)
}


#[cfg(test)]
mod mesh_tests {
    use super::*;

    fn link(fqdn: &str, color: &str, connected: bool, direct: bool, relay: &str) -> MeshLinkHealthV1 {
        MeshLinkHealthV1 {
            peer_fqdn: fqdn.to_string(),
            peer_ip: "100.127.0.9".to_string(),
            color: color.to_string(),
            connected,
            direct,
            conn_type: if !connected { "" } else if direct { "P2P" } else { "Relayed" }.to_string(),
            latency_ms: 0.0,
            relay_address: relay.to_string(),
            handshake_age_ms: 5000,
            transition_count: 0,
        }
    }

    fn detail(links: Vec<MeshLinkHealthV1>) -> MeshDetailV1 {
        let connected = links.iter().filter(|l| l.connected).count() as u32;
        MeshDetailV1 { peers_total: links.len() as u32, peers_connected: connected, vantage_host: "thor.x".to_string(), links }
    }

    #[test]
    fn reason_calls_out_all_relayed() {
        let d = detail(vec![
            link("a.x", "amber", true, false, "rels://relay-dub.x:33073"),
            link("b.x", "amber", true, false, "rels://relay-rla.x:33073"),
            link("off.x", "idle", false, false, ""),
        ]);
        // offline peers don't count toward "connected"; both connected are relayed.
        assert_eq!(mesh_reason(&d), "all 2 connected peers are riding relay fallback");
    }

    #[test]
    fn reason_mixes_relayed_and_direct() {
        let d = detail(vec![
            link("a.x", "green", true, true, ""),
            link("b.x", "amber", true, false, "rels://relay-rla.x:33073"),
        ]);
        assert_eq!(mesh_reason(&d), "1 of 2 riding relay fallback");
    }

    #[test]
    fn reason_all_direct_is_clean() {
        let d = detail(vec![
            link("a.x", "green", true, true, ""),
            link("b.x", "green", true, true, ""),
        ]);
        assert_eq!(mesh_reason(&d), "2 direct P2P links");
    }

    #[test]
    fn reason_flags_slow_direct_link() {
        let mut slow = link("a.x", "amber", true, true, "");
        slow.latency_ms = 320.0;
        let d = detail(vec![slow]);
        assert_eq!(mesh_reason(&d), "1 direct link with high measured latency");
    }

    #[test]
    fn singular_peer_grammar() {
        let d = detail(vec![link("a.x", "amber", true, false, "rels://relay-arn.x:33073")]);
        assert_eq!(mesh_reason(&d), "all 1 connected peer is riding relay fallback");
    }

    #[test]
    fn relay_label_extracts_short_name() {
        assert_eq!(relay_label("rels://relay-dub.zagros.example:33073"), "via relay-dub");
        assert_eq!(relay_label(""), "via relay");
    }

    #[test]
    fn age_formats_by_magnitude() {
        assert_eq!(fmt_age(-1), "—");
        assert_eq!(fmt_age(44_000), "44s ago");
        assert_eq!(fmt_age(120_000), "2m ago");
        assert_eq!(fmt_age(7_200_000), "2h ago");
    }

    #[test]
    fn pad_state_ignores_ansi_width() {
        // Plain (non-TTY in tests) → just the uppercased word, padded to width.
        let s = pad_state("amber", 7);
        assert!(s.starts_with("AMBER"));
        assert_eq!(s.chars().filter(|c| *c == ' ').count(), 2); // 7 - 5
    }

    #[test]
    fn envelope_round_trips_mesh_oneof() {
        let env = EnvHealthV1 {
            domain: "mesh".to_string(),
            scope: "zagros-sandbox".to_string(),
            ts_unix_ms: 1,
            overall_color: "amber".to_string(),
            sample_count: 2,
            window_secs: 30,
            detail: Some(EnvDetail::Mesh(detail(vec![
                link("a.x", "amber", true, false, "rels://relay-rla.x:33073"),
            ]))),
        };
        let bytes = env.encode_to_vec();
        let back = EnvHealthV1::decode(bytes.as_slice()).expect("decode");
        assert_eq!(back.domain, "mesh");
        match back.detail {
            Some(EnvDetail::Mesh(d)) => assert_eq!(d.links[0].peer_fqdn, "a.x"),
            _ => panic!("expected mesh detail"),
        }
    }
}

#[cfg(test)]
mod presence_tests {
    use super::*;
    use crate::health_wire::PresencePeerV1;

    fn peer(label: &str, root: &str, alive: bool, drops: u64) -> PresencePeerV1 {
        PresencePeerV1 {
            token_key: format!("{root}/presence/{label}"),
            write_root: root.to_string(),
            label: label.to_string(),
            alive,
            since_unix_ms: 1,
            drop_count: drops,
        }
    }

    fn detail(peers: Vec<PresencePeerV1>, dropped_recent: u32) -> PresenceDetailV1 {
        let alive_total = peers.iter().filter(|p| p.alive).count() as u32;
        PresenceDetailV1 { peers, alive_total, dropped_recent }
    }

    #[test]
    fn reason_no_drops_is_clean() {
        let d = detail(vec![peer("a", "vendor-a", true, 0), peer("b", "vendor-b", true, 0)], 0);
        assert_eq!(presence_reason(&d), "no drops this window");
    }

    #[test]
    fn reason_calls_out_recent_churn() {
        let d = detail(vec![peer("a", "vendor-a", true, 1), peer("b", "vendor-b", false, 1)], 1);
        assert_eq!(
            presence_reason(&d),
            "1 peer dropped within the last window (churn — look here)"
        );
    }

    #[test]
    fn reason_flags_total_vanish() {
        let d = detail(vec![peer("a", "vendor-a", false, 1)], 1);
        assert_eq!(presence_reason(&d), "the declared presence fleet has vanished — every token dropped");
    }

    #[test]
    fn short_root_truncates_uuid_anchor_only() {
        // 32-hex slot anchor → 8 + ellipsis.
        assert_eq!(short_root("e7a9c75b1234567890abcdef01234567"), "e7a9c75b…");
        // Vendor prefix / ops path → verbatim.
        assert_eq!(short_root("maintainer-operator"), "maintainer-operator");
        assert_eq!(short_root("ops/fabric"), "ops/fabric");
    }

    #[test]
    fn envelope_round_trips_presence_oneof() {
        let env = EnvHealthV1 {
            domain: "presence".to_string(),
            scope: "zagros-sandbox".to_string(),
            ts_unix_ms: 100,
            overall_color: "amber".to_string(),
            sample_count: 2,
            window_secs: 30,
            detail: Some(EnvDetail::Presence(detail(
                vec![peer("synth-01", "e7a9c75b1234567890abcdef01234567", true, 0)],
                1,
            ))),
        };
        let bytes = env.encode_to_vec();
        let back = EnvHealthV1::decode(bytes.as_slice()).expect("decode");
        assert_eq!(back.domain, "presence");
        match back.detail {
            Some(EnvDetail::Presence(d)) => {
                assert_eq!(d.alive_total, 1);
                assert_eq!(d.peers[0].label, "synth-01");
            }
            _ => panic!("expected presence detail"),
        }
    }
}

#[cfg(test)]
mod time_tests {
    use super::*;
    use crate::health_wire::HostClockHealthV1;

    fn host(id: &str, color: &str, verdict: &str, attested: bool) -> HostClockHealthV1 {
        HostClockHealthV1 {
            host_id: id.to_string(),
            color: color.to_string(),
            verdict: verdict.to_string(),
            verdict_reasons: vec![],
            rms_offset_ms: 0.4,
            stratum: 2,
            gnss_present: false,
            peer_median_offset_ms: 0.0,
            divergence_detected: false,
            attestation_age_ms: 5000,
            attested,
        }
    }

    #[test]
    fn reason_all_healthy() {
        let d = TimeDetailV1 { hosts: vec![host("thor.x", "green", "healthy", true)] };
        assert_eq!(time_reason(&d), "all 1 host clock healthy");
    }

    #[test]
    fn reason_mixes_degraded_and_unattested() {
        let d = TimeDetailV1 {
            hosts: vec![
                host("a.x", "red", "degraded", true),
                host("b.x", "idle", "unknown", false),
            ],
        };
        assert_eq!(time_reason(&d), "1 degraded; 1 unattested (idle, never green)");
    }

    #[test]
    fn reason_flags_divergence() {
        let mut h = host("a.x", "amber", "warning", true);
        h.divergence_detected = true;
        let d = TimeDetailV1 { hosts: vec![h] };
        assert_eq!(time_reason(&d), "1 warning; 1 diverging from peer consensus");
    }

    #[test]
    fn envelope_round_trips_time_oneof() {
        let env = EnvHealthV1 {
            domain: "time".to_string(),
            scope: "zagros-sandbox".to_string(),
            ts_unix_ms: 1,
            overall_color: "green".to_string(),
            sample_count: 1,
            window_secs: 30,
            detail: Some(EnvDetail::Time(TimeDetailV1 {
                hosts: vec![host("thor.x", "green", "healthy", true)],
            })),
        };
        let bytes = env.encode_to_vec();
        let back = EnvHealthV1::decode(bytes.as_slice()).expect("decode");
        assert_eq!(back.domain, "time");
        match back.detail {
            Some(EnvDetail::Time(d)) => assert_eq!(d.hosts[0].verdict, "healthy"),
            _ => panic!("expected time detail"),
        }
    }
}

#[cfg(test)]
mod consistency_tests {
    use super::*;
    use crate::health_wire::{ConsistencyRowV1, NodeConsistencyV1};

    fn node(host: &str, grade: &str, color: &str, behind: u32) -> NodeConsistencyV1 {
        NodeConsistencyV1 {
            host_id: host.to_string(),
            member_group: "zagros_hosts".to_string(),
            color: color.to_string(),
            grade: grade.to_string(),
            intent_serial: 3,
            intent_content_hash: "sha256:aa".to_string(),
            behind_by: behind,
            intent_age_seconds: 259_200,
            report_age_ms: 4_000,
            intent_verified: true,
            rows: Vec::new(),
            components_governed: 5,
            components_observed: 3,
        }
    }

    fn detail(nodes: Vec<NodeConsistencyV1>) -> ConsistencyDetailV1 {
        ConsistencyDetailV1 {
            nodes_reporting: nodes.len() as u32,
            nodes,
            goatnet: "zagros".to_string(),
            current_serial: 4,
            current_content_hash: "sha256:bb".to_string(),
            nodes_expected: 2,
            skew_band_generations: 0,
            skew_band_seconds: 0,
            intent_readable: true,
            intent_note: String::new(),
        }
    }

    #[test]
    fn the_reason_names_the_worst_finding_first() {
        // An amber (or a red) without its "why" is half a reading, and the two
        // findings are not interchangeable: "behind" is fixed by re-running and
        // "unexplained" is emphatically not.
        let d = detail(vec![
            node("zagros-a", "match", "green", 0),
            node("zagros-b", "SKEW", "amber", 1),
            node("zagros-c", "unaccountable", "red", 0),
        ]);
        let reason = consistency_reason(&d);
        assert!(reason.starts_with("1 running a generation NO committed artifact explains"), "{reason}");
        assert!(reason.contains("1 behind the current generation"), "{reason}");
        assert!(reason.contains("1 current"), "{reason}");
    }

    #[test]
    fn all_current_reads_as_a_count_with_its_denominator_and_no_percentage() {
        let reason = consistency_reason(&detail(vec![
            node("zagros-a", "match", "green", 0),
            node("zagros-b", "match", "green", 0),
        ]));
        assert_eq!(reason, "all 2 reporting nodes on the current generation");
        assert!(!reason.contains('%'), "a ratio over a set this small invents precision");
    }

    #[test]
    fn nothing_reporting_says_so_rather_than_reading_as_clean() {
        assert_eq!(consistency_reason(&detail(vec![])), "no node reported");
    }

    #[test]
    fn an_unreadable_intent_reports_the_refusal_not_the_nodes() {
        let mut d = detail(vec![node("zagros-a", "match", "green", 0)]);
        d.intent_readable = false;
        assert_eq!(
            consistency_reason(&d),
            "the intent artifact could not be read or verified"
        );
    }

    #[test]
    fn the_behind_column_never_claims_current_for_a_node_nobody_placed() {
        // An unaccountable node HAS a hash and a behind_by of 0 — it was never
        // placed on the generation line — so the naive read would print
        // "current" beside a RED state. That contradiction is the bug this
        // pins.
        assert_eq!(fmt_behind("unaccountable", "sha256:ee", 0), "unexplained");
        assert_eq!(fmt_behind("unreadable", "sha256:aa", 0), "unverified");
        assert_eq!(fmt_behind("match", "sha256:aa", 0), "current");
        assert_eq!(fmt_behind("SKEW", "sha256:aa", 2), "2 gen");
        // No stamp at all: nothing to say, and "—" says exactly that.
        assert_eq!(fmt_behind("indeterminate", "", 0), "—");
    }

    #[test]
    fn intent_age_is_never_rendered_negative() {
        // A reader whose clock sits before the generation reports NO age rather
        // than a negative one — ADR 1037 makes UTC confidence a thing to state.
        assert_eq!(fmt_intent_age(-1), "—");
        assert_eq!(fmt_intent_age(259_200), "3d");
        assert_eq!(fmt_intent_age(7_200), "2h");
        assert_eq!(fmt_intent_age(120), "2m");
    }

    #[test]
    fn the_envelope_round_trips_with_its_remediation_pointer_intact() {
        // The pointer is DATA emitted by the grading library. This CLI prints it
        // verbatim, so the only thing it owes is not to lose it on the wire.
        let mut n = node("zagros-b", "SKEW", "amber", 1);
        n.rows.push(ConsistencyRowV1 {
            subject: "generation".into(),
            name: "intent".into(),
            artifact_type: "forward-only".into(),
            grade: "SKEW".into(),
            color: "amber".into(),
            why: "zagros-b=behind (converged against serial 3) — they differ and nothing declares why".into(),
            observed: "sha256:aa".into(),
            expected: "sha256:bb".into(),
            remediation_kind: "reconverge".into(),
            remediation: "ops/scripts/ansible-preflight.sh -i inventory/mgmt-hosts-mesh.ini zagros-refresh.yml -e target=zagros-b".into(),
            remediation_refuses: false,
        });
        let env = EnvHealthV1 {
            domain: "consistency".into(),
            scope: "zagros-sandbox".into(),
            ts_unix_ms: 1,
            overall_color: "amber".into(),
            sample_count: 1,
            window_secs: 30,
            detail: Some(EnvDetail::Consistency(detail(vec![n]))),
        };
        let bytes = env.encode_to_vec();
        let back = EnvHealthV1::decode(bytes.as_slice()).expect("decode");
        assert_eq!(back.domain, "consistency");
        let Some(EnvDetail::Consistency(d)) = &back.detail else { panic!("consistency arm") };
        assert_eq!(d.nodes[0].rows[0].remediation_kind, "reconverge");
        assert!(d.nodes[0].rows[0].remediation.contains("zagros-refresh.yml"));
        assert!(!d.nodes[0].rows[0].remediation_refuses);
    }

    #[test]
    fn a_refusing_pointer_survives_as_a_refusal() {
        // A refusal is a RENDERED FACT, not a command that fails when pasted.
        let mut n = node("zagros-a", "unaccountable", "red", 0);
        n.rows.push(ConsistencyRowV1 {
            subject: "generation".into(),
            name: "intent".into(),
            artifact_type: "forward-only".into(),
            grade: "unaccountable".into(),
            color: "red".into(),
            why: "no generation explains it".into(),
            observed: "sha256:ee".into(),
            expected: "sha256:bb".into(),
            remediation_kind: "refuse".into(),
            remediation: "no undo path: `intent` is forward-only (ADR 1060). Converge forward to the current generation and keep the finding — do not try to put the node back.".into(),
            remediation_refuses: true,
        });
        assert!(n.rows[0].remediation_refuses);
        assert!(!n.rows[0].remediation.contains("refresh.yml"),
                "an unexplained node must never be handed a re-run");
    }
}

#[cfg(test)]
mod peer_tests {
    use super::*;

    const ZID_A: &str = "3c9a51e0d2b7f4a6c18e05d9b3a7f2e1";
    const ZID_B: &str = "7d2e4f6a8b0c1d3e5f7a9b1c3d5e7f90";
    const OURS: &str = "5be08d2f41a97c36e0d1f8b2a4c6e9d7";

    fn row(
        zid: &str,
        name: Option<&str>,
        offset: f64,
        rate: f64,
        direct: Option<bool>,
        grade: &str,
        color: &str,
    ) -> PeerFaultV1 {
        PeerFaultV1 {
            zid: zid.to_string(),
            kind: PeerFaultKindV1::ClockAhead as i32,
            offset_seconds: offset,
            min_offset_seconds: offset - 0.2,
            max_offset_seconds: offset + 0.1,
            rejections_per_minute: rate,
            rejections_in_window: (rate as u64).max(1),
            first_seen_unix_ms: 1,
            last_seen_unix_ms: 2,
            observed_by: OURS.to_string(),
            attribution_name: name.map(str::to_string),
            attribution_owner: name.map(|_| "example partner".to_string()),
            grade: grade.to_string(),
            color: color.to_string(),
            limit_seconds: 0.5,
            direct_session: direct,
        }
    }

    fn env(color: &str, detail: PeerFaultDetailV1) -> EnvHealthV1 {
        EnvHealthV1 {
            domain: "peer".to_string(),
            scope: "zagros-sandbox".to_string(),
            ts_unix_ms: 1_794_571_200_000,
            overall_color: color.to_string(),
            sample_count: detail.peers.len() as u64,
            window_secs: 60,
            detail: Some(EnvDetail::Peer(detail)),
        }
    }

    fn at_fault(peering: &str) -> PeerFaultDetailV1 {
        PeerFaultDetailV1 {
            peers: vec![
                row(
                    ZID_A,
                    Some("lab-router-b"),
                    17.0,
                    12.0,
                    Some(true),
                    "peer_at_fault",
                    "red",
                ),
                row(ZID_B, None, 0.8, 1.5, None, "peer_at_fault", "red"),
            ],
            verdict: "peer_at_fault".to_string(),
            verdict_reason: "2 peer(s) had timestamps rejected in the last 60 s".to_string(),
            local_clock: "synced".to_string(),
            observed_by: OURS.to_string(),
            peers_known: 2,
            last_line_age_ms: 400,
            unparsed_lines_window: 0,
            peering: peering.to_string(),
            peering_rejections_per_minute: if peering.is_empty() { 0.0 } else { 13.5 },
            peering_max_offset_seconds: if peering.is_empty() { 0.0 } else { 17.1 },
        }
    }

    #[test]
    fn envelope_round_trips_peer_oneof_arm_17() {
        let e = env("red", at_fault("xab"));
        let bytes = e.encode_to_vec();
        // The arm is field 17: tag byte (17 << 3 | 2) = 0x8a 0x01, length-delimited.
        assert!(
            bytes.windows(2).any(|w| w == [0x8a, 0x01]),
            "arm 17 on the wire"
        );
        let back = EnvHealthV1::decode(bytes.as_slice()).expect("decode");
        assert_eq!(back.domain, "peer");
        match back.detail {
            Some(EnvDetail::Peer(d)) => {
                assert_eq!(d.peers.len(), 2);
                assert_eq!(d.peers[0].attribution_name.as_deref(), Some("lab-router-b"));
                assert_eq!(d.peers[0].direct_session, Some(true));
                assert_eq!(d.peers[1].direct_session, None, "absent stays unknown");
                assert_eq!(
                    PeerFaultKindV1::try_from(d.peers[0].kind),
                    Ok(PeerFaultKindV1::ClockAhead)
                );
                assert_eq!(d.peering, "xab");
            }
            _ => panic!("expected peer detail"),
        }
    }

    #[test]
    fn an_envelope_with_an_arm_this_build_does_not_know_still_decodes() {
        // Arm 16 (peering) is not mirrored: the envelope decodes, detail absent.
        let mut bytes = EnvHealthV1 {
            domain: "peering".to_string(),
            scope: "s".to_string(),
            ts_unix_ms: 1,
            overall_color: "green".to_string(),
            sample_count: 1,
            window_secs: 30,
            detail: None,
        }
        .encode_to_vec();
        bytes.extend_from_slice(&[0x82, 0x01, 0x02, 0x0a, 0x00]); // field 16, 2 bytes
        let back = EnvHealthV1::decode(bytes.as_slice()).expect("decode");
        assert_eq!(back.domain, "peering");
        assert!(back.detail.is_none());
    }

    #[test]
    fn render_shows_site_grade_and_each_peer() {
        let out = render_peer(&env("red", at_fault("")));
        assert!(
            out.starts_with("PEER: RED — peer_at_fault: 2 peer(s) had timestamps rejected"),
            "{out}"
        );
        assert!(out.contains("our clock synced"), "{out}");
        let a = out
            .lines()
            .find(|l| l.contains("lab-router-b"))
            .expect("named row");
        assert!(
            a.contains("+17.00 s") && a.contains("12.0") && a.contains("yes"),
            "{a}"
        );
        // No name: the Zenoh ID is the label, and an absent session is unknown.
        let b = out.lines().find(|l| l.contains(ZID_B)).expect("zid row");
        assert!(
            b.contains("+0.80 s") && b.contains("1.5") && b.contains("unknown"),
            "{b}"
        );
        assert!(b.trim_end().ends_with('—'), "no peering: {b}");
        assert!(
            !out.contains("peering "),
            "no peering line without one: {out}"
        );
    }

    #[test]
    fn render_names_the_peering_when_set() {
        let out = render_peer(&env("red", at_fault("xab")));
        assert!(
            out.contains("peering xab: 13.5 rejections/min, largest offset +17.10 s"),
            "{out}"
        );
        let a = out.lines().find(|l| l.contains("lab-router-b")).unwrap();
        assert!(a.trim_end().ends_with("xab"), "{a}");
    }

    #[test]
    fn an_unmeasured_reading_is_never_green_and_says_why() {
        let d = PeerFaultDetailV1 {
            verdict: "unmeasured".to_string(),
            verdict_reason: "no router log line for 900 s".to_string(),
            local_clock: "synced".to_string(),
            last_line_age_ms: -1,
            ..Default::default()
        };
        let out = render_peer(&env("unmetered", d));
        assert!(
            out.starts_with("PEER: UNMETERED — unmeasured: no router log line"),
            "{out}"
        );
        assert!(out.contains("never a false green"), "{out}");
        assert!(!out.contains("PEER  "), "no table without rows: {out}");
    }

    #[test]
    fn a_suspect_local_clock_blames_no_peer() {
        let mut d = at_fault("");
        d.verdict = "local_clock_suspect".to_string();
        d.verdict_reason = "our host clock is not grounded (unsynced)".to_string();
        d.local_clock = "unsynced".to_string();
        for p in &mut d.peers {
            p.grade = "local_clock_suspect".to_string();
            p.color = "idle".to_string();
        }
        let out = render_peer(&env("amber", d));
        assert!(out.contains("our own clock is the likelier fault"), "{out}");
        assert!(!out.contains("peer_at_fault"), "{out}");
    }

    #[test]
    fn json_carries_the_grade_and_every_row() {
        let e = env("red", at_fault("xab"));
        let j = env_detail_json(&e).expect("peer detail");
        assert_eq!(j["verdict"], "peer_at_fault");
        assert_eq!(j["peering"], "xab");
        assert_eq!(j["local_clock"], "synced");
        let peers = j["peers"].as_array().unwrap();
        assert_eq!(peers.len(), 2);
        assert_eq!(peers[0]["label"], "lab-router-b");
        assert_eq!(peers[0]["zid"], ZID_A);
        assert_eq!(peers[0]["offset_seconds"], 17.0);
        assert_eq!(peers[0]["rejections_per_minute"], 12.0);
        assert_eq!(peers[0]["direct_session"], "yes");
        assert_eq!(peers[0]["kind"], "clock_ahead");
        assert_eq!(peers[0]["peering"], "xab");
        assert_eq!(peers[1]["label"], ZID_B);
        assert_eq!(peers[1]["name"], serde_json::Value::Null);
        assert_eq!(peers[1]["direct_session"], "unknown");
        // Without a peering it is null, never an empty string.
        let j = env_detail_json(&env("red", at_fault(""))).unwrap();
        assert_eq!(j["peering"], serde_json::Value::Null);
        // Other domains carry no detail.
        let time = EnvHealthV1 {
            domain: "time".to_string(),
            detail: Some(EnvDetail::Time(TimeDetailV1 { hosts: vec![] })),
            ..e
        };
        assert!(env_detail_json(&time).is_none());
    }

    const VERDICT: &str = r#"{"schema":"goat.peer.verdict.v1","fault":"clock_ahead","state":"at_fault","offset_seconds":17.0,
 "limit_seconds":0.5,"rejections_per_minute":12.0,"observed_at":"2026-11-13T13:19:10.000Z",
 "observed_by":"5be08d2f41a97c36e0d1f8b2a4c6e9d7",
 "sentence":"the router sees your clock 17.0 s ahead of its own; the limit is 0.5 s"}"#;

    #[test]
    fn the_verdict_line_prints_the_routers_sentence() {
        let v = decode_verdict(VERDICT.as_bytes()).expect("verdict");
        assert_eq!(
            verdict_line(ZID_A, &v),
            format!(
                "VERDICT on this machine ({ZID_A}, at_fault): the router sees your clock 17.0 s ahead of its own; \
                 the limit is 0.5 s (observed 2026-11-13T13:19:10.000Z)"
            )
        );
    }

    #[test]
    fn a_cleared_verdict_and_one_without_a_sentence_still_read() {
        let mut v = decode_verdict(VERDICT.as_bytes()).unwrap();
        v.sentence = String::new();
        v.observed_at = String::new();
        assert_eq!(
            verdict_line(ZID_A, &v),
            format!("VERDICT on this machine ({ZID_A}, at_fault): the router sees your clock 17.0 s ahead of its own; the limit is 0.5 s")
        );
        v.state = "cleared".to_string();
        assert!(verdict_line(ZID_A, &v)
            .ends_with("cleared): the router reports the clock fault cleared"));
    }

    #[test]
    fn only_a_verdict_document_is_read_and_its_text_is_made_safe() {
        assert!(decode_verdict(b"not json").is_none());
        assert!(decode_verdict(br#"{"schema":"something.else","sentence":"x"}"#).is_none());
        let v = decode_verdict(br#"{"schema":"goat.peer.verdict.v1","state":"at_fault","sentence":"ahead\u001b[2Jby 2 s"}"#)
            .expect("minimal verdict");
        let line = verdict_line(ZID_A, &v);
        assert!(!line.chars().any(|c| c.is_control()), "{line:?}");
        assert!(line.ends_with("ahead[2Jby 2 s"), "{line}");
    }

    #[test]
    fn the_verdict_document_round_trips_as_json() {
        let v = decode_verdict(VERDICT.as_bytes()).unwrap();
        let j = serde_json::to_value(&v).unwrap();
        assert_eq!(j["schema"], "goat.peer.verdict.v1");
        assert_eq!(j["offset_seconds"], 17.0);
        assert!(
            j.get("peering").is_none() && j.get("zids").is_none(),
            "absent fields stay absent"
        );
    }
}
