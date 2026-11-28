# ADR 0822 — Wide-event observability via OpenTelemetry + OpenObserve

| Field | Value |
|---|---|
| Status | Proposed; amended 2026-11-26 — ntp-admit-sidecar removed from the first wave (retired) |
| Date | 2026-04-26 |
| Repo scope | goat core |
| Supersedes | — |
| Superseded by | — |
| Related | [`0815-observability-stack.md`](0815-observability-stack.md), [`0811-audit-log-integrity.md`](0811-audit-log-integrity.md), [`0507-event-telemetry-schema.md`](0507-event-telemetry-schema.md), [`0506-fabric-schema-evolution.md`](0506-fabric-schema-evolution.md), [`0508-audit-logger-interceptor.md`](0508-audit-logger-interceptor.md), `../findings/F-009-garage-no-object-lock-no-versioning.md` |

## TL;DR (for a layperson)

When something breaks at 3am across multiple parts of the system —
mesh, signal federation, database replication, fabric, audit — the
operator needs one place to ask "show me everything tagged with this
incident's ID." Today's metrics and audit logs sit in separate stores
that don't join. We add a third observability layer alongside metrics
(Prometheus) and audit (Block 17D): **wide events** — structured,
context-rich JSON records emitted via OpenTelemetry from every
service, stored per-site in OpenObserve on top of the existing MinIO
object storage. This doesn't replace metrics or audit; it sits next
to them and is the place you go when you don't yet know which metric
or which audit field to look at.

## Context

The current observability stack ([`0815`](0815-observability-stack.md))
is correct for what it covers:

- **Prometheus + Alertmanager** — metrics (low cardinality)
- **isitup + gatewatch** — operator + sponsor visibility
- **Block 17D MinIO + integrity ledger** — tamper-evident audit

What it doesn't cover is the cross-plane debugging case that becomes
common once a multi-site, multi-component system goes to production:
*"a peer enrollment failed at site X — show me every event from every
plane (mesh, signal, fabric, audit, identity, NTP, policy engine)
tagged with this enrollment's correlation ID."* Prometheus is wrong
for this — high cardinality (per-correlation-ID labels) is precisely
what kills it. Audit storage is wrong for this — Object Lock + Merkle
ledger is purpose-built for tamper-evidence, not ad-hoc query.

The gap has become visible in this project's existing artefacts:

- Soak-test evidence already lives in `docs/reports/*-evidence.jsonl`
  files — these *are* wide events, just unindexed and unjoined.
- The fabric envelope (`correlation_id`, `site_id`, `source_id`,
  `seq`, `ts_ntp_ns`) defined in
  `../design/event-telemetry-schema.md`
  is already structured to support cross-service correlation; nothing
  today consumes that field for query.
- Incident post-mortems on the validation and pilot environments have repeatedly
  required hand-stitching of Prometheus snapshots, audit logs, and
  service logs across multiple files — work that a wide-event store
  would do in one query.

The shape we want is the "Observability 2.0" pattern: structured,
high-cardinality records emitted via OpenTelemetry's OTLP wire
protocol, stored in a backend that supports ad-hoc slicing rather
than fixed-cardinality aggregation.

OpenTelemetry covers only the instrumentation + transport. A backend
is required.

## Decision

Adopt **OpenTelemetry (OTLP) as the wire protocol** and
**OpenObserve as the per-site backend** for wide events.

**Storage layer:** OpenObserve is configured against the per-site
**MinIO** stood up by [`0811`](0811-audit-log-integrity.md). A new
bucket `telemetry-otlp` (no Object Lock, TTL-based retention) sits
alongside the existing `audit-integrity` bucket. **Reuses the
substrate; does not introduce a new object store.** See
`F-009`
for the Garage evaluation that anchored this choice.

**Three coexisting layers:**

| Layer | Backend | Purpose | Cardinality |
|---|---|---|---|
| Metrics | Prometheus (per [`0815`](0815-observability-stack.md)) | Real-time + alerting | Low |
| Wide events | **OpenObserve on MinIO** *(this ADR)* | Ad-hoc cross-plane query, post-mortem reconstruction | High |
| Audit (tamper-evident) | MinIO + Object Lock + Merkle ledger ([`0811`](0811-audit-log-integrity.md)) | Immutable evidence | N/A — append-only |

**Producers.** Existing Go and Rust services emit OTLP via the
OpenTelemetry SDK. First wave (covers the cross-plane debug case
end-to-end):

- `signal-server` (correlation ID source per [`0302`](0302-custom-federated-signal-server.md))
- `gatewatch` ([`0813`](0813-service-registry-pattern.md))
- `db-policy-engine` ([`0805`](0805-policy-engine-active-passive.md))
- `ntp-health-agent` ([`0804`](0804-three-tier-ntp.md))
- `admit-audit` ([`0816`](0816-admit-audit-sidecar.md))
- Zenoh audit-logger interceptor ([`0508`](0508-audit-logger-interceptor.md))
- `mgmt-host-bootstrap` ([`0410`](0410-go-cli-peer-enrollment.md))

**Schema.** The fabric envelope from
`../design/event-telemetry-schema.md`
maps directly onto OTLP attribute conventions:

