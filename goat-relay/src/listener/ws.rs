//! WS-over-TLS listener.
//!
//! Binds a TCP listener on a configurable address (production: `:33073`,
//! the netbird relay port). For each accepted connection:
//!
//! 1. rustls TLS 1.3 handshake.
//! 2. tokio-tungstenite WS upgrade.
//! 3. Path validation against
//!    [`goat_relay_protocol::transport::ws::URL_PATH`] (`/relay`).
//! 4. **Handshake-FSM driver:** read first WS binary frame within
//!    [`HANDSHAKE_TIMEOUT`] (10 s, per spec §3.1), parse via
//!    [`goat_relay_protocol::handshake::HandshakeRequest::parse`],
//!    validate via the supplied [`Validator`], emit `AuthResponse`.
//! 5. **Routing-table registration (step 4):** insert the authenticated
//!    peer-ID + writer-task MPSC sender into the per-shard
//!    [`crate::routing::RoutingTable`].
//! 6. **Forwarding loop (step 4):** the reader half of the WS stream
//!    parses incoming `Transport` frames, rewrites the peer_id field
//!    in-place from destination → source, and dispatches to the
//!    destination peer's writer-task MPSC via the routing table.
//!    The writer task (spawned via `tokio::task::spawn_local`) drains
//!    the MPSC and writes to the WS send half.
//!
//! Per ADR 0709 D7 + D10:
//!
//! - Accept loop runs inside the shard's [`tokio::task::LocalSet`].
//! - Per-connection tasks are spawned via [`tokio::task::spawn_local`];
//!   `tokio::spawn` is forbidden by lint per D7 layer 3.
//! - Per-connection state is `!Send` by virtue of the routing table's
//!   `Rc<RefCell<...>>` (D7 layer 2).
//!
//! TLS provider per ADR 0709 D8: rustls + ring. `aws-lc-rs` migration is
//! deferred per the trigger conditions in the ADR.
//!
//! # Step 4 status
//!
//! Forwarding loop is wired end-to-end for `Transport` (0x03) frames.
//! `Close` (0x04) cleanly terminates the reader loop; other message
//! types (e.g., `HealthCheck` 0x05) are logged + ignored — keepalive
//! integration is step 7. kTLS hand-off is step 5.

use std::cell::{Cell, RefCell};
use std::io;
use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::Arc;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use rustls::ServerConfig;
use tokio::net::{TcpListener, TcpSocket};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::tungstenite::handshake::server::{
    ErrorResponse, Request as WsRequest, Response as WsResponse,
};
use tokio_tungstenite::tungstenite::Message;

use goat_relay_protocol::handshake::{HandshakeRequest, Validator, HANDSHAKE_TIMEOUT};
use goat_relay_protocol::keepalive::Sender as KeepaliveSender;
use goat_relay_protocol::messages::auth::AuthResponse;
use goat_relay_protocol::messages::payload::{
    rewrite_peer_id, TRANSPORT_PAYLOAD_OFFSET, TRANSPORT_PEER_ID_OFFSET,
};
use goat_relay_protocol::messages::{MsgType, PeerId, HEADER_SIZE, PEER_ID_SIZE, PROTOCOL_VERSION};

use crate::keepalive::{self, CloseReason};
use crate::listener::ws_ktls;
use crate::routing::{ForwardError, RoutingTable};

/// Default max concurrent connections per shard — admission control
/// Overridable via `GOAT_RELAY_MAX_CONNS_PER_SHARD`. Bounds task /
/// fd / memory growth and the pre-auth resource commitment a connection
/// flood can force; at capacity new connections are REFUSED (not queued),
/// which is the DoS-resistant behavior. The fd bound is load-bearing: a relay
/// hits EMFILE ("too many open files") under connection accumulation, so both
/// transports cap concurrency.
pub(crate) const DEFAULT_MAX_CONNS_PER_SHARD: usize = 4096;

/// Resolve the per-shard concurrent-connection cap from the environment
/// (`GOAT_RELAY_MAX_CONNS_PER_SHARD`), falling back to
/// [`DEFAULT_MAX_CONNS_PER_SHARD`]. Shared by the WS and QUIC listeners so
/// both transports admit under one identical policy.
#[must_use]
pub(crate) fn max_conns_per_shard() -> usize {
    std::env::var("GOAT_RELAY_MAX_CONNS_PER_SHARD")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_MAX_CONNS_PER_SHARD)
}

/// Timeout for each pre-auth phase (TLS accept, WS upgrade). Without it a
/// slow-loris peer holds a task + buffers indefinitely before auth ever
/// runs. The auth handshake itself is separately bounded by
/// `HANDSHAKE_TIMEOUT`.
const PREAUTH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Graceful-shutdown drain window (C7, F-223). On cancel the accept loop
/// signals every in-flight connection to close, then waits up to this long
/// for them to send their WS Close frame + deregister. **Bounded by design**:
/// connections that don't drain within the window are abandoned exactly as
/// before, so the change can only improve shutdown, never hang it — the
/// worst case degrades to the pre-C7 abrupt abort. Mirrors the QUIC path's
/// `endpoint.close()` + `wait_idle()`.
const WS_DRAIN_GRACE: std::time::Duration = std::time::Duration::from_secs(5);

/// RAII guard for the in-flight-connection count (C7). Increments on
/// construction, decrements on drop — so the count is correct on every exit
/// path (normal return, error, cancel). `Rc<Cell<usize>>` because all
/// connection tasks + the accept loop share one shard's single-threaded
/// `LocalSet` (ADR 0709 D7), so no atomics are needed.
struct ActiveGuard(Rc<Cell<usize>>);

impl ActiveGuard {
    fn new(c: Rc<Cell<usize>>) -> Self {
        c.set(c.get() + 1);
        ActiveGuard(c)
    }
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.0.set(self.0.get().saturating_sub(1));
    }
}

/// Configuration for the WS-over-TLS listener.
pub struct WsListenerConfig {
    /// Address to bind. Production: `[::]:33073` or `0.0.0.0:33073`.
    pub bind_addr: SocketAddr,
    /// Pre-built rustls server config; the binary's caller is responsible
    /// for loading certs (production: from disk; tests: in-memory rcgen).
    pub tls_config: Arc<ServerConfig>,
    /// Address advertised back to clients in the post-handshake
    /// [`AuthResponse`] (`format!("rels://{advertised_addr}")`, spec
    /// §3.3). Distinct from [`bind_addr`] to allow load balancers + DNS
    /// round-robin.
    ///
    /// [`bind_addr`]: WsListenerConfig::bind_addr
    pub advertised_addr: String,
    /// Per-shard routing table — created once per shard and shared
    /// across all per-connection tasks via cheap `Rc` clones.
    pub routing_table: RoutingTable,
    /// Block 33C.1 lever L8.b — when `Some(N)`, attach SO_REUSEPORT_CBPF
    /// for source-IP affinity steering with `src_ip % N` socket selection.
    /// Only ONE shard's listener should pass `Some(N)` (group-wide effect);
    /// other shards pass `None`. Convention: shard 0 attaches; others skip.
    /// Linux-only — on other platforms this field is ignored.
    pub cbpf_steering_num_sockets: Option<u32>,
    /// F-041 mitigation — server-side QUIC-preference bias. When > 0, the
    /// WS accept handler sleeps this many milliseconds after `tcp_accept`
    /// returns and BEFORE the TLS handshake starts. This delays the WS
    /// dialer's return on the netbird daemon side; QUIC's 1-RTT handshake
    /// finishes well within this window, so the daemon's race-dialer
    /// picks QUIC. Default `0` (off, no behavior change). Production /
    /// bench envs that want deterministic QUIC selection set this to 100.
    /// Cost: paid exactly once per peer per daemon process lifetime
    /// (daemons hold the relay connection after race wins).
    pub prefer_quic_bias_ms: u64,
    /// Relay-client auth validator. Shared read-only across all
    /// connection-handler tasks (and across shards) via cheap `Arc`
    /// clones. Production: [`crate::auth::TimedHmacValidator`]. The
    /// binary fails closed if no secret is configured, so this is always
    /// a real validator in the listener path — there is no permissive
    /// default.
    pub validator: Arc<dyn Validator + Send + Sync>,
}

/// Bind a TCP listener with `SO_REUSEADDR` + `SO_REUSEPORT` set, so N
/// shards can each bind the same port and the kernel distributes
/// incoming connections across them. Block 33C.1 lever L8.
///
/// Falls back to a regular bind if the address family is anything other
/// than IPv4 / IPv6 (defensive; the supplied SocketAddr is one of those
/// in practice).
///
/// Verbose logging is enabled so production observability can correlate
/// SO_REUSEPORT setup with bind outcomes. Each step prints the OS fd
/// + result; if any step fails the error propagates to the caller and
///   the listener exits with a logged error.
// `#[allow(dead_code)]` is needed because the only crate-internal callers
// (io_uring path at line ~445 + test paths) are cfg(target_os = "linux")
// or `#[cfg(test)]`-gated; on macOS dev builds nothing reaches the
// wrapper directly. The `_with_cbpf` variant carries the actual logic.
#[allow(dead_code)]
pub(crate) fn bind_listener_reuseport(addr: SocketAddr) -> io::Result<TcpListener> {
    bind_listener_reuseport_with_cbpf(addr, None)
}

