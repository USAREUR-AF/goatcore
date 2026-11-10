# SNITCH — Operations

Synthetic Network Inspection: Topology, Connectivity, Health.

See `docs/design/snitch.md` for architecture, security model, and data model.

Dashboard: `https://snitch.<site>.<zone>`

## Deploy the Collector (Network Host)

First ensure snitch is in `ops/serve/services.yaml` and DNS is registered:

```
cd ops/serve && python3 generate.py
cd ops/ansible
OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES aws-vault exec legacy --no-session -- \
  ansible-playbook -i inventory/hosts.ini nethost-dns.yml
```

Then deploy the service:

```
OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES aws-vault exec legacy --no-session -- \
  ansible-playbook -i inventory/hosts.ini nethost-snitch.yml
```

Or as part of a full deploy: `site.yml` includes SNITCH in tier 2.

### First Run

On first startup, SNITCH generates an admin and probe API key. These are shown once in the container logs and never stored in plaintext:

```
docker logs snitch 2>&1 | grep -A5 'FIRST RUN'
```

Save both keys immediately:
- **Admin key**: Full dashboard + key management access
- **Probe key**: Distribute to mesh peers for probe authentication

### Managing API Keys

With the admin key:

```
# List keys
curl -s -H "Authorization: Bearer $ADMIN_KEY" https://snitch.<site>.<zone>/api/keys

# Create a new key
curl -s -X POST -H "Authorization: Bearer $ADMIN_KEY" \
  -d '{"role":"viewer","label":"monitoring"}' \
  https://snitch.<site>.<zone>/api/keys

# Delete a key
curl -s -X DELETE -H "Authorization: Bearer $ADMIN_KEY" \
  https://snitch.<site>.<zone>/api/keys?id=3
```

## Install the Probe

### Bundle-onboarded peers (Block 61, recommended for production)

For peers that came up via the wg-cp0 bootstrap + offline-enrollment bundle pair (Block 22 silent-control-plane onboarding), use the `snitch-bundle` ansible role. The role reads the snitch binary from the unpacked bootstrap bundle on the host and the per-device probe API key from the offline-enrollment CBOR bundle. No `curl /install` step; no separate trust path.

```
cd ops/ansible
OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES \
  aws-vault exec usareur-af --no-session -- \
  ansible-playbook -i inventory/wg-cp0-prod.ini \
  --limit "<peer-host>" \
  nethost-snitch-bundle.yml
```

See [`ops/ansible/roles/snitch-bundle/README.md`](../../ops/ansible/roles/snitch-bundle/README.md) for the role's variables. The complementary local-relay-bringup runbook is at `local-relay-bringup.md` Step 6.

### Automatic (zero-touch provisioned peers, legacy)

Peers built with `provisioning/build.sh` automatically install the SNITCH probe on first boot. No action needed.

### Manual (dev workstations + ad-hoc onboarding, legacy) — Linux

For dev workstations or snapshot-debug peers without a bundle, the curl flow is still available. **NOT for production peers** — that path bypasses the offline-CA chain.

From any mesh peer (Block 50E.2 — bearer header only; URL-query auth removed because keys leak via proxy / CDN access logs):

```
curl -fsSL -H "Authorization: Bearer $OPERATOR_KEY" \
  https://snitch.<site>.<zone>/install | \
  sudo OPERATOR_KEY="$OPERATOR_KEY" SNITCH_PROBE_KEY="$PROBE_KEY" bash
```

`OPERATOR_KEY` is forwarded into the install script so the inner `/bin/snitch` download can authenticate against the collector.

Options:

```
curl -fsSL -H "Authorization: Bearer $OPERATOR_KEY" \
  https://snitch.<site>.<zone>/install | \
  sudo OPERATOR_KEY="$OPERATOR_KEY" bash -s -- \
  --collector https://snitch.<site>.<zone> \
  --probe-key $PROBE_KEY \
  --interval 60
```

### Manual — macOS (Block 61H, added 2026-05-06)

Operator MacBook joining fleet-health visibility. Uses launchd; binary at `/usr/local/bin/snitch`; service label `net.goat.snitch-probe`.

