# Environment health surface — collective network + time health, one contract, three altitudes

> **60-second summary:** "How healthy is the *environment* we're operating in?" is a question with
> more than one domain — **network/fabric** load and **time/clock** discipline today; GNSS, power,
> thermal later. Fabric health already answers its slice well: a passive meter rolls traffic up and
> publishes an honesty-graded weather reading on the bus ([`fabric-health-surface.md`](fabric-health-surface.md),
> [ADR 1034](../adr/1034-fabric-health-surface.md)). Time health does **not** — its planned bus
> publisher was never built (F-184),
> so today you cannot see network and time health side-by-side anywhere. This doc generalizes the
> fabric pattern into **one cross-domain contract**: a single honesty-graded envelope (`EnvHealthV1`),
> the already-canonical `ops/<domain>/health/<scope>` topic convention, and **one renderer per
> altitude** — `goat health` (CLI glance + drill), a panoscope **environment band** (COP glance),
> and a stethoscope **environment target** (the "why is it red"). Fabric is the first domain; **time
> is the second**, built by completing the planned `ops/time/attest` attestation publisher and adding a
> per-site roll-up. New domains plug in without new surfaces. It **reveals**; it does not enforce.

| | |
|---|---|
| Status | Proposed — see [ADR 1036](../adr/1036-environment-health-surface.md). Build increments in §7. |
| Anchor ADRs | [ADR 1036](../adr/1036-environment-health-surface.md) (this generalization) · [ADR 1034](../adr/1034-fabric-health-surface.md) (the fabric instance this generalizes) · [ADR 1033](../adr/1033-zenoh-qos-prioritization-reserved-mission-slice.md) (QoS umbrella — the Reveal pillar) · [ADR 0844](../adr/0844-zenoh-deployment-topology-and-acl-v1.md) / [ADR 1035](../adr/1035-operator-namespace-ops-canonical.md) (the `ops/` namespace) |
| Reference designs | [`fabric-health-surface.md`](fabric-health-surface.md) (the fabric domain — the pattern source) · [`universal-ntp-hygiene.md`](universal-ntp-hygiene.md) (the time domain — policy + the `ops/time/attest` attestation feed) · [`ntp-strategy.md`](ntp-strategy.md) (time sources + drift) · [`zenoh-surface-deep-links.md`](zenoh-surface-deep-links.md) (the cross-surface hop contract) · `panoscope.md` · [`stethoscope.md`](https://github.com/usareur-af/goatstuff/blob/main/docs/design/stethoscope.md) · `goat-cli.md` |
| Related findings | F-184 (time-on-bus planned, publisher never built — the gap this closes) |

---

## 1. Why this exists

The operator question is "is the environment we're running in healthy?" — and the *environment* has
several **collective** dimensions that no single peer owns:

- **Network / fabric** — is the brokerless fabric under load pressure? (built — fabric-health.)
- **Time / clock** — is the fleet's clock collectively disciplined, or is a host skewing? (not viewable.)
- **Mesh links** — are the WireGuard links healthy, and is anyone riding the relay fallback?
  (third domain, [ADR 1042](../adr/1042-mesh-link-health-domain.md) — §4.3.)
- **Later** — GNSS lock, power/thermal envelope, RF — same shape, same need.

"Collective" is the operative word: this is **not** per-peer liveness (panoscope already shows which
nodes are up). It is the rolled-up condition of a shared substrate — *the fleet's clock*, *the
fabric's pressure* — graded honestly and rendered where operators already look.

Two gaps make this a real design, not a restyle:

1. **Two domains live on two planes, viewable in two unrelated places.** Fabric health is on the bus
   (`ops/fabric/health/<site>`, honesty-graded, read by `goat health`). Time health is on the
   observability plane only (OTLP → OpenObserve, plus a local Unix socket for enforcement) — there is
   **no on-bus time-health reading**, so network and time health cannot be seen together at any
   altitude. (F-184 explains how that
   happened — it was *planned* for the bus and the publisher simply never got built.)
2. **The fabric pattern is right but single-domain and CLI-only.** `ops/fabric/health` + the 5-state
   honesty model + the passive-meter discipline are exactly the right substrate — but they are bound
   to one domain, and the panoscope/stethoscope rendering of even *fabric* health is designed-not-built
   ([`fabric-health-surface.md`](fabric-health-surface.md) §6). Generalizing the contract and building
   the renderers once serves every domain.

This doc does three things: **(a)** lifts the fabric honesty-graded-on-bus pattern into a
domain-agnostic **contract**; **(b)** makes **time** its second domain by completing the already-planned
`ops/time/attest` publisher and adding a viewing roll-up; **(c)** defines **one renderer per altitude**
(`goat health` / panoscope / stethoscope) so any domain shows up everywhere for free.

## 1a. The time gap was a missing build, not a missing plan (F-184)

Worth stating plainly because it shapes the design and answers "why didn't we have this already":
**time health on the bus was designed and resolved, then never built.** [`universal-ntp-hygiene.md`](universal-ntp-hygiene.md)
§9 open-question #1 was **Resolved 2026-04-20: Zenoh** — each host's `ntp-health-agent` publishes its
signed verdict on `ops/time/attest/{host_id}`, subscribers use `ops/time/attest/**`, with HTTPS `:9753/health`
as the outage fallback. What actually shipped in `ntp-health-agent` (`ops/time-hygiene/`) is the agent's
*other* interfaces — the **Unix socket** (`/run/ntp-health.sock`, read by the fail-closed admit sidecar)
and the **HTTP `:9753` + OTLP** path (durable observability). The `ops/time/attest/{host_id}` Zenoh
publisher — a ~0.5-week item in that doc's Phase-3 build table — was never wired (there is no Zenoh
import anywhere in `ops/time-hygiene/`). It slipped because its *consumers* (the cross-site consensus
loop, the router NTP interceptor) are themselves unbuilt, so nothing pulled the publisher in; the
load-bearing admit path never needed the bus.

