# Control-plane storm resilience

Canonical design for [ADR 1081](../adr/1081-control-plane-storm-resilience.md).
The failure class: a control-plane collapse caused not by a
bad link but by **client retry behavior under a bad link**. A
thundering herd we engineer out at four layers, ranked by leverage, plus a
config-level pair-count reduction (lazy connections). Presentation version
(shared plan document) mirrors this file; this file is canonical.

Evidence base: M-085
(storm baseline + flush insufficiency). Trigger-signal source: the
host-net-health band (relay-health-dashboard.md
§"Site network hosts").

## The loop

Loss burst on the site uplink → rathole tunnels drop (they front mgmt/signal)
→ signal streams die fleet-wide → peers retry offers + stream
re-registration at fixed cadence regardless of outcome → retries saturate the
uplink → saturation causes more loss. The loop is self-sustaining because retry pressure is
constant regardless of success rate. Weather starts it; behavior sustains it.

**Design principle: fix the behavior, not the weather.** A control plane
whose load *decays* when delivery fails cannot storm itself — on any link,
at any site, including the denied edge where bad links are the operating
condition.

## Layer 1 — client retry discipline (the cure)

Lands as fork patches under `ops/netbird/patches/` (fork deltas are patches,
never divergent branches).

- **Backoff with full jitter** on signal offer retries
  (`client/internal/peer/` handshake/conn retry paths) and on signal stream
  re-registration.
- **Circuit breaker per peer-pair**: after M consecutive `notfound`s stop
  blind-retrying; half-open on the mgmt network-map peer-online event (the
  positive signal that already exists), timer fallback. Event-driven re-dial
  replaces polling into the void.

| parameter | initial | rationale |
|---|---|---|
| offer retry backoff | base 1 s → cap 60 s, full jitter | snappy first retry; bounded worst case; reset on success |
| re-registration backoff | base 2 s → cap 120 s, full jitter | re-registration is heavier (TLS+gRPC); major storm byte source |
| breaker threshold M | 5 consecutive notfound | tolerates stream flaps; opens fast in a storm |
| half-open probe | peer-online event, else 5 min | event-driven primary; timer is the lost-event safety net |

**Gate:** bench mesh (nova-twin/CGNAT env) with `tc netem loss 15%` on the mgmt
path for 30 min — aggregate signal traffic decays to <5% of storm baseline
within 2 min of streams dying; full reconvergence ≤3 min after loss removal.
Measured from the host-net-health control-plane card. → `docs/measurements/`.

**Rollback:** revert patches, re-pin client. **Risk:** upstream drift on
patched files — keep each mechanism a separate minimal patch.

**Delivery reality (verified against code, 2026-07-15).** L1 only helps clients
running *our* netbird build. Two vectors reach controllable clients: (a) the
operator-laptop-onboarding playbook's patched-binary step (retired 2026-07-20 —
the playbook now installs goat-client, collapsing (a) into (b)), and (b) **goat-client**,
which embeds the netbird client from our fork (`client/embed`, `replace =>
USAREUR-AF/netbird`) — bump its fork dep + rebuild and every goat-client user gets
L1. **Stock netbird from `pkgs.netbird.io` does NOT get L1.** And the server
cannot make a stock client self-throttle: the signal client's `Send()`
(`shared/signal/client/grpc.go`) ignores the gRPC error code and just retries 4×
on a fixed schedule, then drops the message; a rate-limit error on the stream's
`receive()` is treated as a disconnect (`case err != nil: return err` →
`notifyDisconnected`) — which is why a sender limit disconnects a stock
client. So a "gentle retry-after hint" is a **dead end** on
stock netbird — honoring a hint requires patching the client, which is L1
itself. The server-side levers that DO reduce a stock-client storm without a
client change are the **negative cache** (sheds server load, safe) and
**shrinking the peer set** each client attempts — ACL scoping (separate track)
and lazy connections (item 5). Fewer pairs = smaller storm.

## Layer 2 — signal-server load shedding (containment)

The deployed signal plane (`signal-a`) is the **repo-owned `signal-server/`
tree** (from-scratch, "Not a Netbird fork" per its README; the storm's
`relay miss · notfound` anchor is `internal/server/exchange.go`) — so L2
lands as direct source edits there, built/tested by
`.github/workflows/signal-server.yml`, NOT as fork patches (a fork-side
signal patch would be dead code that never deploys). Env-flagged; protects
the plane from unpatched/hostile clients.

