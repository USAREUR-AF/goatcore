# Postgres SQL_ASCII → UTF8 migration

**Status: WIP, not for production use** — the gate is one live migration of a
goatnet run end to end with `ops/ansible/postgres-encoding-migrate.yml`.

A goatnet whose Postgres clusters were created before the compose default
became UTF8 (`ops/compose/mgmt-host/docker-compose.yml`, `POSTGRES_INITDB_ARGS`)
is SQL_ASCII: Postgres stores whatever bytes it is given and validates nothing,
and a joiner created as UTF8 cannot replicate with it (Spock refuses
`encoding conversion for binary datum`). This runbook moves every Spock member
of one goatnet, and each member's streaming standby, to UTF8 in one outage,
keeping the old PGDATA so the goatnet can be put back exactly as it was.

The playbook is `ops/ansible/postgres-encoding-migrate.yml`; its phases live in
`ops/ansible/roles/postgres-encoding-migrate/tasks/`, one file each.

## When

- Before a UTF8 joiner is added to a SQL_ASCII goatnet (site-bootstrap Phase 1C
  otherwise initialises the joiner as SQL_ASCII to match its founder).
- Before identity or organisation names outside ASCII are written: in SQL_ASCII
  they are stored unvalidated and compared byte for byte.

Order across goatnets: the development goatnet first, then preprod, then prod.
Each goatnet is one run; the next goatnet starts after the previous one has
been verified.

## Outage

The mesh keeps working: enrolled peers run peer-to-peer and survive a
management outage. What stops, from phase 2 to the end of phase 9, on every
member: the management API (enrolment, peer and policy changes), sign-in
(Kratos, Hydra), the consoles and anything else in the Postgres container's
compose project or connected to Postgres.

Duration is dominated by dump and restore, which scale with the data. Measured
on the test fixture (two nodes, eleven small databases each, one Docker host):
229 s for phases 1 to 9 including a deliberate pause after phase 4. Measure a
goatnet's own size first: phase 1 reports PGDATA bytes per member.

## Before the run

1. **group_vars.** Every group in `migrate_groups` declares
   `postgres_initdb_encoding: UTF8` or does not declare it (the compose default
   is UTF8). Phase 1 refuses a group that declares anything else. Make that
   change in the goatnet's overlay before the run.
2. **Environment.** `<SITE>_REPLICATOR_PASS` for every Spock site and every
   pair site (the re-wire and the standby rebuild use it), the same as for
   `site-bootstrap.yml`.
3. **Disk.** On each primary, free space of at least, per filesystem:

   | What | Size | Where |
   |---|---|---|
   | Kept copy of PGDATA | 1 × PGDATA | Docker volume filesystem |
   | Dumps | sum of `pg_database_size` (dumps are compressed and come in smaller) | `postgres_encoding_migrate_dir`, default `/var/lib/goat/postgres-utf8-migration` |
   | New full pgBackRest backup | 1 × PGDATA (zst-compressed, comes in smaller) | `/opt/docker-data/pgbackrest` |
   | Reserve | 1 GiB per filesystem | |

   With all three on one filesystem that is **3 × PGDATA + 1 GiB**. A standby
   needs 1 × PGDATA + 1 GiB (its kept copy; its new copy replaces the old data
   in place). Phase 1 measures and refuses; phase 4 measures again with Postgres
   stopped.
4. **A clean mesh.** Every subscription replicating, and every Spock node any
   member subscribes to listed in `migrate_groups`. Phase 1 refuses otherwise.
