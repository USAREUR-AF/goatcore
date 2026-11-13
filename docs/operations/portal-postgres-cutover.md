# Portal registry: SQLite to Postgres cutover, and rollback

**Status:** built (ADR 1165 step A, portal slice).
**Each goatnet's cutover and any rollback is run with the operator present.**

The portal (`goat-zenoh-portal`) keeps its registry (orgs, topics, slots, grants, schemas
index, audit events and the rest) in one SQLite file, `/opt/goat-zenoh-portal/registry.db`. The
Postgres-era image reads it from the `portal_registry` database on `postgres-a` instead, with the
tables unchanged. A cutover copies the rows across once, proves the copy table by table, keeps
the SQLite file read-only for 28 days, and starts the new image. A rollback inside those 28 days
copies Postgres back to SQLite and re-pins the old image.

Schema files under `<schema dir>/<topic_id>/`, vendor and slot bundles, recipient certs and the
operator-key directory stay files where they are. Every row keeps its id, so each file is found
under the same path before and after.

Design: [`island-mode-multi-writer.md`](../design/island-mode-multi-writer.md) §5.
Decision: [ADR 1165](../adr/1165-island-mode-multi-writer-every-site-admits.md).
Program step: `island-mode-program.md` step 3.
The id console's stores follow the same path: `docs/operations/id-console-postgres-cutover.md` (PR #2076).

## Order and soak

| # | Goatnet | Group | fleet-pins key | Ladder rung | Starts after |
|---|---|---|---|---|---|
| 1 | zagros-dev | `zagros_dev_hosts` | `zagros_dev` | dev | the zagros-dev rehearsal below passes |
| 2 | zagros | `zagros_hosts` | `zagros` | preprod | zagros-dev has run on Postgres for at least one day with no rollback |
| 3 | zagros-prod | `zagros_prod_hosts` | `zagros_prod` | prod | zagros has run on Postgres through the ADR 1120 soak with no rollback |

The pins move through `ops/scripts/rc.sh` as for any portal release. The store gate makes the
deploy step of `rc.sh` refuse on a goatnet that has not been cut over, so each goatnet's cutover
sits between the pin landing and `rc.sh` being resumed.

## What is in place

| Piece | Where |
|---|---|
| `store-migrate import / export / dump / checkpoint`, shipped in the image at `/usr/local/bin/store-migrate` | `ops/goat-zenoh-portal/cmd/store-migrate/`, `ops/goat-zenoh-portal/Dockerfile` |
| Image label `org.usareur-af.goat-zenoh-portal.store=postgres` on every image built from the Postgres server; an image without it is SQLite-era | `ops/goat-zenoh-portal/Dockerfile` |
| Declared store `portal_store: sqlite \| postgres` (default `sqlite`), set in the goatnet's group_vars | `ops/ansible/roles/goat-zenoh-portal-store/defaults/main.yml` |
| Store gate: image label, declared store and host marker `/var/lib/goat-zenoh-portal/STORE` must agree, or the portal role refuses before rendering anything | `ops/ansible/roles/goat-zenoh-portal-store/tasks/gate.yml` |
| Database `portal_registry`, login role `portal` owning it, pg_hba line for that role and database only | `ops/ansible/roles/goat-zenoh-portal-store/tasks/postgres-db.yml`, `ops/postgres/spock/init/01-databases.sql`, `ops/postgres/spock/conf/pg_hba.conf` |
| Server input: `PORTAL_DB_DSN` in `.env` and no `--db` when the store is postgres; `--db=<portal_db_path>` otherwise (sqlite render byte-identical to before) | `ops/ansible/roles/goat-zenoh-portal/templates/{portal.env.j2,docker-compose.yml.j2}` |
| Registry WAL gauges (`registry-wal-signals` timer) and `rules/registry-wal.yml`: kept on a sqlite goatnet, removed on a postgres one | `ops/ansible/roles/relay-health/tasks/main.yml`, `ops/ansible/obs.yml` |
| Host-side readers of the registry follow `portal_store`: budget-notify, the lazy-storage pre-flight in the `goat-zenoh` role, `sync-slot-anchors.py`, `reap-orphans.py`, `slot-identity-backfill.py` | each script and role; read Postgres through `docker exec postgres-a psql`, read-only |
| Cutover driver, phases 0–7 | `ops/ansible/portal-postgres-cutover.yml` |
| Rollback driver, phases 0–5 | `ops/ansible/portal-postgres-rollback.yml` |

The database does not join a Spock replication set: it does not carry the `spock` extension, and
`postgres-db.yml` refuses if `spock_mesh_databases` names it. pgBackRest backs it up with the rest
of the cluster. The portal reaches `postgres-a` over the `mgmt` docker network it already joins;
`postgres-db.yml` asserts both containers are on it.

## Before the first goatnet