- **Per-sender token bucket** (keyed on sender peer key) with explicit,
  geometrically growing retry-after.
- **Negative cache**: recipient stream absent → cache "offline", reject
  follow-ups before reading the body (the storm's server cost was drinking
  payloads it could never deliver).

| parameter | initial | rationale |
|---|---|---|
| sender bucket | 30 offers/min, burst 60 | order of magnitude above healthy per-peer peak |
| retry-after growth | 5 s → 80 s, ×2 per breach | compounds with L1 backoff; caps below breaker probe |
| negative-cache TTL | 20 s, purged on re-register | returning peer waits ≤20 s; purge makes the common case instant |

Flags: `SIGNAL_SENDER_RATE_LIMIT` (0=off), `SIGNAL_NEGATIVE_CACHE_TTL` (0=off).

**Deployment rule:** keep
`SIGNAL_SENDER_RATE_LIMIT` at **0 until L1 client backoff is fleet-deployed** —
an unpatched client legitimately exceeds a low per-sender rate and gets
disconnected (`ResourceExhausted`). Enable ONLY the negative cache before L1;
it does ~90% of the shed and never disconnects a sender.

**Live-fire:** server processed-msgs −93%, notfound −92%, CPU ~3%, zero peer regression — but link ingress ROSE against unpatched clients (fast rejection → faster client retry). Proves L2 = containment, L1 = cure; both required. Bench gate still required for the controlled numbers below.

**Gate:** bench storm replay (90 synthetic peers, recorded cadence, absent
recipients): ingress byte-rate drops ≥95% vs unshedded, while a healthy
control pair completes offers un-throttled; plus a cold-start mass-reconnect
run (Thor reboot shape) with zero healthy-client throttling.

**Rollback:** flags to 0. **Risk:** mis-tuned limits throttling legitimate
mass reconnect — burst allowance + purge-on-register are sized for that; the
cold-start run gates it.

## Layer 3 — site-host egress survivability (terrain)

Mgmt-host baseline (ansible), Thor hosts first, behind
`egress_shaping_enabled`. Host-local egress shaping only — the
[ADR 1033](../adr/1033-zenoh-qos-prioritization-reserved-mission-slice.md) "never DSCP for
fabric classes" rule is untouched; this is a different layer and the ADR
states the boundary.

| parameter | initial | rationale |
|---|---|---|
| `cake` bandwidth cap | 90% of measured line rate | keep the queue on the Thor where cake manages it, not the modem; cap follows the host-net-health measured envelope, floored conservatively |
| priority tiers | ctl (rathole/mgmt/signal) > fabric > bulk | control plane is small; strict priority costs bulk ~nothing |
| rathole client retry | 1 s flat → 5 s flat (rathole v0.5.0 config has no exponential backoff) | ends the 1 s hammer into ghost sessions (observed EOF loop); pairs with the shorter heartbeat below |
| rathole heartbeat_timeout | 40 s → 20 s | halves ghost-session lifetime |
| tcp_congestion_control | bbr (Thor hosts) | cubic collapses at ~10% loss; BBR degrades ~linearly |

**Canary result:** BBR is UNAVAILABLE on the JetPack 7 Tegra kernel (`reno cubic` only; no `tcp_bbr`) — BBR is **x86-mgmt-host-only**. And a fixed cake cap is unfit for a highly variable uplink: cap high → cake no-ops during degradation; cap low → throttles good times. cake-on-Thor is **deferred pending an autorate mechanism**; the Thor-side L3 win is the already-live rathole hardening. cake+BBR deploy cleanly on x86 mgmt hosts.

**Gate:** canary Thor with `tc netem loss 10% delay 20ms` — tunnel uptime
≥99%/1 h (storm-day baseline: repeated drops), RTT inflation under saturating
bulk ≤2× idle (observed ~6×), signal round-trip completing during the bulk
transfer.

**Rollback:** toggle off; qdisc + sysctl revert idempotently. **Risk:** wrong
cap on a moving link — cap derives from the rolling measured
envelope.

## Layer 4 — operator storm brake (the safety net)

`goat admin storm brake` (goat-cli) + Thor-side guardrail. Possible because
the storm is now measured (host-net-health).

