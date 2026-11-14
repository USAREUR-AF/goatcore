# Island mode, every site writes: portal, console, NetBird and Kratos on Spock

**Status:** Step A (§5) built: the portal and console run on Postgres, in no Spock replication set. The partition harness (§6) is built for NetBird, Kratos and Hydra; its portal and console scenarios are listed in `scenarios/pending.tsv`. Step B (§5), the per-class turn-on (gate G3) and the drills (gate G4) are designed, not built. Until a write class passes gate G3, the admin surfaces run at one site per goatnet ([ADR 1084](../adr/1084-block-64r-mgmt-repset-and-two-node-federation.md) D3); the intended placement is every site. Decision: [ADR 1165](../adr/1165-island-mode-multi-writer-every-site-admits.md) (Proposed 2026-10-29). Execution and resume point: `island-mode-program.md`. Builds on ADR 1193 (revocations are absorbing; Proposed). §2 is the measurement before step A, against `USAREUR-AF/goatcore` main `2aa281381`, 2026-10-28.

Any site of a goatnet that is cut off from the others must keep working fully —
admitting people, enrolling machines, changing and revoking access — and merge cleanly
when the link returns. This doc is the design that makes that true for the four stores
an admin touches: the portal registry, the id console, NetBird management and Kratos.

The approach in one line: move the portal and console onto the Postgres that Spock
already replicates, then shape every write so two sites almost never touch the same row,
and send the rare collision to an admin instead of letting last-write-wins drop it.

---

## 1. Terms

| Term | Meaning |
|---|---|
| Site | One physical location of a goatnet with its own management host and `postgres-a` |
| Island mode | A cut-off site keeps working on its own, then rejoins ([ADR 0206](../adr/0206-island-mode-by-construction.md)) |
| Spock | pgEdge's multi-master logical replication for Postgres ([ADR 0801](../adr/0801-postgresql-spock-multi-master.md)) |
| Last-write-wins | Spock 5's default conflict rule: keep the row with the later commit timestamp, whole row |
| Owner site | The site that created a record; the only site that changes it. For a person's identity, the site it was registered at or last made active at (`metadata_admin.owner_site`, [ADR 1205](../adr/1205-the-highest-echelon-site-overrides-and-can-sever-every-other-site.md) D5) |
| Hub | The goatnet's highest-echelon site, declared as `goatnet_hub_site`. The only site whose changes take precedence over an owner's and the only site that can sever another (ADR 1205), both through `goat_hub_directives` |
| Harness | The two-node Spock partition test in CI (§6) |

---

## 2. What exists today

**Plain:** Spock replicates NetBird and Ory between sites. Everything an admin does in the
portal or console is in SQLite on one host, and [ADR 1084](../adr/1084-block-64r-mgmt-repset-and-two-node-federation.md)
D3 pins all admin writes, NetBird and Ory included, to one site per goatnet.

| Store | Engine | Replicated | Declared in |
|---|---|---|---|
| NetBird management | Postgres `netbird` | 20 tables, repset `netbird_mgmt` | `roles/spock-mesh-peering/defaults/main.yml` |
| Kratos + Hydra | Postgres `kratos`, `hydra` | 10 tables, repset `ory_narrow` | `roles/ory-narrow-repset/defaults/main.yml:65-78` |
| Portal registry | SQLite `/opt/goat-zenoh-portal/registry.db`, 20 tables | no | `roles/goat-zenoh-portal/defaults/main.yml:257` (`portal_db_path`) |
| Console onboarding | SQLite `/var/lib/goat-id-console/onboard.db`, 6 tables | no | `roles/id-admin-console/defaults/main.yml:149` (`ONBOARD_DB_PATH`) |
| Console orgs | SQLite `/var/lib/goat-id-console/orgs.db`, 4 tables | no | `roles/id-admin-console/defaults/main.yml:167` (`ORG_DB_PATH`) |
| Console invites | process memory (`internal/store/invites.go`) | no; lost on restart | `cmd/console/main.go:79` |
| Peerview hold records | JSONL | no | `roles/peerview/defaults/main.yml:58` |
| SPIRE | Postgres `spire` | no, by design | per-site trust domain, [ADR 1164](../adr/1164-per-site-spire-trust-domains.md) |

Conflict rule: Spock 5 default, `track_commit_timestamp=on`
(`ops/postgres/spock/conf/postgresql.conf:46`). Exception logging: not configured.
DDL: not replicated; `ops/ansible/coordinated-upgrade.yml`.

### 2.1 Portal and console stores, re-measured

The store boundary in Go is clean: SQL lives only in `portal/internal/registry`,
`console/internal/onboardstore`, `console/internal/orgstore`. This replaces the counts in
[`portal-console-multisite-readiness.md`](portal-console-multisite-readiness.md) (2026-09-06).

