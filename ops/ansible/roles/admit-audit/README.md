# admit-audit role

Deploys the admit-audit read-only sidecar (C1 of
[`docs/design/mgmt-plane-threat-mitigations.md`](../../../../docs/design/mgmt-plane-threat-mitigations.md)
§2.1) and the matching IOC alert rules (C3) onto an mgmt-host.

Composes on top of the existing mgmt-host base stack
(`docker-compose.yml`) and the observability overlay
(`docker-compose.obs.yml`) via `docker-compose.admit-audit.yml` —
no changes to either base file.

## What this deploys

- A least-privilege PG role `admit_audit` with `SELECT` on `public.peers` only.
- The admit-audit sidecar container (polls every 30s, rolling window 5m).
- The `admit-ioc.yml` Prometheus rules file.
- An `admit-audit` scrape target appended to the observability overlay's
  Prometheus config.

## Pre-requisites

- `ops/ansible/mgmt-host.yml` already ran (base stack up).
- `ops/ansible/vega-observability.yml` (or equivalent role-call) already ran
  (Prometheus + Alertmanager up; `--web.enable-lifecycle` flag is already on
  the prometheus container, which is required for the reload step).

## Variables

Required per-host (set in inventory, inherited from the observability role):

| Var | Example | Meaning |
|---|---|---|
| `site_name` | `vega-a` | Prometheus `external_labels.site` |

Required at play invocation time (from env / 1Password):

| Var | Meaning |
|---|---|
| `postgres_superuser_password` | needed to bootstrap the `admit_audit` PG role |
| `admit_audit_password` | password granted to the `admit_audit` role |

Defaults (overridable):

| Var | Default | Meaning |
|---|---|---|
| `admit_audit_image` | none; set by the `admit-audit` row of `ops/pins/fleet-pins.yaml` (rendered into `group_vars/<member>/pins.yml`) or by the host group's group_vars, as `ghcr.io/<ghcr_org>/goat-admit-audit:sha-<rev>@sha256:<digest>` | the CI build, pinned by digest; the deploy refuses an empty value |
| `pg_admit_audit_role` | `admit_audit` | PG role name |
| `pg_admit_audit_db` | `netbird` | DB holding the `peers` table |

## Idempotency

- PG role creation uses a `DO $$` block; re-run safe.
- Password reset runs every play (rotation = re-run).
- Compose overlay copy + up are declarative; Docker skips no-op cases.
- Prometheus config patch uses `blockinfile` with a fixed marker — re-runs
  are no-ops.
- Prometheus `/-/reload` only fires when the rules file or the scrape
  config block actually changed.

## Source of truth

- Sidecar source: [`ops/admit-audit/`](../../../admit-audit/).
- Compose overlay: [`ops/compose/mgmt-host/docker-compose.admit-audit.yml`](../../../compose/mgmt-host/docker-compose.admit-audit.yml).
- Alert rules: [`ops/compose/mgmt-host/monitoring/rules/admit-ioc.yml`](../../../compose/mgmt-host/monitoring/rules/admit-ioc.yml).
- Threat-model design: [`docs/design/mgmt-plane-threat-mitigations.md`](../../../../docs/design/mgmt-plane-threat-mitigations.md) §2.1.
