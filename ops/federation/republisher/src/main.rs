// SPDX-License-Identifier: Apache-2.0
//
// goat-fed-republisher — the outbound half of the symmetric publishing rule.
//
//   docs/design/federation-package-spec.md §3.2   the rule
//   docs/design/cross-sovereign-peering-transport.md §5   the declaration
//   ops/federation-drill/cloud/RESULTS.md         the inbound half
//   F-446                                          why this is a process
//
// THE ASYMMETRY THIS CLOSES. Inbound, `from/<partner>/**` is the only origin
// claim that is ENFORCED rather than asserted: the far side's access rules bind
// it to that party's peering identity, and a write
// outside it is refused. Outbound, our data is staged under a LOCAL name —
// `release/weather/**` — and the fabric cannot rename a key expression in
// transit. Lifting the staging name onto the wire name is therefore a
// small process on our own side, over our own data. Building it into the router
// would mean maintaining a fork of the router.
//
// TWO SESSIONS, ONE DIRECTION. It subscribes on the local fabric and publishes
// on the peering edge, and never the reverse. That is the whole of its
// privilege: it is the only component that touches both sides, so it is
// deliberately one-way and does exactly one transformation.
//
// CONFIGURATION IS THE AGREEMENT. The release set, the wire prefix and every
// ceiling come from the composed agreement — the same document that renders the
// access rules. Nothing here is hand-listed, so the rules and the flow cannot
// drift apart by being typed twice.

mod agreement;
mod edge;
mod mapping;
mod metrics;
mod policy;
mod provenance;

use anyhow::{Context, Result};
use axum::http::StatusCode;
use axum::routing::get;
use axum::Router;
use chrono::Utc;
use clap::Parser;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::agreement::Agreement;
use crate::edge::{run_publisher, Outbound, ZenohEdge};
use crate::mapping::KeyMap;
use crate::metrics::Metrics;
use crate::policy::{Decision, Policy};

/// Boot ordering, not an outage strategy: a router routinely comes up after us.
const SESSION_OPEN_RETRY: Duration = Duration::from_secs(5);

#[derive(Parser, Debug)]
#[command(
    name = "goat-fed-republisher",
    about = "Republish a declared release set onto the peering wire name from/<sovereign>/."
)]
struct Args {
    /// Path to the composed agreement, as JSON (spec §8).
    #[arg(long, env = "FED_REPUBLISHER_AGREEMENT")]
    agreement: String,

    /// OUR sovereign_id. Selects the one flow in the agreement that originates
    /// with us; there is no our-form and their-form of the file.
    #[arg(long, env = "FED_REPUBLISHER_SOVEREIGN_ID")]
    sovereign_id: String,

    /// Local fabric router, e.g. `tls/goat-zenoh-router:7447`. Where the
    /// release set is staged.
    #[arg(long, env = "FED_REPUBLISHER_LOCAL_ENDPOINT")]
    local_endpoint: String,
    #[arg(long, env = "FED_REPUBLISHER_LOCAL_CERT")]
    local_cert: Option<String>,
    #[arg(long, env = "FED_REPUBLISHER_LOCAL_KEY")]
    local_key: Option<String>,
    #[arg(long, env = "FED_REPUBLISHER_LOCAL_ROOT_CA")]
    local_root_ca: Option<String>,

    /// Where the wire name is published, e.g. `tls/goat-zenoh-router:7447`.
    /// Deployed (roles/goat-fed-republisher) it is OUR CORE router, the same
    /// one as --local-endpoint: the core admits this process to write
    /// `from/<us>/**`, and the peering edge carries that name out. The
    /// partner's authority still terminates at the edge, never at the core.
    #[arg(long, env = "FED_REPUBLISHER_EDGE_ENDPOINT")]
    edge_endpoint: String,
    #[arg(long, env = "FED_REPUBLISHER_EDGE_CERT")]
    edge_cert: Option<String>,
    #[arg(long, env = "FED_REPUBLISHER_EDGE_KEY")]
    edge_key: Option<String>,
    #[arg(long, env = "FED_REPUBLISHER_EDGE_ROOT_CA")]
    edge_root_ca: Option<String>,

