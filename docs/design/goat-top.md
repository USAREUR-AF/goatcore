# `goat top` — the live fabric dashboard (the `top(1)` of the data fabric)

> **60-second summary:** `goat top` is a single, auto-refreshing terminal pane that answers
> "what is the fabric doing *right now*, and who is driving it?" — the way `top(1)` answers it for a
> host. It is a **pure consumer**: it subscribes the five environment-health + footprint topics that
> already live on the bus ([`environment-health-surface.md`](environment-health-surface.md)), keeps a
> latest-wins live model, and renders a multi-pane ratatui screen — a per-domain weather strip
> (mesh/fabric/time/presence/peer), a fabric class-load row, a sortable **producer table** (the "top"
> rows: who is using the net, msgs/s, B/s, class shares, near-limit, joined to their mesh path +
> latency), and a totals footer. The producer table also carries a **SUBS** column — the
> per-producer *consumer fan-out* (how many distinct subscribers each producer's output actually
> reaches), read from the shared pub/sub census ([`subscriber-consumption-visibility.md`](subscriber-consumption-visibility.md),
> `Census::fanout_for_anchor`); honest `—` when the census is unavailable, never a fabricated 0.
> It adds **no producers and no enforcement**. It inherits the
> environment-health **honesty discipline** wholesale: never a false green — an ungrounded domain
> reads `idle`/`unmetered` (dim), a stale beat shows its age and dims, every reading carries its
> sample-count `n`.

| | |
|---|---|
| Status | **Accepted — built + live-fired** 2026-06-14; core merged #1159, envelope BAND reveal column merged #1160 (its live-fire pending on the qos-throughput-envelopes track). See [ADR 1046](../adr/1046-goat-top-live-fabric-dashboard.md). |
| Anchor ADRs | [ADR 1046](../adr/1046-goat-top-live-fabric-dashboard.md) (this) · [ADR 1036](../adr/1036-environment-health-surface.md) (the domains it composes) · [ADR 1034](../adr/1034-fabric-health-surface.md) (fabric load + footprint) · [ADR 1042](../adr/1042-mesh-link-health-domain.md) (mesh path/latency) · [ADR 1040](../adr/1040-zenoh-advanced-pubsub-liveliness-adoption.md) gap 3 (presence) · [ADR 1019](../adr/1019-goat-cli-unified-operator-binary.md) (the binary) · [ADR 1039](../adr/1039-slot-alias-display-layer.md) (alias display) |
| Reference designs | [`environment-health-surface.md`](environment-health-surface.md) (the one-contract / three-altitudes pattern this extends) · [`fabric-health-surface.md`](fabric-health-surface.md) (footprint) · `goat-cli.md` · [`goat-cli-tui.md`](goat-cli-tui.md) (the `goat ui` / Tail-lens TUI precedent) |
| Web-native equivalent | **panoscope `/top`** (2026-07-09, track `web-top-who`): the same content + honesty discipline rendered in the browser, viewer-scoped per request (ADR 1073/1074 — producer rows gate per producer identity; identity-free aggregates render for every tier). See `panoscope.md` §"Web-native `top` + `who` pages". The terminal `goat top` is unchanged — it runs under the operator's own cert at the edge; the web page is a different trust model (one service identity reads, per-viewer authorization renders). |

---

## 1. Why this exists — a new altitude, not a new signal

The environment-health surface already defines three rendering **altitudes** for the on-bus health
signal ([`environment-health-surface.md`](environment-health-surface.md) §5):

- `goat health` — the **single-domain glance** (`--domain mesh|fabric|time|presence`), line-oriented,
  pipe-safe. One domain, drilled, then you exit.
- panoscope — the **web COP**: an environment band over the persistent node graph.
- stethoscope — the **"why is it red"** target.

None of those is the operator question this command answers: *"give me one screen that shows the
**whole** environment at once and keeps refreshing, and rank the producers driving it."* `goat health`
shows one domain and returns; panoscope is a browser COP that presumes a workstation and a network
path to the web surface. The tactical-edge / SSH-session need is a **collected operator pane** — every
domain's weather plus the producer "top" rows, live, in a terminal over a constrained link.

`goat top` is that fourth altitude:

| Altitude | Surface | Question | Shape |
|---|---|---|---|
| single-domain glance | `goat health --domain X` | "is *one* domain healthy?" | line, one beat, exit |
| **collected operator pane** | **`goat top`** | **"what is the *whole* fabric doing now, and who's driving it?"** | **full-screen, all domains + producer ranking, auto-refreshing** |
| web COP | panoscope | "show me the COP" (graph + band) | browser |
| why-is-it-red | stethoscope | "explain this color" | findings target |

It is **complementary to**, not a replacement for, any of them: it composes the *same* `EnvHealthV1`
domains and the *same* `ProducerFootprintV1` footprints, in the terminal, at the all-at-once altitude.

## 2. Scope — pure aggregation, zero new plumbing

Every input is already published on the bus by shipped work. `goat top` is a **new consumer + TUI**.

| Pane / signal | Topic (already published) | Type | Built by |
|---|---|---|---|
| Mesh weather + per-peer path/latency/flaps | `ops/mesh/health/<site>` | `EnvHealthV1{mesh}` | [ADR 1042](../adr/1042-mesh-link-health-domain.md) |
| Fabric weather + per-class load | `ops/fabric/health/<site>` | `FabricHealthV1` | [ADR 1034](../adr/1034-fabric-health-surface.md) |
| Per-producer footprint (the "top" rows) | `ops/fabric/footprint/<peer>` | `ProducerFootprintV1` | [ADR 1034](../adr/1034-fabric-health-surface.md) |
| Presence (alive / dropped, churn) | `ops/presence/health/<site>` | `EnvHealthV1{presence}` | [ADR 1040](../adr/1040-zenoh-advanced-pubsub-liveliness-adoption.md) gap 3 |
| Time/clock health | `ops/time/health/<site>` | `EnvHealthV1{time}` | [ADR 1036](../adr/1036-environment-health-surface.md) time domain |
| Peer faults (clock offsets our router measured on peers we do not own) | `ops/peer/health/<site>` | `EnvHealthV1{peer}` (arm 17) | ADR 1232 (in review); published by goat-peer-health in goatstuff |

The wire types are the hand-written prost-derive structs `goat health` already decodes
(`crates/goat-cli/src/cmd/health.rs`). To avoid a third hand-copy of those structs, the shared
protobuf types + small display helpers (`fmt_age`, `short_name`, the 5-state color map) are extracted
into a `pub(crate)` module (`crates/goat-cli/src/health_wire.rs`) that both `goat health` and
`goat top` consume — a refactor-only step that keeps `goat health`'s existing tests green
([ADR 1015](../adr/1015-code-review-discipline.md) one-hat-per-commit).

## 3. Host-process CPU/mem resource usage — explicitly deferred to v2

The operator's framing included "resource usage … of the actual producer processes." There is
**no per-host process probe on the bus today** — fabric health observes *byte and message rates* off
real traffic, not the CPU/RSS of the producing process. Adding host CPU/mem would mean adding a **new
producer** (a per-host agent publishing process stats), which is out of scope for an
aggregation-only consumer and would re-open the "is this a new bus publisher" question this command
deliberately avoids.

**v1 decision:** use **msgs/s + B/s** (the footprint rates) as the resource proxy — for a brokerless
data fabric, *fabric throughput per producer* is the resource that matters and the one we can read
honestly without inventing a probe. Resource pressure shows as fabric class-load color + `near-limit`
flags, not as host CPU bars.

**v2 path (scoped, not dropped):** a per-host stats publisher on `ops/host/stats/<host>`, produced
exactly the way mesh-health produces its snapshot — a role-installed systemd timer captures a
host-side snapshot (e.g. `/proc` rollup or a small probe), a passive meter grades + publishes it as a
new `EnvHealthV1{host}` domain (a `oneof` arm, the contract already extends this way per
[`environment-health-surface.md`](environment-health-surface.md) §4.4), and `goat top` adds a
HOST-RESOURCE pane that joins it onto the producer table by host. When that domain exists, `goat top`
consumes it like any other — no structural change. This is named here so it is a known v2 increment,
not a silent gap.

## 4. The screen

Top → bottom, a vertical ratatui layout (mirrors the `goat ui` Tail lens structure in
`crates/goat-cli/src/tui/tail.rs`).

