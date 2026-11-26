# Rebuild a site whose Spock slots were invalidated

**Audience.** The operator of a multi-site goatnet when a site's Postgres can
no longer replicate because a Spock slot it needs was invalidated. The
`SpockSlotInvalidated` alert fires, the detector below refuses with GB-387, or
step 2 of the [post-partition convergence runbook](post-partition-convergence-runbook.md)
finds `wal_status = lost`.

**What it does.** It copies the site's replicated tables again from a healthy
peer, its founder, and creates its lost subscriptions again in both
directions. Rows only that site holds are kept: they are exported before the
copy, and written back or handed to you afterwards. Then it proves that the
site and the founder hold the same rows.

**Status.** Built: `goatbuilder join-node ... --reseed` and `--slots`, and their
helper `ops/scripts/spock-reseed.py`. Measured on three postgres-spock lab
containers (`ops/postgres/spock/tests/site-rebuild/run.sh`;
[goatbuilder-hardware-measurement.md](../design/goatbuilder-hardware-measurement.md),
"The rebuild of a site"). Rehearsed on the real NetBird, Kratos and Hydra schemas with the revocation
guard (the partition harness's sites). Not yet run on a live goatnet.

---

## Why a slot is lost, and why the site must be rebuilt

Each site keeps one logical replication slot for each subscriber of each
replicated database (`netbird`, and `kratos` and `hydra` where Ory
replicates). A slot holds the WAL its subscriber has not read yet. The image
caps that at `max_slot_wal_keep_size` (its values: [The slot WAL cap](../../ops/postgres/spock/README.md#the-slot-wal-cap)).
When a subscriber is away long enough for its provider to write more than the cap, Postgres removes the
WAL at the next checkpoint and marks the slot `lost`. The subscriber cannot
catch up from that slot again. Its subscription may still read `replicating`
until its apply worker next fails.

The rows it missed reach it only by copying its tables again. A
`pg_basebackup` rebuild does not apply to a Spock member: it copies the
provider's whole data directory, and with it the provider's Spock node
identity ([post-partition convergence runbook](post-partition-convergence-runbook.md), "A. Slot rebuild").
The rebuild here is a logical copy through Spock, table by table.

Every site writes ([ADR 1165](../adr/1165-island-mode-multi-writer-every-site-admits.md)).
A site that was cut off may hold rows its founder never received, for example
an identity enrolled at that site. A copy from the founder alone would drop
them. So the rebuild first compares the site with the founder and exports
what only the site holds, and it refuses to truncate anything until you have
accepted that export.

## Before you start

1. **Pick the founder.** The rebuild copies from the site's founder
   (`join_founder_site` in the site's group_vars). Run the detector for the
   founder itself, against each joiner site, to see that its other slots are
   healthy. A founder that is itself missing rows would hand them on.
2. **Name every site the rebuilt site replicates with.** In a full mesh, a
   joiner site also subscribes to the other joiner sites. Give each one with
   `--peer <site>`. The rebuild refuses (GB-390) when the site subscribes to a
   site it cannot read.
3. **Plan a window.** The site's identity and mesh services cannot write while
   its tables are copied. The copy took seconds for a few hundred rows in the
   lab. It takes longer with more data, and step 4 refuses a copy that is more
   than a quarter of the site's own slot cap (GB-347).
4. **Have a second operator** read the export with you (step 3 below).

## Steps

Commands run from the operator's machine, in the deployment's checkout, on
the goatnet's mesh. `<env>` is the goatnet, `<site>` the site being rebuilt,
`<node>` its Postgres host's inventory name (the first host of
`[<site>_hosts]`).

### 1. Read the slots

```bash
goatbuilder join-node <env> --site <site> --node <node> --class core --slots --peer <other-site>
```

It reads only. Each line is one subscription or slot, with its `wal_status`
read where the slot lives. A line that reads `cannot recover` is a lost slot.
An `in` line means the site cannot receive from that provider. An `out` line
means that subscriber cannot receive from this site. The run exits 1 with
GB-387 while any slot is lost.

### 2. Stop the site's writers

On the site's Postgres host, stop every service that connects to a
replicated database: the mesh management service (`management-a`), the
identity services (`kratos`, `hydra`), the admission audit (`admit-audit`)
and the Postgres exporter (`postgres-exporter`). Stop them with the compose
files the site runs, for example:

```bash
cd /opt/mgmt-host
sudo docker compose -f docker-compose.yml -f docker-compose.ory.yml -f docker-compose.obs.yml \
  -f docker-compose.admit-audit.yml stop management-a kratos hydra admit-audit postgres-exporter
```

The rebuild refuses (GB-389) while any client other than itself is connected
to a database it truncates. The refusal names each database, role,
application and client address, so a service missing from this list shows
up there.

### 3. Rehearse, then run to the export

```bash
goatbuilder join-node <env> --site <site> --node <node> --class core --reseed --peer <other-site> --dry-run
goatbuilder --yes join-node <env> --site <site> --node <node> --class core --reseed --peer <other-site>
```

The dry run prints the eight steps and each command, and runs nothing. The
run takes steps 1 and 2 and stops at step 3, `export`, with GB-388 when the
site holds rows the founder lacks. It names the export file
(`<state_dir>/join-node-<site>-<node>-reseed.export.json`, mode 0600; it holds
row contents) and its digest.

Read the export with the second operator. For each table it lists:

| List | What it holds | What the rebuild does with it |
|---|---|---|
| `replay` | rows written at this site after the founder's last applied commit from it, absent at the founder | writes them back after the copy (step 7), so they replicate to the founder |
| `review` | rows the founder lacks that are older than that commit, or that came from another site, and rows this site changed after that commit | nothing: the founder's version stands; they are handed back in step 7 |
| `replaced` | keys of rows the founder holds with newer contents | the copy takes the founder's version |
| `added` | keys of rows only the founder holds | the copy brings them |
| `plan` | the tables the copy takes (those that differ, and the replicated tables that reference them), the tables left alone (already the same), and `emptied`: each table that is not replicated and that the truncate empties, with its row count | copies, leaves, and empties them; on an identity database, `emptied` is the site's own sessions and pending flows when identities differ, and the site's users sign in again |

The export refuses while it holds rows or names a table in `emptied` that
holds rows. A row in `added` that this site deleted while it was cut off comes
back with the copy. Spock keeps no record of a delete. Check `added` for rows this site
removed on purpose, and delete them again after step 8.

When you are satisfied, run again with the digest:

```bash
goatbuilder --yes join-node <env> --site <site> --node <node> --class core --reseed --peer <other-site> --accept <digest>
```

The run starts at step 3 and goes on:

| Step | What it does | Refuses when |
|---|---|---|
| 3 `export` | compares again and writes the export | the digest is not the one given (GB-388), a client is connected (GB-389) |
| 4 `size` | sums the tables to copy against the site's slot cap | the copy is more than a quarter of the cap (GB-347; `--size-fraction` in the script avenue) |
| 5 `reseed` | checks the export still matches, pauses the site's other subscriptions, drops and creates again each lost subscription of the site and the founder's lost subscription from it, truncates the tables in the plan once, in repair mode (not replicated), and copies each one again from the founder | the rows only the site holds changed since the export, or the truncate would empty a table the export did not name (GB-388; step 3 runs again), a client is connected (GB-389) |
| 6 `synced` | waits for every table to be copied and every subscription to replicate, both ways | a table is not copied in time, or a slot on the site is lost during the copy (GB-348) |
| 7 `replay` | writes each `replay` row back on the site, parents first, one statement per row | a row could not be written (GB-391) |
| 8 `converged` | compares every table on the site and the founder, by row count and digest | a table still differs after the wait (GB-349) |

A refusal stops the run at its step. Fix the cause and run the same command
again: it starts at the step that refused. Steps that passed are not taken
again.

### 4. If step 7 hands rows back

GB-391 names the hand-back file
(`<state_dir>/join-node-<site>-<node>-reseed.handback.json`, mode 0600). Each
row that was not written has the reason Postgres gave, for example a unique
email that another site took while this one was cut off. Resolve each row by
hand: write it with a changed value, merge it into the row that won, or decide
to drop it. Then run again with the hand-back's digest as `--accept`. The file
also keeps the `review` rows from step 3 for the record.

### 5. Start the writers and confirm

```bash
cd /opt/mgmt-host
sudo docker compose -f docker-compose.yml -f docker-compose.ory.yml -f docker-compose.obs.yml \
  -f docker-compose.admit-audit.yml start postgres-exporter admit-audit hydra kratos management-a
goatbuilder join-node <env> --site <site> --node <node> --class core --slots --peer <other-site>
```

The detector must exit 0. Then go through the
[post-partition convergence runbook](post-partition-convergence-runbook.md)
from step 1.

### 6. The next site

Step 8 names each peer whose subscription from this site is lost too, for
example the other joiner site. The rebuild leaves that subscription alone:
re-creating it without a copy would hide the rows that peer missed. Rebuild
that peer next, from the founder, the same way, with this site as its
`--peer`. Its copy comes from the founder, which by then holds this site's
replayed rows. Rebuild one site at a time.

## The script avenue

The verb runs `ops/scripts/spock-reseed.py` once per step, reaching each side
over SSH through `docker exec`. The helper can be run directly with any
command that runs `psql` on a side:

```bash
ops/scripts/spock-reseed.py slots --node-psql CMD --site-psql CMD --peer-psql <name>=CMD [--json]
for step in lost caught-up export size reseed synced replay converged; do
  ops/scripts/spock-reseed.py "$step" --node-psql CMD --site-psql CMD --peer-psql <name>=CMD \
    --journal FILE [--accept DIGEST] || break
done
```

Exit 1 is a refusal with a catalog code; exit 2 means a side could not be
reached or a statement failed. Both are recorded in the journal.

## What this runbook does not do

- It does not rebuild the founder. If the founder itself is the site that lost
  data, rebuild it from a site that holds the data, by naming that site as the
  founder in a checkout made for the purpose.
- It does not keep a delete made at the rebuilt site while it was cut off: the
  copy brings the row back from the founder (see `added`, step 3).
- It does not run by itself when an alert fires. An operator decides that a
  site is copied again.
