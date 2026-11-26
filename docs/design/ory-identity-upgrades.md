# Ory identity upgrades: Kratos moves a whole goatnet in one window

| Field | Value |
|---|---|
| Status | **Written:** the window playbook, the per-site refusal in the `ory` role, this procedure. **Unmeasured:** no window has run against real databases; the phase logic was exercised against stubbed `docker`/`psql` only. **Designed, not built:** a rehearsal on a restored two-site copy with real Spock (the Ory candidate suite in [upstream-upgrade-process.md](upstream-upgrade-process.md)) |
| Playbook | [`ops/ansible/ory-kratos-upgrade-window.yml`](../../ops/ansible/ory-kratos-upgrade-window.yml) |
| Refusal | [`ops/ansible/roles/ory/tasks/main.yml`](../../ops/ansible/roles/ory/tasks/main.yml), tasks `kratos-upgrade-window \| ...` |
| Manifest | [`ops/upstream/deps/ory.yaml`](../../ops/upstream/deps/ory.yaml) |
| Decisions | [ADR 1010](../adr/1010-ory-on-spock-mindful-operation.md) (Ory on Spock), [ADR 0857](../adr/0857-patched-selfservice-ui-for-groups-claim.md) (consent image), [ADR 1262](../adr/1262-one-upgrade-process-for-load-bearing-upstream-dependencies.md) (upgrade process), [ADR 1211](../adr/1211-rollout-preflight-readiness-and-reversibility.md) (rollout preflight, rollback class) |
| Rollback class | `restore-required` |

## 1. Why Kratos cannot move one site at a time

**Plain.** Every site of a goatnet keeps its own copy of the identity tables, and Spock copies
rows between them. Spock copies rows but not table changes. When a Kratos release changes those
tables, a site on the new layout and a site on the old one cannot apply each other's rows, so
every site has to change layout together while no Kratos is running.

**Detail.** The `ory_narrow` repset ([`ory-narrow-repset` role](../../ops/ansible/roles/ory-narrow-repset/defaults/main.yml))
replicates `identities`, `identity_credentials`, `identity_credential_identifiers`,
`identity_credential_types`, `identity_recovery_addresses`, `identity_verifiable_addresses`,
`networks` and `goat_revocations` in the `kratos` database. Kratos 26.2.0's migrations over
1.3.1 include, on those tables:

| Migration | Effect under Spock |
|---|---|
| `20250708190000000000_identities_external_id` | adds `identities.external_id`; a row carrying it cannot apply on a site without the column |
| `20251104000000000000_identifiers_devices_identity_id` + Go backfill | adds `identity_credential_identifiers.identity_id` and fills it in batches of 10,000; the backfill's updates are replicated |
| `20251105000000000003/4_identity_id_not_null_fks` | makes `identity_id` NOT NULL with a foreign key; an old Kratos's insert without it fails on a migrated site |
| `20241203105600000000_saml_credential_type` | inserts a fixed-id row into `identity_credential_types` at every site |

Hydra's 14 new migrations touch only site-local tables (`hydra_oauth2_*`, the new
`hydra_oauth2_device_auth_codes`); `hydra_client` and `hydra_jwk`, the two replicated Hydra
tables, are unchanged. Hydra therefore moves site by site through the ordinary path.

## 2. What refuses the per-site move

**Plain.** The ordinary deploy (`ory-deploy.yml`, `ory` role) migrates Kratos only when the
database is new or already current. Anything else stops before a migration runs and names the
window playbook.

**Detail.** Before `kratos-migrate` runs, the role counts rows in `public.schema_migration`. If
there are any, it runs the target image's `kratos migrate sql status -e --format json` and
counts entries with `"state": "Pending"`. Pending migrations on a database that already has
migrations are refused unless `ory_kratos_upgrade_window_run` is set, which only the window
playbook does. A status command that fails is also refused: an unknown state is not migrated. A
new site (empty `kratos` database) migrates as before, so goatnet stand-up and node addition are
unchanged.

## 3. The window

**Plain.** One playbook run takes every site of a goatnet through backup, stop, migrate,
compare, restart and a final data comparison. Sign-in is unavailable from phase 2 until phase 7.

**Detail.** `ory-kratos-upgrade-window.yml -e window_groups=<every site group> -e ory_kratos_window_ack=restore-required`.
It acts on each group's primary (`router_role` A, ADR 1010 D1). Every play is
`any_errors_fatal`: one site failing stops all sites at the same phase.

