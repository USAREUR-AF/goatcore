# Outbound republisher — the other half of the symmetric publishing rule

`goat-fed-republisher` subscribes to the release sets the agreement declares for **our** flow,
and republishes each sample under `from/<our sovereign_id>/`, the wire name.

## Where it runs

**On our core host, the host the peering edge dials, and both of its sessions dial our core
router.** The local session subscribes to each offered `release/<set>/**`; the publishing session
puts each sample under `from/<us>/<set>/**` on the same core. The edge already carries
`from/<us>/**` from our core to the partner, so the partner-facing edge has no listener and no rule
for this process.

| | |
|---|---|
| Deployed by | `ops/ansible/peering-republisher.yml`, role `roles/goat-fed-republisher`, peering phase 12 after the edge |
| Inventory | the core host in group `<peering>_core`, under `[peering_cores:children]` |
| Image | `ghcr.io/<owner>/goat-fed-republisher`, built by `.github/workflows/goat-fed-republisher-image.yml`, pinned by digest as `components.goat-fed-republisher` in `ops/pins/fleet-pins.yaml` |
| Identity | one leaf for both sessions, `republisher.pem`, CN `fed.<us>.<them>.republisher.01`, minted in phase 8 by our CA |
| Admitted by | the core subject `fed-rp-<them>` that `goat federate apply` renders into the ACL fragment: read the offered `release/<set>/**`, write `from/<us>/**`, nothing else |
| Worked when | `/metrics` on loopback reads `local_subscribed 1` and `edge_linked 1`, and the log has one `release … -> wire …` line per offered set |

It is the **only writer of `from/<us>/**`** on our side. Stopping its container stops our data
crossing without touching the edge.

## Why this is a process and not a router feature

[`federation-package-spec.md` §3.2](../../../docs/design/federation-package-spec.md) states one
symmetric rule: every party publishes across the link under `from/<its own sovereign_id>/`, and
each edge grants the counterparty write access under `from/<the counterparty>/` and nowhere
else. That prefix is the **only origin claim that is enforced rather than asserted**, because
the access rules bind it to that party's peering identity.

Two names, two jobs:

| | What it is | Where it lives |
|---|---|---|
| `release/weather/**` | the **staging** name for what we agreed to export | inside us, before anything crosses |
| `from/zagros/weather/**` | the **wire** name — where our data appears once it has crossed | on the link, and on the receiver |

**F-446: the fabric cannot rename a key expression in transit.** The namespace mechanism attaches
to local application sessions, not to forwarded traffic, so lifting the staging name onto the wire
name inside the router would mean maintaining a fork. It is instead a small process on our own
side, over our own data — which is also the only place it *should* live, since it is our claim
about our own origin.

## The key mapping

**The wire key is `from/<our sovereign_id>/` followed by the staging key with its first path
segment removed.**

```
release/weather/forecast        ->  from/zagros/weather/forecast
release/weather/eu/de/hourly    ->  from/zagros/weather/eu/de/hourly
release/imagery/tile/7          ->  from/zagros/imagery/tile/7
```

One sentence, implementable by a partner from the agreement alone, and it is what a partner
expects: the measured drill's inbound half carried `from/any/sensors/temp`, not
`from/any/release/sensors/temp`.

Two rules were rejected and it is worth saying why:

- **Keep the whole staging path** (`from/zagros/release/weather/forecast`) — puts our staging
  vocabulary, which §3.2 says never leaves us, on the wire and into the partner's key tree.
- **Strip the matched pattern's whole literal prefix** (`from/zagros/forecast`) — maps
  `release/weather/forecast` and `release/imagery/forecast` onto one key and silently merges two
  feeds.

Injectivity is **checked at startup, not assumed**: if two release prefixes in the agreement
would map onto intersecting wire keys, the process refuses to start and names both.

## Provenance — what survives the hop

Republishing creates a new sample. A republisher that silently restamps data is rewriting
provenance, so each field is a stated decision rather than a default.

