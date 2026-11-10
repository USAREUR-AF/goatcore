# Portal vendor-progress swimlane (Block 88)

Status: in-flight (track `portal-vendor-progress-swimlane`, 2026-05-21).

## Problem

An operator running the nova trial needs a single page that answers
"where is each external vendor on the onboarding curve?" without having
to cross-reference four surfaces (portal vendor list, netbird dashboard
Peers tab, audit-subscriber MinIO bucket, manual asks-around).

Four progress levels per vendor:

1. **L1 — Asked for accounts** (bundle has been minted).
2. **L2 — Logged in to netbird** (a peer named after the vendor exists
   in netbird-mgmt).
3. **L3 — Published hello world** (any publish matching the canonical
   `first-publisher.py` fingerprint we shipped in the bundle).
4. **L4 — Published real data** (any publish under the vendor prefix
   that is NOT the hello-world fingerprint).

Monotonic: once a level is reached, it stays reached. L4 implies L3 in
the UI even if the vendor skipped running the example.

## Non-goals

- Production-grade SLAs around freshness. ~30-60 s detection latency on
  L3/L4 is fine.
- Per-topic **rates**, payload sizes, or live message streams. Panoscope
  owns the live-traffic view. The activity column (below) carries a
  summary-grade rollup — total publishes, distinct topics touched, and
  the most-recent topic — but never rates, payloads, or a per-topic
  time series. The boundary: this page answers "what has this vendor
  done, at a glance"; panoscope answers "what is flowing right now".
- L4 grading (sustained traffic, distinct topics, schema-published). A
  single off-fingerprint publish flips L4. Operator-tunable later.
- Synthetic / notional vendors. The existing `vendors.synthetic` column
  filters them out of the default view.

## Data sources

| Level | Source | Already in the portal? |
|---|---|---|
| L1 | `vendors.bundle_minted_at` | ✓ |
| L2 | NetBird mgmt API `GET /api/peers`, match `peer.name` prefix `peer-<vendor>-` (per the AutoPeerName fork-patch in CLAUDE.md §83C.3) | ✗ — new HTTP client |
| L3 / L4 | MinIO `audit-integrity` bucket NDJSON written by [Block 82S audit-subscriber](zenoh-audit-external-subscriber.md) | ✗ — new MinIO client |

There is deliberately **no in-portal Zenoh subscriber.** The portal has
no native Zenoh client today (the inspector talks to the REST plugin
over HTTP), and Eclipse does not publish a maintained Zenoh 1.x Go
binding (per CLAUDE.md, the airdefenders publisher pivoted to a REST
shim for the same reason). The audit-subscriber Rust service is already
the canonical "subscribes to everything on the fabric" peer; the portal
tails its output instead of duplicating the subscription.

This composes one mechanism for both first-boot replay AND ongoing
detection — see §"MinIO tailer" below.

## Schema

New table in `ops/goat-zenoh-portal/internal/registry/schema.sql`:

```sql
CREATE TABLE IF NOT EXISTS vendor_progress (
    prefix              TEXT PRIMARY KEY REFERENCES vendors(prefix) ON DELETE CASCADE,
    l2_at               INTEGER,
    l3_at               INTEGER,
    l4_at               INTEGER,
    -- Most recent publish under the vendor prefix, any kind. Updated by
    -- the audit tailer on every event from this prefix.
    last_publish_at     INTEGER,
    -- Current peer count for the vendor (peers whose name starts
    -- 'peer-<prefix>-'). Updated by the netbird poller on every poll.
    peer_count          INTEGER NOT NULL DEFAULT 0,
    updated_at          INTEGER NOT NULL
);

-- Cursor for the MinIO audit tailer. One row per audit_subscriber_id
-- (today only "nova-a"; survives a future second subscriber). last_key
-- is the highest object key processed (sort order matches lex order of
-- '{id}/YYYY/MM/DD/HH/{ms}-{n}.ndjson').
CREATE TABLE IF NOT EXISTS audit_replay_cursor (
    subscriber_id       TEXT PRIMARY KEY,
    last_key            TEXT NOT NULL DEFAULT '',
    last_seen_at        INTEGER NOT NULL
);
```

Plus the activity rollup that backs the Activity column:

```sql
CREATE TABLE IF NOT EXISTS vendor_topic_activity (
    prefix          TEXT NOT NULL REFERENCES vendors(prefix) ON DELETE CASCADE,
    topic           TEXT NOT NULL,
    publish_count   INTEGER NOT NULL DEFAULT 0,
    first_seen_at   INTEGER NOT NULL,
    last_seen_at    INTEGER NOT NULL,
    PRIMARY KEY (prefix, topic)
);
CREATE INDEX IF NOT EXISTS vendor_topic_activity_prefix_idx
    ON vendor_topic_activity(prefix);
```

The audit tailer upserts one row per `(prefix, topic)` on every observed
event (same event stream that drives L3/L4 — no extra subscriber),
incrementing `publish_count` and advancing `last_seen_at` via `MAX(...)`
so an out-of-order older replayed event never rewinds recency. The audit
cursor guarantees each event is processed once, so steady state never
double-counts. `ListProgress` aggregates this table into three derived
columns per vendor (correlated subqueries, no extra round-trip):
`publish_count` (SUM), `distinct_topics` (COUNT), and `last_topic` (the
row with the greatest `last_seen_at`).

All tables are created via the same idempotent `db.Exec(schemaSQL)`
path. No `addColumnIfMissing` needed — the tables are net-new.

### L1 is derived, not stored

`vendors.bundle_minted_at` is already the canonical "this vendor has a
bundle" timestamp. Storing it twice would create a drift surface for
no benefit — re-mints don't move it (CLAUDE.md §83E + sha-a3ec19c
ledger row both confirm the field is set at first mint and not
touched on subsequent operations). `ListProgress` joins
`vendor_progress` against `vendors` and projects
`vendors.bundle_minted_at AS l1_at`, so L1 lights up the moment a
bundle is minted with no separate backfill step.

L2/L3/L4 *are* stored because their underlying signals are inherently
ephemeral (peer roster fluctuates; audit data ages out under the 7-day
MinIO lifecycle). Stickiness for those three is enforced via
`COALESCE(existing, new)` on every detector write: once a timestamp is
set, subsequent updates won't move it.

## NetBird mgmt poller (L2)

New package `internal/netbirdpoll/`:

- `Client.ListPeers(ctx) []Peer` — `GET ${NetbirdMgmtURL}/api/peers`
  with `Authorization: Bearer ${NOVA_NB_API_TOKEN}`.
- Background goroutine in `cmd/portal/main.go` ticks every 60 s
  (`--netbird-poll-interval`, env `PORTAL_NETBIRD_POLL_INTERVAL`).
- For each vendor in the registry, count peers whose `name` matches
  `^peer-<prefix>-`. Update `peer_count`. If count ≥ 1 and `l2_at` is
  NULL, set `l2_at = now()`.
- Graceful no-op when `--netbird-mgmt-url` is empty OR the env-var
  token is unset. The portal logs a warning at startup and the L2
  column shows "—" on every row; nothing else degrades.

Endpoint default for the nova-a deploy:
`https://netbird.nova.example`. The portal container reaches
this through its existing networks (Traefik fronts the public DNS) —
no new docker-network surface needed.

## MinIO audit tailer (L3 + L4 + first-boot replay)

New package `internal/audittail/`:

- AWS-SDK-go-v2 S3 client pointed at `--audit-s3-endpoint` (env
  `PORTAL_AUDIT_S3_ENDPOINT`, default `http://100.127.0.186:9000`).
  Path-style addressing + region `us-east-1` matches MinIO's posture.
- Bucket `--audit-s3-bucket` (env, default `audit-integrity`).
- Credentials via env vars `PORTAL_AUDIT_S3_ACCESS_KEY` /
  `PORTAL_AUDIT_S3_SECRET_KEY`. Distinct from the audit-subscriber's
  `NOVA_AUDIT_SUBSCRIBER_S3_*` because the portal needs a different IAM
  policy (read instead of write).
