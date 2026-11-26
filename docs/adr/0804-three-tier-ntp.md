# ADR 0804 — Three-tier NTP — GNSS + TM1000A + NTS + adaptive

| Field | Value |
|---|---|
| Status | Accepted; amended 2026-11-26 — the ntp-admit-sidecar is retired; management ingress has no clock gate; a clock check on new-peer registration inside the management server is designed, not built |
| Date | 2026-04-25 |
| Repo scope | goat core |
| Related | [`../design/ntp-strategy.md`](../design/ntp-strategy.md), [`../design/universal-ntp-hygiene.md`](../design/universal-ntp-hygiene.md), [`../design/zenoh-ntp-integration.md`](../design/zenoh-ntp-integration.md) |

## TL;DR (for a layperson)

Accurate time is critical for cryptography, audit, replication,
and anomaly detection. Most networks rely on internet NTP,
which we cannot trust. We use three layers of time sources at
each site: a multi-band GPS receiver for sovereign time, a
hardware time appliance (TimeMachines TM1000A) for redundancy,
and authenticated public NTS servers as backup. A daemon at
every node honestly reports its uncertainty when self-clocking.

## Context

Tactical environments cannot trust the public internet for
time. Even when reachable, public NTP is unauthenticated by
default and vulnerable to spoofing. Cryptographic operations,
audit timestamps, replication conflict resolution, and
anomaly detection all degrade with bad time.

The design uses three tiers so any one tier can fail without
losing time discipline.

## Decision

Three-tier time architecture, per site:

| Tier | Source | Role |
|---|---|---|
| **1. Per-site sovereign** | u-blox ZED-F9T multi-GNSS + TimeMachines TM1000A | Primary time discipline; sovereign. |
| **2. NTS-only public** | Authenticated NTS servers (Cloudflare + Google) | Backup; ZTOL-ingested with hygiene rules. |
| **3. Per-node adaptive** | EMA-based drift model | Each node tracks its own clock drift; reports honest uncertainty when self-clocking under partition. |

Local blend via chrony with NTS-only ZTOL ingestion of public
sources.

Hardware BOM in
`../project/procurement-ntp-gps.md`.
Cross-plane policy + daemon + attestation + enforcement in
[`../design/universal-ntp-hygiene.md`](../design/universal-ntp-hygiene.md).
Strategy in [`../design/ntp-strategy.md`](../design/ntp-strategy.md).

Daemon source: `ops/time-hygiene/` (Go module). One binary
ships: `ntp-health-agent`. Management ingress has no clock
gate; a clock check on new-peer registration belongs inside the
management server, where a setup-key registration can be told
apart from a re-login (designed, not built;
[`universal-ntp-hygiene.md`](../design/universal-ntp-hygiene.md) §6). A
setup-key activation pre-hook is described in
`universal-ntp-hygiene.md` §"Setup-key activation pre-hook" but
is not yet shipped as a separate `ntp-enrollment-prehook`
binary — that integration is a forward task.

Ansible roles:
`ops/ansible/roles/ntp-health-agent/`,
`ops/ansible/roles/ntp-hygiene/` (chrony + NTS configuration
+ `chrony_exporter`).

> **Correction note (2026-04-25).** The original draft of this
> ADR listed Tier-2 NTS providers as "Cloudflare, NIST" — the
> canonical `ntp-strategy.md` §3.1 actually pins Cloudflare +
> Google. The original draft also listed `ntp-enrollment-prehook`
> as a shipped daemon — it is not in
> `ops/time-hygiene/cmd/`. The original draft also missed the
> `ntp-hygiene` Ansible role. All three corrected above. Original
> draft authored without opening the design doc or grepping the
> roles directory.

## Alternatives considered

- **Public NTP only.** Insecure; spoofable. Rejected.
- **Single GPS source per site.** Single point of failure.
  Rejected.
- **Cloud-hosted time service.** Loses sovereignty.
  Rejected.
- **No drift model on nodes.** Self-clocking accuracy
  unknown; corrupts anomaly detection. Rejected.

## Consequences

**Positive.** Sovereign time. Three independent tiers.
Honest uncertainty reporting prevents time-based anomaly
false-positives under partition.

**Negative.** Hardware procurement (GNSS antenna,
TM1000A). Per-site cost; accepted.

## Assessor view (SCA-R)

Three-tier NTP serves AU-8 (time stamps), SC-45 (system
time synchronization), and SI-4 (system monitoring —
honest uncertainty supports anomaly detection). NIST SP
800-90 / SP 800-117 (NTPv4) is consistent with the
authenticated NTS posture.

## Trigger conditions for revisit

- GNSS jamming becomes routine in a site's environment.
  Add anti-jamming hardware (CRPA antennas).
- A specific authorization requires Authenticated NTP
  (RFC 8915 NTS) exclusively. Drop tier 1 hardware where
  authentication is the binding requirement.

---

*See `GLOSSARY.md` for terms and acronyms.*
