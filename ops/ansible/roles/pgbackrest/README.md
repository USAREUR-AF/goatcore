# pgbackrest role

Per-site pgBackRest deployment for goat mgmt-hosts. Install the
pgBackRest binary inside the Postgres container, wire `archive_command`
via `ALTER SYSTEM`, and schedule weekly-full / daily-diff / 6-hour-incr
backups via host cron with continuous WAL archive. **Single local repo
per host.** No commercial cloud in the backup path.

One stanza per *host*, not per site: the role runs once on Router-A
(stanza `<site>-a-netbird`) and once on Router-B (stanza
`<site>-b-netbird`). Each stanza writes to `/opt/docker-data/pgbackrest/`
on its own NVMe. The supported role requires `archive_mode=on` on both
nodes; `always` is rejected by its preflight. Use the backup standby option
for the supported streaming-replica backup path.

## Source of truth

Design: [`docs/design/pgbackrest.md`](../../../../docs/design/pgbackrest.md).

Config template:
[`ops/ansible/roles/pgbackrest/templates/pgbackrest.conf.j2`](templates/pgbackrest.conf.j2)

Reference config (kept in sync manually):
`ops/pgbackrest/pgbackrest.conf`

## Existing-host config migration

The base Compose mounts `/etc/pgbackrest` read-only. Before copying that
Compose file, `mgmt-host.yml`, `traefik-acme.yml`, `site-bootstrap.yml` and
`zagros-federation-transport.yml` run the shared config preflight. An existing
config must be a regular file owned by `26:0`, mode `0640`; otherwise the
play stops before publishing a definition that could recreate Postgres.
Converge this role's config with the existing repository cipher passphrase
first. The guard reads metadata and does not change permissions or keys.
A host without pgBackRest config can still bootstrap. Custom config paths
require a matching reviewed Compose definition and are refused by the current
fixed-path integration.

The smoke assertion requires recreation to succeed, the container ID to change,
the replacement to become ready, and `pgbackrest check` to pass. Its command
fixtures do not establish restore integrity or database availability during a
real recreation; the full database smoke and recovery drill remain necessary.

## Variables

Required per-host (set in inventory):

| Var | Example | Meaning |
|---|---|---|
| `site_name` | `vega-a` | stanza becomes `{site_name}-netbird` (e.g. `vega-a-netbird`). Use `<site>-a` on Router-A and `<site>-b` on Router-B. |
| `pgbackrest_archive_mode` | `on` | Postgres `archive_mode`. pgBackRest rejects `always` with error [067] (Wave 1.8 smoke test finding). Use `on` on BOTH sides of a paired topology. Backup I/O offload to the standby uses `--backup-standby` on the backup command, not `archive_mode`. See [`docs/design/pgbackrest.md`](../../../../docs/design/pgbackrest.md) §4.1. |

Optional per-host (override only when needed):

| Var | Default | Override use case |
|---|---|---|
| `postgres_container_name` | `postgres-a` | Set to `postgres-b` when running the role against Router-B's container. In production each host runs only one Postgres container named `postgres-a`, so the default works; override matters for single-host two-container smoke tests (Wave 1.8 with the phase2 compose overlay). |
| `pgbackrest_repo1_dir` | `/opt/docker-data/pgbackrest` | Set to `/opt/docker-data/pgbackrest-b` on the B-side of a single-host two-container smoke test so the two stanzas do not collide on the same host directory. |
| `pgbackrest_log_dir` | `/var/log/pgbackrest` | Same pattern — override to `/var/log/pgbackrest-b` only when A and B share one host. |
| `pgbackrest_spool_dir` | `/var/spool/pgbackrest` | Same pattern — override to `/var/spool/pgbackrest-b` only when A and B share one host. |

### Router-B invocation example (single-host paired-stanza smoke test)