- Background goroutine ticks every 30 s (`--audit-tail-interval`).
- Per tick:
  1. Read cursor row for `subscriber_id="nova-a"` (today only one
     subscriber writes; the design is plural-ready).
  2. `ListObjectsV2` under `nova-a/` with `StartAfter=last_key`.
  3. For each object in lex order: GET, decompress if needed, parse
     NDJSON line-by-line. For each event:
     - First key segment → vendor prefix; skip if vendor row missing
       OR `synthetic = 1`.
     - Update `last_publish_at = max(existing, event.ts)`.
     - Apply fingerprint (see below); set sticky `l3_at` OR `l4_at`.
  4. Update cursor `last_key` to the highest object key seen.
- On first boot the cursor is empty → tailer reads every historical
  object in the bucket once. This IS the first-boot replay. Total
  audit data on nova-a is bounded by the 7-day MinIO lifecycle rule on
  this bucket + the per-bucket 12 GiB cap, so the replay is small.

### Hello-world fingerprint

The bundle's `first-publisher.py` publishes (see
`internal/certmint/certmint.go:496-500`):

- `key = VENDOR + "/hello/v1"`
- `payload = "hello from {} at {:.3f}".format(VENDOR, time.time())`

The tailer matches:

```
key_expr == prefix + "/hello/v1"
&& payload =~ ^hello from <prefix> at \d+\.\d+$
```

The audit-subscriber inlines payloads ≤ 16 KiB as `payload_bytes_b64`,
so the bytes are always available for hello-world events (~30 bytes).

- Match → sticky write `l3_at`.
- Any other publish under `<prefix>/**` → sticky write `l4_at`.
- L4 implies L3 in the UI render even if L3 timestamp is NULL.

## API

`GET /api/progress` — operator-only, returns

```json
[
  {
    "prefix": "aardwolf",
    "display_name": "Aardwolf Labs",
    "synthetic": false,
    "l1_at": 1716340000,
    "l2_at": 1716341200,
    "l3_at": 1716342400,
    "l4_at": null,
    "peer_count": 2,
    "last_publish_at": 1716342400,
    "max_level": 3,
    "publish_count": 142,
    "distinct_topics": 3,
    "last_topic": "aardwolf/track-telemetry/v1"
  },
  …
]
```

- Default filter: `synthetic = 0`.
- `publish_count` / `distinct_topics` are 0 and `last_topic` is omitted
  for a vendor the tailer has seen no publishes from (e.g. stuck at L1
  or L2). They are summary-grade, not live rates — see Non-goals.
- `?include=synthetic` returns all rows; the UI badges synthetic ones.
- `max_level` is derived: `4 if l4_at else 3 if l3_at else 2 if l2_at else 1 if l1_at else 0`.

Operator-only via `requireOperator` middleware. No vendor-scoped read —
this is an operator dashboard. Two of its per-org figures,
`last_publish_at` and `publish_count`, also ride on each org row of
`GET /api/catalog`, which every authenticated principal may read (issue
#1339 item 8, so the Slots page shows "Last publish" to partners on a slot
key); the rest of the row, and the endpoint itself, stay operator-only.

## UI

New page `/progress.html`:

- Top-level "Progress" nav link in the existing nav-grouped header,
  positioned between Home and Catalog. Single page, so it's a plain
  `<a class="nav-link">` (no dropdown).
- Table: vendor (prefix + display_name), 4 swimlane columns
  (L1 / L2 / L3 / L4) each rendering either the local-time formatted
  timestamp or `—`, plus `peer_count`, `last_publish_at`, and an
  **Activity** column.
- The Activity column summarizes what the vendor has published:
  `<N> publishes · <M> topics` with the most-recent topic on a second
  hint line (`latest: <topic>`). `—` until the tailer observes a
  publish. This answers the operator's "I don't think this is up to
  date — what has this vendor actually done?" at a glance without
  bouncing to panoscope; panoscope is still the drill-down for live
  rates and payloads.
- One row per vendor. Default sort: most-recently-active first
  (`last_publish_at` desc, NULLs last); operator can toggle to by-L1
  to see "stalled at L1" easily.
- Reuses existing `app.js` `withAuth()` fetch helper + 3-segment
  theme picker + `nav-grouped` styles. No new CSS file.
