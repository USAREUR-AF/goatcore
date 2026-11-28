# NTP Strategy: Sources, Blending, Node Timekeeping

Three-tier strategy for how time enters, propagates through, and is maintained inside the mesh.

> **⚠ Re-roled by [ADR 1037](../adr/1037-fabric-internal-time-synchronization-first.md) (2026-06-09).** These three tiers are no longer the *foundation* of fabric time — they are the **anchor-supply layer** beneath it. Fabric time is now founded on **internal peer-to-peer synchronization** (every participant mutually synced, no external reference required); GNSS/NTS/peer-NTS acquired here are **opportunistic anchors** that pull the synchronized ensemble toward UTC when reachable. Read Tiers 1–2 as "how anchors are acquired and blended," and Tier 3 (per-node self-clock holdover) as superseded for the *fabric-consistency* purpose: an islanded node must stay synced to its **peers** (internal sync), not just extrapolate its own crystal in isolation — independent per-node holdover lets participants drift apart, which is the specific failure ADR 1037 fixes. The drift model here remains useful as a per-node *anchor-staleness / UTC-confidence* estimator, not as the islanded-consistency mechanism.

**Status:** design. Companion to [`universal-ntp-hygiene.md`](universal-ntp-hygiene.md) (policy + enforcement) and `../project/procurement-ntp-gps.md` (BOM). Tier 1 & 2 are Phase 2-3 deliverables; Tier 3 adaptive drift model is Phase 3. **Viewing** the resulting per-host clock health as a collective, on-bus surface (alongside network health) is [`environment-health-surface.md`](environment-health-surface.md) — the `time` domain rolls each host's `ops/time/attest` verdict up to `ops/time/health/<site>` for `goat health`/panoscope/stethoscope ([ADR 1036](../adr/1036-environment-health-surface.md), F-184).

## 1. The three tiers

| Tier | Question | Scope |
|---|---|---|
| **1. Acquire** | Which external time sources do we pull in, and why? | Per-site hardware + protocol decisions |
| **2. Blend** | How do those sources combine into one authoritative site reference, and how do they cross the ZTOL (zero-trust overlay) boundary safely? | Per-site chrony config + ingestion topology |
| **3. Self-clock** | How does any node maintain trustworthy time when it can't reach any Tier 2 source? | Per-node drift model + confidence reporting |

Each tier has a failure mode that the next tier absorbs. Tier 1 sources can go dark; Tier 2 degrades gracefully. Tier 2 can be unreachable; Tier 3 carries the node across the gap with honest confidence bounds.

---

## 2. Tier 1 — Source acquisition

**Principle:** maximum independent sources within procurement, engineering, and maintenance constraints. *Independent* means different physics, different adversary kill-chain, different failure mode.

### 2.1 Portfolio decision matrix

