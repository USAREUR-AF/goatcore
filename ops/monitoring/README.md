# ops/monitoring

**Role: local-dev sandbox + alert-rule R&D.** Self-contained Prometheus +
Alertmanager compose with a generic ruleset for testing alert expressions
without touching a deployed mgmt-host.

> **Production source of truth lives elsewhere.** Per-site Prometheus rules
> that are actually scraped + evaluated live under
> [`ops/compose/mgmt-host/monitoring/rules/`](../compose/mgmt-host/monitoring/rules/)
> and are deployed via [`ops/ansible/roles/observability/`](../ansible/roles/observability/).
> The two trees are deliberately decoupled (Block 50E.9).
> The earlier "skeleton-source-of-truth + per-site copy-and-adapt"
> contract is retired — it drifted in practice and the per-site rules
> evolved richer (admit-ioc) than this skeleton.

**Use this directory when:**
- Drafting a new alert rule and testing it against a synthetic Prometheus
- Verifying alert expressions resolve cleanly before promoting to a per-site rule file
- Demoing the alert layer on a laptop without provisioning a mgmt-host

**Do NOT use this directory for:**
- Production rule edits — those go directly into `ops/compose/mgmt-host/monitoring/rules/`
- Source-of-truth claims about what alerts are currently firing in a deployed site

Scrapes the signal-server `/metrics` endpoint, Postgres (via
`postgres-exporter`), Spock-specific metrics, and node-exporter on
each mgmt-host. Alert rules implement the §7.3 criteria from
`docs/design/ha-multi-site.md`.

The sandbox's Prometheus sets `site` as an external label (`prometheus/prometheus.yml`), and a rule template's `$labels` does not hold external labels. So each alert summary names the site from the series' own `site` label when it has one, else from `$externalLabels.site`, as the mgmt-host rules do. `prometheus/tests/alert-summaries-name-the-site.test.yml` renders one Spock and one signal-server summary both ways (`promtool test rules`, run by hand; no CI job loads these rules).

## Layout

```
ops/monitoring/
├── prometheus/
│   ├── prometheus.yml      scrape configs (see "Scrape targets" below)
│   └── rules/
│       ├── signal-server.yml    federation / relay / peer count alerts
│       └── spock.yml            replication lag / WAL / conflict alerts
├── alertmanager/
│   └── alertmanager.yml    routing skeleton (TODO: fill in receiver)
├── docker-compose.yml      optional — run Prometheus + Alertmanager locally
└── README.md
```

## Scrape targets

| Target | Port | Produced by |
|---|---|---|
| signal-server `/metrics` | 9090 per instance | `ghcr.io/usareur-af/signal-server` (see `signal-server/internal/server/metrics.go`) |
| postgres-exporter | 9187 | `quay.io/prometheuscommunity/postgres-exporter`, sidecar to each mgmt-host's `postgres-a` |
| node-exporter | 9100 | Standard; install per host via Ansible |
| Prometheus itself | 9090 | `/metrics` on the Prometheus instance |

Postgres-exporter needs to query `pg_replication_slots`,
`pg_stat_replication`, `spock.subscription`, and `spock.resolutions` (conflicts) and `spock.exception_log` (skipped changes) —
the queries live inline in `prometheus.yml` as an `auth.queries` extension
(see `postgres-exporter` docs). **Requires the `replicator` or `postgres`
role to have `SELECT` on those system views.**

## Alert rules (§7.3 mapping)

| §7.3 risk | Rule | Threshold |
|---|---|---|
| Replication lag | `spock_subscription_lag_seconds` | `> 60s for 2m` |
| WAL retention ceiling approached | `pg_replication_slot_retained_wal_bytes` | `> 0.8 × max_slot_wal_keep_size` |
| Any Spock conflict | `spock_resolutions_total` | `increase() > 0` (alert on ANY) |
| Any remote change skipped on apply | `spock_exceptions_total` | `increase() > 0` (alert on ANY) |
| Signal federation broken | `signal_server_forward_sent_total{outcome="rpc_error"}` | `rate > 0.1/s` (10+/min sustained) |
| Peers disconnected | `signal_server_connected_peers` | `== 0` for any site |

## Deployment options

### A. Local (dev): docker-compose in this directory

```bash
cd ops/monitoring
docker compose up -d
# Prometheus UI: http://localhost:9090
# Alertmanager UI: http://localhost:9093
```

Good for testing alert rules + confirming scrape configs resolve.

### B. Alongside mgmt-host stack (per site)

Add `prometheus` + `alertmanager` services to
`ops/compose/mgmt-host/docker-compose.yml` . Each site scrapes its own mgmt-host and federates upstream
to a central Prometheus.

### C. Central (hub): Prometheus at region B, federates from spokes

Production pattern. region B runs the long-term-storage Prometheus;
each spoke runs a local "agent" Prometheus that ships metrics upstream
via `remote_write`. Alertmanager lives at region B. Deferred until
after Phase 2 validation confirms the single-site version works.

## Conventions

- **Label `site`** on every series — set via each Prometheus's
  `external_labels` at scrape time, not on the scraped targets. Lets
  queries like `signal_server_connected_peers{site="region-a"}` work
  uniformly.
- **No secrets in rule files.** Alert annotations reference runbook
  URLs, not credentials.
- **Runbook URLs:** alerts include a `runbook_url` annotation pointing
  at `docs/operations/*-runbook.md`. Operators click through during
  incidents; don't paste fix steps into the alert body.
