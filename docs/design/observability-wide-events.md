# Wide-event observability — OpenTelemetry + OpenObserve on MinIO

## Conclusion

Wide events — high-cardinality, structured, context-rich JSON records
emitted via the OpenTelemetry OTLP wire protocol — are the third
observability layer in this system, sitting alongside metrics
(Prometheus, [`0815`](../adr/0815-observability-stack.md)) and
tamper-evident audit (Block 17D MinIO + Object Lock + Merkle ledger,
[`0811`](../adr/0811-audit-log-integrity.md)). They exist to answer
the cross-plane debugging question — *"show me everything tagged with
this correlation ID across mesh, signal, fabric, audit, identity, NTP,
policy"* — that neither Prometheus nor audit storage can answer.

The store is **OpenObserve, per-site, on top of the existing MinIO**
([`0811`](../adr/0811-audit-log-integrity.md)) substrate. **No new
object store** — a separate `telemetry-otlp` bucket sits next to the
existing `audit-integrity` bucket, with no Object Lock and TTL-based
retention. ADR: [`0822`](../adr/0822-wide-event-observability-otlp-openobserve.md).

This document covers: producer conventions, OTLP attribute mapping,
sampling, retention tiers, query patterns, and the schema-evolution
discipline that applies (inherited from
`fabric-schema-evolution.md`).

## Why a third layer

The two existing observability layers cover what they were built for
and badly cover what they weren't:

| Question | Right tool today | Why wide events are needed |
|---|---|---|
| "Is the system up? What's the rate?" | Prometheus | — |
| "Did anyone tamper with last week's audit?" | MinIO Object Lock + Merkle ledger | — |
| "An enrollment failed at 03:14 — show me every event from every plane tagged with this enrollment's correlation ID" | **None today.** Hand-stitched from Prometheus snapshots + audit logs + service logs + JSONL evidence files | Wide events: one query, all planes, joined by correlation ID |
| "During Wave 3 §3.1 soak, when did Spock replication latency spike, and what was happening on signal-server at that exact moment?" | Hand-stitched | Wide events: filter on `replication_lag_ms > N`, join on `ts_ntp_ns` window |
| "What's the distribution of enrollment latency by `site_id` × `device_class` × `bundle_kind`?" | Cardinality kills Prometheus | Wide events: SQL group-by on the three high-cardinality dimensions |

The shape we want is the "Observability 2.0" pattern: one structured
record per service-meaningful event, with all the dimensions you might
want to slice on baked in at emit time, queried ad-hoc rather than
pre-aggregated.

## Architecture

### Per-site topology

