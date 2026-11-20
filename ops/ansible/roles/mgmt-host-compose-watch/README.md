# mgmt-host-compose-watch role

F-013 mitigation, roadmap 9L. Installs a small bash watcher + systemd
unit on a mgmt-host that revives any `/opt/mgmt-host` docker-compose
service whose container has exited.

## Why

Per F-013,
Docker 29.1.3's `restart: unless-stopped` policy does not trigger
recovery on `docker kill`. v6 nemesis ruled out `restart: always` as a
hypothesis fix. Production must not silently rely on Docker's restart
policy for fault recovery — kratos-a SIGKILL'd in nemesis soak v5 + v6
never came back without operator intervention.

This role provides the explicit recovery path. When kratos-a (or any
mgmt-host service) exits unexpectedly, the watcher detects within
`POLL_INTERVAL` seconds (default 30) and runs
`docker compose up -d <service>` to revive it.

## Pattern

- `/usr/local/sbin/mgmt-host-compose-watch.sh` — the watcher script.
- `/etc/systemd/system/mgmt-host-compose-watch.service` — systemd unit
  with `Restart=always`, so the watcher itself survives crashes.

## Behavior

- Polls every 30 seconds (override via `MGMT_HOST_COMPOSE_WATCH_INTERVAL`
  in unit `Environment=` if needed; default is fine for production).
- Discovers compose files at `/opt/mgmt-host/docker-compose*.yml`.
- For each exited service, logs to journald and runs `docker compose up
  -d <service>` (targeted recovery — does not disturb healthy peers).
- If `docker compose ps` itself fails (transient docker daemon issue),
  logs WARN and retries on next poll.
- SIGTERM-clean for systemd stop.

## Pausing it for a planned stop

A procedure that stops a service of this stack on purpose writes the pause
file first, so the watcher does not start the service again, and removes it
when done:

```sh
echo <reason> | sudo tee /run/mgmt-host-compose-watch.paused   # pause
sudo rm /run/mgmt-host-compose-watch.paused                     # resume
```

While the file exists the watcher revives nothing and logs `paused: <file>
exists (<reason>)` each cycle. It still writes its metrics, and
`compose_watch_paused{site}` is 1. The file is under `/run`, so a reboot
clears a pause someone forgot. `MGMT_HOST_COMPOSE_WATCH_PAUSE_FILE`
names another path (the tests use one).

One procedure uses it today. A joiner whose existing Postgres cluster has
another encoding than its founder's is refused by `site-bootstrap.yml`,
which says to pause with the reason `postgres-reinit`, stop the stack,
remove the `pgdata-a` volume, and re-run. Without the pause, the watcher
started the old `postgres-a` again within its poll interval. Once the re-run's
cluster has the founder's encoding, the play removes a `postgres-reinit`
pause, and no other (`tests/test_site_bootstrap_reinit_pauses_compose_watch.py`).

## Service allowlist (production-relevant once Ory or other migration-style services land)

`MGMT_HOST_COMPOSE_WATCH_SERVICES` env var is an optional space-separated
allowlist. When set, only matching service names are revived; others are
skipped silently. When empty/unset, all exited services are revived
(default behavior, what production does today).

**Why it matters:** services with `restart: "no"` that exit 0 by design
(kratos-migrate, hydra-migrate, db-bootstrap one-shots) trigger a tight
revive loop on every poll cycle without the allowlist — watcher revives
the migration container, it immediately exits 0, next poll revives it
again. Each cycle hammers the database with redundant work. Surfaced
during ory-gate-clearing soak v7: 130 revivals in 11
minutes destabilized kratos-a. Without Ory deployed, production today
has no migration-style services, so the default empty allowlist is
correct. **Once Block 21C-production lands** (compose includes
docker-compose.ory.yml on mgmt-hosts), set the allowlist on the
mgmt-host's systemd unit `Environment=` to the long-running services
only, e.g. `kratos hydra postgres-a postgres-b signal-a signal-b
management-a management-b ...`.

## Validation

After `mgmt-host.yml` apply:

```bash
# Watcher running
systemctl status mgmt-host-compose-watch.service

# Recent logs (should see "starting; compose dir=...")
journalctl -u mgmt-host-compose-watch.service --since '5 min ago'
```

To prove it works, kill a service and watch it come back:

```bash
# Kill kratos (the F-013 reproducer)
docker kill mgmt-host-kratos-1   # or whatever the container name is

# Wait for next poll cycle (≤30s); watch log
journalctl -u mgmt-host-compose-watch.service -f

# Should see: "service 'kratos' exited; running compose up -d kratos"
# then: "service 'kratos' revived: ..."
# then: docker ps shows kratos back up
```

## Retirement

When the Phase 2D Podman+Quadlet migration lands per
docs/design/container-orchestration.md,
Quadlet supervises containers natively via systemd. At that point this
role can be retired — Quadlet's native systemd integration removes the
need for an external watcher.

## Cross-refs

- F-013 — the parent finding.
- docs/project/roadmap.md §9L — the roadmap entry this resolves.
- docs/project/idp-decision-review.md §6 A9 — the gate item this unblocks for Block 21C-production.
- ADR pending: container-supervision-pattern (will be authored when Podman+Quadlet migration formalizes; until then this role is the documented pattern).
