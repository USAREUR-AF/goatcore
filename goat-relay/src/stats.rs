//! Minimal white-box stats endpoint (ADR 1055 §2).
//!
//! A single hand-rolled HTTP/1.1 responder that serves a JSON snapshot of
//! the relay's observability counters — the drop counters, the
//! per-peer forwarded byte/frame counters (ADR 1055 §2b), and the
//! registered-connection count. Every request returns the same snapshot
//! (the method + path are ignored); it is a read-only reveal surface, never
//! a control plane.
//!
//! **Default OFF.** The endpoint binds only when `--stats-addr` is given,
//! and is intended for a LOCALHOST bind (`127.0.0.1:PORT`) consumed by the
//! on-host self-publisher (ADR 1058 — folds the per-peer rows into the
//! `deep` block it already POSTs to the Thor ingest) or an operator SSM
//! pull. It is deliberately not a new public port: the relay is not a mesh
//! peer, so nothing off-host should reach it directly.
//!
//! No `serde` dependency — the crate is minimal-dep on purpose (cf. the
//! hand-rolled arg parser in `main.rs`), and the snapshot is small and
//! purely numeric / hex, so it is hand-formatted. Runs on shard 0's
//! `LocalSet` (`spawn_local`), reading the shared [`RoutingTable`].

use std::future::Future;
use std::net::SocketAddr;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::routing::RoutingTable;

/// Serve the stats snapshot on `addr` until `cancel` resolves.
///
/// Non-fatal by construction: a bind failure logs and returns — stats is
/// advisory and must never take down the data plane. Connections are
/// handled inline under a hard timeout; stats requests are tiny and
/// infrequent (a local poller every ~15 s), so a slow client cannot wedge
/// the accept loop for longer than the cap.
pub async fn serve_stats(addr: SocketAddr, routing: RoutingTable, cancel: impl Future<Output = ()>) {
    let listener = match TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("stats: bind {addr} failed: {e}; stats endpoint disabled (data plane unaffected)");
            return;
        }
    };
    eprintln!("stats: serving JSON snapshot on http://{addr}/ (ADR 1055 white-box surface)");
    tokio::pin!(cancel);
    loop {
        tokio::select! {
            () = &mut cancel => {
                eprintln!("stats: endpoint shutting down");
                return;
            }
            accepted = listener.accept() => match accepted {
                Ok((stream, _peer)) => {
                    let body = render_json(&routing);
                    let _ = tokio::time::timeout(Duration::from_secs(2), handle_conn(stream, body)).await;
                }
                Err(e) => eprintln!("stats: accept error: {e}"),
            },
        }
    }
}

/// Drain the request (ignored — any request gets the same snapshot) and
/// write the JSON body as a `Connection: close` HTTP/1.1 response.
async fn handle_conn(mut stream: TcpStream, body: String) {
    let mut scratch = [0u8; 1024];
    let _ = stream.read(&mut scratch).await; // one read is enough for a GET line; not parsed
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(resp.as_bytes()).await;
    let _ = stream.flush().await;
    let _ = stream.shutdown().await;
}

/// Render the observability snapshot as JSON. Hand-formatted: every value
/// is a `u64` counter or a `[0-9a-f]` hex peer-id, so no string escaping is
/// possible or needed. The peer anchor is the opaque [`crate::routing`]
/// `PeerId` rendered as lowercase hex — never an alias (ADR 1039; the relay
/// has no portal, so display-name resolution is the consumer's job).
#[must_use]
pub fn render_json(routing: &RoutingTable) -> String {
    let drops = routing.drop_stats();
    let peers = routing.peer_stats();
    let mut s = String::with_capacity(96 + peers.len() * 160);
    s.push('{');
    s.push_str(&format!("\"connections\":{},", routing.len()));
    s.push_str(&format!(
        "\"drops\":{{\"writer_full\":{},\"writer_closed\":{},\"peer_not_found\":{},\"datagram_too_large\":{}}},",
        drops.writer_full, drops.writer_closed, drops.peer_not_found, drops.datagram_too_large
    ));
    s.push_str(&format!(
        "\"subscriptions_refused\":{},",
        routing.subscriptions_refused()
    ));
    s.push_str("\"peers\":[");
    for (i, p) in peers.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!(
            "{{\"peer_id\":\"{}\",\"transport\":\"{}\",\"fwd_bytes\":{},\"fwd_frames\":{},\"rcv_bytes\":{},\"rcv_frames\":{}}}",
            hex(&p.peer_id.0), p.transport.as_str(), p.fwd_bytes, p.fwd_frames, p.rcv_bytes, p.rcv_frames
        ));
    }
    s.push_str("]}");
    s
}

/// Lowercase-hex encode a byte slice (peer-id → stable opaque JSON string).
fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::{Frame, RoutingTable};
    use bytes::Bytes;
    use goat_relay_protocol::messages::{PeerId, PEER_ID_SIZE};
    use tokio::sync::mpsc;

    fn peer(b: u8) -> PeerId {
        PeerId::from_bytes([b; PEER_ID_SIZE])
    }

    #[test]
    fn render_json_reports_counts_and_per_peer_hex() {
        let table = RoutingTable::new();
        let (src, dst) = (peer(0xAB), peer(0xCD));
        let (tx_s, _rx_s) = mpsc::channel::<Frame>(crate::routing::WRITER_QUEUE_CAPACITY);
        let (tx_d, mut rx_d) = mpsc::channel::<Frame>(crate::routing::WRITER_QUEUE_CAPACITY);
        table.insert(src, tx_s);
        table.insert(dst, tx_d);
        table.forward_data(&src, &dst, Bytes::from_static(b"abcd")).unwrap(); // 4 bytes
        let _ = rx_d.try_recv();

        let json = render_json(&table);
        assert!(json.contains("\"connections\":2"), "{json}");
        assert!(json.contains("\"peer_not_found\":0"), "{json}");
        assert!(json.contains(&"ab".repeat(PEER_ID_SIZE)), "source peer hex present: {json}");
        assert!(json.contains("\"fwd_bytes\":4"), "source charged ingress: {json}");
        assert!(json.contains("\"rcv_bytes\":4"), "dest charged egress: {json}");
    }

    #[test]
    fn render_json_empty_table_is_valid() {
        let json = render_json(&RoutingTable::new());
        assert_eq!(json, "{\"connections\":0,\"drops\":{\"writer_full\":0,\"writer_closed\":0,\"peer_not_found\":0,\"datagram_too_large\":0},\"subscriptions_refused\":0,\"peers\":[]}");
    }
}
