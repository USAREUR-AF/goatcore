# ADR 0505 — Stateful-consumer pattern on the fabric

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-04-25 |
| Repo scope | goat core |
| Related | [`0500-zenoh-data-fabric.md`](0500-zenoh-data-fabric.md), [`../design/fabric-stateful-consumers.md`](../design/fabric-stateful-consumers.md) |

## TL;DR (for a layperson)

Many fabric consumers need to remember things across messages
— a leaderboard, a status aggregator, a deduplicator. We
codify a single pattern for "stateful consumer": store the
state in the fabric itself (in queryable storage), keep the
consumer logic stateless, restart-safe, and replayable. This
makes consumers cheap to operate, simple to reason about, and
easy to migrate.

## Context

Stateful consumers (services that hold per-key, per-window, or
per-flow state) are easy to write badly: state in process
memory, state in a separate database, state in a sidecar.
Each variant adds operational complexity and complicates
restart, scaling, and auditability.

The pattern: state is in the fabric; the consumer is
restart-safe.

## Decision

Stateful consumers follow this pattern:

- **State lives in fabric storage** ([`0502`](0502-rocksdb-ops-storage.md))
  under a stable key prefix. State is queryable by other
  components.
- **Consumer is restart-safe.** Restart re-queries state from
  the fabric and resumes.
- **Consumer is replayable.** Re-running over the same
  message stream produces the same state (idempotent
  processing).
- **Consumer is migration-safe.** Moving the consumer to a
  different host requires no state migration; it
  re-attaches to fabric state.
- **Schema evolution** ([`0506`](0506-fabric-schema-evolution.md))
  governs state-shape changes.

Reference: airdefenders Phase 2D-interim leaderboard is the
first caller; the design doc
[`../design/fabric-stateful-consumers.md`](../design/fabric-stateful-consumers.md)
codifies the pattern.

## Alternatives considered

- **State in consumer process memory.** Lost on restart;
  unmigrate-able. Rejected.
- **State in a separate database.** Adds operational
  dependency for every stateful consumer. Rejected for the
  default; tolerable case-by-case for specific reasons.
- **Sidecar pattern.** Adds another component per consumer.
  Rejected.

## Consequences

**Positive.** Restart-safe and migration-safe by construction.
State is queryable / observable. Consumers stay small.

**Negative.** State in fabric storage means schema-evolution
discipline is mandatory; we get [`0506`](0506-fabric-schema-evolution.md)
in exchange.

## Assessor view (SCA-R)

The pattern serves CP-10 (system recovery — restart-safe
recovery), SI-13 (predictable failure prevention — replayable
processing), and CM-2 (baseline configuration — state shape
is part of the baseline). The pattern aligns with NIST SP
800-160 vol. 1 design principle of "keep state observable."

## Trigger conditions for revisit

- A consumer class genuinely cannot be made restart-safe
  (rare; usually means the design is wrong). Document the
  exception explicitly.
- Fabric storage cannot meet a consumer's latency / volume
  requirement. Reach for a separate database with explicit
  ADR.

---

*See `GLOSSARY.md` for terms and acronyms.*
