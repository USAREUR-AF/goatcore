//! QUIC listener (RFC 9221 datagrams).
//!
//! Spec section: §2 of `docs/design/netbird-relay-wire-protocol.md`.
//! **Headline:** the netbird relay's QUIC path uses RFC 9221 unreliable
//! datagrams, NOT QUIC streams. The F-015 TCP-over-TCP head-of-line-
//! blocking pathology that affects the WS path does NOT apply here —
//! datagrams have no inter-message ordering guarantees.
//!
//! Per ADR 0709 D9: quinn is the QUIC library (tokio-native; matches
//! D7's tokio runtime; uses rustls + ring per D8). Datagram bytes
//! ride directly as relay-message-layer frames (header + peer_id +
//! payload, exactly the same wire format as the WS path's binary
//! frames per spec §1).
//!
//! # Per-connection lifecycle
//!
//! 1. quinn accepts the QUIC handshake (TLS 1.3 + transport params).
//! 2. The first datagram is the auth handshake (Auth or Hello — the
//!    listener accepts only Auth per spec §3.1; Hello is silently
//!    rejected).
//! 3. The listener emits `AuthResponse` as a datagram.
//! 4. The connection is registered in the per-shard `RoutingTable`.
//! 5. A writer task drains the MPSC and sends datagrams via
//!    `Connection::send_datagram`.
//! 6. The reader loop runs inline, reading datagrams via
//!    `Connection::read_datagram`, dispatching `Transport` frames
//!    through the same routing path as the WS listener (in-place
//!    peer_id rewrite + `routing_table.forward`).
//! 7. On reader exit (connection closed by peer or transport error),
//!    deregister and join the writer task.
//!
//! # ADR 0709 D7 invariants preserved
//!
//! - Accept loop runs inside the shard's `LocalSet`.
//! - Per-connection tasks spawn via `tokio::task::spawn_local`.
//! - Per-connection state binds to shard-local `Rc<RefCell<...>>` via
//!   the `RoutingTable` clone.

use std::cell::RefCell;
use std::io;
use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::Arc;

use bytes::Bytes;
use quinn::Endpoint;
use rustls::ServerConfig;
use socket2::{Domain, Protocol, Socket, Type};
use tokio::sync::{mpsc, oneshot};

use goat_relay_protocol::handshake::{HandshakeRequest, Validator, HANDSHAKE_TIMEOUT};
use goat_relay_protocol::keepalive::Sender as KeepaliveSender;
use goat_relay_protocol::messages::auth::AuthResponse;
use goat_relay_protocol::messages::payload::{
    rewrite_peer_id, TRANSPORT_PAYLOAD_OFFSET, TRANSPORT_PEER_ID_OFFSET,
};
use goat_relay_protocol::messages::{MsgType, PeerId, HEADER_SIZE, PEER_ID_SIZE, PROTOCOL_VERSION};

use crate::keepalive::{self, CloseReason};
use crate::routing::{ForwardError, RoutingTable};

/// Configuration for the QUIC listener. Mirrors [`super::ws::WsListenerConfig`]
/// to allow the binary to bind both transports on the same port with the
/// same cert + routing table.
pub struct QuicListenerConfig {
    /// Address to bind. UDP for QUIC; production binds the same port as
    /// the WS listener (TCP and UDP coexist on the same port number).
    pub bind_addr: SocketAddr,
    /// rustls `ServerConfig`. Must be **QUIC-compatible**: TLS 1.3 only,
    /// `max_early_data_size = u32::MAX`. The binary's `build_server_config`
    /// produces a config that satisfies both WS and QUIC requirements
    /// (same rustls config used for both transports).
    pub tls_config: Arc<ServerConfig>,
    /// Address advertised in `AuthResponse` after a successful handshake
    /// (`format!("rels://{advertised_addr}")`, spec §3.3).
    pub advertised_addr: String,
    /// Per-shard routing table — shared with the WS listener so peers
    /// connected via either transport are reachable from peers connected
    /// via the other.
    pub routing_table: RoutingTable,
    /// Relay-client auth validator — same instance shared with the WS
    /// listener (production: [`crate::auth::TimedHmacValidator`]). Shared
    /// read-only across connection-handler tasks via `Arc`. The binary
    /// fails closed without a configured secret, so there is no permissive
    /// default.
    pub validator: Arc<dyn Validator + Send + Sync>,
    /// Per-connection keepalive cadence. Production passes
    /// [`crate::keepalive::KeepaliveTimings::default`] (the upstream
    /// 25 s / 45 s windows); integration tests inject short windows so an
    /// idle-survival assertion runs in real time.
    pub keepalive_timings: crate::keepalive::KeepaliveTimings,
}

/// Bind a UDP socket with `SO_REUSEADDR` + `SO_REUSEPORT` set, then
/// hand it to quinn as a `std::net::UdpSocket`. Block 33C.1 lever L8 —
/// supports N shards each binding the same UDP port; the kernel
/// distributes datagrams across the shards' sockets.
pub(crate) fn bind_udp_reuseport(addr: SocketAddr) -> io::Result<std::net::UdpSocket> {
    use std::os::fd::AsRawFd;
    let domain = if addr.is_ipv6() {
        Domain::IPV6
    } else {
        Domain::IPV4
    };
    let socket = Socket::new(domain, Type::DGRAM, Some(Protocol::UDP))?;
    let fd = socket.as_raw_fd();
    eprintln!("quic bind_udp_reuseport: socket fd={fd} for addr={addr}");
    // Non-blocking is required by quinn's tokio runtime for the UDP
    // socket once it's converted to a tokio::net::UdpSocket internally.
    socket.set_nonblocking(true)?;
    eprintln!("quic bind_udp_reuseport: fd={fd} non-blocking set");
    socket.set_reuse_address(true)?;
    eprintln!("quic bind_udp_reuseport: fd={fd} SO_REUSEADDR=true set");
    // SO_REUSEPORT — N shards can each bind the same port. cfg-gated
    // because Windows doesn't have it; we don't ship Windows builds
    // (relay is Linux-only in production) but cfg-guard keeps macOS
    // dev builds happy too (macOS has SO_REUSEPORT).
    #[cfg(unix)]
    {
        socket.set_reuse_port(true)?;
        eprintln!("quic bind_udp_reuseport: fd={fd} SO_REUSEPORT=true set");
    }
    socket.bind(&addr.into())?;
    eprintln!("quic bind_udp_reuseport: fd={fd} bind({addr}) ok");
    Ok(socket.into())
}

/// Build the QUIC `TransportConfig` the relay server advertises.
///
/// Sets `max_idle_timeout` to [`goat_relay_protocol::transport::quic::SERVER_MAX_IDLE`]
/// (4 minutes, matching the Go client's request) so the negotiated idle
/// window — the MIN of both endpoints — is not dragged down to quinn's
/// short default, and `keep_alive_interval` to
/// [`goat_relay_protocol::transport::quic::SERVER_KEEP_ALIVE`] so the
/// server actively keeps idle connections warm rather than relying solely
/// on the client's unreliable datagram keepalive.
pub(crate) fn build_quic_transport_config() -> quinn::TransportConfig {
    use goat_relay_protocol::transport::quic::{SERVER_KEEP_ALIVE, SERVER_MAX_IDLE};
    let mut tc = quinn::TransportConfig::default();
    // SERVER_MAX_IDLE (4 min) is well within QUIC's varint range, so the
    // conversion cannot fail; expect() documents the invariant.
    let idle = quinn::IdleTimeout::try_from(SERVER_MAX_IDLE)
        .expect("SERVER_MAX_IDLE fits in a QUIC varint");
    tc.max_idle_timeout(Some(idle));
    tc.keep_alive_interval(Some(SERVER_KEEP_ALIVE));
    tc
}

