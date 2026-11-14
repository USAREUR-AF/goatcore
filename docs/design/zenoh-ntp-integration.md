# Zenoh NTP Hygiene Integration

**Scope:** the Zenoh-side mechanics of the universal NTP hygiene policy. Policy and cross-plane framing live in [`universal-ntp-hygiene.md`](universal-ntp-hygiene.md); this doc covers only the Zenoh data-fabric pieces: the attestation topic, the per-sample attachment, the router interceptor, and subscriber-side handling.

**Status:** mainline, Phase 2/3 deliverable. Attestation topic + per-sample `ntp_class` are Gate 2 work; router-level drop-on-degraded enforcement (Option D per-topic) is Phase 3.

> **Two corrections (2026-06-09) — read with these in mind.** (1) **Attestation topic:** the per-host signed verdict publishes on `ops/time/attest/<host_id>` (subscribers `ops/time/attest/**`), **not** the original `@/ntp-health` — `@/` is Zenoh's reserved adminspace, and `ops/` is the canonical operator namespace ([ADR 1035](../adr/1035-operator-namespace-ops-canonical.md), F-184). (2) **No DLQ:** the `@/dlq/ntp_degraded` "DLQ interceptor" paths below are **superseded** — a dead-letter queue is the wrong category on a brokerless fabric (F-178). A dropped sample is surfaced on the **expected-vs-actual write-denial monitor** ([ADR 1022](../adr/1022-zenoh-denial-observability.md)), not routed to a queue; must-land writes use acked Query/Reply (producer-cooperation). The DLQ references that remain in this doc are pending a follow-on reconciliation pass.

---

## Table of contents