- "Show synthetic" checkbox flips `?include=synthetic`.
- **Block 94 sibling buttons ([`zenoh-surface-deep-links.md`](zenoh-surface-deep-links.md)):** the vendor cell carries `diagnose ↗` (stethoscope), `tail ↗` (flashlight), and `panoscope ↗` anchors next to the vendor link. Each is conditional on the corresponding sibling URL being set at this deployment — `/progress.js` pulls them best-effort from `/api/siblings` on page load; an unconfigured sibling just doesn't render its button (URL-only contract, no runtime coupling). Vendor link itself uses `?vendor=<prefix>` (Block 94 canon) while still accepting the legacy `?v=<prefix>` form on `/vendor.html` as a backwards-compat alias.

Nav update touches every static HTML page (`index.html`, `topics.html`,
`inspector.html`, `vendor.html`, `schema.html`, `docs.html`,
`patterns.html`, `api-docs.html`, plus the 4 `peering/*.html` files).
The Progress link goes in the same slot on every page so cross-page
navigation is uniform.

## Ansible role wiring

Additions to `ops/ansible/roles/goat-zenoh-portal/`:

- `defaults/main.yml`:
  - `portal_netbird_mgmt_url: "https://netbird.nova.example"`
  - `portal_netbird_poll_interval: "60s"`
  - `portal_audit_s3_endpoint: "http://100.127.0.186:9000"`
  - `portal_audit_s3_bucket: "audit-integrity"`
  - `portal_audit_s3_access_key: "goat-portal-audit-read"` (fixed contract = the MinIO read-user name; NOT env-sourced — prevents F-188 access-key drift)
  - `portal_audit_s3_secret_key_envvar: "NOVA_PORTAL_AUDIT_S3_SECRET_KEY"`
  - `portal_audit_tail_interval: "30s"`
- `templates/portal.env.j2` adds:
  - `PORTAL_NETBIRD_MGMT_URL`
  - `NOVA_NB_API_TOKEN` (passed through; operator's `.envrc.local`)
  - `PORTAL_AUDIT_S3_ENDPOINT` / `PORTAL_AUDIT_S3_BUCKET`
  - `PORTAL_AUDIT_S3_ACCESS_KEY` / `PORTAL_AUDIT_S3_SECRET_KEY`
    (looked up from the operator-configured envvar names so the role
    doesn't hard-code per-site naming)
- `templates/docker-compose.yml.j2` passes the new `--netbird-mgmt-url`,
  `--netbird-poll-interval`, `--audit-tail-interval` flags.
- Deploy-history ledger row appended with the new image SHA after CI
  builds the change.

The role gates each substrate on its env var being non-empty:

- No `NOVA_NB_API_TOKEN` → L2 column shows "—" for everyone (poller
  no-ops). Doesn't break the rest of the page.
- No `PORTAL_AUDIT_S3_*` creds → L3/L4 columns show "—". Doesn't
  break L1/L2.

Operator runbook stays out of CLAUDE.md proper; lives at
`docs/operations/portal-vendor-progress-runbook.md` (small — env-var
setup + one-shot MinIO user mint + a "first apply" checklist).

## What this does NOT do

- Does not surface live message rates or recent topic samples — that's
  panoscope's job.
- Does not become a replacement for the vendor catalog at
  `/vendor.html`; that's the schema-publishing surface, this is the
  onboarding-state surface.
- Does not own the audit-subscriber's MinIO data — it tails read-only.
  The audit-subscriber's write policy + retention contract are
  unaffected.
- Does not synthesize L2 from publish data when the netbird token is
  unset. Better to show "unknown" than to lie.

## Open follow-ons (queued, not in this track)

- Backfill L2 timestamps from netbird-mgmt's `peer.created_at` if we
  later care about historical "when did they actually log in" — today
  the first poll after a new peer flips `l2_at` to the poll time
  rather than the actual login time.
- Per-vendor drill-down page (`/progress/<prefix>.html`) showing the
  full publish timeline. Out of scope; operator can click through to
  the inspector + audit dashboard for now.
- Alerting on stalled vendors ("L1 for >7d"). Out of scope until we
  have a notification substrate at this layer.
- Lifting the swimlane into production goat-net once Block 84
  (Partner-peering portal) instantiates a different vendor-list
  source-of-record. The read shape is portable; the netbird-prefix
  match would need to gain the production-fabric AutoPeerName
  contract.
