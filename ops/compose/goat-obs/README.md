# `ops/compose/goat-obs/` — minimal on-mgmt-host observability stack (per-goatnet)

**Co-resident on the AGX Thor** of a single-Thor goatnet that has no dedicated obs host (`has_obs_host=false`). Container + volume names derive from `${OBS_PREFIX}` (set to `<site_id>-obs` by `obs.yml`), so a sibling goatnet never inherits another's names. The co-fail trade-off (the stack fails with the host it observes) is accepted because the alternative (a dedicated Orin-C utility host) is out of scope for these single-Thor goatnets.

## What's in it

| Service | Image | Port | Purpose |
|---|---|---|---|
| `prometheus` | prom/prometheus:v2.54.1 | `127.0.0.1:9192` | TSDB + rule eval |
| `alertmanager` | prom/alertmanager:v0.27.0 | `127.0.0.1:9193` | critical → PagerDuty (ADR 1059); config rendered per-site by `obs.yml` (empty PD key → webhook-noop) |
| `node-exporter` | prom/node-exporter:v1.8.2 | `127.0.0.1:9100` | Thor host metrics |
| `postgres-exporter` | postgres-exporter:v0.15.0 | `127.0.0.1:9187` | PG metrics via `monitoring` role |
| `grafana` | grafana/grafana:11.3.0 | (Traefik-routed) | UI behind Ory at `grafana.<site_id>.<zone>` |

Joins the existing `mgmt` docker network as external so Traefik (in the mgmt-host compose project) can route directly to `grafana:3000`. The relay-health prober (a host process) reaches Alertmanager via the loopback-published `127.0.0.1:9193`.

## Driven by

Operator does **not** `docker compose up` by hand. The playbook [`ops/ansible/obs.yml`](../../ansible/obs.yml) renders `.env`, creates the `monitoring` PG role, provisions the Hydra OAuth2 client via the [`grafana-oidc-client`](../../ansible/roles/grafana-oidc-client/) role (overridden to point at the **site** Hydra), and brings the stack up.

Bringup: the header of [`ops/ansible/obs.yml`](../../ansible/obs.yml) lists the inputs and the steps.

## Authentication

- **Local admin:** `GRAFANA_ADMIN_USER` / `GRAFANA_ADMIN_PASSWORD` from `<SITE>_GRAFANA_ADMIN_USER` + `<SITE>_GRAFANA_ADMIN_PASS` env vars. Recovery + provisioning verification only.
- **No egress:** Grafana makes no outbound call of its own — update/plugin checkers, usage reporting, the home-page news feed, gravatar, the plugin catalog and signing-key fetch are all switched off in the compose env (`GF_ANALYTICS_*`, `GF_NEWS_*`, `GF_SECURITY_DISABLE_GRAVATAR`, `GF_PLUGINS_*`). A mesh-internal service reaching grafana.com is a defect, not a default.
- **Operators:** OIDC via site Hydra at `oauth2.<site_id>.<zone>`. Groups → Grafana role via JMESPath (`obs-admin`/`grafana-admin` → Admin; `obs-readonly`/`grafana-readonly` → Viewer; nothing else gets access).

The site-Hydra-for-operator-flows pattern is a deliberate deviation from a goatnet with an obs host (which has a dedicated operator-IdP Hydra on Orin-C per ADR 0842).

## Editing rules / dashboards

- Rules live in `prometheus/rules/*.yml`. Edits land on operator re-run of `obs.yml`; alternatively `docker exec <site_id>-obs-prometheus killall -HUP prometheus` after pushing files.
- Dashboards live in `grafana/dashboards/*.json`. The provisioning loader picks up file edits every 30s (UI edits do **not** persist past container restart — commit JSON to make changes durable).
- The 11-dashboard goat suite (`relay-fleet-health`, `goatnet-service-slo`, `edge-http-red`, `postgres-health`, `fabric-tenants`, `audit-storage-integrity`, `container-incident`, `observability-self-health`, `mesh-control-plane`, `capacity-forecast`, `incident-home`) is **generated** by `grafana/dashboards/gen-dashboards.py` (spec → generator → JSON; re-run it to re-tune, don't hand-edit the generated JSON). Rationale + an honest "are we using Grafana right?" evaluation: [`docs/design/grafana-dashboard-suite.md`](../../../docs/design/grafana-dashboard-suite.md). The five older imports (`node-exporter-full`/`blackbox`/`cadvisor`/`named-processes`/`goat-audit`) are hand/imported JSON, left as-is.

## Why not just reuse `ops/compose/mgmt-host/docker-compose.obs.yml`?

The existing overlay assumes the two-mgmt-host A↔B topology (peer-scrape targets + postgres-exporter on the same compose project as postgres-a). A single-Thor goatnet has neither; reshaping the prod overlay would be more invasive than a minimal new tree. When a second mgmt host lands or a dedicated obs Orin arrives, lift to a parity topology with the production `obs-host` role and retire this directory.
