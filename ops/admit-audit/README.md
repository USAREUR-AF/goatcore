# admit-audit

Read-only sidecar that watches the local Netbird management Postgres
for new `peers` rows (admits) and emits structured audit log lines +
Prometheus metrics for IOC (indicator-of-compromise) detection.

Implements **C1 (admit audit log)** of
[`docs/design/mgmt-plane-threat-mitigations.md`](../../docs/design/mgmt-plane-threat-mitigations.md) §2.1.

The matching **C3 (IOC alert rules)** are at
[`../compose/mgmt-host/monitoring/rules/admit-ioc.yml`](../compose/mgmt-host/monitoring/rules/admit-ioc.yml).

## Threat model — coincident-infiltration + partition

An adversary who compromises a site AND severs its uplink can admit
themselves silently while isolated, then their peer rows replicate
cluster-wide on reconnect. The attack signature is a cluster of
enrollments during a partition window from one source-user with fresh
WireGuard keys.

This sidecar surfaces that signature by:

1. Emitting one JSON line per detected admit (see Log format below),
   shipped off-host by the usual `docker logs` + log-shipping path.
2. Exposing Prometheus metrics on `:9124`, consumed by `admit-ioc.yml`
   alert rules.

## Log format

One JSON object per line, stdout:

```json
{
  "ts": "2026-04-20T22:15:33.412Z",
  "event": "admit",
  "site": "vega-a",
  "peer_id": "peer_...",
  "account_id": "acc_...",
  "wg_key": "...",
  "ip": "100.127.1.28",
  "name": "orin-04",
  "source_user": "usr_...",
  "meta_hostname": "orin-04"
}
```

On startup one `seed` event is emitted with the count of rows already
present at process launch, so operators can distinguish cold-start
from real admits.

## Metrics (`:9124/metrics`)

| Metric | Type | Labels | Purpose |
|---|---|---|---|
| `admit_audit_admits_total` | counter | site, account_id, source_user | Base counter — rate() feeds `AdmitRateSpike` |
| `admit_audit_peers_total_bywindow` | gauge | site, window | 5m rolling-window admit count — feeds `AdmitBurstDuringPartition` |
| `admit_audit_peers_per_source_user_max` | gauge | site, window | Max per-source-user burst in the window — feeds `AdmitSameSourceUserManyKeys` |
| `admit_audit_distinct_wg_keys_bywindow` | gauge | site, window | Distinct WG key count — feeds `AdmitDistinctKeysSpike` |
| `admit_audit_poll_tick_latency_seconds` | histogram | — | Poll-loop latency (NOT admit-to-detection; see schema gap) |
| `admit_audit_poll_errors_total` | counter | — | DB/query failures |

## Schema gap — no `created_at` on `peers`

Netbird's `peers` table has `peer_status_last_seen` (updated on every
heartbeat) but no row-insert timestamp. We use the sidecar's own
detection time as a best-effort proxy. Worst case: on sidecar restart,
peer rows already present are seeded silently (no admit event, no
detection-latency observation). This is acceptable for C1 because it
is a signal generator, not a tamper-proof ledger. A patched-image
option `netbird-mgmt` already exists (see `docs/design/ha-multi-site.md`);
adding a row-insert timestamp to `peers` in a later patch release
closes this gap cleanly.

## Operational notes

- Poll interval: 30s default (`--poll`). Shorter = faster detection,
  more load on `postgres-a`. 30s is a cheap SELECT on a small table.
- Rolling window: 5m default (`--window`). IOC rules reference the
  gauge label `window="5m0s"` — if you change the default, update
  `admit-ioc.yml`.
- Memory: the in-memory known-IDs set grows indefinitely across
  process lifetime (`map[string]struct{}`). At
  single-digit admits per week, this is negligible. If a bulk-enroll
  pushes this over ~100k peers, add a TTL eviction pass.
- Read-only: the `admit_audit` PG role has `SELECT` on `public.peers`
  only. Cannot modify any table.

## Build

```
cd ops/admit-audit
go build ./...
```

Container build:

```
docker build -t ghcr.io/usareur-af/admit-audit:latest ops/admit-audit/
```

## Local smoke

```
SITE_NAME=local ADMIT_AUDIT_DSN='postgres://admit_audit:pw@127.0.0.1:5432/netbird?sslmode=disable' \
  ./admit-audit --listen=127.0.0.1:9124 --poll=5s
curl 127.0.0.1:9124/metrics | grep admit_audit
```

## Deploy

Via the Ansible role `ops/ansible/roles/admit-audit/` — see the
wrapper playbook `ops/ansible/vega-admit-audit.yml`.

## Detection flow walkthrough — coincident-infiltration signature

1. WAN partition: Spock subscription detaches.
2. `spock_subscription_lag_replicating` drops from 1 to 0 at this site.
3. Adversary enrolls 6 peers via a compromised setup key at the isolated site.
4. On the next 30s poll, the sidecar sees 6 new `peer_id`s, emits 6
   JSON lines to stdout, increments `admit_audit_admits_total` by 6, and
   sets `admit_audit_peers_total_bywindow{window="5m0s"}` from 0 to 6.
5. `AdmitBurstDuringPartition` fires (`> 5` AND partition active) →
   critical alert, lights up the coincident-infiltration signature
   exactly as specified in the design doc.
6. On WAN reconnect, the rows replicate, but by now the alert is
   already visible at every other site's Alertmanager via its local
   Prometheus — the IOC was surfaced BEFORE replication, not after.

## References

- [`docs/design/mgmt-plane-threat-mitigations.md`](../../docs/design/mgmt-plane-threat-mitigations.md) §2.1 (C1–C5)
- `docs/design/federation-security.md`
- `docs/design/ha-multi-site-validation.md` debt item #19