```
curl -fsSL -H "Authorization: Bearer $OPERATOR_KEY" \
  "https://snitch.<site>.<zone>/install?os=darwin" | \
  sudo OPERATOR_KEY="$OPERATOR_KEY" SNITCH_PROBE_KEY="$PROBE_KEY" bash
```

If the collector's `/install?os=darwin` route hasn't been wired up in your environment, run the installer directly:

```
sudo OPERATOR_KEY="$OPERATOR_KEY" SNITCH_PROBE_KEY="$PROBE_KEY" \
  bash ops/snitch/install-probe-darwin.sh
```

The macOS installer runs `snitch self-test --timeout 10s` as a pre-flight before registering the LaunchDaemon — a broken probe binary or unreachable collector surfaces immediately rather than after the daemon wedges.

### Manual — Windows (Block 61H, added 2026-05-06)

Operator Windows laptop joining fleet-health visibility. Uses Windows Service via `sc.exe`; service name `SnitchProbe`; binary at `C:\Program Files\Snitch\snitch.exe`.

Open an Administrator PowerShell prompt:

```powershell
$env:OPERATOR_KEY = "<operator key>"
$env:SNITCH_PROBE_KEY = "<probe key>"
Invoke-RestMethod `
  -Headers @{Authorization = "Bearer $env:OPERATOR_KEY"} `
  -Uri "https://snitch.<site>.<zone>/install?os=windows" `
  -OutFile install-probe-windows.ps1
powershell -ExecutionPolicy Bypass -File .\install-probe-windows.ps1 `
  -OperatorKey $env:OPERATOR_KEY `
  -ProbeKey $env:SNITCH_PROBE_KEY
```

Or run the installer directly from a checkout of this repo:

```powershell
powershell -ExecutionPolicy Bypass -File ops\snitch\install-probe-windows.ps1 `
  -OperatorKey "<operator key>" `
  -ProbeKey "<probe key>"
```

The Windows installer runs `snitch.exe self-test --timeout 10s --pretty=false` as a pre-flight before registering the service. Service runs as `LocalSystem` by default (needed for ICMP ping + interface enumeration); override with `-ServiceAccount LocalService` to drop privileges, accepting that ICMP / interface enumeration may degrade.

### Verify (per-platform)

```
# Linux
systemctl status snitch-probe
journalctl -u snitch-probe -f

# Darwin
sudo launchctl print system/net.goat.snitch-probe
tail -f /var/log/snitch-probe.log

# Windows (PowerShell as Administrator)
Get-Service SnitchProbe
Get-EventLog -LogName Application -Source SnitchProbe -Newest 20

# Cross-platform self-test (any OS — emits per-step JSON)
snitch self-test --timeout 30s
# Windows:
& "C:\Program Files\Snitch\snitch.exe" self-test --timeout 30s
```

## Block 61 — probe-load monitoring + on/off control (do-no-harm)

Snitch v2 ships with two on/off control points and a self-instrumentation surface so probe load is visible fleet-wide and stoppable in seconds.

### Control surface

| Knob | Scope | Latency | Use case |
|---|---|---|---|
| `snitch policy set --mode off ...` (admin CLI) | Fleet-wide | next probe cycle (~60s max) | Stop ALL probing across the fleet — operational disruption, mission window, debugging session that got out of hand |
| `snitch policy set --mode focused-research --duration 10m ...` | Fleet-wide | next probe cycle | Time-bounded deep network-health snapshot. Auto-reverts at expiry; abortable with `--mode default --reason abort` |
| `snitch policy set --mode minimal ...` | Fleet-wide | next probe cycle | Reduces probe load to ~10% of default — peer pings + ss-counts only, no relay TLS handshake, no MTU, no race-dialer |
| `SNITCH_PROBE_DISABLED=<reason>` env var | Per-peer | next probe cycle | Silence ONE peer fast without touching the fleet policy |
| `systemctl stop snitch-probe` | Per-peer | immediate | Hard stop on a specific peer (loses heartbeat reporting; use only if env var is unavailable) |

The CLI talks to the collector via `/api/probe-policy/set` (admin-auth). Probes poll the public `/api/probe-policy` once per cycle.

### Policy modes

| Mode | What runs | Roughly how much network load (per peer per minute) |
|---|---|---|
| `off` | Heartbeat report only (peer ID + timestamp + self-metrics) | ~120 bytes/min |
| `minimal` | Peer ICMP pings + `ss` socket counts (no relay TCP probe, no TLS handshake, no MTU, no race-dialer) | ~1 KB/min |
| `default` | Full Block 61 surface with do-no-harm caches (TLS 1h, MTU on iface change, public IP 6h) | ~5-10 KB/min |
| `focused-research` | Full surface, all caches bypassed, every cycle re-probes | ~30-60 KB/min |

Numbers are rough and depend on peer count + relay count + interval.

### Examples

```bash
# Read current policy
snitch policy get

