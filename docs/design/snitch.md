# SNITCH — Design & Architecture

Browser credential handling follows the [browser session authentication contract](browser-session-authentication.md), including the HTTPS deployment requirement.


Synthetic Network Inspection: Topology, Connectivity, Health.

SNITCH is the mesh performance monitoring subsystem of goat. It continuously measures latency, jitter, loss, pathing, and gateway hardware across every peer-to-peer link in the WireGuard mesh, stores time-series analytics in PostgreSQL, and presents a live dashboard with historical trend analysis.

SNITCH exists in a different dimension from the services and users that form the network paths it analyzes. The mesh itself is the data plane — SNITCH is the observability plane that watches it.

## Problem Statement

A self-healing mesh network is only as good as the operator's ability to observe its behavior. Without active measurement, degradation is invisible until it becomes an outage. Enterprise NPM systems (ThousandEyes, Expert Analytics, NetAct) solve this at scale, but they require cloud connectivity, vendor lock-in, and infrastructure we cannot depend on in disconnected or contested environments.

SNITCH provides the same core capabilities — synthetic probing, historical analytics, event detection, topology visualization — in a self-contained Go binary that runs entirely within the mesh, requires no external dependencies, and operates in island mode when internet connectivity is lost.

## Architecture

### System Overview

```
┌─────────────────────────────────────────────────────────────────────┐
│                      Network Host (Docker)                          │
│                                                                     │
│  ┌──────────────────────┐      ┌─────────────────────────────────┐  │
│  │   PostgreSQL 16      │      │        snitch serve             │  │
│  │   (snitch-db)        │◄────►│                                 │  │
│  │                      │ pgx  │  ┌─────────┐  ┌──────────────┐ │  │
│  │  peer_identities     │      │  │ Identity │  │   Collector   │ │  │
│  │  reports             │      │  │ Manager  │  │              │ │  │
│  │  measurements        │      │  │ HMAC+AES │  │ Ingest+Event │ │  │
│  │  events              │      │  └─────────┘  └──────────────┘ │  │
│  │  api_keys            │      │  ┌─────────┐  ┌──────────────┐ │  │
│  └──────────────────────┘      │  │  Auth    │  │    Store     │ │  │
│                                │  │  RBAC    │  │ Analytics+   │ │  │
│                                │  │  4 roles │  │ Retention    │ │  │
│                                │  └─────────┘  └──────────────┘ │  │
│                                │  ┌─────────────────────────────┐│  │
│                                │  │    HTTP Server :8080        ││  │
│                                │  │  Dashboard + API + Install  ││  │
│                                │  └─────────────────────────────┘│  │
│                                └─────────────────────────────────┘  │
│                                           ▲                         │
│  ┌────────────────────┐                   │ Traefik TLS             │
│  │ Traefik            │───────────────────┘ mesh-only middleware    │
│  │ (HTTPS :443)       │                                             │
│  └────────────────────┘                                             │
└─────────────────────────────────────────────────────────────────────┘
              ▲                    ▲                    ▲
              │ HTTPS POST         │ HTTPS POST         │ HTTPS POST
              │ Bearer auth        │ Bearer auth        │ Bearer auth
              │ /api/report        │ /api/report        │ /api/report
    ┌─────────┴──────┐  ┌─────────┴──────┐  ┌─────────┴──────┐
    │   Probe A      │  │   Probe B      │  │   Probe C      │
    │   LAN peer     │  │   5G peer      │  │   Starlink     │
    │   systemd 60s  │  │   systemd 60s  │  │   systemd 60s  │
    │                │  │                │  │                │
    │ netbird status │  │ netbird status │  │ netbird status │
    │ ICMP ping      │  │ ICMP ping      │  │ ICMP ping      │
    │ gateway fp     │  │ gateway fp     │  │ gateway fp     │
    └────────────────┘  └────────────────┘  └────────────────┘
```

**Hub-and-spoke.** One collector on the network host, lightweight probes on every mesh peer. Probes are stateless — they measure, report, and sleep. All state, identity management, and analytics live on the collector.

### Components

| Component | Role | Runs on |
|-----------|------|---------|
| `snitch serve` | Collector, API, dashboard, analytics | Network host (Docker) |
| `snitch-db` | PostgreSQL 16 time-series storage | Network host (Docker) |
| `snitch probe` | Measure + report loop | Every mesh peer (systemd) |
| `snitch run` | One-shot measurement, stdout | Developer workstation |

### Data Flow

```
Probe                          Collector                         PostgreSQL
  │                                │                                 │
  │  1. netbird status -j          │                                 │
  │  2. ICMP ping (5 pkt/peer)    │                                 │
  │  3. Network fingerprint        │                                 │
  │                                │                                 │
  │  POST /api/report ────────────►│                                 │
  │  (Bearer token)                │  4. Validate API key (SHA-256)  │
  │                                │  5. Tokenize peer IDs (HMAC)    │
  │                                │  6. Encrypt identities (AES)    │
  │                                │                                 │
  │                                │  INSERT report ────────────────►│
  │                                │  INSERT measurements ──────────►│
  │                                │                                 │
  │                                │  7. Compare with prev state     │
  │                                │  8. Detect events               │
  │                                │  INSERT events ────────────────►│
  │                                │                                 │
  │                                │  9. Update in-memory mesh map   │
  │                                │                                 │
```