/// Variant of [`bind_listener_reuseport`] that optionally attaches a
/// SO_REUSEPORT_CBPF program for source-IP affinity (Block 33C.1 lever
/// L8.b). When `cbpf_steering_num_sockets` is `Some(N)`, after the bind
/// completes the kernel runs a 3-instruction classic-BPF program that
/// returns `src_ip % N` for IPv4 incoming connections — pinning each
/// peer to a single shard so cross-shard MPSC sync is eliminated.
///
/// CBPF is group-wide: ONLY ONE shard should call this with
/// `Some(N)` per group. Convention is shard 0; other shards pass `None`.
/// Repeated attach on the same group replaces the program but wastes
/// syscalls.
///
/// Non-Linux: the flag is silently ignored — `cbpf` module is gated on
/// `cfg(target_os = "linux")`.
pub(crate) fn bind_listener_reuseport_with_cbpf(
    addr: SocketAddr,
    cbpf_steering_num_sockets: Option<u32>,
) -> io::Result<TcpListener> {
    use std::os::fd::AsRawFd;
    let socket = if addr.is_ipv6() {
        TcpSocket::new_v6()?
    } else {
        TcpSocket::new_v4()?
    };
    let fd = socket.as_raw_fd();
    eprintln!("ws bind_listener_reuseport: socket fd={fd} for addr={addr}");
    // SO_REUSEADDR — relay restart doesn't wait for TIME-WAIT sockets.
    socket.set_reuseaddr(true)?;
    eprintln!("ws bind_listener_reuseport: fd={fd} SO_REUSEADDR=true set");
    // SO_REUSEPORT — N shards can each bind the same port.
    socket.set_reuseport(true)?;
    eprintln!("ws bind_listener_reuseport: fd={fd} SO_REUSEPORT=true set");
    socket.bind(addr)?;
    eprintln!("ws bind_listener_reuseport: fd={fd} bind({addr}) ok");
    // Default backlog 1024; matches Linux's somaxconn default. tokio's
    // `listen` returns a TcpListener directly.
    let listener = socket.listen(1024)?;
    eprintln!("ws bind_listener_reuseport: fd={fd} listen() ok");

    // Block 33C.1 lever L8.b — attach CBPF program for source-IP
    // affinity steering. Group-wide effect; only the shard 0 caller
    // passes `Some(N)`.
    #[cfg(target_os = "linux")]
    if let Some(num_sockets) = cbpf_steering_num_sockets {
        if let Err(e) = crate::listener::cbpf::attach_source_ip_steering(&listener, num_sockets) {
            // Don't fail the bind on CBPF attach error — log and proceed
            // with default kernel steering. CBPF is an optimization, not
            // a correctness requirement; older kernels (< 4.5) won't have
            // SO_ATTACH_REUSEPORT_CBPF and we don't want to gate boot on
            // a kernel-version check.
            eprintln!(
                "ws bind_listener_reuseport: fd={fd} CBPF attach failed ({e}); falling back to kernel default RSS steering"
            );
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = cbpf_steering_num_sockets; // suppress unused on non-Linux

    Ok(listener)
}

/// Run the WS-over-TLS accept loop until `cancel` resolves.
///
/// Per-connection processing is spawned on the current
/// [`tokio::task::LocalSet`] via [`tokio::task::spawn_local`]; tasks
/// remain bound to the shard's thread for life.
///
/// # Errors
///
/// Returns `Err` if the initial bind fails. Per-connection failures (TLS
/// handshake errors, WS upgrade errors, wrong-path requests, handshake-FSM
/// rejections, handshake timeouts, forwarding-loop errors) are logged at
/// the connection-task level and do NOT terminate the accept loop.
pub async fn serve<C>(
    config: WsListenerConfig,
    cancel: C,
    bound_addr_tx: tokio::sync::oneshot::Sender<Option<SocketAddr>>,
) -> io::Result<()>
where
    C: std::future::Future<Output = ()>,
{
    // Block 33C.1 lever L8 — bind with SO_REUSEPORT so each shard has
    // its own bound socket on the same port. SO_REUSEADDR is also set
    // so a relay restart doesn't wait for TIME-WAIT sockets to drain.
    // Single-shard deploys behave the same as before (the kernel is
    // happy with N=1 SO_REUSEPORT group).
    //
    // Block 33C.1 lever L8.b — when `config.cbpf_steering_num_sockets`
    // is `Some(N)`, the kernel runs a CBPF program returning
    // `src_ip % N` to steer same-source-IP traffic to the same shard
    // (eliminating cross-shard MPSC sync). Group-wide effect; only one
    // shard's listener should pass `Some(N)`.
    let listener = match bind_listener_reuseport_with_cbpf(
        config.bind_addr,
        config.cbpf_steering_num_sockets,
    ) {
        Ok(l) => l,
        Err(e) => {
            let _ = bound_addr_tx.send(None);
            return Err(e);
        }
    };
    let local_addr = listener.local_addr()?;
    let _ = bound_addr_tx.send(Some(local_addr));
    let acceptor = TlsAcceptor::from(config.tls_config);
    let advertised_addr = Arc::new(config.advertised_addr);
    let routing_table = config.routing_table;
    let validator = config.validator;
    let prefer_quic_bias_ms = config.prefer_quic_bias_ms;
    if prefer_quic_bias_ms > 0 {
        eprintln!(
            "ws listener: F-041 prefer-QUIC bias active — sleeping {prefer_quic_bias_ms}ms after each tcp_accept before TLS handshake (delays WS dialer return so QUIC wins netbird race-dialer)"
        );
    }
    eprintln!("ws listener bound on {local_addr}");

    // Admission control — per-shard concurrent-connection cap.
    let max_conns = max_conns_per_shard();
    let conn_limit = Arc::new(tokio::sync::Semaphore::new(max_conns));
    eprintln!("ws listener: admission control = {max_conns} concurrent connections/shard");

    // C7 graceful drain — broadcast shutdown signal + in-flight count.
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let active = Rc::new(Cell::new(0usize));

    tokio::pin!(cancel);

    loop {
        tokio::select! {
            _ = &mut cancel => {
                // C7 — signal every in-flight connection to close, then wait
                // (BOUNDED by WS_DRAIN_GRACE) for them to send their WS Close
                // frame + deregister. Worst case degrades to the pre-C7
                // abrupt abort: after the grace we return regardless and the
                // LocalSet drops any stragglers (cannot hang shutdown).
                let n = active.get();
                eprintln!("ws listener: cancel received; draining {n} connection(s) (grace {}s)", WS_DRAIN_GRACE.as_secs());
                let _ = shutdown_tx.send(true);
                let deadline = tokio::time::Instant::now() + WS_DRAIN_GRACE;
                while active.get() > 0 && tokio::time::Instant::now() < deadline {
                    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                }
                eprintln!("ws listener: drain complete; {} connection(s) still active at return", active.get());
                return Ok(());
            }
            accept_result = listener.accept() => {
                match accept_result {
                    Ok((stream, peer)) => {
                        // F-222: refuse (don't queue) when at the per-shard
                        // cap — bounds pre-auth resource use under a flood.
                        let permit = match Arc::clone(&conn_limit).try_acquire_owned() {
                            Ok(p) => p,
                            Err(_) => {
                                drop(stream); // closes the socket
                                continue;
                            }
                        };
                        let acceptor = acceptor.clone();
                        let advertised = Arc::clone(&advertised_addr);
                        let routing = routing_table.clone();
                        let validator = Arc::clone(&validator);
                        let conn_shutdown_rx = shutdown_rx.clone();
                        let conn_active = Rc::clone(&active);
                        // ADR 0709 D7: spawn_local keeps the per-connection
                        // task on the shard's LocalSet. tokio::spawn is
                        // forbidden by the workspace clippy.toml.
                        tokio::task::spawn_local(async move {
                            // Hold the admission permit for the whole
                            // connection lifetime; released on task exit so
                            // the slot frees for a new connection.
                            let _permit = permit;
                            // F-041 mitigation — server-side QUIC-preference
                            // bias. Sleep BEFORE TLS handshake so the WS
                            // dialer's return is delayed; the netbird daemon's
                            // race-dialer (a true first-to-finish race with no
                            // QUIC preference) picks QUIC every time. Cost
                            // paid exactly once per peer per daemon lifetime.
                            if prefer_quic_bias_ms > 0 {
                                tokio::time::sleep(
                                    std::time::Duration::from_millis(prefer_quic_bias_ms),
                                ).await;
                            }
                            handle_connection(stream, peer, acceptor, advertised, routing, validator, conn_shutdown_rx, conn_active).await;
                        });
                    }
                    Err(e) => {
                        eprintln!("ws listener: accept failed: {e}");
                    }
                }
            }
        }
    }
}

/// Handle a single inbound connection through its full lifecycle:
/// TLS → WS upgrade → auth handshake → routing registration →
/// forwarding loop → deregistration on close.
// 8 args: the connection's full dependency set + the C7 drain context
// (`shutdown_rx`, `active`). Bundling into a struct buys nothing here —
// each is independently sourced in `serve()`'s accept loop.
#[allow(clippy::too_many_arguments)]
async fn handle_connection(
    stream: tokio::net::TcpStream,
    peer: SocketAddr,
    acceptor: TlsAcceptor,
    advertised_addr: Arc<String>,
    routing_table: RoutingTable,
    validator: Arc<dyn Validator + Send + Sync>,
    shutdown_rx: watch::Receiver<bool>,
    active: Rc<Cell<usize>>,
) {
    // Step 5 (post-deferral): cfg-gated kTLS hand-off. On Linux, the
    // helper wraps the TcpStream in `ktls::CorkStream`, runs the
    // rustls handshake, and pushes AEAD keys to the kernel; the
    // returned `KtlsStream` delegates plaintext I/O to the underlying
    // TCP socket while the kernel handles AEAD. On non-Linux, the
    // helper is a pass-through to `acceptor.accept(...)`. Validation
    // gates step 8 (bench-env smoke).
    // F-222: bound the TLS accept so a peer that completes the TCP
    // handshake but stalls the TLS handshake cannot hold this task forever.
    let tls_stream = match tokio::time::timeout(
        PREAUTH_TIMEOUT,
        ws_ktls::accept_tls(acceptor, stream),
    )
    .await
    {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            eprintln!("ws: TLS handshake / kTLS hand-off failed for {peer}: {e}");
            return;
        }
        Err(_) => {
            eprintln!("ws: TLS handshake timed out for {peer} (>{}s)", PREAUTH_TIMEOUT.as_secs());
            return;
        }
    };

    handle_post_tls(tls_stream, peer, advertised_addr, routing_table, validator, shutdown_rx, active).await;
}

/// Post-TLS body shared between the kTLS path ([`handle_connection`])
/// and the io_uring path ([`handle_connection_uring`]). Owns the WS
/// upgrade, auth handshake, routing-table registration, writer / reader
/// task split, keepalive driver, and deregistration on close.
///
/// Generic over `S: WsTransport` so the kTLS path's `Box<dyn WsTransport>`
/// and the uring path's `UringTcpStream`-backed userspace `TlsStream`
/// both flow through one body. Block 33C.1 lever L5 (ADR 0711).
async fn handle_post_tls<S>(
    tls_stream: S,
    peer: SocketAddr,
    advertised_addr: Arc<String>,
    routing_table: RoutingTable,
    validator: Arc<dyn Validator + Send + Sync>,
    shutdown_rx: watch::Receiver<bool>,
    active: Rc<Cell<usize>>,
) where
    S: ws_ktls::WsTransport,
{
    // `clippy::result_large_err`: Err type is `tungstenite::http::Response<Option<String>>`
    // (~136 bytes), forced by the `accept_hdr_async` callback signature. We can't box
    // it without forking tokio-tungstenite, and the path-check error path is rare.
    #[allow(clippy::result_large_err)]
    let path_check = |req: &WsRequest, response: WsResponse| -> Result<WsResponse, ErrorResponse> {
        let expected = goat_relay_protocol::transport::ws::URL_PATH;
        if req.uri().path() != expected {
            let resp = tokio_tungstenite::tungstenite::http::Response::builder()
                .status(404)
                .body(Some(format!(
                    "expected path '{expected}', got '{}'\n",
                    req.uri().path()
                )))
                .expect("static 404 response builds");
            return Err(resp);
        }
        Ok(response)
    };

    // F-222: bound the WS upgrade — a peer that completes TLS but dribbles
    // (or never sends) the HTTP upgrade must not hold this task open.
    let mut ws = match tokio::time::timeout(
        PREAUTH_TIMEOUT,
        tokio_tungstenite::accept_hdr_async(tls_stream, path_check),
    )
    .await
    {
        Ok(Ok(w)) => w,
        Ok(Err(e)) => {
            eprintln!("ws: WS upgrade failed for {peer}: {e}");
            return;
        }
        Err(_) => {
            eprintln!("ws: WS upgrade timed out for {peer} (>{}s)", PREAUTH_TIMEOUT.as_secs());
            return;
        }
    };

    let auth_peer_id = match tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        drive_handshake(&mut ws, validator.as_ref(), advertised_addr.as_str()),
    )
    .await
    {
        Ok(Ok(peer_id)) => peer_id,
        Ok(Err(e)) => {
            eprintln!("ws: handshake failed for {peer}: {e}; closing silently");
            return;
        }
        Err(_) => {
            eprintln!(
                "ws: handshake timed out for {peer} (>{}s); closing silently",
                HANDSHAKE_TIMEOUT.as_secs()
            );
            return;
        }
    };

    eprintln!("ws: connection from {peer} authenticated; entering forwarding loop");

    // Register in the routing table; spawn the writer task; spawn the
    // keepalive task; run the reader loop inline. When the reader
    // returns, deregister, abort the keepalive task, and let the writer
    // drain + exit.
    // Bounded queue per Block 33C.1 lever L4 / audit F-A4 — eliminates the
    // per-message linked-list-node allocation that unbounded_channel does.
    // Capacity 256 → ~2 MB max per connection. Forward path uses try_send
    // and drops on Full (no head-of-line blocking from slow downstream).
    let (tx, rx) = mpsc::channel::<Bytes>(crate::routing::WRITER_QUEUE_CAPACITY);
    let tx_for_keepalive = tx.clone();
    // Retained so the reader-exit teardown can deregister IDENTITY-AWARE
    // (remove_self) — only evicting the routing entry if it is still this
    // connection's, never a peer that reconnected meanwhile.
    let tx_for_dereg = tx.clone();
    if let Some(prior) = routing_table.insert(auth_peer_id, tx) {
        eprintln!(
            "ws: peer {auth_peer_id:?} reconnected; closing prior connection's writer"
        );
        drop(prior);
    }
    // Tag this peer's transport for the stats surface (track netbird-wedge-rca):
    // per-peer QUIC-vs-WS is otherwise un-observable from outside the relay.
    routing_table.set_transport(&auth_peer_id, crate::routing::Transport::Ws);

    // C7 — count this connection as in-flight for the duration of its
    // forwarding loop. `serve()` waits (bounded) for this count to reach
    // zero on shutdown so live connections drain gracefully. The guard
    // decrements on ANY exit path (RAII), so the count can't leak.
    let _active_guard = ActiveGuard::new(active);

    let (ws_writer, ws_reader) = ws.split();

    let writer_task = tokio::task::spawn_local(run_writer(ws_writer, rx));

    // Step 7: keepalive driver. The lib's Sender state machine is shared
    // (Rc<RefCell<...>>) between the keepalive task (interval + timeout
    // ticks) and the reader (on_response when it sees inbound HC). The
    // close-signal oneshot lets the keepalive task interrupt the reader
    // on DeclareTimeout.
    let ka_sender = Rc::new(RefCell::new(KeepaliveSender::new(
        keepalive::attempt_threshold_from_env(),
    )));
    let (close_signal_tx, close_signal_rx) = oneshot::channel::<CloseReason>();
    let keepalive_task = tokio::task::spawn_local(keepalive::run(
        Rc::clone(&ka_sender),
        tx_for_keepalive,
        close_signal_tx,
        keepalive::KeepaliveTimings::default(),
    ));

    let reader_outcome = run_reader(
        ws_reader,
        auth_peer_id,
        routing_table.clone(),
        ka_sender,
        close_signal_rx,
        shutdown_rx,
    )
    .await;

    // Reader exited — deregister IDENTITY-AWARE (no-op if the peer already
    // reconnected and a fresh handle owns the entry, F-218), abort
    // keepalive (drops its tx clone so the writer's rx eventually closes),
    // then DROP the last writer-channel sender so the writer's `rx` closes
    // and it runs its `ws_writer.close()` — sending a clean WS Close frame
    // (C7 graceful drain). Without this drop, `tx_for_dereg` stays alive
    // across `writer_task.await`, the writer parks on `rx.recv()` (the
    // socket is still alive on a graceful shutdown), and the teardown hangs
    // until the drain grace expires + the peer never gets a Close frame —
    // exactly what the C7 empirical test caught. Pre-C7 this was masked
    // because connections only closed once the socket was already dead.
    routing_table.remove_self(&auth_peer_id, &tx_for_dereg);
    keepalive_task.abort();
    let _ = keepalive_task.await;
    drop(tx_for_dereg);
    let _ = writer_task.await;

    eprintln!(
        "ws: connection from {peer} (peer_id={auth_peer_id:?}) closed: {reader_outcome:?}"
    );
}

