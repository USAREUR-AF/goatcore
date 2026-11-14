# Pub/sub best practices on the goat data fabric

This is a vendor-facing explainer for the most common mental-model trips
new participants hit when they first publish to the fabric.
**The single most common one is treating pub/sub like an S3 bucket — it
isn't.** This doc tells you the model that actually applies, gives you
the four design patterns that cover ~95% of real use cases, and shows
you the three footguns that produce the symptom "I published but my
subscriber didn't receive anything."

The fabric is Zenoh under the hood, but the principles in this doc apply
to any pub/sub system you might bridge in later (MQTT, NATS, Kafka,
Solace, etc.). If you've worked with those, skim to §3 (footguns).

---

## 1. The mental model: a topic is a **channel**, not a bucket

If you come from object-storage backgrounds (S3, GCS, Azure Blob), it's
natural to reach for the model "every object gets a unique key; you PUT
the bytes; you GET the bytes back by key." That model is wrong for
pub/sub. Here's what's actually happening:

| You expect (S3 model) | What actually happens (pub/sub model) |
|---|---|
| Each PUT creates a uniquely-keyed object that lives until I delete it | Each `put` is a **message** broadcast onto a **channel** named by the key. Live subscribers on that channel receive it; if no one's listening, the message is gone. |
| GET retrieves the object stored at that key | Two different operations: `subscribe` watches a channel for **future** messages, `get` queries a **storage** that may or may not be configured for that channel. |
| The bucket is the namespace; individual object keys hold the data | The **topic path is the channel name**. Subscribers can pick one channel, or pattern-match many channels at once. Choosing what to put in the path is a design decision with real consequences. |
| Object names are arbitrary identifiers | Topic paths are **patterns subscribers will match against**. If your path embeds random UUIDs that subscribers can't predict, only wildcard subscribers will work. |

The corollary that matters: **the topic path is your contract with
subscribers, not just a unique label for your data.**

---

## 1b. The stable-key contract — the one rule that keeps the fabric fast

**Your topic keys are a small, fixed set you declare once. The things that vary
per message — frame number, timestamp, event id, sensor reading — go in the
payload, not in the key.**

This is not style advice. Every distinct key you publish becomes a permanent
entry in the router's routing table, guarded by one lock; broad reads hold that
lock while they walk it. A publisher that mints a key per message grows the
table without bound and the fabric eventually stops answering *everyone*, not
just that publisher. Measured on a faithful replica: a read costing 1.8 ms idle
took **4.2 seconds** with that pattern in play
(M-093).
This cannot be fixed with bigger hardware — more cores made it worse, because
the lock is serial.

```python
# WRONG — a new key per frame. 64,000 frames = 64,000 routing entries.
session.put(f"{me}/video/frame/{frame_id}", jpeg_bytes)
session.put(f"{me}/telemetry/{sensor}/{timestamp}", reading)

# RIGHT — one stable key; the varying part rides in the payload.
pub = session.declare_publisher(f"{me}/video/frame")     # declared ONCE
pub.put(encode({"frame_id": frame_id, "image": jpeg_bytes}))

pub_t = session.declare_publisher(f"{me}/telemetry")
pub_t.put(encode({"sensor": sensor, "ts": timestamp, "value": reading}))
```

**How to tell which side of the line a dimension falls on:**

| The dimension is… | Where it goes | Why |
|---|---|---|
| something a subscriber would want to **subscribe to separately**, and the set is **small and bounded** (a handful of streams, a fixed entity roster) | **the key** | that is what keys are for — routing |
| **unbounded or ever-growing** (frame/event/message id, timestamp, per-reading id) | **the payload** | it would mint a routing entry per message |
| needed for **historical retrieval** of every event | **payload + a history store** | see §4 — storage keeps the latest value per key, so a new key per event is not how you get replay either |

**Your allowance.** Each tenant gets a bounded number of distinct keys; the
starting allowance is small (a handful), a larger one needs a sponsor, and a
large one needs platform-admin approval. Usage is measured continuously and
per-tenant — you do not have to self-report, and you will be told before you
become a problem for others. Well-behaved publishers land far under the
allowance without trying: a video tenant that follows the contract uses about
ten keys where the same tenant minting per frame used tens of thousands.

