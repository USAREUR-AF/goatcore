# Replication & consistency observability

Status: Accepted (tactical pack), 2026-05-03.

Canonical reference for **how we see, alert on, and RCA** the inter-site and intra-site consistency surface. Consolidates the observability obligations scattered across `ha-multi-site.md`, `ha-multi-site-validation.md`, [`daily-state-reconciliation.md`](daily-state-reconciliation.md), [ADR 0823](../adr/0823-daily-state-reconciliation.md), and [ADR 1010](../adr/1010-ory-on-spock-mindful-operation.md). Companion to (not replacement for) the global metric/alert architecture in [ADR 0815](../adr/0815-observability-stack.md) and the wide-event layer in [ADR 0822](../adr/0822-wide-event-observability-otlp-openobserve.md).

**Forward-composes-with:** [ADR 0224 (progressive configuration rollout with automatic revert)](../adr/0224-progressive-config-rollout-with-auto-revert.md) and the design at [`progressive-config-rollout.md`](progressive-config-rollout.md). The rollout coordinator's activation gates ride this doc's metric/alert substrate; the post-partition convergence behavior the rollout coordinator depends on is the post-partition convergence runbook this design owns. Activation-backlog flush + revert-backlog flush are the two specific events the rollout coordinator adds to the convergence runbook — see [`progressive-config-rollout.md §5`](progressive-config-rollout.md).

This doc owns the *consistency-class* alert taxonomy. Liveness / latency / capacity stay where they live today.

## 1. Problem

Production cutover to Spock-multi-master puts replication-correctness in the load-bearing path of the mgmt-plane. We need to be able to:

1. **See** — at any moment, is replication healthy across all sites for every replicated DB.
2. **Alert** — silent divergence is the worst class of failure; the alert taxonomy must catch it before an operator notices via secondary symptoms (failed enrollments, silently-lost identity column writes, conflict-log accumulation).
3. **RCA** — when something breaks, the operator has a deterministic path from "alert fired" → "root cause identified" → "remediation applied" → "convergence verified."

The hardest case is **post-island-mode rejoin**: a site partitioned for hours/days, conflict resolutions accumulate, slot WAL retention drifts, divergence-by-LWW becomes possible, and the operator has to know whether to declare convergence or remain in incident state. Today this case has no operator-facing runbook and no advisory-mode reconciler.

## 2. Three-layer time-horizon model

Carried forward verbatim from [`daily-state-reconciliation.md`](daily-state-reconciliation.md) §2 — this doc adds the *observability* surface for each layer; the reconciliation logic itself lives there.

| Layer | Cadence | Mechanism | What it catches |
|---|---|---|---|
| **L1 — Spock** | sub-second | Logical replication via Spock 5.x | Steady-state churn |
| **L2a — ACE drift-watch** | hourly | pgEdge ACE row-level diff per [ADR 1010 §D5](../adr/1010-ory-on-spock-mindful-operation.md) | Row-level divergence within an hour |
| **L2b — Schema-coherence tracker** | hourly | `pg_dump -s` SHA-256 cross-site compare | Schema drift |
| **L3 — Daily reconciler** | 24h | Snapshot exchange + canonical merge per [ADR 0823](../adr/0823-daily-state-reconciliation.md) | What L1+L2 missed |

Each layer has independent failure modes. **An observability story that only watches L1 is incomplete by construction.** The tactical pack delivers the L1 + interim-L2a observability; full L2b / L3 observability ride their respective implementation blocks.

## 3. Alert taxonomy

Three classes — keep them disjoint, name them so operators know what changed:

### 3.1 Liveness (already covered)

`PostgresDown`, `SignalServerDown` — exporter unscrape-able. Lives in [`spock.yml`](../../ops/compose/mgmt-host/monitoring/rules/spock.yml) `postgres.availability` group + [`signal-server.yml`](../../ops/compose/mgmt-host/monitoring/rules/signal-server.yml).

### 3.2 Replication health (L1)

| Alert | Severity | Fires when | Status |
|---|---|---|---|
| `SpockSubscriptionLag` | warning | `spock_subscription_lag_seconds > 60` for 2m | shipped |
| `SpockSlotInvalidated` | critical | `pg_replication_slot_wal_status_lost == 1` for 1m | shipped |
| `SpockSlotWALNearLimit` | critical | retained_wal > 80% of `max_slot_wal_keep_size` for 5m | shipped |
| `SpockConflictLogged` | warning | `increase(spock_resolutions_total[5m]) > 0`, one series per replicated database (`datname`) (was `spock_conflict_log_total`, a metric that never existed: F-574) | built: the mgmt host's postgres-exporter counts `spock.resolutions` in every database Spock replicates (the observability role's `pg_monitoring_spock_databases`: the Spock mesh's databases, and Ory's when Ory replicates). Until 2026-11-14 it read only the `postgres` database, which has no spock schema, so the metric was never exported there. Proven by `ops/postgres/spock/tests/exporter-every-database/run.sh` (a real conflict in kratos counted, CI `postgres-spock.yml`) and `ops/compose/goat-obs/prometheus/tests/spock-every-database.test.yml` (promtool) |
| `SpockApplyExceptionLogged` | warning | `increase(spock_exceptions_total[5m]) > 0`, one series per replicated database (`datname`) | built, read in every replicated database as `SpockConflictLogged` is; its promtool case is in the same test file (ADR 1193) |
| **`SpockSubscriptionNotReplicating`** | **critical** | `spock_subscription_replicating == 0` for 2m | **52A new** |
| **`SpockSlotInactive`** | **critical** | `pg_replication_slot_active == 0` for 2m | **52A new** |
| **`SpockSlotWALSenderLag`** | **warning** | per-subscriber WAL sender backlog > 100 MB for 5m | **52A new** |