// ============================================================================
// io_uring path — Block 33C.1 lever L5 (ADR 0711). Linux-only.
// ============================================================================
//
// Sibling of `serve` that drives the data-plane I/O via tokio-uring (io_uring
// SQEs) instead of mio (epoll). Userspace TLS only on this path — kTLS hand-
// off requires a tokio::net::TcpStream's fd type and combining kTLS with
// io_uring is a follow-up integration. M-019 measures the L5 lift in
// isolation against the M-018 mio+kTLS baseline.
//
// Selected by the operator at boot via the `GOAT_RELAY_USE_URING=1` env var
// (consumed in `main.rs`). Both paths coexist in the binary; flipping back
// is one env-var unset away.

/// Bind a TCP listener with `SO_REUSEADDR` + `SO_REUSEPORT` set
/// (reusing the proven [`bind_listener_reuseport`] path), then convert
/// the tokio listener to a `std::net::TcpListener` and wrap as a
/// [`tokio_uring::net::TcpListener`]. The shape mirrors the tokio
/// listener for the io_uring path; the kernel-side SO_REUSEPORT
/// semantics are identical (each shard's bound socket is in the same
/// group; the kernel distributes incoming connections across shards
/// by source-tuple hash by default).
///
/// Why route through `bind_listener_reuseport` instead of duplicating
/// via `socket2`: empirically, a from-scratch socket2 bind path
/// produced `EADDRINUSE` on shards 1..N during the M-019 first-light —
/// some subtle interaction between `socket2::set_reuse_port` and the
/// per-shard ordering. The tokio path is known-good (M-014 → M-018
/// production verdicts), so reusing it eliminates that variable.
#[cfg(target_os = "linux")]
fn bind_uring_listener_reuseport(
    addr: SocketAddr,
) -> io::Result<tokio_uring::net::TcpListener> {
    let tokio_listener = bind_listener_reuseport(addr)?;
    // into_std() removes the fd from tokio's mio reactor. tokio_uring's
    // from_std then registers it with the io_uring driver. Both are
    // synchronous fd-level handoffs; no kernel state changes.
    let std_listener: std::net::TcpListener = tokio_listener.into_std()?;
    Ok(tokio_uring::net::TcpListener::from_std(std_listener))
}

