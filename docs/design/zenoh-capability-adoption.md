# Zenoh capability adoption — closing the gaps a capability review found

**Status:** Design (authored 2026-06-11). Capture + adoption plan; build increments designed-not-built unless marked.
**Decision:** [ADR 1040](../adr/1040-zenoh-advanced-pubsub-liveliness-adoption.md).
**Companion finding:** F-193.
**Related:** [`zenoh-brokerless-consequences.md`](zenoh-brokerless-consequences.md) · `zenoh-data-fabric.md` · [`environment-health-surface.md`](environment-health-surface.md) · [`fabric-health-surface.md`](fabric-health-surface.md) · [ADR 0858](../adr/0858-zenoh-image-upstream-pivot.md) (the router-image boundary this sharpens) · [ADR 1025](../adr/1025-zenoh-dynamic-tenant-authz-slot-pool.md) · F-178 · F-177.

## Why this doc

A 2026-06-11 review walked Zenoh's full capability surface against our usage. The verdict: we use the
**primitives** well and idiomatically (key-expression discipline, static identity-bound ACL, mTLS-everywhere,
external-subscriber-over-plugin), but four native capabilities that directly serve our hardest requirements —
denied-edge resilience, presence, ordering — are unused, hand-rolled, or left on the floor. This doc captures
the review, **corrects a load-bearing misconception** that had parked the two biggest items, and is the plan of
record for adopting them.

## The build boundary, corrected (the load-bearing point)

[ADR 0858](../adr/0858-zenoh-image-upstream-pivot.md) addendum and [ADR 1025](../adr/1025-zenoh-dynamic-tenant-authz-slot-pool.md) §7
both flagged **AdvancedPublisher/AdvancedSubscriber** as "the strongest reason to ever own the build" — i.e. that
adopting them *reopens the router build*. **That conflates two different boundaries:**

| Boundary | What it is | Cost to cross | Examples |
|---|---|---|---|
| **Router image** | `eclipse/zenoh:1.9.0` `zenohd` binary, digest-pinned, never forked (ADR 0858) | Reopen the build from source | `shared-memory` transport, `stats`/`/metrics` — genuine **zenohd compile features** |
| **Client cargo features** | The `zenoh`/`zenoh-ext` crates *we* compile into our own peers | Add a dependency / feature flag | `unstable` → liveliness, **AdvancedPub/Sub**, QueryingSubscriber, PublicationCache |

Almost every client crate we own **already builds with `features = ["unstable"]`** (panoscope-core, flashlight,
synth-vendor, audit-subscriber, rest-pub-bridge, fabric-health, time-health, loadgen, trashgoat,
ingress-gateway — only `goat-cli` opts out). AdvancedPub/Sub and liveliness are **client-side `zenoh-ext`
constructs built on primitives the stock router already routes** (a queryable-backed PublicationCache + a
querying subscriber + liveliness-based publisher detection). Adopting them is a **client dependency addition,
not a router build change** (evidence: F-193). The "own the build" cost never applied to them; it still applies
to SHM and router `stats`.

**Consequence:** the F-178 "no DLQ → producer-cooperation" reliability story and the F-181/1025 slot-model
reliability deferral were both gated on a cost that does not exist for these features. They are adoptable now.

## Gap 1 — Liveliness (presence; the missing health axis)

**Native and stable in 1.9.0** (`Session::liveliness()` is not `unstable`-gated; the subscriber `history` option
is documented). ~~The ACL already permits the variants~~ **Corrected at build time (F-205):
the F-124 wiring taught the config the variant *keywords* but no rule ever granted them — under default-deny all
liveliness was silently denied (the F-177 class) until the gap-3 ACL change landed** (`declare_liveliness_subscriber`
+ `liveliness_query` in the universal read rule, `liveliness_token` egress in the deliver rule + ingress per write
identity, and explicit `**/@adv/**` patterns because `@`-prefixed chunks are verbatim — `**` never matches them).