| Envelope field | OTLP semantic-conventions attribute |
|---|---|
| `correlation_id` | `trace.id` (or `dbb.correlation_id` if the trace context isn't propagated) |
| `site_id` | `service.namespace` |
| `source_id` | `service.instance.id` |
| `ts_ntp_ns` | timestamp of the OTLP record |
| `seq` | `dbb.seq` |
| `signature` | `dbb.signature` |

Schema evolution follows
`../design/fabric-schema-evolution.md`
— additive changes don't bump version; non-additive bump
`schema_version`.

**Topology.** Per-site OpenObserve instance, active-passive behind
the same keepalived VIP pattern as the rest of the mgmt plane
([`1010`](1010-ory-on-spock-mindful-operation.md) D1). No cross-site
federation in v1 — each site owns its own wide-event store. Cross-
site correlation is operator-driven via shared correlation IDs
(operator runs the same query at two sites).

**Sampling.** Tail-based sampling at the OpenTelemetry Collector:
keep 100% of traces tagged `error`, `slow`, or carrying a security-
relevant span; sample non-error traces at a service-specific rate
(default 10%, raised to 100% during active soaks / incidents via
config, no code change).

**Retention tiers** (TTL on the `telemetry-otlp` bucket):

| Tier | Retention | Note |
|---|---|---|
| Hot | 7 days | Full-fidelity, fast query |
| Warm | 90 days | Aggregated to Parquet, S3-tier in MinIO |
| Cold | (none) | Wide events are not evidence — that's audit's job. Drop after warm. |

**OTLP-via-Zenoh path (Phase 3+).** Where producers already publish
on the fabric, the Zenoh audit-logger interceptor
([`0508`](0508-audit-logger-interceptor.md)) gains an OTLP-export
side-channel. Avoids running two separate emit paths in services
that already speak Zenoh. This is **not** the v1 path — v1 is
direct OTLP from each service.

## Alternatives considered

- **Status quo (Prometheus + audit + JSONL files).** Works for
  metrics + evidence but doesn't answer the cross-plane debug case.
  The pain has been measured in incident post-mortem effort.
  Rejected.
- **Grafana Tempo + Loki + Mimir.** Three separate stores
  (traces / logs / metrics), more operational overhead, and Tempo's
  query model is weaker on ad-hoc cardinality slicing than a
  columnar OLAP. Rejected on operational footprint.
- **ClickHouse + Grafana + a separate OTLP collector.** Mature
  combo with the largest community. ClickHouse is local-disk-first;
  to ride the existing MinIO substrate it would need separate S3
  tiering, and we'd need to operate Grafana additionally.
  **Named as the explicit fallback** — if OpenObserve hits a wall
  during the spike, swap to ClickHouse + Grafana with the same
  emitter/schema design. Reversibility cost is bounded.
- **Garage as an alternative MinIO.** Considered; rejected.
  Documented in `F-009`.
- **Self-host Honeycomb or similar SaaS.** Not actually self-host;
  loses sovereignty + Island Mode. Rejected on principle
  ([`0106`](0106-sovereign-tooling-preference.md)).
- **Push everything through Zenoh, no separate observability path.**
  Conflates fabric semantics with telemetry semantics; tightly
  couples observability to Phase 3 fabric-promotion timing. Phase
  3+ enhancement, not v1.

## Consequences

**Positive.**

- One place to ask "what happened across every plane during this
  incident."
- Reuses existing MinIO substrate ([`0811`](0811-audit-log-integrity.md)).
  No new object-store binary, no new sovereignty conversation.
- Prometheus and Block 17D audit storage are unchanged. Layered, not
  replaced.
- Schema discipline already in place
  (`event-telemetry-schema.md`,
  `fabric-schema-evolution.md`)
  applies directly to OTLP attribute conventions.
- Reversible — the alternative (ClickHouse + Grafana) is named and
  feasible if the OpenObserve spike fails.

**Negative.**

- One additional stateful service per site.
- OpenObserve has a smaller community than Prometheus, ClickHouse,
  or Grafana — fewer Stack Overflow answers for edge cases.
- Tail-based sampling at the Collector adds a moving piece that
  must be tuned per service; sampling errors lose data silently.

**Residual risk.** OpenObserve × MinIO interop hasn't been
exercised against the production-shape MinIO config (Object Lock on
the audit bucket; non-Object-Lock on telemetry bucket) at this
project's scale. Spike (Block 27A) closes this before the rest of
the block commits.

## Assessor view (SCA-R)

This ADR adds capability to existing controls; it does not establish
new control coverage:

- **AU-12 (audit generation)** — wide-event store is *not* an
  audit-record source. The authoritative audit path remains Block
  17D ([`0811`](0811-audit-log-integrity.md)) with Object Lock.
  Wide events are operational telemetry; this distinction is
  enforced by separate buckets (`audit-integrity` Object-Locked;
  `telemetry-otlp` not).
- **SI-4 (system monitoring)** — extends Prometheus-based
  monitoring with high-cardinality query capability for incident
  reconstruction.
- **IR-4 (incident handling)** — supports detection-to-response
  workflow by reducing incident-reconstruction time.

The `telemetry-otlp` bucket is explicitly **not** evidence-grade and
is documented as such in the design doc and runbook. Operators must
not use wide-event records for non-repudiation claims.

## Trigger conditions for revisit

- OpenObserve hits a wall in the Block 27A spike. Switch to
  ClickHouse + Grafana per the named fallback.
- Wide-event volume per site exceeds the budgeted 200 GB/year warm
  tier. Tighten sampling, drop verbose attributes, or shorten warm
  retention. Do not lose the cross-plane query property.
- A specific authorization mandates a federated wide-event store
  (cross-site query). Add a cross-site export path; do not abandon
  the per-site sovereign tier.
- OpenTelemetry's OTLP wire format changes incompatibly. Pin
  collector version per [`0703`](0703-pinned-digests-and-pin-regression-ci.md).

---

*See `GLOSSARY.md` for terms and acronyms.*
