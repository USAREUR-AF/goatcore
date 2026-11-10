//! Daemon configuration — the operator's declaration of what E1 distributes keys
//! for, loaded from a JSON file at startup.
//!
//! At launch (Option B, integration-first) the audience is **operator-declared**:
//! `release_sets` maps each release-set to its member ids, and each member's key
//! comes from the F-267 cert directory. This is the same "operator-provisioned"
//! posture as the cert dir — a deliberate, reviewable launch source that the
//! grant-row enumeration (querying `/api/access/grants`) replaces later without
//! touching the daemon loop.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use grant_crypto::service::ServiceError;

fn default_poll() -> u64 {
    30
}

/// The daemon's configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// The key-service signer id, stamped into every bundle's `Signer` field and
    /// verified by members.
    pub signer_id: String,
    /// Path to the key-service's P-256 signing key, PKCS#8 PEM.
    pub signing_key_pem: PathBuf,
    /// Path to the key-service's own SVID (PEM). When set, each published bundle
    /// carries it so members verify the signer self-describingly (validate against
    /// the SPIRE root); typically the cert half of the SPIRE-issued keypair whose
    /// private half is `signing_key_pem`. Omit to publish bundles without the SVID
    /// (members then obtain the verifying key out of band).
    #[serde(default)]
    pub signer_svid_pem: Option<PathBuf>,
    /// Directory of per-member recipient PEMs. With `trust_bundle` + `trust_domain`
    /// set, these are treated as **SVIDs and validated** against the SPIRE root
    /// (the durable path); without them, keys are extracted unvalidated (the
    /// break-glass/offline stopgap — a startup warning is logged).
    pub cert_dir: PathBuf,
    /// SPIRE trust-root bundle PEM (e.g. `/opt/spire/data/server/spire-ca.pem`).
    /// Set together with `trust_domain` to enable SVID validation.
    #[serde(default)]
    pub trust_bundle: Option<PathBuf>,
    /// SPIRE trust domain (e.g. `site.example.net`). Set together with
    /// `trust_bundle`.
    #[serde(default)]
    pub trust_domain: Option<String>,
    /// Trust domains, other than our own, whose SVIDs may hold an audience slot
    /// (ADR 1169 decision 2). A federated partner's identity lives in THEIR
    /// domain, so without naming it here a member id of the form
    /// `spiffe://<their domain>/<path>` is refused.
    ///
    /// EXPLICIT, and deliberately not inferred from `trust_bundle`. The bundle
    /// carries a counterparty root because two SPIRE servers federate, which is
    /// a statement about link authentication; it says nothing about who may read
    /// sealed data. ADR 1169 keeps audience membership entirely ours to decide,
    /// so admitting a domain is a reviewed act. Empty means nobody but us.
    #[serde(default)]
    pub federated_trust_domains: Vec<String>,
    /// The goatnet's **portal offline-CA root** PEM (the cert-sign root, e.g.
    /// `root-cert.pem`). Set together with `portal_uri_prefix` to enable the
    /// validated **portal-CA** key source: portal-minted members (orgs, partners,
    /// human clients, non-SPIRE machines) resolve durably — chain to this root,
    /// CN and identity URI-SAN must equal the member id (WS2; the F-267 durable
    /// half for the non-SPIRE population). Distinct from `portal_url`, which is
    /// the HTTP audience endpoint.
    #[serde(default)]
    pub portal_ca_bundle: Option<PathBuf>,
    /// The per-goatnet identity URI-SAN prefix portal leaves carry (e.g.
    /// `urn:goat:<site>:org=`). Set together with `portal_ca_bundle`.
    #[serde(default)]
    pub portal_uri_prefix: Option<String>,
    /// Release-set → its member ids (the operator-declared audience). Each id must
    /// resolve to a key under `cert_dir`.
    pub release_sets: BTreeMap<String, Vec<String>>,
    /// Seconds between rotation sweeps.
    #[serde(default = "default_poll")]
    pub poll_interval_secs: u64,
    /// Zenoh connect endpoints (e.g. `["tls/host:7447"]`). Empty = the default
    /// Zenoh config (peer scouting).
    #[serde(default)]
    pub zenoh_connect: Vec<String>,
    /// Client mTLS material for a `tls/` router endpoint (a router with
    /// `enable_mtls: true`). Set all three together: the router's
    /// CA (to verify the router's server cert), and this daemon's own certificate +
    /// private key (its SVID, presented to the router — its CN admits it through the
    /// router's ACL). Omit all three for a plaintext `tcp/` endpoint.
    #[serde(default)]
    pub zenoh_tls_root_ca: Option<PathBuf>,
    #[serde(default)]
    pub zenoh_tls_cert: Option<PathBuf>,
    #[serde(default)]
    pub zenoh_tls_key: Option<PathBuf>,
    /// Portal base URL (e.g. `https://portal.example.net`). When set, the daemon derives
    /// each release-set's audience from the portal's grant-row endpoint
    /// (`GET /api/access/audience`) instead of the operator-declared `release_sets`
    /// member lists — so a data owner's `goat access grant` drives the audience
    /// (Piece B). The `release_sets` KEYS still name which release-sets to sweep;
    /// their member-list VALUES are ignored in this mode (put the producer's own
    /// anchor as a placeholder to satisfy validation). Requires `portal_token`.
    #[serde(default)]
    pub portal_url: Option<String>,
    /// Bearer token for the portal (an operator API key or the console↔portal
    /// service token). Required when `portal_url` is set.
    #[serde(default)]
    pub portal_token: Option<String>,
    /// `ip:port` the metrics/health listener binds (`GET /metrics`, `GET /healthz`
    /// — see [`crate::metrics`]). Defaults to `0.0.0.0:9464`: the daemon runs in a
    /// container whose port is not published, so a loopback bind would be
    /// unscrapable from the observability stack, and the surface carries no key
    /// material. Set `127.0.0.1:9464` when running the daemon on a host directly.
    /// Empty string disables the listener — which also removes the only external
    /// evidence the daemon is alive, so prefer changing the address over
    /// disabling. Ignored under `--once` (that container exits).
    #[serde(default = "default_metrics_listen")]
    pub metrics_listen: String,
}