Steps: **flush** (restart `signal-a` — drops zombie streams so registration
state converges; M-085 proves this is insufficient *alone*, which is why the
brake is layer 4, not layer 1) · **tighten** (halve L2 limits, bounded
window) · **shed** (pause declared non-essential publishers via existing
service toggles) · **announce** (one `OperatorActionV1` through the audited
goat-admin chokepoint + degraded-mode notice on the dashboards' host band).

Triggers (from the host-net band):

| signal | source field | engage |
|---|---|---|
| signaling futility | `control.signal_notfound_60s / signal_msgs_60s` | ≥0.5 with ≥100 msgs/min |
| uplink loss | `wan.loss_pct` | ≥5% sustained 10 min |
| tunnel flap | `tunnels.errs_15m` | ≥30 (corroborating, not required) |

Phase 1 **manual** (command + runbook + dashboard tells you when). Phase 2
auto-engage is separately gated: hysteresis (disengage after 15 quiet min),
hard once-per-hour cap — a flapping auto-brake is worse than none. The
futility ratio distinguishes "many messages, delivered" (healthy reconnect)
from "many, undeliverable" (storm) — M-085 verified this separation.

**Gate:** live-fire on ZAGROS in a quiet window — seeded synthetic storm, band
goes amber/red, fire the brake, futility <0.1 within 5 min, action in the
operator-action audit trail.

## Item 5 — lazy connections (pair-count reduction; config, not fork)

NetBird lazy-connection mode defers P2P setup until a pair has traffic. With lazy
connections off and a permissive ACL, every peer holds mostly-idle connections to dozens of
others, and every idle pair's flap costs signaling. Lazy mode shrinks the
signaling-active pair set to "the pairs that talk" — potentially a larger
steady-state reduction than L1 itself, and it compounds with the concurrent
mesh-ACL scoping work (both shrink the active pair set; coordinate, don't
collide — ACL is a separate track/session).

**Rejected sibling idea (for the record):** lengthening WireGuard/rosenpass
rekey lifetimes to reduce signaling. Rekey is in-band between peers and never
touches signal; the constants are protocol-fixed forward-secrecy bounds.
Keepalive lengthening runs the wrong way (NAT expiry → more reconnects →
more signaling).

**Gate (bench, nova-twin):** pair-count + signal-rate before/after; first-packet
latency on fabric paths (zenoh) within budget; rosenpass-permissive and
ACL-group interplay clean over a 24 h soak. Adopt per-goatnet only on pass.

## Defense in depth

| scenario | L1 | L2 | L3 | L4 |
|---|---|---|---|---|
| loss burst, patched fleet | load decays in seconds | idle | tunnels ride it out | idle |
| unpatched/hostile clients | — | rejected cheaply | ctl still prioritized | available |
| sustained degradation (days) | no storm forms | server cool | capacity is what it is | planned degraded mode |
| novel storm shape | helps if retry-driven | helps if send-driven | ctl keeps priority | operator sees it + flushes |

## Sequencing

| order | work | lands in | depends on |
|---|---|---|---|
| 1 | backoff+jitter, circuit breaker (two minimal patches) | `ops/netbird/patches/` → client pin | — |
| 2 | signal rate limit + negative cache (env-flagged) | fork signal svc → mgmt compose pin | — |
| 3 | cake + tiers + rathole/BBR | mgmt-host baseline (+ rathole conf) | host-net-health rate measurement (live) |
| 4 | storm brake (manual) | goat-cli + Thor guardrail | L2 flags; triggers (live) |
| 5 | lazy-connections eval | client config + bench report | nova-twin bench; coordinates with mesh-ACL track |

Order is leverage + independence: 1 removes the disease, 2 contains what 1
misses, 3 hardens the ground, 4 is the safety net (M-085: a brake without
1–3 schedules the next collapse), 5 shrinks the whole problem.

## Out of scope (deliberate)

Cloud/standby signal tier (separate architectural discussion); fabric-layer
QoS (ADR 1033 governs); per-packet relay service time
([relay-packet-service-time.md](relay-packet-service-time.md), stays
designed-not-built).

## Status

Tracks per layer; every gate's result lands in `docs/measurements/` — a
layer is done at its gate, not at merge. Implementation tracking:
`docs/project/in-flight.md`.