```
┌─────────────────────────────────────────────────────────────┐
│ Site                                                        │
│                                                             │
│  ┌──────────────────────────────────────────────────────┐   │
│  │ Producers (Go / Rust services)                       │   │
│  │  signal-server, gatewatch, db-policy-engine,         │   │
│  │  ntp-health-agent, admit-audit, mgmt-host-bootstrap, │   │
│  │  Zenoh audit-logger interceptor, ...                 │   │
│  └────────────────┬─────────────────────────────────────┘   │
│                   │ OTLP/HTTP (mesh-internal)                │
│                   ▼                                         │
│  ┌──────────────────────────────────────────────────────┐   │
│  │ OpenTelemetry Collector  (active-passive, keepalived)│   │
│  │   - tail-based sampling                              │   │
│  │   - attribute enrichment (site_id, mesh peer ID)     │   │
│  │   - rate limiting                                    │   │
│  └────────────────┬─────────────────────────────────────┘   │
│                   │ OTLP                                    │
│                   ▼                                         │
│  ┌──────────────────────────────────────────────────────┐   │
│  │ OpenObserve  (active-passive, keepalived)            │   │
│  │   - ingest API                                       │   │
│  │   - query UI at observe.<site>.netbird.goat.. │   │
│  │   - hot tier: local NVMe                             │   │
│  │   - warm tier: MinIO bucket telemetry-otlp           │   │
│  └────────────────┬─────────────────────────────────────┘   │
│                   │ S3                                      │
│                   ▼                                         │
│  ┌──────────────────────────────────────────────────────┐   │
│  │ MinIO  (per 0811 — same instance, different bucket)  │   │
│  │   bucket: audit-integrity   (Object Lock, 1y+6y)     │   │
│  │   bucket: telemetry-otlp    (no Object Lock, TTL)    │   │
│  │   bucket: runtime-detection-archive (per Block 18G)  │   │
│  └──────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

**HA pattern (revised 2026-04-30 by ADR 0826).** OpenTelemetry Collector + OpenObserve + MinIO **run on a dedicated third Orin host (Orin-C, "observability")** per site, not co-resident with the mgmt-plane pair (Orin-A + Orin-B). Producers point at `observe.<site>.netbird.legacy.example` → Orin-C's mesh IP directly (no VIP since single observability host today; Orin-D passive partner deferred behind documented trigger criteria in ADR 0826). Mgmt-plane on Orin-A/B retains its own VRRP keepalived VIP for control traffic. The diagram above (showing all three tiers in one box) reflects the **logical site** — it does not imply physical co-residency on the same host.

**Why split.** Storage growth (video archive, audit retention, telemetry warm tier) plus failure-domain separation. See ADR 0826 §"Why not two-host with co-residency."

**Cross-site.** No federation in v1. Each site owns its store.
Cross-site correlation is operator-driven: the same `correlation_id`
appears at every site that handled the operation; the operator runs
the query at each site separately. This matches Island Mode by
construction — a partitioned site can still query its own wide
events.

### OTLP attribute mapping

The fabric envelope from
`event-telemetry-schema.md` maps to OTLP
semantic conventions where possible, custom `dbb.*` attributes
otherwise:

| Envelope field | OTLP attribute | Type | Notes |
|---|---|---|---|
| `correlation_id` | `trace.id` | string | When OTel context is propagated end-to-end. Otherwise put under `dbb.correlation_id`. |
| `site_id` | `service.namespace` | string | Standard OTel resource attribute. |
| `source_id` | `service.instance.id` | string | Standard OTel resource attribute. |
| `schema_version` | `dbb.schema_version` | string | Per-topic schema version. |
| `ts_ntp_ns` | OTLP record timestamp | int64 | Use the OTLP-native timestamp; do not duplicate. |
| `ts_mono_ns` | `dbb.ts_mono_ns` | int64 | Useful for in-process interval calc. |
| `seq` | `dbb.seq` | int64 | Per-source monotonic. Gap detection by query. |
| `signature` | `dbb.signature` | string (base64) | Per [`0511`](../adr/0511-publisher-trust-model.md). |
| `payload.*` | `dbb.<topic>.<field>` | various | Topic-specific. Flattened, not nested, so OpenObserve indexes them. |

**Why flatten the payload.** OpenObserve indexes top-level fields;
nested objects are stored but not indexed by default. Flattening at
emit time keeps high-cardinality slicing fast.

### Producers — first wave

The first wave covers the cross-plane debugging case end-to-end. Each
service emits OTLP via the OpenTelemetry SDK for its language. **Status
column is the authoritative landed-vs-gated view** — flip to ✅ landed
in the same commit that lands the producer's OTLP emit + SCHEMA.md.
Lane conflicts that gate ❌ blocked rows are tracked in
`../project/block-27c-scoping-2026-05-05.md`
§"Active-track conflict overlay" until released.

| Service | Language | Repo path | OTLP scope | Status | Schema |
|---|---|---|---|---|---|
| `mgmt-host-bootstrap` | Go | `ops/mgmt-host-bootstrap/` | Bootstrap CLI runs, account seed, setup-key seed | ✅ landed 2026-05-05 (`e646eca`) | [`SCHEMA.md`](../../ops/mgmt-host-bootstrap/SCHEMA.md) |
| `signal-server` | Go | `ops/signal-server/` | Federation RPCs, peer connect/disconnect, correlation-ID source | 🟡 pending | — |
| `gatewatch` | Go | `ops/gatewatch/` | Bridge access events, gate-status changes | 🟡 pending | — |
| `db-policy-engine` | Go | (per [`0805`](../adr/0805-policy-engine-active-passive.md)) | Policy decisions, approvals, exception flows | ⚠ premature (service not yet built — Block 17A.5) | — |
| `ntp-health-agent` | Go | `ops/time-hygiene/cmd/ntp-health-agent/` | Time-source health, drift events | ✅ landed 2026-05-07 | [`SCHEMA.md`](../../ops/time-hygiene/cmd/ntp-health-agent/SCHEMA.md) |
| `admit-audit` | Go | `ops/admit-audit/` | Peer-table poll cycle + per-admit detection events | ✅ landed 2026-05-08 | [`SCHEMA.md`](../../ops/admit-audit/SCHEMA.md) |
| Zenoh audit-logger | Rust | `zenoh-plugins/` | Fabric publish/subscribe events | 🟡 pending | — |

**Status legend.** ✅ landed — OTLP emit + per-producer `SCHEMA.md` live
in tree, covered by tests. 🟡 pending — scope claimed, not yet
implemented; check the scoping note for lane gates. ❌ blocked — active
track holds ownership of the source files; do not start until that
track releases. ⚠ premature — service does not yet exist; OTel arrives
with the initial build, not as a retrofit.

**Status-update discipline.** Every producer landing flips its row to
✅ in the same commit that lands the OTLP emit + SCHEMA.md. Stale 🟡
entries get re-checked against `active-work.yaml`
during quarterly doc-drift sweeps; promote ❌ blocked rows to 🟡
pending when the blocking track releases.

Second wave (Phase 3): application services, vendor shims, partner
federation.

### Schema evolution

Inherits the discipline from
`fabric-schema-evolution.md`:

- **Additive changes** (new attribute, new event type): no version
  bump. Subscribers tolerate absence.
- **Non-additive changes** (rename, type change, removal): bump
  `dbb.schema_version` for the topic. Old version stays queryable
  for the warm-tier retention window.
- **Producer-side schema declaration** lives next to the producer
  (each service ships a schema doc at `<service>/SCHEMA.md` listing
  the OTLP attributes it emits).

### Sampling

Tail-based at the OpenTelemetry Collector. Default policy:

| Trace property | Sample rate |
|---|---|
| `error == true` | 100% |
| `dbb.security_relevant == true` | 100% |
| `dbb.duration_ms > 1000` | 100% |
| Known soak window (`dbb.soak_id` set) | 100% |
| Active incident (config flip, no code change) | 100% |
| Otherwise | 10% (service-specific override allowed) |

**Why tail-based.** Head-based sampling decides at emit time; tail-
based decides at the Collector after seeing the full trace. Catches
the slow / errored / interesting traces that head-based would
randomly drop.

**Sampling-loss mitigation.** Sampled-out traces lose data silently —
operators must not use wide events as the source of truth for "did
this happen?" For that, the audit path
([`0811`](../adr/0811-audit-log-integrity.md)) is authoritative.

### Retention tiers

TTL-based, configured on the `telemetry-otlp` bucket and OpenObserve
indices:

| Tier | Retention | Storage shape | Query latency |
|---|---|---|---|
| Hot | 7 days | OpenObserve native format on local NVMe | < 1s |
| Warm | 90 days | Parquet on MinIO `telemetry-otlp` bucket | seconds |
| Cold | (none) | Wide events are not evidence — that's audit's job. Drop after 90 d. | — |

Storage budget per site (back-of-envelope, refine from real Phase 3
data):

- Hot: ~5 GB local NVMe
- Warm: ~50 GB MinIO bucket
- All sites combined: comfortable inside the 1 TB
  `/opt/docker-data` per [`0905`](../adr/0905-nvme-per-site.md).

### Query patterns

OpenObserve speaks SQL. Common queries:

```sql
-- "Show me everything tagged with this correlation ID, every plane"
SELECT * FROM logs
WHERE dbb_correlation_id = 'abc-123'
ORDER BY _timestamp ASC;

