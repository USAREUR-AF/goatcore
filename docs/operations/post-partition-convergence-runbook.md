# Post-partition convergence runbook

**Audience.** On-call operator at any goat site after a network partition (intra-site or inter-site) of >5 min duration involving the postgres-spock mgmt-plane. Also the destination for runbook links from `SpockSubscriptionNotReplicating`, `SpockSlotInactive`, `SpockSlotWALSenderLag`, `SpockACEDriftDetected`, and `SpockACEDriftWatchStale`.

**Goal.** Move from "partition healed at the network level" → "convergence verified, incident closed." Until every step below passes, **the site stays in incident state.** A green network does not mean a green database.

**Authority.** Implements [`docs/design/replication-consistency-observability.md` §5](../design/replication-consistency-observability.md). Canonical conflict-policy reference: [ADR 1010](../adr/1010-ory-on-spock-mindful-operation.md) D5/D7 + [`daily-state-reconciliation.md`](../design/daily-state-reconciliation.md) §6.

---

## 0. Pre-flight — what you should already have open

- Grafana → Spock-health dashboard
- Prometheus → alerts page
- An admin SSH session into the affected mgmt-host
- The pgEdge ace CLI **if** installed (Phase 3+; not present at v1) — otherwise the L2a check uses the drift-watch cron output

If you are paged with `SpockSlotInvalidated`, **stop reading this and go directly to** §A "Slot rebuild" below — convergence is impossible from an invalidated slot; the slot must be rebuilt first.

---

## 1. L1 caught up — replication has resumed and lag is bounded

```bash
# On any mgmt-host with psql access to local postgres-a
psql -h 127.0.0.1 -p 5432 -U postgres -d netbird -c \
  "SELECT subscription_name, status FROM spock.sub_show_status();"
# Repeat with -d kratos and -d hydra when Ory replicates. Then, on each
# provider (each other site), what its WAL senders still have to send and replay:
psql -h 127.0.0.1 -p 5432 -U postgres -c \
  "SELECT application_name,
          pg_wal_lsn_diff(pg_current_wal_lsn(), sent_lsn) AS pending_bytes,
          COALESCE(EXTRACT(EPOCH FROM replay_lag), 0) AS replay_lag_s
   FROM pg_stat_replication;"
```

**Pass criteria:**

- Every subscription `status` is `replicating` (not `init`, not `down`, not `unknown`).
- At each provider, every Spock WAL sender has `pending_bytes` **0** and `replay_lag_s` **< 5 s**, sustained over **2 consecutive minutes**.
- Not `spock.lag_tracker`'s `replication_lag`: it is the time since the last change arrived, so on a quiet database it grows while everything is caught up.
- The 2-minute window is a hard prereq. Watch the dashboard's WAL sender backlog, not just the absence of alerts.

**Common holds:**

| Symptom | Likely cause | Action |
|---|---|---|
| `status = init` | Subscription is doing initial sync (full table copy) | Wait. Initial sync of the netbird DB at our scale takes single-digit minutes; if >30 min, escalate. |
| `status = down` | Apply worker crashed | `psql ... -c "SELECT spock.sub_enable('<sub>');"`. Check Postgres logs for the underlying error before retry. |
| `pending_bytes` keeps growing | Provider WAL sender is sending faster than network can drain | Check `SpockSlotWALSenderLag` panel — this is the alert that surfaces this case. |
| `pending_bytes` is 0 and `status` says `replicating` | If `pg_replication_slots.active = false` for the same slot, the subscription thinks it's healthy but no bytes are flowing. See §2. |

---

## 2. No invalidated slots, all slots active

```bash
psql -h 127.0.0.1 -p 5432 -U postgres -d postgres -c \
  "SELECT slot_name, slot_type, database, active,
          pg_size_pretty(
            pg_wal_lsn_diff(pg_current_wal_lsn(), restart_lsn)
          ) AS retained_wal,
          wal_status
   FROM pg_replication_slots
   ORDER BY slot_name;"
```

**Pass criteria:**