The three new alerts close known blind spots:

- **NotReplicating** catches a subscription stuck in `init` / `down` / `unknown` state where lag isn't being measured at all (lag stays 0 → no `SubscriptionLag` alert → silent halt).
- **SlotInactive** catches the 2026-04-25 Mac observation pattern: the subscription view says "replicating" while `pg_replication_slots.active = false`. Both directions are needed — `replicating==1` AND `active==1` — and we alert when either falls.
- **SlotWALSenderLag** is the *provider-side* mirror of subscriber lag: catches the case where the provider's WAL is queueing up faster than the network drains it (no consumer-side lag alert fires until the queue actually starts replaying).

### 3.3 Drift / divergence (L2)

| Alert | Severity | Fires when | Status |
|---|---|---|---|
| **`SpockACEDriftDetected`** | **warning** | `spock_ace_drift_rows > 0` (any table-diff row in the last hourly run) | **52A interim cron** |
| `SpockACEDriftWatchStale` | warning | `time() - spock_ace_drift_watch_last_run_seconds > 7200` (2× cadence) | 52A new |
| `SpockSchemaDriftDetected` (L2b) | warning | per-DB schema hash mismatch across sites | deferred — Block 52C |

ACE drift-watch is delivered as a Prometheus textfile-collector cron in the tactical pack — a small bash + SQL script run by systemd timer that writes `/var/lib/node_exporter/textfile/spock_ace_drift.prom`. No new daemon, no new container. Upgrades to a sidecar later iff the cron form proves insufficient (no current evidence it will).

Mode is **advisory-only** at v1: drift-watch *reports*, never *corrects*. Per [ADR 0823](../adr/0823-daily-state-reconciliation.md) §7 the authoritative-merge mode lands separately and is gated on Block 17A.5 policy engine for approval.

### 3.4 Federation health (signal-server)

Already shipped in [`signal-server.yml`](../../ops/compose/mgmt-host/monitoring/rules/signal-server.yml) — `SignalServerForwardRPCErrors`, `SignalServerRelayMissSpike`, `SignalServerDown`. Two gaps closed by the tactical pack:

- **Forward RPC deadline** tightened in code from 5s → **200ms per peer with a 500ms total broadcast budget**, matching `ha-multi-site-validation.md` §5.5. Wave 3.4 probe prereq.
- A peer-reachability *liveness* probe (separate from error-rate) folds into Block 38 chaos catalog — explicitly not in the tactical pack so we don't fork another framework.

### 3.5 Inhibition

Add two inhibition rules in [`alertmanager.yml.j2`](../../ops/ansible/roles/observability/templates/alertmanager.yml.j2):

- `SpockSlotInvalidated` suppresses `SpockSlotInactive` (same `slot_name`) — invalidated slots are necessarily inactive; one alert is enough.
- `SpockSubscriptionNotReplicating` suppresses `SpockSubscriptionLag` (same `subscription_name`) — lag is meaningless when the subscription isn't replicating.

## 4. Metric inventory

Tactical pack adds three custom metrics to [`postgres-exporter-queries.yml`](../../ops/compose/mgmt-host/monitoring/postgres-exporter-queries.yml) and one textfile-collector metric file:

```
# new in postgres-exporter custom queries
spock_subscription_replicating{subscription_name,site}     # 0/1 — already present in shape, promoted to first-class with explicit label set
pg_replication_slot_active{slot_name,database,site}        # 0/1 — extracted from existing pg_replication_slot query
spock_wal_sender_lag_bytes{subscriber,site}                # bytes — per pg_stat_replication

# new via node_exporter textfile collector (cron-emitted)
spock_ace_drift_rows{db,table_pair,site}                   # rows of diff in last run; 0 == clean
spock_ace_drift_watch_last_run_seconds{site}               # unix epoch — last successful run
```

Existing metrics retained unchanged; this is purely additive.

## 5. Post-rejoin RCA loop

A site that was partitioned for >5 min enters **rejoin-incident** state. Operator-facing runbook at [`../operations/post-partition-convergence-runbook.md`](../operations/post-partition-convergence-runbook.md).