### Internal Module Architecture

```
cmd/snitch/main.go
  │
  ├── serve ──► identity.Manager ──► HMAC tokenization + AES-256-GCM
  │             store.Store ────────► PostgreSQL (pgxpool)
  │             auth.Auth ──────────► API key validation + RBAC middleware
  │             collector.Collector ► Ingestion + event detection
  │             server.Server ──────► HTTP routes + embedded dashboard
  │
  ├── probe ──► measure.RunProbe ──► netbird status + ping + fingerprint
  │             postReport ────────► HTTP POST with Bearer auth
  │
  └── run ────► measure.RunProbe ──► same as probe, prints to stdout
```

### Container Architecture

```
┌─────────────────────────────────────────────────────┐
│  Docker Network: netbird_netbird (external)          │
│                                                      │
│  ┌────────────────────┐  ┌────────────────────────┐  │
│  │ snitch-db          │  │ snitch                 │  │
│  │ postgres:16-alpine │  │ snitch:latest          │  │
│  │                    │  │ (multi-stage Go build) │  │
│  │ Port: 5432         │  │ Port: 8080             │  │
│  │ Vol: snitch-db-data│  │ Vol: ./data            │  │
│  │ POSTGRES_DB=snitch │  │ SNITCH_DB_URL=...      │  │
│  │ POSTGRES_USER=     │  │                        │  │
│  │   snitch           │  │ Traefik labels:        │  │
│  │ POSTGRES_PASSWORD= │  │  Host(snitch.nb.db.n)  │  │
│  │   ${SNITCH_DB_PASS}│  │  websecure + TLS       │  │
│  │                    │  │  mesh-only middleware   │  │
│  └────────────────────┘  └────────────────────────┘  │
│              ▲                       │                │
│              └───── postgres://  ────┘                │
└──────────────────────────────────────────────────────┘
```

## Security Architecture

Network measurement data is operationally sensitive. Latency patterns reveal geographic relationships. Gateway fingerprints reveal infrastructure. Peer IP addresses reveal mesh topology. An attacker with this data can map force disposition, identify communication dependencies, and target critical links.

### Threat Model