```
ansible-playbook ... pgbackrest-wave18.yml \
  -e site_name=hadev-b \
  -e postgres_container_name=postgres-b \
  -e pgbackrest_repo1_dir=/opt/docker-data/pgbackrest-b \
  -e pgbackrest_log_dir=/var/log/pgbackrest-b \
  -e pgbackrest_spool_dir=/var/spool/pgbackrest-b \
  -e pgbackrest_archive_mode=on \
  -e pgbackrest_repo1_cipher_pass="$PGBACKREST_CIPHER_PASS_B"
```

In production (Router-A and Router-B on separate hosts), none of the
path overrides are needed — each host runs its own single Postgres
container named `postgres-a` and writes to the default repo paths on
its own NVMe.

Required at play invocation (from env / 1Password):

| Var | Meaning |
|---|---|
| `postgres_superuser_password` | to run `ALTER SYSTEM`, install pgbackrest inside postgres-a, and invoke `pg_reload_conf` |
| `pgbackrest_repo1_cipher_pass` | aes-256-cbc passphrase for local repo (distinct per host) |

Tunable defaults live in [`defaults/main.yml`](defaults/main.yml).

## What the role does (chronologically)

1. Assert required vars.
2. Create host dirs: `/etc/pgbackrest`, `/opt/docker-data/pgbackrest`,
   `/var/log/pgbackrest`, `/var/spool/pgbackrest`.
3. Render `/etc/pgbackrest/pgbackrest.conf` from the Jinja template
   (owner uid `26`, group `root`, mode `0640`; secrets redacted by
   `no_log`). The base compose bind-mounts `/etc/pgbackrest` read-only
   into the Postgres container, so the file survives every recreate.
4. Remove the retired `pgbackrest-runner` sidecar from a host that still has it:
   `/opt/mgmt-host/docker-compose.pgbackrest.yml`, the container, and the
   `PGBACKREST_IMAGE` block in `/opt/mgmt-host/.env`. The repository, log and
   spool directories are host bind mounts and are not touched. Before that, the
   role refuses a repository, log or spool directory holding files not owned by
   uid `26` and names them (`tasks/repo-ownership.yml`, read-only).
5. (none)
6. Install the `pgbackrest` RPM inside the running Postgres container
   (idempotent: gated on `command -v pgbackrest`; **normally a no-op**
   since pgbackrest is baked into the `postgres-spock` image — see
   [`ops/postgres/spock/Dockerfile`](../../../postgres/spock/Dockerfile).
   Kept as a fallback for older-pinned images).
7. Confirm the container's `postgres` user can read
   `/etc/pgbackrest/pgbackrest.conf`, and fail the run if not. A container
   created before the bind mount existed gets a copy instead (`docker cp`)
   and a warning: that copy is lost on the next recreate, so recreate the
   container from the current compose to switch it to the mount.
8. `ALTER SYSTEM SET archive_mode/archive_command/archive_timeout` +
   `pg_reload_conf()`. `archive_mode` value drives per-host behavior
   (`on` on both nodes; `always` is refused).
9. Restart the Postgres container if `archive_mode` was not already at
   the target value (`archive_mode` only takes effect at server start).
10. `pgbackrest stanza-create` + `pgbackrest check` + first full backup
    (doubles as deployment smoke test).
11. Schedule three cron jobs (full / diff / incr) under `root`'s
    crontab.

## Backup scope

pgBackRest works at the cluster level, not per-database. All databases
in the Postgres container (`netbird`, `netbird_idp`, `netbird_events`)
are captured by every full/diff/incr. The per-site-audit scope described
in `docs/design/ha-multi-site.md` §5.6 is about **replication** scope;
backup scope is always the full cluster.

## Idempotency

- Host dirs: `state: directory`.
- Config render: `template` — overwrites only on diff, secrets masked
  via `no_log`.
- Compose copy + env append: `copy` + `blockinfile` with fixed markers.
- pgbackrest install in container: gated on `command -v pgbackrest` —
  no-op when the `postgres-spock` image ships pgbackrest baked in
  (default).