So the fix is **smaller** than inventing a time-health surface from scratch: finish the spec'd publisher,
then add a viewing roll-up. The design below treats the planned `ops/time/attest` feed as the raw source.

## 2. Two planes — enforcement vs viewing (keep them separate, exactly as fabric does)

Time health and fabric health each already separate a **mechanism plane** from a **viewing plane**, and
this surface only touches the viewing plane:

| Domain | Enforcement / mechanism plane (raw, per-host) | Viewing plane (rolled, honesty-graded) |
|---|---|---|
| Fabric | every data sample on `**` (carries class+producer) | `ops/fabric/health/<site>` ← `goat-fabric-health` meter |
| Time | `ops/time/attest/{host_id}` signed attestation (admit-gating, cross-site consensus, router interceptor) | `ops/time/health/<site>` ← **new** roll-up reader |

This mirrors how fabric deliberately keeps the **audit-subscriber** (compliance-critical, excludes
`volatile/**`) separate from the **`goat-fabric-health`** meter ([ADR 1034](../adr/1034-fabric-health-surface.md)
§2). For time, `ops/time/attest/**` stays the signed, enforcement-grade per-host feed exactly as
[`universal-ntp-hygiene.md`](universal-ntp-hygiene.md) §5 specifies; the viewing roll-up is a **separate,
non-enforcing** reader that subscribes it and republishes a glanceable per-site reading. The durable
record stays on OTLP → OpenObserve; the bus reading is for **live co-viewing**, not retention.

**The viewing plane never gates anything.** It meters and reveals (the QoS Reveal pillar, generalized).
Admit-blocking, sample-dropping, and demotion stay on the enforcement plane where they already live.

## 3. The cross-domain contract

Three pieces, all reusing what fabric established.

### 3.1 Topic convention — `ops/<domain>/health/<scope>`

`ops/` is the canonical operator/control namespace ([ADR 0844](../adr/0844-zenoh-deployment-topology-and-acl-v1.md)
/ [ADR 1035](../adr/1035-operator-namespace-ops-canonical.md)). Fabric already publishes
`ops/fabric/health/<site>` — **domain-first**. Every domain follows it:

| Domain | Topic | Scope | Status |
|---|---|---|---|
| fabric | `ops/fabric/health/<site>` | per site | **built** (meter; deploy/live-fire gated) |
| time | `ops/time/health/<site>` | per site (rolled from per-host `ops/time/attest/**`) | **new** (this design) |
| mesh | `ops/mesh/health/<site>` | per site (graded from the vantage host's netbird snapshot) | **third domain** ([ADR 1042](../adr/1042-mesh-link-health-domain.md), §4.3) |
| presence | `ops/presence/health/<site>` | per site (rolled from the liveliness token set) | **fourth domain — deployed + live-fired on ZAGROS 2026-06-13** ([ADR 1040](../adr/1040-zenoh-advanced-pubsub-liveliness-adoption.md) gap 3, §4.3a, M-066) |
| consistency | `ops/consistency/health/<site>` | per goatnet (rolled from per-node `ops/consistency/report/**`) | **sixth domain** ([ADR 1110](../adr/1110-typed-consistency-foundation-and-console.md), `consistency-foundation.md` §5) — the convergence record: what each node runs, and which generation of its goatnet's intent it converged against |
| peering | `ops/peering/health/<edge>` | per peering edge (graded from the edge router's own ACL decision log) | **seventh domain** — [`peering-denial-observability.md`](../operations/peering-denial-observability.md), the S4 G-6 gate of cross-sovereign-federation-plan |
| (future) gnss / power / thermal | `ops/<domain>/health/<host>` | per host or site | designed-extensible |

All `ops/<domain>/health/**` topics are **C0 control** (published `Block` + high priority) so the
health signal is never starved by the load it reports on — same as fabric.

### 3.2 The envelope — `EnvHealthV1`

One protobuf envelope carries the **honesty header** every domain shares; domain-specific structure
rides a `oneof detail`. This is the single thing every renderer decodes:

```protobuf
// canonical: ops/zenoh-trial-peers/time-health/proto/env_health_v1.proto
message EnvHealthV1 {
  string domain        = 1;   // "fabric"|"time"|"mesh"|"presence"|"net"|"consistency"|future "gnss"/"power"/"thermal"
  string scope         = 2;   // site tag (or host id for per-host domains)
  int64  ts_unix_ms    = 3;
  string overall_color = 4;   // 5-state honesty: green|amber|red|idle|unmetered (§6)
  uint64 sample_count  = 5;   // overall grounding n (samples/hosts behind this reading)
  uint32 window_secs   = 6;
  oneof detail {
    FabricDetailV1 fabric = 10;   // repeated ClassHealthV1   (the existing fabric rows)
    TimeDetailV1   time   = 11;   // repeated HostClockHealthV1
    MeshDetailV1   mesh   = 12;   // repeated MeshLinkHealthV1 (ADR 1042; per-link WireGuard health)
    PresenceDetailV1 presence = 13; // repeated PresencePeerV1 (ADR 1040 gap 3; liveliness token set)
    NetDetailV1    net    = 14;   // per-node connection inventory (node-net-health.md)
    ConsistencyDetailV1 consistency = 15; // repeated NodeConsistencyV1 (consistency-foundation.md §5, ADR 1110)
    PeeringDetailV1     peering     = 16; // per-edge ACL decisions at a sovereign peering edge (peering-denial-observability.md)
    // future: GnssDetailV1 gnss = 17; ...
  }
}

// time domain — one row per host, rolled from ops/time/attest/{host_id}
message HostClockHealthV1 {
  string host_id              = 1;
  string color                = 2;   // healthy→green warning→amber degraded→red unknown→idle (§6)
  string verdict              = 3;   // raw daemon verdict: healthy|warning|degraded|unknown
  repeated string verdict_reasons = 4;
  double rms_offset_ms        = 5;   // chrony tracking
  int32  stratum              = 6;
  bool   gnss_present         = 7;
  double peer_median_offset_ms = 8;  // cross-site consensus (universal-ntp-hygiene §5.2)
  bool   divergence_detected  = 9;
  int64  attestation_age_ms   = 10;  // staleness of this host's last ops/time/attest heartbeat (grounding)
  bool   attested             = 11;  // false = unattested-publisher → contributes as unknown/idle
}
message TimeDetailV1   { repeated HostClockHealthV1 hosts = 1; }
message FabricDetailV1 { repeated ClassHealthV1 classes = 1; }  // ClassHealthV1 = today's fabric rows
```

`FabricDetailV1` is exactly today's `repeated ClassHealthV1` ([`fabric-health-surface.md`](fabric-health-surface.md)
§4) — so fabric folds in **without changing its rows**, only its envelope. Fabric continues to ship its
current `FabricHealthV1` until the coordinated `V2` bump (§7 increment 7) the fabric doc already
anticipated; until then the consumer decodes both (one extra decoder, unified display model). New
domains add a `oneof` arm + a renderer arm — nothing else.

### 3.3 The honesty model — generalized, never a false green

The 5-state honesty signal is the load-bearing inheritance from fabric ([`fabric-health-surface.md`](fabric-health-surface.md)
§1a). It is **domain-agnostic**: `green`/`amber`/`red` only when *grounded*, else `idle` (UNCONFIRMED)
or `unmetered`; every reading carries `n`; the per-scope `overall_color` is the **worst *grounded*** row,
never a false green. §6 defines the per-domain mapping.

### 3.4 What the colors MEAN — health, not posture, presence, or utilization

The 5-state color answers exactly one question: **is a thing that is participating working correctly?**
It is easy to slide from that into coloring a *characteristic*, an *event*, or a *count* with alarm
colors — and every such slide misleads the operator, who reads amber/red as "something is wrong." Four
tenets keep the surface honest:

1. **Health colors mean health.** `amber`/`red` require a *grounded measurement of a real problem*
   (loss, excess latency, clock offset out of bound), never an inference from a characteristic.
2. **Absence is not illness.** A peer/host that is *not present* may be **deliberately off** — we do
   NOT assume everything must be powered on and connected at all times, and a node being off does not
   imply worse health. Absence → `idle` (neutral), never `amber`/`red`. Only a *measured* failure of a
   *participating* node grades hot.
3. **Posture is shown, not colored — but it may set the EXPECTATION.** A *characteristic* of how a
   working thing is configured — relayed vs direct P2P, which relay, which path — is a neutral badge/count,
   never a *direct* color input; "relayed" is displayed, not alarmed. It may, however, pick the *yardstick*
   a health measurement is judged against, because what counts as degraded depends on the path: a relayed
   link rides a relay hop (often WAN/satellite) and is expected to be slower, so its latency is graded
   against a looser baseline than a direct link's. The measurement still drives the color — posture only
   chooses the threshold. So a relayed link at 900ms reads **green** (normal for that bearer), while a
   *direct* link at 900ms reads **red** (slow on a path that should be fast). This avoids both errors:
   coloring on the mere fact of relaying, AND flagging normal relayed latency red against a LAN-tight
   threshold.
4. **Utilization is a gauge, not an alarm.** Load-toward-capacity is a real *saturation-risk* signal, so
   it may color — but it must read as a **headroom gauge** ("busy"), not as "sick." Frame it as
   utilization, keep the bars.

Per-domain application (§4): **mesh** grades on connectivity + measured latency only, relay is posture
(§4.3); **presence** is an aliveness count — `green` when ≥1 alive, `idle` when none, drops are a neutral
count, never churn-amber (§4.3a); **fabric** keeps load grading, framed as utilization/headroom (§4.1);
**time** is already correct — a genuine correctness signal (§4.2). ([ADR 1036](../adr/1036-environment-health-surface.md) amendment; the mesh + presence meters implement this.)

## 4. The domains

### 4.1 Fabric (first instance — built)

Unchanged from [`fabric-health-surface.md`](fabric-health-surface.md): `goat-fabric-health` subscribes
`**`, rolls by class + producer, publishes `ops/fabric/health/<site>` (+ `ops/fabric/footprint/<peer>`).
This design reframes it as **the first domain of the env-health contract** and schedules its envelope
fold-in (§7.7). Footprint (`ops/fabric/footprint/<peer>`) is fabric-specific and stays as-is — it is a
per-producer *contribution* surface, orthogonal to the per-domain *condition* envelope.

### 4.2 Time (second instance — new)

A per-site roll-up reader subscribes `ops/time/attest/**` (the signed per-host attestations, once §7.2
ships them), and republishes `ops/time/health/<site>` carrying `EnvHealthV1{domain:"time"}` with one
`HostClockHealthV1` per host. The per-site `overall_color` is the worst grounded host. This mirrors
`goat-fabric-health` precisely: subscribe → roll up → publish honesty-graded, persist nothing.

Brokerless-safe: each host self-publishes its own verdict (no central time authority); the reader is a
passive observer that can run anywhere and is non-load-bearing (losing it loses the *view*, not the
*enforcement* — the admit sidecar reads the Unix socket, untouched).

**Two axes, per [ADR 1037](../adr/1037-fabric-internal-time-synchronization-first.md).** The time domain
does not report a single clock verdict — fabric time is internal-synchronization-first, so a host has two
independent health readings that `HostClockHealthV1` carries side by side:

- **Internal coherence** — is the host locked to the fabric's synchronized ensemble within a bounded
  error? This is the reading that matters for *fabric ordering* (fusion, correlation, LWW) and it holds
  with no external reference — a host can be coherent at the denied edge with GNSS gone.
- **UTC confidence** — is the ensemble anchored to true UTC, and how fresh is the last anchor? Matters
  only for UTC-bound operations (cert/TTL validity, cross-cluster timestamps).

So `overall_color` for the *fabric-ordering* question rolls up **internal coherence** (a fully-coherent
but UTC-unknown island is **green for ordering**, not false-degraded); UTC confidence is a *separate*
graded reading (an islanded site reads "coherent / UTC-stale", honestly, not a flat `degraded`). The
single `ntp_class` verdict the agent produces today is the pre-1037 collapse of these two; until the
internal-sync engine + the two-axis fields land, the roll-up maps the single verdict to the
UTC-confidence axis and reports internal coherence as `unmetered` (honestly unmeasured) — never a false
green for ordering.

### 4.3 Mesh (third instance — [ADR 1042](../adr/1042-mesh-link-health-domain.md))

The WireGuard mesh-link domain: "are the site's mesh links healthy — and is anyone riding the relay
fallback?" A `goat-mesh-health` meter (sibling of the time roll-up, §9) reads a host-side snapshot of
`netbird status --json` — maintained atomically by a role-installed systemd timer, bind-mounted
read-only — grades each link with the honesty colors (§6), and publishes `ops/mesh/health/<site>` as
`EnvHealthV1{domain:"mesh"}` with one `MeshLinkHealthV1` row per peer (direct/relayed, measured
latency, relay address, handshake age, direct↔relayed transition count, plus the vantage host).

Three deliberate boundaries:

- **Passive.** netbird already measures per-peer latency and path type for its own connection
  management; the meter injects nothing (no ICMP). An unmeasured latency (netbird reports `0`)
  never grades a link.
- **Hub vantage (v1).** One meter on the site's hub host grades the links that host sees — the
  participant↔fabric links that matter first. Partner↔partner pairwise matrices, jitter/loss, and
  gateway fingerprinting are [snitch](snitch.md)'s territory, deliberately not duplicated here.
- **Liveness stays panoscope's.** A disconnected peer reads `idle` (visible, ungrounded), never red —
  an off-shift laptop is not a mesh-link failure (§8 "not per-peer liveness" holds).

Because `ops/**` is router-persisted (rocksdb, 7d) and audit-captured (MinIO, 30d), the standing
historical question — "was that partner relayed all afternoon?" — is answered by the existing
Phase G replay path with zero new storage; the meter itself persists nothing.

### 4.3a Presence (fourth instance — [ADR 1040](../adr/1040-zenoh-advanced-pubsub-liveliness-adoption.md) gap 3)

The peer-presence domain: "who is alive on the fabric, right now — pushed, not inferred." Peers declare
a Zenoh liveliness token at `<write_root>/presence/<label>` (the panoscope-core presence convention;
stable zenoh API, no `unstable`); the `goat-presence-health` meter (sibling of the time/mesh roll-ups,
§9) watches the token set with `history(true)` and publishes `ops/presence/health/<site>` as
`EnvHealthV1{domain:"presence"}` with one `PresencePeerV1` row per token. This is the native
quiet-vs-dead distinction the fabric domain's passive traffic metering cannot make (a producer that
stopped publishing vs one whose session died).