# Schedule a 10-minute focused-research window
snitch policy set --mode focused-research --duration 10m \
  --reason "weekly net-health snapshot"

# Abort an in-progress focused-research window early
snitch policy set --mode default --reason abort

# Stop all probes fleet-wide
snitch policy set --mode off --reason "operational disruption"

# Resume normal operation
snitch policy set --mode default --reason "resume after disruption"

# Silence ONE peer (per-peer kill switch)
ssh peer-host
sudo sed -i 's/^.*SNITCH_PROBE_DISABLED.*$//' /etc/snitch-probe.env
echo 'SNITCH_PROBE_DISABLED=local-fiat' | sudo tee -a /etc/snitch-probe.env
sudo systemctl restart snitch-probe
```

Or via ansible (idempotent, fleet-aware):

```bash
cd ops/ansible
OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES \
  aws-vault exec usareur-af --no-session -- \
  ansible-playbook -i inventory/wg-cp0-prod.ini \
  --limit "<peer-host>" \
  -e snitch_probe_disabled="local-fiat" \
  nethost-snitch-bundle.yml
```

### Self-instrumentation (probe load monitoring)

Every probe report carries a `self_metrics` block:

```json
{
  "self_metrics": {
    "policy_mode": "default",
    "policy_source": "collector",
    "kill_switch_active": false,
    "bytes_out_estimate": 480,
    "bytes_in_estimate": 120,
    "cycles_completed": 174,
    "cycles_skipped": 0,
    "relay_probes_run": 522,
    "tls_handshakes": 9,
    "tcp_connects": 522
  }
}
```

Fleet-wide aggregation in the dashboard surfaces:

- Total bytes-out fleet-wide (probe egress load)
- TLS handshake rate per relay (the load metric the relay actually feels)
- Cycles-skipped trend (catches policy=off propagation + budget overruns)
- Per-peer kill-switch status (catches a peer stuck in local-disable)

### Scheduled focused-research windows

To run a 10-minute focused snapshot every Sunday at 14:00 UTC, add to the operator's crontab on a host with the snitch admin key:

```
0 14 * * 0 SNITCH_ADMIN_KEY=$ADMIN_KEY snitch policy set --mode focused-research --duration 10m --reason "weekly snapshot" --set-by cron
```

The natural `expires_at` revert is the safety net — even if the cron host loses connectivity mid-window, every collector auto-reverts the policy at expiry.

To temporarily disable the schedule (e.g. operational sensitivity), comment out the cron line OR push a longer-duration default policy that the cron can't override:

```bash
snitch policy set --mode default --duration 24h --reason "no focused-research windows for 24h"
```

The cron will still fire its `set --mode focused-research`, and at the next cycle the probes will pick up the focused-research policy, **but** the `set` call's `expires_at` is the deciding factor — operators can write a guard cron that only sets focused-research when the current policy is `default` (CLI exit code reserved for that in a follow-up).

## Block 61 — relay-side health surface

After Block 61 lands, every report carries the following additional fields. Dashboards graph each one as a fleet-wide aggregation:

| Field | Type | Source | What to alert on |
|---|---|---|---|
| `relays[].transport` | `quic` / `ws` / `none` | `ss -tnp` / `ss -unp` | fleet-wide WS-share rises above ~10% (firewall drift); any peer reporting `none` (relay unreachable) |
| `relays[].udp_reachable` / `tcp_reachable` | bool | `nc -z` / `nc -zu` | sudden drop to false on a previously-reachable relay (firewall change, container restart) |
| `relays[].tls_days_until_expiry` | int | in-process `tls.Dial` | days < 14 (warning); days < 7 (critical); days <= 0 (already expired) |
| `mtu.path_mtu` | int (1280-1500) | `ping -M do -s N` | unexpected drop below 1500 on a previously-1500 interface (MPLS underlay change) |
| `race_dialer.quic_success` / `ws_success` / failures | int counters | `journalctl -u netbird` parse | fleet-wide `ws_success >> quic_success` (transport thrashing); any peer with `quic_failure > 0` repeatedly (relay-side bug, e.g. F-042-class missing ALPN) |
| `cycle_budget.over_budget` / `skipped_substeps` | bool / int | probe-loop self-instrumentation | fleet-wide rate of over-budget cycles ≥ 5% (collector overload, daemon pathology) |

### New event types

| Event | Trigger | Severity |
|---|---|---|
| `transport_change` | A peer's `relays[N].transport` differs between consecutive cycles | warning when fleet-wide flap rate spikes; informational per-peer |
| `mtu_change` | A peer's `mtu.path_mtu` changes between consecutive cycles | warning (often signals interface flip or substrate change) |
| `cert_expiring` | Any `tls_days_until_expiry` crosses the 14d threshold | warning; promotes to critical at 7d |

### Daily throughput sample

If you've configured the per-region throughput probe, results land via `snitch throughput-probe` JSONL on stdout. Example operator query: `journalctl -u snitch-throughput.timer | jq 'select(.target_host=="...")'`. Storage layer + dashboard surface is a follow-up.

## API

All endpoints except `/` and `/health` require authentication via `Authorization: Bearer <key>` header.

| Endpoint | Method | Min Role | Description |
|----------|--------|----------|-------------|
| `/` | GET | public | Dashboard |
| `/health` | GET | public | Health check (JSON) |
| `/api/report` | POST | probe | Ingest a probe report |
| `/api/mesh` | GET | viewer | Current mesh state (probes, links, gateways) |
| `/api/history` | GET | viewer | Time-series analytics (`?range=1h\|6h\|24h\|7d\|30d`) |
| `/api/events` | GET | viewer | Event timeline (`?range=24h&limit=100`) |
| `/api/uptime` | GET | viewer | Per-peer uptime (`?range=7d`) |
| `/api/run` | POST | operator | Trigger on-demand probe |
| `/install` | GET | operator | Probe install script |
| `/bin/snitch` | GET | operator | Download snitch binary |
| `/api/keys` | GET/POST/DELETE | admin | API key management |
| `/satlink` | GET | public | Satellite-link PoP tracker page |
| `/api/satlink/latest`, `/trend`, `/pops` | GET | viewer | Satellite-link PoP benchmark results |
| `/api/satlink/probe` | POST | operator | Start one satellite-link PoP benchmark |

The satellite-link routes also answer under their earlier names (listed in `ops/snitch/internal/compat/names.go`), and their JSON carries the range flag under both `in_satlink_range` and the earlier key. The subcommand `snitch satlink-probe` and the serve flag `--satlink-interval` likewise accept their earlier names.

### Collector settings

| Variable | Used for | When unset |
|---|---|---|
| `SNITCH_PEER_DOMAIN_SUFFIXES` | Comma-separated FQDN suffixes stripped from a peer's NetBird name to make its auto-alias | none: no suffix is stripped |
| `SNITCH_GATEWATCH_URL` | gatewatch base URL the dashboard reads bridge status from | none: the bridge panel has no source |
| `SNITCH_PUBLIC_URL` | Collector URL the served `/install` script uses when the installing host sets no `SNITCH_COLLECTOR` | none: the script then needs `SNITCH_COLLECTOR` or `--collector` |
| `SNITCH_SATLINK_NAME` | The word the satellite-link tables, columns, indexes and earlier route, subcommand, flag and JSON names are formed from (`<word>_probes`, `/api/<word>/`, ...). A collector keeps the word its history was written under; set it on its probes too | `satlink` |
| `SNITCH_SATLINK_TARGETS` | Relays the satellite-link benchmark measures, `name=ip=region` comma-separated | none: the benchmark measures no relay |

The collector's `docker-compose.yml` also reads, from the `.env` the deploying play writes: `SNITCH_HOST` (the Traefik host rule) and `SNITCH_DNS` (the container's first resolver), both required, and `SNITCH_GATE_STATUS_FILE` (the host file mounted as `/data/gate-status.json`, default `/opt/goat/gate-status.json`). A probe needs its collector named: `--collector` or `SNITCH_COLLECTOR`; the installers and `snitch probe` refuse without one.

## Data & Retention

- **PostgreSQL** (`snitch-db` container): All measurements, events, identities
- **Raw data retention**: 30 days (reports, measurements)
- **Event retention**: 90 days (satellite-link PoP benchmarks: 90 days)
- **Satellite-link PoP tables**: named from `SNITCH_SATLINK_NAME` (`ops/snitch/internal/compat/names.go`) so a collector keeps reading the history it already holds
- **Cleanup**: Automatic, runs hourly
- **Identity key**: `/opt/goat/snitch/data/identity.key` — auto-generated, do not delete (identities become unreadable without it)
- **PG password**: `/opt/goat/snitch/.env` — generated by Ansible on first deploy, persisted across re-deploys

### Backup

The identity key and PG data are the critical state:

```
# Identity key (without this, encrypted identities are unrecoverable)
ssh <user>@<collector-host> "cat /opt/goat/snitch/data/identity.key" > identity.key.backup