    /// Depth of the bounded hand-off queue. Full means the arriving sample is
    /// dropped and counted; it never means blocking (see edge.rs).
    #[arg(long, env = "FED_REPUBLISHER_QUEUE_DEPTH", default_value_t = 4096)]
    queue_depth: usize,

    #[arg(
        long,
        env = "FED_REPUBLISHER_METRICS_BIND",
        default_value = "0.0.0.0:9310"
    )]
    metrics_bind: SocketAddr,

    /// Validate the agreement, build the mapping, print what would cross, and
    /// exit. Opens no session and touches no host.
    #[arg(long, default_value_t = false)]
    check: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let args = Args::parse();

    let raw = std::fs::read_to_string(&args.agreement)
        .with_context(|| format!("reading agreement {}", args.agreement))?;
    let ag = Agreement::from_json(&raw)?;
    let flow = ag.outbound_flow(&args.sovereign_id)?.clone();
    let expires = ag.expiry_date()?;
    // Refuse at startup, rather than starting and refusing every sample. An
    // expired agreement is not an agreement.
    if Utc::now().date_naive() > expires {
        anyhow::bail!("the agreement expired on {expires}; renew it before restarting this flow");
    }
    let map = KeyMap::new(&args.sovereign_id, &flow.release)?;

    info!(
        from = flow.from.as_str(),
        to = flow.to.as_str(),
        release = ?flow.release,
        wire_prefix = map.wire_prefix().as_str(),
        hz = flow.envelope.hz,
        bytes_per_message = flow.envelope.bytes_per_message,
        distinct_keys = flow.envelope.distinct_keys,
        expires = %expires,
        "agreement loaded",
    );
    for prefix in map.subscriptions() {
        // The mapping, spelled out once at startup, so the operator can read
        // what a partner will see without running a sample through it.
        info!(
            "release {} -> wire {}",
            prefix.as_str(),
            prefix
                .as_str()
                .split_once('/')
                .map(|(_, rest)| format!("from/{}/{}", args.sovereign_id, rest))
                .unwrap_or_default()
        );
    }
    if args.check {
        info!("--check: agreement and mapping are valid; no session opened");
        return Ok(());
    }

    let metrics = Metrics::new(&args.sovereign_id, &flow.to)?;
    metrics.queue_depth.set(args.queue_depth as i64);
    metrics
        .distinct_wire_keys_limit
        .set(flow.envelope.distinct_keys as i64);
    metrics.agreement_expiry_seconds.set(
        expires
            .and_hms_opt(23, 59, 59)
            .map(|d| d.and_utc().timestamp())
            .unwrap_or(0),
    );

    // /metrics before any session, so a scrape during a slow router start sees
    // `edge_linked 0` rather than nothing at all.
    let m = metrics.clone();
    let bind = args.metrics_bind;
    // ADR 0709 D7 forbids `tokio::spawn` in goat-relay's shard-pinned hot
    // path. This is not that: a long-running supervisor task on a
    // multi-threaded runtime, which is exactly the reviewed deviation the
    // workspace clippy.toml describes.
    #[allow(clippy::disallowed_methods)]
    tokio::spawn(async move {
        if let Err(e) = serve_metrics(m, bind).await {
            error!(error = %e, "metrics server exited");
        }
    });

    let edge_session = open_session(
        "edge",
        &args.edge_endpoint,
        &args.edge_cert,
        &args.edge_key,
        &args.edge_root_ca,
    )
    .await;
    let our_ts_id = provenance::timestamp_id_from_zid(&edge_session.zid().to_le_bytes())?;
    info!(zid = %edge_session.zid(), endpoint = args.edge_endpoint.as_str(), "publishing session open");

    let local_session = open_session(
        "local",
        &args.local_endpoint,
        &args.local_cert,
        &args.local_key,
        &args.local_root_ca,
    )
    .await;
    info!(zid = %local_session.zid(), endpoint = args.local_endpoint.as_str(), "local session open");

    let (tx, rx) = mpsc::channel::<Outbound>(args.queue_depth);
    let publisher = {
        let m = metrics.clone();
        // Same deviation as above, same reason.
        #[allow(clippy::disallowed_methods)]
        tokio::spawn(async move { run_publisher(rx, ZenohEdge::new(edge_session), m).await })
    };

    let policy = Arc::new(Mutex::new(Policy::new(map, flow.envelope, expires)));
    let mut ingest = tokio::task::JoinSet::new();
    // Bound before the loop: the guard must not be alive across the awaits
    // inside it.
    let subscriptions = policy.lock().unwrap().map().subscriptions().to_vec();
    for prefix in subscriptions {
        let sub = local_session
            .declare_subscriber(prefix.clone())
            .await
            .map_err(|e| anyhow::anyhow!("declare_subscriber({prefix}) failed: {e}"))?;
        let policy = policy.clone();
        let metrics = metrics.clone();
        let tx = tx.clone();
        let label = prefix.to_string();
        ingest.spawn(async move { ingest_loop(sub, label, policy, metrics, tx, our_ts_id).await });
    }
    metrics.local_subscribed.set(1);
    drop(tx);
    info!("republishing");

    // An ingest loop ending means part of the release set has silently stopped
    // crossing. Exit non-zero so the supervisor restarts a whole process rather
    // than running on with a partial feed and a healthy-looking /metrics.
    let ended = tokio::select! {
        r = tokio::signal::ctrl_c() => {
            r.context("install ctrl_c handler")?;
            None
        }
        _ = ingest.join_next() => Some("a release subscription ended"),
    };
    ingest.shutdown().await;
    // The publisher drains until the queue closes, which happens once every
    // ingest task's `tx` clone is dropped.
    let _ = tokio::time::timeout(Duration::from_secs(5), publisher).await;
    match ended {
        None => {
            info!("shutdown signal received");
            Ok(())
        }
        Some(why) => anyhow::bail!("{why} — exiting so the supervisor restarts"),
    }
}

