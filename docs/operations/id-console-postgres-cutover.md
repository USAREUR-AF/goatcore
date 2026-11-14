# id console: SQLite to Postgres cutover, and rollback

**Status:** built (ADR 1165 step A deploy slice).
**Each goatnet's cutover and any rollback is run with the operator present.**

The id console (`goat-id-console`) keeps its onboarding store and org registry in two SQLite
files on the console host. The Postgres-era image reads them from two Postgres databases on
`postgres-a` instead, with the tables unchanged. A cutover copies the rows across once, proves
the copy table by table, keeps the SQLite files read-only for 28 days, and starts the new image.
A rollback inside those 28 days copies Postgres back to SQLite and re-pins the old image.

Design: [`island-mode-multi-writer.md`](../design/island-mode-multi-writer.md) §5.
Decision: [ADR 1165](../adr/1165-island-mode-multi-writer-every-site-admits.md).
Program step: `island-mode-program.md` step 3.

## Order and soak

| # | Goatnet | Group | fleet-pins key | Ladder rung | Starts after |
|---|---|---|---|---|---|
| 1 | zagros-dev | `zagros_dev_hosts` | `zagros_dev` | dev | the zagros-dev rehearsal below passes |
| 2 | zagros | `zagros_hosts` | `zagros` | preprod | zagros-dev has run on Postgres for at least one day with no rollback |
| 3 | zagros-prod | `zagros_prod_hosts` | `zagros_prod` | prod | zagros has run on Postgres through the ADR 1120 soak with no rollback |

The pins move through `ops/scripts/rc.sh` as for any console release. The store gate makes the
deploy step of `rc.sh` refuse on a goatnet that has not been cut over, so each goatnet's cutover
sits between the pin landing and `rc.sh` being resumed.

## What is in place

| Piece | Where |
|---|---|
| `store-migrate import / export / dump / checkpoint`, shipped in the image at `/usr/local/bin/store-migrate` | `ops/goat-id-console/cmd/store-migrate/`, `ops/goat-id-console/Dockerfile` |
| Image label `org.usareur-af.goat-id-console.store=postgres` on every image built from the Postgres server; an image without it is SQLite-era | `ops/goat-id-console/Dockerfile` |
| Declared store `id_admin_console_store: sqlite \| postgres` (default `sqlite`) | `ops/ansible/roles/id-admin-console-store/defaults/main.yml` |
| Store gate: image label, declared store and host marker `/var/lib/goat-id-console/STORE` must agree, or the console role refuses before rendering anything | `ops/ansible/roles/id-admin-console-store/tasks/gate.yml` |
| Databases `id_console_onboard`, `id_console_orgs`, login role `id_console` owning both, pg_hba line for that role and those two databases only | `ops/ansible/roles/id-admin-console-store/tasks/postgres-db.yml`, `ops/postgres/spock/init/01-databases.sql`, `ops/postgres/spock/conf/pg_hba.conf` |
| Server env: `ONBOARD_DB_DSN` / `ORG_DB_DSN` when the store is postgres, `ONBOARD_DB_PATH` / `ORG_DB_PATH` otherwise (sqlite render byte-identical to before) | `ops/ansible/roles/id-admin-console/templates/id-admin-console.env.j2` |
| Cutover driver, phases 0–7 | `ops/ansible/id-console-postgres-cutover.yml` |
| Rollback driver, phases 0–5 | `ops/ansible/id-console-postgres-rollback.yml` |

Neither database joins a Spock replication set: neither carries the `spock` extension, and
`postgres-db.yml` refuses if `spock_mesh_databases` names either. pgBackRest backs them up with
the rest of the cluster.

## Before the first goatnet

1. **Database password, per goatnet.** Create a 1Password item for each goatnet's `id_console`
   role password and add its reference to `.envrc.local.d/*.refs`:
   `ZAGROS_DEV_ID_CONSOLE_DB_PASS`, `ZAGROS_ID_CONSOLE_DB_PASS`, `ZAGROS_PROD_ID_CONSOLE_DB_PASS`
   (`<SITE_ID upper>_ID_CONSOLE_DB_PASS`, the same shape as `<SITE>_SPIRE_DB_PASS`). The
   superuser password is the existing `<SITE>_PG_PASS`.
