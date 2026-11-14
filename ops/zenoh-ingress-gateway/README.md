# zenoh-ingress-gateway

The **goat-side receiver** for data arriving over a Zenoh **router-to-router peering** from an
entity that is not part of our trust domain (a moon pod in the learning setup, a vendor aggregator relaying
third parties — anything we did not onboard end-to-end). This is the implementation of the
Tier-2 ingress path in [ADR 1020](../../docs/adr/1020-zenoh-federated-peering-trust-model.md) /
[`zenoh-federated-peering-trust.md`](../../docs/design/zenoh-federated-peering-trust.md).

## Why it exists

When data reaches our fabric over a peering rather than from a directly-onboarded publisher,
the onboarding contract (per-vendor cert identity, write-scope ACL, storage, schema, audit
attribution) is bypassed at the session boundary: the data is unauthenticated at the publisher
level, unattributable, and can squat any namespace.

The gateway closes that hole: **no third party ever peers directly into a core router.** They
peer with the gateway; the gateway re-publishes their data into the core fabric under its own
goat identity, origin-scoped, with provenance — so the core router's ACL only ever sees
goat-CA-rooted writers, and the boundary is a real component we control and can rate-limit.

## What it does (v0)

Two Zenoh client sessions, one direction of data (partner → core):

1. **Partner-facing session** connects OUT to the peered entity's router (mTLS) and subscribes
   to `--ingress-key-expr` (default `**`).
2. For each received sample, computes the **admitted key** under the policy (below).
3. **Core-facing session** (the gateway's own goat-CA-rooted cert, scoped by the core ACL to
   write only the admit/quarantine prefix) re-publishes the sample under the admitted key.
4. Records **provenance** for every admitted sample: verified peering identity, original key,
   admitted key, mode, timestamps — to structured tracing and (optionally) append-only NDJSON.

### Admission policy (`--admit-mode`)

- **`quarantine`** (default, safe): every key is rewritten to
  `<peering-entity>/<peering-id>/<original-key>`. Nothing the peer sends can land in a
  first-class namespace. This is the ADR 1020 quarantine shape; for example
  `vendor-p/peering-7f3a/example-vendor/tracks/entity/v1`.
- **`passthrough`** (onboarded moon-pods only): keys matching a `--passthrough-allow` prefix
  (e.g. `release/<partner>/**`) are admitted **verbatim** (origin-scoped, since the pod IS
  onboarded end-to-end); everything else is quarantine-rewritten. This is the open
  reconciliation from the moon-pod design — an onboarded pod's first-party `release/<partner>/**`
  is authorized, unlike relayed third-party traffic. The core ACL remains the hard backstop;
  passthrough allow-listing is defense in depth, not the enforcement boundary.

## Relationship to the moon-pod

A moon-pod is a Tier-2 peer. Its egress is received here, not federated directly into a core
router. The pod always publishes `release/<partner>/**`; this gateway decides the admitted
shape. See [`zagros-moon-pod/docs/architecture.md`](https://github.com/USAREUR-AF/zagros-moon-pod/blob/main/docs/architecture.md)
§"Receiver side".

## Status

v0 — Rust crate, mirrors the `ops/zenoh-trial-peers/audit-subscriber` client/mTLS pattern
(`zenoh = "=1.5.1"`). Unit-tested on the admission-policy key computation (no fabric needed).
Not yet deployed.

### TODO before production

- **Derive `peering_entity` from the live session's peer cert** rather than config (v0 trusts
  the operator to match `--peering-entity` to the authenticated peer; the design wants the
  verified cert identity used directly).
- **Bounded storage for the quarantine prefix** (ADR 1020 retention open decision: 7-day TTL?).
- **Rate limiting** — the gateway is the intended chokepoint against floods.
- **`peering-id` reservation convention** (ADR 1020 open decision) — confirm the reserved-space
  prefix so it cannot collide with an onboarded entity's own `<entity>/**`.
- The **ACL `egress`/`ingress` `put` split** (ADR 1020 build-plan step 1) is a prerequisite on
  the core router; this gateway assumes it.
- Ansible role + compose deployment wiring (`ops/ansible/roles/zenoh-ingress-gateway/`).
- Live cross-prefix `put` confirmation test against Zenoh 1.9.0 semantics.

## Run (dev)

```
cargo run -- \
  --peering-entity org112x --peering-id peering-7f3a --admit-mode quarantine \
  --partner-connect-endpoint tls/partner-router:7447 \
  --partner-tls-root-ca   ./tls/partner-ca.pem \
  --partner-tls-client-cert ./tls/gw-cert.pem \
  --partner-tls-client-key  ./tls/gw-key.pem \
  --core-connect-endpoint tls/core-router:7447 \
  --core-tls-root-ca   ./tls/goat-ca.pem \
  --core-tls-client-cert ./tls/gw-core-cert.pem \
  --core-tls-client-key  ./tls/gw-core-key.pem \
  --provenance-ndjson-path /var/lib/goat/ingress-gateway/provenance.ndjson
```

`cargo test` runs the admission-policy unit tests with no fabric.
