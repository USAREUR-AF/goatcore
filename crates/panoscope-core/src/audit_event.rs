// SPDX-License-Identifier: Apache-2.0
//
// AuditEvent — the on-the-wire shape the Block 82S audit-subscriber
// writes into MinIO `audit-integrity/<site>/<YYYY>/<MM>/<DD>/<HH>/...`.
// One NDJSON line per event.
//
// Source of truth for the writer side:
//   ops/zenoh-trial-peers/audit-subscriber/src/records.rs `AuditEvent`
//
// Keep field order + names + types in sync with that file. If the
// writer grows a field, this struct stays backward-compatible by
// virtue of serde's default ignore-unknown behavior; new fields land
// as Option<T> here when consumers need them.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEvent {
    #[serde(default)]
    pub schema_version: u8,
    #[serde(default)]
    pub sample_kind: String,
    #[serde(default)]
    pub receiving_site: String,
    #[serde(default)]
    pub outcome: String,
    #[serde(default)]
    pub namespace_attribution: String,
    #[serde(default)]
    pub policy_evidence: String,
    #[serde(default)]
    pub namespace_slot: Option<String>,
    #[serde(default)]
    pub policy_digest: Option<String>,

    #[serde(default)]
    pub event_id: String,
    #[serde(default)]
    pub anchor: String,
    #[serde(default)]
    pub identity_model: String,
    #[serde(default)]
    pub source_provenance: String,
    #[serde(default)]
    pub payload_capture: String,
    #[serde(default)]
    pub onboarding_classification: String,
    /// Vendor prefix (first segment of the key-expression).
    pub vendor: String,
    /// Full Zenoh key-expression the sample was published on.
    pub key_expr: String,
    /// ISO-8601 UTC, millisecond precision (e.g. "2026-05-26T18:09:46.123Z").
    pub ts: String,
    /// Publisher-asserted Zenoh label; never authenticated identity. Empty when unavailable.
    #[serde(default)]
    pub src_zid: String,
    /// `sha256:<hex>` over the full payload bytes.
    pub payload_hash: String,
    /// Length of the original payload in bytes.
    pub payload_len: usize,
    /// Body from legacy audit or separately verified V2 replay storage.
    /// V2 metadata records never contain this field. Capture is bounded
    /// by the subscriber's configured payload cap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_bytes_b64: Option<String>,
    /// Identifier of the audit-subscriber instance that wrote the row.
    #[serde(default)]
    pub audit_subscriber_id: String,
}

impl AuditEvent {
    /// Parse `ts` into a Unix-millis epoch. Returns `None` if the
    /// timestamp doesn't parse (the caller can choose to skip / count
    /// the row as malformed).
    pub fn ts_unix_ms(&self) -> Option<i64> {
        chrono::DateTime::parse_from_rfc3339(&self.ts)
            .ok()
            .map(|dt| dt.timestamp_millis())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_event() {
        let line = r#"{
          "vendor":"drs",
          "key_expr":"org-1/radar1radar/RADAR-EXAMPLE/tracks",
          "ts":"2026-05-26T18:09:46.123Z",
          "src_zid":"abc123",
          "payload_hash":"sha256:deadbeef",
          "payload_len":412,
          "audit_subscriber_id":"nova-a"
        }"#;
        let e: AuditEvent = serde_json::from_str(line).unwrap();
        assert_eq!(e.vendor, "drs");
        assert_eq!(e.key_expr, "org-1/radar1radar/RADAR-EXAMPLE/tracks");
        assert_eq!(e.payload_len, 412);
        assert!(e.ts_unix_ms().is_some());
    }

    #[test]
    fn parses_event_without_optional_payload() {
        let line = r#"{
          "vendor":"notional",
          "key_expr":"notional/foo/bar/v1",
          "ts":"2026-05-26T18:09:46.000Z",
          "payload_hash":"sha256:01",
          "payload_len":42
        }"#;
        let e: AuditEvent = serde_json::from_str(line).unwrap();
        assert_eq!(e.vendor, "notional");
        assert!(e.payload_bytes_b64.is_none());
        assert!(e.src_zid.is_empty());
    }
}