- `ALTER SYSTEM SET archive_mode`: `changed_when` compares current vs
  target.
- Postgres container restart: only if `archive_mode` was not already
  at the target value.
- `stanza-create`: swallows "already exists" stderr.
- First full backup: runs every time (quick at Phase 2 scale; future
  iteration could gate on `pgbackrest info` returning an existing
  full).
- Cron: `ansible.builtin.cron` name-keyed, replaces in place.

## Operator pre-reqs (ONCE per host)

1. **Generate a cipher pass per host**. Offline:
   ```
   openssl rand -hex 32   # <site>-a-netbird (Router-A)
   openssl rand -hex 32   # <site>-b-netbird (Router-B)
   ```
   Store in 1Password as:
   - `goat — pgBackRest <site> router-a`
   - `goat — pgBackRest <site> router-b`

   Two distinct keys means compromise of one host's repo doesn't unlock
   the other host's repo.

2. **Confirm `/opt/docker-data` is mounted on the NVMe.** On the Orin
   pilot the Ansible provisioning role handles mount; on NETHOST/hadev it
   may be the root filesystem. The role creates subdirectories, not the
   mount point itself.

## Not in scope for this role

Out of scope items, with pointers to where they land:

| Item | Where it lives |
|---|---|
| Cross-site backup durability | Spock multi-master replication over the WireGuard mesh (`docs/design/ha-multi-site.md`). pgBackRest does not cross sites. |
| Total-loss (all sites) recovery | Bootstrap bundle + offline-enrollment ([`docs/design/bootstrap-bundle.md`](../../../../docs/design/bootstrap-bundle.md), [`docs/design/offline-enrollment.md`](../../../../docs/design/offline-enrollment.md)). |
| Monitoring integration (`PgBackRestJobFailed`, `PgBackRestWALArchiveOverdue` alerts) | Depends on `pgbackrest-exporter` sidecar + prometheus.yml scrape config in the observability role. Separate PR. Wire the alert rules from [design §8](../../../../docs/design/pgbackrest.md#8-monitoring--alerting) into `ops/compose/mgmt-host/monitoring/rules/`. |
| Async archive spool persistence across container restarts | Requires bind-mounting `/var/spool/pgbackrest` into the Postgres container. Open question §10.2 in the design. Phase 3 compose refactor (coordinated with other base-compose changes). Meanwhile, `archive-async=y` still works in-sidecar; the postgres-side writes segments to an ephemeral spool. |
| `pgbackrest` RPM version pin | `pgbackrest_rpm_spec` default is unpinned. Pin to `pgbackrest-2.56.0` or similar once we've completed one clean install + upgrade cycle on the region A PoC hosts. |
| Restore drill under stress (Wave 3 §3.3) | Scheduled for the Phase 2 hadev smoke-test week per [design §9](../../../../docs/design/pgbackrest.md#9-phase-2-smoke-test). Checklist lives in `docs/operations/goat-ha-dev-validation.md` (Wave 1.8). |

## Smoke-testing the deploy

After a successful run on each host:

```
# 1. Stanza info shows one full backup, WAL being archived.
ssh <mgmt-host> "docker exec <postgres-container> pgbackrest --stanza=<site>-netbird info"

# 2. Wait ~60s and check archive status reports archived>0.
ssh <mgmt-host> "docker exec -e PGPASSWORD=... <postgres-container> psql -U postgres -tAc \
  \"SELECT archived_count, last_archived_wal FROM pg_stat_archiver;\""

# 3. Confirm the local repo has backup + archive prefixes populated.
ssh <mgmt-host> "ls /opt/docker-data/pgbackrest/"
# expected: archive/ and backup/ directories
```

See `docs/design/pgbackrest.md` §9 for the full Phase 2 smoke-test
checklist (the Wave 1.8 drill on the hadev EC2 pair — which is a
two-host topology that exercises both stanzas simultaneously).
