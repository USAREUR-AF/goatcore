# spock-drift-watch

Interim L2a drift detection per [ADR 1010 §D5](../../../../docs/adr/1010-ory-on-spock-mindful-operation.md) and Block 52A.3. Runs every hour on a mgmt-host, computes per-table content hashes against a configured list of Spock peer postgres instances, emits `spock_ace_drift_*` gauges through the node_exporter textfile collector.

**Interim form, deliberately.** A small bash + psql script. Upgrades to a sidecar / Go daemon iff the cron form proves insufficient — no current evidence it will. ADR 1010 §D5 names the destination state ("ACE drift-watch cron"); pgEdge ACE is not yet packaged in the postgres-spock image, so this role implements an SQL-only equivalent for the watched-tables list.

## Scope

- **Per-site**, with N≥2 Spock peers reachable from the running host. Peer list is config (`spock_drift_watch_peers`); one entry per postgres instance.
- **Pair-wise comparison**: every unordered pair of peers compared on every cycle.
- **Per-table hash + count diff**: caught by `spock_ace_drift_rows{db,table_pair,site_pair}` > 0 → alert.
- **Heartbeat**: `spock_ace_drift_watch_last_run_seconds{site}` updated on every successful run; staleness alert fires at 2× cadence.

If `spock_drift_watch_peers` has fewer than 2 entries, the script writes a heartbeat and exits cleanly with a "no-peers" gauge — no drift checks run, no alerts fire.

## Watched tables (default)

Matches the [`daily-state-reconciliation.md` §5 reconcile-list](../../../../docs/design/daily-state-reconciliation.md):

```yaml
spock_drift_watch_tables:
  - { db: netbird, table: accounts }
  - { db: netbird, table: users }
  - { db: netbird, table: setup_keys }
  - { db: netbird, table: peers }
  - { db: netbird, table: groups }
  - { db: netbird, table: policies }
  - { db: kratos,  table: identities }
  - { db: kratos,  table: identity_credentials }
  - { db: hydra,   table: hydra_client }
```

Override per-site if the table list grows or shrinks.

An entry may carry `where:` to compare only the rows every site must share.
The default list uses it once: `hydra.networks` compares only the pre-seeded
shared row (`created_at < '2013-01-01'`), because each Hydra migration adds
its own default row with a random id. That row is unused,
since Hydra adopts the earliest row, but without the filter the table would
show drift permanently on every goatnet that runs Ory on Spock.

## How the diff works

For each (peer-A, peer-B, db, table):

```sql
SELECT
  COUNT(*) AS row_count,
  COALESCE(md5(string_agg(md5(t::text), ',' ORDER BY t::text)), '') AS table_hash
FROM <table> t
```

Run on both peers. Compare:
- If row counts differ → drift count = abs(count_a - count_b).
- If row counts match but hashes differ → drift count = 1 (column-level divergence; F-003 LWW silent loss class).
- If both match → drift count = 0.

Output one line per (db, table_pair, site_pair) to the textfile collector. The script does not attempt to identify *which* rows differ — that's L3 reconciler scope per ADR 0823.

## Losses a heal hides

The diff compares what the sites hold now. When two sites wrote different
rows under one key while cut off, last-write-wins keeps one of them at both
sites on heal, so the diff reports them equal and the other row is gone.
Spock logs each such replacement in `spock.resolutions` (with
`spock.save_resolutions = on`, set in the image's `postgresql.conf`).

For every peer and every watched table, drift-watch reports how many rows that
log shows replaced with different content:

```text
spock_ace_lww_lost_rows{site,peer,db,table}
```

It counts `insert_exists` and `update_update` rows whose local and remote
versions differ. A resolution whose two versions are equal, such as one row
written identically at both sites, is not a loss. A row whose versions differ
only in a value each site stamps for itself (a `created_at`, say) still
counts. The gauge is the count of the log as it stands, so nothing is kept
between runs, and it falls when the log is pruned. `SpockHealReplacedRow`
(`ops/compose/mgmt-host/monitoring/rules/spock.yml`) alerts on an increase
over 2h, twice the hourly cadence.

Reading `spock.resolutions` needs `USAGE` on schema `spock` and `SELECT` on
the table, which a non-superuser does not have by default. When the peer's
user cannot read it, drift-watch writes no loss count for that peer and
database, reports `spock_ace_lww_lost_readable{site,peer,db} 0`, and still
exits 0. It reports `1` when the count was made, or the database has no
Spock. The partition harness
(`ops/postgres/spock/tests/partition-harness`, scenario
`portal-c1-two-topics-both-sides-loss-reported`) proves the count on a real
heal.

## What this role does NOT do

- **Not authoritative-mode merge.** Reports only. Per ADR 1010 §D5 + ADR 0823 §7.
- **No row-level diff output.** Only counts. L3 reconciler is the destination for canonical state diff.
- **No cross-site Postgres credential management.** Operator wires peer credentials into `.envrc.local` per the existing pattern; this role just consumes env vars at runtime.
- **No automatic peer discovery.** Peer list is explicit config — drift-watch is *not* the place to auto-detect Spock subscriptions.

## Files

- `files/spock-drift-watch.sh` — the diff script
- `files/spock-drift-watch.service` — oneshot systemd unit
- `files/spock-drift-watch.timer` — hourly schedule
- `templates/spock-drift-watch.env.j2` — env file with peer list + credentials
- `defaults/main.yml` — config surface

## Verification

After deploy:

```bash
sudo systemctl status spock-drift-watch.timer
sudo systemctl start spock-drift-watch.service   # one-shot run
cat /var/lib/node_exporter/textfile/spock_ace_drift.prom
```

Expect a heartbeat line + per-pair drift counts. Prometheus `up{job="node-exporter"}` scrape should pick up the new metrics within one cycle.

## Validation drill (alert fire-test)

Per [`replication-consistency-observability.md` §9](../../../../docs/design/replication-consistency-observability.md):

```sql
-- On peer A only — bypass Spock to create deliberate drift
\c netbird
BEGIN;
SET LOCAL spock.replicate = 'off';
INSERT INTO drift_test_synth (id, val) VALUES (gen_random_uuid(), 'drift-watch-test-2026-05-03');
COMMIT;
```

Within one cycle, `spock_ace_drift_rows{table="drift_test_synth"}` should report 1, and `SpockACEDriftDetected` should fire after 5m. Clean up by deleting the row on both sides through normal replication after the alert fires.