5. **Peers that are gone, standbys in other groups.** Name the gone peers'
   subscriptions in `postgres_encoding_migrate_drop_subscriptions` and the
   groups of standbys outside `migrate_groups` in
   `postgres_encoding_migrate_standby_groups` (see
   [Peers that are gone](#peers-that-are-gone) and
   [Physical standbys](#physical-standbys)); phase 1 refuses either otherwise.
6. **Identical replicated rows, no orphans.** Every replicated table holds the
   same primary keys on every member, and no foreign key has a child row
   whose parent is absent on the same node. Phase 1 checks both and refuses
   with names; see [Divergent rows and foreign-key orphans](#divergent-rows-and-foreign-key-orphans).

## Command sequence

From an assembled tree, in `ops/ansible`, with the same inventory pair as
`site-bootstrap.yml`. For a two-node goatnet — founder group `<founder>_hosts`
and joiner group `<joiner>_hosts`; the joiner's obs group is not listed, it
runs no goatnet Postgres:

```bash
P='../scripts/ansible-preflight.sh -i inventory/mgmt-hosts.ini -i inventory/mgmt-hosts-mesh.ini postgres-encoding-migrate.yml -e migrate_groups=<founder>_hosts,<joiner>_hosts'

$P -e postgres_encoding_migrate_stop_after=1   # gates only; opens the run, stops nothing
$P -e postgres_encoding_migrate_action=status   # prints the run id
$P                                              # phases 2-9: the outage
$P -e postgres_encoding_migrate_action=status   # complete
# verify the goatnet (below), then, when the new clusters stay:
$P -e postgres_encoding_migrate_action=cleanup -e postgres_encoding_migrate_confirm=<run id>
```

Every phase leaves a marker per host in `<dir>/<run id>/phase-<n>.done`.
Running the same command again resumes at the first phase not done; a run that
is complete is a no-op.

## Phases

| # | Phase | What it does | Refuses when |
|---|---|---|---|
| 1 | preflight | Reads encodings, Spock nodes and subscriptions, pgBackRest stanza; byte-scans every database and the roles (`pg_dump` → `files/pg-utf8-stream.py scan`); measures disk. Opens the run. Changes nothing. | group_vars declare non-UTF8; a container is down; router_role disagrees with `pg_is_in_recovery()`; the goatnet is part-migrated outside a run; a subscription pulls from a non-member or is not replicating (unless dropped); a named dropped subscription is replicating or pulls from a member; a physical standby of a member is not a host of the run, or a standby of the run is not streaming; a node with subscriptions would not be re-wired by its group_vars; a replicator password is missing; a replicated table's key set differs between members in three samples 15 s apart (unless `accept_divergence`); any foreign key has orphan rows on any node (always); a value is not UTF-8; a filesystem is too small; the Postgres service's compose `environment:` does not pass `POSTGRES_INITDB_ARGS` through (rendered with `docker compose config` and the target encoding set, as phase 5 will start it; the refusal names the host and compose file) |
| 2 | quiesce | Records and stops the active units in `postgres_encoding_migrate_stop_units` (compose-watch, keepalived, the Spock timers, reconcilers), every running container of Postgres's compose project, the declared extras and any container holding a session. Ends the sessions those containers left. | a client session remains (a host process, a timer, a host-network container) |
| 3 | capture | Waits until every Spock slot confirms the quiesce point (a dropped peer's inactive slot excepted); hashes every replicated table on every node; counts every table; takes a full pgBackRest backup; `pg_dumpall --globals-only` and `pg_dump -Fc --create` of every database. Each node dumps its own databases: per-site tables and databases are not on its peers. | slots do not catch up; key sets differ or a foreign key has orphans (the phase 1 checks again, quiesced); a replicated table's content differs between nodes (unless `accept_divergence`) |
| 4 | snapshot | Stops Postgres; copies PGDATA into volume `<volume>-pre-utf8-<run id>` and checks bytes and file count; keeps `.env` and the container's `pg_hba.conf`; moves the pgBackRest repository to `<repo>.pre-utf8-<run id>` and leaves an empty one with the same owner and mode; empties the stanza's archive-async spool (`<spool>/archive/<stanza>`). | the copy does not fit or does not match |
| 5 | reinit | Sets `POSTGRES_INITDB_ARGS="--locale=C --encoding=UTF8"` in the project `.env`; removes the containers holding the old volume and the volume; re-creates Postgres through its own compose project and files, so the image's entrypoint runs initdb and its init scripts. Restores roles (one transaction), then each database through `pg-utf8-stream.py filter`, which declares the stream UTF8 so the server validates every byte. Puts back the old `postgresql.auto.conf` with `archive_mode = 'off'` and without `synchronous_standby_names`, and the old `pg_hba.conf`; restarts. After each start, Postgres must be out of recovery at two reads 10 s apart with no crash in its log. | Postgres does not stay up (the refusal quotes the last FATAL/ERROR lines of the Postgres and pgBackRest logs); archiving is not off; the new cluster or any database is not UTF8; any table's row count differs from phase 3 |
| 6 | standby | Every physical standby of the run (router_role B in a listed group, or a host of `postgres_encoding_migrate_standby_groups`) is rebuilt by `postgres-streaming-replication`'s standby side, forced: PGDATA wiped, `pg_basebackup` from its new primary. The primary's `synchronous_standby_names` (held back in phase 5) goes back. | a standby does not stay up; the primary does not stream to every standby it had in phase 1; a standby is not in recovery, does not read UTF8, or does not carry its new primary's system identifier |
| 7 | pgbackrest | `stanza-create` in the empty repository; then the old `archive_mode` line back, restart, Postgres must stay up; then `check` and a full backup. | stanza-create, staying up, check or the backup fails on a primary (a standby's stanza result is reported) |
| 8 | rewire | 8.1 founders and 8.2 joiners run `spock-mesh-peering`; 8.3 starts kratos and hydra; 8.4 joiners, 8.5 founders, 8.6 joiners run `ory-narrow-repset` in the join order of `goatbuilder join-node` phase 13. Each node is wired only to the members it pulled from before; dropped subscriptions are not created. Subscriptions use `synchronize_data false`; phase 3 proved the data identical. 8.7 compares the subscription set with phase 1's. | a subscription is missing, extra or not replicating |
| 9 | finish | 9.1 in each replicated database, a probe table joins the replication set, each node writes a row of non-ASCII text, every node must hold its own row and one from each provider, intact; the table is removed. 9.2 starts every recorded container (re-creating through compose those removed in phase 5) and then the units. | a probe row does not arrive |

## Peers that are gone

A member may still hold subscriptions to a Spock peer that no longer runs
(a joiner that is powered off, a site that was retired): they are `down` and
their origin is not a member. Phase 1 refuses them, and accepts exactly the
ones named in `postgres_encoding_migrate_drop_subscriptions`:

```bash
$P -e postgres_encoding_migrate_drop_subscriptions=down_nonmembers
$P -e postgres_encoding_migrate_drop_subscriptions=sub_<peer>_to_<member>,sub_<peer>_to_<member2>
```

`down_nonmembers` selects every subscription that is not replicating and
pulls from a node outside `migrate_groups`. Phase 1 reads each
subscription's status five times, 2 s apart, and counts it as replicating
only if every read says so: a subscription whose origin is unreachable reports
`replicating` for a moment each time its apply worker restarts. A named subscription that is
replicating, or pulls from a member, is refused. Phase 1 prints and records
the dropped subscriptions (`dropped` in `<dir>/<run id>/preflight.json`);
phase 3 does not wait for the gone peer's slots; phase 8 does not create
them again; the phase 9 report lists them. The new cluster holds no Spock
node, slot or subscription of the gone peer: phase 5 restores no Spock
catalog, and phase 8 wires each node only to the members it pulled from.

A dropped peer comes back only by joining again at the new encoding:
`goatbuilder join-node` (or `goat-standup.sh`) for that site, which
initialises it with its founder's encoding (site-bootstrap Phase 1C). Until
then take it out of the goatnet's Spock peer lists (`inter_site_spock_peers`,
or its `participates_in_spock`), so a later site-bootstrap with
`full_mesh` does not wire to it. A rollback brings the dropped
subscriptions back as they were (down).

## Physical standbys

Every physical standby that streams from a member is rebuilt in phase 6 and
restored by a rollback, so each must be a host of the run:

- **router_role B in a listed group** (a pair site: `<site>_hosts` holds A
  and B), or
- **a host of a group named in `postgres_encoding_migrate_standby_groups`**
  (a standby kept in a group of its own; those hosts join every phase as
  standbys).

Phase 1 reads `pg_stat_replication` on every primary; a client without a
logical slot is a physical standby. Each is matched to a standby of the run
by `postgres_standby_app_name` or by address (`postgres_standby_lan_ip`,
`ansible_host`, `mesh_ip`). Phase 1 refuses, naming the primary, the client
address and its application_name, a physical client that matches no standby
of the run, and a standby of the run that no member streams to. A standby
needs `postgres_standby_app_name`, `postgres_primary_lan_ip` and
`<SITE>_REPLICATOR_PASS`, as for site-bootstrap Phase 6A.

```bash
$P -e postgres_encoding_migrate_standby_groups=<joiner>_backup_hosts
```

Phase 6 runs only the standby side of `postgres-streaming-replication`:
the primary's replicator role, pg_hba entry and WAL settings came back with
the restore. Its `synchronous_standby_names`, held back in phase 5 so commits
do not wait for a standby that is not there, goes back once the standby
streams. Then the phase requires: the primary streams to every standby it had
in phase 1; each standby is in recovery, reads UTF8 and carries its new
primary's system identifier. A rollback restores each standby's kept PGDATA,
and checks that every primary streams to its standbys again.

## keepalived and the VIP on a pair site

`keepalived.service` is in `postgres_encoding_migrate_stop_units`, so phase 2
stops it on A and on B and the site's VIP is down from phase 2 to phase 9
(the mgmt API is down for that time anyway). With keepalived stopped on both,
nothing can move the VIP to B or run its notify script while A's Postgres is
re-created. Phase 9 starts keepalived last, on every primary first and then
on the standbys: A, with the higher priority and its stack up, takes MASTER
before B's keepalived starts. Then every node must still be what it was (a
primary out of recovery, a standby in recovery) at two reads 10 s apart. With
`keepalived_auto_promote_postgres: false` (the role's default) a MASTER
transition never promotes Postgres; a goatnet that sets it true relies on
that order.

## Archiving during the migration

WAL archiving is off on the new cluster from its initdb in phase 5 until
phase 7 has created its pgBackRest stanza. Phase 5 writes the old
`postgresql.auto.conf` back with `archive_mode = 'off'` and keeps the old
line in `<dir>/<run id>/archive-mode.pre`; phase 7 runs `stanza-create`, puts
that line back, restarts, and only then runs `pgbackrest check` (which needs
archiving on: it switches WAL and waits for the segment in the repository)
and the first full backup. A postmaster therefore never starts with an
archive command pointing at a repository that has no stanza. With
`archive-async`, a failing push in that state takes Postgres into a crash
loop.

WAL written between phase 5 and phase 7 is not archived. The full backup of
the old cluster in phase 3 and the first full backup of the new cluster in
phase 7 bracket that window; nothing in it can be replayed from the archive.

Phase 4 and a rollback also empty the stanza's archive-async spool: its
status files name the WAL segments one cluster pushed, and the other
cluster's segments can carry the same names.

## Verify after the run

- `$P -e postgres_encoding_migrate_action=status` on the same groups: `complete`.
- The playbook's 9.2 output lists any container that did not come back.
- `goat health` for the goatnet, a sign-in, an enrolment.

## Values that are not UTF-8

Phase 1 lists, per database and table, how many rows hold bytes that are not
valid UTF-8, with each sample row's first field (usually its primary key) and
the bytes around the first invalid one. Two ways forward:

- **Correct the rows** in the live SQL_ASCII cluster, then run again.
- **They were written as LATIN1** (or a Windows single-byte code page whose
  printable range matches it): run with `-e postgres_encoding_migrate_repair=latin1`.
  Each byte that is not part of a valid UTF-8 sequence is read as a LATIN1
  character and stored as its UTF-8 encoding; valid UTF-8 sequences are kept
  as they are. The choice is recorded in the run's `preflight.json`.

Without a repair choice the restore also refuses an invalid byte, so a run
never loads altered text silently.

## Divergent rows and foreign-key orphans

Rows a node wrote before its Spock subscriptions existed never reach its
peers: the subscriptions are created with `synchronize_data false`. A
service that writes its own row while migrating its schema leaves exactly
that: Ory's `networks` table in `kratos` and `hydra` holds the shared row on
every node plus each node's own row, present only on that node. Rows that
reference such a row and do replicate (Ory's `hydra_client`, kratos
identities) arrive on a peer whose `networks` lacks their parent. Spock
applies replicated rows without enforcing foreign keys, so the live cluster
accepts them; the restore re-creates every foreign key and validates it, and
would fail in phase 5 with the goatnet down.

Phase 1 therefore refuses:

- **foreign-key orphans**, always — per node, database, child table,
  constraint, parent table and row count. `accept_divergence` does not
  change this.
- **replicated tables whose key sets differ** between members, unless
  `accept_divergence=true` — per table, which member lacks how many keys
  (the first five keys listed).

When the missing rows exist on another member, copy them across with the
reconcile action; the refusal prints the table list to pass:

```bash
$P -e postgres_encoding_migrate_action=reconcile -e postgres_encoding_migrate_reconcile_tables=kratos.public.networks,hydra.public.networks
```

For each named table (`<database>.<schema>.<table>`), every member receives
the rows its peers hold and it lacks, matched by primary key, in one
transaction per table with Spock's repair mode on, so the insert is not
replicated back out. It then runs the key-set and orphan checks again and
reports them; it fails if a named table still differs. It refuses, writing
nothing, when a key exists on two members with different content, a table
has no primary key or more than 10000 rows, or a run is open past phase 1.

The writers stay up during reconcile. For Ory's `networks` that is safe: Ory
writes that table only when it migrates its schema. For a table services
write in normal operation, stop its writers first.

When no member holds the missing parent rows, the child rows themselves are
wrong: delete them or insert their parents by hand, then run phase 1 again.

## Rollback

Puts every member back on its pre-migration cluster: the kept PGDATA volume,
`.env`, `pg_hba.conf` and pgBackRest repository, then starts what phase 2
stopped and waits for the old subscriptions to replicate. It acts on the whole
run at once, because the old Spock slots and each standby's position line up
only when every member returns to the same quiesce point. Writes made to the
new clusters after phase 9 are not carried back.

```bash
$P -e postgres_encoding_migrate_action=rollback -e postgres_encoding_migrate_confirm=<run id>
```

Available from any phase until cleanup. A run that stopped before phase 4
finished has destroyed nothing; rollback then only starts what was stopped.
The new cluster's pgBackRest repository is kept as `<repo>.utf8-<run id>`.

After a rollback the run is closed (`<dir>/<run id>/rolled-back`, and
`<dir>/current-run` removed), so the next `migrate` opens a new run with a
new run id; nothing of the rolled-back run is reused. The rolled-back run
still keeps, per node, its copy volume `<volume>-pre-utf8-<run id>`
(identical to the live data rollback restored from it), the set-aside
`<repo>.utf8-<run id>` and its dumps. Phase 1 of the next run lists those
volumes, and its space check counts them as used. Remove them with cleanup
and the rolled-back run's id, before or after the next run:

```bash
$P -e postgres_encoding_migrate_action=cleanup -e postgres_encoding_migrate_confirm=<rolled-back run id>
```

## Cleanup

Deletes what one run kept, named by `postgres_encoding_migrate_confirm`:

| Run state | Deleted |
|---|---|
| complete (phase 9 done) | the old PGDATA volume, the old pgBackRest repository `<repo>.pre-utf8-<run id>`, the dumps. After this there is no rollback: run it only once the goatnet is verified on the new clusters |
| rolled back | the copy volume, the new cluster's repository `<repo>.utf8-<run id>`, the dumps |

An open run that has not completed is refused. Reports, row counts and the
stop record stay in `<dir>/<run id>/`, marked `cleaned`. `action=status`
lists every run a host has seen, its state, and what it still keeps.

## Inputs

| Variable | Default | Meaning |
|---|---|---|
| `migrate_groups` | (required) | Comma-separated groups holding the goatnet's Spock members and standbys |
| `postgres_encoding_migrate_action` | `migrate` | `migrate`, `reconcile`, `status`, `rollback`, `cleanup` |
| `postgres_encoding_migrate_stop_after` | `9` | Last phase to run |
| `postgres_encoding_migrate_confirm` | | Run id; required by rollback and cleanup |
| `postgres_encoding_migrate_repair` | `none` | `latin1` reinterprets invalid bytes |
| `postgres_encoding_migrate_accept_divergence` | `false` | Carry a replicated-table difference over unchanged (foreign-key orphans still refuse) |
| `postgres_encoding_migrate_reconcile_tables` | | `reconcile`: `<db>.<schema>.<table>`, comma-separated |
| `postgres_encoding_migrate_drop_subscriptions` | `[]` | Subscriptions to gone peers to drop: names, or `down_nonmembers` |
| `postgres_encoding_migrate_standby_groups` | | Groups of physical standbys not in `migrate_groups`, comma-separated |
| `postgres_encoding_migrate_extra_containers` | `[]` | Writers outside Postgres's compose project |
| `postgres_encoding_migrate_stop_units` | see role defaults | Units stopped and started again when active |
| `postgres_encoding_migrate_dir` | `/var/lib/goat/postgres-utf8-migration` | Run state, reports and dumps (mode 0700; the dumps hold every row and every role's password hash) |

All defaults: `ops/ansible/roles/postgres-encoding-migrate/defaults/main.yml`.

## Tests

`ops/ansible/roles/postgres-encoding-migrate/tests/syntax-check.sh` — parse
checks and the stream tool; runs in CI (`ansible-syntax-check.yml`).

`ops/ansible/roles/postgres-encoding-migrate/tests/run-test.sh` — on one
Docker host, on the pinned postgres-spock image: a founder and a joiner
(SQL_ASCII, wired with `spock-mesh-wire.yml`, pgBackRest on the founder with
the role's settings: `archive-async`, spool, `archive_timeout`); a third Spock
node stopped before the run; a physical standby of the founder in a group of
its own, built by `postgres-streaming-replication`; writers inside and
outside the compose project; on each node a parent row written before Spock
was wired and a replicated child row referencing it.

| Case | Proves |
|---|---|
| D | phase 1 refuses the gone peer's subscriptions; refuses dropping a member's replicating subscription; refuses the standby until its group is named, giving its address and application_name |
| 0 | phase 1 refuses the orphan and the divergent parent (with `accept_divergence` too); reconcile refuses a conflicting key, then copies the missing rows without a Spock echo |
| 1 | an invalid byte is refused with its table |
| 2 | a run stopped after phase 4 resumes; the gone peer's subscriptions are dropped (`down_nonmembers`) |
| 3 | encodings, row counts, text bytes, owners, database settings, sequences, role passwords; replication both ways; writers back; no crash restart, archiving back on, `pgbackrest check`; phase 6: the standby carries the new founder's system identifier, is in recovery, reads UTF8, streams and receives a write; the gone peer's subscriptions, node and slots absent, and listed in the final report |
| 4 | a re-run changes nothing |
| 5 | rollback: SQL_ASCII, old rows, old subscriptions (the gone peer's back, down), old repository; the standby back on the old cluster and streaming |
| 6 | a second run (new run id, named dropped subscriptions, `repair=latin1`); cleanup of the completed and the rolled-back run |

The Ory steps (8.3–8.6) are not exercised: the fixture has no Ory
containers.
