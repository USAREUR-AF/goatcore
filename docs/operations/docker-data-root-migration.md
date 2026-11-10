# Docker data-root migration — eMMC → NVMe ceremony

> **Triggered by F-093.** Production mgmt-host Orins ship with a 1 TB
> NVMe at `/opt/docker-data` per [`os-baseline-and-luks.md`](../design/os-baseline-and-luks.md)
> §"Three-tier storage", but the initial bootstrap never wired
> Docker's `data-root` flag — Docker stayed on the eMMC default.
> This runbook is the operator-fired migration to
> retroactively move data-root onto the NVMe without losing any
> running container state.
>
> **Forward-looking deploys** (clean-slate site bring-up after the
> `docker-engine` role's `daemon.json` + pre-flight changes land) will
> NOT need this — the role configures `data-root` before any container
> ever runs. This runbook covers the **retrofit case** for sites
> deployed before that fix landed.

## When to run this

Any mgmt-host where `sudo docker info | grep "Docker Root Dir"` reports
`/var/lib/docker` AND `/opt/docker-data` is mounted from NVMe. Both
conditions must be true — otherwise either there's nothing to migrate
or there's no NVMe to migrate to (in which case stop and reach out;
that's a different problem class).

## Pre-flight

On the target host:

```
mount | grep /opt/docker-data
df -h /var/lib/docker /opt/docker-data
sudo docker info 2>/dev/null | grep -E "Docker Root Dir|Server Version"
sudo docker ps --format 'table {{.Names}}\t{{.Status}}'
sudo du -sh /var/lib/docker
```

Expected pre-flight state:

- `/opt/docker-data` is on a separate device (NVMe), shows up in `mount` as ext4 / xfs / btrfs.
- `df -h /opt/docker-data` shows free space ≥ size of `/var/lib/docker` + 5 GB headroom.
- `docker info` reports `Docker Root Dir: /var/lib/docker` (the thing we're fixing).
- `docker ps` lists every expected container in `Up` or `healthy`.

If `/opt/docker-data/docker/` already exists from a prior aborted
attempt, **stop**. Either complete the prior migration or rename it
out of the way (`sudo mv /opt/docker-data/docker /opt/docker-data/docker.aborted-$(date +%s)`)
before continuing.

## The ceremony

### Step 1 — Probe current state

```
sudo docker info | grep "Docker Root Dir"
sudo docker ps --format 'table {{.Names}}\t{{.Status}}' | tee /tmp/pre-migration-containers.txt
sudo du -sh /var/lib/docker
df -h /var/lib/docker /opt/docker-data
```

Save the `docker ps` output to `/tmp/pre-migration-containers.txt` — Step 9 cross-checks
that every container that was running before is running after.

### Step 2 — Stop all containers

Don't reach for `docker compose down` here — a mgmt-host can have multiple
compose projects layered (mgmt-host base + Ory overlay + admit-audit
overlay) and the admit-audit overlay required `ADMIT_AUDIT_PASSWORD`
to even parse. A blanket stop sidesteps that:

```
sudo docker stop $(sudo docker ps -q)
```

This sends `SIGTERM` to every running container; postgres-spock + Hydra + Kratos
all flush cleanly within their grace windows. Verify zero containers running:

```
sudo docker ps
```

### Step 3 — Stop dockerd

```
sudo systemctl stop docker.socket docker.service
sudo systemctl is-active docker
```

`is-active` should print `inactive`. If it prints `active`, dockerd is
still up — re-issue the stop and check `journalctl -u docker --since '5 min ago'`
for restart loops.

### Step 4 — rsync /var/lib/docker → /opt/docker-data/docker

```
sudo rsync -aHAX --info=progress2 /var/lib/docker/ /opt/docker-data/docker/
```

Flag rationale:

- `-a` archive (preserves perms, ownership, timestamps, symlinks)
- `-H` hardlinks — Docker overlay2 uses many; without `-H`, layer dedup is lost and the destination balloons
- `-A` ACLs
- `-X` xattrs — containerd layer metadata + selinux labels live in xattrs

This is **non-destructive** — `rsync` copies, doesn't move. The source
stays intact for rollback.

Reference timing: 41 GB took ~25 minutes,
eMMC IO bound at ~25-44 MB/s. Largest single volume was
`mgmt-host_pgdata-a` at 39 GB (mostly `pg_wal/` retained for offline
Spock subscribers — see `max_slot_wal_keep_size` in the
`postgres-streaming-replication` role).

If rsync interrupts, see [Failure modes](#failure-modes--recovery) — restart is safe.

### Step 5 — Verify rsync completion

```
sudo du -sh /var/lib/docker /opt/docker-data/docker
sudo find /var/lib/docker -name '*.tmp' -o -name '*.partial' | wc -l
```

Sizes should match within ~1 MB. The temp/partial count should be `0`.
Mismatch > 1 MB means rsync didn't finish; re-run Step 4 (rsync is
idempotent — it'll only copy the delta).

### Step 6 — Configure daemon.json

Create or merge `/etc/docker/daemon.json`. Match the shape the
`docker-engine` role would render so a future role run is no-op
idempotent:

```
sudo tee /etc/docker/daemon.json >/dev/null <<'EOF'
{
  "data-root": "/opt/docker-data/docker",
  "log-driver": "json-file",
  "log-opts": {
    "max-size": "10m",
    "max-file": "3"
  }
}
EOF
sudo chmod 0644 /etc/docker/daemon.json
```

If `daemon.json` already existed with other keys, merge by hand —
don't blindly overwrite. Validate the result with `jq`:

```
sudo jq . /etc/docker/daemon.json
```

`jq` exits non-zero on malformed JSON. If it errors, fix before Step 7
— a malformed `daemon.json` prevents dockerd from starting.

### Step 7 — Start dockerd

```
sudo systemctl start docker
sudo systemctl is-active docker
sudo docker info | grep "Docker Root Dir"
```

`is-active` should print `active`. `Docker Root Dir` should now read
`/opt/docker-data/docker`. If it still says `/var/lib/docker`, dockerd
ignored the `daemon.json` — check `journalctl -u docker --since '2 min ago'`
for parse errors.

### Step 8 — Restart compose stack

Compose-up is site-specific. For a site with the Ory overlay:

```
cd /opt/mgmt-host
OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES sudo docker compose \
  -f docker-compose.yml \
  -f docker-compose.ory.yml \
  --env-file .env \
  up -d
```

For sites without the Ory overlay, drop `-f docker-compose.ory.yml`.
For sites with admit-audit overlay enabled, also include `-f docker-compose.admit-audit.yml`
and ensure `ADMIT_AUDIT_PASSWORD` is in `.env`.

The canonical compose-up shape is whatever `site-bootstrap.yml`
Phase 5 renders for the site — when in doubt, check the role
`mgmt-host-compose` for the
authoritative argv.

### Step 9 — Verify all containers healthy

```
sudo docker ps --format 'table {{.Names}}\t{{.Status}}'
diff <(sort /tmp/pre-migration-containers.txt) <(sudo docker ps --format 'table {{.Names}}\t{{.Status}}' | sort)
```

Every container from `/tmp/pre-migration-containers.txt` should be back
and Up. Then smoke-test the service surface:

```
# Postgres can read its volume
sudo docker exec mgmt-host-postgres-a-1 psql -U postgres -c 'SELECT 1'

# Mgmt-API ready
curl -sk https://198.18.0.1:443/api/health

# Hydra (if Ory overlay)
curl -sk https://oauth2.<site>.netbird-prod.goat.example/health/ready
```

Spock subscriptions on a primary should resume from where they left off —
verify with `SELECT subscription_name, status FROM spock.sub_show_status();`
in postgres on the primary.

### Step 10 — Old /var/lib/docker cleanup (delayed)

Don't `rm -rf` yet. Rename for safety:

```
sudo mv /var/lib/docker /var/lib/docker.bak
sudo touch /var/lib/docker.bak/.MIGRATED-$(date -u +%Y-%m-%dT%H%M%SZ)
```

Hold for **at least 24 h** of healthy operation; ideally one full week
including a Spock catchup cycle and a pgBackRest backup window.
Then:

```
sudo rm -rf /var/lib/docker.bak
```

## Failure modes + recovery

| Symptom | Cause | Recovery |
|---|---|---|
| `rsync` interrupted (Ctrl-C, ssh drop, OOM) | Operator / network | Re-run Step 4. `rsync -aHAX` resumes from the delta; safe to repeat. |
| `systemctl start docker` fails after Step 7 | `daemon.json` parse error | `journalctl -u docker --since '5 min ago'` shows the parse error line. Fix `/etc/docker/daemon.json`, retry. |
| `docker info` still shows `/var/lib/docker` after Step 7 | dockerd loaded an older config | Confirm `/etc/docker/daemon.json` is the one being read (`sudo systemctl cat docker` — no `--data-root` override on the unit). Restart docker. |
| Container can't find its volume after Step 8 | Named volume path mismatch (rare — overlay2 paths are relative to `data-root`, named volumes are too) | Verify `sudo ls /opt/docker-data/docker/volumes/` shows the expected volume dirs. If empty, Step 4 didn't complete — re-run. |
| Postgres won't start, complains about WAL | Permissions drift on the new mountpoint | `sudo chown -R 999:999 /opt/docker-data/docker/volumes/mgmt-host_pgdata-a/_data` (postgres image UID is 999). |
| Disk fills mid-rsync | Source bigger than `/opt/docker-data` free space | Stop rsync. Free space (e.g. delete unused images: `sudo docker image prune -af` would have helped pre-migration; now only manual cleanup of obviously-old layers in the destination). Re-run Step 4. |
| Need to abort entirely partway | Anything | dockerd is still stopped, source is intact. `sudo rm -rf /opt/docker-data/docker`, leave `/etc/docker/daemon.json` unchanged (or revert to prior content), `sudo systemctl start docker`. System is back to pre-migration state with zero data loss. |

## containerd root migration

> Same class as Docker's data-root, separate dimension —
> containerd has its own root directory for image layers + container
> snapshots. Without this second migration, every `docker pull` adds
> to `/var/lib/containerd` on the eMMC even though the running
> daemon's overlay2 tree is on NVMe.

The forward-looking `docker-engine` role now renders both
`/etc/docker/daemon.json` (data-root) AND `/etc/containerd/config.toml`
(containerd root); on a freshly-built site after the role lands,
neither retrofit is needed. This section covers the **retrofit
case** for sites that already migrated Docker's data-root but
still have containerd on eMMC.

### When to run the containerd retrofit

Any mgmt-host where:

```
sudo awk -F '"' '/^root = /{print $2; exit}' /etc/containerd/config.toml
```

prints nothing (or prints `/var/lib/containerd`) AND
`/var/lib/containerd` exists with > 0 bytes of content.

### The containerd ceremony

```bash
ssh goat@<host-lan-ip>
```

Then on the host:

```bash
# Step 1 — stop watchdog
sudo systemctl stop mgmt-host-compose-watch

# Step 2 — stop docker (which also stops the containers)
sudo systemctl stop docker.socket docker.service

# Step 3 — stop containerd
sudo systemctl stop containerd

# Step 4 — rsync to NVMe
sudo mkdir -p /opt/docker-data/containerd
sudo rsync -aHAX /var/lib/containerd/ /opt/docker-data/containerd/

# Step 5 — verify size match
sudo du -sh /var/lib/containerd /opt/docker-data/containerd

# Step 6 — rename old to .bak
sudo mv /var/lib/containerd /var/lib/containerd.bak-2026-05-09

# Step 7 — write new config.toml (matches the docker-engine role's render)
sudo tee /etc/containerd/config.toml > /dev/null <<'EOF'
# Managed by ops/ansible/roles/docker-engine — do not hand-edit.
# See defaults/main.yml docker_engine_containerd_root for the knob.
version = 2
root = "/opt/docker-data/containerd"
state = "/run/containerd"
EOF

# Step 8 — start containerd
sudo systemctl reset-failed containerd
sudo systemctl start containerd
sleep 2
sudo systemctl is-active containerd

# Step 9 — start docker (which auto-restarts compose containers via restart=always)
sudo systemctl start docker.socket docker.service
sleep 5
sudo docker info | grep -E 'Root Dir|Server Version'

# Step 10 — verify containers came back up
sudo docker ps --format 'table {{.Names}}\t{{.Status}}'

# Step 11 — restart watchdog
sudo systemctl start mgmt-host-compose-watch
sudo systemctl is-active mgmt-host-compose-watch
```

### Why no `disabled_plugins` line

The Docker-shipped `/etc/containerd/config.toml` declares
`disabled_plugins = ["cri"]` at the top level. We deliberately drop
this line in the role's render. Reasoning:

- containerd v2 (≥ 2.0) requires the FULL plugin URI (e.g.
  `"io.containerd.grpc.v1.cri"`); the shorthand `"cri"` is rejected.
- The shipped config gets away with the shorthand because it omits
  `version =` so containerd treats it as v1 syntax.
- We declare `version = 2` (required for any other settings to be
  parsed correctly), so the shorthand stops working.
- Verified failing on containerd v2.2.3 (Ubuntu 22.04 Tegra)
  — first start of the freshly-rendered
  config.toml entered a 5-restart-loop until the line was removed.
- Leaving CRI enabled is harmless for Docker — the gRPC listener
  sits idle; Docker talks to containerd via its native API, not
  CRI.

If a future deploy needs CRI explicitly disabled, switch the line
in the role to the v2 syntax: `disabled_plugins = ["io.containerd.grpc.v1.cri"]`.

### Cleanup

After 24h-stable observation:

```bash
sudo rm -rf /var/lib/containerd.bak-2026-05-09
```

## Cross-refs

- F-093 — Docker data-root default + Spock unbounded WAL
- [`docker-engine` role](../../ops/ansible/roles/docker-engine/) — forward-looking fix; renders `daemon.json` with `data-root` before any container runs
- [`postgres-streaming-replication` role](../../ops/ansible/roles/postgres-streaming-replication/) — `max_slot_wal_keep_size` controls the `pg_wal/` retention that was the dominant disk consumer
- `compute-spec-minimums.md` — per-site storage spec; eMMC is intentionally small
- [`site-bootstrap.yml`](../../ops/ansible/site-bootstrap.yml) — pre-flight + canonical compose-up shape; future versions add the data-root pre-flight gate
- [`os-baseline-and-luks.md`](../design/os-baseline-and-luks.md) §"Three-tier storage" — why the NVMe exists at `/opt/docker-data`