# PostgreSQL dump
ssh <user>@<collector-host> "docker exec snitch-db pg_dump -U snitch snitch" > snitch.sql
```

## Troubleshooting

**Dashboard shows login prompt**: Enter an API key with at least `viewer` role. The key is stored in browser localStorage.

**Probe not reporting**: Check `journalctl -u snitch-probe -f`. Common causes: wrong collector URL, missing/invalid probe key, peer not connected to mesh.

**No API keys in logs**: Keys are only printed on first run. If snitch-db already has keys from a previous deploy, the output is suppressed. Use an admin key to create new keys via `/api/keys`.

**DNS not resolving**: Run `generate.py` then `nethost-dns.yml` to register snitch in mesh-hosts. The snitch playbook does not manage DNS — that's the standard service pipeline's job.

**Container up but 404 via Traefik**: Verify the container is on the `netbird_netbird` Docker network: `docker inspect snitch --format '{{range $k, $v := .NetworkSettings.Networks}}{{$k}} {{end}}'`

## Run Locally (Development)

One-shot probe with JSON output:

```
cd ops/snitch
go run ./cmd/snitch run
```

Start collector locally (requires PostgreSQL):

```
go run ./cmd/snitch serve --addr :8080 --db-url "postgres://snitch:snitch@localhost:5432/snitch?sslmode=disable"
```

## Source

```
ops/snitch/
  cmd/snitch/main.go              Entry point (serve/probe/run)
  internal/
    types/types.go                Shared types
    measure/                      Netbird status, ICMP ping, network fingerprint, gateway discovery
    identity/identity.go          HMAC tokenization + AES-256-GCM encryption
    store/store.go                PostgreSQL schema, CRUD, retention
    store/analytics.go            Time-series bucketing, events, uptime queries
    auth/auth.go                  API key RBAC, middleware
    collector/collector.go        Report ingestion, event detection, mesh state
    compat/                       Names and built-in defaults existing deployments depend on
    server/server.go              HTTP API + auth-protected routes
    server/dashboard.html         Embedded dashboard (Chart.js, login, events)
    server/install.sh             Embedded probe install script
    server/satlink.{go,html}      Satellite-link PoP tracker (routes + page)
  Dockerfile                      Multi-stage Go build (golang:1.25 + debian:bookworm-slim)
  docker-compose.yml              snitch + snitch-db (Postgres 16), Traefik labels
  go.mod / go.sum                 Dependencies (pgx/v5)
```