| Field | Treatment |
|---|---|
| payload | forwarded byte-for-byte |
| encoding | forwarded verbatim |
| attachment | forwarded verbatim — the release set governs keys, and everything under a released key is released |
| sample kind | `Put` and `Delete` both forwarded |
| **timestamp instant** | **preserved** |
| **timestamp identifier** | **replaced** with our peering session's ZID |
| **zenoh `SourceInfo`** | **dropped**, and counted |

**The instant is preserved** because the receiver uses it for last-writer-wins and for freshness.
Restamping at the crossing would tell the receiver the data was observed when the link happened
to carry it — false, and false in the direction that makes stale data look fresh.

**The identifier is not**, because it is the ZID of one of our internal nodes. The receiver cannot
verify it — the drill record already states that provenance markers cross unverified, with nothing
comparing them to the transport peer — so it carries no evidence, while a node identifier is
exactly the inventory a sovereign does not hand over
(`cross-sovereign-peering-transport.md` §1,
"Disclosure"). Replacing it makes the timestamp say what is true *and* enforceable: this instant,
asserted by the party publishing under `from/zagros/`.

**`SourceInfo` is dropped** for those two reasons plus a third: it carries the origin publisher's
own sequence number, which means nothing in our sequence space. Forwarding it would place a
second, unverifiable origin claim beside the one the far edge actually enforces, and a reader
comparing the two would have no way to know which is load-bearing.
`goat_fed_republisher_source_info_dropped_total` counts it, so the choice is visible.

### The honest gap

**A sample that arrives with no timestamp crosses with none, and the receiving fabric stamps it on
arrival with the CROSSING time, not the observation time.** No republisher can invent an instant
that was never recorded. `sample.timestamp()` is `None` whenever nothing upstream had HLC
timestamping enabled — our routers do
([`roles/goat-zenoh`](../../ansible/roles/goat-zenoh/templates/config.json5.j2)), but a peer or
client session need not.

`goat_fed_republisher_untimestamped_total` counts these. **A non-zero value means the receiver's
clock for that feed is our link's clock.** The fix is upstream — enable timestamping on the
publishing session — not here.

## Loss and backpressure

**Drop, bounded, counted. Never block, never buffer unboundedly.** The choice is not configurable:

- **Blocking** would apply backpressure through the local subscription into the local fabric. A
  federation partner is the least trusted client on the system, and
  [ADR 1123](../../../docs/adr/1123-client-demand-must-not-amplify.md) is that client demand must
  never amplify into fabric load. A partner who stops reading must not slow down our own bus.
- **Unbounded buffering** would let the partner's downtime choose our memory footprint.

One bounded hand-off queue (`--queue-depth`, default 4096). Full means the arriving sample is
dropped and counted. **When the edge session has no transport peer the sample is dropped before
the put, not after** — a put on a linkless client session succeeds locally and is discarded by the
transport, which is loss no counter would ever see.

What the operator sees:

| Metric | Meaning |
|---|---|
| `goat_fed_republisher_edge_linked` | 0 the moment the publishing session loses our core router |
| `..._dropped_total{reason="edge_down"}` | climbs at the feed's real rate while that session is down, so its cost is a number |
| `..._dropped_total{reason="queue_full"}` | the edge is up but slower than the feed |
| `..._dropped_total{reason="publish_error"}` | the put itself was refused |
| `..._refused_total{reason}` | the **agreement** refused it: `not_released`, `oversize`, `rate_shed`, `cardinality_shed`, `agreement_expired`, `unmappable` |
| `..._received_total{prefix}` / `..._published_total{prefix}` | the two ends of the pipeline, per release prefix |
| `..._untimestamped_total` | see the honest gap above |
| `..._agreement_expiry_seconds` | alertable **before** the flow stops, not after |

**What these metrics do not see:** the partner link. The publishing session dials our core, so
`edge_linked` and `edge_down` describe that session; with the core reachable and the partner link
down, samples reach our core and go no further. The round trip (`../roundtrip/`) and the partner's
own store are what show that. And a sample put while `edge_linked` was 1, on a link that failed
before the transport flushed it, is lost with no local signal. That residual is
inherent to a one-way put and closes only if the receiver reports back, which no part of this
design does.