- `wal_status` is `reserved` or `extended` for every slot (NOT `lost`).
- `active` is `t` for every slot belonging to a Spock subscription.
- `retained_wal` < `max_slot_wal_keep_size`. Read it on this site (`SHOW max_slot_wal_keep_size;`): the Spock image sets one value and a site with a streaming pair raises it ([The slot WAL cap](../../ops/postgres/spock/README.md#the-slot-wal-cap)).

The detector reads every Spock subscription and slot of one site, each slot where it lives, and names the ones that cannot recover (refused GB-387 when one is lost). It changes nothing:

```bash
goatbuilder join-node <env> --site <site> --node <node> --class core --slots [--peer <other-site> ...]
```

**If `wal_status = lost` on any slot** → go to §A. The slot is invalidated; convergence cannot proceed without rebuild.

**If `active = false` but `wal_status` healthy** → the consumer disconnected without invalidating. Check:

```bash
psql ... -c "SELECT pid, application_name, state, sync_state, sent_lsn, flush_lsn
             FROM pg_stat_replication;"
```

If empty or missing the expected subscriber: the subscriber's WAL receiver isn't connected. On the *subscribing* node, run `psql ... -c "SELECT spock.sub_enable('<sub>');"` to retry. Network path issues: verify mesh IP reachability and TCP 5432 from subscriber to provider.

---

## 3. Conflicts triaged

Every conflict and every skipped change recorded during the partition window
must be human-reviewed. Revocations are not part of this review: the revocation
guard (ADR 1193) keeps them whichever write wins; step 3c checks it.

Spock 5 has no `spock.conflict_log`. Conflicts are in `spock.resolutions`
(`spock.save_resolutions = on`, set in the `postgres-spock` image) and changes
the apply worker could not apply are in `spock.exception_log`. Both are per
database: run each query in `netbird`, `kratos` and `hydra`, on every site. A
conflict is recorded only at the site that **kept** its local row; the site whose
row was overwritten records nothing, so read every site.

**3a. Conflicts resolved by last-writer-wins**

```bash
psql -h 127.0.0.1 -p 5432 -U postgres -d netbird -x -c \
  "SELECT * FROM spock.resolutions
   WHERE log_time > NOW() - INTERVAL '<partition duration> + 10 minutes'
   ORDER BY log_time DESC;"
```

For **each row**, decide:

1. **LWW winner correct** — the row that "won" via last-writer-wins is the operationally correct version. No action needed; mark the row reviewed.
2. **LWW winner wrong** — the row that lost is the correct one. Apply a compensating UPDATE on the winning side via the relevant service's admin API (NOT direct SQL — per ADR 1010 D4). Document the override in the incident log.
3. **Conflict reflects competing intent** — both sides intended different things; this is a mission-policy question, escalate to the policy owner.

**3b. Remote changes skipped on apply**

```bash
psql -h 127.0.0.1 -p 5432 -U postgres -d netbird -x -c \
  "SELECT * FROM spock.exception_log ORDER BY remote_commit_ts DESC LIMIT 50;"
```

An `UPDATE` with "did not find row to be updated" on a row this site deleted is
expected and correct: the delete stands. Any other row is a change that did not
land here, and the sites disagree on that row until it is re-applied through the
service's admin API.

**3c. Revocations held**

```bash
for db in netbird kratos; do
  psql -h 127.0.0.1 -p 5432 -U postgres -d "$db" -tAc "SELECT '$db sweep changed ' || goat_revocation_sweep() || ' rows';"
done
```

Expect `0` on every site. A non-zero count means a revoked row had come back
and the sweep has just corrected it; record which (`SELECT * FROM
goat_revocations ORDER BY at DESC LIMIT 20;`) in the incident log.

Do not declare convergence until 3a and 3b have zero un-reviewed rows newer than
the partition start and 3c returns 0 on every site.

For Ory tables (`kratos.identities`, `hydra_client`) — F-003 silent column-loss can occur on concurrent updates (LWW is *per-row*, not per-column). §4 catches divergence between sites; it cannot catch a value both sites agree on, which is why 3c exists for revocations.

---

## 4. L2a clean — manual ACE drift-watch run

**Standard path (drift-watch cron installed per Block 52A.3):**

```bash
sudo systemctl start spock-drift-watch.service
cat /var/lib/node_exporter/textfile/spock_ace_drift.prom \
  | grep -E '^spock_ace_drift_rows' \
  | awk '{ if ($NF != "0") print }'
```

**Pass criteria:** every `spock_ace_drift_rows` line ends in `0`. Any non-zero value indicates row-level divergence on that table-pair.

**Fallback path (drift-watch cron not yet installed):**

```bash
# For each watched table, run on each peer and diff
TABLES="accounts users setup_keys peers groups policies"
for db in netbird; do
  for tab in $TABLES; do
    echo "=== $db.$tab ==="
    for peer in postgres-a postgres-b; do
      psql -h 127.0.0.1 -p $([ "$peer" = "postgres-a" ] && echo 5432 || echo 5433) \
           -U postgres -d "$db" -A -t -c \
           "SELECT COUNT(*), md5(string_agg(md5(t::text), ',' ORDER BY t::text)) FROM $tab t;"
    done
  done
done
```

The two output lines for each table must match exactly. Mismatches → divergence.

**On detected drift:** the row-level identification is **L3 reconciler scope** (ADR 0823) and is not yet implemented. Until then:

- Log every drifting (table, peer-pair) tuple in the incident log.
- Compare canonical state via the relevant service's admin API where possible (Kratos → admin API GET on each identity; netbird-mgmt → API GET on each account/peer).
- Apply compensating writes through admin APIs only.

---

## 5. L2b clean — schema coherence

Until Block 52C ships the schema-drift cron, this is manual:

```bash
for peer_port in 5432 5433; do
  echo "=== port $peer_port ==="
  pg_dump -h 127.0.0.1 -p $peer_port -U postgres -s netbird | sha256sum
  pg_dump -h 127.0.0.1 -p $peer_port -U postgres -s kratos  | sha256sum
  pg_dump -h 127.0.0.1 -p $peer_port -U postgres -s hydra   | sha256sum
done
```

**Pass criteria:** SHA-256 hashes for the same DB across peers match exactly.

**On schema mismatch:** STOP. Do not proceed to §6. Schema drift means a DDL was applied to one peer and not the other, almost certainly a migration that didn't replicate. This is a CRITICAL incident — schema drift can cause silent application errors, data corruption on subsequent writes, and breaks replication invariants.

Resolution: identify the missing migration via `\dx` extension list compare + `\d <table>` per table, apply the missing DDL by hand (with operator pair-review), then re-run §5 until clean.

---

## 6. 30-minute observation window

**Only after §1–§5 pass:** start a 30-minute observation timer. During the window:

- No new alerts from `SpockSubscriptionNotReplicating`, `SpockMeshSubscriptionsMissing`, `SpockSlotInactive`, `SpockSlotInvalidated`, `SpockSlotWALNearLimit`, `SpockSlotWALSenderLag`, `SpockConflictLogged`, `SpockACEDriftDetected`.
- Drift-watch cron emits a fresh successful-run timestamp at least once.
- WAL sender `pending_bytes` stays 0 and `replay_lag_s` < 5 s on both directions.

If any of the above fires during the 30-min window, **the clock resets to step 1.** Convergence is not declared while alerts are flapping.

After 30 clean minutes: declare convergence in the incident log + close the page.

---

## A. Slot rebuild — the destructive path

This appendix is invoked when `wal_status = lost`. The subscriber can no longer replay from the slot; it must be seeded again. **This is destructive on the subscriber side** — its data is wiped and replaced from the provider. Confirm with a second operator before proceeding if site impact is non-trivial.

**First, which kind of subscriber lost its slot.** A Spock member (every site's `postgres-a`, and a phone node) is seeded again through Spock, below under "A Spock member". The `pg_basebackup` rebuild after it copies the provider's whole data directory, and with it the provider's Spock node identity, so it applies only to a streaming standby.

### A Spock member

Follow [spock-site-rebuild.md](spock-site-rebuild.md). `goatbuilder join-node <env> --site <site> --node <node> --class <class> --reseed` rebuilds the member from its founder in eight journaled steps, with a refusal at each. Rows only the member holds are exported and accepted before anything is truncated, and written back after the copy. The lost subscriptions are created again in both directions. Convergence with the founder is proven by row count and digest. `--dry-run` prints each step's command.

Do not rebuild a member by hand with `spock.sub_resync_table(<subscription>, <table>, true)`. Its truncate cascades to the tables that reference the one copied, and those tables are not copied again: measured on lab containers, a parent table's resync emptied its child table on the member ([goatbuilder-hardware-measurement.md](../design/goatbuilder-hardware-measurement.md), "The re-seed in goatbuilder"). The rebuild truncates every table once, itself, in repair mode, and then copies each without truncate.

### A streaming standby

The mechanics are documented in [`goat-ha-dev-validation.md` §1.6](goat-ha-dev-validation.md#16-wal-slot-invalidation--pg_basebackup-rebuild). Summary:

```bash
# On the subscriber side (the one with wal_status=lost)
docker compose -f /opt/mgmt-host/docker-compose.yml stop postgres
sudo rm -rf /opt/docker-data/postgres/pgdata/*

# From a healthy provider
pg_basebackup -h <provider-host> -p <provider-port> -U replicator \
  -D /opt/docker-data/postgres/pgdata -X stream -P -R

# Restart subscriber, recreate Spock subscription
docker compose -f /opt/mgmt-host/docker-compose.yml up -d postgres
# Spock node + subscription registration handled by the
# coordinated-upgrade Ansible play; refer to that play rather than
# typing SQL by hand for production sites.
```

After rebuild completes, return to step 1 and run the full convergence loop — a rebuild is itself a partition+heal in the data path.

---

## Convergence declaration template (paste into incident log)

```
Convergence declared at <UTC ISO timestamp> for site <site_id>.
- Partition window: <start UTC> → <network heal UTC> (<duration>)
- Subscriptions caught up at: <UTC>  (lag < 5s sustained 2m)
- Conflict log rows reviewed: <count>  (overrides applied: <count>)
- ACE drift-watch: clean (last run <UTC>)
- Schema coherence: matched across peers (manual sha256, log attached)
- 30-minute observation window: clean

Operator: <name>
Pair-reviewed: <yes/no — only "no" if site-A or site-B unattended>
Compensating writes applied: <list of tickets, or "none">
```

---

## What this runbook does NOT do

- **Auto-remediation.** Every step requires operator decision + pair-review where the runbook calls for it. The mgmt-plane state is too sensitive for auto-merge before [ADR 0823](../adr/0823-daily-state-reconciliation.md) authoritative-mode lands with explicit per-table conflict policy.
- **Replace [`goat-ha-dev-validation.md`](goat-ha-dev-validation.md).** That doc owns the validation probes. This doc owns the production-incident response.
- **Cover relay, signal, or Zenoh consistency.** Those have their own surfaces. Mgmt-plane only here.
- **Backstop a missing pair operator.** The "pair-reviewed: no" entry in the template is an audit trail, not a green light. Single-operator convergence at 03:00 is acceptable; un-reviewed convergence is not.