| Source | Typical accuracy | GNSS-indep? | Per-site cost | Ongoing burden | Decision |
|---|---|---|---|---|---|
| Multi-GNSS receiver (u-blox ZED-F9T: GPS+Galileo+GLONASS+BeiDou) + PPS | <20 ns | partial — resists GPS-only spoofing | ~$475 GNSS bundle (A1–A5 in `procurement-ntp-gps.md`); ~$620 with enclosure + site YubiKeys | annual antenna check | **YES — primary Tier 1 physical reference (stratum 0 via PPS refclock)** |
| TimeMachines TM1000A (multi-GNSS Ethernet NTP server) | ~1 µs (stratum 1 downstream) | partial — multi-GNSS | ~$325 per unit | zero (turnkey appliance) | **YES — committed parallel integration path; enters chrony as `server` line (stratum 1), not `refclock`; no raw PPS or GNSS data exposed to host** |
| Canonical-LAN switch NTP/NTS server | sub-ms RTT same-LAN; stratum depends on switch's own backing | conditional — depends on switch's own source | included with switch procurement | low (firmware + monitoring) | **YES as portfolio option, NEVER as dependency** — see [ADR 0834](../adr/0834-per-site-nts-server-tier.md) "Switch as portfolio option". Open verifications: (1) does the platform speak NTS or only plain NTP / md5-auth? (2) is the stratum-1 claim backed by a switch-side refclock or chained from a public NTS upstream (in which case stratum-2)? Configured as `server <switch-ip> nts noselect prefer` so it appears in `chronyc sources` without ever being chosen unless explicitly promoted. Plain-NTP fallback is acceptable per [`universal-ntp-hygiene.md`](universal-ntp-hygiene.md) §3.2 LAN-trusted carve-out (the switch is on the same trust substrate that mediates VRRP failover per `canonical-lan.md`). |
| Public NTS (Cloudflare `time.cloudflare.com`, Google `time.google.com`) via WAN | 1–10 ms | full | free | zero | **YES — authenticated WAN source** |
| Internal peer NTS (other mgmt hosts in cluster) | sub-ms intra-LAN / tens of ms WAN | full | free (uses existing mesh) | zero | **YES — intra-cluster cross-check** |
| TCXO holdover (inherent to ZED-F9T and Orin reference oscillators) | 0.5–2 ppm drift (tens of µs/hour) | yes (oscillator, not receiver) | included with hardware | zero | **YES — included free, TCXO-grade** |
| OCXO holdover (separate GPSDO module — **not** in current BOM) | 0.01–0.1 ppm drift (sub-µs/hour) | yes | ~$200–600 per site if added | low | **DEFER** — reconsider alongside CSAC if TCXO-grade holdover proves insufficient in operational data |
| CSAC (Microchip SA.65) | ~1 µs/day | yes | ~$1,500 | 10-yr lifetime, battery swap | **DEFER** — reconsider if operational GNSS-denial episodes exceed TCXO-grade holdover (or OCXO-grade, if the add-on above is deployed first) |
| Rubidium (SA.22c class) | <100 ns/day | yes | ~$800 | periodic lamp replacement | **NO** — CSAC wins $/size at that tier |
| Iridium STL (Satelles Stratum-1 over Iridium) | ~200 ns | full (LEO sat, non-GNSS) | ~$2k hardware + subscription | subscription mgmt | **DEFER** — evaluate Phase 4 as GNSS-denial hedge |
| eLoran | 50–100 ns | full | — | — | **NO** — no coverage at our sites |
| Shortwave time (WWV, DCF77, MSF, JJY) | ~1 ms | full | ~$100 receiver | antenna siting | **NO** — no usable transmitters |
| PTP/IEEE 1588 intra-LAN | sub-µs | N/A (same LAN only) | switch-dependent | — | **NO Phase 2-3** — chrony-over-mesh is sufficient at current precision targets |
| Cellular NITZ | 1–2 s | full | — | — | **NO** — too coarse |

**Phase 2-3 per-site portfolio (decided):**
1. u-blox ZED-F9T with PPS as physical stratum-0 (primary path — raw PPS, GNSS-level precision, spoof detection).
2. TimeMachines TM1000A as committed parallel stratum-1 Ethernet reference (whichever disciplines more stably stays forward, the other stays as lab reference).
3. Two public NTS providers (Cloudflare + Google) as WAN reference, operator-diverse.
4. Peer NTS from other cluster mgmt hosts as intra-cluster cross-check.
5. TCXO holdover inherent to the GNSS board and Orin reference — covers minutes-to-hours of GNSS loss, not days. Real OCXO-grade holdover is a separate procurement and is deferred with CSAC.
6. Canonical-LAN switch NTP/NTS server as portfolio option — `noselect prefer` until verified per row above; never a dependency.

**Deferred (tracked in open questions):** OCXO add-on module, CSAC, Iridium STL. All three are hedges against sustained GNSS-denied operation, ordered by $/benefit. We ship without them and add if operational experience justifies the cost.

### 2.2 Why this portfolio is "enough independent"