/// Run the QUIC accept loop until `cancel` resolves.
///
/// # Errors
///
/// Returns `Err` if the initial bind fails or the rustls config cannot be
/// converted to a `QuicServerConfig` (TLS 1.3-only + `max_early_data_size`
/// constraints; see `build_server_config` in `main.rs`).
pub async fn serve<C>(
    config: QuicListenerConfig,
    cancel: C,
    bound_addr_tx: tokio::sync::oneshot::Sender<Option<SocketAddr>>,
) -> io::Result<()>
where
    C: std::future::Future<Output = ()>,
{
    let quic_crypto = match quinn::crypto::rustls::QuicServerConfig::try_from(
        config.tls_config.as_ref().clone(),
    ) {
        Ok(c) => c,
        Err(e) => {
            let _ = bound_addr_tx.send(None);
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("rustls ServerConfig is not QUIC-compatible: {e}"),
            ));
        }
    };
    let mut quic_server_config = quinn::ServerConfig::with_crypto(Arc::new(quic_crypto));
    // F-217 fix — without an explicit TransportConfig, quinn advertises its
    // own (short) default `max_idle_timeout`, and since the negotiated idle
    // timeout is the MIN of the two endpoints (RFC 9000 §10.1) the window
    // collapses far below the 4 minutes the Go client requests
    // (`shared/relay/client/dialer/quic/quic.go:47`). Low-traffic relayed
    // flows then idle out after tens of seconds → session flap. Advertise
    // the matching 4-minute idle window and have the server actively send
    // keepalive PINGs so liveness does not depend solely on the client's
    // unreliable datagram keepalive.
    quic_server_config.transport_config(Arc::new(build_quic_transport_config()));

    // Block 33C.1 lever L8 — bind UDP with SO_REUSEPORT so each shard
    // has its own bound socket on the same port. The kernel distributes
    // incoming UDP datagrams across the shards' sockets by source-tuple
    // hash. Single-shard deploys behave the same as before. quinn's
    // `Endpoint::server` would bind internally without REUSEPORT, so
    // we build the socket via socket2 + hand it to `Endpoint::new`.
    let udp_socket = match bind_udp_reuseport(config.bind_addr) {
        Ok(s) => s,
        Err(e) => {
            let _ = bound_addr_tx.send(None);
            return Err(e);
        }
    };

    let endpoint = match Endpoint::new(
        quinn::EndpointConfig::default(),
        Some(quic_server_config),
        udp_socket,
        Arc::new(quinn::TokioRuntime),
    ) {
        Ok(e) => e,
        Err(e) => {
            let _ = bound_addr_tx.send(None);
            return Err(e);
        }
    };
    let local_addr = endpoint.local_addr()?;
    let _ = bound_addr_tx.send(Some(local_addr));
    eprintln!("quic listener bound on {local_addr}");

    let advertised = Arc::new(config.advertised_addr);
    let routing_table = config.routing_table;
    let validator = config.validator;
    let keepalive_timings = config.keepalive_timings;

    // Admission control — per-shard concurrent-connection
    // cap, identical policy to the WS listener. Bounds pre-auth task / fd /
    // memory under a QUIC connection flood; the F-229 forensics showed the
    // relay hitting EMFILE under connection accumulation, so the QUIC accept
    // path caps concurrency too (it previously had none).
    let max_conns = crate::listener::ws::max_conns_per_shard();
    let conn_limit = Arc::new(tokio::sync::Semaphore::new(max_conns));
    eprintln!("quic listener: admission control = {max_conns} concurrent connections/shard");

    tokio::pin!(cancel);

    loop {
        tokio::select! {
            _ = &mut cancel => {
                eprintln!("quic listener: cancel signal received; closing endpoint");
                endpoint.close(0u32.into(), b"shutdown");
                endpoint.wait_idle().await;
                return Ok(());
            }
            incoming = endpoint.accept() => {
                let Some(incoming) = incoming else {
                    eprintln!("quic listener: endpoint closed");
                    return Ok(());
                };
                // F-222: refuse (don't accept) when at the per-shard cap —
                // bounds pre-auth resource use under a connection flood.
                // `Incoming::refuse` sends CONNECTION_REFUSED so the client
                // fails fast rather than timing out.
                let permit = match Arc::clone(&conn_limit).try_acquire_owned() {
                    Ok(p) => p,
                    Err(_) => {
                        incoming.refuse();
                        continue;
                    }
                };
                let advertised = Arc::clone(&advertised);
                let routing = routing_table.clone();
                let validator = Arc::clone(&validator);
                tokio::task::spawn_local(async move {
                    // Hold the admission permit for the whole connection
                    // lifetime; released on task exit so the slot frees for a
                    // new connection.
                    let _permit = permit;
                    let connection = match incoming.await {
                        Ok(c) => c,
                        Err(e) => {
                            eprintln!("quic: connection accept failed: {e}");
                            return;
                        }
                    };
                    let peer = connection.remote_address();
                    handle_connection(
                        connection,
                        peer,
                        advertised,
                        routing,
                        validator,
                        keepalive_timings,
                    )
                    .await;
                });
            }
        }
    }
}

/// Per-connection lifecycle: handshake → register → forwarding loop.
async fn handle_connection(
    connection: quinn::Connection,
    peer: SocketAddr,
    advertised_addr: Arc<String>,
    routing_table: RoutingTable,
    validator: Arc<dyn Validator + Send + Sync>,
    keepalive_timings: crate::keepalive::KeepaliveTimings,
) {
    let auth_peer_id = match tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        drive_handshake(&connection, validator.as_ref(), advertised_addr.as_str()),
    )
    .await
    {
        Ok(Ok(id)) => id,
        Ok(Err(e)) => {
            eprintln!("quic: handshake failed for {peer}: {e}; closing connection");
            connection.close(0u32.into(), b"handshake failed");
            return;
        }
        Err(_) => {
            eprintln!(
                "quic: handshake timed out for {peer} (>{}s); closing connection",
                HANDSHAKE_TIMEOUT.as_secs()
            );
            connection.close(0u32.into(), b"handshake timeout");
            return;
        }
    };

    eprintln!("quic: peer {peer} authenticated as {auth_peer_id:?}; entering forwarding loop");

    // Bounded per Block 33C.1 lever L4 / audit F-A4. Capacity 256 → ~2 MB
    // max per connection. Forward uses try_send (non-blocking) per spec
    // §3.3 silent-drop semantics.
    let (tx, rx) = mpsc::channel::<Bytes>(crate::routing::WRITER_QUEUE_CAPACITY);
    // Retained for identity-aware deregister on reader exit.
    let tx_for_dereg = tx.clone();
    if let Some(prior) = routing_table.insert(auth_peer_id, tx) {
        eprintln!(
            "quic: peer {auth_peer_id:?} reconnected via QUIC; closing prior connection's writer"
        );
        drop(prior);
    }
    // Tag this peer's transport for the stats surface (track netbird-wedge-rca):
    // per-peer QUIC-vs-WS is otherwise un-observable from outside the relay.
    routing_table.set_transport(&auth_peer_id, crate::routing::Transport::Quic);

    let conn_for_writer = connection.clone();
    let writer_task =
        tokio::task::spawn_local(run_writer(conn_for_writer, rx, routing_table.clone()));

    // Step 7: keepalive driver — same shape as the WS path (see
    // listener/ws.rs::handle_connection). The keepalive task uses the
    // QUIC `Connection` directly (cloned cheaply) for `send_datagram`
    // when emitting HC frames; on `DeclareTimeout` it signals the
    // reader via the close-signal oneshot.
    let ka_sender = Rc::new(RefCell::new(KeepaliveSender::new(
        keepalive::attempt_threshold_from_env(),
    )));
    let (close_signal_tx, close_signal_rx) = oneshot::channel::<CloseReason>();
    let keepalive_task = tokio::task::spawn_local(keepalive::run(
        Rc::clone(&ka_sender),
        connection.clone(),
        close_signal_tx,
        keepalive_timings,
    ));

    let reader_outcome = run_reader(
        &connection,
        auth_peer_id,
        routing_table.clone(),
        ka_sender,
        close_signal_rx,
    )
    .await;

    // Identity-aware deregister — no-op if the peer reconnected meanwhile
    // and a fresh handle owns the entry.
    routing_table.remove_self(&auth_peer_id, &tx_for_dereg);
    keepalive_task.abort();
    let _ = keepalive_task.await;
    let _ = writer_task.await;

    eprintln!(
        "quic: connection from {peer} (peer_id={auth_peer_id:?}) closed: {reader_outcome:?}"
    );
}