- **Honesty mapping:** no token ever seen → `idle` (nothing declares presence — UNCONFIRMED, also the
  honest reading against a router whose ACL predates the liveliness grant, F-205); ≥1 alive and no drop
  within the window → `green`; a drop within the window → `amber` (churn — look here); tokens were seen
  but zero alive → `red` (the declared fleet vanished). A token-less fleet never false-greens.
- **Relationship to panoscope liveness:** panoscope's traffic-inferred staleness stays; presence is the
  push-based ground truth that arrives the instant a session dies, partition-honest (a partitioned
  watcher sees drops for the far side — that is true information about reachability, not a lie).
- `goat presence` reads the token set directly (one-shot or `--watch`); the meter exists so presence is
  ALSO a domain on the shared contract (this doc) rather than only a CLI view.

### 4.4 Future domains

GNSS lock, power, thermal, RF: each is a per-host signal a small agent publishes (or an existing agent
extends), a roll-up reader rolls to `ops/<domain>/health/<scope>` as `EnvHealthV1{domain:"<x>"}`, and the
three renderers pick it up via the `oneof` arm. No new surfaces, no new contract.

**Peer (designed, not built).** `ops/peer/health/<site>` carries faults observed on machines we do not own,
measured by a reader beside each router that terminates an outside connection
([`peer-fault-handling.md`](peer-fault-handling.md), ADR 1232). A clock offset that reader observes is a
row in the `time` domain (§4.2), marked observed by a neighbor and not self-reported.

