# postgres-spock

Spock-enabled PostgreSQL 16 image for the goat multi-site mesh.

Design: `docs/design/ha-multi-site.md` §5.

## What it is

A thin overlay on `ghcr.io/pgedge/pgedge-postgres:16-spock5-minimal`:

- Config file baked in (`postgresql.conf`, `pg_hba.conf`) tuned for Spock.
- Init SQL (`init/*.sql`) runs on first boot: verifies config, creates the
  three Netbird databases, loads the Spock extension per replicated DB.

Node registration (`spock.node_create`) and subscriptions (`spock.sub_create`)
are driven by Ansible per-site, not baked in — every node needs site-specific
DSNs, peer lists, and a replicator password.

Published as `ghcr.io/usareur-af/postgres-spock` by the matching CI workflow
([.github/workflows/postgres-spock.yml](../../../.github/workflows/postgres-spock.yml)),
multi-arch (`linux/amd64` + `linux/arm64`).

## Layout

```
ops/postgres/spock/
├── Dockerfile
├── conf/
│   ├── postgresql.conf      baked into image; used via `-c config_file=...`
│   └── pg_hba.conf          same; Ansible renders site-specific replicator
│                            lines between the BEGIN/END markers at deploy time
├── init/
│   ├── 00-verify-config.sql fails fast if wal_level/spock preload missing
│   ├── 01-databases.sql     CREATE DATABASE netbird, netbird_idp, netbird_events
│   └── 02-spock-extension.sql CREATE EXTENSION spock in replicated DBs
└── README.md
```

## Run

Always start with the baked config path:

```
docker run -d --name pg-a \
  -p 5432:5432 \
  -e POSTGRES_PASSWORD=... \
  ghcr.io/usareur-af/postgres-spock:latest \
  postgres -c config_file=/etc/postgresql/postgresql.conf
```

The `init/00-verify-config.sql` script will abort startup if you forget the
`-c config_file=...` and end up with the initdb-default `postgresql.conf`
(which has `wal_level=replica`).

## Spock prerequisites encoded in the config

The bits of `postgresql.conf` that matter for replication correctness
(captured here because it's easy to drift):

| Setting | Value | Why |
|---|---|---|
| `wal_level` | `logical` | Required for any logical replication |
| `max_replication_slots` | `32` | One slot per remote subscriber per DB, and one for a streaming standby. See "Slot and sender capacity" |
| `max_wal_senders` | `32` | Same — one per outbound replication stream |
| `max_slot_wal_keep_size` | set here, and raised on a pair's primary (both values below) | Safety valve — invalidates a slow slot before disk fills. **Critical for island-mode scenarios.** See "The slot WAL cap" |
| `track_commit_timestamp` | `on` | Required for LWW conflict resolution |
| `shared_preload_libraries` | `spock` | Loads the Spock extension at startup |

## Slot and sender capacity

Each site holds one logical replication slot, and one WAL sender, for every
other site that subscribes to each replicated database (`netbird` from
`spock-mesh-peering`, `kratos` and `hydra` from `ory-narrow-repset`), and one
more for a streaming standby when the site has a pair. With three databases
that is three for each other site, so the earlier limit of 10 ran out at a
four-site goatnet: 9 logical slots and a standby's fill all 10, leaving none
for another subscriber. The image allows 32 of each. A change of either setting takes effect only when Postgres restarts. On
a site with a streaming pair, the standby needs values at least as large as
the primary's, so it restarts on the new image first.

`max_worker_processes` (16) may become the limit before the slots do; measured value to follow.

## The slot WAL cap

`max_slot_wal_keep_size` is the most WAL a replication slot may hold before
Postgres invalidates it, and the subscriber behind it must be seeded again
(`docs/operations/post-partition-convergence-runbook.md` §A). Two places set
it, and both values are meant:

| Where | Value | Which Postgres |
|---|---|---|
| This image's `conf/postgresql.conf`, which every site's `postgres-a` starts with (`-c config_file=`) | 1 GB | every site |
| The `postgres-streaming-replication` role, variable `postgres_max_slot_wal_keep_size`, written with `ALTER SYSTEM` | 6 GB | the primary of a site with a streaming pair (`intra_site_pair: true`), and its standby, whose data directory is a copy of the primary's |

Postgres reads `postgresql.auto.conf`, where `ALTER SYSTEM` writes, after the
configuration file, so on a pair's primary the role's value is the one in
effect. Nothing passes the setting on the command line. Every other site runs
with the image's value.

The two differ because the hosts do. A site with a streaming pair runs its
database on the storage tier sized for it, and holds a slot's WAL for longer
outages: 6 GB is about 18 hours at the write rate measured during F-093
(about 330 megabytes an hour), and days at a normal rate. A site with no pair may be
one small machine, such as a goatlite or a phone node, and the image keeps its
worst case to 1 GB for each slot it holds (at most `max_replication_slots`, 32).
The slots it holds are the ones its subscribers use (see "Slot and sender
capacity"), not the maximum.

How long a subscriber may be away depends on the site that holds its slot:
the cap there divided by that site's WAL rate. Read the live value on that
site (`SHOW max_slot_wal_keep_size;`, or the exporter's
`spock_config_max_slot_wal_keep_size_bytes`). Never assume either number.
`ops/ci/check-slot-wal-cap-agrees.py` holds this section's table to the two
values, row by row: the image's as Postgres reads it (the last assignment,
includes followed) and the role's default. It refuses a command line that
passes the setting, and a size written beside `max_slot_wal_keep_size` in any
other file, except a file listed with its reason in
`ops/ci/slot-wal-cap-mentions.allow`.

## Operational notes

- **Coordinated upgrades**: Netbird AutoMigrate-emitted DDL does not replicate
  through Spock. Follow `docs/design/ha-multi-site.md` §6
  before upgrading Netbird at any site.
- **Slot invalidation recovery**: if `max_slot_wal_keep_size` is exceeded
  during a multi-day partition, the slot goes invalid and the disconnected
  node must be rebuilt via `pg_basebackup` + re-subscribe. Runbook:
  [`docs/operations/post-partition-convergence-runbook.md`](../../../docs/operations/post-partition-convergence-runbook.md) §A.