- **Spoof**: multi-GNSS + public NTS + peer NTS disagree under a GPS-only spoof. The consensus layer in [`universal-ntp-hygiene.md`](universal-ntp-hygiene.md) §5 catches it.
- **Jam (GNSS-wide)**: WAN-available sites fall back to NTS; WAN-denied sites fall to TCXO holdover then Tier 3 self-clock.
- **WAN cut**: GNSS continues. NTS goes away. Chrony selects GNSS.
- **Simultaneous GNSS + WAN denial**: TCXO holdover (minutes to hours at 0.5–2 ppm) then Tier 3 drift model takes over explicit uncertainty reporting.
- **Mesh partition within a site**: peer NTS unaffected (local); public NTS affected only if the partition severs WAN for that subnet.

A simultaneous GNSS-wide jam + sustained WAN denial crosses the TCXO-holdover threshold within hours, so the Tier 3 self-clock + adaptive-drift path matters in practice, not just as a theoretical tail. If operational data shows this scenario repeatedly, the Phase 4 uplift path is OCXO module first (cheapest, hours-to-days holdover), then CSAC (days-plus), then Iridium STL (independent time source, not just better oscillator).

---

## 3. Tier 2 — Local blend and ZTOL ingestion

### 3.1 Per-site chrony config

chrony is the blending engine; we do **not** reinvent source selection. chrony's internal select/combine weights sources by reported precision and dispersion.

Per-site chrony source priority (configured via stratum + `prefer`/`noselect` flags):

```
refclock   SOCK /var/run/chrony.ttyACM0.sock   prefer  # GNSS PPS, stratum 0 (F9T)
server     <tm1000a-ip>        iburst                  # TM1000A stratum-1 LAN reference
server     <switch-lan-ip>     nts noselect prefer     # switch NTS — portfolio option per ADR 0834, available not depended-on
server     <peer-mgmt-host-1>  iburst  nts             # intra-cluster peer
server     <peer-mgmt-host-2>  iburst  nts             # intra-cluster peer
server     nts.netnod.se       iburst  nts             # WAN NTS
server     time.google.com     iburst  nts             # WAN NTS
local      stratum 10                                  # orphan mode last-resort

# Server-side fan-out (per ADR 0834) — gated by chrony_serve_nts: true
ntsserverkey  /etc/chrony/server-key.pem               # offline-CA leaf via cert-sign --eku serverAuth
ntsservercert /etc/chrony/server-cert.pem
ntsdumpdir    /var/lib/chrony
ntsport       4460                                     # NTS-KE (RFC 8915)
allow         192.168.1.0/24                           # canonical LAN (sub-ms RTT — primary client path)
allow         198.18.0.0/24                            # wg-cp0 (cross-site, sloppy precision; opt-out per-site)
```

**Note:** TM1000A is a `server` line (stratum 1), not a `refclock`. It sits between the F9T PPS (stratum 0) and the intra-cluster peers. When both F9T and TM1000A are present, chrony selects F9T as the primary (prefer flag + lower stratum); TM1000A provides an independent cross-check and a fallback if F9T loses fix. Sites with only TM1000A (no F9T installed) bump TM1000A to their best available reference.

Ordering: GNSS > intra-cluster peer > WAN NTS > orphan. Chrony picks the best available; operator doesn't micromanage.

**Orphan mode** (`local stratum 10`) lets a single host with no reachable sources still serve time to downstream peers, marked stratum 10 so peers know it's a last-resort reference. Orphan is *always* configured — cheap insurance.

### 3.1.1 Cold-boot pre-sync (the NTS chicken-and-egg)

Tegra Orin carriers ship without RTC batteries — every cold boot starts with the system clock at epoch 0 (`1970-01-01T00:00:00Z`). Steady-state operation depends on chrony catching up within seconds and stepping the clock to truth.