/// L5 accept loop. Drives the data-plane via io_uring (SQPOLL +
/// multishot recv when tokio-uring 0.5 enables them). Userspace TLS;
/// kTLS deferred per the path comment above.
#[cfg(target_os = "linux")]
pub async fn serve_uring<C>(
    config: WsListenerConfig,
    cancel: C,
    bound_addr_tx: tokio::sync::oneshot::Sender<Option<SocketAddr>>,
) -> io::Result<()>
where
    C: std::future::Future<Output = ()>,
{
    let listener = match bind_uring_listener_reuseport(config.bind_addr) {
        Ok(l) => l,
        Err(e) => {
            let _ = bound_addr_tx.send(None);
            return Err(e);
        }
    };
    let local_addr = listener.local_addr()?;
    let _ = bound_addr_tx.send(Some(local_addr));
    let acceptor = TlsAcceptor::from(config.tls_config);
    let advertised_addr = Arc::new(config.advertised_addr);
    let routing_table = config.routing_table;
    let validator = config.validator;
    let prefer_quic_bias_ms = config.prefer_quic_bias_ms;
    if prefer_quic_bias_ms > 0 {
        eprintln!(
            "ws-uring listener: F-041 prefer-QUIC bias active — sleeping {prefer_quic_bias_ms}ms after each tcp_accept before TLS handshake"
        );
    }
    eprintln!("ws-uring listener bound on {local_addr} (L5 path; userspace TLS)");

    // C7 graceful drain — same bounded shutdown as the mio path.
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let active = Rc::new(Cell::new(0usize));

    tokio::pin!(cancel);

    loop {
        tokio::select! {
            _ = &mut cancel => {
                let n = active.get();
                eprintln!("ws-uring listener: cancel received; draining {n} connection(s) (grace {}s)", WS_DRAIN_GRACE.as_secs());
                let _ = shutdown_tx.send(true);
                let deadline = tokio::time::Instant::now() + WS_DRAIN_GRACE;
                while active.get() > 0 && tokio::time::Instant::now() < deadline {
                    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                }
                eprintln!("ws-uring listener: drain complete; {} connection(s) still active at return", active.get());
                return Ok(());
            }
            accept_result = listener.accept() => {
                match accept_result {
                    Ok((uring_stream, peer)) => {
                        let acceptor = acceptor.clone();
                        let advertised = Arc::clone(&advertised_addr);
                        let routing = routing_table.clone();
                        let validator = Arc::clone(&validator);
                        let conn_shutdown_rx = shutdown_rx.clone();
                        let conn_active = Rc::clone(&active);
                        tokio::task::spawn_local(async move {
                            // F-041 mitigation — same bias as the mio path.
                            if prefer_quic_bias_ms > 0 {
                                tokio::time::sleep(
                                    std::time::Duration::from_millis(prefer_quic_bias_ms),
                                ).await;
                            }
                            handle_connection_uring(
                                uring_stream,
                                peer,
                                acceptor,
                                advertised,
                                routing,
                                validator,
                                conn_shutdown_rx,
                                conn_active,
                            )
                            .await;
                        });
                    }
                    Err(e) => {
                        eprintln!("ws-uring listener: accept failed: {e}");
                    }
                }
            }
        }
    }
}

/// L5 per-connection handler. Wraps the `tokio_uring::net::TcpStream`
/// in [`UringTcpStream`](crate::listener::uring_compat::UringTcpStream)
/// (which exposes tokio's `AsyncRead + AsyncWrite + Unpin`), runs
/// userspace `tokio_rustls` accept, then hands off to the shared
/// [`handle_post_tls`] body.
#[cfg(target_os = "linux")]
#[allow(clippy::too_many_arguments)] // + the C7 drain context, as handle_connection
async fn handle_connection_uring(
    uring_stream: tokio_uring::net::TcpStream,
    peer: SocketAddr,
    acceptor: TlsAcceptor,
    advertised_addr: Arc<String>,
    routing_table: RoutingTable,
    validator: Arc<dyn Validator + Send + Sync>,
    shutdown_rx: watch::Receiver<bool>,
    active: Rc<Cell<usize>>,
) {
    use crate::listener::uring_compat::UringTcpStream;
    let async_stream = UringTcpStream::new(uring_stream);
    let tls_stream = match acceptor.accept(async_stream).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("ws-uring: TLS handshake failed for {peer}: {e}");
            return;
        }
    };
    handle_post_tls(tls_stream, peer, advertised_addr, routing_table, validator, shutdown_rx, active).await;
}

/// Read first frame, parse as [`HandshakeRequest`], validate, send
/// [`AuthResponse`]. Returns the authenticated peer-id on success.
async fn drive_handshake<S>(
    ws: &mut tokio_tungstenite::WebSocketStream<S>,
    validator: &(impl Validator + ?Sized),
    advertised_addr: &str,
) -> Result<PeerId, String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let frame = ws
        .next()
        .await
        .ok_or_else(|| "client closed before handshake".to_string())?
        .map_err(|e| format!("ws read: {e}"))?;
    let bytes = match frame {
        Message::Binary(b) => b,
        other => return Err(format!("expected binary frame as handshake, got {other:?}")),
    };
    let req = HandshakeRequest::parse(&bytes).map_err(|e| format!("handshake parse: {e}"))?;
    if !req.is_modern() {
        return Err("deprecated Hello handshake path not supported".to_string());
    }
    validator
        .validate(req.peer_id(), req.credential_bytes())
        .map_err(|e| format!("validator rejected: {e}"))?;

    let peer_id = *req.peer_id();
    // ADR 1021 migration telemetry — emit the auth-algo (1 = unbound,
    // 2 = peer-bound) so operators can grep handshake logs to confirm the
    // fleet has moved to peer-bound before flipping GOAT_RELAY_REQUIRE_PEER_BOUND.
    let auth_algo = req.credential_bytes().first().copied().unwrap_or(0);
    eprintln!("ws: peer {peer_id:?} authenticated (auth_algo={auth_algo})");
    let response = AuthResponse::new(format!("rels://{advertised_addr}"))
        .map_err(|e| format!("AuthResponse::new: {e}"))?;
    ws.send(Message::Binary(Bytes::from(response.marshal())))
        .await
        .map_err(|e| format!("ws send AuthResponse: {e}"))?;
    Ok(peer_id)
}