/// Drive the auth-handshake-FSM over the first datagram exchange.
/// Returns the authenticated peer-id on success.
async fn drive_handshake(
    connection: &quinn::Connection,
    validator: &(impl Validator + ?Sized),
    advertised_addr: &str,
) -> Result<PeerId, String> {
    let datagram = connection
        .read_datagram()
        .await
        .map_err(|e| format!("read first datagram: {e}"))?;
    let req = HandshakeRequest::parse(&datagram).map_err(|e| format!("handshake parse: {e}"))?;
    if !req.is_modern() {
        return Err("deprecated Hello handshake path not supported".into());
    }
    validator
        .validate(req.peer_id(), req.credential_bytes())
        .map_err(|e| format!("validator rejected: {e}"))?;

    let peer_id = *req.peer_id();
    // ADR 1021 migration telemetry — emit the auth-algo (1 = unbound,
    // 2 = peer-bound) so operators can confirm the fleet has moved to
    // peer-bound before flipping GOAT_RELAY_REQUIRE_PEER_BOUND.
    let auth_algo = req.credential_bytes().first().copied().unwrap_or(0);
    eprintln!("quic: peer {peer_id:?} authenticated (auth_algo={auth_algo})");
    let response = AuthResponse::new(format!("rels://{advertised_addr}"))
        .map_err(|e| format!("AuthResponse::new: {e}"))?;
    let response_bytes = Bytes::from(response.marshal());
    connection
        .send_datagram(response_bytes)
        .map_err(|e| format!("send_datagram (AuthResponse): {e}"))?;
    Ok(peer_id)
}

/// Reader loop: receive datagrams, parse `Transport` (forward via
/// routing), `Close` (return), `HealthCheck` (call
/// `Sender::on_response`). Also observes the close-signal `oneshot`
/// from the keepalive task and returns immediately on `DeclareTimeout`.
async fn run_reader(
    connection: &quinn::Connection,
    auth_peer_id: PeerId,
    routing_table: RoutingTable,
    ka_sender: Rc<RefCell<KeepaliveSender>>,
    close_signal_rx: oneshot::Receiver<CloseReason>,
) -> ReaderOutcome {
    tokio::pin!(close_signal_rx);
    loop {
        let read_result = tokio::select! {
            biased;
            signal = &mut close_signal_rx => {
                return match signal {
                    Ok(reason) => ReaderOutcome::KeepaliveSignal(reason),
                    Err(_) => ReaderOutcome::PeerClosed,
                };
            }
            r = connection.read_datagram() => r,
        };
        let bytes = match read_result {
            Ok(b) => b,
            Err(quinn::ConnectionError::ApplicationClosed { .. })
            | Err(quinn::ConnectionError::ConnectionClosed { .. })
            | Err(quinn::ConnectionError::LocallyClosed) => return ReaderOutcome::PeerClosed,
            Err(e) => return ReaderOutcome::ReadError(format!("{e}")),
        };

        if bytes.len() < HEADER_SIZE {
            eprintln!(
                "quic: peer_id={auth_peer_id:?} sent {} bytes (<header size {}); dropping",
                bytes.len(),
                HEADER_SIZE
            );
            continue;
        }
        if bytes[0] != PROTOCOL_VERSION {
            eprintln!(
                "quic: peer_id={auth_peer_id:?} sent unsupported version 0x{:02x}; dropping",
                bytes[0]
            );
            continue;
        }
        let msg_type = match MsgType::try_from(bytes[1]) {
            Ok(m) => m,
            Err(_) => {
                eprintln!(
                    "quic: peer_id={auth_peer_id:?} sent unknown msg_type 0x{:02x}; dropping",
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
                if let Err(e) = dispatch_transport(bytes.to_vec(), auth_peer_id, &routing_table) {
                    eprintln!(
                        "quic: dispatch failed for peer_id={auth_peer_id:?}: {e}; dropping frame"
                    );
                }
            }
            MsgType::Close => return ReaderOutcome::AppClose,
            MsgType::HealthCheck => {
                // Echo of our HC; liveness already credited above.
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
                            "quic: peer_id={auth_peer_id:?} parse_peer_list returned non-(Sub/Unsub) kind {other_kind:?}; dropping"
                        );
                    }
                    Err(e) => {
                        eprintln!(
                            "quic: peer_id={auth_peer_id:?} parse_peer_list failed: {e}; dropping"
                        );
                    }
                }
            }
            other => {
                eprintln!(
                    "quic: peer_id={auth_peer_id:?} sent {other:?} (mid-conn); ignoring"
                );
            }
        }
    }
}

/// Why the reader loop returned, for diagnostic logging.
#[derive(Debug)]
#[allow(dead_code)]
enum ReaderOutcome {
    /// Peer or transport closed the connection.
    PeerClosed,
    /// Peer sent an application-level `Close` (msg_type 0x04).
    AppClose,
    /// `read_datagram` returned a fatal error.
    ReadError(String),
    /// Keepalive task signalled the connection to close.
    KeepaliveSignal(CloseReason),
}

/// Dispatch a Transport datagram: extract dest peer-id, rewrite the
/// peer_id field in place to the source (the authenticated sender's id),
/// forward to the destination's writer.
fn dispatch_transport(
    mut buf: Vec<u8>,
    source_peer_id: PeerId,
    routing_table: &RoutingTable,
) -> Result<(), String> {
    if buf.len() < TRANSPORT_PAYLOAD_OFFSET {
        return Err(format!(
            "Transport datagram too short ({} bytes; need at least {})",
            buf.len(),
            TRANSPORT_PAYLOAD_OFFSET
        ));
    }
    let mut dest_id_bytes = [0u8; PEER_ID_SIZE];
    dest_id_bytes
        .copy_from_slice(&buf[TRANSPORT_PEER_ID_OFFSET..TRANSPORT_PAYLOAD_OFFSET]);
    let dest_id = PeerId::from_bytes(dest_id_bytes);

    rewrite_peer_id(&mut buf, &source_peer_id)
        .map_err(|e| format!("rewrite_peer_id: {e}"))?;

    let frame = Bytes::from(buf);
    // All drop reasons are silent per spec §3.3 and now COUNTED in
    // `forward()`'s DropCounters. No per-frame logging on the hot
    // path — that is itself a log-flood DoS; the observable is
    // `RoutingTable::drop_stats`. `forward_data` also attributes the bytes
    // to the source + destination per-peer counters (ADR 1055 §2b).
    match routing_table.forward_data(&source_peer_id, &dest_id, frame) {
        Ok(()) | Err(ForwardError::PeerNotFound | ForwardError::WriterClosed | ForwardError::WriterFull) => Ok(()),
    }
}

