# Island mode: the fabric audit of a cut-off site, and a capped spool for a phone

**Status:** designed, and only the declared absence (§8) is built. It answers
one question for [ADR 1165](../adr/1165-island-mode-multi-writer-every-site-admits.md) (island
mode): how does a site that is cut off from the hub keep an audit record of what was published on
its fabric? Measured against `USAREUR-AF/goatcore` main `2133f4d4` and the audit-subscriber in
`USAREUR-AF/goatstuff` main `a2920eeb`, 2026-11-13.

The examples use the synthetic goatnet `zagros`: hub site `zagros-a`, a fixed joiner site
`zagros-b`, and a joiner site `zagros-c` that is a phone.

| Part | Site | What it needs | Status |
|---|---|---|---|
| A local store | fixed site (`zagros-b`) | its own audit-subscriber and object store with a small quota: existing roles, configuration only (§2) | designed, not built |
| A capped spool, delivered to the hub | phone (`zagros-c`) | new subscriber code, role settings, reader changes (§3 to §7) | designed, not built |
| The absence, declared | phone, until the spool is built | one group_var and one end-state catalog row (§8) | built (§8) |

---

## 1. What happens today

**Plain:** only the founder runs an audit-subscriber. Data published at a cut-off joiner reaches no
subscriber, so it has no audit record, and nothing reports that it has none.