/// Reader loop: parse each incoming binary frame as a relay-message.
/// For `Transport` (0x03) frames, rewrite the peer_id field in place
/// from destination → source and dispatch via the routing table. For
/// `HealthCheck` (0x05) frames, call `on_response` on the shared
/// keepalive `Sender` to reset the failure counter. For `Close` (0x04)
/// frames, return cleanly.
///
/// The reader also observes a close-signal `oneshot::Receiver`: if the
/// keepalive task fires `DeclareTimeout`, the reader returns immediately
/// without waiting for the next frame.
async fn run_reader<S>(
    mut ws_reader: futures_util::stream::SplitStream<tokio_tungstenite::WebSocketStream<S>>,
    auth_peer_id: PeerId,
    routing_table: RoutingTable,
    ka_sender: Rc<RefCell<KeepaliveSender>>,
    close_signal_rx: oneshot::Receiver<CloseReason>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> ReaderOutcome
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    tokio::pin!(close_signal_rx);
    loop {
        let msg = tokio::select! {
            biased;
            signal = &mut close_signal_rx => {
                return match signal {
                    Ok(reason) => ReaderOutcome::KeepaliveSignal(reason),
                    Err(_) => ReaderOutcome::StreamEnded,
                };
            }
            // C7 graceful drain — listener is shutting down. Return so the
            // teardown deregisters + the writer task sends a WS Close frame
            // (run_writer's `ws_writer.close()`), giving the peer a clean
            // close instead of an abrupt RST.
            res = shutdown_rx.changed() => {
                if res.is_err() || *shutdown_rx.borrow() {
                    return ReaderOutcome::Shutdown;
                }
                continue;
            }
            msg = ws_reader.next() => msg,
        };
        let Some(msg) = msg else {
            return ReaderOutcome::StreamEnded;
        };
        let bytes = match msg {
            Ok(Message::Binary(b)) => b,
            Ok(Message::Close(_)) => return ReaderOutcome::PeerClosed,
            Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => continue,
            Ok(other) => {
                eprintln!(
                    "ws: peer_id={auth_peer_id:?} sent unexpected non-binary frame {other:?}; ignoring"
                );
                continue;
            }
            Err(e) => {
                return ReaderOutcome::ReadError(format!("{e}"));
            }
        };

        if bytes.len() < HEADER_SIZE {
            eprintln!(
                "ws: peer_id={auth_peer_id:?} sent {} bytes (<header size {}); dropping",
                bytes.len(),
                HEADER_SIZE
            );
            continue;
        }
        if bytes[0] != PROTOCOL_VERSION {
            eprintln!(
                "ws: peer_id={auth_peer_id:?} sent unsupported version 0x{:02x}; dropping",
                bytes[0]
            );
            continue;
        }
        let msg_type = match MsgType::try_from(bytes[1]) {
            Ok(m) => m,
            Err(_) => {
                eprintln!(
                    "ws: peer_id={auth_peer_id:?} sent unknown msg_type 0x{:02x}; dropping",
                    bytes[1]
                );
                continue;
            }
        };

        // Any successfully-parsed inbound frame is evidence the peer is
        // alive — credit liveness on data, not only on HealthCheck echoes
        // Tight borrow scope (no `.await` held).
        ka_sender.borrow_mut().mark_alive();

        match msg_type {
            MsgType::Transport => {
                if let Err(e) = dispatch_transport(bytes, auth_peer_id, &routing_table) {
                    eprintln!(
                        "ws: dispatch failed for peer_id={auth_peer_id:?}: {e}; dropping frame"
                    );
                }
            }
            MsgType::Close => return ReaderOutcome::AppClose,
            MsgType::HealthCheck => {
                // Inbound HC is the peer's echo of our HC. Liveness was
                // already credited above (mark_alive); HC needs no forward.
            }
            MsgType::SubscribePeerState | MsgType::UnsubscribePeerState => {
                use goat_relay_protocol::messages::peer_state::{parse_peer_list, PeerListKind};
                match parse_peer_list(&bytes) {
                    Ok((PeerListKind::Subscribe, subjects)) => {
                        routing_table.subscribe(auth_peer_id, &subjects);
                    }
                    Ok((PeerListKind::Unsubscribe, subjects)) => {
                        routing_table.unsubscribe(auth_peer_id, &subjects);
                    }
                    Ok((other_kind, _)) => {
                        eprintln!(
                            "ws: peer_id={auth_peer_id:?} parse_peer_list returned non-(Sub/Unsub) kind {other_kind:?}; dropping"
                        );
                    }
                    Err(e) => {
                        eprintln!(
                            "ws: peer_id={auth_peer_id:?} parse_peer_list failed: {e}; dropping"
                        );
                    }
                }
            }
            other => {
                eprintln!(
                    "ws: peer_id={auth_peer_id:?} sent {other:?} (mid-conn); ignoring"
                );
            }
        }
    }
}

/// Why the reader loop returned, for diagnostic logging.
///
/// `#[allow(dead_code)]` — contained values are surfaced exclusively
/// through the `Debug` impl in [`handle_connection`]'s log line;
/// rustc's dead-code lint doesn't model this as a "read."
#[derive(Debug)]
#[allow(dead_code)]
enum ReaderOutcome {
    /// Peer sent a WS Close frame.
    PeerClosed,
    /// Peer sent an application-level `Close` (msg_type 0x04).
    AppClose,
    /// Underlying WS stream ended without a Close frame.
    StreamEnded,
    /// Underlying WS stream returned an error.
    ReadError(String),
    /// Keepalive task signalled the connection to close.
    KeepaliveSignal(CloseReason),
    /// The listener is shutting down (C7 graceful drain) — teardown runs
    /// normally so the peer gets a clean WS Close frame.
    Shutdown,
}

/// Dispatch a Transport frame: extract dest peer-id, look up in routing
/// table, rewrite peer_id field in place to the source (the authenticated
/// sender's id), forward to the destination's writer.
///
/// Takes `Bytes` (from tokio-tungstenite 0.26+ Message::Binary). Converts
/// to `BytesMut` via `try_into_mut` for the in-place rewrite — when the
/// Bytes is uniquely held (the common case at read time), this is zero-
/// copy; otherwise falls back to one allocation (still better than the
/// pre-0.26 path which had a writer-side `.to_vec()` per send).
/// Zero-copy audit F-A1.
fn dispatch_transport(
    buf: Bytes,
    source_peer_id: PeerId,
    routing_table: &RoutingTable,
) -> Result<(), String> {
    if buf.len() < TRANSPORT_PAYLOAD_OFFSET {
        return Err(format!(
            "Transport frame too short ({} bytes; need at least {})",
            buf.len(),
            TRANSPORT_PAYLOAD_OFFSET
        ));
    }
    let mut dest_id_bytes = [0u8; PEER_ID_SIZE];
    dest_id_bytes
        .copy_from_slice(&buf[TRANSPORT_PEER_ID_OFFSET..TRANSPORT_PAYLOAD_OFFSET]);
    let dest_id = PeerId::from_bytes(dest_id_bytes);

    // Zero-copy mutate path: convert Bytes → BytesMut without alloc when
    // we're the unique holder (we just received from the WS reader, so
    // we always are in steady state). On rare contention falls back to
    // one allocation.
    let mut buf_mut = match buf.try_into_mut() {
        Ok(m) => m,
        Err(b) => bytes::BytesMut::from(&b[..]),
    };

    // In-place rewrite: destination peer-id field → source peer-id.
    // After this the buffer is wire-correct for the s→c direction.
    rewrite_peer_id(&mut buf_mut, &source_peer_id)
        .map_err(|e| format!("rewrite_peer_id: {e}"))?;

    let frame = buf_mut.freeze();
    // `forward_data` attributes the bytes to the source + destination
    // per-peer counters (ADR 1055 §2b) on top of the delivery + drop
    // accounting `forward` already does.
    match routing_table.forward_data(&source_peer_id, &dest_id, frame) {
        // All drop reasons are silent per spec §3.3 (the relay never
        // back-pressures the source) and are now COUNTED in `forward()`'s
        // DropCounters — do NOT log per frame here: a hostile or
        // slow peer can flood this path, and a per-frame `eprintln` on the
        // hot path is itself a log-flood DoS. The counters are the
        // observable; a stats/health surface reads `RoutingTable::drop_stats`.
        Ok(()) | Err(ForwardError::PeerNotFound | ForwardError::WriterClosed | ForwardError::WriterFull) => Ok(()),
    }
}