### 4.1 Header weather-strip — all domains at a glance

One color chip per domain: **MESH · FABRIC · TIME · PRESENCE · PEER**. Each chip carries:

- the **honesty color** (green/amber/red, or dim `idle`/`unmetered`),
- the one-word state,
- the grounding `n`,
- the **last-beat age** (how long since this domain's last reading arrived).

A domain with no beat ever seen reads `unmetered` (dim). A domain whose last beat is **stale** (older
than ~2.5× its publish window) is rendered dim with its age — never as a live color. This is the
weather-strip generalization of the `goat health` board glance, all domains side-by-side.

The PEER chip is built (unit-tested; not yet run against a live goat-peer-health publisher). It
shows the site grade of `ops/peer/health/<site>` only; the per-peer rows are `goat health --domain
peer`.

### 4.2 Fabric class-load row

Per-class bar (from `FabricHealthV1.classes`): the class code, its canonical short-name, msgs/s vs
target, headroom %, honesty color. This is the fabric weather rendered as bars rather than lines, e.g.
`C0 control   4/5000 msgs/s   0%  GREEN`.

The canonical names — `C0 control` / `C1 mission` / `C2 situational` / `C3 bulk`
([`zenoh-qos-and-prioritization.md`](zenoh-qos-and-prioritization.md),
[`fabric-health-surface.md`](fabric-health-surface.md)) — are a **display-only client-side map** shown
beside the code as a legend; the wire key, ACL, and `FabricHealthV1.class` field stay `C0`..`C3`
(same display-only spirit as the slot-alias layer, ADR 1039). An unknown/future class code (e.g. `C4`)
renders as just the bare code, so the row degrades gracefully. The compact producer SHARES column stays
bare codes — the class-load row is the legend; shares don't repeat it.

### 4.3 Producer table — the "top" rows (the focus)

One row per producer seen on `ops/fabric/footprint/**`, **sortable** (default msgs/s; cycle to B/s,
then peer-name). Columns:

| Column | Source |
|---|---|
| RANK | `ProducerFootprintV1.rank` |
| PEER | peer anchor, **alias-rendered** via the ADR 1039 registry (`registry.render(anchor)`); degrades to UUID when the portal is absent |
| MSG/S | `sample_rate` |
| B/S | `byte_rate` |
| BAND | per-actor throughput-envelope verdict ([ADR 1045](../adr/1045-qos-throughput-envelopes.md)): `OVER`/`HIGH`/`ok`/`LOW` (worst of the msgs/s and bytes/s axes against the declared envelope), or an honest `—` when no envelope is declared / the portal is absent — never a faked green |
| SHARES | per-class `ClassShareV1` (compact: `C2 80% · C0 20%`) |
| NEAR-LIMIT | `near_limit` flag (⚠) |
| PATH | joined from the **mesh** domain — direct P2P / relayed (relay named) |
| LATENCY | joined from the mesh domain — measured latency, or `unmeasured` (never faked) |

The PATH + LATENCY join is by peer: a footprint row's peer anchor is matched to a `MeshLinkHealthV1`
row (by short-name / fqdn). When mesh has no row for a producer (e.g. an off-vantage peer), PATH/LATENCY
read `—` honestly — the absence is shown, not invented.

`↑/↓` (`j`/`k`) select a row for highlight; selection is stable across refreshes (rides the peer
anchor, not the row index), mirroring tail.rs's stable-selection design. `↵` drills into the selected
producer (§4.5).

### 4.5 Producer drill-down + legend (modal lenses)

Two read-only overlays sit on top of the live table (which keeps updating underneath); both close with
`Esc`/`Enter`/`q`/`?`, and neither can mutate the fabric.

- **`↵` producer drill-down** — the selected producer's full footprint (throughput, envelope band,
  class shares, mesh path + latency) plus its **live consumer fan-out from the census**: the count and
  the distinct subscriber ZIDs whose subscription key-expression intersects a topic this producer
  publishes, and the producer's own published topics. This is the subscribe-side half of the pub/sub
  picture at single-producer altitude (`subscriber-consumption-visibility.md`). An empty census reads
  "consumer set not yet measured" — never a confident `0`. When an axis is **out of its envelope**, the
  band line expands with a per-axis breach explainer — the allotment it crossed and by how much, e.g.
  `msgs/s 40 · OVER max 20 · +20 over (200%)` (and the symmetric form for `HIGH`/`LOW`) — so the verdict
  is never just a color. `↑/↓` browse producer-to-producer without closing.
