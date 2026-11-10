// SPDX-License-Identifier: Apache-2.0
//
// The peering edge, and what happens when it is not there.
//
// LOSS BEHAVIOUR — the choice, and why it is not configurable:
//
//   BLOCK is forbidden. Blocking on a slow or absent partner link applies
//   backpressure through the local subscription and into the local fabric. A
//   federation partner is the least trusted client on the system, and ADR 1123
//   is that client demand must never amplify into fabric load. A partner who
//   stops reading must not be able to slow down our own bus.
//
//   BUFFER-UNBOUNDED is forbidden. The buffer's size would then be chosen by
//   the partner's downtime, which is a memory exhaustion any partner can
//   trigger by unplugging a cable.
//
//   So: DROP, bounded, and counted. One bounded hand-off queue; when it is
//   full the arriving sample is dropped and counted. When the edge session has
//   no transport peer the sample is dropped BEFORE the put rather than after —
//   a put on a linkless client session succeeds locally and is discarded by
//   the transport, which is loss that no counter would ever see.
//
// WHAT THE OPERATOR SEES. The publishing session dials OUR CORE router, which
// the peering edge carries the wire name out from (roles/goat-fed-republisher).
// `edge_linked` therefore reports that session: it falls to 0 the moment the
// link to our core drops, and `dropped_total{reason="edge_down"}` then climbs at
// the feed's real rate, so the cost of that outage is a number rather than an
// inference. It does NOT see the partner link: with the core reachable and the
// partner link down, samples reach our core and go no further, and only the
// round trip (ops/federation/roundtrip/) and the partner's own store show it.
//
// The names keep `edge` because the code path is the one the README describes
// for a session to a peering edge; the deployment points it at the core.
//
// WHAT STILL CANNOT BE SEEN FROM THIS SIDE. A sample put while `edge_linked`
// was 1, on a link that failed before the transport flushed it, is lost with
// no local signal. That residual is inherent to a one-way put and is closed
// only by the receiver reporting back, which no part of this design does.

use anyhow::Result;
use tokio::sync::mpsc;
use tracing::{info, warn};
use zenoh::bytes::{Encoding, ZBytes};
use zenoh::sample::SampleKind;
use zenoh::time::Timestamp;
use zenoh::Wait;

use crate::metrics::Metrics;

/// One sample, already mapped, ready for the wire. It carries the staging key
/// for LOGS ONLY — the staging name never reaches a put.
#[derive(Debug, Clone)]
pub struct Outbound {
    pub wire_key: String,
    pub staging_key: String,
    pub release_prefix: String,
    pub kind: SampleKind,
    pub payload: Vec<u8>,
    pub encoding: Encoding,
    pub timestamp: Option<Timestamp>,
    pub attachment: Option<Vec<u8>>,
}

pub trait EdgeSink: Send + 'static {
    /// Put (or delete) one sample on the peering edge.
    fn publish(&self, out: &Outbound) -> Result<()>;
    /// Whether the edge session currently has a transport peer. Racy by
    /// nature; used to turn the common, long case (link down) from silent loss
    /// into a counted one.
    fn linked(&self) -> bool;
}

pub struct ZenohEdge {
    session: zenoh::Session,
}

impl ZenohEdge {
    pub fn new(session: zenoh::Session) -> Self {
        Self { session }
    }
}

impl EdgeSink for ZenohEdge {
    fn publish(&self, out: &Outbound) -> Result<()> {
        match out.kind {
            SampleKind::Put => {
                let mut b = self
                    .session
                    .put(out.wire_key.as_str(), ZBytes::from(out.payload.clone()))
                    .encoding(out.encoding.clone());
                if let Some(ts) = out.timestamp {
                    b = b.timestamp(ts);
                }
                if let Some(a) = &out.attachment {
                    b = b.attachment(ZBytes::from(a.clone()));
                }
                b.wait()
                    .map_err(|e| anyhow::anyhow!("put({}) failed: {e}", out.wire_key))
            }
            SampleKind::Delete => {
                let mut b = self.session.delete(out.wire_key.as_str());
                if let Some(ts) = out.timestamp {
                    b = b.timestamp(ts);
                }
                b.wait()
                    .map_err(|e| anyhow::anyhow!("delete({}) failed: {e}", out.wire_key))
            }
        }
    }