2. **Scripts that write the SQLite files.** `ops/scripts/backfill-invite-recipients.py` and
   `ops/goat-id-console/scripts/retro-backfill.py` open `onboard.db` directly. After a cutover the
   file is root-owned `0444`, so they fail on their first write instead of writing rows Postgres
   never sees. Repointing them is island-mode step B5.
3. **Rehearsal on zagros-dev** (design §5 step A3), read-only against the live file: copy
   `onboard.db` and `orgs.db` off the host, run `store-migrate import` into a scratch Postgres, then
   `export`, then `dump` both SQLite files and compare byte for byte. Gate G1 needs this equal.

## Two rules

1. **Rehearse on the goatnet's own files first, every goatnet, never skipped.** Copy its live files
   off the host with the SQLite online backup, import into a scratch Postgres, export, dump and compare.
   A skipped rehearsal has already cost a rollback: one goatnet's console import refused on a value
   the rehearsed goatnet never had (a manifest SQLite stored with blob affinity), with the console
   already stopped. The import tool now reads such a value as the
   text it is (#2232); the rule stands because the next difference will be a different one.
2. **The goatnet runs the latest SQLite-era image before the cutover.** The import refuses any column-set
   difference by design; a goatnet on an older image lacks the columns and tables later SQLite-era
   images add at startup. Put the last
   SQLite-era image on the goatnet first, let it start once, then cut over; the rollback target is then
   that image, not an older one.

## Cutting one goatnet over

Commands run from `ops/ansible` in a checkout of `main`. `<inventory flags>` are the ones
`ops/scripts/deploy-component.sh id-admin-console --goatnet <key> --dry-run` prints for this
laptop. Shown for zagros-dev; substitute the group, key and site prefix from the table above.

1. **Declare the store.** One PR, merged:
   `ops/ansible/group_vars/zagros_dev_hosts/id-admin-console.yml` gains
   `id_admin_console_store: postgres`. From the merge until step 3 finishes, every console deploy
   to zagros-dev refuses at the store gate.
2. **Land the Postgres-era pin.** `ops/scripts/rc.sh id-admin-console --version <X.Y.Z> --ack "<name>"`.
   It tags, lands the zagros-dev pin, then stops at its deploy step with the store gate's refusal
   naming this runbook.
3. **Run the cutover.**
   ```bash
   ../scripts/with-secrets ../scripts/ansible-preflight.sh <inventory flags> \
     id-console-postgres-cutover.yml -e target=zagros_dev_hosts \
     -e postgres_superuser_password="$ZAGROS_DEV_PG_PASS"
   ```
   `--list-tasks` prints the phases without running them. A run that stops anywhere is resumed by
   running the same command again.
4. **Check the ledger.** `ops/scripts/deploy-component.sh id-admin-console --goatnet zagros_dev --verify-only`.
5. **Resume the release.** `ops/scripts/rc.sh id-admin-console --tag <tag> --from rows` for
   zagros-dev; for zagros, `--from preprod-rows` after its own steps 1 and 3; for zagros-prod, the prod
   fold `--from prod` stops at its deploy, then steps 1 and 3 for zagros-prod, then
   `deploy-component.sh ... --goatnet zagros_prod --verify-only`.

### The phases

| Phase | Kind | Does | Done when |
|---|---|---|---|
| 0 | GATE | Reads only. Runs on the `router_role` A host. Refuses unless: group_vars declare `postgres`; the pin carries the `postgres` label; every env var the console deploy templates resolves (`ops/scripts/deploy-secrets-needed.py`); the newest pgBackRest backup finished within 26 h; free space for archive, export and import; `postgres-a` up; both SQLite files present; the running image is SQLite-era; the target databases hold no rows | It prints the running image, both SQLite paths and the target image |
| 1 | A, GATE | Role `id_console`, both databases owned by it, pg_hba line, reload | Both databases exist, owned by `id_console`, every table empty |
| 2 | A | `docker stop goat-id-console` | Container not running |
| 3 | A | `store-migrate checkpoint` on both files; copy to `/var/lib/goat-id-console/sqlite-archive/<UTC stamp>/` (`0444`, directory `0555`); `MANIFEST.yml` with sha256, original paths, the image that was running and `retain_until`; originals made `root:root 0444` in place; `sqlite-archive/CURRENT` points at the stamp | Copies byte-identical to originals; no `-wal`/`-shm`/`-journal` |
| 4 | A | `store-migrate import` from the archive copy. On any difference nothing is committed, the console stays stopped and the play prints the rollback command | `import-report.txt` beside the archive; marker says `postgres` |
| 5 | GATE | Declared store and host marker both `postgres` | — |
| 6 | A | The `id-admin-console` role deploys the pin (env with DSNs, ledger row) | Container recreated on the pin |
| 7 | GATE | `/healthz` 200; running image = pin; env has exactly `ONBOARD_DB_DSN` and `ORG_DB_DSN`; `onboard_proposals` and `orgs` row counts in Postgres equal the import report | Prints `CUT OVER` |

### Done looks like

- The play ends with `CUT OVER: <host> serves the id console from Postgres`.
- `cat /var/lib/goat-id-console/STORE` on the host starts with `postgres`.
- `/var/lib/goat-id-console/sqlite-archive/<stamp>/` holds `onboard.db`, `orgs.db`, `MANIFEST.yml`,
  `import-report.txt`, every row of the report `ok`.
- `deploy-component.sh ... --verify-only` passes.
- An admin opens the console's queue and sees the same proposals as before.

## Rollback

Inside the archive's `retain_until` (28 days):

```bash
../scripts/with-secrets ../scripts/ansible-preflight.sh <inventory flags> \
  id-console-postgres-rollback.yml -e target=zagros_dev_hosts
```

| Phase | Kind | Does |
|---|---|---|
| 0 | GATE | Reads `sqlite-archive/CURRENT`, its `MANIFEST.yml` and the marker. Refuses past `retain_until`, or if the previous image is not SQLite-era. Mode: **export** when the marker says `postgres`, **restore** otherwise (the import never committed) |
| 1 | A | Stop the console |
| 2 | A | Export mode: `store-migrate export` to `sqlite-archive/<stamp>-rollback-<UTC stamp>/` with the current image, table by table against Postgres; `export-report.txt` kept there |
| 3 | A | Move any `-wal`/`-shm`/`-journal` aside; exported files (or, in restore mode, the archived bytes) at the original paths, owned `65532`, `0644`; marker `sqlite` |
| 4 | A | `id-admin-console` role with the previous image and store `sqlite` (ledger row) |
| 5 | GATE | `/healthz` 200; running image = previous image; env has exactly `ONBOARD_DB_PATH` and `ORG_DB_PATH` |

**Rolled back looks like:** the play ends with `ROLLED BACK`, the marker starts with `sqlite`,
the console runs the image named in `MANIFEST.yml` `previous_image`, and admins see every
proposal including those made while on Postgres. The Postgres databases are left as they were.

**Inside the 28 days, two rules hold.** Accounts and proposals created after the cutover live only in
Postgres; the export brings every one of them into the fresh SQLite files, so the rollback never
restores the stale archive over them. And `store-migrate export` copies schema version 1 only, so a
release that migrates the console schema while a goatnet is inside its window ships the matching export in
the same release, or the rollback refuses at phase 2.

**Then, in one PR:** set `id_admin_console_store: sqlite` (or delete the line) for the goatnet and
put its pin back to `previous_image`. Until that merges every routine console deploy to the
goatnet refuses at the store gate.

## Cutting over again after a rollback

Import fills only empty databases, so the earlier rows are removed first, with the console on
SQLite and the operator present:

```bash
docker exec -e PGPASSWORD=... postgres-a psql -U postgres \
  -c 'DROP DATABASE id_console_onboard' -c 'DROP DATABASE id_console_orgs'
```

The cutover's "Postgres databases, login role and pg_hba" step creates both again. Then follow "Cutting one goatnet over" from step 1.