## Configuration is the agreement, not a list

`--agreement <path>` is the composed agreement (spec §8); `--sovereign-id` selects the one flow
that originates with us. There is no our-form and their-form of the file — the partner runs the
same code path against the same bytes and gets their own flow.

Everything comes from it:

| From the agreement | Used as |
|---|---|
| `flows[from == us].release` | the local subscriptions |
| `flows[…].publish_under` | re-derived and compared; a mismatch means the file was hand-edited, and is a refusal |
| `envelope.bytes_per_message` | oversize samples are dropped **whole**, never truncated |
| `envelope.hz` | shed above the receiver's declared rate |
| `envelope.distinct_keys` | a new wire key beyond the ceiling is refused; keys already admitted keep flowing |
| `expires` | refused at startup, and the flow stops on the day after without an operator touching anything |

The ceilings are the **receiver's**, and a sender cannot raise anyone else's ceiling. Shedding
locally is not politeness: shedding at their edge produces a denial we never see, so
local is the only place the shed is visible to us at all.

`--check` validates the agreement, builds the mapping, prints every `release -> wire` line and
exits. It opens no session and touches no host.

### One divergence found, not fixed here

`ops/federation/reference/internal/fedpkg/compose.go` emits an agreement that does **not** match
[spec §8.1](../../../docs/design/federation-package-spec.md) or the conformance vectors in
[`../vectors/`](../vectors/): `envelope.bytes` for `envelope.bytes_per_message`, no `expires`,
`transport.reachability` for `transport.listener`, plus `fips` and `decisions`. **The spec and the
vectors are the contract**, so this parses those and refuses an unknown member inside `flows` —
loudly, rather than reading a `release` list out of a file it does not understand. Reconciling the
Go reference is lane A's.

## Build and test

```bash
cd ops/federation/republisher
cargo test          # no fabric, no host, no network
cargo build --release
```

Standalone crate with its own workspace and lockfile, matching the sibling long-running fabric
components (`ops/zenoh-trial-peers/audit-subscriber`, `…/fabric-crawler`). zenoh pinned `=1.9.0`
per [`zenoh-version-pinning.md`](../../../docs/design/zenoh-version-pinning.md). No dependency is
new to the repository.

## What this does NOT prove

This closes the code gap. It does not close the evidence gap, and the S5 criterion is a measured
one. **A live run must still demonstrate, on a real link:**

1. **A sample crosses under its wire name.** Publish to `release/weather/forecast` on the ZAGROS
   side; observe `from/zagros/weather/forecast` — and *only* that key — in the partner's own store.
   The measured drill
   (`../../federation-drill/cloud/RESULTS.md` case 4)
   carried `release/weather/forecast`, its staging name. That row is what this replaces.
2. **Outbound origin is enforced, not merely asserted.** A write from our peering identity to a
   key outside `from/zagros/**` must be **refused by the partner's edge**, logged there, and the
   refusal must fire again when repeated (F-177: a cached decision is logged once, so a single
   observation cannot distinguish "refused" from "never attempted"). Inbound has this
   (RESULTS.md case 2, `ops/fabric/hijack`); outbound has not been observed at all.
3. **The link comes back on its own.** Restart our core router under load and observe
   `edge_linked` fall and rise, `dropped_total{reason="edge_down"}` account for the gap, and the
   feed resume with no operator action. A restart of the peering edge is seen by the round trip,
   not by these metrics.
4. **The timestamp instant survives end to end.** Compare the timestamp on the sample in the
   partner's store against the instant it was staged with. Nothing local can prove this: it
   depends on the receiving router accepting a timestamp it did not mint.
5. **The envelope holds under real load.** `hz`, `bytes_per_message` and `distinct_keys` are
   enforced here against numbers the agreement declares; whether those numbers are the right ones
   is a measurement on a real link, and the cloud drill measured one partner at one rate.

Until 1 and 2 are observed on a live link, the symmetric rule remains **proven inbound and
implemented outbound** — which is better than asserted, and is not the same as measured.