1. **Database password, per goatnet.** Create a 1Password item for each goatnet's `portal` role
   password and add its reference to `.envrc.local.d/*.refs`: `ZAGROS_DEV_PORTAL_DB_PASS`,
   `ZAGROS_PORTAL_DB_PASS`, `ZAGROS_PROD_PORTAL_DB_PASS` (`<SITE_ID upper>_PORTAL_DB_PASS`, the same
   shape as `<SITE>_ID_CONSOLE_DB_PASS`). The superuser password is the existing `<SITE>_PG_PASS`.
2. **Rehearsal on zagros-dev** (design §5 step A3), read-only against the live file: copy
   `registry.db` off the host with the SQLite online backup (`sqlite3 registry.db ".backup
   copy.db"`, never `cp` of a live file), run `store-migrate import` into a scratch Postgres, then
   `export`. Then `store-migrate prune --sqlite copy.db --out pruned.db` and compare the `dump` of
   `pruned.db` with the `dump` of the exported file, byte for byte. Gate G1 needs this equal.

   **Rows the import removes.** The SQLite build never enforced its foreign keys, so a purged
   topic left its `topic_schemas` and `topic_code_examples` rows behind
   (F-593).
   `import` applies each declared `ON DELETE CASCADE` on a private copy and lists every removed
   row in its report as a `# dropped` line; the archived file keeps them for 28 days. A missing
   reference under a key that is not `CASCADE` stops the import.

## Two rules, from zagros on 2026-10-30

1. **Rehearse on the goatnet's own files first, every goatnet, never skipped.** Copy its live files
   off the host with the SQLite online backup, import into a scratch Postgres, export, dump and compare.
2. **The goatnet runs the latest SQLite-era image before the cutover.** The import refuses any column-set
   difference by design; a goatnet on an older image lacks the columns and tables later SQLite-era
   images add at startup. Put the last
   SQLite-era image on the goatnet first, let it start once, then cut over; the rollback target is then
   that image, not an older one.

## Cutting one goatnet over

Commands run from `ops/ansible` in a checkout of `main`. `<inventory flags>` are the ones
`ops/scripts/deploy-component.sh portal --goatnet <key> --dry-run` prints for this laptop. Shown
for zagros-dev; substitute the group, key and site prefix from the table above.

1. **Declare the store.** One PR, merged: `ops/ansible/group_vars/zagros_dev_hosts/vars.yml` gains
   `portal_store: postgres`. From the merge until step 3 finishes, every portal deploy to zagros-dev
   refuses at the store gate.
2. **Land the Postgres-era pin.** `ops/scripts/rc.sh portal --version <X.Y.Z> --ack "<name>"`. It
   tags, lands the zagros-dev pin, then stops at its deploy step with the store gate's refusal naming
   this runbook.
3. **Run the cutover.**
   ```bash
   ../scripts/with-secrets ../scripts/ansible-preflight.sh <inventory flags> \
     portal-postgres-cutover.yml -e target=zagros_dev_hosts \
     -e postgres_superuser_password="$ZAGROS_DEV_PG_PASS"
   ```
   `--list-tasks` prints the phases without running them. A run that stops anywhere is resumed by
   running the same command again.
4. **Check the ledger.** `ops/scripts/deploy-component.sh portal --goatnet zagros_dev --verify-only`.
5. **Repoint the host jobs.** Phase 2 stopped `budget-notify.timer` and `registry-wal-signals.timer`.
   Run, from the same checkout:
   - `budget-notify-deploy.yml` — renders the notifier with its Postgres reader and starts the timer;
   - `relay-health-deploy.yml` — removes `registry-wal-signals` and its `.prom` file;
   - `obs.yml` (zagros-dev and zagros, where the obs stack is co-resident) — stops loading
     `rules/registry-wal.yml`.
   The next `goat-zenoh` deploy reads live slots from Postgres in its lazy-storage pre-flight.
6. **Resume the release.** `ops/scripts/rc.sh portal --tag <tag> --from rows` for zagros-dev; for
   zagros, `--from preprod-rows` after its own steps 1, 3 and 5; for zagros-prod, the prod fold
   `--from prod` stops at its deploy, then steps 1, 3 and 5 for zagros-prod, then
   `deploy-component.sh ... --goatnet zagros_prod --verify-only`.

### The phases