| Threat | Mitigation |
|--------|-----------|
| Database exfiltration | Identity tokenization (HMAC-SHA256) + encryption (AES-256-GCM). Without the master key, an attacker sees opaque tokens and encrypted blobs — no peer names, IPs, or topology. |
| Unauthorized dashboard access | RBAC with API key authentication. Dashboard data requires at minimum `viewer` role. |
| Probe impersonation | Probe API key required for report submission. Keys are SHA-256 hashed in storage. |
| Network interception | Traefik terminates TLS (Let's Encrypt). Mesh-only middleware blocks non-WireGuard sources. |
| Key compromise | Master key stored on filesystem (0600), never transmitted. API keys shown once at first run, never stored in plaintext. |

### Identity Protection (Zero Trust Data at Rest)

Peer identifiers (names, IPs) never touch PostgreSQL in plaintext. The identity package implements a two-layer scheme:

**Tokenization (HMAC-SHA256).** Every peer identifier is passed through a keyed HMAC before use as a database key. The HMAC key is derived from the master key with domain separation (`snitch-hmac:` prefix). Tokens are deterministic (same input = same token), enabling joins and aggregation without exposing identifiers. Tokens cannot be reversed without the master key.

**Encryption (AES-256-GCM).** A separate `peer_identities` table maps tokens to encrypted names and IPs. The AES key is derived from the same master key with a different domain prefix (`snitch-aes:`). Each encrypted value uses a random nonce (nonce || ciphertext format). The identity table enables the dashboard to display human-readable names for authenticated users while keeping data at rest opaque.

```
┌─────────────────────────────────────────────────────────────┐
│                    Key Derivation                            │
│                                                              │
│  identity.key (32 bytes, auto-generated, 0600)               │
│       │                                                      │
│       ├── SHA-256("snitch-hmac:" || key) ──► HMAC-SHA256     │
│       │                                      tokenization    │
│       │                                      key             │
│       │                                                      │
│       └── SHA-256("snitch-aes:" || key) ───► AES-256-GCM     │
│                                              encryption      │
│                                              key             │
└─────────────────────────────────────────────────────────────┘
```

**What an attacker sees with database access but no master key:**

```
peer_identities:
  token              encrypted_name                encrypted_ip
  a7f3b2c1e9d04f28   \x8a3f...92b1 (48 bytes)    \x1c7e...4d03 (40 bytes)
  d1e5f9a2b3c74086   \x6b2a...f1c8 (52 bytes)    \x9e3d...7a15 (40 bytes)

measurements:
  from_token          to_token            latency_avg   direct   path_type
  a7f3b2c1e9d04f28   d1e5f9a2b3c74086   12.4          true     LAN

  No peer names. No IPs. No topology map. Just opaque tokens and numbers.
```

### RBAC

```
┌──────────────────────────────────────────────────────┐
│                 Role Hierarchy                        │
│                                                      │
│  admin ──────► operator ──────► viewer                │
│    │              │                │                  │
│    │ /api/keys    │ /api/run       │ /api/mesh        │
│    │ (manage)     │ /install       │ /api/history     │
│    │              │ /bin/snitch    │ /api/events      │
│    │              │                │ /api/uptime      │
│    │              │                │                  │
│    │              │                │                  │
│  probe (separate branch ── POST /api/report only)    │
└──────────────────────────────────────────────────────┘
```

| Role | Access | Use case |
|------|--------|----------|
| `admin` | Everything + key management | System operator |
| `operator` | Dashboard + analytics + probe trigger + install script | Field operator |
| `viewer` | Dashboard + analytics (read-only) | Monitoring |
| `probe` | `POST /api/report` only | Mesh peer probes |

API keys use a `snitch_` prefix followed by 64 hex characters. Keys are SHA-256 hashed before storage in PostgreSQL — the plaintext is shown once at creation and never stored. First-run automatically generates one `admin` and one `probe` key, printed to container logs.

Authentication is via `Authorization: Bearer <key>` header only. The legacy `?key=<key>` URL-query parameter was removed (Block 50E.2) because URL queries leak into proxy access logs, CDN caches, and shell history.

## Data Model

### PostgreSQL Schema

```
┌──────────────────────┐         ┌─────────────────────────────────┐
│  peer_identities     │         │  reports                        │
│  ────────────────    │         │  ───────                        │
│  token (PK)          │◄───┐    │  id (PK, BIGSERIAL)             │
│  encrypted_name      │    │    │  token ─────────────────────────┤
│  encrypted_ip        │    │    │  ts                              │
│  first_seen          │    │    │  interface_name, interface_type  │
│  last_seen           │    │    │  gateway_ip_token                │
└──────────────────────┘    │    │  gateway_rtt_ms                  │
                            │    │  public_ip_hash                  │
                            │    │  asn, asn_org, subnet            │
                            │    │  gateway_mac, manufacturer,model │
                            │    └──────────────┬──────────────────┘
                            │                   │ report_id
                            │    ┌──────────────▼──────────────────┐
                            │    │  measurements                    │
                            │    │  ────────────                    │
                            ├────┤  from_token, to_token            │
                            │    │  ts, connected, direct           │
                            │    │  latency_avg, latency_p50,       │
                            │    │  latency_p95, jitter, loss       │
                            │    │  samples, path_type              │
                            │    └─────────────────────────────────┘
                            │
                            │    ┌─────────────────────────────────┐
                            │    │  events                          │
                            │    │  ──────                          │
                            └────┤  token, target_token             │
                                 │  ts, event_type                  │
                                 │  old_value, new_value, details   │
                                 └─────────────────────────────────┘

┌─────────────────────────────────┐
│  api_keys                        │
│  ────────                        │
│  key_hash (UNIQUE, SHA-256)      │
│  role (admin/operator/viewer/    │
│        probe)                    │
│  label, created_at, last_used    │
└─────────────────────────────────┘
```

### Indexing Strategy

BRIN (Block Range INdex) on all `ts` columns. BRIN is optimal for time-series data that arrives in roughly chronological order — tiny index size, fast range scans. Composite B-tree indexes on `(token, ts)` and `(from_token, to_token, ts)` for per-peer and per-link queries.

### Retention

```
                   ┌────────────────────────────────────────────┐
  Raw data         │█████████████████████████████████            │ 30 days
  (reports,        │                                             │
  measurements)    └────────────────────────────────────────────┘

  Events           │█████████████████████████████████████████████████████████████│ 90 days
  (peer_online,    │                                                             │
  link_lost, etc.) └─────────────────────────────────────────────────────────────┘

  Cleanup: hourly goroutine  ──► DELETE WHERE ts < now() - retention
```

## Analytics

### Time-Series Bucketing

The dashboard supports five time ranges with server-side aggregation. Bucket size increases with range to keep response sizes constant (~60-360 data points per response):

```
  Range     Bucket    Points     Resolution
  ─────     ──────    ──────     ──────────
  1 hour    60s       ~60        Per-minute
  6 hours   120s      ~180       Per-2-minutes
  24 hours  300s      ~288       Per-5-minutes
  7 days    1800s     ~336       Per-30-minutes
  30 days   7200s     ~360       Per-2-hours
```

Each bucket aggregates: average latency, P95 latency, jitter, loss percentage, direct path percentage, and sample count.

### Queries

- **Mesh trend**: Mesh-wide averages over time (all connected links)
- **Link trend**: Per-link time-series for a specific peer pair
- **Events**: State change timeline (peer online/offline, path changes, link new/lost)
- **Uptime**: Per-peer uptime percentage (minutes with reports / total minutes in range)

### Event Detection

The collector maintains in-memory previous state per peer. On each report, it compares current link state against the previous report to detect transitions:

```
Previous state          Current report          Event generated
──────────────          ──────────────          ───────────────
(no record)             Probe reports      ──►  peer_online
Link A-B (direct)       Link A-B absent    ──►  link_lost
(no link A-C)           Link A-C (direct)  ──►  link_new
Link A-D (direct)       Link A-D (relayed) ──►  path_change (direct → relayed)
Link A-E (relayed)      Link A-E (direct)  ──►  path_change (relayed → direct)
```

## Probe Measurement

Each 60-second cycle:

```
┌────────────────────────────────────────────────────────────────┐
│  Probe Cycle (60s)                                             │
│                                                                │
│  1. netbird status -j ───► Discover peers, IPs, direct/relay  │
│                                                                │
│  2. For each connected peer:                                   │
│     ping -c 5 <peer_ip> ───► avg, P50, P95, jitter, loss     │
│                                                                │
│  3. Network fingerprint:                                       │
│     ├── Interface detection (type, name)                       │
│     ├── ARP cache ──► gateway MAC ──► OUI manufacturer         │
│     ├── HTTP probe ──► gateway web UI ──► model inference      │
│     ├── mDNS query ──► _http._tcp advertisements               │
│     ├── Subnet detection (CIDR from interface)                 │
│     └── Public IP + ASN lookup                                 │
│                                                                │
│  4. POST /api/report ───► Collector (Bearer auth)              │
└────────────────────────────────────────────────────────────────┘
```

### Path Type Classification

| Type | Latency | Identification |
|------|---------|----------------|
| LAN | <2ms | Same public IP, direct, <3ms RTT |
| 5G | 5-20ms | Same public IP, direct, >3ms RTT |
| Internet | 20-80ms | Different public IP, direct, non-Starlink ASN |
| Starlink | 40-120ms | Either peer on AS14593 (SpaceX) |
| Relayed | 100-300ms | Netbird relay fallback (not P2P) |

### Gateway Characterization

```
  ARP cache ──► MAC address ──► OUI prefix ──► Manufacturer
                                                (Peplink, GL.iNet,
                                                 Teltonika, Cisco,
                                                 SpaceX, etc.)

  HTTP probe ──► Gateway web UI ──► <title> + Server header ──► Model
  (port 80/443    (2s timeout)       "GL-MT3000", "RUTX11",
   on gateway IP)                    "IBR900", etc.

  mDNS ────────► _http._tcp query ──► Service advertisements
                  (2s timeout)

  Interface ───► Local CIDR ──► Subnet
  Public IP ───► External lookup ──► ASN + ASN Org
```

## Block 61 — Relay-side health probing (added 2026-05-04)

Closing the F-042 visibility gap. Added 2026-05-04 in the same change as the snitch v2 onboarding integration.

### Per-relay transport assertion

Each cycle, snitch reads the local kernel's socket table (`ss -tnp` for TCP, `ss -unp` for UDP) and counts ESTABLISHED sockets to each configured relay's `:33073`. UDP-presence is dispositive: `udp ≥ 1` → `quic`, `tcp ≥ 1 ∧ udp = 0` → `ws`, neither → `none`. Source: [`internal/measure/relay.go::ClassifyTransport`](../../ops/snitch/internal/measure/relay.go).

This is the **same shape as the goat-relay tier-4b bench env's Phase 6 transport-assert smoke** (`ops/wave-33/tier-4b/inchworm.sh cmd_ramp`) — by design, so production + bench share a mental model. Block 62 lifts this primitive into a shared package.

### Per-relay TCP reachability

`nc -z` opens a TCP connect against the relay's `:33073` and immediately closes (SYN + RST). Cheap, surfaces TCP-listener-down even when the daemon is on QUIC. Run every cycle.

**Active UDP reachability probing was deliberately removed** as part of the do-no-harm review (2026-05-04). The previous `nc -zu` behavior sent unsolicited UDP packets to `:33073` every cycle from every peer; goat-relay's QUIC listener parsed each packet, failed to recognize the QUIC initial-packet shape, and dropped — wasted CPU on the relay at fleet scale. The signal we want — "is UDP reachable?" — is already in `ss -unp count`: if the netbird daemon's race-dialer established a UDP socket, UDP IS reachable; if `udp_count=0 AND tcp_count=1`, UDP is known-unreachable. `RelayHealth.UDPReachable` is now derived from socket classification (`Transport == "quic"`), not from an active probe.

### Relay TLS cert days-until-expiry (cached)

In-process `tls.Dial` with `InsecureSkipVerify: true` and SNI ServerName set when host is a DNS name (metadata-only — we're not authenticating the server, just collecting cert chain). Returns days-until-`NotAfter` rounded up. Dashboards alert at 7d remaining. Source: [`certProbeOpenSSL`](../../ops/snitch/internal/measure/relay.go).

**Cached for 1h** by default ([`RelayProbeConfig.CertProbeTTL`](../../ops/snitch/internal/measure/relay.go)) — cert `NotAfter` changes by 1 day per day, so 1h cadence is 24× over-sampled already. The naive 60s cadence at stretch fleet scale (6000 peers × 3 relays) would have produced ~100 full TLS handshakes/sec/relay (~15% relay CPU just for cert metadata). At 1h cadence it's ~1.7/sec/relay (~0.25% relay CPU). The cache is process-shared in `cmd/snitch` so all relay probes in one peer process share it.

### Path-MTU probe (interface-change-cached)

`ping -M do -s 1472` (full Ethernet) + `ping -M do -s 1252` (WG floor) at probe-process boot, then **only re-runs on interface change**. Caches in process via [`MTUProberCachingByInterface`](../../ops/snitch/internal/measure/mtu.go); reports the cached result on every cycle so analytics see the value over the retention window. MTU correlates with interface flips, not the 60s loop — per-cycle DF=on ping to the gateway adds cost on cellular / satellite uplinks for no signal.

### Race-dialer outcome counters

Parses `journalctl -u netbird --since "<window> ago"` for `successfully dialed via: <transport>` and `failed to dial via <transport>:` lines. Returns four counters: `quic_success`, `ws_success`, `quic_failure`, `ws_failure` over the window. Catches transport thrashing — fleet-wide reconnect storms where the daemon is flipping QUIC↔WS frequently per F-041.

### Daily throughput sample (opt-in)

Separate `snitch throughput-probe` sub-command, called from a systemd timer once per day per region. 10s × 1 flow + 10s × 4 flow iperf3 ramp against a configured sentinel target. One JSONL line per probe run, suitable for log shipping. Per-region sentinel pair selected at deployment time. Wires into the snitch DB in a follow-up; instrumentation discipline (one JSON object per run) is the load-bearing part.

### Probe-loop hygiene

- **Jitter:** the 60s loop sleeps uniform[55s, 65s] — random per cycle. Avoids fleet-synchronized POST bursts on the collector.
- **Soft per-cycle budget:** 5s default. Sub-steps that would START past the budget are skipped + counted in the `cycle_budget` field. Sub-steps in flight are NOT cancelled (corruption avoidance).
- **Cache-on-change network detection:** gateway HTTP-banner + mDNS re-run only on MAC change; public IP + ASN cache 6h with subnet-flip invalidation. Net per-cycle runtime drops ~80% on steady-state. See [`internal/measure/cache.go`](../../ops/snitch/internal/measure/cache.go).

### Onboarding integration (Block 22 wg-cp0)

For wg-cp0-onboarded peers, the probe binary arrives via the bootstrap bundle (added to [`bootstrap-bundle.md`](bootstrap-bundle.md) §2 in the same change). The per-device probe API key arrives via the offline-enrollment CBOR bundle as the new `snitch_probe_key` field per [offline-enrollment.md §11.6a](offline-enrollment.md#116a-snitch-probe-credential-extension-block-61-added-2026-05-04). The new `ops/ansible/roles/snitch-bundle/` role wires both. The legacy `curl /install` path from `install-probe.sh` stays for ad-hoc dev-workstation onboarding (no bundle in flight); it is NOT for production peers.

See the operations-side runbook in `local-relay-bringup.md` Step 6.

### Probe-key issuance + collector registration (bundle-create wiring)

The per-device probe key is generated INSIDE the bundle issuance ceremony — bundle-create is the source of truth for the random 32 bytes. Operator passes `--snitch=true` to embed the key in the CBOR bundle. With the additional `--snitch-collector <url>` + `--snitch-admin-key <key>` (or `$SNITCH_ADMIN_KEY`), bundle-create POSTs the wire-form bearer (`snitch_<hex>`) to `<collector>/api/keys` BEFORE writing the bundle to disk so a registration failure leaves no orphan file.

The collector path is `auth.CreateKeyWithPlaintext` — idempotent on label, so re-issuing a bundle for the same `device-id` replaces the prior key in one transaction (DELETE existing rows for that label, INSERT the new hash). This matches the operator workflow where key rotation and bundle rotation are the same act.

The wire format split — bundle stores 32 raw bytes (omitempty CBOR field), the device-side env template at [`snitch-probe.env.j2`](../../ops/ansible/roles/snitch-bundle/templates/snitch-probe.env.j2) prepends the literal `snitch_` prefix to the hex output of `bundle-extract --hex` to form the bearer the probe presents. The collector hashes that prefixed string and matches against the row inserted by `CreateKeyWithPlaintext`. One source of prefixing — the env template — keeps the rendering layer thin and the bundle agnostic to the on-the-wire form.

See the operator runbook §3a in `wg-cp0-bundle-ceremony.md`.

### Multi-platform probe builds (Block 61H, landed 2026-05-06)

The snitch probe ships pre-built for **all six common Go targets**:
`linux/amd64`, `linux/arm64`, `darwin/amd64`, `darwin/arm64`,
`windows/amd64`, `windows/arm64`. CGO_ENABLED=0 across the board — pure
Go, no native deps. The collector image bundles all six binaries under
[`/app/bin/<goos>-<goarch>/`](../../ops/snitch/Dockerfile) and the
[`/bin/snitch?os=&arch=`](../../ops/snitch/internal/server/server.go)
endpoint serves the right one (with `.exe` suffix on Windows).

**Per-OS probe-step matrix.** Some probe steps don't have an analog on
every platform; snitch reports them as structural skips rather than
failures.

| Probe step | Linux | Darwin | Windows |
|---|---|---|---|
| ICMP ping (`measure/ping.go`) | `ping -c -W <s>` | `ping -c -W <ms>` | `ping -n -w <ms>` |
| Default-route iface | `ip route show default` | `route -n get default` | PowerShell `Get-NetRoute` + `Format-List InterfaceAlias` |
| Default-route gateway IP | `ip route show default` | `route -n get default` (gateway: line) | PowerShell `Get-NetRoute` + `Format-List NextHop` |
| Gateway ARP MAC | `ip neigh show <ip>` | `arp -n <ip>` | `arp -a <ip>` (dash separators normalized) |
| Gateway HTTP banner | pure Go (cross-platform) | pure Go | pure Go |
| mDNS service discovery | `avahi-browse` | `dns-sd -B` | **skipped** (no built-in mDNS browser) |
| WiFi SSID | `iwgetid -r <iface>` | `networksetup -getairportnetwork` | `netsh wlan show interfaces` |
| Path-MTU DF=on (`measure/mtu.go`) | `ping -M do -s <N>` | `ping -D -s <N>` | `ping -f -l <N>` |
| Relay socket count | `ss -Hnt`/`-Hnu` | `lsof -nP -i <PROTO>@host:port` | `netstat -an -p <PROTO>` |
| Relay TCP reachability | `nc -z` | Go `net.DialTimeout` | Go `net.DialTimeout` |
| Relay TLS cert probe | pure Go (`tls.Dial`, all platforms) | pure Go | pure Go |
| Race-dialer log read | `journalctl -u netbird` | **skipped** (returns zero, false) | **skipped** (returns zero, false) |
| Daily throughput sample | `iperf3 -c -J` | `iperf3` if installed | `iperf3.exe` if installed |

The race-dialer log read is structurally Linux-only because netbird on
Darwin runs under launchd (logs in `log show --predicate ...`) and on
Windows writes to the Windows Event Log; both are doable but the parsing
shape differs enough that the operational value (catching transport
thrash on operator laptops, which are typically not the high-uptime
peers where thrash matters most) doesn't justify the work yet. The
broader gap — that netbird daemon emits these state transitions only as
free-text logrus lines through three different per-OS sinks, with no
uniform structured-event surface — is captured at
F-081.
The per-OS log readers (macOS `log show`, Windows `Get-WinEvent`) are
queued as Block 61K in `docs/project/implementation-plan.md`;
patching the netbird fork to emit structured events directly is the
strictly-superior alternative captured as a watchpoint at the bottom of
the same block.

**Build-tag-split files** (per-OS production runners) keep the inner
parsers as pure Go in [`relay_runner_parsers.go`](../../ops/snitch/internal/measure/relay_runner_parsers.go)
+ [`fingerprint_parsers.go`](../../ops/snitch/internal/measure/fingerprint_parsers.go).
This lets a Mac developer verify the Windows netstat parser in a unit
test without spinning up a VM. 78 unit tests cover the cross-platform
text shapes (canned `ss` / `lsof` / `netstat` / PowerShell / netsh /
arp / ping outputs) — all run on every host.

**Per-OS installer matrix.**

| Platform | Installer | Service manager | Install path |
|---|---|---|---|
| Linux | [`install-probe.sh`](../../ops/snitch/install-probe.sh) | systemd | `/usr/local/bin/snitch` + `/etc/systemd/system/snitch-probe.service` |
| Darwin | [`install-probe-darwin.sh`](../../ops/snitch/install-probe-darwin.sh) | launchd | `/usr/local/bin/snitch` + `/Library/LaunchDaemons/net.goat.snitch-probe.plist` |
| Windows | [`install-probe-windows.ps1`](../../ops/snitch/install-probe-windows.ps1) | sc.exe / Windows Service | `C:\Program Files\Snitch\snitch.exe` + service `SnitchProbe` |

Each installer runs a pre-flight `snitch self-test --timeout 10s`
before registering the service so a broken probe binary or unreachable
collector surfaces at install time, not after the daemon is wedged.

**Bundle role** [`ops/ansible/roles/snitch-bundle/`](../../ops/ansible/roles/snitch-bundle/)
dispatches by `ansible_facts['system']` to either `Linux.yml` (systemd)
or `Darwin.yml` (launchd). Windows is **deliberately not in this role** —
the WinRM connection layer + `win_*` module surface differs enough from
the POSIX modules to warrant a sibling role; documented in the role
[README §"OS support"](../../ops/ansible/roles/snitch-bundle/README.md#os-support).

**Self-test sub-command.** `snitch self-test` runs every probe step
once on the local host and emits per-step JSON (`status: ok | skip |
fail`) plus an `overall_ok` rollup. Required-on-this-platform steps
that fail trigger non-zero exit; structurally-skipped steps (e.g.
journalctl on Darwin) pass. Useful for verifying a fresh install
without waiting a full probe cycle. Source: [`cmd/snitch/selftest.go`](../../ops/snitch/cmd/snitch/selftest.go).

**CI cross-OS gate.** [`.github/workflows/snitch.yml`](../../.github/workflows/snitch.yml)
runs the same six-target build matrix on every PR + push to main as
a `build-matrix` job (parallel to `unit` + `integration`). Each cell:
`go vet`, `go build` with `-trimpath -ldflags="-s -w"`, `go test -c`
on the measure package (catches build-tag mismatches that runtime
builds would miss), uploads the binary + `.sha256` sidecar as a
workflow artifact, annotates the run summary with the binary's
SHA-256. A `checksums` aggregator job downloads all six artifacts
and emits a sorted `SHA256SUMS` manifest (also a workflow artifact).
On a pushed `snitch-v<semver>` tag the `release` job (gated by
`if: startsWith(github.ref, 'refs/tags/snitch-v')`, scoped
`contents: write` permission) attaches the same six binaries plus the
manifest to a GitHub Release. The container image at
[`ops/snitch/Dockerfile`](../../ops/snitch/Dockerfile) remains the
canonical fleet-pull source — workflow artifacts are the pre-image
regression catch + per-commit binary trail; releases are the durable
versioned distribution surface.

**Mobile (iOS / iPadOS / Android) is out of scope.** snitch is a root
daemon needing ICMP / interface enumeration / outbound HTTPS; mobile
sandboxes don't grant the first two and don't run background daemons
without operating-system-specific lifecycle work that doesn't fit a
fleet-health probe shape. If foreground-app periodic-measurement on
mobile becomes a real ask, treat it as a separate component and a
separate ADR — not a snitch port.

## Testing Architecture

SNITCH has a multi-layered testing strategy spanning unit tests, integration probes, and continuous production verification. The goal is fast feedback during development and ongoing confidence in production.

### Test Pyramid

```
                    ┌───────────┐
                    │ Production│  Continuous: SNITCH probes (60s),
                    │ Monitoring│  smoketest dashboard, health checks
                    └─────┬─────┘
                          │
                 ┌────────┴────────┐
                 │   Integration   │  Smoketest service probes,
                 │   Tests         │  Ansible nethost-verify.yml,
                 │                 │  smoke-test.sh
                 └────────┬────────┘
                          │
            ┌─────────────┴─────────────┐
            │        Unit Tests         │  Go: measure/*_test.go
            │                           │  BATS: provisioning, deploy
            │                           │  ShellCheck + yamllint
            └─────────────┬─────────────┘
                          │
       ┌──────────────────┴──────────────────┐
       │           CI Pipeline               │  GitHub Actions: push + PR
       │  ShellCheck → yamllint → BATS →     │  gate before merge
       │  secret scan                        │
       └─────────────────────────────────────┘
```

### Unit Tests (Go)

```
ops/snitch/internal/measure/
  netbird_test.go       Netbird status JSON parsing
                        - Peer extraction, connectivity counts
                        - Relay detection, ICE candidate types
                        - Fixture: testdata/netbird-status.json (27 peers)

  probe_test.go         Full probe cycle with stubs
                        - Stubbed netbird status, ping, network functions
                        - Validates latency measurements, peer connectivity
                        - Direct vs relayed path distinction
```

Run locally:

```bash
cd ops/snitch && go test ./...
```

### CI Pipeline (GitHub Actions)

```
Push to main / PR
       │
       ▼
┌──────────────────────────┐
│  1. ShellCheck           │  Lint all shell scripts
│  2. yamllint             │  Validate YAML configs
│  3. BATS unit tests      │  Provisioning + deployment tests
│  4. Secret scan          │  Regex scan for hardcoded credentials
└──────────────────────────┘
       │
       ▼
  Pass/fail gate
```

The CI pipeline runs on every push to `main` and every pull request. Defined in `.github/workflows/ci.yml`. Runs `make check` which chains `make lint` and `make test`.

**Note:** Go tests for SNITCH are not yet in the CI pipeline. They run locally via `go test ./...` but should be added to GitHub Actions as a separate job when Go is installed in CI.

### Production Verification

Three layers of production testing run continuously after deployment:

#### SNITCH self-monitoring (continuous, every 60s per probe)

SNITCH is both the service and its own integration test. Every probe cycle validates:

- Netbird mesh connectivity (can we reach peers?)
- ICMP reachability (are links alive?)
- Path quality (latency, jitter, loss within expected ranges?)
- Event detection catches regressions (link_lost, path_change)

#### Smoketest dashboard (continuous service probes)

The smoketest service (`ops/smoketest/`) probes all hosted services including SNITCH:

- HTTP health check: `GET /health` returns 200
- Configured in `ops/smoketest/services.yaml`
- Results displayed on the smoketest dashboard

#### Ansible post-deploy verification

```bash
ansible-playbook -i inventory/hosts.ini nethost-verify.yml
```

Checks: Docker containers running, Traefik routing, DNS resolution, health endpoints.

#### Manual smoke test harness

`ops/smoke-test.sh` — comprehensive mesh diagnostic that checks DNS, relays, STUN, management API, network host containers, and mesh peer connectivity. Outputs JSON for historical tracking.

### Development Workflow

```
  Edit code
     │
     ▼
  go test ./...              ◄── Fast unit tests (seconds)
     │
     ▼
  go build ./cmd/snitch      ◄── Compile check
     │
     ▼
  go vet ./...               ◄── Static analysis
     │
     ▼
  git push                   ◄── Triggers CI (BATS, lint, secret scan)
     │
     ▼
  ansible-playbook           ◄── Deploy to network host
  nethost-snitch.yml
     │
     ▼
  Smoketest + SNITCH         ◄── Continuous production verification
  probes confirm health
```

## Technology Stack

| Layer | Technology | Rationale |
|-------|-----------|-----------|
| Language | Go 1.25 | Single static binary, cross-compiles for probe deployment on any arch |
| Database | PostgreSQL 16 (Alpine) | Time-series queries, BRIN indexes, connection pooling, consistent with TAK and efforts services |
| DB driver | pgx/v5 + pgxpool | Pure Go, no CGO, built-in connection pooling |
| Crypto | stdlib crypto/aes, crypto/hmac, crypto/sha256 | No external crypto dependencies |
| HTTP | stdlib net/http | No framework overhead, embedded HTML/JS dashboard |
| Charts | Chart.js (CDN) | Graceful degradation — charts hidden when offline, data tables still work |
| Container | Docker multi-stage build | Go build stage + Debian slim runtime (needs ping, arp, ip, avahi-browse) |
| Routing | Traefik labels + mesh-only middleware | Consistent with all goat services |
| DNS | Mesh dnsmasq (`snitch.netbird.legacy.example`) | Standard service pipeline via services.yaml + generate.py |
| Testing | Go testing + BATS + smoketest | Unit (Go), CI (GitHub Actions), production (smoketest + self-monitoring) |

## Deployment Model

SNITCH follows the standard goat service pipeline:

```
  1. services.yaml         ◄── Single source of truth (type: container)
         │
         ▼
  2. python3 generate.py   ◄── Updates mesh-hosts, services.env, Caddyfile
         │
         ├──► ops/dns/mesh-hosts           (DNS entry)
         ├──► ops/status/services.env      (health check target)
         └──► ops/serve/docker-compose.yml (Caddy routing — N/A for containers)
         │
         ▼
  3. nethost-dns.yml          ◄── Deploys dnsmasq config to the network host
         │
         ▼
  4. nethost-snitch.yml       ◄── Service-specific deployment
         │
         ├──► Sync Go source (rsync)
         ├──► Generate/preserve PG password (.env)
         ├──► Build Docker image (multi-stage)
         └──► Start snitch + snitch-db containers
```

The playbook handles only snitch-specific concerns. DNS, TLS, and Traefik routing come from the standard pipeline.

## Comparison with Enterprise NPM

SNITCH covers the core capabilities that matter for a tactical mesh:

| Capability | Enterprise NPM | SNITCH |
|-----------|---------------|--------|
| Synthetic probing | Yes | Yes — ICMP ping, 5 pkt, avg/P50/P95/jitter/loss |
| Historical analytics | Yes (months) | Yes — 30d raw, 90d events, bucketed time-series |
| Event detection | Yes | Yes — online/offline, path change, link new/lost |
| Topology mapping | Yes | Yes — live mesh state, gateway grouping |
| Identity protection | Varies | HMAC tokenization + AES-256-GCM encryption |
| Cloud dependency | Required | None — runs entirely within the mesh |
| Island mode | No | Yes — operates without internet |
| Gateway fingerprinting | Limited | Yes — MAC OUI, HTTP banner, mDNS, model inference |

Not implemented (out of scope for current mesh size):
- Flow-level traffic analysis (NetFlow/sFlow)
- Application-layer probing (HTTP, DNS synthetic tests)
- Multi-collector federation
- Alerting/notification integrations

## Source Layout

```
ops/snitch/
  cmd/snitch/main.go                    Entry point — serve/probe/run modes
  internal/
    types/types.go                      Shared types (ProbeReport, MeshState, etc.)
    measure/
      netbird.go                        Netbird status parser
      netbird_test.go                   Parser tests (27-peer fixture)
      ping.go                           ICMP ping with statistics
      fingerprint.go                    Network environment detection
      gateway.go                        Gateway hardware discovery
      probe.go                          Probe runner (composable via function injection)
      probe_test.go                     Probe tests with stubs
      testdata/netbird-status.json      Test fixture
    identity/identity.go                HMAC tokenization + AES-256-GCM encryption
    store/
      store.go                          PostgreSQL schema, migrations, CRUD, retention
      analytics.go                      Time-series bucketing, events, uptime queries
    auth/auth.go                        API key RBAC, middleware
    collector/collector.go              Report ingestion, event detection, mesh state
    server/
      server.go                         HTTP API + auth-protected routes
      dashboard.html                    Embedded dashboard (Chart.js, login, events)
      install.sh                        Embedded probe install script
  Dockerfile                            Multi-stage Go build (golang:1.25 + debian:bookworm-slim)
  docker-compose.yml                    snitch + snitch-db containers, Traefik labels (DEV-TREE — NETHOST collector)
  docker-compose.prod-orin.yml          snitch + snitch-db containers, plain :8080 published (PROD-TREE — Orin-C collector, no Traefik)
  go.mod / go.sum                       Dependencies (pgx/v5)
```

