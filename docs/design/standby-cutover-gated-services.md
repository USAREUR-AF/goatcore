# Standby cutover-gated services

The living reference for the service class that **must lie dormant on a backup
node and be switched on only at cutover**. Decision: [ADR 1111](../adr/1111-cutover-gated-service-class-for-standby-nodes.md).
Labels (the SoT): `ops/standby/service-classes.yaml`.

## The problem this solves

A dormant standby (topology #2, e.g. zagros-b) is only *safe* if the services that
would collide with the live node stay off until promotion. Some services are
fine to run on a backup; some are not. That line was drawn only by hand — a prose
"does not run" comment + scattered flags — with no definition and no enforcement,
so it drifted and a service could be turned on active on a standby by accident.

## The one test (four triggers)

A service is **cutover-gated** if it trips any of:

| Trigger | Meaning | Examples |
|---|---|---|
| **shared-write** | writes a Spock-replicated plane (identity via Hydra/Kratos `ory_narrow`; account/policy via netbird DB) or DNS (isitup/CoreDNS) | portal, id-console, dashboard, account-seed, isitup, relay-health (Hydra client) |
| **route** | serves clients on a route/DNS/ingress the fleet points at | every dashboard/console/inspection surface, MinIO console, isitup |
| **side-effect** | fires outbound effects (email/SES, onboarding, cert mint, notifications) | onboard-runner/intake, SES, stage-notify |
| **singleton** | holds a one-per-goatnet "vantage"/rollup/ingest role | relay-fleet prober, node-net-health rollup, relay-metrics ingest |

Everything else is **standby-safe** and runs continuously:

- **Reveal-only self-publishers** — write only the node's OWN router under a
  node-scoped key, no route, no shared write (the env-health publishers, host-net
  publish, node_net_health *publisher*, the fabric-crawler *census*).
- **Host-local probes** — must run where the thing they probe is (zenoh-health).
- **The listen-only router** — no `connect:`, no clients routed to it.
- **Passive replication receivers** — Spock/ory_narrow/MinIO-async pull.

**Granularity:** the split is sometimes per-MODE, not per-service —
node_net_health *publisher* (safe) vs *rollup* (gated); MinIO *storage* (safe) vs
*console* (gated). The manifest splits those into two rows.

## How it's engineered (not prose)

1. **Node flag** — `node_standby: true` in the node's group_vars declares the
   class applies (zagros-b sets it).
2. **Manifest** — `ops/standby/service-classes.yaml` labels every service:
   class, `signal` (the group_vars key whose truthy value = active, or
   `pin:<component>` when a fleet-pins `absent` deviation governs it), triggers,
   reason, dormancy sub-mode (`absent` vs `deployed-dormant`).
3. **CI guard** — `ops/ci/check-standby-cutover-gated.sh` fails if a
   boolean-signal cutover-gated service is active in a `node_standby` scope.
   Image-pin ones ride the existing fleet-pins `absent` enforcement.
4. **Promotion derives the switch-on list** — the promotion runbook /
   `promote-<node>.sh` reads `cutover_gated` from the manifest to know exactly
   what to start at cutover, so the "off" set (guard) and the "start" set
   (promotion) can never diverge. Start-on-exactly-one-node is the promotion
   split-brain gate's job.

## Dormancy sub-modes

- **`absent`** — not deployed on the standby at all; the promotion runbook
  *deploys* it at cutover. Default for mutation/admin surfaces (simplest, safest).
- **`deployed-dormant`** — deployed but not started / no route / provisioning
  nothing; the promotion runbook *starts* it. For surfaces where a faster cutover
  is worth carrying the (dormant, side-effect-free) install.

## Status

A promotion runbook that consumes this manifest is planned.

## Related

- [ADR 1084](../adr/1084-block-64r-mgmt-repset-and-two-node-federation.md) D3 — the identity-plane subset this generalizes.
- [node-topologies-and-replication-mechanisms.md](node-topologies-and-replication-mechanisms.md) — zagros-b is topology #2.
- zagros-two-node-signal-bringup-and-netbird-perf-learnings.md §8.5 — the P-phase build (P1/P2 = standby-safe; P3+ = cutover-gated).
- environment-parity.md — the sibling manifest+guard pattern (`parity-capabilities.yaml`).


## Standby execution

Status: built. Avoid audit-role writes on a PostgreSQL standby, avoid a local enrollment-apex pin on a known standby, and check management service health only where it should run. Refuse role writes on an unknown recovery state; preserve DNS pin state on an unknown recovery state.

The `ansible-syntax-check` workflow runs the audit-role, apex-pin and management-smoke regression fixtures on every applicable PR. Tests substitute Docker responses and use temporary files. They do not establish replication correctness, failover safety or retained writes during promotion. No database migration or failover was performed.

Offline regression evidence and remaining release gates are recorded in the corresponding draft PR. No live deployment is claimed.