async fn ingest_loop(
    sub: zenoh::pubsub::Subscriber<zenoh::handlers::FifoChannelHandler<zenoh::sample::Sample>>,
    release_prefix: String,
    policy: Arc<Mutex<Policy>>,
    metrics: Metrics,
    tx: mpsc::Sender<Outbound>,
    our_ts_id: zenoh::time::TimestampId,
) {
    loop {
        let sample = match sub.recv_async().await {
            Ok(s) => s,
            Err(e) => {
                warn!(prefix = release_prefix.as_str(), error = %e, "release subscription ended");
                return;
            }
        };
        let staging_key = sample.key_expr().to_string();
        let payload = sample.payload().to_bytes().to_vec();
        metrics
            .received
            .with_label_values(&[release_prefix.as_str()])
            .inc();

        let (decision, shed_warning) = {
            let mut p = policy.lock().unwrap();
            let now = now_ms();
            let d = p.decide(&staging_key, payload.len(), now, Utc::now().date_naive());
            metrics
                .distinct_wire_keys
                .set(p.distinct_keys_seen() as i64);
            // A new key refused at the ceiling never crosses: warn, at most once a minute.
            let w = match &d {
                Decision::Refuse(policy::Refusal::CardinalityShed { .. }) => {
                    p.cardinality_shed_warning(now)
                }
                _ => None,
            };
            (d, w)
        };
        let wire_key = match decision {
            Decision::Publish(k) => k,
            Decision::Refuse(r) => {
                metrics.refused.with_label_values(&[r.label()]).inc();
                match r {
                    // Should be unreachable — the subscriptions ARE the release
                    // set — so reaching it means the router routed us something
                    // we never asked for, and forwarding it would have been an
                    // undeclared disclosure.
                    policy::Refusal::NotReleased
                    | policy::Refusal::Unmappable(_)
                    | policy::Refusal::AgreementExpired(_) => {
                        warn!(staging_key = staging_key.as_str(), "refused: {r}");
                    }
                    // A new key over the distinct-keys ceiling is data that will
                    // never cross: an operator must see it, so it warns (rate-limited
                    // in the policy; the held-back refusals are counted into the next).
                    policy::Refusal::CardinalityShed { limit } => policy::log_cardinality_shed(
                        release_prefix.as_str(),
                        staging_key.as_str(),
                        limit,
                        shed_warning,
                    ),
                    // The rate and size sheds repeat by nature. The counter is the
                    // repeatable observable; the log would be a firehose.
                    _ => debug!(staging_key = staging_key.as_str(), "refused: {r}"),
                }
                continue;
            }
        };

        if sample.timestamp().is_none() {
            metrics.untimestamped.inc();
        }
        if sample.source_info().is_some() {
            metrics.source_info_dropped.inc();
        }

        let out = Outbound {
            wire_key,
            staging_key: staging_key.clone(),
            release_prefix: release_prefix.clone(),
            kind: sample.kind(),
            payload,
            encoding: sample.encoding().clone(),
            timestamp: provenance::rewire_timestamp(sample.timestamp(), our_ts_id),
            attachment: sample.attachment().map(|a| a.to_bytes().to_vec()),
        };

        // try_send, never send: see edge.rs on why blocking is forbidden.
        if let Err(e) = tx.try_send(out) {
            match e {
                mpsc::error::TrySendError::Full(_) => {
                    metrics.dropped.with_label_values(&["queue_full"]).inc();
                }
                mpsc::error::TrySendError::Closed(_) => {
                    warn!("publisher task is gone; ingest stopping");
                    return;
                }
            }
        }
    }
}