| Item | Count | Where |
|---|---|---|
| Tables | 30 (20 / 6 / 4) | new since Sept: `onboard_actions`, `probation_reactivation_requests` |
| Integer AUTOINCREMENT tables | 6 | `topics`, `topic_schemas`, `topic_code_examples`, `org_parent_edges`, `audit_events`, `trashgoat_specs` |
| `LastInsertId` | 5 | `registry.go:911`, `schemas.go:108`, `code_examples.go:85`, `trashgoat.go:123`, `affiliation_repair.go:203` |
| `ORDER BY …, rowid` | 7 | `actions.go:236,238,275`, `probation.go:96`, `proposals.go:316,352,380,406` |
| SQLite-only SQL | 14 | `PRAGMA table_info` ×2, `PRAGMA wal_checkpoint` ×2 (`walwatch.go:172,227`), `sqlite_master` ×2, `json_extract` ×4, two-argument `MAX` ×3 (`progress.go`), `INSERT OR IGNORE` ×1 (`envelope.go:187`) |
| Uniqueness by error string | 7 | `strings.Contains(err.Error(),"UNIQUE")`, e.g. `registry.go:635`, `orgstore/store.go:361` |
| `SetMaxOpenConns(1)` relied on as a lock | 3 stores | `onboardstore/store.go:15`, `invites.go:394` |
| Places a DB file is opened | 6 | `registry.go:176`, `reader.go:97`, `probe.go:43`, `walwatch.go:109`, `onboardstore/store.go:41`, `orgstore/store.go:137` |
| Scripts opening the files directly | 11 (3 write; `pubsub-export` runs migrations) | write: `ops/scripts/backfill-invite-recipients.py:204`, `goat-id-console/scripts/retro-backfill.py:304`; read: `reap-orphans.py`, `sync-slot-anchors.py`, `slot-identity-backfill.py`, `budget-notify.py:92`, `register-drift.sh`, `roles/goat-zenoh/tasks/main.yml:194`, `registry-wal-signals` role, `registry-wal.yml` alert rules |
| Foreign keys | 10; 6 `ON DELETE CASCADE`, all from `vendors` | `registry.go:1032` fans one delete across 5 tables |
| Secrets held in rows | 3 columns | `operator_keys.secret`, `onboard_proposals.setup_key`, `bundle_token` |
| Files referenced from rows | 5 kinds | vendor/slot/proposal bundles, schema files at `<SchemaDir>/<int topic_id>/`, recipient certs, `OPERATOR_KEY_DIR` |

---

## 3. The five write classes

**Plain:** last-write-wins is safe when two sites never change the same row. Every write
across the four stores is put in one of five classes, and each class has a rule that
keeps sites apart or makes the merge order-free. The per-path assignment, 126 paths with
file:line, is [`island-mode-write-inventory.md`](island-mode-write-inventory.md).