| Phase | Kind | Does | Done when |
|---|---|---|---|
| 0 | GATE | Reads only. Runs on the `router_role` A host. Refuses unless: group_vars declare `postgres`; the pin carries the `postgres` label; every env var the portal deploy templates resolves (`ops/scripts/deploy-secrets-needed.py`); the newest pgBackRest backup finished within 26 h; free space for archive, export and import; `postgres-a` up; the SQLite file named by the running compose file's `--db=` is present; the running image is SQLite-era; the target database holds no rows | It prints the running image, the SQLite path and the target image |
| 1 | A, GATE | Role `portal`, database owned by it, pg_hba line, reload; `postgres-a` on the portal's network | Database exists, owned by `portal`, every table empty |
| 2 | A | Stop `registry-wal-signals.timer` and `budget-notify.timer` (absent is fine), then `docker stop goat-zenoh-portal` | Container not running |
| 3 | A | `store-migrate checkpoint`; copy to `/var/lib/goat-zenoh-portal/sqlite-archive/<UTC stamp>/registry.db` (`0444`, directory `0555`); `MANIFEST.yml` with sha256, original path, the image that was running and `retain_until`; original made `root:root 0444` in place; `sqlite-archive/CURRENT` points at the stamp | Copy byte-identical to the original; no `-wal`/`-shm`/`-journal` |
| 4 | A | `store-migrate import` from the archive copy. On any difference nothing is committed, the portal stays stopped and the play prints the rollback command | `import-report.txt` beside the archive; marker says `postgres` |
| 5 | GATE | Declared store and host marker both `postgres` | — |
| 6 | A | The `goat-zenoh-portal` role deploys the pin (`PORTAL_DB_DSN`, no `--db`, ledger row) | Container recreated on the pin |
| 7 | GATE | `goat-zenoh-portal healthcheck` against `/api/health` (which reads the registry) passes; running image = pin; `.env` has `PORTAL_DB_DSN` and the compose file no `--db`; `vendors`, `topics` and `slots` row counts in Postgres equal the import report | Prints `CUT OVER` |

### Done looks like

- The play ends with `CUT OVER: <host> serves the portal registry from Postgres`.
- `cat /var/lib/goat-zenoh-portal/STORE` on the host starts with `postgres`.
- `/var/lib/goat-zenoh-portal/sqlite-archive/<stamp>/` holds `registry.db`, `MANIFEST.yml`,
  `import-report.txt`, every row of the report `ok`.
- `deploy-component.sh ... --verify-only` passes.
- `goat topic ls` and the portal catalog list the same orgs and topics as before; a topic with
  an uploaded schema still renders it.
- `systemctl status budget-notify.timer` is active again after step 5.

## Rollback

Inside the archive's `retain_until` (28 days):

```bash
../scripts/with-secrets ../scripts/ansible-preflight.sh <inventory flags> \
  portal-postgres-rollback.yml -e target=zagros_dev_hosts
```

| Phase | Kind | Does |
|---|---|---|
| 0 | GATE | Reads `sqlite-archive/CURRENT`, its `MANIFEST.yml` and the marker. Refuses past `retain_until`, or if the previous image is not SQLite-era. Mode: **export** when the marker says `postgres`, **restore** otherwise (the import never committed) |
| 1 | A | Stop `budget-notify.timer` and the portal |
| 2 | A | Export mode: `store-migrate export` to `sqlite-archive/<stamp>-rollback-<UTC stamp>/registry.db` with the current image, table by table against Postgres; `export-report.txt` kept there |
| 3 | A | Move any `-wal`/`-shm`/`-journal` aside; the exported file (or, in restore mode, the archived bytes) at the original path, owned `65532`, `0644`; marker `sqlite` |
| 4 | A | `goat-zenoh-portal` role with the previous image and store `sqlite` (ledger row); start `registry-wal-signals.timer` where its unit is still installed |
| 5 | GATE | `healthcheck` passes; running image = previous image; compose has `--db=<original path>` and `.env` no DSN |

**Rolled back looks like:** the play ends with `ROLLED BACK`, the marker starts with `sqlite`,
the portal runs the image named in `MANIFEST.yml` `previous_image`, and the catalog shows every
org and topic including those created while on Postgres. The Postgres database is left as it was.

**Inside the 28 days, two rules hold.** Accounts and proposals created after the cutover live only in
Postgres; the export brings every one of them into the fresh SQLite files, so the rollback never
restores the stale archive over them. And `store-migrate export` copies schema version 1 only, so a
release that migrates the registry schema while a goatnet is inside its window ships the matching export in
the same release, or the rollback refuses at phase 2.

**Then, in one PR:** set `portal_store: sqlite` (or delete the line) for the goatnet and put its
pin back to `previous_image`. Until that merges every routine portal deploy to the goatnet
refuses at the store gate. After it merges, run `budget-notify-deploy.yml`,
`relay-health-deploy.yml` and `obs.yml` so the SQLite-era readers and the WAL gauges and rules
come back.

## Cutting over again after a rollback

Import fills only an empty database, so the earlier rows are removed first, with the portal on
SQLite and the operator present:

```bash
docker exec -e PGPASSWORD=... postgres-a psql -U postgres -c 'DROP DATABASE portal_registry'
```

The cutover's phase 1 creates it again. Then follow "Cutting one goatnet over" from step 1.