**Rule of thumb:** if you are building the key with an f-string that
interpolates something which increments, stop — that value belongs in the
payload.

## 2. Four topic-design patterns (pick one consciously)

The decision you have to make for each piece of data you publish: what
**shape** does your topic path take, and what does that imply for
subscribers and for storage?

### Pattern A — Single channel, all events: `vendor/system/v1`

One topic; every message goes to the same key.

```python
# Publisher:
session.put("acme/router/cot/v2", cot_xml_bytes)

# Subscriber:
session.declare_subscriber("acme/router/cot/v2", handler)
```

- **Best for:** continuous event streams where consumers want "everything that happened on this channel."
- **Storage behavior:** rocksdb-style storages keep only the **latest** value per key — so this pattern preserves only the most recent event in history. If you need replay of every event, use Pattern C.
- **Subscriber gotcha:** literal key, no wildcard needed. Simple.

### Pattern B — Per-entity state: `vendor/system/v1/<entity-id>`

One topic per logical entity, stable key per entity.

```python
# Publisher (entity X's latest state):
session.put(f"acme/router/cot/v2/{entity_uid}", cot_xml_bytes)

# Subscriber (all entities, live):
session.declare_subscriber("acme/router/cot/v2/*", handler)
# OR get latest known state for a specific entity (queryable history):
session.get(f"acme/router/cot/v2/{specific_entity_uid}", queue)
```

- **Best for:** "latest known state per object" semantics — vehicle tracks, sensor positions, anything where consumers want "what's entity X doing right now?"
- **Storage behavior:** rocksdb keeps the latest message per entity. Storage size scales with **number of entities**, not number of events. `session.get` returns each entity's most recent state — exactly what you want for caching, dashboard rendering, late-joiner catchup.
- **Subscriber gotcha:** wildcard required if you want all entities. Use `/*` to match exactly one segment (the entity UID); use `/**` to match any depth.

### Pattern C — Per-event archive: `vendor/system/v1/<entity-id>/<event-id>` or `vendor/system/v1/<uuid>`

Every message gets its own unique key.

```python
# Publisher (fresh UUID per message):
session.put(f"acme/router/cot/v2/{uuid.uuid4()}", cot_xml_bytes)

# Subscriber (live tap):
session.declare_subscriber("acme/router/cot/v2/**", handler)
# OR query history (returns every event ever stored):
session.get("acme/router/cot/v2/**", queue)
```

- **Best for:** archival / audit / replay workloads where every individual event must be queryable in retrospect.
- **Storage behavior:** every message is a new key; rocksdb stores all of them up to the configured time-to-live (7 days on the trial fabric, longer in production). Storage size scales with **event volume**, which can be large.
- **Subscriber gotcha:** **wildcard is mandatory.** Subscribers using a literal key never match. This is the most common failure mode on the trial fabric.

### Pattern D — Request/reply via queryables: `vendor/service/v1/<method>`

Not a pure pub/sub pattern — Zenoh queryables. Out of scope for this
doc; covered in the portal's `/peering/guide.html` walkthrough.

### Picking between B and C

If a downstream consumer asks "what's the current state of entity X?",
use Pattern B. If they ask "give me every event entity X has ever
emitted," use Pattern C. If both, use Pattern C and have consumers do
the latest-state dedup in their client, OR run two publishers (one B,
one C) on different topic prefixes.

**CoT specifically:** the Cursor on Target XML format already carries
`uid="..."` as an attribute inside the payload — that's the canonical
entity identity. If you're publishing CoT and your consumer wants
"latest known position per track," Pattern B with the CoT uid in the
topic path is correct: `acme/router/cot/v2/<cot-uid>`. Don't mint a
fresh UUID per message for the topic path — that's Pattern C, which has
different consumer ergonomics and accumulates entries proportional to
message rate, not entity count.

---

## 3. The three footguns that produce "my subscriber didn't receive anything"

If you're seeing this symptom, it's almost always one of:

### Footgun 1: Literal subscriber against a wildcarded publish