fn now_ms() -> u64 {
    use std::sync::OnceLock;
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u64
}

/// Open a client session, retrying forever.
///
/// `connect/exit_on_failure: false` is the load-bearing setting: with it the
/// session opens whether or not the far router is up, and zenoh re-establishes
/// the link on its own when it returns. That is what makes an edge restart
/// invisible to the operator — the process does not need to be restarted, and
/// while the link is down `edge_linked` is 0 and the drops are counted.
async fn open_session(
    role: &str,
    endpoint: &str,
    cert: &Option<String>,
    key: &Option<String>,
    root_ca: &Option<String>,
) -> zenoh::Session {
    loop {
        match build_and_open(endpoint, cert, key, root_ca).await {
            Ok(s) => return s,
            Err(e) => {
                warn!(role, endpoint, error = %e, retry_in_s = SESSION_OPEN_RETRY.as_secs(),
                      "session open failed; retrying");
                tokio::time::sleep(SESSION_OPEN_RETRY).await;
            }
        }
    }
}

async fn build_and_open(
    endpoint: &str,
    cert: &Option<String>,
    key: &Option<String>,
    root_ca: &Option<String>,
) -> Result<zenoh::Session> {
    let mut config = zenoh::Config::default();
    config
        .insert_json5("mode", "\"client\"")
        .map_err(|e| anyhow::anyhow!("config mode: {e}"))?;
    config
        .insert_json5("connect/endpoints", &format!("[\"{endpoint}\"]"))
        .map_err(|e| anyhow::anyhow!("config connect/endpoints: {e}"))?;
    config
        .insert_json5("connect/exit_on_failure", "false")
        .map_err(|e| anyhow::anyhow!("config connect/exit_on_failure: {e}"))?;
    if endpoint.starts_with("tls/") {
        let cert = cert
            .as_deref()
            .context("--cert required for a tls/ endpoint")?;
        let key = key
            .as_deref()
            .context("--key required for a tls/ endpoint")?;
        let root_ca = root_ca
            .as_deref()
            .context("--root-ca required for a tls/ endpoint")?;
        // Same shape as the sibling trial-peer crates: TLS is selected by the
        // endpoint scheme, and hostname verification is off because the router
        // cert SAN binds the mesh IP, not the name we dial through.
        let tls = format!(
            r#"{{ "root_ca_certificate": "{root_ca}", "connect_certificate": "{cert}", "connect_private_key": "{key}", "enable_mtls": true, "verify_name_on_connect": false }}"#
        );
        config
            .insert_json5("transport/link/tls", &tls)
            .map_err(|e| anyhow::anyhow!("config transport/link/tls: {e}"))?;
    }
    zenoh::open(config)
        .await
        .map_err(|e| anyhow::anyhow!("zenoh::open failed: {e}"))
}

async fn serve_metrics(metrics: Metrics, bind: SocketAddr) -> Result<()> {
    let app = Router::new()
        .route(
            "/metrics",
            get(move || {
                let m = metrics.clone();
                async move {
                    match m.render() {
                        Ok(body) => (StatusCode::OK, body),
                        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
                    }
                }
            }),
        )
        .route("/healthz", get(|| async { StatusCode::OK }));
    let listener = tokio::net::TcpListener::bind(bind).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
