# postgres-encoding-migrate

**Status: WIP, not for production use** — the gate is one live migration of a
goatnet run end to end.

Moves every Spock member of one goatnet, and each member's streaming standby,
from SQL_ASCII to UTF8, keeping the old PGDATA for rollback. Driven by
`ops/ansible/postgres-encoding-migrate.yml`; the runbook (when, outage, disk,
phases, gates, rollback, cleanup) is
[docs/operations/postgres-utf8-migration.md](../../../../docs/operations/postgres-utf8-migration.md).

| File | Phase |
|---|---|
| `tasks/context.yml` | facts every phase reads: member kind, run, compose project, PGDATA volume |
| `tasks/phase.yml` | runs one phase unless its marker says done |
| `tasks/preflight.yml` | 1 |
| `tasks/integrity.yml`, `tasks/integrity-gate.yml`, `tasks/integrity-resample.yml` | 1 and 3: replicated key sets, foreign-key orphans |
| `tasks/quiesce.yml` | 2 |
| `tasks/capture.yml`, `tasks/counts.yml` | 3 |
| `tasks/snapshot.yml` | 4 |
| `tasks/reinit.yml`, `tasks/wait-ready.yml` | 5 |
| `tasks/standby.yml` | 6 (`postgres-streaming-replication`) |
| `tasks/pgbackrest.yml` | 7 |
| `tasks/rewire.yml` | 8 (`spock-mesh-peering`, `ory-narrow-repset`) |
| `tasks/finish.yml`, `tasks/start-writers.yml` | 9 |
| `tasks/reconcile.yml`, `tasks/rollback.yml`, `tasks/cleanup.yml`, `tasks/status.yml` | actions |
| `files/pg-utf8-stream.py` | byte scan of a dump; the restore filter (encoding declarations, optional LATIN1 repair) |

Tests: `tests/syntax-check.sh` (CI), `tests/run-test.sh` (Docker, two nodes).