/// Writer task: drains the per-connection MPSC and sends each frame as
/// a QUIC datagram via `Connection::send_datagram`. Exits when the MPSC
/// closes (sender dropped) or `send_datagram` returns a fatal error.
///
/// **Block 33C.1 lever L6 — batched I/O.** Drains up to
/// [`crate::routing::WRITER_QUEUE_CAPACITY`] frames per wake via
/// [`mpsc::Receiver::recv_many`], then enqueues each into quinn's
/// per-connection datagram queue. `send_datagram` is non-blocking and
/// returns immediately — quinn's IO loop coalesces the queued
/// datagrams into UDP `sendmmsg` batches at the transport layer
/// (GSO/UDP-segmentation when supported by the kernel + NIC; per-
/// packet `sendmsg` otherwise). The batched drain at the application
/// level reduces task-wakeup overhead and lets quinn see more
/// candidates per IO-loop pass. Co-designed with L4 (bounded MPSC)
/// — the queue cap bounds worst-case batch latency.
async fn run_writer(
    connection: quinn::Connection,
    mut rx: mpsc::Receiver<Bytes>,
    routing: RoutingTable,
) {
    let mut batch: Vec<Bytes> = Vec::with_capacity(crate::routing::WRITER_QUEUE_CAPACITY);
    // Rate-limit the too-large log line — an oversized flow must not become
    // a per-frame log flood. First occurrence + every 1000th.
    let mut too_large: u64 = 0;
    loop {
        batch.clear();
        let n = rx
            .recv_many(&mut batch, crate::routing::WRITER_QUEUE_CAPACITY)
            .await;
        if n == 0 {
            // Channel closed (sender dropped) — normal teardown.
            break;
        }
        for frame in batch.drain(..) {
            match connection.send_datagram(frame) {
                Ok(()) => {}
                Err(quinn::SendDatagramError::TooLarge) => {
                    // Per-DATAGRAM condition, not a connection fault: the
                    // frame exceeds the connection's current datagram limit
                    // (path-MTU-bound; ~1,200 B before PMTUD completes,
                    // ~1,450 B after). Upstream Go logs-and-continues here
                    // (`relay/server/peer.go`). Exiting instead permanently
                    // killed this peer's receive path while the connection
                    // stayed healthy-looking — the keepalive task sends its
                    // 2-byte probes via the Connection directly, so neither
                    // end ever noticed (track netbird-wedge-rca, 87 live
                    // occurrences on relay-rla). Drop the frame, count it,
                    // keep the writer alive.
                    routing.count_datagram_too_large();
                    too_large += 1;
                    // `% == 0` matches MSRV 1.75; `is_multiple_of` is 1.87+.
                    if too_large == 1 || too_large % 1000 == 0 {
                        eprintln!(
                            "quic: send_datagram dropped: datagram too large ({too_large} so far on this conn); writer continues"
                        );
                    }
                }
                Err(e) => {
                    // Connection-level failure (lost, or datagrams
                    // unsupported/disabled by the peer): this writer can
                    // never deliver again. Close the CONNECTION — not just
                    // this task — so the reader exits, teardown runs the
                    // identity-aware deregister + offline fan-out, and the
                    // client reconnects promptly. Exiting silently left a
                    // routable-to-no-one zombie the (writer-bypassing)
                    // keepalive sustained indefinitely.
                    eprintln!("quic: send_datagram failed: {e}; closing connection");
                    connection.close(1u32.into(), b"relay writer failed");
                    return;
                }
            }
        }
    }
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

    #[test]
    fn quic_transport_config_builds_with_idle_and_keepalive() {
        // F-217 regression guard: the server must advertise an explicit
        // TransportConfig. This builds it (exercising the IdleTimeout
        // conversion `expect`, which would panic if SERVER_MAX_IDLE ever
        // exceeded the QUIC varint range) and confirms it can be wrapped
        // into a ServerConfig the same way `serve()` does.
        let tc = build_quic_transport_config();
        let _arc = Arc::new(tc); // mirrors serve()'s `transport_config(Arc::new(..))`
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

    /// netbird-wedge-rca regression: an oversized frame must NOT kill the
    /// writer task. Pre-fix, one `SendDatagramError::TooLarge` made the
    /// writer exit permanently — the peer was then auto-deregistered on the
    /// next forward while its connection (and the writer-bypassing
    /// keepalive) stayed healthy-looking: a silent zombie only a client
    /// `down/up` cleared. This drives `run_writer` over a REAL quinn
    /// connection: a 2,000-byte frame (always above the ≤1,452-byte
    /// datagram limit) must be dropped + counted, and a small frame queued
    /// BEHIND it must still be delivered.
    #[tokio::test(flavor = "current_thread")]
    async fn quic_writer_survives_oversized_datagram() {
        let (certs, key) = self_signed_localhost();
        let server_cert = certs[0].clone();
        let server_config = quic_compatible_server_config(certs, key);
        let client_config = quic_client_config(server_cert);

        let local = tokio::task::LocalSet::new();
        local
            .run_until(async move {
                let quic_crypto = quinn::crypto::rustls::QuicServerConfig::try_from(
                    server_config.as_ref().clone(),
                )
                .expect("quic server config");
                let mut qsc = quinn::ServerConfig::with_crypto(Arc::new(quic_crypto));
                qsc.transport_config(Arc::new(build_quic_transport_config()));
                let server = quinn::Endpoint::server(qsc, "127.0.0.1:0".parse().unwrap())
                    .expect("server endpoint");
                let bound = server.local_addr().expect("bound addr");

                let mut client = quinn::Endpoint::client("127.0.0.1:0".parse().unwrap())
                    .expect("client endpoint");
                client.set_default_client_config(client_config);
                let connecting = client.connect(bound, "localhost").expect("connect");
                let (server_conn, client_conn) = tokio::join!(
                    async {
                        server
                            .accept()
                            .await
                            .expect("incoming")
                            .await
                            .expect("server conn")
                    },
                    async { connecting.await.expect("client conn") },
                );

                let routing = RoutingTable::new();
                let (tx, rx) =
                    mpsc::channel::<Bytes>(crate::routing::WRITER_QUEUE_CAPACITY);
                let writer =
                    tokio::task::spawn_local(run_writer(server_conn, rx, routing.clone()));

                // Oversized (dropped, counted, writer survives) …
                tx.send(Bytes::from(vec![0u8; 2000])).await.expect("queue big");
                // … then a small frame that must still arrive.
                tx.send(Bytes::from_static(b"still-alive")).await.expect("queue small");

                let received = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    client_conn.read_datagram(),
                )
                .await
                .expect("writer must survive the oversized frame (netbird-wedge-rca)")
                .expect("datagram");
                assert_eq!(&received[..], b"still-alive");
                assert!(
                    routing.drop_stats().datagram_too_large >= 1,
                    "oversized drop is counted on the stats surface"
                );

                drop(tx); // all senders gone → writer exits via normal teardown
                let _ = writer.await;
                client_conn.close(0u32.into(), b"done");
            })
            .await;
    }

    /// QUIC-compatible rustls ServerConfig: TLS 1.3 only +
    /// max_early_data_size = u32::MAX.
    fn quic_compatible_server_config(
        certs: Vec<CertificateDer<'static>>,
        key: PrivateKeyDer<'static>,
    ) -> Arc<rustls::ServerConfig> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut config = rustls::ServerConfig::builder_with_protocol_versions(&[
            &rustls::version::TLS13,
        ])
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .expect("server config");
        config.max_early_data_size = u32::MAX;
        // F-022: harmless on QUIC (quinn doesn't extract via this
        // mechanism), but parallel to test_server_config in
        // listener/ws.rs::tests for symmetry.
        config.enable_secret_extraction = true;
        Arc::new(config)
    }

    fn quic_client_config(
        server_cert: CertificateDer<'static>,
    ) -> quinn::ClientConfig {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut roots = rustls::RootCertStore::empty();
        roots.add(server_cert).expect("add cert to root store");
        let client_crypto = rustls::ClientConfig::builder_with_protocol_versions(&[
            &rustls::version::TLS13,
        ])
        .with_root_certificates(roots)
        .with_no_client_auth();
        let quic_client = quinn::crypto::rustls::QuicClientConfig::try_from(client_crypto)
            .expect("QuicClientConfig");
        quinn::ClientConfig::new(Arc::new(quic_client))
    }

    fn peer_id(byte: u8) -> PeerId {
        let mut b = [0u8; PEER_ID_SIZE];
        b[..4].copy_from_slice(b"sha-");
        b[4..].fill(byte);
        PeerId::from_bytes(b)
    }

    /// Shared secret for listener tests — the test validator and
    /// `fake_auth_for` both use it so handshakes authenticate.
    const TEST_AUTH_SECRET: &[u8] = b"quic-listener-test-secret";

    /// Build an Auth message carrying a real, valid **peer-bound** (algo-2,
    /// ADR 1021) HMAC token for [`TEST_AUTH_SECRET`], bound to the same
    /// `peer_id` the Auth frame carries — exercising the full binding path
    /// end-to-end (the token only authenticates if the listener threads the
    /// frame's `peer_id` into the validator).
    fn fake_auth_for(peer_id_byte: u8) -> Bytes {
        let expiry = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_secs() as i64
            + 3600;
        let peer = peer_id(peer_id_byte);
        let token = crate::auth::generate_test_token_bound(TEST_AUTH_SECRET, &peer, expiry);
        Bytes::from(
            Auth::new(peer, Bytes::from(token))
                .expect("auth")
                .marshal(),
        )
    }

    /// Build an Auth frame whose peer-bound token carries an explicit
    /// absolute `expiry` (unix secs) — lets P2 mint a near-expiry token to
    /// simulate the relay clock running ahead of the minter.
    fn auth_bytes_with_expiry(peer_id_byte: u8, expiry: i64) -> Bytes {
        let peer = peer_id(peer_id_byte);
        let token = crate::auth::generate_test_token_bound(TEST_AUTH_SECRET, &peer, expiry);
        Bytes::from(
            Auth::new(peer, Bytes::from(token))
                .expect("auth")
                .marshal(),
        )
    }

    /// Connect a QUIC client, perform the auth handshake (datagram-based),
    /// return the authenticated quinn::Connection.
    async fn connect_and_auth(
        bound: SocketAddr,
        client_config: quinn::ClientConfig,
        peer_byte: u8,
        expected_advertised: &str,
    ) -> quinn::Connection {
        let mut client_endpoint =
            quinn::Endpoint::client("127.0.0.1:0".parse().unwrap()).expect("client endpoint");
        client_endpoint.set_default_client_config(client_config);

        let connection = client_endpoint
            .connect(bound, "localhost")
            .expect("connect")
            .await
            .expect("handshake");

        connection
            .send_datagram(fake_auth_for(peer_byte))
            .expect("send Auth");

        let response = connection
            .read_datagram()
            .await
            .expect("AuthResponse datagram");
        let resp = AuthResponse::parse(&response).expect("parse AuthResponse");
        assert_eq!(resp.address, format!("rels://{expected_advertised}"));
        connection
    }

    /// End-to-end: bind ephemeral QUIC port, two clients connect + auth +
    /// register, peer A sends a Transport datagram addressed to peer B,
    /// peer B receives the rewritten datagram (peer_id field = A's id;
    /// payload preserved).
    #[tokio::test(flavor = "current_thread")]
    async fn quic_listener_routes_transport_between_two_peers() {
        let (certs, key) = self_signed_localhost();
        let server_cert = certs[0].clone();
        let server_config = quic_compatible_server_config(certs, key);
        let client_config = quic_client_config(server_cert);

        let bind_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let advertised = "test-relay.example.invalid:33073".to_string();
        let advertised_for_assert = advertised.clone();
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        let (bound_tx, bound_rx) = oneshot::channel::<Option<SocketAddr>>();
        let routing = RoutingTable::new();
        let routing_for_server = routing.clone();

        let local = tokio::task::LocalSet::new();
        let server_handle = local.spawn_local(async move {
            let cfg = QuicListenerConfig {
                bind_addr,
                tls_config: server_config,
                advertised_addr: advertised,
                routing_table: routing_for_server,
                validator: std::sync::Arc::new(crate::auth::TimedHmacValidator::from_secret(
                    TEST_AUTH_SECRET,
                )),
                keepalive_timings: crate::keepalive::KeepaliveTimings::default(),
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

                let peer_a_id = peer_id(0xAA);
                let peer_b_id = peer_id(0xBB);
                let conn_a =
                    connect_and_auth(bound, client_config.clone(), 0xAA, &advertised_for_assert)
                        .await;
                let conn_b =
                    connect_and_auth(bound, client_config, 0xBB, &advertised_for_assert).await;

                // Wait for the server to register both peers in the
                // routing table. Same poll-pattern as the WS test.
                for _ in 0..50 {
                    if routing.len() == 2 {
                        break;
                    }
                    tokio::task::yield_now().await;
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                assert_eq!(routing.len(), 2, "both peers registered");

                let payload = Bytes::from_static(b"quic-hello-from-A-to-B");
                let transport = Transport::new(peer_b_id, payload.clone()).expect("transport");
                let transport_bytes = Bytes::from(transport.marshal());
                conn_a
                    .send_datagram(transport_bytes)
                    .expect("A → B send_datagram");

                let received = conn_b
                    .read_datagram()
                    .await
                    .expect("B receives Transport");
                let parsed = Transport::parse(&received).expect("parse forwarded Transport");
                assert_eq!(parsed.peer_id, peer_a_id, "peer_id rewritten to source");
                assert_eq!(parsed.payload, payload, "payload preserved");

                conn_a.close(0u32.into(), b"test done");
                conn_b.close(0u32.into(), b"test done");

                let _ = cancel_tx.send(());
                let server_result = server_handle.await.expect("server task joins");
                server_result.expect("serve() returns Ok");
            })
            .await;
    }

    /// P1 — integration proof that a peer reconnect does NOT let
    /// the OLD connection's teardown evict the freshly-reconnected session.
    /// The test forces the bug-triggering interleaving: new connection
    /// registers first, THEN the old connection closes and runs its
    /// deregister. On the pre-fix code that deregister was an unconditional
    /// `remove(peer_id)` that deleted the new entry, so a frame addressed
    /// to the peer afterwards never arrived ("netbird down/up doesn't
    /// recover"). With identity-aware `remove_self` the fresh session
    /// survives and still receives.
    #[tokio::test(flavor = "current_thread")]
    async fn quic_reconnect_does_not_evict_fresh_session() {
        let (certs, key) = self_signed_localhost();
        let server_cert = certs[0].clone();
        let server_config = quic_compatible_server_config(certs, key);
        let client_config = quic_client_config(server_cert);

        let bind_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let advertised = "test-relay.example.invalid:33073".to_string();
        let advertised_for_assert = advertised.clone();
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        let (bound_tx, bound_rx) = oneshot::channel::<Option<SocketAddr>>();
        let routing = RoutingTable::new();
        let routing_for_server = routing.clone();

        let local = tokio::task::LocalSet::new();
        let server_handle = local.spawn_local(async move {
            let cfg = QuicListenerConfig {
                bind_addr,
                tls_config: server_config,
                advertised_addr: advertised,
                routing_table: routing_for_server,
                validator: std::sync::Arc::new(crate::auth::TimedHmacValidator::from_secret(
                    TEST_AUTH_SECRET,
                )),
                keepalive_timings: crate::keepalive::KeepaliveTimings::default(),
            };
            let cancel_fut = async move {
                let _ = cancel_rx.await;
            };
            serve(cfg, cancel_fut, bound_tx).await
        });

        local
            .run_until(async move {
                let bound = bound_rx.await.expect("bind result").expect("bound");
                let peer_p = peer_id(0xCC);

                // Peer P connects (old connection); wait for registration.
                let conn_old =
                    connect_and_auth(bound, client_config.clone(), 0xCC, &advertised_for_assert)
                        .await;
                for _ in 0..50 {
                    if routing.len() == 1 {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                assert_eq!(routing.len(), 1, "peer P registered (old connection)");

                // Peer P reconnects (new connection, SAME peer-id). `insert`
                // replaces the entry; let it run before the old one tears down.
                let conn_new =
                    connect_and_auth(bound, client_config.clone(), 0xCC, &advertised_for_assert)
                        .await;
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;

                // Old connection closes → its server reader exits and runs
                // the deregister, which must be a NO-OP for the superseded
                // connection (the bug-triggering order).
                conn_old.close(0u32.into(), b"old gone");
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;

                // A third peer sends to P. The fresh session must still
                // receive it (would be PeerNotFound on the pre-fix code).
                let conn_c =
                    connect_and_auth(bound, client_config, 0xDD, &advertised_for_assert).await;
                let payload = Bytes::from_static(b"to-reconnected-P");
                let transport = Transport::new(peer_p, payload.clone()).expect("transport");
                conn_c
                    .send_datagram(Bytes::from(transport.marshal()))
                    .expect("C -> P send_datagram");

                let received = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    conn_new.read_datagram(),
                )
                .await
                .expect("fresh session must still receive after reconnect")
                .expect("datagram");
                let parsed = Transport::parse(&received).expect("parse forwarded Transport");
                assert_eq!(parsed.payload, payload, "payload delivered to reconnected session");

                conn_new.close(0u32.into(), b"done");
                conn_c.close(0u32.into(), b"done");
                let _ = cancel_tx.send(());
                let server_result = server_handle.await.expect("server task joins");
                server_result.expect("serve() returns Ok");
            })
            .await;
    }

    /// P4 — malformed/truncated inbound datagrams are
    /// dropped WITHOUT killing the reader loop or evicting the peer. The
    /// flap-relevant property: a peer that emits a bad frame (short read,
    /// stale version, unknown type, truncated Transport) must not lose its
    /// routing slot — otherwise one corrupt datagram becomes a
    /// `PeersWentOffline`. Sends a barrage of junk from A, then a VALID
    /// Transport A→B, and asserts B still receives it and A is still
    /// registered (reader survived every malformed frame).
    #[tokio::test(flavor = "current_thread")]
    async fn quic_malformed_frames_dropped_reader_survives() {
        let (certs, key) = self_signed_localhost();
        let server_cert = certs[0].clone();
        let server_config = quic_compatible_server_config(certs, key);
        let client_config = quic_client_config(server_cert);

        let bind_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let advertised = "test-relay.example.invalid:33073".to_string();
        let advertised_for_assert = advertised.clone();
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        let (bound_tx, bound_rx) = oneshot::channel::<Option<SocketAddr>>();
        let routing = RoutingTable::new();
        let routing_for_server = routing.clone();

        let local = tokio::task::LocalSet::new();
        let server_handle = local.spawn_local(async move {
            let cfg = QuicListenerConfig {
                bind_addr,
                tls_config: server_config,
                advertised_addr: advertised,
                routing_table: routing_for_server,
                validator: std::sync::Arc::new(crate::auth::TimedHmacValidator::from_secret(
                    TEST_AUTH_SECRET,
                )),
                keepalive_timings: crate::keepalive::KeepaliveTimings::default(),
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
                let conn_a =
                    connect_and_auth(bound, client_config.clone(), 0xAA, &advertised_for_assert)
                        .await;
                let conn_b =
                    connect_and_auth(bound, client_config, 0xBB, &advertised_for_assert).await;
                for _ in 0..50 {
                    if routing.len() == 2 {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                assert_eq!(routing.len(), 2, "both peers registered");

                // A truncated-but-header-valid Transport: correct version +
                // msg_type but a body shorter than TRANSPORT_PAYLOAD_OFFSET,
                // so `dispatch_transport` rejects it (the F-220 too-short
                // path) rather than the pre-parse header guards.
                let good = Transport::new(peer_b_id, Bytes::from_static(b"valid"))
                    .expect("transport")
                    .marshal();
                let truncated_transport = Bytes::copy_from_slice(&good[..HEADER_SIZE + 4]);

                // Barrage of malformed datagrams — each must be dropped and
                // the reader must keep going.
                let junk: Vec<Bytes> = vec![
                    Bytes::from_static(b""),                         // empty (< HEADER_SIZE)
                    Bytes::from_static(b"x"),                        // 1 byte (< HEADER_SIZE)
                    Bytes::from_static(&[0xFF, MsgType::Transport as u8]), // bad version
                    Bytes::from_static(&[PROTOCOL_VERSION, 0x7E]),   // unknown msg_type
                    truncated_transport,                             // valid header, too-short body
                ];
                for frame in junk {
                    conn_a.send_datagram(frame).expect("send junk datagram");
                }

                // Now a genuinely valid Transport A→B. If the reader had
                // died on any junk frame, this never arrives.
                let payload = Bytes::from_static(b"survives-the-junk");
                let transport = Transport::new(peer_b_id, payload.clone()).expect("transport");
                conn_a
                    .send_datagram(Bytes::from(transport.marshal()))
                    .expect("A → B valid send");

                let received = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    conn_b.read_datagram(),
                )
                .await
                .expect("B must still receive after the malformed barrage")
                .expect("datagram");
                let parsed = Transport::parse(&received).expect("parse forwarded Transport");
                assert_eq!(parsed.peer_id, peer_a_id, "peer_id rewritten to source");
                assert_eq!(parsed.payload, payload, "payload preserved");
                assert_eq!(routing.len(), 2, "no peer evicted by malformed frames");

                conn_a.close(0u32.into(), b"done");
                conn_b.close(0u32.into(), b"done");
                let _ = cancel_tx.send(());
                let server_result = server_handle.await.expect("server task joins");
                server_result.expect("serve() returns Ok");
            })
            .await;
    }

    /// P2 — the auth handshake honors the clock-skew grace
    /// end-to-end through the real QUIC listener: a token that expired
    /// 30 s ago (relay clock running ahead of the minter, within the 60 s
    /// `CLOCK_SKEW_GRACE_SECS`) still authenticates and registers, while a
    /// token expired well past the grace (120 s) is rejected and never
    /// registers. The auth-unit tests prove the validator in isolation;
    /// this proves the listener actually routes the verdict through
    /// `drive_handshake` (an intermittent near-expiry drop was the
    /// reconnect-becomes-persistent failure mode).
    #[tokio::test(flavor = "current_thread")]
    async fn quic_auth_honors_clock_skew_grace() {
        let (certs, key) = self_signed_localhost();
        let server_cert = certs[0].clone();
        let server_config = quic_compatible_server_config(certs, key);
        let client_config = quic_client_config(server_cert);

        let bind_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let advertised = "test-relay.example.invalid:33073".to_string();
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        let (bound_tx, bound_rx) = oneshot::channel::<Option<SocketAddr>>();
        let routing = RoutingTable::new();
        let routing_for_server = routing.clone();

        let local = tokio::task::LocalSet::new();
        let server_handle = local.spawn_local(async move {
            let cfg = QuicListenerConfig {
                bind_addr,
                tls_config: server_config,
                advertised_addr: advertised,
                routing_table: routing_for_server,
                validator: std::sync::Arc::new(crate::auth::TimedHmacValidator::from_secret(
                    TEST_AUTH_SECRET,
                )),
                keepalive_timings: crate::keepalive::KeepaliveTimings::default(),
            };
            let cancel_fut = async move {
                let _ = cancel_rx.await;
            };
            serve(cfg, cancel_fut, bound_tx).await
        });

        local
            .run_until(async move {
                let bound = bound_rx.await.expect("bind result").expect("bound");
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("clock after epoch")
                    .as_secs() as i64;

                // --- within grace: expired 30 s ago, must AUTHENTICATE ---
                let mut ep = quinn::Endpoint::client("127.0.0.1:0".parse().unwrap())
                    .expect("client endpoint");
                ep.set_default_client_config(client_config.clone());
                let conn = ep
                    .connect(bound, "localhost")
                    .expect("connect")
                    .await
                    .expect("handshake");
                conn.send_datagram(auth_bytes_with_expiry(0xAA, now - 30))
                    .expect("send near-expiry Auth");
                let response = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    conn.read_datagram(),
                )
                .await
                .expect("near-expiry token within grace must auth")
                .expect("AuthResponse datagram");
                AuthResponse::parse(&response).expect("parse AuthResponse");
                for _ in 0..50 {
                    if routing.len() == 1 {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                assert_eq!(routing.len(), 1, "near-expiry peer registered (within grace)");

                // --- past grace: expired 120 s ago, must be REJECTED ---
                let mut ep2 = quinn::Endpoint::client("127.0.0.1:0".parse().unwrap())
                    .expect("client endpoint 2");
                ep2.set_default_client_config(client_config);
                let conn2 = ep2
                    .connect(bound, "localhost")
                    .expect("connect 2")
                    .await
                    .expect("handshake 2");
                conn2
                    .send_datagram(auth_bytes_with_expiry(0xBB, now - 120))
                    .expect("send past-grace Auth");
                // The server rejects + closes; the client either reads no
                // AuthResponse (timeout) or sees the connection closed.
                let rejected = match tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    conn2.read_datagram(),
                )
                .await
                {
                    Err(_elapsed) => true,             // no AuthResponse arrived
                    Ok(Err(_conn_err)) => true,        // server closed the connection
                    Ok(Ok(_)) => false,                // got a datagram → wrongly accepted
                };
                assert!(rejected, "token expired past the skew grace must be rejected");
                // The rejected peer must never have registered.
                assert_eq!(routing.len(), 1, "past-grace peer did not register");

                conn.close(0u32.into(), b"done");
                let _ = cancel_tx.send(());
                let server_result = server_handle.await.expect("server task joins");
                server_result.expect("serve() returns Ok");
            })
            .await;
    }

    /// P0 — a connection that keeps sending DATA
    /// but never echoes a HealthCheck survives past multiple keepalive
    /// timeout windows, while a fully silent connection is evicted. This
    /// is the integration proof of the data-as-liveness fix: on the
    /// pre-fix code only inbound HealthCheck echoes credited liveness, so
    /// a low-traffic-but-active relayed flow (the DNS-probe path that read
    /// `Nameservers 0/1`) was declared timed-out and evicted even though
    /// it was carrying real frames.
    ///
    /// Uses injected short keepalive windows (`KeepaliveTimings`) so the
    /// assertion runs in ~1 s of real time — the keepalive loop drives a
    /// live QUIC connection, so tokio's paused clock can't be used. The
    /// silent peer arm also proves the timeout *actually fires* with the
    /// short windows (not a no-op), so the data peer's survival is
    /// meaningful. (R1, the QUIC transport idle window, is covered
    /// separately by `quic_transport_config_builds_with_idle_and_keepalive`
    /// plus the real `keep_alive_interval`.)
    #[tokio::test(flavor = "current_thread")]
    async fn quic_data_traffic_keeps_session_alive_silent_evicted() {
        use std::time::Duration;

        let (certs, key) = self_signed_localhost();
        let server_cert = certs[0].clone();
        let server_config = quic_compatible_server_config(certs, key);
        let client_config = quic_client_config(server_cert);

        let bind_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let advertised = "test-relay.example.invalid:33073".to_string();
        let advertised_for_assert = advertised.clone();
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        let (bound_tx, bound_rx) = oneshot::channel::<Option<SocketAddr>>();
        let routing = RoutingTable::new();
        let routing_for_server = routing.clone();

        // Short windows: data peer sends every 60 ms, well inside the
        // 200 ms timeout window; the silent peer (no inbound activity)
        // dies on its first timeout tick (Sender starts alive=false,
        // threshold 1).
        let timings = crate::keepalive::KeepaliveTimings {
            hc_interval: Duration::from_millis(70),
            timeout_window: Duration::from_millis(200),
        };

        let local = tokio::task::LocalSet::new();
        let server_handle = local.spawn_local(async move {
            let cfg = QuicListenerConfig {
                bind_addr,
                tls_config: server_config,
                advertised_addr: advertised,
                routing_table: routing_for_server,
                validator: std::sync::Arc::new(crate::auth::TimedHmacValidator::from_secret(
                    TEST_AUTH_SECRET,
                )),
                keepalive_timings: timings,
            };
            let cancel_fut = async move {
                let _ = cancel_rx.await;
            };
            serve(cfg, cancel_fut, bound_tx).await
        });

        local
            .run_until(async move {
                let bound = bound_rx.await.expect("bind result").expect("bound");
                let peer_a = peer_id(0xAA); // data-active
                let peer_s = peer_id(0xCC); // silent
                let conn_a =
                    connect_and_auth(bound, client_config.clone(), 0xAA, &advertised_for_assert)
                        .await;
                let _conn_s =
                    connect_and_auth(bound, client_config, 0xCC, &advertised_for_assert).await;
                for _ in 0..50 {
                    if routing.len() == 2 {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                assert_eq!(routing.len(), 2, "both peers registered");

                // Drive A active for ~720 ms (> 3 timeout windows) with
                // DATA only — never a HealthCheck echo. Destination is an
                // unregistered dummy id; the relay drops the forward, but
                // the reader credits liveness (`mark_alive`) on the inbound
                // frame BEFORE dispatch, which is the whole point.
                let dummy_dest = peer_id(0x11);
                for _ in 0..12 {
                    let t = Transport::new(dummy_dest, Bytes::from_static(b"keepalive-data"))
                        .expect("transport");
                    conn_a
                        .send_datagram(Bytes::from(t.marshal()))
                        .expect("A active data send");
                    tokio::time::sleep(Duration::from_millis(60)).await;
                }

                // Membership via `forward` (no delivery dependency, no
                // registration-gap race): A present → Ok; S evicted →
                // PeerNotFound.
                let a_present = routing
                    .forward(&peer_a, Bytes::from_static(b"probe-a"))
                    .is_ok();
                let s_present = routing
                    .forward(&peer_s, Bytes::from_static(b"probe-s"))
                    .is_ok();
                assert!(
                    a_present,
                    "data-active peer must survive past the timeout windows (R3, F-219)"
                );
                assert!(
                    !s_present,
                    "silent peer must be evicted by the keepalive timeout (windows are real)"
                );
                assert_eq!(routing.len(), 1, "only the data-active peer remains");

                conn_a.close(0u32.into(), b"done");
                let _ = cancel_tx.send(());
                let server_result = server_handle.await.expect("server task joins");
                server_result.expect("serve() returns Ok");
            })
            .await;
    }

    // ---------------------------------------------------------------
    // M-072 live probe — measures idle-survival against an EXTERNAL
    // relay (old binary then new), the "compare before deploy" gate.
    // ---------------------------------------------------------------

    /// Accept any server certificate. The M-072 probe connects to a real
    /// relay whose cert we don't pin — we are measuring keepalive
    /// behavior, not authenticating the relay (the relay authenticates
    /// US via the HMAC token). TEST-ONLY, only reachable from the
    /// `#[ignore]`d probe.
    #[derive(Debug)]
    struct SkipServerVerification(std::sync::Arc<rustls::crypto::CryptoProvider>);

    impl rustls::client::danger::ServerCertVerifier for SkipServerVerification {
        fn verify_server_cert(
            &self,
            _end_entity: &rustls::pki_types::CertificateDer<'_>,
            _intermediates: &[rustls::pki_types::CertificateDer<'_>],
            _server_name: &rustls::pki_types::ServerName<'_>,
            _ocsp: &[u8],
            _now: rustls::pki_types::UnixTime,
        ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        }
        fn verify_tls12_signature(
            &self,
            message: &[u8],
            cert: &rustls::pki_types::CertificateDer<'_>,
            dss: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            rustls::crypto::verify_tls12_signature(
                message,
                cert,
                dss,
                &self.0.signature_verification_algorithms,
            )
        }
        fn verify_tls13_signature(
            &self,
            message: &[u8],
            cert: &rustls::pki_types::CertificateDer<'_>,
            dss: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            rustls::crypto::verify_tls13_signature(
                message,
                cert,
                dss,
                &self.0.signature_verification_algorithms,
            )
        }
        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            self.0.signature_verification_algorithms.supported_schemes()
        }
    }

    /// quinn client that skips cert verification + advertises the relay's
    /// `nb-quic` ALPN (QUIC mandates a matching ALPN).
    fn insecure_quic_client() -> quinn::ClientConfig {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
        let mut crypto =
            rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
                .dangerous()
                .with_custom_certificate_verifier(std::sync::Arc::new(SkipServerVerification(
                    provider,
                )))
                .with_no_client_auth();
        crypto.alpn_protocols =
            vec![goat_relay_protocol::transport::quic::ALPN.as_bytes().to_vec()];
        let quic_client =
            quinn::crypto::rustls::QuicClientConfig::try_from(crypto).expect("QuicClientConfig");
        quinn::ClientConfig::new(std::sync::Arc::new(quic_client))
    }

    /// M-072 — live idle-survival probe against an EXTERNAL relay. Connects
    /// a QUIC peer, sends low-rate `Transport` data but NEVER echoes a
    /// HealthCheck (the real flap mode: lost/absent HC echoes on a relayed
    /// path), and times how long until the relay closes the session. On the
    /// OLD relay (HC-only liveness) the peer is evicted ~one
    /// `SENDER_TIMEOUT_WINDOW` (~45 s) despite carrying data; on the NEW
    /// relay (data-as-liveness, F-219/ADR 1057) it HOLDS for the full
    /// window. Old fails this test, new passes it — the live analog of P0.
    ///
    /// ```text
    /// RELAY_ADDR=198.51.100.162:33073 RELAY_SECRET=... \
    ///   cargo test -p goat-relay --lib m072_external_relay_idle_survival \
    ///   -- --ignored --nocapture
    /// ```
    /// Optional env: `M072_HOLD_SECS` (default 120), `M072_DATA_INTERVAL_SECS`
    /// (default 10).
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "M-072 live probe; needs RELAY_ADDR + RELAY_SECRET env"]
    async fn m072_external_relay_idle_survival() {
        use std::time::{Duration, Instant};

        let addr: SocketAddr = std::env::var("RELAY_ADDR")
            .expect("RELAY_ADDR=ip:port")
            .parse()
            .expect("parse RELAY_ADDR");
        let secret = std::env::var("RELAY_SECRET").expect("RELAY_SECRET");
        let hold_secs: u64 = std::env::var("M072_HOLD_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(120);
        let interval: u64 = std::env::var("M072_DATA_INTERVAL_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(10);

        let mut ep =
            quinn::Endpoint::client("0.0.0.0:0".parse().unwrap()).expect("client endpoint");
        ep.set_default_client_config(insecure_quic_client());
        eprintln!(
            "[M-072] connecting to {addr} (hold target {hold_secs}s, data every {interval}s)"
        );
        let conn = ep
            .connect(addr, "localhost")
            .expect("connect")
            .await
            .expect("handshake");

        let peer = peer_id(0x42);
        let expiry = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64)
            + 3600;
        let token = crate::auth::generate_test_token_bound(secret.as_bytes(), &peer, expiry);
        conn.send_datagram(Bytes::from(
            Auth::new(peer, Bytes::from(token)).expect("auth").marshal(),
        ))
        .expect("send Auth");
        let resp = conn.read_datagram().await.expect("AuthResponse");
        let parsed = AuthResponse::parse(&resp).expect("parse AuthResponse");
        eprintln!("[M-072] authed OK; relay advertised {}", parsed.address);

        // First data frame immediately, then every `interval`.
        let send_data = |conn: &quinn::Connection| -> Result<(), String> {
            let t = Transport::new(peer_id(0x99), Bytes::from_static(b"m072-keepalive-data"))
                .expect("transport");
            conn.send_datagram(Bytes::from(t.marshal()))
                .map_err(|e| format!("{e}"))
        };
        send_data(&conn).expect("first data send");

        let start = Instant::now();
        let mut last_data = Instant::now();
        loop {
            tokio::select! {
                r = conn.read_datagram() => {
                    match r {
                        // Relay HC or any frame: discard, do NOT echo.
                        Ok(_dg) => {}
                        Err(e) => {
                            let secs = start.elapsed().as_secs_f64();
                            eprintln!("[M-072] RESULT: FLAP — relay closed idle-data session after {secs:.1}s ({e})");
                            panic!("relay evicted idle-data peer after {secs:.1}s (expected on OLD; must HOLD on NEW)");
                        }
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(250)) => {
                    if last_data.elapsed().as_secs() >= interval {
                        if let Err(e) = send_data(&conn) {
                            let secs = start.elapsed().as_secs_f64();
                            eprintln!("[M-072] RESULT: FLAP — send failed after {secs:.1}s ({e})");
                            panic!("relay closed after {secs:.1}s");
                        }
                        last_data = Instant::now();
                        eprintln!("[M-072] t={:.0}s data sent; still connected", start.elapsed().as_secs_f64());
                    }
                    if start.elapsed().as_secs() >= hold_secs {
                        eprintln!("[M-072] RESULT: HELD — survived {hold_secs}s of idle-data, no eviction");
                        break;
                    }
                }
            }
        }
        conn.close(0u32.into(), b"m072 done");
    }

    /// Multi-peer-pair concurrency integration test for QUIC, mirroring
    /// upstream's `relay/test/benchmark_test.go::TestRelayDataTransfer`
    /// shape. Skipped by default (`#[ignore]`) like upstream's
    /// `t.SkipNow()`. Run via:
    ///
    /// ```text
    /// cargo test --bin goat-relay --release -- --ignored \
    ///   quic_listener_multi_peer_pair_routing
    /// ```
    ///
    /// Sweeps peer-pair counts; for each `N`, spawns `2*N` QUIC
    /// connections (paired into A/B), each pair exchanges
    /// `FRAMES_PER_PAIR` `Transport` datagrams; assert all forwarded
    /// datagrams arrive at the receiver with `peer_id` rewritten and
    /// payload preserved.
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "long-running multi-peer concurrency test; run with --ignored"]
    async fn quic_listener_multi_peer_pair_routing() {
        const PEER_PAIR_COUNTS: &[usize] = &[2, 10, 30];
        const FRAMES_PER_PAIR: usize = 50;

        let (certs, key) = self_signed_localhost();
        let server_cert = certs[0].clone();
        let server_config = quic_compatible_server_config(certs, key);
        let client_config = quic_client_config(server_cert);

        let bind_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let advertised = "test-relay.example.invalid:33073".to_string();
        let advertised_for_assert = advertised.clone();
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        let (bound_tx, bound_rx) = oneshot::channel::<Option<SocketAddr>>();
        let routing = RoutingTable::new();
        let routing_for_server = routing.clone();

        let local = tokio::task::LocalSet::new();
        let server_handle = local.spawn_local(async move {
            let cfg = QuicListenerConfig {
                bind_addr,
                tls_config: server_config,
                advertised_addr: advertised,
                routing_table: routing_for_server,
                validator: std::sync::Arc::new(crate::auth::TimedHmacValidator::from_secret(
                    TEST_AUTH_SECRET,
                )),
                keepalive_timings: crate::keepalive::KeepaliveTimings::default(),
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
                    eprintln!("[multi-peer-quic] sweep pair_count={pair_count}");
                    run_one_sweep_step_quic(
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

    /// One iteration of the QUIC peer-pair-count sweep.
    async fn run_one_sweep_step_quic(
        pair_count: usize,
        frames_per_pair: usize,
        bound: SocketAddr,
        client_config: quinn::ClientConfig,
        advertised: &str,
        routing: &RoutingTable,
    ) {
        let mut a_ids = Vec::with_capacity(pair_count);
        let mut b_ids = Vec::with_capacity(pair_count);
        let mut a_conns = Vec::with_capacity(pair_count);
        let mut b_conns = Vec::with_capacity(pair_count);
        for i in 0..pair_count {
            let a_byte = i as u8;
            let b_byte = 0x80u8 | i as u8;
            a_ids.push(peer_id(a_byte));
            b_ids.push(peer_id(b_byte));
            a_conns.push(connect_and_auth(bound, client_config.clone(), a_byte, advertised).await);
            b_conns.push(connect_and_auth(bound, client_config.clone(), b_byte, advertised).await);
        }

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

        for i in 0..pair_count {
            for f in 0..frames_per_pair {
                let payload = Bytes::from(format!("a={i:02x}-frame={f:04x}-pair-{pair_count}"));
                let transport = Transport::new(b_ids[i], payload).expect("Transport");
                let bytes = Bytes::from(transport.marshal());
                a_conns[i]
                    .send_datagram(bytes)
                    .expect("A → B send_datagram");
            }
        }

        for i in 0..pair_count {
            for f in 0..frames_per_pair {
                let datagram = b_conns[i]
                    .read_datagram()
                    .await
                    .unwrap_or_else(|e| panic!("B[{i}] expected datagram {f}/{frames_per_pair}: {e}"));
                let parsed = Transport::parse(&datagram).expect("parse forwarded Transport");
                assert_eq!(
                    parsed.peer_id, a_ids[i],
                    "B[{i}] frame {f}: peer_id rewritten to source"
                );
                let expected = format!("a={i:02x}-frame={f:04x}-pair-{pair_count}");
                assert_eq!(
                    &parsed.payload[..],
                    expected.as_bytes(),
                    "B[{i}] frame {f}: payload preserved"
                );
            }
        }

        for c in a_conns.into_iter().chain(b_conns) {
            c.close(0u32.into(), b"sweep iter done");
        }
        for _ in 0..200 {
            if routing.is_empty() {
                break;
            }
            tokio::task::yield_now().await;
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }
}