## 5. Three altitudes — one contract, rendered three ways

The same `EnvHealthV1` stream, rendered at the altitude that fits the question. All three already exist
as surfaces; this wires the envelope into each.

### 5.1 `goat health` — CLI glance + drill (the line-oriented altitude)

Generalize the existing fabric-only command into a multi-domain board:

- `goat health` — **board**: one line per domain per scope —
  `FABRIC  AMBER  n=2.1k/5s  nova-a` / `TIME  GREEN  n=4 hosts  nova-a`. One screen, glanceable, pipe-safe.
- `goat health --domain time` (**built**) — time detail: a headline reason (`all 4 host clocks healthy`
  / `1 degraded; 1 unattested (idle, never green)`) + per-host HOST/STATE/VERDICT/OFFSET/STR/GNSS/ATTEST
  rows (an unattested host shows `no` in ATTEST and reads `idle` — never a false green) + the two-watches
  footer. `--domain fabric` — the existing fabric weather. Subscribes `ops/time/health/**`.
- `goat health --domain presence` (**built**) — the board glance of `goat presence`: a headline reason
  (`19 peers alive; no drops this window` / `1 peer dropped within the last window (churn — look here)`)
  + per-peer PEER/STATE/ROOT/SINCE/DROPS rows (32-hex slot anchors truncated for the board; the
  alias-rendered per-token view stays `goat presence`) + a footer pointing at `goat presence` for the
  `--watch` join/leave edges and `--subscribers-of`. Subscribes `ops/presence/health/**`.
- `goat health --domain mesh` (**built**) — mesh-link detail: a plain-English headline reason
  (`all 3 connected peers are riding relay fallback`) + per-link PEER/STATE/PATH/LATENCY/HANDSHAKE/FLIPS
  rows (relay named per link; an unmeasured latency shown as `unmeasured`, not 0; offline peers `idle`)
  + a footer that says what amber/relayed *means* and the `netbird status -d` drill-in. Subscribes
  `ops/mesh/health/**`.