    fn linked(&self) -> bool {
        // ROUTERS AS WELL AS PEERS. The edge session is opened in `mode: "client"`
        // against the far side's router, where a router appears in `routers_zid()`
        // and `peers_zid()` is structurally always empty. With `peers_zid()` alone
        // `linked()` would be permanently false and EVERY outbound sample would be
        // counted `dropped{reason="edge_down"}` while the TLS session to that
        // router is open and healthy. A mock EdgeSink returning `linked() == true`
        // cannot catch this, so the one thing that can be wrong here is the one
        // thing such a test replaces.
        let info = self.session.info();
        !info.peers_zid().wait().collect::<Vec<_>>().is_empty()
            || !info.routers_zid().wait().collect::<Vec<_>>().is_empty()
    }
}

/// How often the edge-link gauge is refreshed when no traffic is flowing, so it
/// reports the edge's state rather than the state at the last sample.
const EDGE_LINK_POLL: std::time::Duration = std::time::Duration::from_secs(5);

/// Drain the hand-off queue onto the edge. Ends only when the queue closes,
/// which happens when the subscriber side is gone — at which point the process
/// exits so its supervisor restarts it, rather than idling as a healthy-looking
/// no-op with a live /metrics endpoint (the shape that cost the audit
/// subscriber five silent hours).
pub async fn run_publisher<S: EdgeSink>(
    mut rx: mpsc::Receiver<Outbound>,
    sink: S,
    metrics: Metrics,
) {
    let mut was_linked: Option<bool> = None;

    // Publish the gauge on a timer as well as on traffic. Setting it ONLY in the
    // receive loop means it reads 0 until the first sample arrives, which is
    // indistinguishable from "the edge is down" — and on a quiet link that is the
    // normal state, not an edge case. It cost a wrong diagnosis : the
    // gauge read 0 on a healthy edge with both TLS sessions established, simply
    // because nothing had flowed yet.
    //
    // The gauge answers "is the edge up", so it has to be answerable without
    // traffic. This is a read of local session state, not a probe, so ticking it
    // costs nothing on the wire.
    metrics.edge_linked.set(i64::from(sink.linked()));

    loop {
        let out = tokio::select! {
            recv = rx.recv() => match recv {
                Some(out) => out,
                None => break,
            },
            _ = tokio::time::sleep(EDGE_LINK_POLL) => {
                metrics.edge_linked.set(i64::from(sink.linked()));
                continue;
            }
        };
        let linked = sink.linked();
        metrics.edge_linked.set(i64::from(linked));
        if was_linked != Some(linked) {
            if linked {
                info!("publishing session linked to our core; outbound flow resumed");
            } else {
                warn!(
                    "publishing session has no transport peer; outbound samples are being dropped"
                );
            }
            was_linked = Some(linked);
        }
        if !linked {
            metrics.dropped.with_label_values(&["edge_down"]).inc();
            continue;
        }
        match sink.publish(&out) {
            Ok(()) => {
                metrics
                    .published
                    .with_label_values(&[out.release_prefix.as_str()])
                    .inc();
            }
            Err(e) => {
                metrics.dropped.with_label_values(&["publish_error"]).inc();
                warn!(
                    wire_key = out.wire_key.as_str(),
                    staging_key = out.staging_key.as_str(),
                    error = %e,
                    "put on the peering edge failed",
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct FakeEdge {
        linked: Arc<AtomicBool>,
        fail: Arc<AtomicBool>,
        sent: Arc<Mutex<Vec<Outbound>>>,
    }

    impl EdgeSink for FakeEdge {
        fn publish(&self, out: &Outbound) -> Result<()> {
            if self.fail.load(Ordering::SeqCst) {
                anyhow::bail!("edge refused");
            }
            self.sent.lock().unwrap().push(out.clone());
            Ok(())
        }
        fn linked(&self) -> bool {
            self.linked.load(Ordering::SeqCst)
        }
    }

    fn sample(key: &str) -> Outbound {
        Outbound {
            wire_key: key.to_string(),
            staging_key: "release/weather/forecast".into(),
            release_prefix: "release/weather/**".into(),
            kind: SampleKind::Put,
            payload: b"clear-32C".to_vec(),
            encoding: Encoding::default(),
            timestamp: None,
            attachment: None,
        }
    }

    fn metrics() -> Metrics {
        Metrics::new("zagros", "any").unwrap()
    }

    #[tokio::test]
    async fn a_linked_edge_carries_the_sample() {
        let edge = FakeEdge::default();
        edge.linked.store(true, Ordering::SeqCst);
        let m = metrics();
        let (tx, rx) = mpsc::channel(8);
        tx.send(sample("from/zagros/weather/forecast"))
            .await
            .unwrap();
        drop(tx);
        run_publisher(rx, edge.clone(), m.clone()).await;
        let sent = edge.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].wire_key, "from/zagros/weather/forecast");
        assert_eq!(m.edge_linked.get(), 1);
    }

    #[tokio::test]
    async fn an_unlinked_edge_drops_and_counts_rather_than_putting_into_the_void() {
        let edge = FakeEdge::default(); // linked == false
        let m = metrics();
        let (tx, rx) = mpsc::channel(8);
        for _ in 0..3 {
            tx.send(sample("from/zagros/weather/forecast"))
                .await
                .unwrap();
        }
        drop(tx);
        run_publisher(rx, edge.clone(), m.clone()).await;
        assert!(
            edge.sent.lock().unwrap().is_empty(),
            "nothing may be put on a dead link"
        );
        assert_eq!(m.dropped.with_label_values(&["edge_down"]).get(), 3);
        assert_eq!(
            m.published.with_label_values(&["release/weather/**"]).get(),
            0
        );
        assert_eq!(m.edge_linked.get(), 0);
    }

    #[tokio::test]
    async fn an_edge_that_comes_back_resumes_with_no_operator_action() {
        let edge = FakeEdge::default();
        let m = metrics();
        let (tx, rx) = mpsc::channel(8);
        tx.send(sample("from/zagros/weather/a")).await.unwrap();
        let pump = {
            let edge = edge.clone();
            let m = m.clone();
            #[allow(clippy::disallowed_methods)] // test harness, not the relay hot path
            tokio::spawn(async move { run_publisher(rx, edge, m).await })
        };
        // Let the first sample be dropped against a dead link, then restore it.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        edge.linked.store(true, Ordering::SeqCst);
        tx.send(sample("from/zagros/weather/b")).await.unwrap();
        drop(tx);
        pump.await.unwrap();
        let sent = edge.sent.lock().unwrap();
        assert_eq!(sent.len(), 1, "only the post-recovery sample crosses");
        assert_eq!(sent[0].wire_key, "from/zagros/weather/b");
        assert_eq!(m.dropped.with_label_values(&["edge_down"]).get(), 1);
    }

    #[tokio::test]
    async fn a_failing_put_is_counted_and_does_not_stop_the_flow() {
        let edge = FakeEdge::default();
        edge.linked.store(true, Ordering::SeqCst);
        edge.fail.store(true, Ordering::SeqCst);
        let m = metrics();
        let (tx, rx) = mpsc::channel(8);
        tx.send(sample("from/zagros/weather/a")).await.unwrap();
        tx.send(sample("from/zagros/weather/b")).await.unwrap();
        drop(tx);
        run_publisher(rx, edge, m.clone()).await;
        assert_eq!(m.dropped.with_label_values(&["publish_error"]).get(), 2);
    }

    #[test]
    fn a_full_queue_drops_the_arriving_sample_and_never_blocks() {
        // The ingest side's contract: `try_send`, never `send`. Blocking here
        // would push a partner's slowness back into our own fabric.
        let (tx, _rx) = mpsc::channel::<Outbound>(2);
        assert!(tx.try_send(sample("from/zagros/weather/a")).is_ok());
        assert!(tx.try_send(sample("from/zagros/weather/b")).is_ok());
        match tx.try_send(sample("from/zagros/weather/c")) {
            Err(mpsc::error::TrySendError::Full(_)) => {}
            other => panic!("expected Full, got {other:?}"),
        }
    }
}