/// Writer task: drains the per-connection MPSC and sends each frame as
/// `Message::Binary` on the WS send half. Exits when the MPSC closes
/// (sender dropped — i.e., the routing table dropped the entry) or the
/// underlying WS send fails.
///
/// **Block 33C.1 lever L6 — batched I/O.** Drains up to
/// [`crate::routing::WRITER_QUEUE_CAPACITY`] frames per wake via
/// [`mpsc::Receiver::recv_many`], `feed`s each into the tungstenite
/// writer (buffering at the WS frame level), then issues a single
/// `flush` per batch. tungstenite's `flush` writes its accumulated
/// frame buffer to the underlying TLS stream in one `write()` call,
/// which on Linux with kTLS hand-off is one `write()` syscall + kernel
/// AEAD; on userspace rustls is one `write()` plus rustls-side AEAD
/// per fragment. Either way, the syscall count drops from `N` per
/// batch to `~1` per batch. Co-designed with L4 (bounded MPSC) — the
/// queue cap caps `recv_many` at 256, which bounds worst-case batch
/// latency.
///
/// **F-225 — bounded writes.** Every `feed`/`flush`/`close` is wrapped in
/// [`WRITE_IDLE_TIMEOUT`]. A consumer whose TCP socket is alive but never
/// drains (zero-window / dead-but-not-RST — the F-213 wedged-tunnel class)
/// would otherwise park `flush().await` forever, leaking this writer task
/// and its buffers and hanging the reader-exit cleanup that awaits it. The
/// timeout makes the writer self-terminate so the connection is reaped.
const WRITE_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