```python
# Publisher puts to a unique-per-event key:
session.put(f"acme/router/cot/v2/{uuid.uuid4()}", payload)

# Subscriber waits on the literal parent:
session.declare_subscriber("acme/router/cot/v2", handler)   # WRONG — never matches
```

The publisher's key is `acme/router/cot/v2/<some-uuid>`. The
subscriber's literal key is `acme/router/cot/v2`. In Zenoh's
key-expression matching, those are different keys. The subscriber sees
nothing.

**Fix:** add `/**` (any depth) or `/*` (exactly one more segment):

```python
session.declare_subscriber("acme/router/cot/v2/**", handler)   # RIGHT
```

### Footgun 2: Subscriber not alive at publish time

Live pub/sub has **no backlog**. If your publisher fires before any
subscriber declares interest, the message is forwarded to whatever
subscribers exist (could be zero) and then it's gone. Storage helps
late joiners *query history*, but it doesn't replay live samples
retroactively.

```python
# This shape works only if you keep the subscriber process alive:
sub = session.declare_subscriber("acme/**", handler)
# ... your publisher in another window puts messages now ...
# sub is alive — receives them.

# This shape fails:
session.put("acme/router/cot/v2/abc", payload)        # router has no subscribers, drops
sub = session.declare_subscriber("acme/**", h)        # too late
```

If pub + sub are in the same script, declare the subscriber **before**
any put, and give the declaration time to round-trip to the router (a
`session.sleep(0.5)` or wait on a `Liveliness` token).

If pub + sub are in separate processes, **start the subscriber first**
and verify it's actually connected before you fire the publisher.

### Footgun 3: Subscriber session silently failed to connect

`zenoh.open()` doesn't always raise when the underlying TLS link fails
to establish — depending on config, it can return a "session" that's
not actually peered with any router. `declare_subscriber` on that
session succeeds locally but never registers with the router, and no
samples arrive.

**Verify the session is actually peered** before relying on it:

```python
session = zenoh.open(config)
# At minimum: do a session.get against a known-good key and see if it
# returns ANY entries. If it returns nothing, your TLS handshake
# likely failed silently:
replies = session.get("@/*/router", zenoh.Queue())
got_any = False
for r in replies:
    got_any = True
    break
if not got_any:
    print("ERROR: session not actually peered with the router")
    sys.exit(1)
```

The router endpoint, cert paths, and mTLS posture are all in your
onboarding bundle's `first-publisher.py`. If you copy that scaffolding
and your subscriber matches its session-config shape, the silent-fail
mode goes away.

---

## 3b. How you JOIN the fabric — client vs router, and never store `**`

Everything above is about what you publish. This section is about what your
process *is* to the fabric, and it is the rule most likely to cause an outage
that is not yours.

**Join as a client.** `zenohd` — the standalone zenoh daemon — defaults to
`mode: "router"`. A router is a *routing participant*: the goatnet router
forwards queries to it and waits for a final reply, and must push routing and
declaration state to it as **non-droppable** (reliable) messages. A client does
neither. It publishes, subscribes and asks questions, and nothing on the fabric
depends on it answering.

If you run `zenohd` on an edge node and point it at a goatnet with `--connect`,
you have silently become part of that goatnet's routing plane. Set `mode`
explicitly:

```json5
{ mode: "client" }        // a node attached to a SHARED fabric
{ mode: "router" }        // ONLY when this box IS the fabric (isolated bench)
```

**Never declare a storage on `**`.** A storage declares a *queryable* for its
key expression. `key_expr: "**"` declares your process the queryable of record
for the **entire fabric keyspace** — every matching query, fleet-wide, routes to
you. Scope it to your own write root:

```json5
// WRONG — you are now the answer to every question on the fabric
storages: { all_keys_mem: { key_expr: "**", volume: "memory" } }

// RIGHT — you answer only for what you own
storages: { own_keys_mem: { key_expr: "<your-slot-anchor>/**", volume: "memory" } }
```

Usually you need no storage at all: the goatnet router already runs per-slot
storage (ADR 1101), so a second copy on the node is redundant.

The asymmetry worth internalising: **a misbehaving client harms only itself; a
misbehaving router harms everyone.** Choosing client mode costs you nothing and
removes your node from that blast radius entirely.