-- "Enrollment latency by site x device-class x bundle-kind"
SELECT service_namespace, dbb_device_class, dbb_bundle_kind,
       PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY duration_ms)
FROM traces
WHERE dbb_event_type = 'enrollment.complete'
AND _timestamp BETWEEN '2026-04-26' AND '2026-04-27'
GROUP BY service_namespace, dbb_device_class, dbb_bundle_kind;

-- "What was signal-server doing when Spock replication lagged?"
SELECT * FROM logs
WHERE service_name = 'signal-server'
AND _timestamp BETWEEN
  (SELECT _timestamp FROM logs
   WHERE dbb_metric = 'spock_replication_lag_ms' AND value > 5000
   ORDER BY _timestamp DESC LIMIT 1) - INTERVAL '1 minute'
AND _timestamp + INTERVAL '1 minute';
```

OpenObserve also exposes a Lucene-style search for ad-hoc operator
use without writing SQL.

### Mesh-internal exposure

OpenObserve UI lives at
`observe.<site>.netbird.legacy.example`, mesh-internal only,
behind Traefik with OIDC ForwardAuth via Ory Kratos
([`0411`](../adr/0411-ory-idp-mvp.md)). No public DNS, no public
internet exposure (consistent with the Block 22 silent-control-
plane invariant).

OpenObserve's ingest endpoint is on the mesh-internal interface
only. Producers within the same site push directly; cross-site
producers are explicitly out of scope for v1.

## What this layer is not

- **Not an audit replacement.** Audit storage stays in
  `audit-integrity` with Object Lock. Wide events are best-effort
  operational telemetry.
- **Not a metrics replacement.** Prometheus stays. Alert rules stay.
- **Not a fabric replacement.** Zenoh is the data fabric for
  application data; OTLP is the wire protocol for observability
  data. The audit-logger interceptor
  ([`0508`](../adr/0508-audit-logger-interceptor.md)) bridges the
  two for events that already ride the fabric.
- **Not cross-site federated in v1.** Each site is independent.

## Implementation tracking

`../project/implementation-plan.md`
Block 27.

Phases:

- **27A.** OpenObserve × MinIO interop spike (one-node, OTLP
  ingestion suite).
- **27B.** OpenTelemetry Collector deployment (Ansible role,
  active-passive).
- **27C.** First-wave producer instrumentation (signal-server →
  gatewatch → policy engine → time-hygiene → admit-audit →
  mgmt-host-bootstrap → Zenoh audit-logger).
- **27D.** Per-site OpenObserve deployment + traefik route + OIDC
  forward-auth.
- **27E.** Sampling tuning + retention rollover validation.

## Related artefacts

- [`0822-wide-event-observability-otlp-openobserve.md`](../adr/0822-wide-event-observability-otlp-openobserve.md)
  — the decision
- [`0815-observability-stack.md`](../adr/0815-observability-stack.md)
  — Prometheus / Alertmanager / isitup / gatewatch
- [`0811-audit-log-integrity.md`](../adr/0811-audit-log-integrity.md)
  — Block 17D MinIO + Object Lock (storage substrate this builds on)
- `event-telemetry-schema.md` — fabric
  envelope conventions reused for OTLP attributes
- `fabric-schema-evolution.md` —
  schema-evolution discipline
- [`0508-audit-logger-interceptor.md`](../adr/0508-audit-logger-interceptor.md)
  — Zenoh interceptor for fabric-native producers
- `F-009-garage-no-object-lock-no-versioning.md`
  — finding that anchored the MinIO-not-Garage choice

---

*See `../adr/GLOSSARY.md` for terms and acronyms.*
