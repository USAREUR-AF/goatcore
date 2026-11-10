// SPDX-License-Identifier: Apache-2.0
//
// The operator surface. Every way a sample can fail to reach the partner has a
// counter, because the alternative is the F-177 shape: an outbound feed that
// has silently stopped looks exactly like an outbound feed with nothing to say.
//
// Label cardinality is bounded by construction: `prefix` takes one value per
// release prefix in the agreement, and `reason` one per variant of the refusal
// and drop enums. Neither is derived from a key.

use anyhow::Result;
use prometheus::{
    register_int_counter_vec_with_registry, register_int_counter_with_registry,
    register_int_gauge_with_registry, Encoder, IntCounter, IntCounterVec, IntGauge, Registry,
    TextEncoder,
};
use std::sync::Arc;

#[derive(Clone)]
pub struct Metrics {
    registry: Arc<Registry>,
    /// Samples the local release subscriptions delivered to us.
    pub received: IntCounterVec,
    /// Samples put onto the peering edge session.
    pub published: IntCounterVec,
    /// Refused by the agreement — the release set or an envelope ceiling.
    pub refused: IntCounterVec,
    /// Lost in our own pipeline: queue full, edge down, put failed.
    pub dropped: IntCounterVec,
    /// Samples whose origin instant did not exist, so the receiver's timestamp
    /// is the crossing time and not the observation time.
    pub untimestamped: IntCounter,
    /// Samples whose zenoh SourceInfo we deliberately did not forward.
    pub source_info_dropped: IntCounter,
    /// 1 when the peering edge session has at least one transport peer.
    pub edge_linked: IntGauge,
    /// 1 once every local release subscription is declared.
    pub local_subscribed: IntGauge,
    /// Distinct wire keys published so far, against the receiver's ceiling.
    pub distinct_wire_keys: IntGauge,
    pub distinct_wire_keys_limit: IntGauge,
    /// Unix seconds of the agreement's expiry, so it can be alerted on BEFORE
    /// the flow stops rather than after.
    pub agreement_expiry_seconds: IntGauge,
    pub queue_depth: IntGauge,
}

impl Metrics {
    pub fn new(sovereign_id: &str, partner: &str) -> Result<Self> {
        let registry = Registry::new_custom(
            None,
            Some(
                [
                    ("sovereign".to_string(), sovereign_id.to_string()),
                    ("partner".to_string(), partner.to_string()),
                ]
                .into_iter()
                .collect(),
            ),
        )?;
        macro_rules! ctr {
            ($n:expr, $h:expr, $l:expr) => {
                register_int_counter_vec_with_registry!($n, $h, $l, registry)?
            };
        }
        macro_rules! g {
            ($n:expr, $h:expr) => {
                register_int_gauge_with_registry!($n, $h, registry)?
            };
        }
        Ok(Self {
            received: ctr!(
                "goat_fed_republisher_received_total",
                "Samples delivered by the local release subscriptions.",
                &["prefix"]
            ),
            published: ctr!(
                "goat_fed_republisher_published_total",
                "Samples put onto the peering edge under the wire prefix.",
                &["prefix"]
            ),
            refused: ctr!(
                "goat_fed_republisher_refused_total",
                "Samples the agreement refused to let cross, by reason.",
                &["reason"]
            ),
            dropped: ctr!(
                "goat_fed_republisher_dropped_total",
                "Samples lost inside this process rather than by the agreement, by reason.",
                &["reason"]
            ),
            untimestamped: register_int_counter_with_registry!(
                "goat_fed_republisher_untimestamped_total",
                "Samples with no origin instant. For these the receiver's timestamp is the \
                 crossing time, not the observation time; the fix is upstream timestamping.",
                registry
            )?,
            source_info_dropped: register_int_counter_with_registry!(
                "goat_fed_republisher_source_info_dropped_total",
                "Samples whose zenoh SourceInfo was deliberately not forwarded across the \
                 sovereign boundary.",
                registry
            )?,
            edge_linked: g!(
                "goat_fed_republisher_edge_linked",
                "1 when the publishing session has a transport peer: our core router, which the \
                 peering edge carries the wire name out from. It does not see the partner link."
            ),
            local_subscribed: g!(
                "goat_fed_republisher_local_subscribed",
                "1 once every release subscription is declared on the local session."
            ),
            distinct_wire_keys: g!(
                "goat_fed_republisher_distinct_wire_keys",
                "Distinct wire keys published since start."
            ),
            distinct_wire_keys_limit: g!(
                "goat_fed_republisher_distinct_wire_keys_limit",
                "The receiver's declared distinct-key ceiling from the agreement."
            ),
            agreement_expiry_seconds: g!(
                "goat_fed_republisher_agreement_expiry_seconds",
                "Unix seconds at which the agreement expires and the flow stops."
            ),
            queue_depth: g!(
                "goat_fed_republisher_queue_depth",
                "Configured depth of the bounded hand-off queue."
            ),
            registry: Arc::new(registry),
        })
    }

    pub fn render(&self) -> Result<String> {
        let mut buf = Vec::new();
        TextEncoder::new().encode(&self.registry.gather(), &mut buf)?;
        Ok(String::from_utf8(buf)?)
    }
}