### And never SUBSCRIBE to everything either — `**` is a named grant

A client cannot wedge the fabric by answering slowly. It can wedge it by
*reading* slowly.

The rule is narrow, and it is about SHAPE rather than about wildcards:

> **No single subscription may match the entire keyspace.** Filter as widely as
> you like; just do not ask for all of it at once.

```python
# REFUSED unless your certificate is named in the goatnet's
# zenoh_fabric_wide_read — and refused SILENTLY:
session.declare_subscriber("**", handler)

# ALLOWED for every peer — these are FILTERS, not the keyspace:
session.declare_subscriber("**/example_vendor/**", handler)  # one vendor, all tenants
session.declare_subscriber("**/airtracks/v1/*", handler)  # one topic, all tenants
session.declare_subscriber("*/geo/track/v1/*", handler)   # one topic, one level down
session.declare_subscriber("<their-anchor>/**", handler)  # one tenant
session.declare_subscriber("ops/**", handler)             # one namespace
```

One wrinkle if you filter across tenants: a `**`-leading shape has to be
**declared** in the goatnet's `zenoh_peer_cross_tenant_read` list, because zenoh
cannot express "any `**`-leading filter" as a single rule — the expression that
would, `**/*/**`, is not a valid key expression (zenoh canonizes `**/*` to
`*/**`).

Your shape is probably already listed. A brand-new one is **denied silently**
: your subscriber declares without an error and then receives nothing,
which looks exactly like a quiet topic. So if a cross-tenant filter you have
never used before returns nothing, ask before you debug — the fix is one line of
platform config:

> **Adding a shape** (platform side): append it to `zenoh_peer_cross_tenant_read`
> in that goatnet's `group_vars`, exactly as you declare it, re-render, deploy
> the `goat-zenoh` role and restart the router (F-156 — SIGHUP does not reload
> the ACL). Tell us the exact key expression and we will add it.

**If you think you need literally everything**, say what for. A sink that
archives the fabric, a bridge that mirrors it, an inspection surface: those are
real, and they get a named grant with an owner and a review date. "It was easier
than naming the topics" is not.

---

## 5. Payloads larger than ~2 MB

Pub/sub fabrics are tuned for **many small messages**, not for a few
large ones. The default behaviors below kick in around 2 MB; expect
silent failure modes above that size unless you actively design around
them.

### What Zenoh does by default

The transport layer fragments large samples across the underlying
TCP/TLS link and the subscriber side reassembles. So a multi-megabyte
`put` doesn't fail at the wire — your publisher returns "clean" and
the bytes reach the router. The failure modes are downstream of that.

### What we've seen on the trial fabric

A 12.5 MB `put` from a properly-configured vendor under their
prefix's storage stanza returned cleanly from the publisher but the
bytes never persisted to the per-vendor rocksdb dir on the router.
The storage backend silently dropped the reassembled sample — no
error to the publisher, no entry in the audit log indicating a
storage commit failure. Live subscribers (if any had been online)
might have received the sample over the wire, but `session.get`
afterward returned nothing.

That's the behavior to expect from any pub/sub system under default
configuration: the wire path is more permissive than the storage path,
so an artifact too big for storage just evaporates after delivery to
live subscribers (and even those have receive-side buffer caps).

### Sizing thresholds (rules of thumb on the trial fabric)

| Payload size | Expected behavior |
|---|---|
| **< 64 KB** | Comfortable across the wire, storage, and audit log. Audit captures the full payload bytes (base64 in NDJSON). |
| **64 KB – 1 MiB** | Live subscribers fine; storage persists; audit captures metadata + payload. Schema-body uploads via the portal use this range (1 MiB hard cap). |
| **1 MiB – 2 MiB** | Live subscribers fine; storage persists; audit captures metadata but drops the payload bytes (`payload_bytes_b64` absent above ~1 MiB). |
| **2 MiB – 16 MiB** | **Treat as not-persisted.** Live subscribers MAY receive (depends on their reassembly buffer); storage drops silently. `session.get` will not return your bytes. |
| **> 16 MiB** | Don't. Even live delivery becomes unreliable. |