- **`?` legend** — an in-app reference laid out as an aligned two-column grid (term | meaning, one
  concept per row) for the weather chips, the five honesty colors (including the "absence is not
  illness" rule for `idle`/`unmetered`), the producer columns (incl. `SUBS`), and the envelope bands.
  Kept in lock-step with the surfaces it describes.

**Incremental producer filter** (htop idiom): `/` opens a live filter that narrows the table to
producers whose alias **or** anchor contains the typed substring (case-insensitive); `Esc` clears it.
`--filter <substr>` sets it non-interactively for `--once`. The table title carries the active filter
and an `N of M shown` count so a narrowed view never reads as the whole fabric.

### 4.4 Totals footer

- active peers (distinct footprint producers in the window),
- total msgs/s + B/s (summed across producers),
- links direct vs relayed (from the mesh domain),
- presence alive / dropped (from the presence domain),
- the keybinding help line.

## 5. The live model — latest-wins, bounded, honest about staleness

A **producer task** (mirroring `tail.rs::produce`) owns the Zenoh `Session` and declares one
subscriber per key — `ops/fabric/**`, `ops/mesh/health/**`, `ops/time/health/**`,
`ops/presence/health/**` — all feeding one unbounded mpsc channel. A **synchronous ratatui event loop**
on `tokio::task::spawn_blocking` drains the channel each tick (`--interval`, default 2 s) and redraws.
Terminal teardown (`disable_raw_mode` + `LeaveAlternateScreen` + show cursor) is guaranteed even on
error/panic, exactly as `tail.rs::run_loop` does it.

The state model is **latest-wins per key** with a per-entry last-seen `Instant`:

- one current `EnvHealthV1`/`FabricHealthV1` per domain (replaced on each beat),
- one `ProducerFootprintV1` per peer anchor in a `HashMap` (replaced on each beat).

Bounded memory: there is **no unbounded ring** of historical samples (unlike tail.rs, which holds a
scrollback ring — `goat top` is a *current-state* dashboard, not a log). Footprint entries that have
not been refreshed within their **staleness window** (~2.5× the publish interval) are dropped from the
table on the next tick, so a producer that goes quiet ages out rather than lingering as a false-live
row. A domain chip whose last beat is past its staleness window dims and shows its age — **never
rendered live**.

The state-fold logic (apply-beat, apply-footprint, staleness-expiry, sort ordering, color mapping)
lives in **pure, unit-testable methods** with no terminal and no wall clock (time passed in), mirroring
`tail.rs`'s testable `apply_sample` / `bucket_ages` split.

**One view model, two renderers.** The live state is projected once into a renderer-agnostic
`ViewModel` (`TopState::view_model(now)`) that applies the honesty discipline a single time —
stale-collapse and ungrounded→`idle`/`unmetered` are computed there, not in either renderer. Two
renderers then consume it: the **ratatui** interactive draw (default), and a **plain-text one-frame**
renderer for `--once` (§7). Both therefore agree by construction — there is no way for the batch
snapshot and the live screen to disagree about whether a domain is honestly green.

## 6. Honesty discipline (inherited wholesale from the env-health surface)

The 5-state honesty model is load-bearing and **identical** to `goat health`
([`fabric-health-surface.md`](fabric-health-surface.md) §1a,
[`environment-health-surface.md`](environment-health-surface.md) §6):

- `green`/`amber`/`red` only when **grounded**; otherwise `idle` (UNCONFIRMED) or `unmetered` (no
  signal). In ratatui these map to `Color::Green` / `Color::Yellow` / `Color::Red` /
  `Color::DarkGray` (dim, for both `idle` and `unmetered`).
- every chip and row shows its `n`.
- a **stale** beat dims and shows its age; it is never re-asserted as a live color.
- the color map is computed in `goat top` against the same color strings; it does **not** reuse
  `health.rs::colorize()` (that emits ANSI escapes for line output — wrong layer for ratatui, which
  styles via `Style::fg`).

The rule is: **the dashboard never shows a green it cannot ground.** A quiet fabric, an off-vantage
peer, a meter that stopped — each reads its honest dim state, not a reassuring green.

## 7. Command surface

```
goat top [--interval <secs>] [--site <scope>] [--sort <msgs|bytes|peer>] [--filter <substr>] [--once]
```

- `--interval` (default `2`) — redraw / drain cadence in seconds (and the `--once` collection window,
  floored so it captures at least one fabric beat).
- `--site <scope>` (optional) — filter to one site scope when multiple publish on the same fabric.
- `--sort` (default `msgs`) — initial producer-table sort; `s` cycles it live.
- `--filter <substr>` (optional) — initial producer-table filter (alias or anchor substring,
  case-insensitive); `/` opens it live and `Esc` clears it.
- `--once` — **batch snapshot** (the `top -bn1` analog): collect beats for one bounded window, render
  **one frame as plain text** to stdout, then exit 0. The scriptable / CI counterpart to the
  interactive screen.

Keys (interactive): `q` / `Esc` / `Ctrl-C` quit; `s` cycle sort; `/` open the producer filter
(`Esc` clears it); `p` pause refresh (freeze the current view); `↑/↓` (`j`/`k`) select a producer row;
`↵` drill into the selected producer (consumer fan-out + published topics); `?` open the legend.
Both overlays close with `Esc`/`Enter`/`q`/`?` (§4.5).

**TTY gating, by mode:** the **interactive** full-screen dashboard is TTY-gated via
`crate::tui::require_tty("goat top")` *before* any session open — a non-TTY (pipe / CI) invocation is
refused by construction so a script can never hang waiting for input. **`--once` is the deliberate
exception:** it does **not** call `require_tty` and is allowed on a non-TTY, because it is the
scriptable batch path that renders one plain-text frame and exits (it can never block on input). Both
modes feed the same view model (§5), so the snapshot's honesty matches the live screen's:
`idle`/`unmetered` print as the honest state word, a stale beat prints its age, never a faked green.
The line-oriented `goat health` remains the single-domain scriptable path.

## 8. Session robustness — read-only, random ZID (does not monopolize the operator session)

`goat top` is a pure read-only subscriber, so it opens its session **without** the ADR 1041 ZID-pin
(a **random ZID**, via `cmd/pubsub.rs::open_session_readonly` = `open_zenoh_session_opts(profile,
false)`). Pinning the operator's slot ZID earns no provenance for a non-publisher (provenance is the
publisher property `src_zid == anchor`), and the router permits only one transport per ZID — so a
**long-lived** dashboard holding a pinned session would monopolize the operator's single slot identity
and give any concurrent `goat` command (health / sub / a second `top`) `MAX_LINKS`, which the brief
connect-retry could not outlast on a session held for the dashboard's whole lifetime. A random ZID lets
the dashboard **coexist** with other `goat` commands; attribution and authz ride the cert CN, not the
ZID. This is the same lever `goat blob` uses for its concurrent transfer sessions
(F-195;
`open_session_for_transfer`).

The `MAX_LINKS` retry + connect-timeout hardening in the session-open path (Task 1,
F-209 #3) is a
**separate track**; it still covers transient router-side reaps for `goat top`'s open. `goat top` does
**not** modify that session-open path itself — it just selects the no-pin variant.

## 9. What this is not

- **Not a producer.** It publishes nothing — pure subscribe + render.
- **Not enforcement.** Viewing only; admit-gating / sample-drop / demote stay on the per-domain
  enforcement planes ([`environment-health-surface.md`](environment-health-surface.md) §2).
- **Not a new health authority.** It rolls up no central verdict; it reads the per-domain readings
  each meter already publishes. If `goat top` dies, the raw feeds and enforcement are untouched —
  brokerless-safe by construction ([`zenoh-brokerless-consequences.md`](zenoh-brokerless-consequences.md)).
- **Not host process resource usage in v1** — deferred to v2 via `ops/host/stats/<host>` (§3).
- **Not a scrollback log.** It is current-state; `goat sub --tui` (the Tail lens) is the scrollback
  surface.
```