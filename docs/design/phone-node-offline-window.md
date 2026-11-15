# A phone node off the network: the slot window and the re-seed

| Field | Value |
|---|---|
| Status | **Measured on lab containers; the re-seed path in `goatbuilder` is designed, not built.** The window and what a returning node sees are measured with two postgres-spock containers (`ops/postgres/spock/tests/slot-offline-window/run.sh`, four runs on 2026-11-15). The re-seed procedure is measured on the same containers. Nothing here is measured on a phone or on a live goatnet |
| Decision | Operator decision, 2026-11-15 (gate group D, D2): a phone stays a full `core` joiner for goatbuilder 1.0. A lighter profile is a later class value ([phone-peer-profile.md](phone-peer-profile.md)) |
| Design | [phone-node.md](phone-node.md) · [goatbuilder-hardware-measurement.md](goatbuilder-hardware-measurement.md), "How long a phone may be off" · [replication-consistency-observability.md](replication-consistency-observability.md) · [post-partition-convergence-runbook.md](../operations/post-partition-convergence-runbook.md) |
| Audience | whoever runs a goatnet with phone nodes, and whoever builds the re-seed into `goatbuilder` |

## The question

A phone node is a full replication member: its Postgres subscribes to every
replicating site, and every such site keeps a logical replication slot for it.
While the phone is away, each of those slots holds the WAL the phone has not
read. Postgres caps that with `max_slot_wal_keep_size`; the value and where it
is set are in [ops/postgres/spock/README.md](../../ops/postgres/spock/README.md),
"The slot WAL cap". Past the cap the slot is invalidated, the phone can no
longer catch up, and it has to be seeded again.

Three things needed measuring before gate row H8's seven-day run: how much WAL
a slot takes before it is invalidated, what the phone sees when it comes back,
and how the phone is seeded again.

## How it was measured

`ops/postgres/spock/tests/slot-offline-window/run.sh` starts two sites from the
postgres-spock image, a founder (A) and a phone (B), and replicates one table
both ways, as `spock-mesh-peering` does: no initial copy, no forwarded origins.
Then:

1. B leaves the network (`docker network disconnect`). It keeps running and
   writes 10 rows of its own, as a phone off its uplink does. A writes 50
   replicated rows that B does not see.
2. A writes WAL at a stated rate into a table it does not replicate. Any WAL
   counts against a slot, whatever table it comes from. Once a second, the run
   reads A's slot for B from `pg_replication_slots`: `wal_status`,
   `safe_wal_size`, the WAL it holds, and whether it is active.
3. When `wal_status` reads `lost`, B returns: by the network, or by a restart
   (`RETURN=restart`, a phone that rebooted). The run reads B's subscription
   status every 5 seconds and both sites' logs.
4. B is seeded again (below), and the run compares both sites' rows by count
   and digest.

The inputs were frozen by copy: the image was built from `ops/postgres/spock`
at main `e106d420`, and each run used a copy of the script. The image reported
the cap as 1 GiB, `max_wal_size` 1 GB, `checkpoint_timeout` 5 min and 16 MB WAL
segments.

| Run | Rate asked | Rate made | B returned by | Slot `unreserved` | Slot `lost` | B's status read `down` | Result |
|---|---|---|---|---|---|---|---|
| 1 | 16 MiB/s | 13 MiB/s | network | 78 s, 1,088,711,280 bytes | 115 s, 1,614,458,480 bytes | not within 45 s | a first version of the script; see "Two findings" |
| 2 | 16 MiB/s | 14 MiB/s | network | 70 s, 1,081,801,536 bytes | 103 s, 1,600,834,728 bytes | 21 s after B returned | PASS |
| 3 | 32 MiB/s | 14 MiB/s | restart | 72 s, 1,118,848,624 bytes | 105 s, 1,601,627,184 bytes | 6 s after B returned | PASS |
| 4 | 4 MiB/s | 4 MiB/s | network | 237 s, 1,083,541,640 bytes | 347 s, 1,600,192,720 bytes | 119 s after B returned | PASS |

The byte counts are the WAL A wrote after the cut. Run 3 asked for more than
the inserts could make; the rate made is read from A's WAL position.

## What the measurement shows

**The slot is safe up to the cap, and lost at the next checkpoint after it.**
In every run the slot read `unreserved` once it held more than the cap, and
read `lost` only when a checkpoint then ran and removed the WAL. Between the
two the slot could still have been read, had B come back. How long that gap
lasts depends on when the next checkpoint runs: within `checkpoint_timeout`,
or sooner when enough WAL has been written. In these runs it came at about
1.6 GB in every case, whatever the rate. A phone's budget must not count on
that gap. **The budget is the cap divided by the founder's WAL rate.**

So a phone may be off for about the cap divided by the WAL rate of the busiest
site that keeps a slot for it. The rate is read on each site as
[goatbuilder-hardware-measurement.md](goatbuilder-hardware-measurement.md),
"How long a phone may be off", describes. The measurement here does not give
a goatnet's rate, which comes from H8's week. As arithmetic: at 2 MiB an hour,
a 1 GiB cap lasts about 21 days, and at the F-093 rate in the Spock README,
about three hours.

**What the phone sees when it comes back.**
- For a while, nothing is wrong. B's subscription from A read `replicating`
  for 21 s and 119 s after B returned by the network, and 6 s after a restart.
  That is how long the apply worker took to find its old connection dead.
