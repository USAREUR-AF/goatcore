# ADR 1268 — Sensor events on the fabric use the published ZMeta event model, implemented in our own compiled engines

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-11-26 |
| Repo scope | goat core (this decision, the fabric key layout, the upstream manifest), goat-processors (ingress and egress packs, the validator in each engine), goatstuff (surfaces that read tracks) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 0844](0844-zenoh-deployment-topology-and-acl-v1.md), [ADR 1033](1033-zenoh-qos-prioritization-reserved-mission-slice.md), [ADR 1035](1035-operator-namespace-ops-canonical.md), [ADR 1037](1037-fabric-internal-time-synchronization-first.md), [ADR 1208](1208-sovereign-environments-release-namespaces-and-grant-encryption.md), [ADR 1262](1262-one-upgrade-process-for-load-bearing-upstream-dependencies.md), [`zenoh-brokerless-consequences.md`](../design/zenoh-brokerless-consequences.md), [`zenoh-data-fabric.md`](../design/zenoh-data-fabric.md) |

## TL;DR (for a layperson)

Sensors, models and trackers all describe what they see in their own formats, so every pair
that must talk needs its own translator. ZMeta is an open, published standard that gives all
of them one format and a short list of rules: a raw measurement may not claim to know what it
saw, a conclusion must name the measurements it rests on, and a message that breaks a rule is
refused out loud instead of being quietly fixed. goat adopts that format for sensor data on
its fabric. The checking is done by goat's own compiled software, not by the project's
demonstration code.

## Context

goat's sensor pipeline runs from raw feeds through goat-processors (filter, correlate, fuse,
quality) to clean tracks and the operator's picture. Its records today are goat's own: a
track report shaped after Cursor on Target, keyed by `track.source_id` and `uid`, with `ce`
and `stale_ms`. A partner integrating a sensor has to learn that shape, and the shape cannot
say what a report rests on, how sure its producer is, or how good its clock was.

ZMeta (github.com/JTC-byte/zmeta-spec, Apache-2.0, public since January 2026) defines six
event classes: observation (what a sensor measured), inference (what a model concluded),
fusion (which reports are one object), state (what the operator sees), command (bounded
tasking) and system (health, timing, receipts). It makes four rules machine-checkable:

- every event above an observation lists its parents (`lineage.based_on`);
- an observation may not carry an identity, and an inference may not assign a track id;
- a refused event is replaced by a diagnostic that names it and the reason, never repaired;
- every event may carry its clock source, sync state and estimated error (`timing_quality`).

It ships projections to Cursor on Target, SAPIENT, KLV, a JREAP-style form and MAVLink
mission intent, and three link profiles (L, M, H) that thin traffic without changing meaning.
Version 1.0 of the schema is locked; the 1.1 additions are marked experimental. The
reference implementation is a single Python process that validates JSON received over UDP
and forwards it, with unpinned dependencies installed when its container starts.

The fabric is brokerless ([`zenoh-brokerless-consequences.md`](../design/zenoh-brokerless-consequences.md)):
a denied or malformed message disappears without a trace at either end. A format whose
refusals are themselves events is a direct answer to that.

## Decision

1. **Wire format.** Sensor events on the fabric carry ZMeta v1.0 events, JSON-encoded, as the
   Zenoh payload. The key layout stays goat's ([ADR 1035](1035-operator-namespace-ops-canonical.md),
   [ADR 0844](0844-zenoh-deployment-topology-and-acl-v1.md)); ZMeta governs what is inside a
   message, never where it is published or who may read it.
2. **Our engines do the checking.** Every goat-processors engine gains a ZMeta ingress pack and
   egress pack, starting with the Rust engine. Validation (schema, closed lists, the layer
   rules, lineage) is compiled code in those engines, proven against ZMeta's published
   must-pass and must-fail vectors at the pinned tag.
3. **The layer rules are adopted whole.** Lineage is mandatory above an observation, an
   observation names no identity, an inference assigns no track id, and a correction is a new
   event with lineage, never an edit.
4. **Refusals are published.** A refused event produces a ZMeta system event naming its id and
   reason code, on a diagnostics key beside the stream it was refused from, so a silent drop
   becomes a visible one.
5. **Timing quality is carried.** Producers fill `timing_quality` from the node's own time
   state ([ADR 1037](1037-fabric-internal-time-synchronization-first.md)); a consumer may
   weigh or refuse an event whose clock it cannot trust.
6. **Profiles apply at boundaries only.** Inside a goatnet the fabric carries profile H. The L
   and M profiles are applied where a constrained link or a release set
   ([ADR 1208](1208-sovereign-environments-release-namespaces-and-grant-encryption.md)) needs
   them. Priority stays with QoS by identity and namespace
   ([ADR 1033](1033-zenoh-qos-prioritization-reserved-mission-slice.md)).
7. **Command events are out of scope.** Tasking over the fabric is not decided here.
8. **No reference-code runtime.** ZMeta's Python gateway and tools are not deployed on any
   goatnet. CI may run them as a second opinion on the conformance vectors.
9. **One pinned version.** The schema is pinned to the locked v1.0 tag by content hash, with an
   `ops/upstream/deps/zmeta.yaml` manifest ([ADR 1262](1262-one-upgrade-process-for-load-bearing-upstream-dependencies.md)).
   The 1.1 lane is adopted only once it is locked.
10. **Contribute back.** goat offers the project a Zenoh transport binding (key-expression
    mapping, payload and diagnostics conventions) and a compiled validator that passes its
    vectors.

## Consequences

- The goat-processors specs (`filter.md`, `correlate.md`, `fuse.md`, `quality.md`) take ZMeta
  observation, fusion and state events as input and output; the Cursor on Target shaped record
  becomes one egress projection.
- Partners integrate a sensor by writing one ZMeta adapter, from the project's authoring
  guide, instead of learning goat's record.
- A refusal is visible on the fabric, which costs one extra event per refused message.
- goat depends on a standard maintained by one organization. Pinning the locked v1.0 and
  owning the validator keep that dependency to a document, not a running component.

## Alternatives

- **Keep goat's own track record.** Rejected: partners must learn a goat-only shape that
  cannot carry lineage, confidence or clock quality.
- **Cursor on Target as the canonical form.** Rejected: no lineage or layer separation; it
  remains a projection for TAK.
- **SAPIENT as the canonical form.** Rejected: built for sensor management, narrower than the
  measurement-to-picture chain; it remains a projection.
- **Run the reference gateway.** Rejected: one Python process forwarding over UDP with
  unpinned dependencies is a demonstration, not a data path.

## Build order (designed, not built)

1. `ops/upstream/deps/zmeta.yaml` and the vendored v1.0 schema and vectors at the pinned tag.
2. The Rust engine's validator and ingress pack in goat-processors, passing every vector.
3. The diagnostics key convention and the refusal event, documented in
   [`zenoh-data-fabric.md`](../design/zenoh-data-fabric.md).
4. Egress packs: Cursor on Target first, then SAPIENT.
5. The other engines, each against the same vectors.
6. The upstream contribution (decision 10).
