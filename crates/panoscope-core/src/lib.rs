// SPDX-License-Identifier: Apache-2.0
//
// panoscope-core — shared substrate for the inspection-sibling cluster
// (panoscope, stethoscope, flashlight). Two waves of factor-out have
// landed:
//
// Wave 1 — stethoscope Phase B (Block 90):
//   - portal::PortalClient — HTTP client for goat-zenoh-portal's
//     /api/vendors + /api/schemas surface. Both fetch_fds (panoscope's
//     existing use) and the enumeration calls stethoscope needs
//     (list_topics, get_topic, list_schemas).
//   - audit_event::AuditEvent — shape mirroring the audit-subscriber
//     writer (ops/zenoh-trial-peers/audit-subscriber/src/main.rs).
//   - audit_tail::AuditTail — generic S3 ListObjectsV2 + GetObject
//     walker over the audit-integrity bucket's NDJSON layout. Returns
//     parsed AuditEvent values; consumers attach their own apply-side
//     semantics (panoscope = topology upsert, stethoscope = flow-rate
//     count + sample decode).
//
// Wave 2 — flashlight Phase B (Block 89):
//   - types: PeerId, TopicKey, Role, NodeKind (role taxonomy)
//   - keyexpr: keyexprs_related, strip_wildcards
//   - time: now_ms, extract_vendor (URI-SAN parser)
//   - peer: PeerInfo (struct + role() classification method)
//   - topology: Topology (peer registry with schema-decl + admin-namespace
//     discovery paths, F-147 upsert_admin_peer semantics)
//   - tls: TlsConfig + apply_tls_to_zenoh_config (zenoh 1.9.0 mTLS
//     injection)
//
// NOT in scope here (stays in the consumer crates):
//   - FlowState / EdgeState / EdgeKey / TokenBucket (panoscope-specific
//     per-edge aggregator + sampling rate limiter)
//   - The Zenoh data_tail / admin_tail runtime loops
//   - The MinIO replay orchestration (`minio_replay::replay_into_topology`
//     uses our `AuditEvent` for parsing + our `Topology` for the apply
//     target, but the walker is panoscope-specific)
//   - panoscope's protobuf descriptor cache (`decoder.rs` ~340 LOC)
//   - flashlight-specific surface code (URL filter grammar, SSE event
//     shape, Tier 1 HTML/JS assets, Tier 2 boot logic)

pub mod aclcheck;
pub mod advanced_sub;
pub mod alias;
pub mod audit_event;
pub mod audit_tail;
pub mod census;
pub mod envelope;
pub mod keyexpr;
pub mod meshgraph;
pub mod netbird;
pub mod peer;
pub mod peername;
pub mod peerowner;
pub mod pinned_tls;
pub mod portal;
pub mod portsets;
pub mod presence;
pub mod pubsub_hygiene;
pub mod time;
pub mod tls;
pub mod topology;
pub mod types;

pub use advanced_sub::{
    declare_catchup_subscriber, CatchupConfig, CatchupSubscriber, DEFAULT_MAX_AGE_SECS,
    DEFAULT_RECOVERY_PERIOD,
};
pub use alias::{
    key_vendor_matches, strip_anchor, vendor_segment, AliasRegistry, AliasRegistryHealth,
    AliasRegistryState, Labeled,
};
pub use audit_event::AuditEvent;
pub use audit_tail::{AuditTail, AuditTailConfig};
pub use census::{
    apply_reply, census_sources, parse_resource_key, query_census, Census, ResourceRole,
    CENSUS_SELECTORS,
};
pub use keyexpr::{keyexprs_related, strip_wildcards};
pub use peer::PeerInfo;
pub use portal::{
    AliasRow, DecodedSample, FetchOutcome, PortalClient, PortalError, SchemaMeta, Topic, Vendor,
};
pub use presence::{
    declare_presence_token, declare_presence_watch, parse_presence_key, presence_key,
    query_presence, PresenceId, ALL_PRESENCE_KEYEXPR, PRESENCE_SEGMENT,
};
pub use pubsub_hygiene::{
    classify_publisher, HygieneFinding, PublisherHygiene, Retrievability, Severity, TopicStat,
    Verdict,
};
pub use time::{extract_vendor, now_ms};
pub use tls::{apply_tls_to_zenoh_config, TlsConfig};
pub use topology::Topology;
pub use types::{
    is_reserved_namespace, NodeClass, NodeKind, PeerId, Role, TopicKey, INFRA_VENDORS,
    RESERVED_NAMESPACE_PREFIXES,
};

pub mod replay_fetch;

#[cfg(test)]
mod audit_metadata_tests;