The audit of record is the founder's audit-subscriber
([`zenoh-audit-external-subscriber.md`](zenoh-audit-external-subscriber.md)). It is a Zenoh client
of the site's router that subscribes to `**`, collects events in memory and, once a minute,
writes one NDJSON object to the site's `audit-integrity` bucket. The object key is
`<subscriber id>/<YYYY>/<MM>/<DD>/<HH>/<ms>-<count>.ndjson`
([`flush_batch`](https://github.com/usareur-af/goatstuff/blob/a2920eebdf2c649993c7b5c3407d4961f3b4a720/ops/zenoh-trial-peers/audit-subscriber/src/main.rs#L818)).

Four facts about that writer decide this design:

- **A failed upload is dropped.** When `put_object` fails, the writer increments
  `goat_audit_write_errors_total`, logs, and discards the batch
  ([`main.rs:903`](https://github.com/usareur-af/goatstuff/blob/a2920eebdf2c649993c7b5c3407d4961f3b4a720/ops/zenoh-trial-peers/audit-subscriber/src/main.rs#L903)).
  A subscriber whose store is unreachable keeps no copy to send later.
- **The local file is not a spool.** `AUDIT_SUBSCRIBER_LOCAL_NDJSON_DIR` appends each batch to a
  dated file inside the same flush, as a hand-off to off-box logging, and nothing reads it back
  for delivery (`ops/ansible/roles/audit-subscriber/defaults/main.yml:274`).
- **The memory window is the loss window.** Between flushes the batch exists only in memory
  (`ops/ansible/roles/audit-subscriber/defaults/main.yml:252-263`), so a crash loses up to
  `audit_subscriber_batch_interval_ms` of events.
- **A reconnecting subscriber receives live data only.** Zenoh delivers what is published while
  the subscriber is connected. Events published while a site's subscriber cannot reach any router
  are gone; so the subscriber must run at the site itself, against the site's own router.

The end state does not show the gap. The `audit-subscriber` catalog row reads presence from the pin
table only (`ops/goatbuilder/endstate/catalog.yaml:327-333`), and the pin table declares joiners
`absent: true`, so a joiner reads "absent" with no statement that its fabric has no audit.

## 2. The fixed site: its own subscriber and store

**Plain:** a fixed joiner runs the same two services the founder runs, sized small. This needs no
new code.

`zagros-b` runs the `minio` and `audit-subscriber` roles as they are. Its audit bucket gets a hard
cap with `minio_audit_bucket_quota_gib` (`ops/ansible/roles/minio/defaults/main.yml:210`). At the
cap the store refuses new objects and `goat_audit_write_errors_total` rises, which is the signal
that the cap is too small: under §1's first fact, a refused batch is lost. The site's subscriber id
is `zagros-b-A`, so its objects never share a key prefix with the hub's.

Its record stays at `zagros-b`. Copying it to the hub is not part of this design (§11).

Declared in the overlay (configuration only):

| group_vars of `zagros_b_hosts` | Value |
|---|---|
| `minio_enabled` | `true` |
| `minio_audit_bucket_quota_gib` | the site's cap, sized as audit rate x event size x retention days |
| `audit_subscriber_s3_endpoint` | the site's own store |
| fleet pins | `zagros-b` no longer `absent: true` for `audit-subscriber` and `minio` |

## 3. The phone: what changes

**Plain:** on a phone, the subscriber writes every batch to a capped folder on local disk first,
and a second task copies that folder to the hub's bucket whenever the hub can be reached. One path
is used whether the link is up or down, so the path that runs while cut off is the one that runs,
and is tested, every day.

A new setting, `audit_subscriber_mode`, takes two values:

| Mode | The subscriber writes to | Used by |
|---|---|---|
| `store` (the default, today's behaviour) | the site's own store, `audit_subscriber_s3_endpoint` | the founder, fixed sites |
| `spool` | a local spool folder; a drainer delivers it to the hub's store | a phone |

In `spool` mode the writer has no store client at all. The drainer is the only code that opens a
connection to a store (§6).

## 4. The spool: format and cap

### Format

The spool is a folder, `audit_subscriber_spool_dir`, with three subfolders:

| Folder | Holds |
|---|---|
| `open/` | the one segment being written: `<start_ms>.ndjson` |
| `pending/` | sealed segments waiting for delivery: `<start_ms>-<count>-<sha256 first 16 hex>.ndjson` |
| `state.json` (a file) | the last sealed segment's `start_ms`, and the loss counters of §4 |

- **Append, then fsync, every flush.** Each flush appends the batch's NDJSON lines to the open
  segment and calls `fsync`. A crash then loses at most the in-memory batch, as today. In spool mode
  the flush interval can be short (the default is 5 s) because a flush is a local append, not an
  object.
- **Seal by age or size.** The open segment is sealed when it is
  `audit_subscriber_spool_segment_seconds` old (default 60) or holds
  `audit_subscriber_spool_segment_max_bytes` (default 8 MiB). Sealing fsyncs it, computes the
  SHA-256 of its bytes, and renames it into `pending/`. A rename is atomic, so a segment in
  `pending/` is always whole. One segment becomes one object at the hub, so the hub holds about as
  many objects as a founder subscriber's 60 s flushes produce.
- **A torn tail is cut, and counted.** On start, an open segment left by a crash is sealed. A last
  line without a newline is a write the crash interrupted; it is removed, and its bytes are added
  to `goat_audit_spool_torn_bytes_total`.
- **Time never goes backwards.** A segment's `start_ms` is never less than the previous sealed
  segment's (`state.json`). A phone clock that steps back is clamped and counted in
  `goat_audit_spool_clock_clamped_total`. This keeps object keys in time order within the phone's
  prefix, which the hub's readers depend on (§7).

Lines are the same `AuditEvent` JSON the founder's subscriber writes, so a reader cannot tell a
delivered segment from a founder batch except by its key prefix.

### The cap

`audit_subscriber_spool_max_bytes` caps the bytes in `open/` and `pending/` together. It has no
default: a phone's free disk and expected outage length belong to the deployment, so the role
refuses `spool` mode without it, naming the setting. Size it as audit rate x bytes per event (about
650) x the longest outage to ride through.

**At the cap, the oldest pending segment is dropped,** before the new batch is appended. The
alternative, refusing new batches, keeps the start of an outage and loses everything after it, and
the end of an outage is the part closest to the reconnection that will be investigated. Dropping
oldest keeps the newest `max_bytes` of the outage.

A drop is never silent:

- `goat_audit_spool_dropped_events_total` and `goat_audit_spool_dropped_bytes_total` increase, and
  the subscriber logs the dropped segment's `start_ms`, event count and bytes at `error`.
- A **gap record** is written: one line
  `{"kind":"audit_gap","subscriber":"zagros-c-A","first_ts":…,"last_ts":…,"events":…,"bytes":…,"reason":"spool_cap"}`,
  spooled under a separate prefix, `_audit-gaps/<subscriber id>/<YYYY>/<MM>/<DD>/<HH>/`, and
  delivered like any segment. So the hub's bucket itself says which time range is missing. It sits
  outside the `<subscriber id>/` prefix so no event reader parses it as an event or moves a cursor
  past it.
- An alert fires on any increase of the dropped counters.

## 5. Delivery: resumable and idempotent

**Plain:** each segment has one object name, fixed when it is sealed. Before sending, the drainer
asks the hub whether that object is already there. It deletes its local copy only once the hub
holds the object with the same checksum. Sending twice is therefore impossible to notice, and a
crash at any point leaves at worst a local copy that the next pass confirms and removes.

The drainer is a task in the same process. It delivers `pending/` segments one at a time, oldest
first, to the object key derived from the file name alone:

```
<subscriber id>/<YYYY>/<MM>/<DD>/<HH>/<start_ms>-<count>-<sha16>.ndjson
```

For each segment:

| Step | Request | On answer |
|---|---|---|
| 1 | `HEAD` the key, with checksum mode enabled | `200` with `x-amz-checksum-sha256` equal to the segment's SHA-256: already delivered, go to 3. `200` with another checksum: a conflict, go to 4. `404`: go to 2. Anything else: stop this pass, back off |
| 2 | `PUT` the segment with `x-amz-checksum-sha256` and `If-None-Match: *` | `200`: go to 3. `412` (the key exists): go back to 1. Anything else, including a quota refusal: stop this pass, back off |
| 3 | delete the local segment | count `goat_audit_spool_delivered_total{outcome="put"\|"already_present"}` |
| 4 | move the segment to `conflict/`, keep it | count `goat_audit_spool_conflicts_total`, log at `error`, alert. The SHA-256 in the key makes this a store or operator fault, never a normal path |

- **In order, or not at all.** A failure on one segment stops the pass; no later segment is sent
  before it. The hub's portal reads each subscriber prefix with a `StartAfter` cursor (§7), so a
  later key landing first would move the cursor past an earlier one that never gets read.
- **Back off, then resume.** After a failed pass the drainer waits, doubling from 5 s up to 5
  minutes, and starts again from the oldest pending segment. Nothing else records progress: the
  folder is the queue.
- **One drainer per spool.** The drainer holds an exclusive lock on `<spool_dir>/.lock`, so a second
  process on the same spool exits rather than racing it.
- **No duplicates in a versioned bucket.** The audit bucket is versioned with Object Lock, so a
  second `PUT` of an existing key would add a version rather than replace it. Steps 1 and 2 never
  issue that second `PUT`: a segment is sent only after a `404`, and `If-None-Match: *` refuses the
  write if the key appeared in between.

`If-None-Match` on `PUT` and the SHA-256 checksum on `HEAD` are S3 features the store must have. The
store conformance job of [ADR 1222](../adr/1222-object-store-moves-from-minio-to-rustfs.md) gains
both as assertions (§9), for MinIO and for RustFS, before any phone runs `spool` mode.

## 6. Which store the subscriber may connect to

**Plain:** the hub's store is named in its own settings, read only by the drainer. The writer of a phone site has no store at all.

| Connection | Setting |
|---|---|
| a `store`-mode writer to its site's store | `audit_subscriber_s3_endpoint`, as today |
| a `spool`-mode drainer to the hub's store | `audit_subscriber_spool_deliver_endpoint` |

- `spool` mode with `audit_subscriber_s3_endpoint` set is refused at start and by the role: one
  process does not write to two stores.
- **The credential can only add the phone's own objects.** The hub's store gets a user for the
  phone, declared in the hub's `minio_extra_users`
  (`ops/ansible/roles/minio/defaults/main.yml:245`), whose policy allows `s3:PutObject` and
  `s3:GetObject` on `audit-integrity/zagros-c-A/*` and `audit-integrity/_audit-gaps/zagros-c-A/*`,
  and nothing else. A lost phone can therefore add objects under its own prefix, which Object Lock
  keeps, and can neither read nor delete anything another site wrote. The hub revokes the user to
  stop it.

## 7. Reading delivered segments at the hub

**Plain:** the hub's readers today read one subscriber prefix each. They will read a list.

| Reader | Today | Change |
|---|---|---|
| panoscope and stethoscope (`crates/panoscope-core/src/audit_tail.rs`) | one `key_prefix` (`audit_tail.rs:61`), walked hour by hour for the last N hours from now (`audit_tail.rs:526`) | `key_prefixes`, a list; each walked the same way |
| the portal's vendor-progress tail (`ops/goat-zenoh-portal/internal/audittail/audittail.go`) | one `SubscriberID`, read with `StartAfter` from a cursor (`audittail.go:144`) | one cursor per subscriber id in a list |

A delivered segment keeps its own time in its key, not its delivery time. A reader that walks "the
last N hours" therefore sees a late segment only if it was delivered within N hours of being
written. The portal's cursor sees every segment, because within one prefix segments arrive in key
order (§5).

## 8. Declaring the absence until it is built

**Plain:** until the spool exists, the phone site says, in its own configuration, that it has no
fabric audit, and `goatbuilder endstate` prints that in the declared-absent list instead of saying
nothing.

A new end-state catalog row, `audit-spool`, reads the phone's flag:

```yaml
audit-spool:
  class: mgmt
  present: {var: audit_subscriber_spool_enabled}
  containers: [audit-subscriber]
  ledger: audit-subscriber
  zenoh: {sub: ["**"]}
```

| The site declares (group_vars) | `goatbuilder endstate` reports `audit-spool` as |
|---|---|
| nothing (the founder, fixed sites) | declared absent, reason `role default audit_subscriber_spool_enabled: False` |
| `audit_subscriber_spool_enabled: false` (the phone, until built) | declared absent, reason `group_vars/zagros_c_hosts: audit_subscriber_spool_enabled: False`: the site's own statement that its fabric has no audit |
| `audit_subscriber_spool_enabled: true` (the phone, once built) | present, and graded like any service: deployed, active, healthy, and the fabric grant on `**` |

The phone sets the flag explicitly even while it equals the role default, so the reason the end
state prints names the phone's own configuration rather than a default every site shares.

Built: the `audit-spool` row in `ops/goatbuilder/endstate/catalog.yaml`, the role default
`audit_subscriber_spool_enabled: false`, and two fixtures under `ops/goatbuilder/fixtures/endstate/`
(`audit-spool-declared-absent`, `audit-spool-on-not-running`) that measure test 11. Because the flag
must be read by the code that deploys the service (`ops/ci/check-role-honours-presence-flag.py`), and
nothing deploys a spool yet, the audit-subscriber role refuses `audit_subscriber_spool_enabled: true`
with GB-296 rather than deploying nothing. The
reasons are the texts `endstate.py` already prints for a flag
(`ops/goatbuilder/endstate/endstate.py:232-238`).

## 9. Tests that will prove it

| # | Test | Proves |
|---|---|---|
| 1 | spool unit tests: append and fsync per flush; seal by age and by size; atomic rename; a torn last line cut and counted; a clock step back clamped and counted | §4 format |
| 2 | cap unit test: fill past `max_bytes`; the oldest segment is gone, the dropped counters equal its events and bytes, a gap record names its time range | §4 cap |
| 3 | delivery against a local S3 stub: link down for 30 segments, then up; the stub holds each key exactly once and the spool is empty | §5 resume |
| 4 | crash between `PUT` and local delete (the stub accepts the `PUT`, the drainer is killed): on restart the drainer sends `HEAD`, no second `PUT`, deletes the local copy | §5 idempotence |
| 5 | in order: the stub refuses segment 3 of 5; segments 4 and 5 are not sent until 3 is accepted | §5 order |
| 6 | conflict: a different object already at the key; the segment moves to `conflict/`, nothing is overwritten | §5 conflict |
| 7 | in `spool` mode the writer opens no store connection (the stub counts zero connections before the drainer starts); `spool` with `audit_subscriber_s3_endpoint` set is refused | §6 one store |
| 8 | store conformance, MinIO and RustFS (ADR 1222 job): `If-None-Match: *` refuses an existing key with `412`; `HEAD` returns the SHA-256 checksum a `PUT` sent; a forced redelivery leaves one version per key in an Object Lock bucket | §5 relies on the store |
| 9 | role render: `spool` mode renders the spool and delivery settings and mounts the spool folder; without `audit_subscriber_spool_max_bytes` or the deliver endpoint, the role stops and names the missing setting | §4, §6 |
| 10 | readers: `audit_tail.rs` walks two prefixes; the portal keeps one cursor per subscriber and reads a segment delivered into a past hour | §7 |
| 11 | end state: a fixture phone site with `audit_subscriber_spool_enabled: false` reads `audit-spool` declared absent with that reason; with `true` and no container, active fails | §8 |
| 12 | live partition drill (measured, not unit): cut `zagros-c` off, publish a known set of events, restore the link; the hub's bucket holds each event exactly once, checked by `payload_hash` and `ts` against the publisher's log | the whole path |

## 10. Files the code will change

| Repository | File | Change |
|---|---|---|
| goatstuff | `ops/zenoh-trial-peers/audit-subscriber/src/main.rs` | `--mode`, the spool settings, writer hands batches to the spool in `spool` mode |
| goatstuff | `ops/zenoh-trial-peers/audit-subscriber/src/spool.rs` (new) | segments, seal, cap, gap records, `state.json` |
| goatstuff | `ops/zenoh-trial-peers/audit-subscriber/src/deliver.rs` (new) | the drainer, `HEAD` then conditional `PUT` |
| goatcore | `ops/ansible/roles/audit-subscriber/defaults/main.yml` | `audit_subscriber_mode`, `audit_subscriber_spool_*`, `audit_subscriber_spool_enabled: false` |
| goatcore | `ops/ansible/roles/audit-subscriber/templates/docker-compose.yml.j2` | spool settings, the spool volume; the `mgmt` network optional, since a phone has none |
| goatcore | `ops/ansible/roles/audit-subscriber/templates/env.j2` | the delivery credential |
| goatcore | `ops/ansible/roles/audit-subscriber/tasks/main.yml` | the refusals of test 9 |
| goatcore | `crates/panoscope-core/src/audit_tail.rs` | `key_prefixes` |
| goatcore | `ops/goat-zenoh-portal/internal/audittail/audittail.go`, `ops/goat-zenoh-portal/internal/config/config.go` | one cursor per subscriber id |
| goatcore | `ops/goatbuilder/endstate/catalog.yaml`, a fixture under `ops/goatbuilder/fixtures/endstate/` | the `audit-spool` row, test 11 |
| goatcore | `ops/errors/catalog.yaml` | the role's refusal for a missing spool setting |
| goatcore | `ops/compose/goat-obs/prometheus/rules/audit-subscriber.yml` and a rule test | alerts on dropped, conflict and the age of the oldest pending segment |

## 11. Not designed here

- Copying a fixed site's audit to the hub (§2).
- A phone that never reconnects: its spool is lost with it, and the hub's record shows the phone's
  last delivered segment and nothing after.
- Audit of the hub's own fabric while cut off from a joiner: unchanged, the hub's subscriber runs
  at the hub.