1. [Two data flows](#1-two-data-flows)
2. [Attestation topic (control plane for time hygiene)](#2-attestation-topic-control-plane-for-time-hygiene)
3. [Per-sample `ntp_class` attachment (data plane)](#3-per-sample-ntp_class-attachment-data-plane)
4. [Router NTP interceptor](#4-router-ntp-interceptor)
5. [Subscriber-side policy](#5-subscriber-side-policy)
6. [Integration with the DLQ](#6-integration-with-the-dlq)
7. [References](#7-references)

---

## 1. Two data flows

Zenoh carries two NTP-hygiene-related flows:

- **Attestation flow (control plane)** — each mgmt-plane host's `ntp-health-agent` publishes a signed attestation on `ops/time/attest/{host_id}`. Heartbeat rate (~30s). Consumed by other hosts' daemons (cross-site consensus) and by Zenoh router interceptors (producer health lookup for remote producers).
- **Per-sample attachment (data plane)** — every published sample carries a 5-byte Zenoh attachment (`ntp_class` + `producer_epoch`) emitted by the producer's `ntp-health-agent`. Subscribers read the byte for fast-path decisions; rich subscribers reconcile to full attestation via the cache from the attestation flow.

These two flows share a source of truth (the daemon) but serve different latency / scale requirements. Covered separately below.

---

## 2. Attestation topic (control plane for time hygiene)

Every mgmt-plane host's `ntp-health-agent` daemon publishes its signed time attestation on:

```
ops/time/attest/{host_id}
```

- **Heartbeat rate:** ~30s
- **Payload:** signed JSON per [`universal-ntp-hygiene.md`](universal-ntp-hygiene.md) §4.3 (authoritative state + ed25519 signature)
- **Subscription pattern:** `ops/time/attest/**` to consume all hosts' attestations without enumeration

### Consumers

- Other mgmt-plane hosts' daemons — cross-site consensus input per [`universal-ntp-hygiene.md`](universal-ntp-hygiene.md) §5
- Zenoh router NTP interceptors — producer-health lookup for remote producers
- Prometheus exporter — a scrape bridge that subscribes to `ops/time/attest/**` and exposes metrics for dashboards and the demote-job

### Zenoh-outage fallback

If the subscription loses publishers for >2 consecutive heartbeat intervals, receivers fall back to polling each known host's HTTPS `:9753/health` endpoint directly. Cross-site consensus degrades but does not stop. Zenoh availability is **not** a single point of failure for NTP hygiene observability.

---

## 3. Per-sample `ntp_class` attachment (data plane)

Every Zenoh publication carries a 5-byte attachment:

```
ntp_class:       1 byte  enum { healthy=0, warning=1, degraded=2, unknown=3 }
producer_epoch:  4 bytes monotonically-incrementing counter from producer's daemon
```

### Subscriber patterns

| Posture | Reads | Cost per sample |
|---|---|---|
| **Fast-path gate** ("drop everything tagged degraded") | `ntp_class` byte only | 1 byte comparison |
| **Rich consumer** (fusion, correlation, audit) | `ntp_class` + cache lookup by `(producer_id, epoch)` | Sub-ms — cache is in-process |
| **Forensics / audit** | Cache + Prometheus history | Offline |

### Why the epoch matters

Without the epoch, a sample tagged `healthy` arriving at a subscriber might be matched against a DIFFERENT attestation (from the producer's newer heartbeat) during lookup. The epoch binds the sample to the specific attestation that was in effect at emission time, eliminating the race.

### What's NOT inlined

The signed JSON attestation is **never** duplicated into every sample. It's published once per heartbeat on the attestation topic. At 100k samples/sec rates, inlining full attestations would add tens of MB/s of wire overhead per producer for a value that's already available via the heartbeat topic and cache.

### Forward-compatibility — `trust_flags` extension (Phase 3+)

The 5-byte attachment carries only the NTP dimension. When non-NTP attestation dimensions start mattering on the wire (first non-NTP use case: DMZ ingestion of external-source samples per [`publisher-trust-model.md`](publisher-trust-model.md) §4), the attachment extends to 6 bytes by prepending a `trust_flags` byte per [`publisher-trust-model.md`](publisher-trust-model.md) §5.

**Extension is additive.** Existing 5-byte consumers interpret the legacy samples as having default `trust_flags = mesh_native | cert_trusted | ntp_attested | enrollment_attested` (all bits set for fully-attested mesh-native publishers). New 6-byte consumers read the explicit flags.

**Do not pre-build the extension in Phase 2.** Ship the 5-byte format with NTP only. Add `trust_flags` at the moment DMZ / partner-ingestion / external-source handling actually ships.

---

## 4. Router NTP interceptor

> A router with `drop_future_timestamp: false` already replaces a timestamp more than 500 ms ahead of its own clock and logs the rejection. Reading those rejections per peer, and telling the peer's owner, is [`peer-fault-handling.md`](peer-fault-handling.md) (ADR 1232; designed, not built).

A compiled Rust interceptor `.so` in the Zenoh router chain. Per `zenoh-cds-data-conditioning.md`, the router supports an ordered interceptor chain; this is one entry.

### Producer-side (attach `ntp_class`)

On each published sample:

1. Read producer's current NTP verdict from the local `ntp-health-agent` Unix socket (`/run/ntp-health.sock`).
2. Attach the 5-byte `ntp_class + producer_epoch` to the sample's Zenoh attachment field.

Fast-path. Sub-ms. No network calls.

### Relay-side (validate + optionally gate)

On samples transiting through a relay router:

1. Read the incoming sample's `ntp_class + producer_epoch`.
2. For high-integrity topics (see §5), the interceptor may set `drop_intent=ntp_degraded` on samples tagged `degraded`. The DLQ interceptor at the end of the chain routes these to `@/dlq/ntp_degraded/...` per [`zenoh-dlq.md`](zenoh-dlq.md).
3. For all other topics, samples pass through unchanged.

### Remote-producer health lookup

If a relay needs to make a decision about a sample from a producer on another host (e.g., to apply drop-on-degraded policy), it does NOT look up the producer's daemon over HTTPS per sample. Instead, it maintains a local cache from `ops/time/attest/**`. Sample decisions query the cache, not the remote daemon.

### 4.1 Failure-mode matrix — Zenoh, daemon, control-plane independence

The control plane (`wg-cp0` mgmt-API on-ramp per ADR 0313 + [`offline-enrollment.md`](offline-enrollment.md) §11), the data fabric (Zenoh), and the local time daemon (`ntp-health-agent`) are **independent failure domains**. The router interceptor and §5.1 Zenoh-outage fallback in [`universal-ntp-hygiene.md`](universal-ntp-hygiene.md) cover the daemon and Zenoh axes; this table makes the cross-product explicit so silent control-plane work doesn't introduce a hidden coupling.

| `wg-cp0` mgmt-API | Local `ntp-health-agent` | Zenoh router (local) | Effect | What's still safe |
|---|---|---|---|---|
| up | healthy | up | Normal operation | All paths hot |
| up | healthy | **down** | New devices can still enroll over `wg-cp0`; samples in flight lose `ntp_class` attachment for the duration; peer attestations stop publishing on `ops/time/attest/{me}` until router recovers; receivers fall back to HTTPS `:9753/health` polling per `universal-ntp-hygiene.md` §5.1; cached attestations from earlier heartbeats remain valid for ~2 heartbeat intervals (~60s) | Enrollment path; HTTPS health-poll fallback; subscriber drop-on-degraded gates can still trip on cached classes |
| up | **degraded** | up | **Admit-class is fail-closed at the sidecar** per [`universal-ntp-hygiene.md`](universal-ntp-hygiene.md) §7 — `POST /api/setup-keys`, `POST /api/peers`, `POST /api/groups`, and gRPC `/management.ManagementService/Login` return 503. Existing peers continue (Sync passes through; data plane runs P2P / relay-fallback). Samples emitted carry `ntp_class=degraded`; subscribers gate per §5; cross-site consensus marks this host as outlier. Bundle-side cryptographic signature checks (offline-CA-signed `ntp_attestation_pubkey`) are not affected by local time, but bundle *expiry* checks ARE if local clock skew is large. | Existing-peer Sync; data plane mesh; per-sample honesty; cross-site quorum still functions on the *other* hosts |
| up | **degraded** | **down** | Worst common case for new enrollments. Admit-sidecar still blocks (it gates on the local daemon's Unix socket, which is independent of Zenoh). Samples carry no class (no router). Local degraded state not published. Cross-site loses one input. | Existing-peer Sync; data plane mesh via P2P; HTTPS `:9753/health` poll still answers for any consumer that wants a fresh verdict |
| **down** | healthy | up | Existing peers continue mesh ops via P2P / data-plane relays; **no new enrollment** until `wg-cp0` recovers; data fabric carries on; cross-site time consensus continues | Mesh data plane; existing peer attestations |
| **down** | healthy | **down** | Same as previous row plus the local Zenoh-degraded effects | Mesh data plane via direct P2P; HTTPS poll for peers that need cross-site verdicts |

**Implication for router interceptor.** The §4 cache from `ops/time/attest/**` is *advisory* in degraded modes — when no fresh attestation is available for the producer host, the interceptor must NOT silently treat the sample as `healthy`. ADR 0511 publisher-trust model + [`zenoh-dlq.md`](zenoh-dlq.md) reason `ntp_unattested` cover that path; for high-integrity topics, an unattested sample lands in the DLQ rather than the subscriber.

**Implication for Block 22 cutover.** When `wg-cp0` is brought up before the bundle pipeline carries `cp_device_pubkey` end-to-end (Block 22A → 22B sequencing in implementation-plan.md), enrollment through `wg-cp0` is gated on the device having a Block-22-compliant bundle. Devices on legacy bundles continue using the dual-stack public listener until the 14-day overlap window (Block 22D) closes. This matrix's "wg-cp0 up" rows assume the device has a compliant bundle in hand.

---

## 5. Subscriber-side policy

**Default:** PASS with flag visible. Samples reach subscribers regardless of `ntp_class`; the attachment is always available for subscriber-side logic.

**Per-topic router override (Option D):** for high-integrity topics, the router operator can configure drop-on-degraded at the router interceptor (§4). Samples never reach subscribers; they go to the DLQ (`@/dlq/ntp_degraded/...`).

**Subscriber library helper:** a one-line opt-in for drop-on-degraded at subscription time. Subscribers that want strict filtering enable it; simple subscribers that don't care leave it off.

**Prominent logging:** subscribers using the standard library log a warning-level event every time a `degraded` sample is passed through (rate-limited, not per-sample). Naive subscribers that ignore the flag still produce a visible trail.

### Selection guide

Use drop-on-degraded at the router for a topic when:
- The topic carries load-bearing-timing data.
- Downstream consumers cannot reason about degraded data safely.
- The cost of dropped-but-legitimate data is less than the cost of degraded-reaching-consumer.

Use default pass-with-flag when:
- Consumers are fusion/correlation engines that can reason about confidence.
- The data is advisory (telemetry, health, mission chat, positions).
- Loss-during-NTP-glitch is worse than degraded-but-flagged.

---

## 6. Integration with the DLQ

When the router interceptor drops a sample due to `ntp_class=degraded` and drop-on-degraded policy, the sample goes to `@/dlq/ntp_degraded/{original-key}` per [`zenoh-dlq.md`](zenoh-dlq.md) with reason `ntp_degraded`.

Default retention for `ntp_degraded` DLQ entries: 24h. This creates the window for replay-on-NTP-recovery workflows (Phase 3).

---

## 7. References

- [`docs/design/universal-ntp-hygiene.md`](universal-ntp-hygiene.md) — cross-plane policy + daemon design (`ntp-health-agent`)
- `docs/design/zenoh-data-fabric.md` — parent Zenoh doc
- [`docs/design/zenoh-dlq.md`](zenoh-dlq.md) — DLQ pattern; `ntp_degraded` is one reason code
- `docs/design/zenoh-cds-data-conditioning.md` — router interceptor chain mechanism (one of many use cases)
- [`docs/design/mgmt-plane-threat-mitigations.md`](mgmt-plane-threat-mitigations.md) — Netbird-plane NTP mechanics