### Recommended patterns for artifacts > 2 MB

> **If you don't want the artifact persisted at all** (you're moving a
> bounded large file peer-to-peer, not publishing it for replay), use the
> `volatile/<peer>/blob/**` class — it routes the chunks live but writes
> nothing to rocksdb or the audit bucket, so a 10–100 MiB transfer can't
> fill the router disk. Same chunking as Pattern E below; the difference is
> retention. The receiver must be online for the transfer (no
> store-and-fetch). See [`zenoh-volatile-class.md`](zenoh-volatile-class.md)
> ([ADR 1032](../adr/1032-zenoh-volatile-non-persisted-namespace-class.md)).
> The patterns below are for artifacts you *do* want persisted/queryable.

**Pattern E — Chunk it.** Split the artifact into N keys under a
sub-prefix and publish each chunk under the 2 MB threshold.
Subscribers reassemble in order.

```python
# Publisher:
chunk_size = 1_500_000   # 1.5 MB — comfortably under storage cap
for i in range(0, len(payload), chunk_size):
    chunk = payload[i:i+chunk_size]
    session.put(f"vendor/artifact/openapi/v1/chunk/{i:08d}", chunk)
session.put("vendor/artifact/openapi/v1/manifest",
            json.dumps({"chunks": ceil(len(payload)/chunk_size),
                        "size": len(payload),
                        "sha256": sha256(payload).hexdigest()}).encode())

# Consumer:
manifest = json.loads(session.get("vendor/artifact/openapi/v1/manifest").payload)
chunks = []
for i in range(manifest["chunks"]):
    chunks.append(session.get(f"vendor/artifact/openapi/v1/chunk/{i*1_500_000:08d}").payload)
payload = b"".join(chunks)
```

Each chunk persists in storage independently; consumers `get` the
manifest first, then walk the chunks. Treat the manifest as the
canonical "is the artifact ready" signal.

**Pattern F — Treat the artifact as schema material.** If the
artifact IS a schema (a `.proto`, a `.fds`, or — yes — an OpenAPI
spec interpreted as schema-of-API), upload it to the portal's schema
endpoint with `format=json` (or `format=protobuf-source` for protos).
The portal's schema body cap is 1 MiB per upload, so still needs
chunking for >1 MiB artifacts. Schemas are queryable from the portal
UI at `/schema.html?key=<topic>` without going through Zenoh storage
at all.

**Pattern G — External storage pointer.** Publish a small "blob
handle" message (URL + content-hash + size + content-type) under a
normal topic. Store the artifact in a vendor-owned object store
(MinIO, S3, etc). Consumers fetch the handle via Zenoh, then GET the
artifact from the object store.

```python
session.put("vendor/artifact/openapi/v1/handle",
            json.dumps({"url": "https://vendor-bucket.example/openapi/v1.json",
                        "sha256": "...", "size": 12_500_000,
                        "content_type": "application/json"}).encode())
```

This is the right shape for artifacts that are large AND infrequently
updated AND have existing object-store delivery infrastructure.

**Pattern H — Queryable.** Vendor stands up a queryable that responds
to `session.get` requests with the artifact materialized from their
own process. No fabric storage involved; the artifact lives in vendor
memory or local disk. Good when the artifact is computed or
project-specific.

### What NOT to do

- **Don't** treat `vendor/path/big-artifact` as your single retrieval
  URL with a multi-MB body in the `put`. That's the S3 model from §1
  again; it doesn't work here for payloads above the storage cap.
- **Don't** rely on the publisher's "clean return" as confirmation
  that the artifact persisted. The wire and the storage paths have
  different size budgets; the publisher only sees the wire success.
- **Don't** assume the audit log will tell you a large put dropped.
  The audit subscriber records that the publish happened (with
  metadata) but may not flag a storage-side drop.

### Confirming persistence

For any single `put` you want to know whether storage actually
captured, the canonical end-to-end test is:

1. `put` your bytes
2. Wait ~1 second
3. `get` the same key from a fresh session
4. Compare the SHA256 of returned payload to what you put