- `goat health --watch` — live refresh (all domains). `--me` / `--peer <id>` — fabric footprint
  (fabric only; rejected for any non-fabric `--domain`).
- Subscribes `ops/fabric/health/**` + `ops/time/health/**` + `ops/mesh/health/**`; for raw per-host
  time detail beyond the roll-up it may also read `ops/time/attest/**`.

### 5.2 Panoscope — the environment band (the COP glance altitude)

A persistent **environment band** at the top of the panoscope COP: per-domain, per-site color chips
(`FABRIC ▮ AMBER   TIME ▮ GREEN`), each with its `n` and honesty state, click-through to the drill-downs.
Optionally **tint peer nodes** by their own clock health (a host whose `ops/time/attest` verdict is
`degraded` gets a red ring) — turning the existing node graph into a time-health heat map. Reuses
panoscope's `/api/legend` + selection substrate ([`zenoh-surface-deep-links.md`](zenoh-surface-deep-links.md)),
fed by the panoscope backend subscribing `ops/<domain>/health/**`. (This is also the home for the
fabric web view that [`fabric-health-surface.md`](fabric-health-surface.md) §6 deferred — built once,
for all domains.)

### 5.3 Stethoscope — the environment target (the "why is it red" altitude)

A stethoscope **environment target** — `?domain=time&site=<s>` (or `goat inspect env:time`) — answers
*why* a domain is amber/red, reusing the existing `Check`-plugin + findings substrate
([`stethoscope.md`](https://github.com/usareur-af/goatstuff/blob/main/docs/design/stethoscope.md)): for **time**, enumerate hosts from `ops/time/attest/**`, surface
which are degraded / diverging / stale and their `verdict_reasons` ("host-7: GNSS lock lost, in
holdover, offset 42 ms vs peer median"); for **fabric**, which producer/class drove the pressure (ties
to the [ADR 1022](../adr/1022-zenoh-denial-observability.md) expected-vs-actual monitor). The glance
(panoscope/CLI) says *what color*; stethoscope says *why, and what to do*.

### 5.3a `goat top` — the collected operator pane (a fourth, terminal altitude)

[`goat-top.md`](goat-top.md) ([ADR 1046](../adr/1046-goat-top-live-fabric-dashboard.md), accepted — built + live-fired on ZAGROS, M-067) adds
a fourth rendering altitude: a single, auto-refreshing terminal pane that composes **all** the
`EnvHealthV1` domains at once (a per-domain weather strip) plus the `ProducerFootprintV1` "top" rows
(who is driving the fabric — msgs/s, B/s, class shares, joined to their mesh path + latency). Where
`goat health --domain X` is the single-domain glance and panoscope is the web COP, `goat top` is the
*whole-environment, terminal, refreshing* altitude for the SSH / constrained-link operator. Pure
consumer (no producer, no enforcement); same 5-state honesty discipline (never a false green; stale
beats dim and show their age). Host-process CPU/mem is v1-deferred (no process-stats producer exists on
the bus) with the `ops/host/stats/<host>` publisher named as the scoped v2 path — a future
`EnvHealthV1{host}` `oneof` arm (§4.4).

### 5.4 The hop contract — `domain=`

The three altitudes link via the Block 94 deep-link contract ([`zenoh-surface-deep-links.md`](zenoh-surface-deep-links.md)),
which gains one param: **`domain=<fabric|time|…>`**. A panoscope env-band chip → stethoscope
`?domain=time&site=…`; a stethoscope verdict → `goat health --domain time`. Preserve-the-target,
unchanged in spirit.

## 6. Measurement trust — the honesty rules, per domain

The five "no red herrings" rules ([`fabric-health-surface.md`](fabric-health-surface.md) §1a) are
**inherited wholesale** and govern every domain: honest-unknown-beats-false-green; grounding (`n`)
travels with every reading; measured-vs-assumed-vs-inferred kept distinct; dipstick-simple on top;
passive-only (ride existing signal, probe only when quiet). The per-domain verdict → color mapping:

| Domain | Source verdict | → honesty color | Grounding (`n`) | Never-false-green guard |
|---|---|---|---|---|
| fabric | (as today) | green/amber/red vs target; idle = too little traffic; unmetered = no target | sample count behind the window | a class with no traffic / no target never reads green |
| time | `healthy` | **green** | hosts attested + fresh behind the roll-up | a **stale** attestation (older than 2 heartbeat intervals, `attestation_age_ms`) downgrades that host to `idle` — never trust a stale green |
| time | `warning` | **amber** | " | — |
| time | `degraded` | **red** | " | — |
| time | `unknown` / `unattested` | **idle** (UNCONFIRMED) | " | a host that cannot form a verdict, or an `unattested-publisher`, contributes `idle`, never green |
| time | (no attestation on bus at all) | **unmetered** | n=0 | absence of signal is `unmetered`, not green |
| mesh | connected + direct (P2P) | **green**; a *measured* latency ≥ the **direct** amber/red threshold (250/600ms) escalates | connected links behind the reading | netbird reports `latency: 0` when it has no measurement — an unmeasured latency never grades a link |
| mesh | connected + relayed | **green**; a *measured* latency ≥ the **relayed** amber/red threshold (looser, 1000/2500ms) escalates | " | relay is posture, not health — it sets the yardstick, not the color (§3.4 tenet 3); a relayed link at normal-for-its-bearer latency is green |
| mesh | disconnected | **idle** | (not grounded) | absence is not illness (§3.4 tenet 2) — a deliberately-off peer never grades amber/red |
| mesh | disconnected peer | **idle** (visible, ungrounded) | excluded from `n` | liveness is panoscope's job; an off-shift peer is not a mesh failure, never a red |
| mesh | snapshot stale / missing | **unmetered** | n=0 | a snapshot older than the staleness window publishes `unmetered` — never a false green from old data |

The time mapping is a direct, faithful image of the `ntp-health-agent` verdict state machine
(`healthy/warning/degraded/unknown`, [`universal-ntp-hygiene.md`](universal-ntp-hygiene.md) §4.3) onto
the honesty model — and `attestation_age_ms` is the time analog of fabric's "ride real traffic": a green
you can't ground (stale heartbeat) is reported `idle`, not asserted.

## 7. Build increments

1. **The contract + this design + ADR — (this track).** `environment-health-surface.md` +
   [ADR 1036](../adr/1036-environment-health-surface.md) + F-184
   + the `EnvHealthV1` `.proto` shape + all doc tie-ins. No service code yet; the contract is ratifiable
   on its own.
2. **Complete the planned `ops/time/attest` publisher.** Split in two once it surfaced that the agent does
   not yet *produce* a signed attestation to publish (the investigation behind F-184):
   - **2a — the agent produces + serves the signed attestation (DONE).** `ntp-health-agent` integrates the
     Ed25519 signer at each poll (`internal/attest`, alg `ntp-attest-v1` — the versioned canonical signing
     view in [`universal-ntp-hygiene.md`](universal-ntp-hygiene.md) §4.5) and serves the signed verdict
     (`signature`/`attestation_pubkey`/`valid_until`/`attested`) on the Unix socket + HTTP `:9753/health`.
     No key configured → honest `attested:false`. Backward-compatible (the admit sidecar reads only
     `ntp_class_str`). This `attested` flag is exactly the `HostClockHealthV1.attested` input the time
     roll-up needs (§6 honesty mapping).
   - **2b — the bus publisher.** A small **Rust** per-host sidecar reads the agent's signed `:9753/health`
     and publishes it on `ops/time/attest/{host_id}` over mTLS (Go had no native Zenoh mTLS binding when
     this was chosen — the `rest-pub-bridge` reason, [ADR 1023](../adr/1023-zenoh-rest-pub-bridge-for-non-rust-publishers.md);
     a cgo `zenoh-go` later shipped in 1.9.0, but Rust stays the choice here;
     the sidecar forwards the already-signed blob verbatim) + the `ops/time/attest/**` router ACL grant.
     `ops/time/attest/**` then subscribable ([`universal-ntp-hygiene.md`](universal-ntp-hygiene.md) §5.1).
     Independently valuable — also unblocks cross-site consensus + the router interceptor. Closes the
     F-184 root cause.
3. **The time roll-up reader — built.** `goat-time-health` (`ops/zenoh-trial-peers/time-health/`) — a
   sibling meter templated on `goat-fabric-health` (§9): subscribes `ops/time/attest/**`, keeps the latest
   verdict per host (+ the local receive-time as the honest staleness clock — it does *not* trust the
   verdict's own timestamps to decide staleness), and on a timer rolls per-host → publishes
   `ops/time/health/<site>` as `EnvHealthV1{domain:"time"}` (the canonical envelope; `proto/env_health_v1.proto`).
   Persists nothing. Honesty mapping per §6 (healthy→green/warning→amber/degraded→red/unknown→idle;
   unattested or stale → idle, never a false green; `overall_color` = worst grounded host, `unmetered` when
   none reported). The single `color` is the UTC-confidence axis for now; internal coherence stays unmeasured
   until the sync engine (§4.2). Crate + tests + clean `--release --locked`; the `ops/time/health/**` ACL
   grant (CN `goat-time-health-trial-peer`) + the deploy layer (image workflow + role + playbook + group_vars)
   are wired. **Remaining:** operator-run image build + cert mint + deploy + live-fire.
4. **`goat health` multi-domain consumer.** Add the `EnvHealthV1` decoder + the board view + `--domain` +
   `--host`. Keep the existing fabric path until the §7.7 fold-in. **`--domain mesh` built (2026-06-13):**
   `goat health --domain mesh` decodes `EnvHealthV1{mesh}` and renders the per-link table with the *why*
   in words — a headline reason (`all 3 connected peers are riding relay fallback`), per-link
   PEER/STATE/PATH/LATENCY/HANDSHAKE/FLIPS rows (relay named, an unmeasured latency shown honestly as
   `unmeasured`, offline peers as `idle`), and a footer that says what amber/relayed *means* and how to
   drill in (`netbird status -d`). `--watch` for the live stream. ANSI-safe column padding; 9 unit tests.
   The time-domain board + `--host` rows remain (the fabric path is untouched pending the §7.7 fold-in).
5. **Panoscope environment band** (§5.2) + **stethoscope environment target** (§5.3) + the `domain=`
   deep-link param. The glance and the why.
6. **Live-fire on nova-a → M-NNN.** Deploy; confirm fabric + time show side-by-side at all three
   altitudes; a deliberately skewed host moves `TIME` toward red, shows in the host drill-down, and
   stethoscope explains it.
7. **Coordinated fabric fold-in.** Migrate `FabricHealthV1` → `EnvHealthV1{domain:"fabric"}` at the
   `V2` bump the fabric doc already anticipated (folds with Layer-B underlay), retiring the parallel
   decoder. End state: one envelope, all domains.
8. **The mesh domain ([ADR 1042](../adr/1042-mesh-link-health-domain.md), 2026-06-12) — built.**
   `goat-mesh-health` (`ops/zenoh-trial-peers/mesh-health/`) + the `mesh-health` role (host-side
   `mesh-health-snapshot.timer` + the containerized meter) + the `infra-mesh-health`/`infra-w-mesh-health`
   router ACL grant + the `MeshDetailV1` arm (oneof 12) in the canonical proto. ZAGROS first
   (`mesh-health-deploy.yml -e target=zagros_hosts`). **Read via `goat health --domain mesh`**
   (increment 4 above, built 2026-06-13) — the raw `goat sub 'ops/mesh/health/**'` path still works but
   the rendered table is the operator surface. §4.3 for the domain's deliberate boundaries.

## 8. What this is not

- **Not enforcement.** Viewing plane only — it reveals; admit-gating / sample-drop / demote stay on the
  per-domain enforcement plane (§2). The fabric "near-limit" + the time roll-up are both advisory.
- **Not a new health authority.** No central aggregator computes "the fleet's health." Each host
  self-publishes (`ops/time/attest`, the `**` it already emits); a passive reader rolls up a *view*.
  Re-introducing a central enforcer would re-introduce the broker the fabric rejected
  ([`zenoh-brokerless-consequences.md`](zenoh-brokerless-consequences.md)).
- **Not persistence.** Rolling counters, nothing durable. OTLP → OpenObserve stays the retained record
  for time; the audit-subscriber for fabric. The bus reading is live co-viewing.
- **Not per-peer liveness.** That is panoscope's existing node graph. This is *collective* substrate
  condition — the fleet's clock, the fabric's pressure.
- **Not the time *mechanism*.** It does not discipline the clock (chronyd does), compute consensus (the
  daemon does), or replace `ops/time/attest` (it consumes it).

## 9. Component shape — sibling meter now, consolidation later

The time roll-up ships as a **separate `goat-time-health` meter**, templated on `goat-fabric-health`
exactly as `goat-fabric-health` was itself templated on the audit-subscriber — same precedent, lowest
disruption to shipped fabric code (and `goat-mesh-health` continues the line, templated on the time
meter). The clean **end state** is a single `goat-env-health` meter hosting
pluggable domain rollers (a fabric roller subscribing `**`, a time roller subscribing `ops/time/attest/**`,
a mesh roller reading the netbird snapshot),
publishing per-domain `ops/<domain>/health/<scope>` envelopes from one deployable. Consolidation is a
named later increment (folds with §7.7), not a precondition — keeping the viewing roll-up an *add-on*
(losing it loses a view, not enforcement) is the right blast-radius posture and matches how time
health's load-bearing parts (daemon + admit sidecar) already sit separately.

## 10. Brokerless safety check

Re-read against [`zenoh-brokerless-consequences.md`](zenoh-brokerless-consequences.md): nothing here is a
central component solving a brokerless gap. Health is **observed** (each node self-publishes), **passively
rolled** (a reader with no authority), and **non-enforcing** (no central QoS/time arbiter appears). The
roll-up reader is a convenience vantage, not a chokepoint — if it dies, the raw per-host feeds
(`ops/time/attest/**`, `**`) and the enforcement planes are untouched. That is the same posture as
`goat-fabric-health`, and the reason this generalizes cleanly rather than smuggling a broker back in.
