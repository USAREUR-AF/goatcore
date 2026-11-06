# Universal NTP Hygiene

Cross-plane policy document. Time correctness is a first-class system property that spans the Netbird management plane, the Netbird data plane, and the Zenoh data fabric. This doc is the single source of truth for the policy; plane-specific mechanics live in their own docs and reference back here.

**Status:** canonical. Rapid-win controls are Gate 2 blockers (`ha-multi-site-validation.md` §7). Deeper controls and the `ntp-health-agent` daemon are Phase 2/3 deliverables.

> **⚠ Foundational reframe — [ADR 1037](../adr/1037-fabric-internal-time-synchronization-first.md) (2026-06-09).** This document was written external-reference-first: external time (GNSS/NTS, [`ntp-strategy.md`](ntp-strategy.md) Tiers 1–2) as the *foundation* of correctness, per-node holdover (Tier 3) as the *fallback*. That is inverted. The fabric is brokerless and lives at the contested/denied/mobile edge where GNSS is jammed/spoofed and the WAN is cut — so external reference is exactly what cannot be assumed in the conditions that matter, and the population that must agree on time is **every fabric participant** (hundreds–thousands), not a control set. What the fabric needs is **relative consistency** (participants agreeing with each other on ordering/spacing), not each node being individually correct against UTC. Per [ADR 1037](../adr/1037-fabric-internal-time-synchronization-first.md): **internal peer-to-peer synchronization is the foundation** (gossip coupling, no external reference required — the model Corsaro designed for this regime and the one Zenoh itself was built for), **external references are opportunistic anchors** that pull the synced ensemble toward UTC when reachable, and **time health has two axes** — *internal coherence* (locked to the fabric ensemble; governs ordering/fusion/LWW) and *UTC confidence* (anchor presence + freshness; governs cert/TTL validity and cross-cluster timestamps). Peer clock inputs are **attestation-gated** (`ops/time-hygiene/internal/attest`) for Byzantine resistance. The sections below remain valid as the *anchor-supply + enforcement + attestation* mechanics; read them as subordinate to that foundation, not as the source of fabric time.

> **Runnable companion.** `hello-galaxy-time-hygiene` (Hello Galaxy series) walks this model in six scenarios — five of which need no infrastructure. It verifies a real `ntp-attest-v1` signature against a readable reference, exercises the five-state honesty grading, models the liar and the anchor-loss cases the engine would handle, and reproduces the RTC-less cold-boot failure with openssl. Its `docs/honesty-matrix.md` is the one place the live-versus-designed line is drawn for the whole time domain.

---

## Table of contents