For chunked artifacts (Pattern E), additionally verify the manifest
chunks-count matches the count you wrote, and each chunk's hash
matches before assembly.

If `get` returns nothing or returns a truncated payload, the
artifact exceeded a downstream cap. Drop to a smaller chunk size or
move to Pattern G.

---

## 6. The audit log is not your retrieval surface

The fabric's audit subscriber captures every put on `**` into MinIO.
That's for **compliance and forensics**, not for normal consumer
retrieval. Don't build your consumer to scrape MinIO — use `session.get`
or `declare_subscriber` against the router instead.

(If you genuinely need an event archive and don't trust rocksdb's
latest-per-key semantics, talk to us — there's a path that's more
operator-friendly than MinIO scraping.)

---

## 7. Worked example: CoT publisher + subscriber done right

This is the shape we'd put in a vendor's `first-subscriber.py` if we
shipped one in the onboarding bundle. Pattern B (per-entity state)
matches CoT's typical use case.

```python
# publisher.py — publishes one CoT event per track per tick
import zenoh, uuid, time, xml.etree.ElementTree as ET

session = zenoh.open(config)  # config from onboarding bundle's first-publisher.py

while True:
    for track in get_tracks():
        cot_xml = render_cot_xml(track)         # has uid=track.uid inside
        # Use the track's stable uid as the key — NOT a fresh UUID:
        session.put(f"acme/router/cot/v2/{track.uid}", cot_xml.encode())
    time.sleep(1)
```

```python
# subscriber.py — live tap on every track update
import zenoh

session = zenoh.open(config)  # SAME config shape as publisher

# Verify we actually peered (footgun 3 guard):
seen_router = False
for r in session.get("@/*/router", zenoh.Queue()):
    seen_router = True
    break
if not seen_router:
    raise SystemExit("session.get('@/*/router') returned nothing — TLS handshake probably failed silently")

def on_sample(sample):
    print(sample.key_expr, "->", bytes(sample.payload).decode("utf-8"))

# Wildcard match: any entity uid under v2:
sub = session.declare_subscriber("acme/router/cot/v2/*", on_sample)
print("subscriber declared, waiting...")

import time
while True:
    time.sleep(1)
```

Run the subscriber first, leave it running, then run the publisher in
another shell. The subscriber should print one line per CoT track per
publisher tick.

---

## 8. Quick reference: key-expression syntax

| Expression | Matches |
|---|---|
| `acme/router/cot/v2` | The literal key `acme/router/cot/v2`. Nothing else. |
| `acme/router/cot/v2/*` | One more segment, exactly. E.g. `acme/router/cot/v2/abc` (matches), `acme/router/cot/v2/abc/def` (does NOT match). |
| `acme/router/cot/v2/**` | Any depth, including zero segments. E.g. `acme/router/cot/v2` (matches), `acme/router/cot/v2/abc/def/ghi` (matches). |
| `acme/**` | Anything under your vendor prefix. The "fire hose" form. |
| `**` | Everything on the entire fabric, in one declaration. **A named grant** (§3b) — held by the audit subscriber and the inspection surfaces, refused silently for everyone else. |
| `**/tracks/v1`, `*/geo/track/v1/*` | A FILTER across tenants, allowed for every peer. A `**`-leading one must be in the goatnet's declared shape list (§3b). |

When in doubt, use `/**` for subscribers and a stable per-entity key for
publishers (Pattern B). That covers almost all use cases without
surprises.

---

## 9. Delivery reconciliation — knowing what's landing

`put` is **fire-and-forget**: it returns locally even when the router refuses your write (you're not onboarded for that key-expr, or your cert doesn't cover it) or drops it. Nothing tells you. The platform watches this centrally — an operator-side monitor reconciles *which vendors should be producing* against actual sample rates and alerts when an expected producer goes silent (F-177, [ADR 1022](../adr/1022-zenoh-denial-observability.md)). You do **not** have to build delivery assurance to be covered. But if you want **local** knowledge of your own message health, here's the tiered contract — it's the producer vantage of the same expected-vs-actual reconciliation the platform uses.