| # | Class | Rule | Portal | Console | NetBird | Kratos / Hydra |
|---|---|---|---|---|---|---|
| 1 | **New rows, site-unique or derived ids** | ids are xid/UUID, or derived from the inputs so two sites creating the same thing write the same row | topics (id derived from org + key pattern), schemas, code examples, grants, access requests, govern nominations | invites, proposals, actions, ssh keys, operator keys, probation requests | setup keys, peers, users (xid) | identities, credentials (UUID) |
| 2 | **Changes to an owned record** | `owner_site` on the row; only the owner writes; a logged takeover moves ownership. The owner may be a site, or the org or person the record belongs to | slot claim (owner = the site whose range holds the slot), vendor and budget edits, machine enrolment; org-owned topics, envelopes and examples | proposal state, runner claims and leases (leases replicate; the `owner_site` filter on every claim stops a second site's runner minting), key reissue, dormancy release, invite and org edits | peer group membership for peers the site admitted | identity traits |
| 3 | **Revocations** | any site; append-only ledger rows; effective state = revoked if any site revoked, until a reinstatement that names the revoke events it cancels | grant revoke, org offboard, `network_admin` removal | offboard, invite revoke, operator-key revoke, mesh hold | disable/delete peer; group-membership removal and `users.role` downgrade (new ledger kinds) | identity deactivate; Kratos session and Hydra consent revoke, re-run when the ledger row arrives at each site |
| 4 | **Unique by value** | any site; a collision goes to the resolution queue | `vendors.prefix`, alias, probation hold | email, invite token, ssh key, one live operator key per person, org slug | — | email identifier (`identity_credential_identifiers`) |
| 5 | **Global derived state** | while cut off, regenerate only by updating existing ids; creates and deletes run at the hub after heal | slot pool initialisation (deterministic), router ACL render | operator-key directory | warrant ACL policies (`PUT` existing ids only while cut off; `POST`/`DELETE` at the hub after heal); `accounts.network_serial` reset by the hub above both sites after heal | `hydra_client` |

Org records (`orgs.id`, aliases, parent edges) are class 4: a slug registered at two sites
resolves through the existing org merge (`orgstore/merge.go`). The merge itself is the one
write refused while a site is cut off.

### 3.1 What each class needs in the schema

| Class | Schema change |
|---|---|
| 1 | Text ids on `topics`, `topic_schemas`, `topic_code_examples`, `org_parent_edges`; derived ids for topics, govern reconcile and the multipub pool handle |
| 2 | `owner_site` on proposals, actions, invites, slots, grants; `slot_ranges(site, lo, hi)` over the pre-cut pool, partner blocks split per site; claims by `UPDATE … WHERE owner_site = $me` with `FOR UPDATE SKIP LOCKED`; each slot claim gets an assignment id; `ownership_takeovers` audit table |
| 3 | NetBird and Kratos: the ADR 1193 ledger and guard (`ops/postgres/spock/revocation-guard/`, PR #1538), extended with kinds for group membership, user role, holds, sessions and consent; reinstatement names the revoke events it cancels, so order does not depend on clocks. Portal and console: the same ledger shape in their own databases. A revoke is keyed on the person or machine, not on row ids, and re-runs when its ledger row arrives |
| 4 | Each secondary unique index becomes the primary key or is removed; a `value_collisions` view over the tables plus `spock.exception_log` feeds the queue; cross-row rules (machine quota, last org admin, one pending action, no parent cycle) are checked locally and violations go to the same queue after heal |
| 5 | None; regeneration is a job keyed by the hub's `site_id` (`goatnet_hub_site`, ADR 1205 D1) |

Status that changes on a read or a timer (invite expiry, lease reaping) is computed when
read, and reapers act only at the owner site. Routine runtime writes on admin rows (API-key
digest backfill, ssh installer reports) move to per-site tables.

### 3.2 Data that stays local and never replicates

`audit_events`, `audit_replay_cursor`, `vendor_progress` (its foreign key to `vendors` is
dropped), `trashgoat_specs`, the three secret columns (moved to a local collect-once table),
download and publish counters, and the per-site runtime tables above. These go in a
per-site database (`portal_local`, `console_local`) outside every repset.
`vendor_topic_activity` has had no writer since `progress.go:233` and is dropped.
`govern_meta` replicates, because its marker gates a one-time migration.

---

## 4. Failure is visible

**Plain:** a write that cannot be applied at the other site is logged and shown to an
admin; a divergence between sites raises an alert.

- The ADR 1193 apply settings on every node, in `ops/postgres/spock/conf/postgresql.conf`:
  `spock.exception_behaviour = discard` (a failed row is logged, the rest of the
  transaction applies; `transdiscard` dropped whole transactions, F-574) and
  `spock.save_resolutions = on`.
- The console reads `spock.exception_log`, `spock.resolutions` and the `value_collisions`
  view into one resolution queue, with the cause and the fix for each row.
- `spock-drift-watch` (role `spock-drift-watch`, ADR 1084 D5) gains every replicated table of
  `portal`, `console`, `orgs` in `spock_drift_watch_tables`, and every site gets its peers in
  `spock_drift_watch_peers`.
- Alert on: subscription not `replicating`, empty repset, any unresolved exception
  older than 1 h, any drift-watch mismatch.

---

## 5. Engine swap: blue-green, SQLite kept four weeks

**Plain:** the engine changes first with the tables unchanged, so going back is a
straight copy. The schema changes afterwards on Postgres, where point-in-time restore
is the way back.

**Step A — engine, tables identical**

1. Postgres build of portal and console: port the 14 SQLite-only statements, the 7
   rowid orderings, the 5 `LastInsertId` sites (via `RETURNING`); replace error-string
   checks with SQLSTATE `23505`; replace connection-as-lock with transactions; delete
   `walwatch.go`, `reader.go`, `probe.go` and the `registry-wal` alerts; DSN config keys
   replace `portal_db_path`, `ONBOARD_DB_PATH`, `ORG_DB_PATH`.
2. `store-migrate import` (SQLite → Postgres, per-table row counts and hashes) and
   `store-migrate export` (Postgres → SQLite, straight copy), shipped in the console image.
   For the console, the databases are `id_console_onboard` and `id_console_orgs` on
   `postgres-a`, owned by the `id_console` role, in no Spock replication set.
3. Rehearsal on the dev goatnet: import, export, byte-compare table dumps.
4. Cut over per goatnet: dev, preprod, prod, each after the previous has soaked. The
   pin moves through `ops/scripts/rc.sh`; the copy and the switch are one driver,
   `ops/ansible/id-console-postgres-cutover.yml`
   ([runbook](../operations/id-console-postgres-cutover.md)). The console role refuses to
   start a Postgres-era image (image label `org.usareur-af.goat-id-console.store=postgres`)
   on a goatnet whose group_vars do not declare `id_admin_console_store: postgres`, or on a
   host the cutover has not marked.
5. Archive: checkpoint and `chmod 0444` the SQLite files in place, copy them read-only to a
   dated directory on the console host with a manifest (sha256, the previous image,
   retain-until), keep 28 days. The blue image stays pinned in the ledger.
6. Rollback inside 28 days: `export`, then pin the blue image
   (`ops/ansible/id-console-postgres-rollback.yml`).

**Step B — schema, on Postgres**

1. Versioned migration table in both modules; version 1 is the step A schema.
2. Migrations for §3.1 and §3.2, each expand-then-contract.
3. Invite store and peerview hold records into Postgres.
4. Bundles and schema files into `bytea` rows, so another site has them without file
   replication.
5. After 28 days on all three goatnets: delete the archives, remove `modernc.org/sqlite`
   from both `go.mod` files, repoint the 10 scripts.

---

## 6. The partition harness

**Plain:** a CI job that cuts two Spock nodes apart, runs every write path on both
sides, reconnects, and fails if anything diverged, was dropped silently, or was issued
twice.

- Two `pgedge-postgres` containers from the pinned image (`ops/postgres/spock/Dockerfile`),
  Spock subscriptions both ways; portal, console, NetBird management, Kratos, Hydra and an
  onboard-runner on each. NetBird is written by the onboard-runner through
  `goat admin onboard --native`, not by the console.
  Base: `ops/postgres/spock/tests/partition-revoke/run.sh` and its CI job `partition-revoke`
  (PR #1538), which already proves class 3 for NetBird and Kratos with `GUARD=0`/`GUARD=1`.
- Partition by disabling both subscriptions; heal by enabling.
- One scenario per class per store, plus the cross-store cases: the same email onboarded
  at both sites; a revoke at A while B reissues the key; a slot claimed at both sites;
  the same org slug registered at both sites.
- Pass: drift-watch reports equal on every table; every exception is in the queue with
  a resolution; no slot id or setup key appears for two subjects.
- Runs on every PR touching a store package, `ops/postgres/spock/`, or a write handler.

---

## 7. Accepted risks

| Risk | Mitigation |
|---|---|
| The same person or org created at two sites while cut off | Class 4 queue; admin resolves with offboard or org merge |
| `postgres-a` becomes the single dependency for admin | Precondition: F-581 disk and WAL-archive guards, live prod standby (#1953) |
| Every schema change needs coordinating across sites | Additive migrations only; coordinated-upgrade pattern |
| Clock drift on a cut-off site | No class orders by timestamp: reinstatements name the revokes they cancel, and the hub resets `network_serial` after heal; class 4 goes to a human |
| Spock fails its operational bar | `ha-multi-site.md` §17 fallback (streaming replication, one writer) |

---

## 8. Gates

Gates G2 to G4 are rows of the goatbuilder 1.0 gate, group I ([goatbuilder-v1-gate.md](goatbuilder-v1-gate.md)): 1.0 is not tagged until the partition drill has passed on two standing two-site goatnets.

| Gate | Evidence |
|---|---|
| G0 preconditions | ADR 1193 / PR #1538 merged and rolled out; F-581 guards live on all goatnets; prod standby replicating (#1953); harness green on main |
| G1 step A | Round-trip rehearsal byte-equal on the dev goatnet; three goatnets on Postgres |
| G2 step B | Migration table at the §3 schema on three goatnets; SQLite gone from main |
| G3 per class | Harness scenarios for that class green; ADR 1084 D3 lifted for that class in group_vars |
| G4 drills | Partition drill passes on a two-node dev goatnet, then on preprod |

## Related

- [`island-mode-preminted-certificates.md`](island-mode-preminted-certificates.md) — a pre-minted certificate batch for a site that holds no signing key, such as a phone (designed, not built)
- [`island-mode-write-inventory.md`](island-mode-write-inventory.md) — every write path of the four stores with its class and harness scenario, and the corrections to this doc (program step 5)
- [`portal-console-multisite-readiness.md`](portal-console-multisite-readiness.md) — the 2026-09-06 review §2.1 replaces
- [`hub-spoke-admit-split.md`](hub-spoke-admit-split.md) §2 — the write taxonomy class 3 adopts
- [`replication-consistency-observability.md`](replication-consistency-observability.md) — conflict and resolution monitoring
- `ha-multi-site.md` §6 DDL, §17 fallback
- `zenoh-storage-taxonomy-sharding.md` — slot storage
