# ADR 1232 — A fault on a peer we do not own follows one five-stage path: measured at our router, attributed, told, tracked, closed

| Field | Value |
|---|---|
| Status | Accepted — decided by the operator on 2026-11-13. Nothing in it is built |
| Date | 2026-11-13 |
| Repo scope | Multi (`goatcore`; each goatnet's configuration repository declares where the reader runs) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 1036](1036-environment-health-surface.md) (the health contract), [ADR 1037](1037-fabric-internal-time-synchronization-first.md) (whose clock is right), [ADR 1068](1068-anomaly-probation-lifecycle.md) (the probation list), [ADR 1168](1168-self-hosted-paging-and-automatic-oncall-enrolment.md) (the pager), design [`peer-fault-handling.md`](../design/peer-fault-handling.md) |

## TL;DR (for a layperson)

A machine that belongs to another organization can connect to a goatnet in a faulty state: its clock is
wrong, it trusts an old certificate authority, or it reconnects in a loop. The goatnet keeps working, so
nobody notices, and the owner is never told. This decision gives every such fault the same path. The
router that sees the fault measures it. The fault is tied to an owner. The goatnet's admins, the owner
and the faulty machine are each told. The fault is tracked with a deadline. It closes by itself when the
measurement reads clean.

## Context

Zenoh is brokerless: a router tolerates a faulty peer where it can and reports nothing to the peer
([`zenoh-brokerless-consequences.md`](../design/zenoh-brokerless-consequences.md), consequence 4). Two
cases show the cost.

- **A peer whose clock runs ahead.** A router refuses a timestamp more than 500 ms ahead of its own clock.
  With `drop_future_timestamp: false` it keeps the sample and writes its own time in its place. Stored
  data from that peer is then ordered by arrival, a reader cannot tell the time was replaced, and the only
  record is two log lines per sample on the router.
- **A peer that trusts a root the router no longer presents.** The peer refuses the router's certificate
  and holds no session. Its container and its web front door stay healthy. A router at `RUST_LOG=info`
  logs nothing for a handshake the client ends.

The time designs define what a bad clock is and where it is enforced. The health surface defines how a
reading is published and rendered. The probation design defines how a held item is tracked to a deadline.
None of them says how a fault on a machine we do not own is found, whose it is, or how its owner learns
of it.

## Decision

1. **One path for every fault kind.** Measure, attribute, tell, track, close, as specified in
   [`peer-fault-handling.md`](../design/peer-fault-handling.md). A fault kind supplies a detector and a
   notice text. The other four stages are shared.
2. **The measurement is taken at the router we own**, by a reader that runs beside each router that
   terminates a connection from outside: the per-partner edge routers of a federation first, the core
   router second. The reader stores nothing and enforces nothing.
3. **Readings use the existing health contract.** A new domain `peer` on `ops/peer/health/<site>` as
   `EnvHealthV1`. The clock reading is published in the existing `time` domain, marked as observed by a
   neighbor and not self-reported.
4. **The first notice to an owner for each fault is approved by an operator.** Reminders and the closing
   notice are sent automatically. They repeat a message a person already approved.
5. **A peer is never disconnected automatically for a fault.** Escalation at the deadline flags and pages.
   Disconnecting a peer is an operator decision.
6. **A federated partner gets one released key carrying our verdict on their connection**, under that
   partner's release set. Health keys do not cross a boundary node; a released key is the only way the
   partner's own tooling can show the fault.
7. **`drop_future_timestamp` stays `false`.** A router re-stamps a sample from a fast clock and keeps it.
8. **The tracked entry is a probation entry of kind `peer`**, the kind ADR 1068 reserves.

## Alternatives considered

- **Drop samples with a future timestamp.** Rejected: it discards a partner's data for a fault the partner
  cannot see.
- **Disconnect a peer when its deadline passes.** Rejected: removing a real thing on a timer is the failure
  the probation design exists to prevent.
- **Send every owner notice automatically.** Rejected for the first notice: it is a message to a person at
  another organization about their machine, and a wrong attribution should be caught by a person first.
- **Require an agent on the peer.** Rejected: a machine we do not own runs what its owner chooses. The
  router already holds the evidence.
- **A separate list for peer faults.** Rejected: the probation list, its command and its portal page exist.

## Consequences

- A fault on an outside machine becomes a row on `goat health`, panoscope and stethoscope with the
  machine or the partner connection named, and a page when it stays red.
- An owner can read the verdict with their own `goat health`, without anyone on our side sending it.
- Each new fault kind costs a detector and a notice template.
- The reader is a new component beside each router. If it stops, the fabric behaves as before and the
  `peer` domain reads unmeasured, never green.
- The ID in a timestamp rejection is the router that stamped the sample, which across a federation can be
  a router we hold no session with (measured 2026-11-13, design doc). The connection is therefore named by
  which of our routers logged the rejection: one reader per per-partner edge router.

## Assessor view (SCA-R)

The decision adds detection and notification. It grants no access and removes none. The reader holds a
subscribe-and-publish identity limited to the `ops/peer/` and `ops/time/` health keys and the router's
admin space. The released verdict key carries one partner's own connection state to that partner only.

## Trigger conditions for revisit

- Several partners come to share one edge router, so the router that logged a rejection no longer names
  the peering.
- A fault kind appears that the fabric cannot contain while the owner is being reached, so that a timed
  disconnect is the lesser harm.