**Start here — tier 0 (default).** For bulk telemetry, just publish fire-and-forget and rely on the operator monitor. **Do not** confirm every message — a per-message round-trip loads the fabric for no benefit on loss-tolerant streams. If the stream is high-throughput **and** you don't want it persisted (the live sample is all that matters), publish it on the `volatile/<peer>/stream/**` class so it never touches rocksdb or the audit bucket — see [`zenoh-volatile-class.md`](zenoh-volatile-class.md) ([ADR 1032](../adr/1032-zenoh-volatile-non-persisted-namespace-class.md)).

Add a tier *only* where you need it:

- **Tier 1 — self-canary (cheap local health).** Keep a small **intent ledger** (what you published — keys, counts) and *periodically* (e.g. every 30 s, not per-message) read your own recent output back and reconcile:
  ```python
  # publish normally at full rate; separately, on a timer:
  replies = session.get(f"{my_prefix}/**", consolidation=zenoh.ConsolidationMode.NONE)
  landed = {r.ok.key_expr for r in replies if r.ok}
  missing = my_recent_keys - landed         # gap = a delivery problem on your side
  if missing: alert_local(missing)          # you now KNOW, locally — back off / retry / log
  ```
  Catches *total* breakage (your writes stopped landing) within one interval. Use a **unique payload/key** per probe so a stale retained value can't mask a denial.

- **Tier 2 — intent heartbeat (helps the platform catch *partial* failure).** Publish a small heartbeat each interval — `published N samples under <prefix>` — to a `<prefix>/_meta/heartbeat` key. The operator monitor compares your *intended* count to what actually landed and flags **partial** denial (e.g. you sent 100, 60 landed) that a pure "did anything arrive?" check misses. This is the highest-leverage tier: low cost, and it upgrades the platform's central detection for everyone.

- **Tier 3 — acknowledged submission (must-land messages).** For the rare message that *must* be confirmed at publish time, don't use fire-and-forget `put` — submit via **Query/Reply** to an admission queryable that accepts/rejects synchronously (Zenoh's native acked-write; see F-178 on why this — not a dead-letter queue — is the fabric-idiomatic answer). Reserve for control/command-grade writes; it costs a round-trip per message.

The one-shot operator/debug version of tier 1 is `goat pub --confirm` (read-back after a single publish). All of these need *your* client to participate — which is why the operator monitor (which needs no cooperation) is the platform's floor, and these are the local agency you layer on top.

**Runnable reference:** [`zagros-moon-pod/examples/delivery_reconcile.py`](https://github.com/USAREUR-AF/zagros-moon-pod/blob/main/examples/delivery_reconcile.py) is a working shape of tiers 1 + 2 (intent ledger → self-canary read-back → intent heartbeat). It ships in the moon-pod kit; the nova portal serves the equivalent per-topic snippet in its Examples tab.

## 10. Self-check before you ask for help

When something doesn't seem to work, run these in order:

1. **Did the router accept my publish?**
   The portal has a Live View at `https://portal.<site>.<zone>/`
   that shows recent publishes. If your topic appears there, the router
   got your message.
2. **Is my subscriber actually peered?** Run a `session.get("@/*/router", q)`
   and confirm at least one reply. If zero replies, your TLS handshake
   probably failed silently — check cert paths + endpoint hostname.
3. **Does my subscriber's key_expr actually match my publisher's key?**
   Print both side by side. If publisher puts to
   `acme/router/cot/v2/<uuid>` and subscriber declares on
   `acme/router/cot/v2` (no wildcard) — that's footgun 1.
4. **Was my subscriber running BEFORE my publisher fired?**
   If you can't answer this for sure, run them in two separate shells
   and start the subscriber first.
5. **Can I `session.get` what I just put?** If yes, the publish + storage
   path works. If subscribers still don't receive, the issue is purely
   subscriber-side (footguns 1-3).

If you've run those and you're still stuck, post in the feedback
channel listed at the top of the portal's `/peering/guide.html` with:
- Your subscriber's `session.open` call and `declare_subscriber` line
  (just those two — no need to share the whole script).
- One example key your publisher emits.
- Whether the symptom is "no messages received" or "wrong messages received."

That's enough for someone with router access to tell you exactly which
footgun you hit.