fn default_metrics_listen() -> String {
    "0.0.0.0:9464".to_string()
}

impl Config {
    /// Load and validate a config from a JSON file.
    pub fn load(path: &Path) -> Result<Config, ServiceError> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| ServiceError::Resolve(format!("config {}: read: {e}", path.display())))?;
        let cfg: Config = serde_json::from_str(&raw)
            .map_err(|e| ServiceError::Resolve(format!("config {}: parse: {e}", path.display())))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Reject a config that could not run correctly — fail-closed at startup rather
    /// than mid-rotation.
    pub fn validate(&self) -> Result<(), ServiceError> {
        if self.signer_id.trim().is_empty() {
            return Err(ServiceError::Resolve("config: signer_id is empty".into()));
        }
        // In grant-row mode (portal_url set) the sweep list + audiences are
        // derived from the portal, so operator-declared release_sets are
        // optional — a producer enters scope via `goat access grant` alone (WS4
        // auto-sweep-list). Any keys that ARE listed act as a pinned floor, and
        // their member VALUES are ignored in this mode, so an empty member list
        // is fine. In operator-declared mode the release_sets ARE the source of
        // truth, so they must be present and non-empty (fail-closed).
        let grant_row_mode = self.portal_url.is_some();
        if !grant_row_mode {
            if self.release_sets.is_empty() {
                return Err(ServiceError::Resolve(
                    "config: no release_sets declared (operator-declared mode)".into(),
                ));
            }
            for (rs, members) in &self.release_sets {
                if members.is_empty() {
                    return Err(ServiceError::Resolve(format!(
                        "config: release-set {rs:?} has no members"
                    )));
                }
            }
        }
        if self.poll_interval_secs == 0 {
            return Err(ServiceError::Resolve(
                "config: poll_interval_secs must be > 0".into(),
            ));
        }
        if self.trust_bundle.is_some() != self.trust_domain.is_some() {
            return Err(ServiceError::Resolve(
                "config: trust_bundle and trust_domain must be set together".into(),
            ));
        }
        if self.portal_ca_bundle.is_some() != self.portal_uri_prefix.is_some() {
            return Err(ServiceError::Resolve(
                "config: portal_ca_bundle and portal_uri_prefix must be set together".into(),
            ));
        }
        let tls_set = [
            &self.zenoh_tls_root_ca,
            &self.zenoh_tls_cert,
            &self.zenoh_tls_key,
        ]
        .iter()
        .filter(|o| o.is_some())
        .count();
        if tls_set != 0 && tls_set != 3 {
            return Err(ServiceError::Resolve(
                "config: zenoh_tls_root_ca, zenoh_tls_cert, zenoh_tls_key must all be set together"
                    .into(),
            ));
        }
        if !self.metrics_listen.trim().is_empty()
            && self.metrics_listen.parse::<SocketAddr>().is_err()
        {
            return Err(ServiceError::Resolve(format!(
                "config: metrics_listen {:?} is not an ip:port (e.g. \"0.0.0.0:9464\"); \
                 empty disables the listener",
                self.metrics_listen
            )));
        }
        if self.portal_url.is_some() && self.portal_token.as_deref().unwrap_or("").trim().is_empty()
        {
            return Err(ServiceError::Resolve(
                "config: portal_url is set but portal_token is empty — the grant-row audience \
                 endpoint requires a bearer token"
                    .into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(json: &str) -> PathBuf {
        // Unique temp path from the content length + a fixed prefix; content differs
        // per test so collisions are avoided without a clock/rng.
        let dir = std::env::temp_dir().join(format!("keyd-cfg-{}", json.len()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("config.json");
        std::fs::write(&p, json).unwrap();
        p
    }

    #[test]
    fn loads_a_minimal_config_with_defaults() {
        let p = write(
            r#"{
                "signer_id": "zagros-key-service",
                "signing_key_pem": "/etc/keyd/signer.pem",
                "cert_dir": "/etc/keyd/members",
                "release_sets": {"coalition-air": ["a", "b"]}
            }"#,
        );
        let cfg = Config::load(&p).expect("load");
        assert_eq!(cfg.signer_id, "zagros-key-service");
        assert_eq!(cfg.poll_interval_secs, 30); // default
        assert_eq!(cfg.release_sets["coalition-air"], vec!["a", "b"]);
        assert!(cfg.zenoh_connect.is_empty());
    }

    #[test]
    fn rejects_empty_release_sets_and_members() {
        let no_sets =
            write(r#"{"signer_id":"s","signing_key_pem":"k","cert_dir":"d","release_sets":{}}"#);
        assert!(Config::load(&no_sets).is_err());

        let empty_members = write(
            r#"{"signer_id":"s2","signing_key_pem":"k","cert_dir":"d","release_sets":{"rs":[]}}"#,
        );
        assert!(Config::load(&empty_members).is_err());
    }

    #[test]
    fn trust_bundle_and_domain_are_both_or_neither() {
        let only_bundle = write(
            r#"{"signer_id":"s","signing_key_pem":"k","cert_dir":"d","release_sets":{"rs":["a"]},"trust_bundle":"/spire-ca.pem"}"#,
        );
        assert!(Config::load(&only_bundle).is_err());

        let both = write(
            r#"{"signer_id":"s","signing_key_pem":"k","cert_dir":"d","release_sets":{"rs":["a"]},"trust_bundle":"/spire-ca.pem","trust_domain":"zagros.zagros.example"}"#,
        );
        let cfg = Config::load(&both).expect("both set is valid");
        assert_eq!(cfg.trust_domain.as_deref(), Some("zagros.zagros.example"));
    }

    #[test]
    fn grant_row_mode_allows_empty_release_sets() {
        // portal_url set → sweep list is derived; empty release_sets is valid.
        let p = write(
            r#"{"signer_id":"s","signing_key_pem":"k","cert_dir":"d","release_sets":{},"portal_url":"https://portal.zagros","portal_token":"t"}"#,
        );
        assert!(
            Config::load(&p).is_ok(),
            "grant-row mode should not require operator-declared release_sets"
        );
        // still rejected in operator-declared mode (no portal_url).
        let q =
            write(r#"{"signer_id":"s","signing_key_pem":"k","cert_dir":"d","release_sets":{}}"#);
        assert!(Config::load(&q).is_err());
    }

    #[test]
    fn portal_ca_bundle_and_uri_prefix_are_both_or_neither() {
        let only_bundle = write(
            r#"{"signer_id":"s","signing_key_pem":"k","cert_dir":"d","release_sets":{"rs":["a"]},"portal_ca_bundle":"/root-cert.pem"}"#,
        );
        assert!(Config::load(&only_bundle).is_err());

        let both = write(
            r#"{"signer_id":"s","signing_key_pem":"k","cert_dir":"d","release_sets":{"rs":["a"]},"portal_ca_bundle":"/root-cert.pem","portal_uri_prefix":"urn:goat:zagros:org="}"#,
        );
        let cfg = Config::load(&both).expect("both set is valid");
        assert_eq!(
            cfg.portal_uri_prefix.as_deref(),
            Some("urn:goat:zagros:org=")
        );
    }

    #[test]
    fn zenoh_tls_is_all_three_or_none() {
        let partial = write(
            r#"{"signer_id":"s","signing_key_pem":"k","cert_dir":"d","release_sets":{"rs":["a"]},"zenoh_tls_cert":"/c.pem"}"#,
        );
        assert!(Config::load(&partial).is_err());

        let all = write(
            r#"{"signer_id":"s","signing_key_pem":"k","cert_dir":"d","release_sets":{"rs":["a"]},"zenoh_tls_root_ca":"/ca.pem","zenoh_tls_cert":"/c.pem","zenoh_tls_key":"/k.pem"}"#,
        );
        assert!(Config::load(&all).is_ok());
    }

    #[test]
    fn metrics_listen_defaults_on_and_must_parse() {
        let p = write(
            r#"{"signer_id":"s4","signing_key_pem":"k","cert_dir":"d","release_sets":{"rs":["a"]}}"#,
        );
        assert_eq!(
            Config::load(&p).expect("load").metrics_listen,
            "0.0.0.0:9464",
            "the metrics surface is on by default — an unobservable keyd is the defect"
        );

        let bad = write(
            r#"{"signer_id":"s5","signing_key_pem":"k","cert_dir":"d","release_sets":{"rs":["a"]},"metrics_listen":"not-an-addr"}"#,
        );
        assert!(Config::load(&bad).is_err());

        let off = write(
            r#"{"signer_id":"s6","signing_key_pem":"k","cert_dir":"d","release_sets":{"rs":["a"]},"metrics_listen":""}"#,
        );
        assert!(Config::load(&off).is_ok(), "empty disables the listener");
    }

    #[test]
    fn rejects_unknown_fields() {
        let p = write(
            r#"{"signer_id":"s3","signing_key_pem":"k","cert_dir":"d","release_sets":{"rs":["a"]},"typo_field":true}"#,
        );
        assert!(
            Config::load(&p).is_err(),
            "deny_unknown_fields should catch typos"
        );
    }
}