| Phase | What happens at every primary | Stops the run when |
|---|---|---|
| 0 preflight | Reads running images, applied migrations, `ory_narrow` subscription status, database size, free space; counts rows the `identity_id` backfill cannot fill (§4); fingerprints each replicated Kratos table (row count and md5 of every row) | Target images missing or not digest-pinned; sites declare different Kratos or consent images; sites not on one applied migration set; any backfill blocker; a subscription not `replicating`; free space under twice the `kratos`+`hydra` size |
| 1 backup | `pg_dump -Fc` of `kratos` and `hydra` into `/var/backups/ory-kratos-window/<run>/`, checked with `pg_restore --list`, sha256 and `record.json` (images before, targets, dumps) | A dump fails or reads back empty |
| 2 stop | Stops `mgmt-host-compose-watch.service` (it revives stopped services), then `kratos` and `selfservice-ui` | Either is still running |
| 3 disable | Records and disables every enabled `ory_sub_%` subscription in `kratos` (`spock.sub_disable(name, true)`) | One stays enabled |
| 4 migrate | One site at a time: copies the compose overlay, runs the `ory` role with `ory_kratos_window_hold: true` (config, pins, pull, consent-image gate, Kratos and Hydra migrations, Hydra started; Kratos and the UI stay stopped and the `restart kratos` handler is skipped) | The role fails; any migration still pending afterwards |
| 5 compare | sha256 of `pg_dump --schema-only --schema=public -d kratos` (comments and `SET` lines removed) and the applied migration list | Either differs between sites |
| 6 re-enable | `spock.sub_enable` for the names recorded in phase 3 | Not all `replicating` within `ory_window_replicating_timeout_seconds` (300) |
| 7 start | `ory` role `tasks/start.yml`: Kratos, Hydra and the UI up, health, first administrators, issuer smoke check; watcher started | Kratos is not running the target image |
| 8 drift | Waits until every Spock slot on `kratos` has confirmed this site's current WAL position; no `identity_credential_identifiers` row with a NULL `identity_id`; fingerprints again | A table identical across sites in phase 0 now differs. Tables already different in phase 0 are reported, not failed. `spock.exception_log` row counts are reported |

The backfill and the fixed-id `saml` row are written at every site while subscriptions are
disabled, and replay to peers after phase 6 as same-value changes. Phase 8 compares the data
itself rather than relying on an empty exception log.

A failure after phase 2 leaves Kratos and the watcher stopped on purpose, so an old Kratos never
starts on a migrated schema. Every phase is idempotent: fix the cause and re-run the playbook.

## 4. The revocation guard and the `identity_id` backfill

**Plain.** The revocation guard (ADR 1193) silently drops writes to a deleted identity's rows.
Kratos 26.2.0 fills a new `identity_id` column on every identifier and then requires it. Any
identifier the guard protects, or whose owner is already gone, stays empty and stops that
migration with Kratos stopped at every site. The window counts those rows first and refuses to
start while there are any.

**Detail.** The Postgres backfill
(`persistence/sql/migrations/go/20251105000000000000_identity_id_not_null_fks.go`) repeats
`UPDATE identity_credential_identifiers ... SET identity_id = ic.identity_id` in batches of
10,000 for rows joined to `identity_credentials` on `id` and `nid`, until a batch updates
nothing; the same is done for `session_devices` from `sessions`. Migration
`20251105000000000003` then sets both columns NOT NULL with a foreign key to `identities`. The
`goat_guard` trigger on `identity_credential_identifiers` returns NULL for an owner with an
uncancelled `kratos.identity.deleted` revocation, so the UPDATE leaves that row NULL and the
NOT NULL step fails. A delete applied by Spock runs with foreign-key triggers off, so a site can
also hold children of an identity that no longer exists; those fill with a missing id and the
foreign key fails.

[`ops/ansible/roles/ory/files/kratos-backfill-blockers.sql`](../../ops/ansible/roles/ory/files/kratos-backfill-blockers.sql)
counts four classes: `identifier_without_credential`, `identifier_owner_missing`,
`identifier_owner_deleted`, `session_device_orphaned`. Phase 0 requires every count to be 0.
Remedy: `SELECT goat_revocation_sweep();` in `kratos` (removes a revoked identity's children),
delete the remaining orphans, re-run.

[`ops/postgres/spock/tests/kratos-26-guard/run.sh`](../../ops/postgres/spock/tests/kratos-26-guard/run.sh)
(`postgres-spock` workflow, job `kratos-26-guard`) runs the real Kratos 1.3.1 and 26.2.0
migrations on a postgres-spock database with the guard installed and rows written by Kratos:
the backfill and NOT NULL complete, no identifier is NULL or mismatched, every guard trigger is
still `ENABLE ALWAYS`, a deleted identity's re-inserted rows are dropped on the 26.2.0 schema,
and on a copy seeded with both blocker kinds the query counts them and the 26.2.0 migration
fails.

## 5. Restore

**Plain.** Kratos 1.3.1 cannot read the 26.2.0 schema, so going back means putting every site's
phase-1 dump back and redeploying the old pins. This is why the run demands
`ory_kratos_window_ack=restore-required`.

**Detail.** Designed, not rehearsed. At every primary: stop the watcher, `kratos` and
`selfservice-ui`; disable the `ory_sub_%` subscriptions in `kratos`; `pg_restore --clean
--if-exists --single-transaction -d kratos` from `<run>/kratos.dump` (and `hydra.dump` if Hydra
moved); compare the applied migration list across sites; re-enable the subscriptions; set the
previous `ory_kratos_image`, `kratos_selfservice_ui_image` and `ory_hydra_image` from
`record.json` and run `ory-deploy.yml`. Identities created after phase 7 are lost unless
re-imported.

## 6. Moving together

Kratos, the consent image (rebuilt on `oryd/kratos-selfservice-ui-node` of the same version,
all five patches applied and checked by `ops/ory-consent/tests/verify-patched.sh`) and the
config `version:` keys move in the same window. `upstream.py check ory` holds every image
reference, index digest and config version to the manifest.

Retest after a window: `/admin/sessions` no longer sends `x-total-count` (no goatcore caller
reads it; `/admin/identities/{id}/sessions` keeps `page`/`per_page`); `require_verified_address`
now starts a verification flow instead of returning an error; the login-form double-submit guard
(patch 0005) against the new node-group layout.