Today presence is inferred **passively from data traffic** (the fabric-health meter assumes a producer is alive
because it's publishing). That cannot distinguish a mission producer that went *quiet* from one that *died* — a
real false-negative for C2. Liveliness is the native push-based answer: a token declares "I exist," subscribers
are told the instant it drops, late-joiners query the current set.

**Adopt for:**
- The **presence axis** of the environment-health surface ([ADR 1036](../adr/1036-environment-health-surface.md)) —
  each peer declares a liveliness token under `ops/<domain>/presence/<peer>` (namespace TBD in build); a passive
  roll-up renders peer up/down without polling.
- The **subscriber-delivery line** already designed for `goat narrate`/`goat pub` (goat-cli.md §pub --narrate,
  zenoh-trial-fabric.md) — "N subscribers live on this key" via a liveliness fan-out query.

## Gap 2 + 4 — Advanced Pub/Sub + QueryingSubscriber (reconnect catch-up + must-deliver)

These are one family in `zenoh-ext` (`unstable`): `QueryingSubscriber`/`FetchingSubscriber`,
`PublicationCache`, and the higher-level `AdvancedPublisher`/`AdvancedSubscriber` (`.cache()`, `.history()`,
`.recovery()`, `.sample_miss_listener()`, `.publisher_detection()`).

**Adopt for:**
- **Reconnect catch-up** — a subscriber that on-connect queries history for what it missed, then switches
  seamlessly to live. This is the core of our resilience pitch ("survives partitions") and is currently
  hand-rolled per consumer (or absent). Standardize an `AdvancedSubscriber::history().recovery()` wrapper in
  `panoscope-core` so every inspection surface (panoscope, flashlight, stethoscope) inherits correct
  catch-up. Answers "I reconnected — show me what happened while I was gone."
- **Must-deliver producers** — `AdvancedPublisher::cache()` + `sample_miss_listener` is the **native answer to
  F-178**: automatic sample-miss detection + recovery *is* the producer-cooperation we were going to hand-roll
  as acked Query/Reply. We keep `goat pub --confirm` as the explicit one-shot path; AdvancedPublisher is the
  continuous-stream path.

**Brokerless-safe — this does NOT reintroduce the broker we rejected.** Per the standing warning in
[`zenoh-brokerless-consequences.md`](zenoh-brokerless-consequences.md): the `PublicationCache` lives in the
**producer's own session** (or a co-located storage), serving retransmission queries for **that producer's own
keys only**. It is per-producer, not a central funnel — no message-rate bottleneck, no single QoS enforcer, no
central delivery-lifecycle authority. It is the cooperative, edge-local reliability the brokerless model always
intended; it adds it *without* a central component.

## Gap 3 — HLC timestamping (assert the default; ordering substrate for ADR 1037)

Router config default is `timestamping.enabled = router: true` (zenoh-config 1.9.0 defaults). So routers
**already** stamp samples with an HLC, and storages **already** use it for last-writer-wins + cross-router
alignment — the latent multi-site correctness concern is covered by default. Two actions remain:

1. **Assert it explicitly** in our router config rather than inheriting the default, so a future config edit can't
   silently disable LWW correctness. (Verify current router config; add `timestamping: { enabled: true }` if absent.)
2. **Consume it.** [ADR 1037](../adr/1037-fabric-internal-time-synchronization-first.md) makes
   *internal-coherence ordering* (independent of UTC) the foundation of our time model. Zenoh's HLC is exactly
   such an internal-coherent logical clock. The application ordering/fusion layer should consume the sample HLC
   timestamp as its ordering clock rather than building a parallel one. (Design hook; not a build item here.)

## Encoding field — self-describing payloads (free win)

Zenoh 1.x carries a real `Encoding` on every sample. We publish payloads with the encoding left default and the
format declared **out-of-band** (portal registry `topic.format = protobuf:<MsgName>`). Wiring the encoding —
`Encoding::APPLICATION_PROTOBUF.with_schema("<MsgName>")` on publish, `sample.encoding()` on the consumer — lets
panoscope/flashlight/the schema-viewer pick a decoder from the wire itself instead of an out-of-band lookup.

Additive and display-only. The portal `topic.format` stays the registry SoT (it's how the viewer fetches the
`.fds`); the wire just stops being mute. Apply on the publishers we own (synth-vendor, rest-pub-bridge, `goat
pub`) and read it in the decode path.

**Set the encoding from the format the peer already declares — don't hand-classify.** Synth peers publish a
**mix** (json, cbor, **and protobuf** for some); each peer already computes a `format` string for its
`attest/schema/**` schema-decl. Drive the publisher's `Encoding` off that same string
(`json → APPLICATION_JSON`, `cbor → APPLICATION_CBOR`, `protobuf[:<Msg>] → APPLICATION_PROTOBUF.with_schema(<Msg>)`,
`xml[:<Root>] → APPLICATION_XML` — the `<Root>` binds to an uploaded `.xsd`, mirroring the protobuf `.fds` path)
so there's one source of truth and we never decide which synth is which here. `rest-pub-bridge` already sets
`.encoding()` from the inbound HTTP content-type (the one encoding-aware publisher today); this generalizes the
pattern to the rest.

## Source provenance + router ZID pinning ([ADR 1041](../adr/1041-zenoh-router-zid-pinning-and-source-provenance.md))

A sibling identity review (the ZID write-up, same 2026-06-11 cycle) found our Zenoh-identity posture strong on the
load-bearing axes — cert is the trust anchor (`cert_common_names` ACL, mTLS enforced), transport is TLS with
explicit `connect`-lists (no multicast scout on exposed segments), version pinned `=1.9.0` — with two gaps:

- **Routers minted a random ZID each restart.** The ZID identifies a Zenoh *runtime*; left unset it is random per
  session, so the same router reads as a new origin every bounce — churning the routing graph, the audit-subscriber
  `(src_zid, key_expr)` dedup key, and panoscope topology. **Fixed:** pin `id` deterministically per host in the
  three long-lived router roles (`zenoh`/`goat-zenoh`/`nova-lan-zenoh`) — truncated lowercase SHA-256 of the
  inventory hostname + realm (16 bytes). Derived, never baked into the image (cloneable-artifact collision trap),
  never from `machine-id` (VM clone-collision source). The sandbox router stays random.

- **`source_info` was never attached, so the provenance consumers already read came back empty.** flashlight and
  panoscope already call `sample.source_info().source_id().zid()`, but a plain `put()` leaves it `None`, so they
  fall back to topic-prefix attribution. **Fixed (synth-vendor):** attach `SourceInfo::new(publisher.id(), sn)` on
  the canonical data producer, so a recorded sample traces to a real — and, with the pinned ZID, restart-stable —
  origin. This is the **same wire mechanism the AdvancedPublisher auto-attaches** (gap 5): plain producers attach
  manually, reliability producers get it for free on conversion, so the env-health publishers are deliberately not
  hand-wired here. `goat pub` (no `unstable`) can't attach it — accepted gap; `rest-pub-bridge` is the bridged path
  for non-Rust producers if provenance there is required.

Both stay strictly **labels**: the ZID never becomes an authz input, and site/region stays in the namespace +
cert URI-SAN, not a structured ZID prefix (the [deliberate delta](https://github.com/usareur-af/goatdocs/blob/main/docs/design/deliberate-deltas.md) §10.9). No router fork.

## Watch-items (capture, do not implement yet)

- **MatchingListener / matching-status** — `Publisher::matching_listener()` fires when a key gains/loses
  subscribers; useful to *suppress* publishing when nobody's listening (save CPU/bandwidth on the denied edge).
  Not needed until a measured idle-publish cost justifies it. Revisit alongside the fabric-health active
  idle-probe roadmap.
- **zenoh-nostd (a Zenoh client for microcontrollers)** —
  [`eclipse-zenoh/zenoh-nostd`](https://github.com/eclipse-zenoh/zenoh-nostd) is a pure-Rust Zenoh client that
  needs no standard library and no heap. Watched, not adopted: it cannot present a client certificate, so it
  cannot join a goat fabric directly, and no goat component runs Zenoh on a device that small today.
  - *What it offers (read from its source at tag `0.2.0`, 2026-11-16):* put, publisher, subscriber, get and
    queryable in client mode; platforms for ESP32-S3 (Embassy), ordinary hosts and the browser
    (`wasm32-unknown-unknown`); a small broker that joins several clients and can forward to a Zenoh network;
    a `zenoh-sansio` crate that holds the transport state machine with no I/O attached. Links are `tcp`, `udp`
    and `ws`. Its README marks it as early development; it is consumed as a git dependency and needs Rust 1.91.
  - *What it lacks for goat:* TLS, and with it the client certificate every goat router requires; the `serial`
    link (parsed, not implemented, upstream issue #11); the Interest protocol (upstream issue #46).
  - *How a device that small would join:* behind a gateway peer that holds the fabric certificate, has a
    recorded owner and publishes for the device, the same shape as the REST publish bridge
    ([ADR 1023](../adr/1023-zenoh-rest-pub-bridge-for-non-rust-publishers.md)). Designed, not built.
  - *Revisit when any one of these is true:* upstream adds TLS with client certificates; upstream implements
    the serial link, which makes it a candidate for bearers below IP; a microcontroller-class publisher is
    required by a goat component or a partner integration.
  - *Lowest-risk first trial:* the browser playground's isolated router, where the client would speak the
    native protocol over `ws` and the remote-api plugin would not be needed.
- **Hiroz (a ROS 2 stack written in Rust on Zenoh)** —
  [`ZettaScaleLabs/hiroz`](https://github.com/ZettaScaleLabs/hiroz) lets a robot program publish and subscribe
  to ROS 2 topics with no ROS installation, and it interoperates with ROS 2 systems that use the Zenoh
  middleware (`rmw_zenoh`). Watched, not adopted: no goat component speaks ROS 2 today, and its README marks
  it as experimental.
  - *What it offers (read from its README and workspace manifest at version `0.2.1`, 2026-11-16):* a pure-Rust
    ROS 2 implementation built on the `zenoh` crate at `1.9.0`, the version the goat fleet pins; a middleware
    crate (`rmw-zenoh-rs`) so existing C, C++ and Python ROS 2 programs keep working; Python and Go packages;
    code generation for ROS 2 message types. It is tested with ROS 2 Humble, Jazzy, Kilted and Lyrical.
  - *What a goat integration has to settle:* the workspace enables only the `transport_tcp` and
    `transport_serial` features of `zenoh`, so a build that joins a goat fabric must add `transport_tls` to
    present its client certificate (unmeasured); ROS 2 key expressions follow the `rmw_zenoh` layout, not the
    owner-rooted layout goat routers authorize, so the mapping between the two is a design decision, not a
    configuration value.
  - *Revisit when any one of these is true:* a robot or vehicle that runs ROS 2 has to publish to or read from
    a goatnet; a conversion between ROS 2 messages and another format is built and needs a ROS 2 endpoint to
    prove it against; upstream declares a stable release.

## What stays behind the router-image boundary (no change)

- **Shared-memory transport** — a zenohd compile feature; only pays off for co-located containers on one Orin.
  Correctly deferred (ADR 0858 addendum).
- **Router `stats`/`/metrics`** — a zenohd compile feature; the aggregate denial counter is largely redundant
  with the expected-vs-actual monitor. Correctly deferred.
- **Custom interceptor plugins / zenohd fork** — hard boundary (ADR 0858). We prefer external mTLS-peer services
  (the audit-subscriber pattern). Unchanged.

These are the *real* "own the build" items. Adopting Gaps 1/2/4 above does **not** move us toward owning the
build — that is the F-193 correction.

## Build + rollout order, and status

Client-side, no router fork, stock `eclipse/zenoh:1.9.0` throughout. Operator runs the deploys (Ansible);
this plan prepares code + config.

| # | Increment | Touches | Status |
|---|---|---|---|
| 0 | Verify `zenoh-ext = "=1.9.0"` (`unstable`) resolves in our workspace (pulls `zenoh/internal`) | a throwaway build + one client crate | **done** (2026-06-11 — `cargo check` exit 0; deps resolve + compile, `AdvancedPublisher/SubscriberBuilderExt` present) |
| 1 | Encoding field on publishers + decode-path read | synth-vendor, rest-pub-bridge, `goat pub`, panoscope/flashlight decoder | **built (code) — operator live-fire remains.** **Publishers:** synth-vendor wired (`Personality::wire_format` → `format_to_encoding`, handles `protobuf:<Msg>`); rest-pub-bridge already encoding-aware; **`goat pub --encoding`** wired (was a no-op flag; `goat_encoding` mirrors the same `json`/`cbor`/`text`/`protobuf:<Msg>` vocabulary). **Consumers:** **flashlight** reads `sample.encoding()` into an encoding-authoritative `SampleFormat` (new `Protobuf`/`Cbor` variants + `classify_with_encoding`), carries it on the SSE event, and the decode path prefers the wire `protobuf:<Name>` after the portal SoT (also decodes protobuf when the portal is down); UI gets protobuf/cbor glyphs. **panoscope** captures the wire encoding onto `SampleEntry` (data-tail + stale-recall) and the decoder selects the message via the wire `protobuf:<Name>` when the portal row declares no `format` (resilience fallback before the first-message-in-FDS guess). The portal `topic.format` stays the schema SoT throughout. |
| 2 | Assert router `timestamping: { enabled: true }` | both router configs (`zenoh/`, `goat-zenoh/`) | **done in code** (asserted in both `config.json5.j2`; operator deploy pending) |
| 3 | Liveliness presence + subscriber-delivery line | env-health roll-up, `panoscope-core`, goat-cli narrate | **DEPLOYED + live-fired on ZAGROS 2026-06-13 (M-066):** `goat presence` showed the live peer set, the honesty cycle ran green→amber(drop-in-window)→green across a synth kill/restart, and `--watch` showed the leave edge the instant the session died. Built: ACL grants landed (goat-zenoh + nova-lan-zenoh; F-205). `panoscope_core::presence` (key convention `<write_root>/presence/<label>` + token/watch/query helpers, stable zenoh API so goat-cli consumes it). synth-vendor declares a presence token per peer (best-effort — warns and continues against a pre-grant router). New `goat-presence-health` meter (presence domain of ADR 1036, `ops/presence/health/<site>`, `EnvHealthV1{domain:"presence"}` oneof arm 13; honesty: never-seen→idle, alive+quiet→green, drop-in-window→amber, all-gone→red) + `presence-health` ansible role + image workflow. `goat presence` (one-shot set / `--watch` join-leave edges, alias-rendered; `--subscribers-of <topic>` = the subscriber-delivery line via `@adv/sub` detection tokens — a floor, not a census). |
| 4 | `panoscope-core` AdvancedSubscriber reconnect-catch-up wrapper | panoscope-core + its three consumers (mind the three lockfiles) | **built (code)** — `panoscope_core::advanced_sub` (`CatchupConfig` + `declare_catchup_subscriber` → `CatchupSubscriber`); flashlight wired (recovery-only); 4 unit tests; three lockfiles refreshed. Operator live-fire pending. See §"The catch-up wrapper" below |
| 5 | AdvancedPublisher must-deliver path | producer crates + zenoh-dlq.md reconcile | **DEPLOYED + live-fired on ZAGROS 2026-06-13 (M-066):** `@adv/pub` publisher-detection tokens live (one per data producer; a plain put() declares none) prove the must-deliver chain is active. Built: synth-vendor's data publisher is an `AdvancedPublisher`: per-producer `cache(64)` (a queryable in the producer's own session under `<topic>/@adv/**` — brokerless-safe, per-producer, not a central funnel), `sample_miss_detection` (sequence numbers turn the gap-4 wrapper's recovery into true known-miss gap-filling), `publisher_detection` (liveliness token at `<topic>/@adv/pub/…`). SourceInfo now auto-attached by the AdvancedPublisher (supersedes the manual ADR 1041 attach — exactly the "free on conversion" path it anticipated). `goat pub` stays plain (no `unstable`); rest-pub-bridge conversion deferred until a bridged producer needs the reliability tier. zenoh-dlq.md banner reconciled. |

Per-increment live-fire on nova-a then ZAGROS is operator-run.

### The catch-up wrapper (increment 4, built)

`panoscope_core::advanced_sub` is the workspace's single AdvancedSubscriber construction site, so the
history/recovery policy lives in one reviewed spot instead of hand-rolled per consumer:

- `declare_catchup_subscriber(session, key_expr, &CatchupConfig) -> CatchupSubscriber`. The returned
  `AdvancedSubscriber<FifoChannelHandler<Sample>>` derefs to its channel receiver, so a consumer's existing
  `subscriber.recv_async()` loop is unchanged — only the declaration line swaps.
- `CatchupConfig` separates the two behaviours the AdvancedSubscriber bundles: **on-connect history**
  (`detect_late_publishers` / `max_samples` / `max_age_secs`) and **ongoing sample-miss recovery**
  (`recovery_period` → `RecoveryConfig::periodic_queries`). Constructors: `default()` (history + recovery, age-capped
  at 1h so a reconnecting subscriber catches up the recent past not the storage's full 7d TTL), `recovery_only(period)`
  (heal drops during a session, no on-connect backlog), `live_only()` (uniform call site, no history, no recovery).
- **Consumer fit drove the choice, not a blanket default.** flashlight is wired with `recovery_only(5s)` because its
  own design is a *live tail* (deep replay comes off the audit-tail, not the subscriber) — it should heal samples
  dropped mid-session without dumping a backlog when an operator opens a tail. panoscope's broad `**` data-tail is
  **deliberately not auto-wired**: it already has its own `replay_schema_history` + on-demand stale-recall `get`, and
  blanket `history()`+periodic `recovery()` across `**` would amplify queries over the whole keyspace; if it adopts the
  wrapper it should be via `live_only()` (uniformity, no behaviour change). stethoscope has no live subscriber (it is
  query-based), so it is out of scope.
- **Honest limit (the gap-5 dependency):** against today's plain publishers, `recovery` re-queries the latest retained
  value on its timer and `history` pulls from the router's per-`<peer>/**` rocksdb storage. True *sequence-numbered*
  gap-filling (knowing sample N was missed) only activates once publishers adopt `AdvancedPublisher` sample-miss
  detection (increment 5). The wrapper is the call site that makes that upgrade a publisher-side change, not a
  re-plumb of every consumer.

## Invariants preserved

- **Stock router** — no zenohd fork, no router rebuild (ADR 0858 intact).
- **Brokerless** — every construct here is per-peer/client; the PublicationCache is per-producer, not central.
- **Identity-bound ACL** — liveliness variants already ACL-gated by identity; no new authz surface.
- **Display-only encoding/alias invariants** — encoding is additive metadata; no machine path changes.