- Then the subscription reads `down`, and every few seconds B's log shows
  `could not send replication command "START_REPLICATION SLOT ..."` with
  `ERROR: can no longer get changes from replication slot`. A's log shows the
  same error with `DETAIL: This slot has been invalidated because it exceeded
  the maximum reserved size.`
- **B's own writes are not lost.** The slot B keeps for A is on B, and B wrote
  little while away, so it was not invalidated. In every run all 10 of B's
  offline rows reached A after B returned.
- **B never receives what A wrote while it was away** (0 of the 50 rows in
  runs 2 to 4), nor anything later. The two sites diverge until B is seeded
  again.
- On the founder, the slot reads `wal_status = lost`, `active = f`, with no
  `restart_lsn`. `SpockSlotInvalidated` alerts on exactly that
  ([replication-consistency-observability.md](replication-consistency-observability.md)).

## Two findings from the first run

1. **`sub_create` with `synchronize_data := true` does not seed a node that
   still holds rows.** Run 1 dropped B's subscription and created it again
   with the initial copy. The copy failed on B's existing rows (`duplicate key
   value violates unique constraint`), and the subscription still read
   `replicating` with the table not copied. The re-seed below copies each
   table again after a local truncate instead.
2. **A re-seed writes the copied data as WAL on the phone, and that WAL counts
   against the slot the phone keeps for the founder.** In run 1 the replicated
   table held about 1.4 GB, because the load went into it. The copy into B
   wrote that much WAL on B within about a minute, and B invalidated its own
   slot for A (`can no longer get changes from replication slot
   "spk_netbird_site_b_sub_b_to_a"`). So a re-seed is safe only when the data
   it copies is well under the phone's own cap. A goatnet's control-plane
   databases are far smaller than that. Runs 2 to 4 put the load in a table
   that is not replicated, and the copy wrote 30 KB to 60 KB of WAL.

## Seeding a phone again

Measured on lab containers in runs 2 to 4. The order matters: the phone's own
writes reach the founder first, and only then is the phone's copy replaced.

1. **Confirm the slot is lost**, on the site that held it:
   `SELECT slot_name, active, wal_status FROM pg_replication_slots;` reads
   `lost` for the phone's slot, which is named
   `spk_<db>_<provider node>_sub_<provider node>_to_<phone node>`.
2. **Let the founder take the phone's own writes.** The founder's subscription
   from the phone must read `replicating` and have caught up to the phone's
   current WAL position. Truncating the phone's tables before then would lose
   whatever the phone wrote while it was away.
3. **Check the size.** The data to copy, in total, must be well under the
   phone's own cap (finding 2): check `pg_total_relation_size` of the
   replicated tables on the founder.
4. **On the phone, for each replicated database:** drop the subscription from
   the founder, which also drops the lost slot on the founder
   (`SELECT spock.sub_drop('sub_<founder>_to_<phone>');`). Create it again
   with the parameters `spock-mesh-peering` uses, without the initial copy.
   Then copy each table of its replication sets again, truncated first
   (`SELECT spock.sub_resync_table('sub_<founder>_to_<phone>', '<table>',
   true);`). The truncate happens on the phone only and is not replicated. In
   run 2, the founder's rows were untouched.
5. **Wait** until every table reads `r` in `spock.local_sync_status`, and the
   phone's slot for the founder still reads `reserved`.
6. **Check convergence**: run drift-watch, as the convergence runbook's step 4
   does. In runs 2 to 4 both sites held the same 161 rows with the same digest.

The operations form of this procedure is in
[post-partition-convergence-runbook.md](../operations/post-partition-convergence-runbook.md),
"A. Slot rebuild". The `pg_basebackup` rebuild there copies the provider's
whole data directory, and with it the provider's Spock node identity. It
applies to a streaming standby, not to a Spock member such as a phone node.

## The re-seed in goatbuilder

**Designed, not built.** It is more than a small change: a new verb, run
against a node's database over the mesh, with refusals and a journal. It is
listed for gate row H8's prerequisites.

```
goatbuilder join-node <node> --reseed [--dry-run]
```

It runs from the operator's machine against the phone and every site that kept
a slot for it, over the same path `join-node` uses. Each step below is a
refusal with its own catalog code when its condition does not hold:

| Step | What it checks or does | Refuses when |
|---|---|---|
| 1 | Reads every replicating site's slots for the node | no slot for the node reads `lost`: nothing to re-seed, and a re-seed of a healthy node would only cost a copy |
| 2 | Reads each site's subscription from the node, and the node's WAL position | a site has not caught up with the node's own writes: they would be lost by the truncate |
| 3 | Sums the replicated tables' sizes on the site it copies from, and reads the node's live cap | the data is not well under the node's cap (finding 2): the copy would cost the founder its slot on the node |
| 4 | Drops and creates each subscription from the site whose slot was lost, with `spock-mesh-peering`'s parameters, and runs `sub_resync_table` with truncate for each table of its replication sets | |
| 5 | Waits for `spock.local_sync_status` to read `r` for every table, and for the node's slots to read `reserved` | a table fails to copy, or a slot on the node reads `lost` |
| 6 | Runs drift-watch between the node and the site | a table differs |

`--dry-run` stops after step 3 and prints what it would do. Each step is
journaled, as `federate` journals its phases, so a run that stops resumes at
the step it stopped at. The verb runs only when asked: it never re-seeds
because an alert fired, since an operator decides that a node is to be copied
again (runbook, "What this runbook does NOT do").

Not built: the verb, its catalog codes, the journal, and a test against the
harness above. Until it is built, the operator runs the procedure above by
hand.
