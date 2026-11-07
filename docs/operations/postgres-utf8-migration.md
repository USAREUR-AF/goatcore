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
| 1 | preflight | Reads encodings, Spock nodes and subscriptions, pgBackRest stanza; byte-scans every database and the roles (`pg_dump` → `files/pg-utf8-stream.py scan`); measures disk. Opens the run. Changes nothing. | group_vars declare non-UTF8; a container is down; router_role disagrees with `pg_is_in_recovery()`; the goatnet is part-migrated outside a run; a subscription pulls from a non-member or is not replicating; a node with subscriptions would not be re-wired by its group_vars; a replicator password is missing; a value is not UTF-8; a filesystem is too small |
| 2 | quiesce | Records and stops the active units in `postgres_encoding_migrate_stop_units` (compose-watch, keepalived, the Spock timers, reconcilers), every running container of Postgres's compose project, the declared extras and any container holding a session. Ends the sessions those containers left. | a client session remains (a host process, a timer, a host-network container) |
| 3 | capture | Waits until every Spock slot confirms the quiesce point; hashes every replicated table on every node; counts every table; takes a full pgBackRest backup; `pg_dumpall --globals-only` and `pg_dump -Fc --create` of every database. Each node dumps its own databases: per-site tables and databases are not on its peers. | slots do not catch up; a replicated table differs between nodes |
| 4 | snapshot | Stops Postgres; copies PGDATA into volume `<volume>-pre-utf8-<run id>` and checks bytes and file count; keeps `.env` and the container's `pg_hba.conf`; moves the pgBackRest repository to `<repo>.pre-utf8-<run id>` and leaves an empty one with the same owner and mode. | the copy does not fit or does not match |
| 5 | reinit | Sets `POSTGRES_INITDB_ARGS="--locale=C --encoding=UTF8"` in the project `.env`; removes the containers holding the old volume and the volume; re-creates Postgres through its own compose project and files, so the image's entrypoint runs initdb and its init scripts. Restores roles (one transaction), then each database through `pg-utf8-stream.py filter`, which declares the stream UTF8 so the server validates every byte. Puts back the old `postgresql.auto.conf` (without `synchronous_standby_names`) and `pg_hba.conf`; restarts. | the new cluster or any database is not UTF8; any table's row count differs from phase 3 |
| 6 | standby | On a pair site, runs `postgres-streaming-replication` forced: A gets its synchronous-standby settings back, B is wiped and re-copied with `pg_basebackup`. | the role's own checks |
| 7 | pgbackrest | `stanza-create`, `check`, full backup into the empty repository. | any of the three fails on a primary (a standby's result is reported) |
| 8 | rewire | 8.1 founders and 8.2 joiners run `spock-mesh-peering`; 8.3 starts kratos and hydra; 8.4 joiners, 8.5 founders, 8.6 joiners run `ory-narrow-repset` in the join order of `goatbuilder join-node` phase 13. Each node is wired only to the members it pulled from before. Subscriptions use `synchronize_data false`; phase 3 proved the data identical. 8.7 compares the subscription set with phase 1's. | a subscription is missing, extra or not replicating |
| 9 | finish | 9.1 in each replicated database, a probe table joins the replication set, each node writes a row of non-ASCII text, every node must hold its own row and one from each provider, intact; the table is removed. 9.2 starts every recorded container (re-creating through compose those removed in phase 5) and then the units. | a probe row does not arrive |

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
After a rollback the run is closed and the next `migrate` opens a new one.

## Cleanup

Deletes the kept PGDATA volume, the old pgBackRest repository and the dumps.
Run it only after the goatnet has been verified on the new clusters: after
cleanup there is no rollback. Reports, row counts and the stop record stay in
`<dir>/<run id>/`.

## Inputs

| Variable | Default | Meaning |
|---|---|---|
| `migrate_groups` | (required) | Comma-separated groups holding the goatnet's Spock members and standbys |
| `postgres_encoding_migrate_action` | `migrate` | `migrate`, `status`, `rollback`, `cleanup` |
| `postgres_encoding_migrate_stop_after` | `9` | Last phase to run |
| `postgres_encoding_migrate_confirm` | | Run id; required by rollback and cleanup |
| `postgres_encoding_migrate_repair` | `none` | `latin1` reinterprets invalid bytes |
| `postgres_encoding_migrate_accept_divergence` | `false` | Carry a replicated-table difference over unchanged |
| `postgres_encoding_migrate_extra_containers` | `[]` | Writers outside Postgres's compose project |
| `postgres_encoding_migrate_stop_units` | see role defaults | Units stopped and started again when active |
| `postgres_encoding_migrate_dir` | `/var/lib/goat/postgres-utf8-migration` | Run state, reports and dumps (mode 0700; the dumps hold every row and every role's password hash) |

All defaults: `ops/ansible/roles/postgres-encoding-migrate/defaults/main.yml`.

## Tests

- `ops/ansible/roles/postgres-encoding-migrate/tests/syntax-check.sh` — parse
  checks and the stream tool; runs in CI (`ansible-syntax-check.yml`).
- `ops/ansible/roles/postgres-encoding-migrate/tests/run-test.sh` — on one
  Docker host: a founder and a joiner on the pinned postgres-spock image,
  SQL_ASCII, wired with `spock-mesh-wire.yml`, a pgBackRest stanza on the
  founder, writers inside and outside the compose project. It checks the
  refusal of an invalid byte, a run stopped after phase 4 and resumed, the
  result (encodings, counts, text bytes, owners, database settings, sequences,
  role passwords, subscriptions, a write each way, writers, the new backup), a
  no-op re-run, rollback, and a second run with `repair=latin1` followed by
  cleanup. The streaming standby (phase 6) and the Ory steps (8.3–8.6) are not
  exercised by it: the fixture has no pair site and no Ory containers.