**The fail-closed footgun:** every `server` line in §3.1 carries the `nts` keyword (NTS-only ZTOL ingestion is the trust posture). NTS-KE (RFC 8915) is a TLS-1.3 handshake; TLS validates the server cert against the local clock; a clock at epoch 0 means every NTS-KE handshake fails with "certificate not yet valid"; chrony stays in stratum-10 orphan mode forever; time-bound TLS-using services (image-registry mirrors, openobserve, OpenObserve-OTel pair, Hydra, Kratos, signal-server, etc.) crash-loop on cert-time validation. A box that power-cycles never recovers.

**The escape valve:** `chrony-presync.service` — a systemd oneshot installed by the [`ntp-hygiene` role](../../ops/ansible/roles/ntp-hygiene/) that runs ONCE at boot, BEFORE `chrony.service` + `docker.service` + `containerd.service` (`Before=` deps), gated on `ConditionPathExists=!/run/chrony-presync.done` + a `date -u +%Y < chrony_presync_min_year` check (default `2025`). When the clock is implausibly old, runs `chronyd -q "pool <pool> iburst"` against plain (non-NTS) public NTP pools (default `2.pool.ntp.org`, `pool.ntp.org`) to step the clock close to truth, then exits. `chrony.service` then starts and engages the NTS-secured steady-state chain in §3.1.

**Trust posture invariant preserved:** the steady-state chrony.conf still has only NTS sources. The pre-sync is a one-shot bootstrap — no plain-NTP source on the trust path during steady-state. The moment chrony.service starts, NTS is the only authority. On a healthy warm reboot (RTC fine, year ≥ 2025), the pre-sync no-ops in milliseconds.

**Relocation / late-WAN self-heal:** the original pre-sync assumed the network was reachable within a one-shot boot-time budget. A host powered on *before* its uplink is up (relocation, cold travel-router/satellite WAN) burned the budget with the network down, `exit 0`'d, and the unconditional `.done` sentinel then blocked any retry — clock stuck at 1970 indefinitely. Three changes close that, with the NTS-only steady-state posture unchanged:

- **`chrony-presync-retry.timer`** — a monotonic timer (unaffected by the bad clock it fixes) re-fires `chrony-presync.service` every `chrony_presync_retry_interval_sec` (60s) until it succeeds, so the host self-heals whenever the WAN comes up, with no operator action.
- **`.done` written on genuine success only** (moved out of the unit's `ExecStartPost` into the script). A failed pass exits non-zero → unit `failed` → the timer re-fires it. `SuccessExitStatus=0`.
- **DHCP default gateway tried as a local non-NTS source first** (`chrony_presync_use_gateway`, default true) — most edge/travel routers serve NTP and carry plausible time even with no public internet, so a relocated host bootstraps offline. Public pools follow.

The script is idempotent across both invocation paths: at boot (chrony not yet started) it just runs `chronyd -q`; when the timer fires later (chrony already up at a bad clock) it cycles chrony around the one-shot step.

Tracked as F-096; the relocation-class self-heal hardening is F-165.

### 3.2 Getting external NTS across the ZTOL boundary

The mesh is zero-trust by design: every node authenticates every peer, and WAN exposure is minimized. External NTS is pulled in without widening that exposure.

Options (preferred order):

1. **Direct authenticated NTS from mgmt hosts (default).**
   Each mgmt host talks NTS to public providers. NTS is TLS + authenticated NTP — no new trust assumption. Egress UDP 123 to known hostnames; TCP 4460 for NTS-KE. This is the default because: (a) it adds no infrastructure, (b) sources are cryptographically authenticated, (c) any manipulation requires breaking NTS TLS.

2. **Relay-side NTS bridge** (for sites where ZTOL policy forbids arbitrary mgmt-host WAN egress).
   The existing VPS relays already terminate WAN TLS for Netbird management. Extend them as intra-mesh NTS servers: relays discipline themselves from public NTS upstream; mesh peers consume from the relays. One external ingress point per region, identity-gated via the mesh.

3. **One-way hardware diode** (not warranted at current scale — listed for completeness; would only matter if a site runs fully air-gapped with no WAN and still needs external time).

**Recommended deployment:**
- Lab and cloud sites: Option 1 direct.
- Forward sites: Option 1 direct when WAN is available; relays already provide Option 2 as a fallback path by virtue of running chrony.

**What we don't do:**
- No unauthenticated public NTP. NTS only. An unauthenticated public time source is an external trust dependency we refuse to take.
- No NTP over the TAK federation bridge or other partner links — those links have their own trust envelopes; time flows inside our envelope only.

**LAN-trusted carve-out (added 2026-05-06).** The "NTS only" rule above governs *external* sources crossing the ZTOL boundary. Plain-NTP from devices on the same canonical LAN as the mgmt-host — specifically the switch under `canonical-lan.md`, which already mediates VRRP failover and inter-VLAN routing for the site — sits inside the LAN's existing trust substrate, not on the external trust boundary. If the switch platform speaks NTS (RFC 8915), prefer NTS. If only plain NTPv4 with symmetric-key authentication is available, that is acceptable as a **portfolio option** per [ADR 0834](../adr/0834-per-site-nts-server-tier.md) — never as a dependency, never promoted above NTS-authenticated sources, always carrying `noselect prefer` until explicitly trusted by the operator. The carve-out is narrow: same-LAN, same-trust-substrate-as-VRRP, never-required.

### 3.3 Site-reference authority hierarchy

For any mgmt host, effective source authority at time T:

| Priority | Source | Condition |
|---|---|---|
| 1 | Local GNSS PPS | receiver has 3D fix AND multi-GNSS agreement (resists GPS-only spoof) |
| 2 | Intra-cluster peer NTS | peer's own verdict is `healthy` per [`universal-ntp-hygiene.md`](universal-ntp-hygiene.md) §4 |
| 3 | WAN NTS (direct or via relay) | NTS auth passes, RTT sane |
| 4 | Orphan mode (`local stratum 10`) | all of the above unreachable |

Chrony implements this via its built-in selection; the host's `ntp-health-agent` daemon (§4 of [`universal-ntp-hygiene.md`](universal-ntp-hygiene.md)) reports the *actual* selected reference in its verdict so downstream consumers know which tier is active.

### 3.4 Server-tier fan-out (per ADR 0834)

Tier 1 and Tier 2 cover *upstream* source acquisition: where each mgmt-host gets its own time. They do not cover *downstream* fan-out: where every other goat-net peer (operator laptops, MFT nodes, sensors, future workloads) gets time from. Today (pre-ADR-0834) every peer falls through to public NTS over its own egress, which widens trust surface and undermines the "no public-internet-resident secure plane" invariant for time.

[ADR 0834](../adr/0834-per-site-nts-server-tier.md) adds an NTS *server* role on each mgmt-host, gated by per-site var `chrony_serve_nts: true`. Same chrony binary, same `ntp-hygiene` role; new directives (`ntsserverkey` / `ntsservercert` / `allow`) appear under the gate. The server tier serves whatever upstream chrony has selected — today public NTS (Orin = stratum-2; LAN clients = stratum-3), post-Block-12C with F9T or TM1000A landed (Orin = stratum-1; LAN clients = stratum-2). Architecture is identical pre- and post-GNSS; only the upstream source layer changes.

Cert chain: NTS-KE leaves are issued by [ADR 0407](../adr/0407-internal-x509-ca-for-service-tls.md)'s internal X.509 CA via `cert-sign --eku serverAuth`. Clients trust the offline-CA root via the offline-enrollment bundle's `tls_trust_anchor` field — same trust path as Traefik service TLS. No new `--kind` variant; rotation is the same operator ceremony.

Allow-list defaults to `192.168.1.0/24` (canonical LAN, sub-ms RTT, primary client path) + `198.18.0.0/24` (wg-cp0, cross-site, sloppy precision, opt-out per site). LAN-only fan-out is the recommended primary path; cross-site over wg-cp0 is best-effort when no nearer Orin reachable.

FIPS posture: internal-mesh, Option A — NTS TLS rides Ubuntu Pro FIPS OpenSSL 3.0.x with offline-CA Ed25519 root, same posture as Postgres / Ory / signal-server / Block 22J Traefik. Cross-boundary FIPS strictness lives at Block 19 strongSwan + AWS-LC, not at every internal service.

---

## 4. Tier 3 — Node self-clock with adaptive drift

### 4.1 Problem

A node (mgmt host, Orin, or mesh peer) has been out of contact with Tier 2 for non-trivial time — minutes, hours, days. It must still emit samples with honest confidence about its own clock, not blindly claim `healthy`.

### 4.2 State the daemon tracks

Extend the `ntp-health-agent` authoritative state ([`universal-ntp-hygiene.md`](universal-ntp-hygiene.md) §4.3) with a `drift_model` object:

```
drift_model {
  ppm_mean:              running estimate of this host's crystal drift (ppm)
  ppm_stddev:            running estimate of observation noise (ppm)
  last_sync_utc:         timestamp of last high-confidence source agreement
  seconds_since_sync:    now - last_sync_utc
  expected_skew_usec:    ppm_mean × seconds_since_sync          (point estimate)
  skew_uncertainty_usec: ppm_stddev × seconds_since_sync × k    (k = 2 for ~95% band)
  sample_count:          number of observations folded into the mean so far
  source_of_last_sync:   "gnss" | "peer_nts" | "wan_nts" | "orphan"
}
```

`expected_skew_usec` is the node's best guess at how far its clock has drifted since it last had a trustworthy reference. `skew_uncertainty_usec` is its honest uncertainty about that guess.

### 4.3 Update rule — exponential moving average (not Bayesian, same spirit)

Each time the node resyncs to a high-confidence source (Tier 2 priority 1–3), it has:
- `observed_offset` — chrony's reported correction in seconds
- `elapsed_since_last_sync` — seconds since the previous high-confidence sync
- `observed_ppm = observed_offset / elapsed_since_last_sync × 1e6`

Update via Welford-style online EMA (one-liners, no library needed):

```
α = 0.3   # smoothing factor; tune during Wave 3 drills
Δ = observed_ppm - ppm_mean
ppm_mean   ← ppm_mean + α × Δ
ppm_stddev ← sqrt((1-α) × ppm_stddev² + α × Δ²)
sample_count ← sample_count + 1
```

This converges toward the node's true hardware drift (which is a stable crystal property per unit). After ~10 observations the estimate is usable; after ~100 it's tight.

**This is not Bayesian, but captures the same idea:** each observation updates belief; belief tightens with data; the variance estimate gives honest uncertainty. If we later want actual posteriors over drift rate + aging, the data we're already logging supports that without re-architecture.

### 4.4 Bootstrap for fresh nodes

A new node has no history. Defaults before the first sync:

| Oscillator type | `ppm_mean` default | `ppm_stddev` default |
|---|---|---|
| Uncharacterized (PC crystal) | 0 | 25 (covers ±50 ppm range, commodity XO spec) |
| TCXO (common server / F9T board) | 0 | 2 |
| OCXO | 0 | 0.1 |
| CSAC (if present) | 0 | 0.001 |

The node declares its oscillator type in config at install. Sync observations replace the default as soon as data arrives.

### 4.5 `ntp_class` under self-clock

Until now, `ntp_class` ([`universal-ntp-hygiene.md`](universal-ntp-hygiene.md) §4.3) was binary-ish (`healthy/warning/degraded/unknown`) based on chrony state. Extend it with **predicted skew**:

| Condition | `ntp_class` |
|---|---|
| Tier 2 source currently disciplining the clock, chrony healthy | `healthy` |
| Self-clock, but `skew_uncertainty_usec` ≤ 100 µs | `healthy` |
| Self-clock, `skew_uncertainty_usec` ≤ 10 ms | `warning` |
| Self-clock, `skew_uncertainty_usec` ≤ 1 s | `degraded` |
| Self-clock, `skew_uncertainty_usec` > 1 s OR `sample_count` < 3 | `unknown` |

Thresholds are **defaults**, not prescriptions. High-precision topics (e.g. sensor fusion) set tighter thresholds per topic; coarse topics (chat, status) accept looser bounds. Per-topic override mechanism is Phase 3 — same place Option D (drop-on-degraded) is configured ([`zenoh-ntp-integration.md`](zenoh-ntp-integration.md) §5).

### 4.6 Distributing self-estimates

Each node publishes `drift_model` fields in its attestation heartbeat on `ops/time/attest/{host_id}` (extend the payload at [`universal-ntp-hygiene.md`](universal-ntp-hygiene.md) §4.3). Subscribers can reason:

> "mgmt-01 is 3 hours since last GNSS sync, drift 1.2 ppm ± 0.2 ppm, expected skew 13 ms ± 2 ms — still inside the tolerance for positional data but not for the tightest topics."

The mission layer or topic policy decides what "inside tolerance" means; the fabric just exposes the numbers honestly.

### 4.7 What Tier 3 is not

- Not a cryptographic freshness guarantee — that's the attestation signature + `producer_epoch` in the 5-byte sample attachment.
- Not a replacement for chrony — chrony still disciplines the clock when any source is available.
- Not a global consensus — each node's estimate is local; cross-site consensus remains the peer-attestation mechanism in [`universal-ntp-hygiene.md`](universal-ntp-hygiene.md) §5.
- Not a claim of absolute time correctness when self-clocking — it's a claim of *honest uncertainty*. The whole point is that a self-clocked node tells downstream consumers "I don't know better than this."

---

## 5. Interaction with adjacent docs

| This doc covers | Adjacent doc covers |
|---|---|
| What sources exist and which we acquire (§2) | `procurement-ntp-gps.md` — BOM, ship plan |
| How sources blend at a site and cross the ZTOL boundary (§3) | [`universal-ntp-hygiene.md`](universal-ntp-hygiene.md) §4 — daemon consumes the blend |
| How a disconnected node estimates its own drift (§4) | [`universal-ntp-hygiene.md`](universal-ntp-hygiene.md) §4.3 — daemon state extended with `drift_model` |
| Per-topic tolerance policy (§4.5) | [`zenoh-ntp-integration.md`](zenoh-ntp-integration.md) §5 — subscriber-side and router-side enforcement |
| Threat framing for GNSS spoof / WAN cut | [`mgmt-plane-threat-mitigations.md`](mgmt-plane-threat-mitigations.md) §1 |

---

## 6. Phasing

| Phase | Scope |
|---|---|
| **Phase 2 (Gate 2)** | Tier 2 chrony config + NTS ingestion path (Option 1 direct, Option 2 via relays as alternate). `ntp-health-agent` v1 reports current source only — no drift model yet. |
| **Phase 3** | Tier 1 multi-GNSS physically deployed per `procurement-ntp-gps.md`. `ntp-health-agent` v2 implements the Tier 3 drift model + attestation payload extension. Per-topic threshold config in Zenoh. |
| **Phase 4 (contingency)** | Uplift path (cheapest-first): OCXO GPSDO add-on → CSAC → Iridium STL. Triggered when operational GNSS-denial episodes at any site exceed the TCXO-grade holdover currently shipping in the BOM. |

---

## 7. Open questions

1. **α tuning.** The 0.3 smoothing factor is a starting point. Empirically tune during Wave 3 HA-validation by inducing sync losses and replaying observed drift. Document chosen value per oscillator class.
2. **Default thresholds in §4.5.** Current values (100 µs / 10 ms / 1 s) are reasonable placeholders. Validate against real mission-profile tolerances once §4.5 per-topic override is implemented; the defaults should cover chat/status, not the tightest mission tolerances.
3. **Orphan-mode vs. Tier 3 interplay.** Chrony orphan mode (`local stratum 10`) already synthesizes a reference from the local oscillator. Verify chrony's orphan selection does *not* suppress the daemon's drift-model reporting — daemon should treat orphan as "no high-confidence source" and continue accumulating `seconds_since_sync`.
4. **Sustained GNSS denial as trigger for CSAC procurement.** Define the operational signal that moves CSAC from Phase 4 "contingency" to committed Phase 3 scope. Candidate: any site logging >N hours of self-clock in `ntp_class=degraded` in a single month.

---

## 8. Alternatives watchlist

Time-daemon alternatives we are tracking but not adopting today. Listed so future-us doesn't have to rediscover the comparison.

### 8.1 ntpd-rs (Project Pendulum)

- **What:** Memory-safe Rust NTP implementation with first-class NTS, sponsored by Sovereign Tech Fund / ISRG. Repo: <https://github.com/pendulum-project/ntpd-rs>.
- **Maturity — be precise about which role.** ntpd-rs is **proven in production** for the role it was designed for: NTS-first server and client without refclocks. Large public NTS operators run it in production. It is **less mature than chrony** specifically for **refclock-blending stratum-1 deployments** (GNSS PPS + GPSDO appliances + multi-source local fan-out). That's chrony's stronghold and Pendulum has not prioritized matching it. Both statements are true at once; collapsing them into "unproven" or "feature-poor" is wrong.
- **Why it's interesting here:** Aligns with the 2026-04-24 tech-strategy direction ("prefer Rust for data-services work"). NTS-first design with PQ-NTS and NTSv2-extension work landing earlier than in chrony. Memory-safe daemon at a layer where we currently run C — directly relevant for hardened / boundary-node roles where C-network-parser CVE history is a real threat-model concern.
- **Why we are not switching now:**
  1. Tier 1 of *this* strategy is **blended refclocks** (u-blox ZED-F9T + PPS, TM1000A) per §2 — the exact role where ntpd-rs is least mature. We need chrony's refclock + PPS path, not an NTS-only path.
  2. The verifier in `ops/time-hygiene/internal/chrony/client.go` speaks chronyd's control protocol. Swapping daemon = rewriting the verifier and re-validating `ntp-health-agent` and every reader of its verdict.
  3. Deployed and stable at sub-millisecond offsets disciplined against public NTS. No forcing function.
- **Where ntpd-rs could land *partially* without a full swap:**
  - Boundary nodes / relay-side NTS server (§3.2 Option 2) — no refclocks involved, NTS server is exactly the role ntpd-rs is proven in. Potential narrow adoption ahead of any Tier-2 mgmt-host swap.
- **Trigger conditions to revisit broader adoption:**
  - chrony hits an NTS feature gap we need (e.g., PQ-NTS lands in ntpd-rs first and we need it).
  - Memory-safety posture for hardened / boundary-node roles becomes load-bearing in a compliance or threat-model review.
  - Pendulum's refclock + PPS support reaches feature parity with chrony for ZED-F9T-class hardware.
- **Owner of next review:** whoever drafts the Phase 4 Tier-1 uplift work (OCXO / CSAC / Iridium STL) — natural moment to re-examine the daemon at the same time. Boundary-node-only adoption can be considered earlier when the FIPS-PQC boundary-node design firms up.

---

## 9. References

- [`universal-ntp-hygiene.md`](universal-ntp-hygiene.md) — policy, daemon, attestation, enforcement
- [`zenoh-ntp-integration.md`](zenoh-ntp-integration.md) — Zenoh-side mechanics
- [`mgmt-plane-threat-mitigations.md`](mgmt-plane-threat-mitigations.md) §1 — GNSS / NTP threat model
- `../project/procurement-ntp-gps.md` — hardware BOM and ship plan
- [NTS — RFC 8915](https://datatracker.ietf.org/doc/html/rfc8915)
- chrony documentation — source selection, orphan mode, refclock