async fn run_writer<S>(
    mut ws_writer: futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<S>,
        Message,
    >,
    mut rx: mpsc::Receiver<Bytes>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut batch: Vec<Bytes> = Vec::with_capacity(crate::routing::WRITER_QUEUE_CAPACITY);
    'outer: loop {
        batch.clear();
        let n = rx
            .recv_many(&mut batch, crate::routing::WRITER_QUEUE_CAPACITY)
            .await;
        if n == 0 {
            // All senders dropped (routing table removed the peer).
            break;
        }
        for frame in batch.drain(..) {
            // tokio-tungstenite 0.26+ Message::Binary takes Bytes directly —
            // zero-copy forward path matches the QUIC writer's send_datagram
            // shape. Pre-0.26 this was Message::Binary(frame.to_vec()) which
            // allocated + memcpy'd the entire payload per send; eliminated
            // by zero-copy audit F-A1.
            //
            // F-225: bound the write — a wedged consumer must not park us
            // here forever. Timeout OR send-error → abandon the connection.
            match tokio::time::timeout(WRITE_IDLE_TIMEOUT, ws_writer.feed(Message::Binary(frame)))
                .await
            {
                Ok(Ok(())) => {}
                Ok(Err(_)) | Err(_) => break 'outer,
            }
        }
        // F-225: bound the flush too (this is where a zero-window peer
        // actually stalls — feed buffers, flush hits the socket).
        match tokio::time::timeout(WRITE_IDLE_TIMEOUT, ws_writer.flush()).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) | Err(_) => break,
        }
    }
    // Best-effort close on the way out — also bounded so a wedged socket
    // cannot hang the writer task's exit.
    let _ = tokio::time::timeout(WRITE_IDLE_TIMEOUT, ws_writer.close()).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use bytes::Bytes;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    use tokio::sync::oneshot;

    use goat_relay_protocol::messages::auth::Auth;
    use goat_relay_protocol::messages::payload::Transport;
    use goat_relay_protocol::messages::{PeerId, PEER_ID_SIZE};

    /// L8 same-process two-thread SO_REUSEPORT bind test. Reproduces the
    /// production failure mode where two shards in the same process try
    /// to bind the same TCP port. Per Linux SO_REUSEPORT semantics, both
    /// binds should succeed when both sockets have SO_REUSEPORT set
    /// before bind(). Each thread runs in its own current_thread runtime
    /// to mirror the goat-relay shard topology.
    ///
    /// On macOS, SO_REUSEPORT semantics differ from Linux (allows
    /// multiple binds without strict same-uid + same-group checks)
    /// so this test is more meaningful on Linux. Run via:
    ///   cargo test --bin goat-relay l8_so_reuseport_two_thread_same_port_succeeds
    /// or in CI's Linux job.
    #[test]
    fn l8_so_reuseport_two_thread_same_port_succeeds() {
        use std::net::SocketAddr;
        use std::thread;
        // Pick a high port unlikely to be in use; using port 0 wouldn't
        // exercise the same-port-shared semantics SO_REUSEPORT cares
        // about.
        let port: u16 = 31_173;
        let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();

        let h0 = thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("rt");
            rt.block_on(async move { super::bind_listener_reuseport(addr) })
        });
        let h1 = thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("rt");
            rt.block_on(async move { super::bind_listener_reuseport(addr) })
        });
        let r0 = h0.join().expect("thread 0 joined");
        let r1 = h1.join().expect("thread 1 joined");
        match (&r0, &r1) {
            (Ok(_), Ok(_)) => {} // expected: both shards bind same port via SO_REUSEPORT
            other => panic!(
                "L8 same-process two-thread bind FAILED — both should succeed with SO_REUSEPORT. Got {other:?}"
            ),
        }
        drop(r0);
        drop(r1);
    }

    fn self_signed_localhost() -> (Vec<CertificateDer<'static>>, PrivateKeyDer<'static>) {
        let key_pair = rcgen::KeyPair::generate().expect("rcgen key");
        let cert = rcgen::CertificateParams::new(vec!["localhost".to_string()])
            .expect("rcgen params")
            .self_signed(&key_pair)
            .expect("self-sign");
        let cert_der = CertificateDer::from(cert.der().to_vec());
        let key_der: PrivateKeyDer<'static> =
            PrivateKeyDer::from(PrivatePkcs8KeyDer::from(key_pair.serialize_der()));
        (vec![cert_der], key_der)
    }

    fn test_server_config(
        certs: Vec<CertificateDer<'static>>,
        key: PrivateKeyDer<'static>,
    ) -> Arc<ServerConfig> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .expect("server config");
        // F-022: required for the cfg(target_os = "linux") kTLS path
        // in `super::handle_connection` to succeed. macOS userspace
        // path doesn't use it but the opt-in is harmless.
        config.enable_secret_extraction = true;
        Arc::new(config)
    }

    fn test_client_config(server_cert: CertificateDer<'static>) -> Arc<rustls::ClientConfig> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut roots = rustls::RootCertStore::empty();
        roots.add(server_cert).expect("add cert to root store");
        let config = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        Arc::new(config)
    }

    fn peer_id(byte: u8) -> PeerId {
        let mut b = [0u8; PEER_ID_SIZE];
        b[..4].copy_from_slice(b"sha-");
        b[4..].fill(byte);
        PeerId::from_bytes(b)
    }

    /// Shared secret for listener tests — the test validator and
    /// `fake_auth_for` both use it so handshakes authenticate.
    const TEST_AUTH_SECRET: &[u8] = b"ws-listener-test-secret";

    /// Build an Auth message whose payload is a real, valid **peer-bound**
    /// (algo-2, ADR 1021) HMAC token for [`TEST_AUTH_SECRET`], bound to the
    /// same `peer_id` the Auth frame carries. This exercises the full
    /// binding path end-to-end: the token only authenticates if the
    /// listener threads the frame's `peer_id` into the validator, so it
    /// also proves that wiring (a misrouted `peer_id` would fail auth).
    fn fake_auth_for(peer_id_byte: u8) -> Vec<u8> {
        let expiry = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_secs() as i64
            + 3600;
        let peer = peer_id(peer_id_byte);
        let token = crate::auth::generate_test_token_bound(TEST_AUTH_SECRET, &peer, expiry);
        Auth::new(peer, Bytes::from(token))
            .expect("auth")
            .marshal()
    }

    /// Connect a TLS+WS client, perform the auth handshake, return the
    /// authenticated WebSocketStream. Asserts the AuthResponse address
    /// matches `expected_advertised`.
    async fn connect_and_auth(
        bound: SocketAddr,
        client_config: Arc<rustls::ClientConfig>,
        peer_byte: u8,
        expected_advertised: &str,
    ) -> tokio_tungstenite::WebSocketStream<
        tokio_rustls::client::TlsStream<tokio::net::TcpStream>,
    > {
        let tcp = tokio::net::TcpStream::connect(bound)
            .await
            .expect("tcp connect");
        let connector = tokio_rustls::TlsConnector::from(client_config);
        let server_name =
            rustls::pki_types::ServerName::try_from("localhost").expect("server name");
        let tls = connector.connect(server_name, tcp).await.expect("tls");

        let url = format!("wss://localhost:{}/relay", bound.port());
        let (mut ws, _resp) = tokio_tungstenite::client_async(url.as_str(), tls)
            .await
            .expect("ws upgrade");

        ws.send(Message::Binary(Bytes::from(fake_auth_for(peer_byte))))
            .await
            .expect("send Auth");

        let response_frame = ws.next().await.expect("server frame").expect("ok");
        let response_bytes = match response_frame {
            Message::Binary(b) => b,
            other => panic!("expected binary AuthResponse, got {other:?}"),
        };
        let resp = AuthResponse::parse(&response_bytes).expect("AuthResponse parse");
        assert_eq!(resp.address, format!("rels://{expected_advertised}"));
        ws
    }

    /// End-to-end: bind ephemeral port, server runs the full
    /// TLS → WS → handshake → routing pipeline, two clients connect and
    /// auth, peer A sends a Transport addressed to peer B, peer B
    /// receives the rewritten frame whose peer_id field now holds peer
    /// A's id (source) and whose payload is unchanged.
    #[tokio::test(flavor = "current_thread")]
    async fn ws_listener_routes_transport_between_two_peers() {
        let (certs, key) = self_signed_localhost();
        let server_cert = certs[0].clone();
        let server_config = test_server_config(certs, key);
        let client_config = test_client_config(server_cert);

        let bind_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let advertised = "test-relay.example.invalid:33073".to_string();
        let advertised_for_assert = advertised.clone();
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        let (bound_tx, bound_rx) = oneshot::channel::<Option<SocketAddr>>();
        let routing = RoutingTable::new();
        let routing_for_server = routing.clone();

        let local = tokio::task::LocalSet::new();
        let server_handle = local.spawn_local(async move {
            let cfg = WsListenerConfig {
                bind_addr,
                tls_config: server_config,
                advertised_addr: advertised,
                routing_table: routing_for_server,
                cbpf_steering_num_sockets: None,
                prefer_quic_bias_ms: 0,
                validator: std::sync::Arc::new(crate::auth::TimedHmacValidator::from_secret(
                    TEST_AUTH_SECRET,
                )),
            };
            let cancel_fut = async move {
                let _ = cancel_rx.await;
            };
            serve(cfg, cancel_fut, bound_tx).await
        });

        local
            .run_until(async move {
                let bound = bound_rx
                    .await
                    .expect("server reported bind result")
                    .expect("server bound");

                // Two peers: A (0xAA) and B (0xBB).
                let peer_a_id = peer_id(0xAA);
                let peer_b_id = peer_id(0xBB);
                let mut ws_a =
                    connect_and_auth(bound, client_config.clone(), 0xAA, &advertised_for_assert)
                        .await;
                let mut ws_b =
                    connect_and_auth(bound, client_config, 0xBB, &advertised_for_assert).await;

                // Wait for the server to register both peers in the
                // routing table. The auth response is sent BEFORE the
                // server inserts (insert happens in handle_connection
                // after drive_handshake returns), so a small yield
                // bound is sufficient — but to keep the test
                // deterministic without sleeping, we poll the routing
                // table directly.
                for _ in 0..50 {
                    if routing.len() == 2 {
                        break;
                    }
                    tokio::task::yield_now().await;
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                assert_eq!(routing.len(), 2, "both peers registered");

                // Peer A sends a Transport addressed to peer B.
                let payload = Bytes::from_static(b"hello-from-A-to-B");
                let transport = Transport::new(peer_b_id, payload.clone()).expect("transport");
                let transport_bytes = transport.marshal();
                ws_a.send(Message::Binary(Bytes::from(transport_bytes)))
                    .await
                    .expect("A → B send");

                // Peer B receives the forwarded frame. The relay's
                // in-place rewrite means the peer_id field at offset
                // 2..38 now holds peer A's id (source), and the
                // payload at offset 38.. is unchanged.
                let received_frame = ws_b.next().await.expect("B receives").expect("ok");
                let received_bytes = match received_frame {
                    Message::Binary(b) => b,
                    other => panic!("expected binary, got {other:?}"),
                };
                let parsed =
                    Transport::parse(&received_bytes).expect("parse forwarded Transport");
                assert_eq!(parsed.peer_id, peer_a_id, "peer_id rewritten to source");
                assert_eq!(parsed.payload, payload, "payload preserved");

                drop(ws_a);
                drop(ws_b);

                let _ = cancel_tx.send(());
                let server_result = server_handle.await.expect("server task joins");
                server_result.expect("serve() returns Ok");
            })
            .await;
    }

    /// P4 — malformed/truncated inbound frames
    /// are dropped without killing the reader or evicting the peer. Mirror
    /// of the QUIC `quic_malformed_frames_dropped_reader_survives`; the WS
    /// reader is an independent parse path (`run_reader` in this module),
    /// so it carries its own regression guard.
    #[tokio::test(flavor = "current_thread")]
    async fn ws_malformed_frames_dropped_reader_survives() {
        let (certs, key) = self_signed_localhost();
        let server_cert = certs[0].clone();
        let server_config = test_server_config(certs, key);
        let client_config = test_client_config(server_cert);

        let bind_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let advertised = "test-relay.example.invalid:33073".to_string();
        let advertised_for_assert = advertised.clone();
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        let (bound_tx, bound_rx) = oneshot::channel::<Option<SocketAddr>>();
        let routing = RoutingTable::new();
        let routing_for_server = routing.clone();

        let local = tokio::task::LocalSet::new();
        let server_handle = local.spawn_local(async move {
            let cfg = WsListenerConfig {
                bind_addr,
                tls_config: server_config,
                advertised_addr: advertised,
                routing_table: routing_for_server,
                cbpf_steering_num_sockets: None,
                prefer_quic_bias_ms: 0,
                validator: std::sync::Arc::new(crate::auth::TimedHmacValidator::from_secret(
                    TEST_AUTH_SECRET,
                )),
            };
            let cancel_fut = async move {
                let _ = cancel_rx.await;
            };
            serve(cfg, cancel_fut, bound_tx).await
        });

        local
            .run_until(async move {
                let bound = bound_rx.await.expect("bind result").expect("bound");
                let peer_a_id = peer_id(0xAA);
                let peer_b_id = peer_id(0xBB);
                let mut ws_a =
                    connect_and_auth(bound, client_config.clone(), 0xAA, &advertised_for_assert)
                        .await;
                let mut ws_b =
                    connect_and_auth(bound, client_config, 0xBB, &advertised_for_assert).await;
                for _ in 0..50 {
                    if routing.len() == 2 {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                assert_eq!(routing.len(), 2, "both peers registered");

                let good = Transport::new(peer_b_id, Bytes::from_static(b"valid"))
                    .expect("transport")
                    .marshal();
                let truncated_transport = Bytes::copy_from_slice(&good[..HEADER_SIZE + 4]);
                let junk: Vec<Bytes> = vec![
                    Bytes::from_static(b"x"),                        // 1 byte (< HEADER_SIZE)
                    Bytes::from_static(&[0xFF, MsgType::Transport as u8]), // bad version
                    Bytes::from_static(&[PROTOCOL_VERSION, 0x7E]),   // unknown msg_type
                    truncated_transport,                             // valid header, too-short body
                ];
                for frame in junk {
                    ws_a.send(Message::Binary(frame)).await.expect("send junk");
                }

                let payload = Bytes::from_static(b"survives-the-junk-ws");
                let transport = Transport::new(peer_b_id, payload.clone()).expect("transport");
                ws_a.send(Message::Binary(Bytes::from(transport.marshal())))
                    .await
                    .expect("A → B valid send");

                let received_frame = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    ws_b.next(),
                )
                .await
                .expect("B must still receive after the malformed barrage")
                .expect("B receives")
                .expect("ok");
                let received_bytes = match received_frame {
                    Message::Binary(b) => b,
                    other => panic!("expected binary, got {other:?}"),
                };
                let parsed = Transport::parse(&received_bytes).expect("parse forwarded Transport");
                assert_eq!(parsed.peer_id, peer_a_id, "peer_id rewritten to source");
                assert_eq!(parsed.payload, payload, "payload preserved");
                assert_eq!(routing.len(), 2, "no peer evicted by malformed frames");

                drop(ws_a);
                drop(ws_b);
                let _ = cancel_tx.send(());
                let server_result = server_handle.await.expect("server task joins");
                server_result.expect("serve() returns Ok");
            })
            .await;
    }

    /// C7 — graceful-drain empirical validation. Hypothesis: when
    /// the listener is cancelled, an in-flight connected peer receives a
    /// clean WS **Close frame** (not an abrupt RST) within the drain grace,
    /// and `serve()` returns within the bound. The falsifiable observation
    /// is the `Message::Close`: on the pre-C7 code the accept loop returned
    /// immediately and the connection task was abandoned, so the peer saw a
    /// dropped stream, never a Close frame. This test connects a real WS
    /// peer, fires cancel, and asserts the Close frame arrives.
    #[tokio::test(flavor = "current_thread")]
    async fn ws_graceful_drain_sends_close_frame_on_shutdown() {
        let (certs, key) = self_signed_localhost();
        let server_cert = certs[0].clone();
        let server_config = test_server_config(certs, key);
        let client_config = test_client_config(server_cert);

        let bind_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let advertised = "test-relay.example.invalid:33073".to_string();
        let advertised_for_assert = advertised.clone();
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        let (bound_tx, bound_rx) = oneshot::channel::<Option<SocketAddr>>();
        let routing = RoutingTable::new();
        let routing_for_server = routing.clone();

        let local = tokio::task::LocalSet::new();
        let server_handle = local.spawn_local(async move {
            let cfg = WsListenerConfig {
                bind_addr,
                tls_config: server_config,
                advertised_addr: advertised,
                routing_table: routing_for_server,
                cbpf_steering_num_sockets: None,
                prefer_quic_bias_ms: 0,
                validator: std::sync::Arc::new(crate::auth::TimedHmacValidator::from_secret(
                    TEST_AUTH_SECRET,
                )),
            };
            let cancel_fut = async move {
                let _ = cancel_rx.await;
            };
            serve(cfg, cancel_fut, bound_tx).await
        });

        local
            .run_until(async move {
                let bound = bound_rx.await.expect("bind result").expect("bound");
                let mut ws =
                    connect_and_auth(bound, client_config, 0xAA, &advertised_for_assert).await;
                for _ in 0..50 {
                    if routing.len() == 1 {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                assert_eq!(routing.len(), 1, "peer registered");

                // Fire shutdown.
                let _ = cancel_tx.send(());

                // EMPIRICAL observation: the peer must receive a WS Close
                // frame within the drain grace. (Pre-C7: the stream would
                // just drop — no Close.)
                let mut got_close = false;
                let read = tokio::time::timeout(std::time::Duration::from_secs(4), async {
                    while let Some(msg) = ws.next().await {
                        match msg {
                            Ok(Message::Close(_)) => {
                                got_close = true;
                                break;
                            }
                            Ok(_) => continue, // ignore any stray frame
                            Err(_) => break,    // abrupt drop = no graceful close
                        }
                    }
                })
                .await;
                assert!(read.is_ok(), "drain must complete within the grace window (C7)");
                assert!(got_close, "peer must receive a graceful WS Close frame on shutdown (C7)");

                // serve() returns within the bound (cannot hang).
                let server_result =
                    tokio::time::timeout(std::time::Duration::from_secs(8), server_handle)
                        .await
                        .expect("serve() returns within the bounded drain (C7)")
                        .expect("server task joins");
                server_result.expect("serve() returns Ok");
            })
            .await;
    }

    /// Multi-peer-pair concurrency integration test, mirroring
    /// upstream's `relay/test/benchmark_test.go::TestRelayDataTransfer`
    /// shape. Skipped by default (`#[ignore]`) like upstream's
    /// `t.SkipNow()` — too long for routine `cargo test --workspace`.
    /// Run via:
    ///
    /// ```text
    /// cargo test --bin goat-relay --release -- --ignored \
    ///   ws_listener_multi_peer_pair_routing
    /// ```
    ///
    /// Sweeps peer-pair counts. For each `N`, spawns `2*N` clients
    /// (paired into A/B sender/receiver), each pair exchanges
    /// `FRAMES_PER_PAIR` `Transport` frames; assert all forwarded
    /// frames arrive at the receiver with `peer_id` rewritten to the
    /// sender's id and the payload preserved.
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "long-running multi-peer concurrency test; run with --ignored"]
    async fn ws_listener_multi_peer_pair_routing() {
        const PEER_PAIR_COUNTS: &[usize] = &[2, 10, 30];
        const FRAMES_PER_PAIR: usize = 50;

        let (certs, key) = self_signed_localhost();
        let server_cert = certs[0].clone();
        let server_config = test_server_config(certs, key);
        let client_config = test_client_config(server_cert);

        let bind_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let advertised = "test-relay.example.invalid:33073".to_string();
        let advertised_for_assert = advertised.clone();
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        let (bound_tx, bound_rx) = oneshot::channel::<Option<SocketAddr>>();
        let routing = RoutingTable::new();
        let routing_for_server = routing.clone();

        let local = tokio::task::LocalSet::new();
        let server_handle = local.spawn_local(async move {
            let cfg = WsListenerConfig {
                bind_addr,
                tls_config: server_config,
                advertised_addr: advertised,
                routing_table: routing_for_server,
                cbpf_steering_num_sockets: None,
                prefer_quic_bias_ms: 0,
                validator: std::sync::Arc::new(crate::auth::TimedHmacValidator::from_secret(
                    TEST_AUTH_SECRET,
                )),
            };
            let cancel_fut = async move {
                let _ = cancel_rx.await;
            };
            serve(cfg, cancel_fut, bound_tx).await
        });

        local
            .run_until(async move {
                let bound = bound_rx
                    .await
                    .expect("server reported bind result")
                    .expect("server bound");

                for &pair_count in PEER_PAIR_COUNTS {
                    eprintln!("[multi-peer-ws] sweep pair_count={pair_count}");
                    run_one_sweep_step(
                        pair_count,
                        FRAMES_PER_PAIR,
                        bound,
                        client_config.clone(),
                        &advertised_for_assert,
                        &routing,
                    )
                    .await;
                }

                let _ = cancel_tx.send(());
                let server_result = server_handle.await.expect("server task joins");
                server_result.expect("serve() returns Ok");
            })
            .await;
    }

    /// One iteration of the peer-pair-count sweep. Connects `2*N`
    /// clients, performs the auth handshake on each, sends
    /// `frames_per_pair` `Transport` frames from A→B in each pair,
    /// asserts each receiver gets all `frames_per_pair` frames with
    /// the correct rewrite + payload preservation.
    // `clippy::result_large_err`: tungstenite::Error is 136 B; we hit
    // the lint via `b_streams[i].next().await.unwrap_or_else(...)`
    // panicking on None. Boxing in test code adds noise without
    // benefit; the API-side fix (Box-ing tungstenite::Error) belongs
    // upstream. Test code only.
    #[allow(clippy::result_large_err)]
    async fn run_one_sweep_step(
        pair_count: usize,
        frames_per_pair: usize,
        bound: SocketAddr,
        client_config: Arc<rustls::ClientConfig>,
        advertised: &str,
        routing: &RoutingTable,
    ) {
        // Build the 2*pair_count peer ids — A pairs are 0x00..pair_count,
        // B pairs are 0x80..0x80+pair_count (avoids collision and keeps
        // the byte-count in the suffix-fill clearly distinct).
        let mut a_ids = Vec::with_capacity(pair_count);
        let mut b_ids = Vec::with_capacity(pair_count);
        let mut a_streams = Vec::with_capacity(pair_count);
        let mut b_streams = Vec::with_capacity(pair_count);
        for i in 0..pair_count {
            let a_byte = i as u8;
            let b_byte = 0x80u8 | i as u8;
            a_ids.push(peer_id(a_byte));
            b_ids.push(peer_id(b_byte));
            a_streams.push(connect_and_auth(bound, client_config.clone(), a_byte, advertised).await);
            b_streams.push(connect_and_auth(bound, client_config.clone(), b_byte, advertised).await);
        }

        // Wait for the routing table to register all 2*pair_count
        // peers before sending traffic.
        let target = pair_count * 2;
        for _ in 0..200 {
            if routing.len() >= target {
                break;
            }
            tokio::task::yield_now().await;
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert_eq!(
            routing.len(),
            target,
            "expected {target} peers registered (pair_count={pair_count})"
        );

        // Each A sends frames_per_pair Transport frames to its paired B.
        for i in 0..pair_count {
            for f in 0..frames_per_pair {
                let payload = Bytes::from(format!("a={i:02x}-frame={f:04x}-pair-{pair_count}"));
                let transport = Transport::new(b_ids[i], payload).expect("Transport");
                a_streams[i]
                    .send(Message::Binary(Bytes::from(transport.marshal())))
                    .await
                    .expect("A → B send");
            }
        }

        // Each B receives frames_per_pair forwarded frames; verify
        // peer_id rewritten + payload preserved.
        for i in 0..pair_count {
            for f in 0..frames_per_pair {
                let frame = b_streams[i]
                    .next()
                    .await
                    .unwrap_or_else(|| panic!("B[{i}] expected frame {f}/{frames_per_pair}"))
                    .expect("frame is Ok");
                let bytes = match frame {
                    Message::Binary(b) => b,
                    other => panic!("B[{i}] expected binary frame, got {other:?}"),
                };
                let parsed = Transport::parse(&bytes).expect("parse forwarded Transport");
                assert_eq!(
                    parsed.peer_id, a_ids[i],
                    "B[{i}] frame {f}: peer_id rewritten to source"
                );
                let expected =
                    format!("a={i:02x}-frame={f:04x}-pair-{pair_count}");
                assert_eq!(
                    &parsed.payload[..],
                    expected.as_bytes(),
                    "B[{i}] frame {f}: payload preserved"
                );
            }
        }

        // Tear down all clients before next sweep iteration.
        for s in a_streams.into_iter().chain(b_streams) {
            drop(s);
        }
        // Wait for the routing table to drain (peers deregister on
        // close; eventual consistency).
        for _ in 0..200 {
            if routing.is_empty() {
                break;
            }
            tokio::task::yield_now().await;
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }
}
