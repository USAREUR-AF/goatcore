# postgres-streaming-replication

Per-site intra-LAN Postgres streaming replication between Router-A
(primary) and Router-B (hot standby). Idempotent — detects an
already-healthy standby and skips the destructive basebackup.

This is the **intra-site** replication path (single WAL stream,
read-only standby). **Spock is the inter-site path** (multi-master,
async, LWW conflict resolution); the two are complementary and not
interchangeable. See `docs/design/ha-failover.md` Phase B for design
context.

## What it does

1. **Primary side:** ensures a `replicator` PostgreSQL role exists with
   the supplied password (creates or rotates), then sets
   `synchronous_standby_names = '<standby_app_name>'` and
   `synchronous_commit = remote_write` and reloads the server.
2. **Standby detection:** queries `pg_is_in_recovery()` and
   `pg_stat_wal_receiver` on the local postgres-a container. If the
   standby is already streaming and `force_rebuild=false`, skip the
   destructive rebuild.
3. **Standby rebuild (when needed):**
   - Stops `mgmt-host-compose-watch.service` (else it races the wipe).
   - `docker compose stop postgres-a management-a signal-a traefik`.
   - `find <data_volume> -mindepth 1 -delete` (catches dotfiles, which
     `rm -rf <dir>/*` misses).
   - `docker run --rm --network=host postgres:16-alpine pg_basebackup
     -h <primary> -U replicator -D /data -Fp -Xs -P -R`. The
     `-R` flag writes `standby.signal` + `primary_conninfo` so
     postgres-a boots as a standby on next start.
   - `chown -R 26:26` on the data dir (basebackup ran as root via
     docker; postgres-spock runs as RHEL UID 26).
   - `chmod 700` on the data dir (postgres rejects looser perms).
   - Patches `postgresql.auto.conf` to include
     `application_name=<standby_app_name>` in `primary_conninfo` so the
     primary's `synchronous_standby_names` matches.
   - `docker compose up -d` for the same four services.
   - Waits for postgres-a healthcheck to report `healthy`.
   - Restarts the compose watchdog.
4. **Verify:** primary's `pg_stat_replication` shows the standby in
   `state=streaming sync_state=sync` with `replay_lag` near zero.

## Required vars

| Var | Where | Example |
|---|---|---|
| `postgres_primary_lan_ip` | per-site `group_vars/<site>_hosts/vars.yml` | `192.168.1.132` |
| `postgres_primary_app_name` | per-site `group_vars/<site>_hosts/vars.yml` | `orin_a` |
| `postgres_standby_lan_ip` | per-site `group_vars/<site>_hosts/vars.yml` | `192.168.1.128` |
| `postgres_standby_app_name` | per-site `group_vars/<site>_hosts/vars.yml` | `orin_b` |
| `postgres_replicator_password` | env (`-e` extra-var) | from `<SITE>_REPLICATOR_PASS` in `.envrc.local` |

## Optional vars (with defaults)

| Var | Default | Notes |
|---|---|---|
| `postgres_major_version` | `16` | The basebackup client image's major MUST match the server's (`postgres:14-alpine` cannot basebackup against postgres-spock 16.x — incompatible_protocol; surfaced). |
| `postgres_streaming_force_rebuild` | `false` | Force a destructive rebuild even if the standby is healthy. |
| `postgres_streaming_synchronous` | `true` | Sync replication for stronger failover durability; small write-latency cost. |

## Why `ansible.builtin.command + docker exec` instead of `community.docker.docker_container_exec`

The `community.docker.docker_container_exec` Ansible module requires
the Python `docker` SDK to be installed on the target host. On a
freshly-flashed Jetson AGX Orin (Ubuntu 22.04 + JetPack 6.1), the
Python `docker` package is NOT installed by default — even though the
`docker` CLI binary IS. The module hangs silently for many minutes
during import-retry / pip-discovery with no output and no error.

This role uses `ansible.builtin.command` with `argv:` lists invoking
the `docker exec` CLI directly. No Python SDK on the target is
required. The CLI is sufficient and is already a precondition of the
mgmt-stack.

See F-056
for the surfaced incident.

## Why `find -delete` not `rm -rf <dir>/*`

The glob `<dir>/*` does not match dotfiles or hidden state. Postgres
clusters use a flat data dir but some files (like a corrupted partial
basebackup left over from a failed previous attempt) can hide. `find
... -mindepth 1 -delete` catches everything cleanly. Surfaced
when the first basebackup attempt left `pg_basebackup: directory exists
but is not empty` because postgres-a was being restarted by the
compose-watch watchdog mid-wipe and re-creating files.

## Idempotency

Re-runs against an already-healthy standby skip the destructive rebuild
entirely (the role's Phase 2 detection handles this). Re-runs against
a freshly-rebuilt standby see synchronous settings already applied and
report no change (the ALTER SYSTEM is unconditional but no-op-on-equal).

## Where this role is invoked

- `ops/ansible/site-bootstrap.yml` Phase 6 (per-site turnkey
  orchestrator, between management-a initial bring-up and the smoke
  check, with per-site var overrides).
- `ops/ansible/site-split-brain-recovery.yml` (re-basebackup of the
  standby).

## Related

- `docs/design/ha-failover.md` — design context.
- [`ops/ansible/roles/keepalived-mgmt/`](../keepalived-mgmt/) — VRRP layer that owns the VIP and orchestrates `pg_ctl promote` on B during A failure.
- [`ops/ansible/roles/mgmt-host-compose-watch/`](../mgmt-host-compose-watch/) — the watchdog this role temporarily disables during rebuild.