The loop is intentionally explicit and cumulative — declare convergence *only* after every check passes:

1. **L1 caught up** — `spock_subscription_lag_seconds` < 5s on every subscription, both directions, for 2 consecutive minutes (the `SpockSubscriptionLag` alert clearing isn't sufficient — it has a 2m `for:` clause, can flap).
2. **No invalidated slots** — `pg_replication_slot_wal_status_lost` == 0 across all sites. If non-zero, the runbook routes into `pg_basebackup` rebuild before re-attempting convergence.
3. **Conflicts triaged** — every row added to `spock.resolutions` and `spock.exception_log` during the partition window is human-reviewed; LWW winner confirmed correct or compensating write applied; `goat_revocation_sweep()` returns 0 on every site (ADR 1193).
4. **L2a clean** — manual ACE drift-watch run (`spock-ace-drift-watch.sh --once --verbose`) shows zero diff rows on the canonical Ory + netbird tables.
5. **L2b clean** — `spock-schema-drift.sh --once` (deferred — Block 52C) produces matching schema hashes across sites. Until shipped, runbook calls out a manual `pg_dump -s | sha256sum` cross-site compare.
6. **30-minute observation window** with no new alerts before declaring convergence.

Steps 1-3 cover what the tactical pack ships; steps 4-5 work today via manual invocation; step 6 is operator discipline.

## 6. Cross-component coverage

The tactical pack scope is **Spock + signal-server + Ory-on-Spock** because they're the load-bearing parts of the production cutover. Other consistency surfaces ride later blocks:

| Surface | Layer | Where | Status |
|---|---|---|---|
| MinIO Object Lock + audit ledger integrity | L1-equivalent | Block 17D | designed, not shipped |
| Zenoh router federation propagation | L1-equivalent | future Block (not yet sized) | not designed |
| Zenoh DLQ backlog | L1-equivalent | [`zenoh-dlq.md`](zenoh-dlq.md) | designed, observability missing |
| netbird-mgmt setup-key hashing under replication | L1 | covered by Spock alerts above | shipped |
| pgBackRest WAL archive freshness | adjacent (backup, not replication) | [`pgbackrest.md`](pgbackrest.md) | shipped |

Tactical pack does **not** add Zenoh / MinIO consistency alerts — those substrates aren't in production yet at a level where the alert thresholds would be meaningful. They get their own surface entries when their respective blocks land.

## 7. Sequencing

### Tactical (52-tactical, ships now)

Zero new dependencies. All additive. Validates against the vega cutover.

- 3 new alert rules + inhibition pair in `spock.yml`
- 3 new postgres-exporter metrics
- ACE drift-watch cron + textfile metrics + 2 alert rules
- Forward RPC deadline tightening
- Spock-health Grafana dashboard
- Post-partition convergence runbook

### Strategic (52-strategic, slots into existing blocks)

Genuine upstream dependencies; do not pull forward.

- **52C** — L2b schema-drift cron → light enough to ship alongside 52A but designed *after* the tactical pack lands so we measure whether we actually need the extra signal in production
- **52D** — ADR 0823 advisory-mode wiring → blocked on Block 17A.5 policy engine (notification adapter) + ADR 0823 flipping Proposed → Accepted
- **52E** — `correlation_id` on netbird-mgmt + signal-server + Spock state changes → blocked on Block 27A-C OpenObserve infra
- **52F** — Spock + signal-federation chaos probes → fold into Block 38 catalog when 38A framework lands; do not parallel-build

## 8. Non-goals

- **Authoritative-mode merge.** Out of scope; lives in ADR 0823 §7.
- **Zenoh / MinIO consistency alerts.** Not yet in production at a level where threshold-tuning would be evidence-driven.
- **Auto-remediation.** Every alert in this taxonomy fires for human attention. No alert closes itself by mutating state.
- **Block 38 framework.** Chaos probes for replication invariants land *in* Block 38 when its substrate exists; we do not build a sidecar framework here.
- **Replacement of `ha-multi-site-validation.md`.** That doc owns the validation probes (Wave 3.x); this doc owns the *production observability* of the same surface. Different lifecycle.

## 9. Verification

A consistency observability story that isn't itself verified is theater. Tactical pack ships with:

- **Alert fire-test** for each new rule — induce condition, observe alert, document SLO. Folds into the existing [`ha-multi-site-validation.md` §5.6 / Wave 3.6](../operations/goat-ha-dev-validation.md#36-observability-alert-fire-tests-obsbkp) table.
- **Drift-watch dry-run** — induce a deliberate single-row drift via direct SQL on one site, run `spock-ace-drift-watch.sh --once`, assert non-zero output and `SpockACEDriftDetected` firing.
- **Post-partition runbook** drilled on integdev (or equivalent EC2 pair) by a non-author operator before the vega production cutover declares 52-tactical complete.

Failure of any of these blocks the tactical-pack release marker.