1. [Principle](#1-principle)
2. [Publisher vs subscriber](#2-publisher-vs-subscriber)
3. [Three sources of truth — matched to consumer pattern](#3-three-sources-of-truth--matched-to-consumer-pattern)
4. [The `ntp-health-agent` daemon](#4-the-ntp-health-agent-daemon)
5. [Cross-site peer attestation](#5-cross-site-peer-attestation)
6. [Enforcement points across planes](#6-enforcement-points-across-planes)
7. [Fail-closed semantics](#7-fail-closed-semantics)
8. [Implementation phasing](#8-implementation-phasing)
9. [Open questions](#9-open-questions)
10. [References](#10-references)

---

## 1. Principle

**Every entity that publishes data to the tactical network must demonstrate time correctness as a precondition for publishing. Pure subscribers (entities that only receive and do not influence the state of other peers) can participate without enforcement.**

Per [ADR 1037](../adr/1037-fabric-internal-time-synchronization-first.md), "time correctness" resolves into **two distinct axes**, and the precondition for *publishing* is the first one:

- **Internal coherence (the publish precondition).** Is the publisher locked to the fabric's synchronized ensemble within a bounded error — i.e. will its timestamps order and correlate consistently with everyone else's? This is achievable peer-to-peer with **no external reference**, so it holds at the denied edge, and it is what event ordering, fusion/correlation, and multi-master LWW actually require (LWW correctness needs the *writers* to agree, not to be UTC-correct).
- **UTC confidence (required only for UTC-bound operations).** How well is the ensemble anchored to true UTC, and how fresh is the last anchor? Required for certificate / JWT / setup-key validity windows and for cross-cluster or human-facing absolute timestamps — **not** for ordering data on the fabric.

A node that is internally-coherent but UTC-unknown (the normal denied-edge state) is a **valid publisher** and degraded only for UTC-bound operations. Conflating the two (a single "is the clock correct against UTC" gate) is the error ADR 1037 corrects.

Time hygiene is *directed* (the required outcome is stated) but *not prescribed in implementation* (any mechanism that meets the consensus-backed attestation standard is acceptable). Participating systems implement the standard in whatever way suits their stack; the network validates the outcome, not the technique.

### 1.1 Why time correctness matters at this scale

- **Spock LWW conflict resolution** in the management plane is clock-dependent. A skewed mgmt-plane host wins conflicts it shouldn't.
- **Tactical data consumers** (fusion, track correlation, C2) rely on producer timestamps to order events, age-out stale data, and validate causality. Wrong time = wrong decisions downstream.
- **Cryptographic envelopes** (TLS certs, JWTs, signed attestations) depend on wall-clock validity windows.
- **Nation-state adversaries** with GPS-spoofing capability (documented in recent conflicts) can attack time directly, in addition to the usual supply-chain and credential-theft paths.

Time is infrastructure. Treating it as "just NTP" has hidden this.

### 1.2 Cold-boot clock persistence — the RTC-less denied-edge case

The RTC-less Jetson AGX Orin cold-boots at **1970** (no battery-backed RTC). Two layers keep a mgmt host usable from the first second regardless of connectivity, and they compose:

- **`chrony-presync`** — steps the clock to correct *once a network time source is reachable*, before Docker starts. Closes the **online** cold-boot case.
- **`fake-hwclock`** — persists the wall clock to disk (hourly + on shutdown) and restores it at boot *before the network*, so an **offline/denied-edge** cold boot starts at last-known-good instead of 1970. Without it, an offline mgmt host sits at 1970 and every mTLS handshake fails cert validation ("not yet valid") — NetBird/Zenoh/Ory/registry all break and the box can't rejoin its own fabric, defeating the ADR-1037 denied-edge posture. `fake-hwclock load` only *advances* the clock (never regresses), so it is safe fleet-wide and a no-op on RTC-equipped x86 hosts. Enabled by default in the `ntp-hygiene` role (`ntp_fake_hwclock_enabled`).

Together: fake-hwclock = *start near-correct offline*; chrony-presync = *step to correct online*; peer mutual-sync (§5) = *coherence at the denied edge*.

---

## 2. Publisher roles (NTP dimension)

The full role taxonomy — `publisher`, `unattested-publisher`, `subscriber` — is defined in [`publisher-trust-model.md`](publisher-trust-model.md). This section covers only the NTP-specific implications of that taxonomy.

| Role | NTP enrollment check | NTP ongoing check | NTP impact on published samples |
|---|---|---|---|
| **`publisher`** | Required — attestation validated at enrollment per §4 | Required — ongoing self-report + cross-site attestation per §5 | Samples emitted with `ntp_class = healthy` (or `warning`/`degraded` if sustained, triggering demote per §3.3 of [`publisher-trust-model.md`](publisher-trust-model.md) |
| **`unattested-publisher`** (subset reason `no_ntp_attestation`) | Skipped — NTP attestation not required | Not enforced | Samples emitted with `ntp_class = unknown` inherently |
| **`subscriber`** | N/A (no publish path) | N/A | N/A |

**The publisher/subscriber intuition is narrower than it seems.** Most mesh entities publish via at least one channel — last-seen updates, audit events, cert-expiry assertions, sensor readings. Pure subscribers are rare and must be explicitly declared, not assumed. See [`publisher-trust-model.md`](publisher-trust-model.md) §1 for the full framing.

**Role classification is per-setup-key.** Operator explicitly issues a `role=<value>` key. Role cannot be implicitly inferred from behavior. Transitions between roles require re-enrollment (see [`publisher-trust-model.md`](publisher-trust-model.md) §8).

---

## 3. Three sources of truth — matched to consumer pattern

A single logical property (a host's time health) surfaces through three source-of-truth mechanisms, each fit to its consumer pattern. **No one source is load-bearing for all consumers.** Conflating them was the prior architectural error.

| Source | Consumer pattern | Critical path? | Example consumers |
|---|---|---|---|
| **Local `ntp-health-agent` daemon** on each host — reads chrony + GNSS + received peer attestations; computes authoritative aggregate verdict | Fast local read; sub-ms | Yes | Admit-fail-safe sidecar; Zenoh router interceptor on the same host; local tooling |
| **Per-sample / per-request attestation** — signed attestation attached inline to each emitted data item | Inline validation by receiver | Yes (data plane) | Zenoh subscribers; mgmt API callers (optional) |
| **Prometheus time-series** — metrics scraped from each host's `ntp-health-agent` exporter endpoint | Eventually consistent, batch query | No — analytics only | Demote-job background loop; dashboards; alert evaluation; IOC watch |

**Key architectural move:** the daemon is the canonical local source. Everything else (inline attestation, Prometheus metrics, cross-site publication) derives from the daemon's one authoritative state. This eliminates the "four ad-hoc channels" anti-pattern.

---

## 4. The `ntp-health-agent` daemon

Small local Go daemon, one per mgmt-plane host, ~300–500 LOC. Provides a single authoritative local view of this host's time health with multiple consumption interfaces.

### 4.1 Scope and responsibility

- Aggregates multiple independent time-correctness signals into one verdict per host.
- Publishes that verdict through the consumption interfaces listed in §4.4.
- Does not itself manage the clock (chronyd does that).
- Does not replicate state across hosts (each host runs its own daemon; cross-site consensus comes from peer attestation in §5).

### 4.2 Inputs

| Input | Source | Independence from other inputs |
|---|---|---|
| `chronyd` tracking state | Local chrony control socket | Reflects chrony's aggregated view of NTS + GNSS + any refclock inputs |
| Direct GNSS state (Phase 3) | gpsd or direct device reads | Independent of NTP entirely — physical RF reference |
| Received cross-site peer attestations | Subscribed from peer attestation topic (see §5) | External — other hosts' views of their own clocks |

### 4.3 Authoritative state (in-memory, ~1s refresh)

```
{
  "host_id": "site-a-mgmt-01",
  "chrony": {
    "leap_status": "Normal",
    "ref_id": "203.0.113.141",
    "stratum": 2,
    "rms_offset_seconds": 0.000452,
    "last_offset_seconds": 0.000187,
    "nts_sources_synced": 2,
    "total_sources_synced": 3
  },
  "gnss": {
    "present": true,
    "fix_type": "3D",
    "satellites_used": 11,
    "agrees_with_chrony_ms": 0.3
  },
  "peer_consensus": {
    "peers_observed": 4,
    "median_offset_ms_vs_this_host": 2.1,
    "max_offset_ms_vs_this_host": 7.4,
    "divergence_detected": false
  },
  "verdict": "healthy",
  "verdict_reasons": [],
  "last_updated": "2026-04-20T15:42:03.127Z",
  "verdict_valid_until": "2026-04-20T15:47:03.127Z",
  "signed_attestation": "<base64 ed25519 signature over canonical fields>"
}
```

### 4.4 Consumption interfaces

All derive from the single in-memory authoritative state. Same data, different transport.

| Interface | Consumer | Purpose |
|---|---|---|
| **Unix socket** (`/run/ntp-health.sock`, JSON API) | Local admit-fail-safe sidecar; any local tool | Fast local read, sub-ms, no network |
| **HTTPS `:9753/health`** (signed JSON API) | Other hosts (cross-host queries); Zenoh router interceptors for remote producer gating; ad-hoc ops inspection | Authenticated remote query; response is signed with the host's attestation key |
| **Prometheus scrape `:9753/metrics`** | Prometheus | Analytics; dashboards; demote-job input; alerts |
| **Pub/sub** (Zenoh key expression `ops/time/attest/{host_id}`; subscribers use `ops/time/attest/**`) | Other hosts (cross-site peer attestation consumers); Zenoh routers needing continuous producer health | Continuous; signed; drives §5 consensus. Zenoh-outage fallback: consumers poll remote daemons' HTTPS `:9753/health` endpoint directly. |
| **CLI `ntp-health status`** | Operators | Human-readable |

### 4.5 Signing and attestation

Each externally-visible response includes a short-lived signature (~5 minute validity) over the canonical-form state, using the host's per-host Ed25519 attestation key. Purpose:

- Prevents forgery: an unauthenticated observer cannot claim "host X said it was healthy at T."
- Enables cross-site consensus: other hosts can verify attestations before using them.
- Compromise scope: theft of the attestation key = ability to forge for that one host; does not grant mesh admin or cross-site impact.

**Trust anchor for the public attestation key.** The peer's public attestation key is carried in the `ntp_attestation_pubkey` field of its offline-enrollment bundle ([`offline-enrollment.md`](offline-enrollment.md) §3), which is signed by the offline YubiKey-PIV Ed25519 CA. Verifiers therefore root attestation-key authenticity in the same CA chain that authorizes the peer's enrollment — a mgmt-host compromise cannot rewrite this binding because the CA signature is not held on any network-reachable host. Peers enrolled via operator-issued setup key (no bundle) have no CA-signed binding and their attestations are treated as `unattested-publisher` per [`publisher-trust-model.md`](publisher-trust-model.md) — accepted for local telemetry, excluded from cross-site consensus and publisher-scoped ACL groups.

Keys rotate per federation-security §O1 credential-rotation runbook. Rotation re-issues the bundle with a new `ntp_attestation_pubkey`.

**Canonical signing view — `ntp-attest-v1` (as built, increment 2a).** The signature covers a fixed, versioned subset of the verdict, serialized deterministically (sorted-key JSON over a struct with stable field order; timestamps as RFC3339Nano UTC strings so there is no monotonic-clock / precision drift between signer and verifier). The covered fields are exactly:

`alg` (`"ntp-attest-v1"`) · `host_id` · `ntp_class_str` · `selected_source` · `stratum` · `ref_id` · `rms_offset_ns` · `last_updated_utc` · `valid_until`.

The signature attests **the verdict and its time** — the load-bearing claim for cross-site consensus (§5) and admit gating. Volatile / not-yet-active fields (`drift_model`, `system_time_offset_ns`, `freq_ppm`, `leap_status`) are deliberately **excluded** so the signed bytes are stable; a future `ntp-attest-v2` can widen coverage without a silent mismatch (consumers that don't recognize the `alg` treat the attestation as *unverifiable*, not invalid). The agent embeds the result in its state as `signature` (base64 Ed25519), `attestation_pubkey` (base64 32-byte key), `attestation_alg`, and `attested` (false when no key is configured). Producer and verifier route through one shared `signingBytes` (`ops/time-hygiene/internal/attest`) so they can never diverge; `Verify` checks self-consistency only — trust in the key is rooted in the offline-CA-signed bundle above, and `Expired` separately enforces the validity window (staleness is a distinct failure mode from forgery).

### 4.6 What the daemon does NOT do

Important negatives:

- Does not override or replace chronyd — chrony still disciplines the clock.
- Does not itself gate any action — it's a read-only source; enforcement lives in the Netbird sidecar, Zenoh router interceptors, and the policy engine.
- Does not persist state durably across restarts — the in-memory view refreshes in seconds.
- Does not attempt to verify its own host's integrity — that's supply-chain / host-integrity defense (federation-security.md §1).

---

## 5. Cross-site peer attestation

Second independent signal. Resolves the "self-diagnosis paradox" (a host cannot verify its own clock purely internally).

### 5.1 Mechanism

- Each mgmt-plane host's `ntp-health-agent` publishes its signed attestation (§4.5) to Zenoh key expression `ops/time/attest/{host_id}` at a regular heartbeat (e.g., every 30s).
- Other hosts' daemons subscribe to `ops/time/attest/**` — Zenoh key-expression semantics give them all peers without enumeration.
- Each receiving daemon computes `pairwise_offset(local, peer)` using its local time + measured RTT vs. the peer's claimed time.
- Aggregated across peers, a host that disagrees with the majority is the suspect.
- **Zenoh-outage fallback:** if the subscription loses publishers for >2 consecutive heartbeat intervals, receivers fall back to polling each known peer's HTTPS `:9753/health` endpoint directly. Cross-site consensus degrades but doesn't stop.

### 5.2 Consensus algorithm (simple form)

For each host H, the daemon on H computes:

```
offsets = []
for peer P in reachable_peers:
    offsets.append(pairwise_offset(local=H, peer=P))
median_offset = median(offsets)
divergence_detected = abs(median_offset) > 100ms OR range(offsets) > 500ms
```

`divergence_detected` flows into H's own verdict (§4.3).

### 5.3 Why this breaks the paradox

- Host X's verdict now depends on signals FROM OTHER HOSTS, not only from X itself.
- For the adversarial case: to hide a skew on X, the adversary must compromise multiple hosts simultaneously and forge consistent attestations across all of them.
- For the non-adversarial case (chrony config bug, single source poisoned): peer consensus catches it immediately.

### 5.4 Residual trust assumption

The cross-site attestation mechanism trusts that `ntp-health-agent` itself is uncompromised. Defense for THAT assumption is host-integrity (supply chain, Canonical-signed Ubuntu Pro binaries, IMA/EVM on the agent binary, AIDE file-integrity monitoring, tamper detection) — not NTP-specific. Federation-security.md §1 covers it. See also [`orin-os-decision.md`](orin-os-decision.md) for the replacement of the original read-only-rootfs assumption.

---

## 6. Enforcement points across planes

| Plane | Enforcement point | Reads from | Action |
|---|---|---|---|
| Mgmt-plane hosts (LWW-correctness) | **Admit-fail-safe sidecar** in front of Netbird management API | Local `ntp-health-agent` Unix socket | Return 503 on admit-class calls when verdict ≠ `healthy`; revoke-class calls always allowed |
| Netbird enrollment | **Setup-key activation pre-hook** on enrolling host | Peer-provided attestation block in enrollment request payload + server time for RTT sanity | Deny enrollment if attestation missing, malformed, or fails RTT probe (§4 of [`mgmt-plane-threat-mitigations.md`](mgmt-plane-threat-mitigations.md) spec) |
| Netbird role governance | **Demote-job** cron | Prometheus (sustained degradation over N minutes) | Move peer from `role.publisher` group to `role.subscriber` group via mgmt API |
| Netbird ACL | **Netbird policy engine** (native) | Group membership | `role.subscriber` group policy denies reach to admin endpoints |
| Zenoh data fabric (producer-side) | **Zenoh router interceptor** at publish ingress | Local `ntp-health-agent` Unix socket (for local producers) OR remote daemon HTTPS `:9753/health` (for remote producers) | Drop samples from unhealthy producers; or tag with `ntp_class=degraded` |
| Zenoh data fabric (per-sample) | **`ntp_class` attachment** on every sample | Producer's local daemon state at publish time | Subscriber-side evaluation; unhealthy-class samples discarded or flagged by consumer |
| Zenoh data fabric (per-producer) | **`ntp-health/{producer-id}` heartbeat topic** | Producer's daemon publishing its own state | Subscribers correlate sample freshness against heartbeat recency |
| Mgmt-host NTS server tier (per [ADR 0834](../adr/0834-per-site-nts-server-tier.md)) | **chrony `allow` list scoped to LAN + wg-cp0** | Static config — `chrony_serve_nts_allow` per-site var | NTS-KE handshakes from outside the allowed CIDRs are dropped at the chrony layer; network-layer ACLs (LAN segmentation + wg-cp0 admission) are the primary defense; chrony allow-list is defense-in-depth |

This table is the single place to look when asking "where is time hygiene enforced for X?"

---

## 7. Fail-closed semantics

Every enforcement point fails **closed** — if the check cannot be performed (daemon unreachable, socket error, malformed response), treat as rejection, not permission.

### 7.1 Why fail-closed admit, not "admit + sever to island mode" (added 2026-05-02)

Recurring architectural question: when NTP is unhealthy at a site, should we **block new admits** (current design — fail-closed), or **admit but quarantine the new peer into island mode until NTP heals**?

**Current design wins on simplicity for the same correctness guarantee.** Reasoning:

NTP at a peer affects **both peer-local AND cross-network** correctness. The cross-network impact is what actually drives the design:

- **Spock multi-master LWW conflict resolution** — this peer's writes get a `commit_ts`; other peers across the mesh resolve conflicts by comparing those timestamps. A peer with bad time always wins (future-dated) or always loses (past-dated) → silent data corruption that's hard to recover from.
- **Setup-key TTLs** — issued at one peer, validated at another; TTL math breaks under skew.
- **mTLS cert validity** — peer presents certs to others; their validity is time-windowed.
- **Audit log correlation** — spans peers; needs a coherent time base. Bad-time peer poisons audit chain.

So "just admit and let the peer be in island mode" doesn't help — even an isolated peer making local writes corrupts shared state when those writes replicate. To make quarantine work you'd need:

- A **quarantine ACL group** with default-deny on all writes
- **Application-layer support** in postgres-spock (block writes from quarantined peers — non-trivial), netbird-mgmt (don't issue setup-keys until peer's NTP-healthy), audit pipeline (don't accept events from quarantined peers)
- **Promotion gate** that fires on NTP-healthy verdict and lifts the peer to its target ACL group

That's a real engineering project. Today's fail-closed-admit is a much smaller surface for the same correctness guarantee — block until NTP is healthy, then admit normally. NTP outages should be brief; admit can wait.

**When we'd build the quarantine path:** if operational reality shows we hit "NTP unhealthy AND need to admit emergency peer" repeatedly. Until then, fail-closed admit is the right design.

### 7.2 Service classification: load-bearing vs add-on (added 2026-05-02)

Not every service in the mgmt-plane is equally load-bearing. Operators triaging a degraded site need to know which services warrant immediate response and which can wait. Classification across the stack:

| Class | Services | What "down" means | Response |
|---|---|---|---|
| **Core load-bearing** (system halts/corrupts without these) | chrony NTP daemon, postgres-spock, signal-server, netbird-mgmt, WireGuard tunnels, LUKS unlock chain | Existing peers lose connectivity OR data corrupts silently | Page-immediately. Revert the change that broke it; don't accumulate downtime. |
| **Load-bearing for new admits** (gate for NEW peer enrollment, not existing operation) | ntp-health-agent, ntp-admit-sidecar, db-policy-engine, offline-CA validator, chrony NTS server tier (per [ADR 0834](../adr/0834-per-site-nts-server-tier.md) — distinct from chrony-as-client which is core load-bearing) | Existing peers keep working; new peers can't join until restored | Page-business-hours. Existing fleet is unaffected. Restore before next planned admit. |
| **Add-ons** (system-degrades, doesn't halt) | observability (Prom + OpenObserve), pgBackRest, Zenoh, admit-audit, tuwunel + hedwig (chat substrate per [ADR 0835](../adr/0835-secure-messaging-substrate-tuwunel.md)), hosted services (gatewatch, isitup, airdefenders) | Visibility / convenience features missing; mesh + identity + replication still healthy | Triage on next-business-day. Restore at next maintenance window. |

**Key implication for ntp-health-agent specifically:** it sits in the "load-bearing for new admits" class — losing the agent doesn't kill existing peers' WireGuard tunnels, signal connections, or replication. It blocks the admit path (which is the right fail-closed posture per §7.1) but doesn't degrade ongoing operations. Operators who arrive at a site with the agent crashed should see "no new peers can join" and "existing peers are fine, including their data flows." That's a load-bearing service in a specific dimension, not categorically — useful nuance for triage.

| Enforcement point | Check fails when... | Action (fail-closed) |
|---|---|---|
| Admit-fail-safe sidecar | Unix socket unreadable, daemon down, response malformed, signature invalid | Return 503 on admit-class calls |
| Enrollment pre-hook | Attestation missing from payload, parse error, RTT probe fails | Deny enrollment with clear error |
| Zenoh router interceptor | Daemon API unreachable for remote host, signature verification fails, timeout | Drop sample OR tag `ntp_class=unknown` (config choice per router) |
| `ntp_class` attachment | Producer daemon down at publish time | Producer client library sets `ntp_class=unknown` |
| Demote-job | Prometheus query fails or times out | Job retries on next cycle; no demotions that cycle (not critical path) |

Availability cost: when the daemon is down at a site, that site cannot accept admits until the daemon is restored. We accept this. The alternative (fail-open) would silently admit under uncertain time — the threat model we're defending against explicitly includes "time-manipulation-during-admit" as an attack vector.

---

## 8. Implementation phasing

### Rapid wins (Phase 1–2, Gate 2 blockers)

| Scope | Items | Effort |
|---|---|---|
| Chrony + NTS + observability | Items N1–N5 from [`mgmt-plane-threat-mitigations.md`](mgmt-plane-threat-mitigations.md) §1.1 | ~1 day |
| `ntp-health-agent` v1 | Daemon: chrony input + Unix socket + HTTPS + Prometheus exporter. No GNSS yet, no peer attestation yet. Single-signal integration. | ~3–5 days |
| Admit-fail-safe sidecar | Reads daemon over Unix socket; returns 503 on unhealthy; integrates with Netbird management API ingress | ~2 days |
| Enrollment pre-hook | Validates attestation payload + RTT probe at setup-key activation | ~1 day |
| Universal NTP as `role.publisher` / `role.subscriber` groups | Netbird group + policy configuration; demote-job cron | ~2 days |
| Coincident-infiltration rapid wins | Items C1–C5 from [`mgmt-plane-threat-mitigations.md`](mgmt-plane-threat-mitigations.md) §2.1 | ~4 days |

**Total Phase 2 effort: ~12–15 days** across the NTP hygiene surface. All Ansible / sidecar / Go-daemon work; no Netbird source modifications.

### Deeper layers (Phase 3+)

| Scope | Items | Effort |
|---|---|---|
| `ntp-health-agent` v2 | Add GNSS input; add cross-site peer attestation subscribe + pairwise offset computation; add signed attestation publishing | ~1 week |
| GNSS hardware rollout | ZED-F9T-based receivers per `procurement-ntp-gps.md`; Ansible integration | ~2 weeks (parallel to hardware lead time) |
| Zenoh router NTP interceptor | Compiled Rust `.so` interceptor that queries local daemon + validates per-sample `ntp_class` | ~1 week |
| Zenoh per-sample `ntp_class` library | Client library helper that producers call to attach `ntp_class` at publish | ~0.5 week |
| Zenoh heartbeat topic publisher | Daemon publishes `ntp-health/{host_id}` continuously on Zenoh | ~0.5 week |
| Cross-site attestation alerts | Prometheus rules for divergence detection; routing to IOC watch | ~0.5 week |
| Coincident-infiltration deeper layers | Items C6–C10 from [`mgmt-plane-threat-mitigations.md`](mgmt-plane-threat-mitigations.md) §2.2 | multi-week, staged |

---

## 9. Open questions

1. ~~Attestation topic transport~~ — **Resolved 2026-04-20: Zenoh** (originally on `@/ntp-health/{host_id}`). **Topic revised 2026-06-09 → `ops/time/attest/<host_id>`** (subscribers `ops/time/attest/**`): `@/` is Zenoh's *reserved adminspace* — router-introspection via GET/queryable, not a pub/sub data topic — and the original key predates [ADR 1035](../adr/1035-operator-namespace-ops-canonical.md)'s `ops/` operator-namespace canon, so the per-host signed attestation now lives under `ops/` like every other operator/control topic (the rolled per-site *viewing* reading is the separate `ops/time/health/<site>`; see F-184). Rationale (unchanged): zero new infrastructure, native fan-out, aligns with the "one fabric" architectural ethos at the mesh's site scale. Cold-start and Zenoh-outage degradation handled by fallback to each daemon's HTTPS `:9753/health` endpoint for cross-site consensus when the topic channel is unavailable. Revisit if Zenoh upgrades become invasive enough that security-control traffic independence becomes required.
   **Build status (2026-06-09), being closed in two steps (F-184):**
   - **2a — the signed attestation is now produced + enabled by default (DONE).** `ntp-health-agent` integrates the Ed25519 signer at each poll (`internal/attest`, alg `ntp-attest-v1`) and serves the signed verdict — `signature` + `attestation_pubkey` + `valid_until` + `attested` — on both the Unix socket and HTTP `:9753/health`. The canonical signing view is fixed and versioned (§4.5). The `ntp-health-agent` ansible role now **wires signing on by default** (`ntp_health_agent_attestation_enabled: true`): it passes `--attestation-key` pointed at the per-host key the role generates with `openssl genpkey -algorithm Ed25519` (**PKCS#8** — `signer.Load` accepts both PKCS#8 and raw-seed), and **asserts a signing-capable binary before enabling the gate** — a pre-2a binary doesn't know the flag and would crash-loop the daemon (blocking enrollment via the admit sidecar), so the role fails first with a `make build` instruction. Set the var false to roll back to unattested (`attested:false`, §2 unattested-publisher). Backward-compatible: the admit sidecar still reads only `ntp_class_str`.
   - **2b — the bus publisher + its deploy layer built; operator deploy + roll-up remain.** `goat-ntp-health-pub` (`ops/zenoh-trial-peers/ntp-health-pub/`) — a small Rust per-host sidecar (templated on `goat-fabric-health`) that reads the agent's signed `:9753/health` and republishes it **verbatim** (it does not re-sign) on `ops/time/attest/<host_id>` over mTLS at a 30 s heartbeat, in C0 (Go had no native Zenoh mTLS binding when this was chosen — same reason as the `rest-pub-bridge`, [ADR 1023](../adr/1023-zenoh-rest-pub-bridge-for-non-rust-publishers.md); cgo `zenoh-go` shipped in 1.9.0, but Rust stays the choice here). The router ACL grant for `ops/time/attest/**` (CN `ntp-health-pub-trial-peer`, infra peer) is wired in the `goat-zenoh` template (covers nova-a + ZAGROS). The **deploy layer** is built too (mirroring `fabric-health`): the `ntp-health-image` multi-arch image workflow, the `ntp-health-pub` ansible role (defaults + cert/CA pre-flight + compose + deploy-history), and `ntp-health-pub-deploy.yml` (`-e target=zagros_hosts` for ZAGROS). The container is on the zenoh network and reaches the host agent via `host.docker.internal:9753`. **Remaining:** operator-run steps — `make build` the image (it pins `:latest` pending first build), mint the `ntp-health-pub-trial-peer` leaf cert, run the playbook, live-fire — and the `goat-time-health` roll-up reader. A separate, **non-enforcing viewing plane** then rolls it up: `goat-time-health` subscribes `ops/time/attest/**` and republishes a per-site honesty-graded reading on `ops/time/health/<site>` (`EnvHealthV1{domain:"time"}`), co-located with fabric health and rendered at `goat health`/panoscope/stethoscope ([`environment-health-surface.md`](environment-health-surface.md)). The `ops/time/attest/**` feed stays the signed enforcement-grade source; the roll-up only *views* it. Completing 2b also unblocks the cross-site consensus loop (§5) and the router interceptor (§6), whose absent consumer-pull is why the publisher slipped.
2. ~~`ntp_class` wire format~~ — **Resolved 2026-04-20:** Zenoh sample attachment carrying a 1-byte enum (`healthy=0 | warning=1 | degraded=2 | unknown=3`) plus a 4-byte monotonic `producer_epoch` from the producer's `ntp-health-agent` daemon. **5 bytes per sample.** Fast-path subscribers read the byte alone; rich subscribers reconcile via a local cache populated from the `ops/time/attest/**` attestation topic, looking up by `(producer_host_id, producer_epoch)`. Cache-miss fallback is HTTPS `:9753/health` query against the producer's daemon. Full attestation (signed JSON) is never inlined per-sample — published once per heartbeat on the attestation topic, not smuggled into every data sample. Rationale: avoids per-sample bandwidth cost while keeping detail at sub-ms cache lookup; epoch prevents race between sample and newer attestation.
3. ~~Subscriber-side behavior on `ntp_class=degraded`~~ — **Resolved 2026-04-20:** Default PASS-with-flag (samples reach subscribers regardless; subscriber-side library reads the flag and logs when degraded samples pass through). Router operators can opt specific high-integrity topics into drop-on-degraded policy (Option D) per-topic via router config — when that fires, **the drop is surfaced on the write-denial observability surface (the expected-vs-actual monitor, [ADR 1022](../adr/1022-zenoh-denial-observability.md)) rather than being silently dropped**. (The 2026-04-20 draft sent drops to a `@/dlq/ntp_degraded` dead-letter queue; that is **superseded** — a DLQ is a broker delivery-attempt pattern and Zenoh is brokerless with no native dead-letter (F-178); ADR 1022 chose expected-vs-actual monitoring + producer-cooperation as "the wrong category" replacement.) Subscriber SDK makes drop-on-degraded a one-line opt-in at subscription time. Rationale: mission decisions stay in the mission layer; high-integrity topics get fabric-level enforcement when operators choose; naive subscribers leave a visible trail via prominent logging.
4. ~~Mgmt-plane host response to local daemon failure~~ — **Resolved 2026-04-20:** Default `halt_replication` — local Spock worker pauses (both outgoing push and inbound accept) when `ntp-health-agent` fails for >N seconds; admit-fail-safe sidecar also returns 503. Parameterized via `mgmt_host_on_daemon_failure` Ansible variable; opt-in `admit_blocks_only` for deployments prioritizing availability over correctness. Three operator-invokable break-glass severance ops (unilateral-disconnect, bidirectional-sever, isolate-this-site) documented in `pg-spock-security.md` §X. Break-glass semantics are partial-by-default (firewall our side; upgrade to binary if peer reachable). A designated coordinator node (the "high-resource environment node") has the privileged capability to invoke a cluster-wide isolation — severing a target site from all other sites — propagated via SSM/mesh to every reachable peer. Non-coordinator sites can only sever their own subscriptions.

5. **Cluster size is architecturally capped at 5 sites.** Cross-site attestation is O(N²) at N sites — 20 links at 5 sites, manageable. At larger N the fan-out cost and the correlated DDL-coordination + LWW-conflict surface grow unacceptably under approach B.

   **Scaling the system without exceeding 5 sites per cluster:**
   - **Vertical per site** — a "site" is one mgmt-plane host. Peers served by that host can grow unbounded (hundreds, thousands). Scale hardware, not cluster-site count.
   - **Per-region clusters with bridging** — to expand further, build a second independent 5-site cluster for the new region. Each cluster is internally multi-master; clusters exchange only selected data via bridges at stable locations. Not a single cluster, but multiple clusters that cooperate.

   **This is not a solved-elegantly problem.** The unsolved part is how region-cluster bridging handles identity, policy, and event correlation — essentially "federation of multi-master clusters." That's a separate design effort; deferred until we actually need it.

---

## 10. References

- [`docs/design/ntp-strategy.md`](ntp-strategy.md) — three-tier source / blend / self-clock strategy (companion; this doc is policy + enforcement, that doc is sources + drift)
- `docs/design/ha-multi-site.md` — committed multi-master design (approach B)
- `docs/design/ha-multi-site-validation.md` — validation plan + tech-debt register + go/no-go gates
- [`docs/design/mgmt-plane-threat-mitigations.md`](mgmt-plane-threat-mitigations.md) — Netbird-plane-specific mechanics; points back to this doc for policy framing
- `docs/design/federation-security.md` — federation attack surface + threat model (supply-chain controls for `ntp-health-agent` integrity)
- `docs/design/zenoh-cds-data-conditioning.md` — Zenoh interceptor chain architecture (where the NTP interceptor plugs in)
- `docs/design/zenoh-data-fabric.md` — Zenoh as the data fabric
- `docs/project/procurement-ntp-gps.md` — GNSS receiver procurement for Phase 3 GNSS rollout
- [NTS — Network Time Security (RFC 8915)](https://datatracker.ietf.org/doc/html/rfc8915)
- [chrony documentation](https://chrony-project.org/doc/)
