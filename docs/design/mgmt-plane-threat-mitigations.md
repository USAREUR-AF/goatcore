# Management-Plane Threat Mitigations

**Scope:** first-class defense layers for two named threats against the multi-master active-active management plane:

1. **NTP correctness** — because LWW conflict resolution in Spock is clock-dependent; silent clock skew silently breaks conflict correctness.
2. **Coincident infiltration + partition** — because island-mode admits are a defender gift to any adversary who compromises a site and severs its link.

These are the layered defenses that make **approach B** (multi-master active-active per `ha-multi-site.md`) defensible against a nation-state threat profile (per `federation-security.md` §1).

Each section is split into:

- **Rapid wins** — Phase 1–2 deliverables, baked into the current HA implementation plan. No Netbird source modifications. Ansible + Prometheus + Grafana + config.
- **Deeper layers** — Phase 3+ deliverables. May involve custom code, hardware, operator workflow design.
- Where applicable, a **universal requirement** framing — how the control is enforced as a precondition for mesh participation.

---

## Table of contents

1. [NTP correctness](#1-ntp-correctness)
2. [Coincident infiltration + partition defense](#2-coincident-infiltration--partition-defense)
3. [Universal NTP hygiene requirement](#3-universal-ntp-hygiene-requirement)
4. [Integration with HA implementation plan](#4-integration-with-ha-implementation-plan)
5. [References](#5-references)

---

## 1. NTP correctness

**Goal:** time correctness is a precondition for writability. Redundant sources, active anomaly detection, and bounded damage on failure.

**Threat recap:** Spock LWW resolves conflicts by commit timestamp. A site with a skewed clock wins conflicts it shouldn't. An adversary who can influence NTP at a site can steer admit/revoke resolution. Silent skew also happens without adversarial intent (NTP source degradation, holdover oscillator drift).

### 1.1 Rapid wins (Phase 1–2, Gate 2 deliverables)

All of these run on existing infrastructure with no new hardware.

| # | Control | Effort | Delivered by |
|---|---|---|---|
| N1 | **chrony with ≥2 authenticated NTP sources (NTS — Network Time Security)** at every mgmt-plane host. NTS sources include `time.cloudflare.com`, `nts.netnod.se`, `ptbtime1.ptb.de`. | 2h Ansible | `ops/ansible/mgmt-host.yml` (new role `ntp`) |
| N2 | **`chrony_exporter` on every host**, scraped by Prometheus. Exposes offset, stratum, source count, sync status. | 1h per host | Ansible role + existing Prometheus config |
| N3 | **Alerts (wired into debt item 5 observability work):** | 2h rules + test | Alertmanager |
| | • **Warning:** `chronyd_tracking_rms_offset_seconds > 0.1` (>100ms local skew) | | |
| | • **Warning:** fewer than 2 sources marked synced | | |
| | • **Critical:** `chronyd_tracking_last_offset_seconds > 5` (>5s disagreement) | | |
| | • **Critical:** `chronyd_tracking_leap_status != "normal"` for >10 min (indicates degraded sync) | | |
| | • **Critical:** no `chrony_exporter` scrape response for >5 min | | |
| N4 | **Pre-flight NTP check in `coordinated-upgrade.yml`** — refuse to run upgrade if any target host has degraded NTP. Adds to the preflight check set already planned for debt item 3. | 1h | Ansible |
| N5 | **NTP health in smoke-test** — `ops/smoketest/` already runs regularly; add NTP assertion. Any host failing NTP triggers an alert, not just a silent log line. | 1h | smoketest |

**Total Phase 2 effort: ~1 day. No new hardware. No code changes to Netbird or our signal server.** These are sufficient to catch the accidental-failure and most-adversarial-NTP-poisoning cases.

### 1.2 Deeper layers (Phase 3+, production hardware rollout)

These require hardware procurement (see `docs/project/procurement-ntp-gps.md`) and/or custom code.

| # | Control | Notes | Gate |
|---|---|---|---|
| N6 | **Multi-constellation GNSS receivers at every site** (GPS + Galileo + GLONASS + BeiDou). Feeds chrony as a local authoritative source independent of internet NTP. | Hardware: u-blox ZED-F9T-based boards. See procurement doc. | Phase 3 |
| N7 | **GPS-spoofing detection heuristics** — cross-check GNSS time against NTS consensus; alert if GNSS disagrees with 2+ NTS sources by >500ms. Mitigates GPS spoofing (real in contested theater — documented Russia/Syria, Russia/Ukraine). | Software on receiver host. Rule lives in chronyd config + Prometheus. | Phase 3 |
| N8 | **Cross-site signed time attestation** — each site periodically writes a signed time observation to a low-churn replicated table `time_attestations`. Each site reads others' observations, computes pairwise offset, alerts on divergence. | ~1 week of code + schema. Bypass route if Spock is catching up. | Phase 3 |
| N9 | **NTP check on new-peer registration inside the management server** — on a setup-key registration, management reads the local clock verdict and refuses the registration when time sync is degraded. A re-login by an existing peer is not gated; revokes are always allowed. Management ingress has no clock gate. | A change inside the patched management server ([ADR 0301](../adr/0301-patched-netbird-management-image.md)). Designed, not built. | Phase 3 |
| N10 | **Post-hoc commit-time verification** — replication receiver stamps a second timestamp on row arrival; reconcile job flags rows whose commit-time is implausibly offset from arrival-time. | ~3 days + scheduled job. | Phase 3 |

---

## 2. Coincident infiltration + partition defense

**Goal:** an isolated site can still admit during partition (mission requirement in `ha-multi-site.md` §1.2), but an adversarial admit cannot reach full effect without independent observation by a human — and the observation happens on a channel the adversary has to separately compromise.

**Threat recap:** adversary compromises admin creds at one site AND severs that site's link AND admits themselves via the local mgmt API. On reconnect, the admit replicates to all sites. State adversaries are patient enough to wait for or manufacture this condition.

### 2.1 Rapid wins (Phase 1–2, Gate 2 deliverables)

| # | Control | Effort | Delivered by |
|---|---|---|---|
| C1 | **Audit log on every admit operation** — every admit-class call hits a tamper-evident append-only log: sender identity, operation, target, timestamp (local + arrival), partition-state flag. Shipped off-host to the hub site in near-real-time. | ~1 day (management-server sidecar or mgmt-plane middleware) | sidecar + existing observability pipeline |
| C2 | **Primary Ops dashboard in Grafana** — single view of all admits across all sites in the last 24h, with `partition-pending` tag, site, operator identity, target. Freshness indicator per site. | ~0.5 day | Grafana against C1 log |
| C3 | **IOC alert rules on admit patterns** — fire on: | ~1 day (rules + testing) | Alertmanager |
| | • Burst: >3 admits from one site in 10 min where baseline is <2/week | | |
| | • Off-hours: admits between local 22:00–06:00 (configurable per-site) | | |
| | • Novel operator: identity that's never issued an admit before | | |
| | • Partition-pending: any admit made during an observed partition (auto-tagged) | | |
| C4 | **Primary Ops watch SOP** — documented workflow: Primary Ops must explicitly acknowledge every `partition-pending` admit within 4h of reconnect or the admit is auto-revoked. Ack button on dashboard. Auto-revoke job is a cron that queries C1 log + mgmt API. | ~1 day (doc + job) | `docs/operations/primary-ops-watch.md` + cron job |
| C5 | **Rate limit on admits per site** — simple token bucket on setup-key registration inside the management server (designed, not built). Default: N admits per rolling 24h period, configurable per-site. During partition, the rate limit still applies, which bounds damage. | ~0.5 day | management server |

**Total Phase 2 effort: ~4 days. No Netbird source modifications.** These layers together provide detection + bounded damage + time to respond.

### 2.2 Deeper layers (Phase 3+)

| # | Control | Notes | Gate |
|---|---|---|---|
| C6 | **Post-admit quarantine ACL group** — new peers enter a scoped ACL group (site-local resources only) for N hours. Graduate to full-privilege group only after quarantine period AND Primary Ops acknowledgment. Enforced at Netbird policy layer. | ~1 week: quarantine group definition + scheduled graduation job + testing | Phase 3 |
| C7 | **Out-of-band admit notification** — every admit emits a signed notification over an independent channel. Options (priority order): (a) a separate WireGuard mesh on different VPS infrastructure for admin-only traffic; (b) cellular SMS via hardened gateway; (c) a secondary satellite terminal. Adversary must compromise both channels. | ~1–2 weeks including operator-side receiver + integration | Phase 3 |
| C8 | **Two-person rule for admits** — admit API requires co-signature from two operator identity keys on separate hardware (YubiKeys). The management server verifies both sigs before completing the registration. | ~2 weeks including key distribution + operator training | Phase 3 |
| C9 | **Capability ceiling (hub-issued envelopes)** — hub pre-issues bounded admit capabilities per site ("max N admits of class X, expiring in M hours"). Consumed at registration inside the management server. During partition, sites have only what hub issued in advance — bounded damage even if all other layers fail. | ~2 weeks: envelope schema, issuance flow, consumption + expiry logic | Phase 3 |
| C10 | **Behavioral anomaly detection refinement** — replace rule-based C3 with a baseline-trained detector that adapts per-site normal. | ~2 weeks | Phase 4 |

---

## 3. Universal NTP hygiene — Netbird-plane mechanics

**Policy framing and cross-plane scope live in [`universal-ntp-hygiene.md`](universal-ntp-hygiene.md). This section covers only the Netbird-plane-specific mechanics.** For the `ntp-health-agent` daemon (source of truth), cross-site peer attestation, per-sample Zenoh attestation, and the full enforcement matrix across planes, see that doc.

### 3.1 Netbird-plane enforcement points

Derived from the universal policy. Three enforcement sites in the Netbird plane specifically:

| Site | Mechanism | Reads from | Critical path? |
|---|---|---|---|
| **Registration clock check inside management** | On a setup-key registration, the management server refuses the registration when the local NTP verdict fails the check; a re-login by an existing peer is not gated. Designed, not built: management ingress has no clock gate | Local `ntp-health-agent` via Unix socket (`/run/ntp-health.sock`) | **Yes, once built — load-bearing for LWW correctness.** It is the one place where time-hygiene enforcement is designed to be directly load-bearing for cluster correctness. |
| **Netbird enrollment pre-hook** | Setup-key activation validates the peer's attestation payload + RTT probe before completing enrollment | Peer-provided attestation block + server time | Yes — gates joining |
| **Role-demote background job** | Cron or event listener; moves peers between `role.publisher` and `role.subscriber` groups based on sustained NTP degradation | Prometheus (sustained degradation over a time window) | No — background, eventually consistent |

### 3.2 Role governance through Netbird groups

Two baseline Netbird groups:

- `role.publisher` — default for leaf peers. Policy allows reach to admin/control endpoints.
- `role.subscriber` — opt-in at enrollment OR demoted from publisher due to sustained NTP degradation. Policy denies reach to admin/control endpoints; allows data-plane receive.

Enforcement is by Netbird's native policy engine — no new code. Demotion is a mgmt API call that changes group membership; policy re-evaluation on peer sync (~30–60s) completes the enforcement path.

**Caveat to confirm in Phase 2:** Netbird's policy engine primarily gates data-plane traffic between peers. Whether it gates peer → mgmt-API writes based on group membership is an open question. If it doesn't, the registration check inside management (above) also needs to honor `role.subscriber` → deny. Verify during the Netbird-under-Spock 72h soak (probe §5.1).

### 3.3 What this section does NOT cover

- **The daemon design itself.** See [`universal-ntp-hygiene.md`](universal-ntp-hygiene.md) §4.
- **Cross-site peer attestation.** See [`universal-ntp-hygiene.md`](universal-ntp-hygiene.md) §5.
- **Zenoh data-plane enforcement.** See [`universal-ntp-hygiene.md`](universal-ntp-hygiene.md) §6 + `zenoh-cds-data-conditioning.md`.
- **The source-of-truth architecture** (why daemon vs Prometheus for each consumer). See [`universal-ntp-hygiene.md`](universal-ntp-hygiene.md) §3.
- **Fail-closed semantics.** See [`universal-ntp-hygiene.md`](universal-ntp-hygiene.md) §7.

---

## 4. Integration with HA implementation plan

### 4.1 Additions to Gate 2

Add to the validation plan debt register as part of debt items 4 (observability) and 5 (backup):

- **NTP rapid wins N1–N5** (§1.1)
- **Coincident-infiltration rapid wins C1–C5** (§2.1)
- **Universal NTP hygiene minimum** (§3.4 — enrollment pre-hook + publisher/subscriber role flag)

All are Phase 1–2 scope. Cumulative effort: ~10 days across one engineer. Most are Ansible + Prometheus + Grafana + simple sidecar work — no new custom services, no Netbird source modifications.

Gate 2 entry criteria updated: these must be live on the EC2 pair and fire-tested alongside the observability alert suite.

### 4.2 Phase 3+ items

- **NTP deeper layers N6–N10** (§1.2), including GPS hardware procurement per `docs/project/procurement-ntp-gps.md`.
- **Coincident-infiltration deeper layers C6–C10** (§2.2), staged across Phases 3–4.
- **Universal NTP hygiene full flow** (§3.4 — publisher demotion job, ongoing telemetry integration).

### 4.3 What does NOT change

- The greenfield plan in `ha-multi-site.md` §12 is unchanged.
- The phase gates are unchanged in structure; Gate 2 just gains specific additional criteria.
- Approach B (multi-master active-active) remains the committed design.
- Approach C (hub-and-spokes admit split) remains documented in [`hub-spoke-admit-split.md`](hub-spoke-admit-split.md) as a deferred alternative.

---

## 5. References

- `docs/design/ha-multi-site.md` — committed multi-master design
- `docs/design/ha-multi-site-validation.md` — validation plan, tech-debt register, gates
- `docs/design/federation-security.md` — federation-specific threat model + roadmap
- [`docs/design/hub-spoke-admit-split.md`](hub-spoke-admit-split.md) — deferred alternative architecture
- `docs/project/procurement-ntp-gps.md` — hardware procurement request
- [NTS — Network Time Security (RFC 8915)](https://datatracker.ietf.org/doc/html/rfc8915)
- [chrony documentation](https://chrony-project.org/doc/)
- [`chrony_exporter`](https://github.com/superq/chrony_exporter) — Prometheus metrics
