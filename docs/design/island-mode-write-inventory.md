# Island mode write inventory: every write to the four admin stores, with its class

**Status:** Written 2026-10-29, read from code at `8591afeb7` (branch `track/island-multiwriter-plan`) and the NetBird fork `USAREUR-AF/netbird` at `274076d5f`. Every rule in §8 is Designed; none is built. Step 5 of `island-mode-program.md`. Design: [`island-mode-multi-writer.md`](island-mode-multi-writer.md). Decision: [ADR 1165](../adr/1165-island-mode-multi-writer-every-site-admits.md).

Island mode lets every site of a goatnet change admin state while cut off from the others, and
it is safe only if every write is shaped so the merge after the link returns loses nothing. The
design sorts writes into five classes, each with a rule. This document lists every place goat
writes to the portal registry, the id console, NetBird management or Kratos/Hydra, gives each
one class, names the schema change it needs and the partition-harness scenario that proves it.
126 write paths fit as follows: 13 in class 1, 26 in class 2, 17 in class 3, 13 in class 4,
21 in class 5, 5 local-only, and 31 that fit no class as the code stands. §8 names 15 shapes
that break the model: 12 cover those 31 paths, and 3 more (X2, X4, X14) break paths that are
otherwise in classes 2 and 3. Each shape has a rule that resolves it. §9 lists what the design
doc states wrongly.

---

## 1. How to read the tables

Each row is one write path: the HTTP route, CLI verb, playbook task or timer that writes, with
the function it reaches. **Class** holds exactly one value:

| Value | Meaning (design §3) |
|---|---|
| **1** | New row with a site-unique id |
| **2** | Change to a record one owner writes; others take ownership through a logged takeover |
| **3** | Revocation: any site, append-only ledger, revoked if any site revoked until an attributed reinstatement |
| **4** | Unique by value: any site; a collision goes to the resolution queue |
| **5** | Derived or declared state: regenerated from its source; the hub regenerates once after heal |
| **L** | Site-local; never replicated (design §3.2) |
| **X1-X15** | Fits no class as written; see §8 for the rule that makes it fit |

**Scenario** is the name of the partition-harness case (design §6) that proves the row:
`<store>-c<class>-<case>` for a class rule, `<store>-x<n>-<case>` for a §8 rule. §10 lists every
scenario once. "Both sides" in a scenario name means the write is made at site A and at site B
while the Spock subscriptions between them are disabled, then the subscriptions are re-enabled.

Counts per store:

| Store | 1 | 2 | 3 | 4 | 5 | L | X | Total |
|---|---|---|---|---|---|---|---|---|
| Portal (§3) | 6 | 12 | 2 | 2 | 4 | 5 | 11 | 42 |
| Console (§4) | 4 | 12 | 9 | 9 | 3 | 0 | 10 | 47 |
| NetBird, writes made by goat (§5) | 3 | 0 | 3 | 0 | 7 | 0 | 6 | 19 |
| Kratos and Hydra, outside the console (§6) | 0 | 2 | 3 | 2 | 4 | 0 | 4 | 15 |
| Scripts that write SQLite directly (§7) | 0 | 0 | 0 | 0 | 3 | 0 | 0 | 3 |
| **All** | **13** | **26** | **17** | **13** | **21** | **5** | **31** | **126** |

Eight more scripts open the SQLite files read-only (§7).

---

## 2. Who writes what

**Plain:** goat's admin writes come from five places, and three of them are not the service
that owns the store.

| Writer | Writes to | Single-writer assumption today |
|---|---|---|
| Portal (`ops/goat-zenoh-portal`) | `registry.db`; NetBird setup keys and peers (machine enrolment and revoke); router config and admin space | ADR 1084 D3; `portal-deploy.yml:40` "single-homed BY INTENT"; `assert-admin-write-host.yml` |
| Id console (`ops/goat-id-console`) | `onboard.db`, `orgs.db`, the in-memory invite map; Kratos admin API. **No NetBird write and no Hydra write** | ADR 1084 D3; `SetMaxOpenConns(1)` used as a lock (`onboardstore/store.go:45`, `orgstore/store.go:141`) |
| onboard-runner (`ops/onboard-runner`) | Nothing directly. It claims console work and runs `goat admin onboard … --native` (`ops/onboard-runner/runner.go:758`, `:554`; `offboard.go:71`) and `lifecycle-sweep.py --lane release` (`ops/onboard-runner/runner.go:448`) | `onboard-runner-deploy.yml:33`: "Two runners on one goatnet would each auto-execute the same intake queue" |
| goat-cli (`crates/goat-cli`) | NetBird through `panoscope-core/src/netbird.rs`; Kratos over SSH (`admin.rs:6463,6509`); the console machine API | No host guard |
| Playbooks, timers and scripts under `ops/` | NetBird policies, groups, DNS, users; Hydra clients; Kratos identities; two SQLite backfills | `warrant-membership.yml`, `peerview-deploy.yml`, `netbird-admin-reconcile.yml` and the ACL and DNS playbooks do not include `assert-admin-write-host.yml` |

---

## 3. Portal (`ops/goat-zenoh-portal`, `registry.db`)

All SQL is in `internal/registry`. Paths below are relative to `ops/goat-zenoh-portal/`. Routes
registered through `dualRoute` (`internal/api/server.go:585`) answer on both `/api/vendors…` and
`/api/orgs…`; the `/api/orgs` form is shown. `pubsub-apply` (`cmd/pubsub-apply`) calls these same
routes over HTTP (`internal/pubsubstate/client.go`) and is not a separate path.

The slot pool is not a set of new rows. `InitializeSlotPool` (`internal/registry/slots.go:76`)
inserts every slot once, with id `sha256(salt:k)[:32]`, and a claim is an `UPDATE` of the lowest
free row (`slots.go:229`). The per-site slot range (design §3.1) therefore makes a claim a class 2
write on a row its range owner holds, not a class 1 insert.

### 3.1 HTTP handlers

| # | Route → handler | Registry call | Tables | Today | Class | Schema change | Scenario |
|---|---|---|---|---|---|---|---|
| P1 | `POST /api/orgs` → `createVendor` `internal/api/handlers.go:299` → `onboardSlotSystem` `:443` | `AllocateSlot` / `AllocateSlotIn` `slots.go:152,170` (`allocateSlot` `:178`), `CreateVendor` `registry.go:625`, `StampSlotIdentity` `slots.go:260`, `ClearSlotCert` `:397`, `MarkBundle` `registry.go:838`, `StampSlotCert` `slots.go:372`, `SetMachineGrant` `machines.go:450` | `slots`, `vendors`, `machine_grants`, `envelopes` | Lowest free `idx` fleet-wide; alias uniqueness checked inside the tx (`slots.go:187`) | **2** | `slot_ranges(site, lo, hi)`; the claim adds `AND idx BETWEEN lo AND hi`. The `vendors` row is keyed by the anchor (class 1); the alias stays a class 4 value | `portal-c2-slot-claim-both-sides`, `portal-c4-alias-both-sides` |
| P2 | `POST /api/orgs` on a per-vendor goatnet (no slot pool) → `createVendor` `handlers.go:299-421` | `CreateVendor`, `SetVendorLabels` `labels.go:128`, `MarkBundle` | `vendors` (PK = human prefix) | Uniqueness by error string (`registry.go:635`) | **4** | SQLSTATE `23505` | `portal-c4-prefix-both-sides` |
| P3 | `PATCH /api/orgs/{vendor}` → `updateVendor` `handlers.go:1099` | `UpdateVendorDisplayName` `registry.go:761` | `vendors` | — | **2** | `vendors.owner_site` | `portal-c2-vendor-edit-owner-only` |
| P4 | `POST /api/orgs/{vendor}/sponsorship` → `setVendorSponsorship` `handlers.go:1159` | `UpdateVendorSponsorship` `registry.go:782` | `vendors` | — | **2** | as P3 | `portal-c2-vendor-edit-owner-only` |
| P5 | `PUT /api/orgs/{vendor}/labels` → `handleSetLabels` `labels.go:73` | `SetVendorLabels` `registry/labels.go:128` | `vendors` | — | **2** | as P3 | `portal-c2-vendor-edit-owner-only` |
| P6 | `POST /api/orgs/{vendor}/rotate` → `rotateVendor` `handlers.go:732` → `rotateAnchor` `:744`; `POST /api/orgs/{vendor}/rekey` → `rekeyVendor` `rekey.go:26`; `POST /api/orgs/rotate` → `rotateByIdentity` `rotate_by_identity.go:29` | `UpdateVendorAPIKey` `registry.go:809`, `MarkBundle`, `StampSlotCert` | `vendors.api_key_hash`, `api_key_sha256`, bundle columns; `slots` | The key is a column overwritten in place | **X1** | `vendor_api_keys` insert-only; retiring a key is a ledger row | `portal-x1-rotate-at-a-edit-at-b` |
| P7 | `POST /api/orgs/collect` → `collectFabricCredential` `collect.go:70` | `UpdateVendorAPIKey` `:171`, `MarkBundle` `:183`, `StampSlotCert` `:187`, `StampSlotCollected` `:215` | `vendors`, `slots` | Collect-once: reads `collected_at`, then stamps it | **2** | Refused off the range owner unless taken over | `portal-c2-collect-once-both-sides` |
| P8 | `DELETE /api/orgs/{vendor}` → `deleteVendor` `handlers.go:1214` | `DeleteVendor` `registry.go:1021` (hard delete; `ON DELETE CASCADE` to `topics`, `topic_schemas`, `topic_code_examples`, `envelopes`, `vendor_progress`), `RevokeAssignedSlot` `slots.go:435`, ACL `RemoveVendor` `internal/acl/acl.go:218` | `vendors` + 5 cascaded, `slots` | Hard delete; the slot returns to the pool in place | **3** | Ledger row `portal.vendor`; rows tombstoned, purged locally after heal; slot reuse per X2 | `portal-c3-offboard-at-a-edit-at-b` |
| P9 | `POST /api/machines/enroll` → `enrollMachine` `machine_enroll.go:361`; `POST /api/machines/enroll-bulk` → `enrollMachinesBulk` `machine_enroll_bulk.go:57` | `AllocateSlot` `machine_enroll.go:236`, `RecordMachineEnrollment` `:250`, `CreateVendor` `:295`, `MarkBundle` `:309`, `StampSlotCert` `:312`; NetBird `MintScoped` `:216` | `slots`, `vendors`; NetBird `setup_keys` | Custodian quota re-checked inside the claim (`:234`) | **2** | as P1; the quota is X3 | `portal-c2-machine-enroll-both-sides`, `portal-x3-quota-both-sides` |
| P10 | `POST /api/machines/{anchor}/mesh-key` → `machineMeshKey` `machine_mesh_key.go:48` | NetBird `POST /api/setup-keys` (`internal/netbirdkeys/netbirdkeys.go:251`) | NetBird `setup_keys` | — | **1** | — | `netbird-c1-setup-key-both-sides` |
| P11 | `DELETE /api/machines/{anchor}` → `machineRevoke` `machine_revoke.go:71` | `CutMachine` `netbirdkeys.go:353` (lists keys and peers, deletes name matches), `DeleteVendor` `:150`, `RevokeAssignedSlot` `:159` | NetBird `setup_keys`, `peers`, `group_peers`; `vendors`, `slots` | Revokes what this site can list at that moment | **X5** | Ledger row keyed on the machine handle | `portal-x5-revoke-at-a-mint-at-b` |
| P12 | `POST /api/machines/grant` → `grantMachineQuota` `machine_grant.go:52` | `SetMachineGrant` `:91`, `SetMachineQuota` `:107,134`, `SetNetworkAdmin` `:117` (`registry/machines.go:420,450,505`) | `machine_grants` (PK `custodian`) | One row holds quota, affiliation, site and the `network_admin` privilege | **X6** | `owner_site` for quota fields; `network_admin` grant and removal as ledger rows | `portal-x6-network-admin-remove-at-a-edit-at-b` |
| P13 | `POST /api/machines/affiliation/repair` → `repairMachineAffiliation` `affiliation_repair.go:10` | `RepairAffiliations` `registry/affiliation_repair.go:140` | `slots`, `machine_grants`, `audit_events` | Bulk rewrite of every machine | **5** | — (hub only, after heal) | `portal-c5-affiliation-repair-hub-only` |
| P14 | `PUT /api/slots/{anchor}/identity` → `handlePutSlotIdentity` `budgets.go:193` | `StampSlotIdentity` `slots.go:260` | `slots.identity_id` | — | **2** | slot owned by range | `portal-c2-slot-claim-both-sides` |
| P15 | `PUT /api/slots/{anchor}/custody` → `handlePutSlotCustody` `slot_custody.go:53` | `RecordMachineEnrollment` `:89` | `slots` | — | **2** | as P14 | `portal-c2-slot-claim-both-sides` |
| P16 | `PUT /api/slots/{anchor}/lifecycle` → `handlePutSlotLifecycle` `budgets.go:162` | `SetSlotLifecycleClass` `registry/budgets.go:121` | `slots.lifecycle_class` | — | **2** | as P14 | `portal-c2-slot-claim-both-sides` |
| P17 | `PUT` and `DELETE /api/budget-grants/{owner}` → `handlePutBudgetGrant` `budgets.go:122`, `handleDeleteBudgetGrant` `:144` | `UpsertTopicBudget` `registry/budgets.go:58`, `DeleteTopicBudget` `:91` | `topic_budgets` (PK `owner`) | Upsert; hard delete | **2** | `owner_site`; delete becomes a row state | `portal-c2-budget-edit-owner-only` |
| P18 | `PUT /api/orgs/{vendor}/envelope` → `putEnvelope` `envelope.go:62` | `UpsertEnvelope` `registry/envelope.go:202`, `EnsureDefaultEnvelope` `:183` (`INSERT OR IGNORE` `:187`) | `envelopes` (PK `anchor`) | Written with the org's own key | **X7** | `ON CONFLICT DO NOTHING` for the default | `portal-x7-subject-edit-either-side` |
| P19 | `POST /api/orgs/{vendor}/topics` → `createTopic` `handlers.go:903`; `POST …/topics/import` → `importTopics` `bulk.go:141` (`:258`) | `CreateTopic` `registry.go:896` | `topics` (INTEGER AUTOINCREMENT; `UNIQUE(vendor_prefix, key_pattern)`) | `LastInsertId` `:911`; error-string check `:906` | **1** | `topics.id` = hash of `(vendor_prefix, key_pattern)`, so one topic registered at two sites is one row | `portal-c1-topic-create-both-sides` |
| P20 | `PATCH …/topics/{id}` → `updateTopic` `handlers.go:964`; import `bulk.go:272`; schema upload `schemas.go:288` | `UpdateTopic` `registry.go:941` | `topics` | Written with the org's own key | **X7** | — | `portal-x7-subject-edit-either-side` |
| P21 | `DELETE …/topics/{id}` → `deleteOrRetireTopic` `handlers.go:1306`; import `bulk.go:282` | `RetireTopic` `registry.go:956`, or `DeleteTopic` `:970` (hard delete, cascades) | `topics`, `topic_schemas`, `topic_code_examples` | Hard delete when never published | **X7** | `DeleteTopic` removed; retire only | `portal-x7-subject-edit-either-side` |
| P22 | `POST …/topics/{id}/schemas` → `uploadSchema` `schemas.go:81` | `InsertSchema` `registry/schemas.go:84` | `topic_schemas` (INTEGER; `UNIQUE(topic_id, version)`); file at `<SchemaDir>/<topic_id>/` | `LastInsertId` `schemas.go:108`; file on local disk | **1** | `(topic_id, version)` as the key; file in `bytea` (design §5 B4) | `portal-c1-schema-upload-both-sides` |
| P23 | `PUT` and `DELETE …/topics/{id}/examples/{lang}` → `putCodeExample` `code_examples.go:52`, `deleteCodeExample` `:173` | `UpsertCodeExample` `registry/code_examples.go:62`, `DeleteCodeExample` `:121` | `topic_code_examples` (`UNIQUE(topic_id, language)`) | Upsert on the pair | **X7** | `(topic_id, language)` as the PK | `portal-x7-subject-edit-either-side` |
| P24 | `POST /api/access/requests` → `handleCreateAccessRequest` `data_access.go:152` | `CreateRequest` `requests.go:62` | `data_access_requests` (`dar-<hex>`) | — | **1** | — | `portal-c1-access-request-both-sides` |
| P25 | `POST /api/access/requests/{id}/approve` → `data_access.go:217`; `…/decline` → `:263` | `CreateGrant` `:241`, `DecideRequest` `:245,274` (`requests.go:145`) | `grants`, `data_access_requests` | `WHERE status = 'pending'` | **2** | `owner_site` on requests; grant id derived from the request id | `portal-c2-decide-off-owner-refused` |
| P26 | `POST /api/access/grants` → `handleCreateGrant` `data_access.go:318`; `POST /api/access/co-publish` → `:502` | `CreateGrant` `grants.go:143` | `grants` (`grant-<hex>`) | — | **1** | — | `portal-c1-grant-both-sides` |
| P27 | `POST /api/access/grants/{id}/revoke` → `handleRevokeGrant` `data_access.go:647` | `RevokeGrant` `grants.go:284` | `grants.status` | In-place `UPDATE`; nothing else writes a grant row after insert | **3** | Ledger row `portal.grant` (design §3.1) | `portal-c3-grant-revoke-both-sides` |
| P28 | `POST /api/access/multipub-topics` → `handleCreateMultipubTopic` `data_access.go:399` | `CreateMultipubTopic` `multipub_topics.go:161` → `EnsureOrgMultipubPool` `:114` | `multipub_topics`, `org_multipub_pools` (random `pool_handle`, `org UNIQUE`) | First topic of an org mints a random handle | **X8** | `pool_handle` = hash(salt, org) | `portal-x8-first-multipub-both-sides` |
| P29 | `POST /api/access/govern-admins` → `handleNominateGovernAdmin` `data_govern_admins.go:191` | `NominateGovernAdmin` `govern.go:56` | `govern_admins` (`gadm-<hex>`) | — | **1** | — | `portal-c1-govern-nominate-both-sides` |
| P30 | `POST /api/access/govern-admins/{id}/remove` → `handleRemoveGovernAdmin` `data_govern_admins.go:278` | `RemoveGovernAdmin` `govern.go:132` | `govern_admins` | Refuses removing an org's last active admin by a count (`govern.go:142`) | **X3** | Ledger row; zero-admin check in `value_collisions` | `portal-x3-remove-last-admins-both-sides` |
| P31 | `POST /api/access/probation` → `handleProbationHold` `access_probation.go:50` | `HoldProbation` `probation.go:91` | `probation` (partial unique on `ref` while open, `schema.sql:546`) | — | **4** | `value_collisions` covers the partial index | `portal-c4-probation-hold-both-sides` |
| P32 | `POST /api/access/probation/{id}/decide` → `handleProbationDecide` `access_probation.go:84` | `DecideProbation` `probation.go:178` | `probation.disposition` | `WHERE disposition = 'probation'` | **2** | `owner_site` | `portal-c2-decide-off-owner-refused` |
| P33 | `POST`, `PATCH`, `DELETE /api/admin/trashgoat/specs…` and `/activate`, `/pause`, `/retire`, `/cleanse` → `trashgoat.go:158,203,269,298-317` | `trashgoat.go:90-288` | `trashgoat_specs` | — | **L** | `portal_local` (design §3.2) | — |
| P34 | `GET /api/org-bundle` → `handleVendorBundle` `handlers.go:1401` | `RecordBundleDownload` `registry.go:855` | `vendors.bundle_download_count`, `bundle_downloaded_at` | A counter on the replicated `vendors` row | **L** | moves to `portal_local` (design §3.2); until it does, it has the X9 shape | `portal-x9-runtime-write-vs-rekey` |
| P35 | Every API call authenticated by bcrypt → `resolvePrincipal` `internal/api/auth.go:241` | `SetVendorAPIKeySHA256` `registry.go:713` | `vendors.api_key_sha256` | Backfills the digest at whichever site the caller reached | **X9** | Digest lives with the key in `vendor_api_keys` (X1) | `portal-x9-runtime-write-vs-rekey` |

### 3.2 Boot, timers and side effects

| # | Trigger | Call | Tables | Class | Change | Scenario |
|---|---|---|---|---|---|---|
| P36 | Boot, `cmd/portal/main.go:132` | `InitializeSlotPool` `slots.go:76` | `slots` (identical rows at every site) | **5** | Pool baked once at the hub; other sites receive it | `portal-c5-slot-pool-init-once` |
| P37 | Boot, `registry.Open` `registry.go:158` | `normalizeMachineCustodians` `machines.go:36`, `backfillPublishCounts` `registry.go:436`, `addColumnIfMissing` `:461` | `slots`, `machine_grants`, `vendor_progress`; DDL | **5** | Data backfills move into the versioned migration table, run once at the hub through `coordinated-upgrade.yml` | `portal-c5-boot-migration-once` |
| P38 | Timer, `RunGovernReconcile` `govern_seed.go:198` (5 min, `cmd/portal/main.go:293`) → `seedExistingOrgsAllMembers` `:151`, `reconcileFirstMember` `:178` | `NominateGovernAdmin` `:126`, `GovernMetaSet` `:172` | `govern_admins`, `govern_meta` | **X8** | `gadm-` id = hash(org, identity); `govern_meta` replicates (§9 D5) | `portal-x8-govern-reconcile-both-sides` |
| P39 | Timer, `netbirdpoll.Poll` `internal/netbirdpoll/netbirdpoll.go:185` | `MarkL2` `progress.go:26` | `vendor_progress` | **L** | `portal_local`; FK to `vendors` dropped (§9 D6) | — |
| P40 | Timer, `audittail.Tailer.Run` `internal/audittail/audittail.go:91` | `ApplyAuditObject` `progress.go:245` | `vendor_progress`, `audit_replay_cursor` | **L** | as P39 | — |
| P41 | Every handler | `LogAuditEvent` `registry.go:1037` | `audit_events` | **L** | `portal_local` | — |
| P42 | Org create and delete (`handlers.go:1462`, `:1278`, `:677`) | ACL `Render` `internal/acl/acl.go:113`, `RemoveVendor` `:218`, `AdminREST.AddStorage` `internal/acl/adminrest.go:142` | Router config file + `SIGHUP`; router admin space (not a store) | **5** | — (each site renders from its own registry) | `portal-c5-acl-render-after-heal` |

---

## 4. Id console (`ops/goat-id-console`)

Paths are relative to `ops/goat-id-console/`. Routes are registered in
`internal/handlers/handlers.go:294-581`. Gates: **m** machine API key, **og** onboarding admin,
**ag** admin, **ss** self-service, **pub** none. The console reads NetBird
(`meshstate.go:307,373,407`, `enrollwatch.go:252,275`) and Hydra (`hydra/client.go:62`) and writes
neither; mesh changes for a person go through `onboard_actions` and proposal claims that the
onboard-runner executes (§5).

### 4.1 `onboard.db`

| # | Route → store call | Tables | Today | Class | Schema change | Scenario |
|---|---|---|---|---|---|---|
| C1 | `POST /api/onboard/intake` m (`onboard_api.go:1033`), `POST /api/onboard/intake/batch` m (`onboard_batch.go:103`) → `CreateProposal` `onboardstore/proposals.go:258` | `onboard_proposals` (`prop-<hex>`) | No email uniqueness in the DB | **4** | `value_collisions`: two live proposals with one `lower(email)` | `console-c4-same-email-both-sides` |
| C2 | `POST /public/onboard/submit` pub (`onboard_public.go:578`) → `ConsumeInvite` `invites.go:341` | `onboard_invites`, `onboard_proposals` | Not one transaction; the rollback relies on `SetMaxOpenConns(1)` (`invites.go:394`); email check-then-insert at `onboard_public.go:493` | **4** | One transaction; the token is single-use by value | `console-c4-invite-token-both-sides` |
| C3 | `POST /api/onboard/register` m (`onboard_register.go:75`) → `RegisterMintedProposal` `register.go:35` | `onboard_proposals` | Check-then-insert on `lower(email)` or `(handle, env)`; two statements | **4** | as C1 | `console-c4-same-email-both-sides` |
| C4 | `POST /onboarding/{id}/{action}` og, `POST /api/onboard/proposals/{id}/{sponsor-approve,sponsor-reject,approve,reject}` m → `decideProposal` `onboarding.go:1804-1881` → `RecordSponsorApproval` `proposals.go:421`, `DecideProposal` `:438` | `onboard_proposals.status` | Status checked in the handler; the `UPDATE … WHERE id = ?` is unguarded | **2** | `owner_site`; each transition becomes `WHERE id = ? AND status = ? AND owner_site = current_site()` | `console-c2-decide-off-owner-refused` |
| C5 | `POST /onboarding/{id}/fields` og, `/api/onboard/proposals/{id}/fields` m, `/onboarding/{id}/org-attach`, `/org-new` og → `UpdateProposalManifest` `proposals.go:675`, `SetProposalOrgSource` `:702`, `UpdateDraftNote` `drafts.go:43`, `CompleteProposal` `drafts.go:15` | `onboard_proposals.manifest_json` | Read-modify-write in the handler; the guard checks status only | **2** | as C4 | `console-c2-decide-off-owner-refused` |
| C6 | `POST /github` ss, `/onboarding/{id}/github` og, `/api/onboard/proposals/{id}/github` m, `/repo-access` m → `SetProposalGitHub` `github.go:30`, `SetProposalRepoGrants` `proposals.go:607`, `SetProposalRepoOverrides` `:627` | `onboard_proposals` | unguarded | **2** | as C4 | `console-c2-decide-off-owner-refused` |
| C7 | `POST /api/onboard/proposals/{id}/claim` m (`onboard_api.go:488`) → `ClaimProposal` `proposals.go:650` | `onboard_proposals.status`, `claimed_at` | CAS `approved → onboarding` | **2** | The claim adds `AND owner_site = current_site()`; without it a runner at each site claims the same proposal and both mint a setup key and a Kratos identity | `console-c2-claim-both-runners` |
| C8 | `POST …/{id}/onboarded` m (`onboard_api.go:464`) → `SetProposalStatus` `:552`; `POST …/{id}/result` m (`:617`), `POST /onboarding/{id}/welcome/resend` og (`onboard_welcome.go:235`) → `SetProposalResult` `:571` | `onboard_proposals` (`status`, `setup_key`, `handset_key`, `bundle_path`) | unguarded | **2** | `setup_key`, `handset_key` to `console_local` (design §3.2) | `console-c2-claim-both-runners` |
| C9 | `POST /api/onboard/proposals/{id}/archive` m → `ArchiveProposal` `:729`; restore and retry (`onboarding.go:1411,1785`, `onboard_api.go:1201,1225`) → `reenterApproved` `:779` | `onboard_proposals` | guarded | **2** | as C4 | `console-c2-decide-off-owner-refused` |
| C10 | `POST /onboarding/{id}/offboard` og, `/api/onboard/proposals/{id}/offboard` m → `MarkOffboardedFrom` `proposals.go:465`; `POST /onboarding/{id}/offboard-cut` og → `RequestOffboard` `:499` | `onboard_proposals`, `onboard_actions` | guarded tx | **3** | Ledger row `console.person` keyed on the Kratos identity id | `cross-c3-offboard-at-a-reissue-at-b` |
| C11 | `POST /api/onboard/proposals/{id}/offboard-claim` m (`onboard_api.go:524`) → `ClaimOffboard` `:521` | `onboard_proposals` | CAS + lease | **3** | Any site's runner may run an offboard: the NetBird and Kratos revokes are idempotent and land in the ADR 1193 ledgers. The claim moves to `console_local.offboard_claims(proposal_id, site)` | `console-c3-offboard-runs-at-both-sites` |
| C12 | `POST /onboarding/invite` og (`onboarding.go:939`), `POST /api/onboard/invite` m (`onboard_api.go:823`) → `MintInvite` `invites.go:163` | `onboard_invites` (`inv-<hex>`) | — | **1** | — | `console-c1-invite-mint-both-sides` |
| C13 | `…/invite/email/verify`, `/email/send`, `/email/eml` og → `AddressInvite` `invites.go:274`, `RecordInviteSend` `:290` | `onboard_invites` | unguarded | **2** | `owner_site` | `console-c2-invite-edit-owner-only` |
| C14 | `POST /onboarding/invite/{id}/revoke` og, `/api/onboard/invites/{id}/revoke` m → `RevokeInvite` `invites.go:312` | `onboard_invites.status` | guarded | **3** | Ledger row `console.invite` | `console-c3-invite-revoke-vs-consume` |
| C15 | `GET /onboarding`, `GET /public/onboard/preview`, `GET /api/onboard/invites/{id}` → `lazyExpire` `invites.go:409`; hourly timer `cmd/console/main.go:192` → `ArchiveStaleInvites` `:548` | `onboard_invites.status` | A read flips status at whichever site served it | **X10** | Expiry computed at read; archive only where `owner_site = current_site()` | `console-x10-expiry-is-derived` |
| C16 | `POST /onboarding/{id}/handset-key/reissue`, `/device-key/reissue` og → `queueFollowUp` `onboard_actions.go:92` → `EnqueueActionWithParams` `actions.go:171` | `onboard_actions` (`act-<hex>`) | One pending action per proposal by `COUNT` inside a tx; no index | **X3** | Partial unique index `(proposal_id, kind) WHERE status IN ('queued','claimed')`, reported through `value_collisions` | `console-x3-two-reissues-both-sides` |
| C17 | `POST /holds/{peer}/release` ag (`hold_release.go:260`) → `EnqueueHoldAction` `actions.go:110` | `onboard_actions` | `COUNT` on `json_extract` of the peer and hold | **3** | A release is a `reinstate` row naming the hold it ends (X14) | `cross-c3-hold-at-a-release-at-b` |
| C18 | `POST /api/onboard/actions/{id}/claim` m (`onboard_actions.go:148`) → `ClaimAction` `actions.go:283`; `POST …/{id}/result` m (`:274,293,316`) → `SetReleaseActionResult` `:343`, `BeginReissueDelivery` `reissue_lifecycle.go:50`, `RecordActionDelivery` `:118` | `onboard_actions`, `onboard_proposals`, `probation_reactivation_requests` | CAS on `claim_token`; `finishReissue` writes first to take the SQLite writer lock (`reissue_lifecycle.go:68`) | **2** | The claim adds `owner_site = current_site()` | `console-c2-claim-both-runners` |
| C19 | Timer every minute `cmd/console/main.go:238` → `RequeueStaleClaims` `proposals.go:960`; on `GET /api/onboard/actions` and every `EnqueueHoldAction` → `requeueStaleReleaseClaimsAt` `actions.go:251` | `onboard_proposals`, `onboard_actions` | Timer and read path at every site | **X10** | Acts only on rows with `owner_site = current_site()` | `console-x10-lease-reaper-owner-only` |
| C20 | `enrollwatch` timer `cmd/console/main.go:168` → `MarkEnrolled` `proposals.go:885`; boot sweep `cmd/console/main.go:135` → `CompleteProposal` | `onboard_proposals` | Every site | **2** | Filter on `owner_site` | `console-c2-decide-off-owner-refused` |
| C21 | `POST /__goat/probation/request` pub (`probation.go:152`) → `RequestReactivation` `probation.go:36` | `probation_reactivation_requests` (`UNIQUE(peer_id, hold_entered_at)`) | Upsert on the pair | **1** | The pair becomes the PK, so both sites write one row | `console-c1-reactivation-request-both-sides` |
| C22 | `POST /reactivation-requests/{id}/decline` ag → `DeclineReactivationRequest` `probation.go:117`; `GET /reactivation-requests` ag → `CloseReactivationRequest` `:134` | `probation_reactivation_requests` | Close runs on a GET | **X10** | Close derived from the hold state; decline is class 2 by `owner_site` | `console-x10-expiry-is-derived` |
| C23 | `POST /ssh-key` ss, `POST /api/onboard/ssh-keys` m → `SubmitSSHKey` `sshkeys.go:166` | `ssh_keys` (`UNIQUE(handle, key_body)`) | Check-then-insert | **4** | `(handle, key_body)` as the key | `console-c4-ssh-key-both-sides` |
| C24 | `POST /ssh-keys/{id}/{approve,reject}` og, `/api/onboard/ssh-keys/{id}/…` m → `setSSHKeyState` `sshkeys.go:298`; `…/installed` → `MarkSSHKeyInstalled` `:334`; `…/skip-report` → `RecordSSHKeySkip` `:373` | `ssh_keys` | Installer reports arrive from hosts at every site and write the same row a revoke writes | **X9** | Reports move to `console_local.ssh_key_installs(key_id, host)`; approve and reject are class 2 | `console-x9-installer-report-vs-revoke` |
| C25 | `…/ssh-keys/{id}/revoke` → `RevokeSSHKey` `sshkeys.go:347` | `ssh_keys.status` | guarded | **3** | Ledger row `console.ssh_key` | `console-x9-installer-report-vs-revoke` |
| C26 | `POST /operator-key` ss, `POST /api/onboard/operator-keys` m → `RequestOperatorKey` `operatorkeys.go:259` | `operator_keys` (partial unique `(handle, env)` while live) | Raw constraint error | **4** | `value_collisions` | `console-c4-operator-key-both-sides` |
| C27 | `POST /operator-keys/{id}/approve` ag, `/api/onboard/operator-keys/{id}/approve` m → `ApproveOperatorKey` `operatorkeys.go:306`; reject `:335`; `GET /operator-key/collect/{id}` ss, `GET /api/onboard/operator-keys/{id}/collect` m → `CollectOperatorKey` `:360` | `operator_keys` (`status`, `secret`, `fingerprint`, `delivered_at`) | No status guard on the `UPDATE`; collect-once is check-then-act on a GET | **2** | `owner_site`; `secret` to `console_local` (design §3.2) | `console-c2-operator-key-approve-owner-only` |
| C28 | `POST /operator-keys/{id}/revoke` ag, `/api/onboard/operator-keys/{id}/revoke` m → `RevokeOperatorKey` `operatorkeys.go:386` | `operator_keys.status` | unguarded | **3** | Ledger row `console.operator_key` | `console-c3-operator-key-revoke-both-sides` |
| C29 | `opkeys.Dir.Write` / `Remove` (`operatorkeys_api.go:233,249`, `operatorkeys_page.go:230,251`); `POST /api/onboard/operator-keys/reconcile` (`operatorkeys_api.go:319`) | Key directory on the host, read by the portal | Written only at the site that approved | **5** | Each site reconciles its key directory from the replicated rows on a timer | `console-c5-operator-key-dir-converges` |
| C30 | Boot, `onboardstore.Open` `store.go:39` (`backfillOrgSource` `:133`) | `onboard_proposals` | Every start, every site | **5** | Versioned migration, hub only | `portal-c5-boot-migration-once` |

### 4.2 `orgs.db`

| # | Route → store call | Tables | Today | Class | Schema change | Scenario |
|---|---|---|---|---|---|---|
| C31 | `POST /api/org-registry/orgs` m (`org_registry.go:174`), `POST /onboarding/{id}/org-new` og (`org_match_review.go:476`) → `CreateOrg` `orgstore/store.go:303` | `orgs` (PK = slug) | Uniqueness by error string (`store.go:361`); alias-namespace check not atomic | **4** | SQLSTATE `23505`; resolved by org merge | `console-c4-org-slug-both-sides` |
| C32 | `POST /api/org-registry/orgs/{id}/update` m → `UpdateOrg` `store.go:640` | `orgs` | in place | **2** | `orgs.owner_site` | `console-c2-org-edit-owner-only` |
| C33 | `POST /api/org-registry/orgs/{id}/parent` m (`org_registry.go:236`), create, org-new → `SetParent` `store.go:552`, `ClearParent` `:604` | `org_parent_edges` (INTEGER AUTOINCREMENT; partial unique "one open edge per org") | Cycle check outside the tx | **X3** | Text id (design §3.1); one open edge is a class 4 value; a cycle check in `value_collisions` | `console-x3-parent-cycle-both-sides` |
| C34 | `POST /api/org-registry/orgs/{id}/aliases` m, org-attach → `AddAlias` `store.go:685` | `org_aliases` (PK = alias) | Error-string check (`:709`) | **4** | SQLSTATE `23505` | `console-c4-org-slug-both-sides` |
| C35 | `POST /api/org-registry/orgs/{id}/merge` m → `MergeOrg` `merge.go:28`; `…/unmerge` → `UnmergeOrg` `merge.go:131` | `orgs`, `org_aliases` (unmerge deletes), `org_parent_edges` | Status, kind and nation read outside the tx | **X12** | Refused while any `orgs` subscription is not `replicating` | `console-x12-merge-refused-while-cut-off` |
| C36 | `org_registry.go:198`, `org_match_review.go:502` → `RecordNegativePair` `match.go:212` | `org_match_overrides` (PK `(name_norm, org_id)`) | `ON CONFLICT DO NOTHING` | **1** | — | `console-c1-negative-pair-both-sides` |
| C37 | Boot, `orgstore.Open` `store.go:135` (`backfillNationAlpha3` `:198`, `backfillSectorFromKind` `:245`) | `orgs` | Every start | **5** | Versioned migration, hub only | `portal-c5-boot-migration-once` |

### 4.3 In-memory invite store (`internal/store/invites.go`)

| # | Route → method | Today | Class | Change | Scenario |
|---|---|---|---|---|---|
| C38 | `POST /invites` ag (`handlers.go:2371`) → `Create` `:70` | Process map, lost on restart | **1** | Postgres table (design §5 B3) | `console-c1-invite-mint-both-sides` |
| C39 | `POST /invites/{token}/accept` pub (`candidate_invites.go:81`) → `GetForAccept`, Kratos `CreateIdentity`, recovery mint, then `MarkUsed` `:139` | `MarkUsed` runs after the identity exists; its failure is only logged | **4** | Mark used first, in the same transaction as the read | `console-c4-invite-token-both-sides` |
| C40 | `POST /invites/{token}/revoke` ag (`handlers.go:2404`) → `Revoke` `:160` | in memory | **3** | Ledger row `console.invite` | `console-c3-invite-revoke-vs-consume` |

### 4.4 Kratos admin writes from the console (`internal/kratos/client.go`)

Replicated Kratos tables are listed in §6.

| # | Call | Triggered by | Tables | Class | Change | Scenario |
|---|---|---|---|---|---|---|
| C41 | `CreateIdentity` `:467`: `POST /admin/identities` | `POST /users/new` ag (`handlers.go:2091`), invite accept (`candidate_invites.go:69`) | `identities`, `identity_credentials`, `identity_credential_identifiers`, `identity_verifiable_addresses`, `identity_recovery_addresses` | **4** | `value_collisions` over `identity_credential_identifiers.identifier` | `kratos-c4-same-email-both-sides` |
| C42 | `MintRecoveryLink` `:657`: `POST /admin/recovery/code` (fallbacks `/admin/recovery/link/code/admin`, `/admin/recovery/link`) | `POST /users/new`, `POST /users/{id}/recovery-link` ag, invite accept, `GET /onboarding/{id}/welcome` og, welcome resend, runner result | `selfservice_recovery_flows`, `identity_recovery_tokens` / `codes` (not replicated) | **X13** | Link names the issuing site's host; the welcome GET stops minting | `kratos-x13-recovery-link-other-site` |
| C43 | `PatchGroups` `:385`: `PATCH /admin/identities/{id}` with a `test` op on `metadata_public` | `POST /users/{id}/groups` ag, `POST /api/users/groups` m (`group_change.go:119`) | `identities.metadata_public` | **X6** | Group removal recorded as ledger kind `kratos.identity.group`, guarded | `kratos-x6-group-remove-at-a-edit-at-b` |
| C44 | `PatchState` `:440`: `PATCH /admin/identities/{id}` `/state` | `POST /users/{id}/state` ag (`handlers.go:2242`), `POST /api/users/state` m (`users_api.go:225`) | `identities.state` | **3** | `inactive` is ADR 1193 `kratos.identity`; `active` must call `goat_reinstate` | `kratos-c3-deactivate-at-a-edit-at-b` |
| C45 | `DeleteIdentity` `:504`: `DELETE /admin/identities/{id}` | `POST /users/{id}/delete` ag (`handlers.go:2007`) | `identities` + cascades | **3** | ADR 1193 `kratos.identity.deleted` | `kratos-c3-delete-at-a-edit-at-b` |
| C46 | `DeleteIdentityCredential` `:517`: `DELETE /admin/identities/{id}/credentials/{totp,webauthn,lookup_secret}` | `POST /users/{id}/reset-2fa` ag (`reset_2fa.go:67`) | `identity_credentials` | **2** | Owner is `metadata_admin.owner_site` on the identity (ADR 1205 D5); the hub's edits go as a directive carried out by the owner (D3) | `kratos-c2-reset-2fa-owner-only` |
| C47 | `RevokeIdentitySessions` `:633`: `DELETE /admin/identities/{id}/sessions` | `POST /users/{id}/revoke-sessions` ag (`handlers.go:1964`), after a group change (`group_change.go:126`) | `sessions` (not replicated) | **X11** | Ledger kind `kratos.sessions` | `kratos-x11-session-revoke-reaches-b` |

The console also writes three systems outside the four stores, each a single instance reached
across sites: portal routes (P6, P7, P14), Kanidm `GET /v1/person/{h}/_credential/_update_intent/{ttl}`,
which creates an intent token (`kanidmdoor/client.go:171`, from `POST /setup-code` ss), and GitHub
`PUT /repos/{o}/{r}/collaborators/{u}` (`repoaccess.go:370`). A cut-off site cannot reach them;
they belong with the outbound effects of program step 7.

---

## 5. NetBird management writes made by goat

Every table these paths write is in the 20-table repset `spock_mesh_netbird_mgmt_tables`
(`ops/ansible/roles/spock-mesh-peering/defaults/main.yml:119-139`) except `setup_keys_audit`, a
goat-made table written by two manual scripts. NetBird's activity events go to its separate
events database, which is not replicated. The API-to-table mapping is
read from the fork (`management/server/group.go:153-166`, `management/server/policy.go:86,240`, `management/server/peer.go:484,784`,
`management/server/user.go:675`, `management/server/nameserver.go:76,127`, `management/server/store/sql_store.go:3378`).

| API call | Tables written | Bumps `accounts.network_serial` |
|---|---|---|
| `POST /api/setup-keys`; `PUT` (only `auto_groups`, `revoked`, `updated_at`); `DELETE` | `setup_keys` | no |
| `PUT /api/groups/{id}` | `groups` (name, `resources` JSON); `group_peers` insert/delete of the diff against the stored row | yes |
| `POST /api/groups`; `DELETE` | `groups`, `group_peers` | yes |
| `POST`, `PUT`, `DELETE /api/policies[/{id}]` | `policies`, `policy_rules` (sources, destinations, ports as JSON) | yes, unless unchanged |
| `DELETE /api/peers/{id}` | `group_peers`, `policies` and `policy_rules` naming the peer, `peers` | yes |
| `PUT /api/users/{id}` | `users` (role, blocked, `auto_groups` JSON); `peers` when blocked; `group_peers` when groups propagate | when peers change |
| `POST /api/users`, `POST /api/users/{id}/tokens` | `users`, `personal_access_tokens` | no |
| `POST`, `PUT /api/dns/nameservers[/{id}]` | `name_server_groups` (`groups` list replaced whole) | yes |
| Client enrolment (gRPC Login with a setup key) | `peers` insert, `group_peers`, `users.last_login`, `setup_keys.used_times` | yes |

Every goat group-membership write is a read-modify-write of the whole peer list
(`PUT /api/groups/{id}`) with no `If-Match`. The server applies only the diff, so a PUT built from
a stale list removes peers another writer added.

| # | Surface (file:line) | Trigger | Call | Class | Change | Scenario |
|---|---|---|---|---|---|---|
| N1 | `crates/goat-cli/src/cmd/admin.rs:5570` `provision_netbird_native`, `:5757` `mint_handset_key`; `ops/ansible/operator-onboard.yml:1049`; `ops/scripts/replace-single-use-setup-keys.sh:158`; `ops/scripts/rosenpass_revoke_and_reinvite.py:257` | `goat admin onboard <h> --native` (run by the onboard-runner `ops/onboard-runner/runner.go:758`); playbook; manual | `POST /api/setup-keys` | **1** | — | `netbird-c1-setup-key-both-sides` |
| N2 | `admin.rs:6001` `reissue_handset_key_native`, `:6245,6251` `reissue_device_key_native` | `--reissue-{handset,device}-key` (runner `ops/onboard-runner/runner.go:554`, from console C16/C18) | `PUT /api/setup-keys/{id}` revoked, then `POST` | **1** | The new key is a new row; the old key's revoke is N3. A reissue at B after an offboard at A is X5 | `cross-c3-offboard-at-a-reissue-at-b` |
| N3 | `admin.rs:5664` `revoke_keys` (after a failed mint); `ops/scripts/lifecycle-sweep.py:1230` (purge lane, also empties `auto_groups`); `ops/ansible/scripts/setup-key-auto-revoke.sh`, `revoke-setup-key.sh` (SQL `UPDATE setup_keys SET revoked`) | CLI; manual | `PUT /api/setup-keys/{id}` `revoked: true`; direct SQL | **3** | ADR 1193 `netbird.setup_key` captures both the API and the SQL path | `netbird-c3-key-revoke-at-a-use-at-b` |
| N4 | `admin.rs:2998` `run_peer`; `ops/peer-cleanup.sh:35`; `lifecycle-sweep.py:1763` (revoke lane) | `goat admin peer cut <sel>`; manual | `DELETE /api/peers/{id}` by id | **3** | ADR 1193 `netbird.peer.deleted` | `netbird-c3-peer-delete-at-a-connect-at-b` |
| N5 | `admin.rs:7872,7898` `revoke_one`; `operator-onboard.yml:2117,2149`; `rosenpass_revoke_and_reinvite.py:275` | `goat admin onboard <h> --revoke --native` (runner `offboard.go:71`; `ops/scripts/offboard.sh:122`) | Lists peers and keys, deletes peers and revokes keys whose name matches the handle (`revoke_peer_matches`) | **X5** | Ledger kind `netbird.subject` keyed on the handle; arrival re-runs the cut | `cross-x5-revoke-at-a-mint-at-b` |
| N6 | `crates/goat-cli/src/cmd/platform_admin.rs:1403` `step_nb_role` | `goat admin platform-admin grant\|revoke [--full]` | `PUT /api/users/{id}` `{role, auto_groups, is_blocked}`, `auto_groups` echoed from an earlier read | **X6** | Role demotion as ledger kind `netbird.user.role`, guarded | `netbird-x6-role-demote-at-a-login-at-b` |
| N7 | `ops/ansible/netbird-role-realign.yml:163` | playbook | `PUT /api/users/{id}` | **5** | — (declared roles, hub only) | `netbird-c5-declared-state-hub-only` |
| N8 | `roles/netbird-admin-reconcile/files/netbird-admin-reconcile.sh:214` (timer, 5 min); `ops/ansible/dashboard-deploy.yml:299` | timer; deploy | SQL `UPDATE users SET role = …` (promotion only) | **5** | — (same result at every site) | `netbird-c5-declared-state-hub-only` |
| N9 | `crates/goat-cli/src/cmd/warrant_membership.rs:2415,2434` `apply_plan` | `goat warrant apply --membership` from `warrant-membership.timer` (`roles/warrant-membership/templates/warrant-membership.timer.j2:8`) | `PUT /api/groups/{id}`, fresh read per group | **5** | — (warrant files are the source; both sites compute the same membership) | `netbird-c5-warrant-membership-both-sides` |
| N10 | `platform_admin.rs:1466,1492` `apply_mesh` | `goat admin platform-admin grant\|revoke` on an ungoverned scope | `PUT /api/groups/{id}`: add, verify, remove | **X6** | Removal as ledger kind `netbird.group_peer`, guarded | `netbird-x6-group-remove-at-a-edit-at-b` |
| N11 | `crates/goat-cli/src/cmd/members.rs:827,858,882` `run_move` | `goat members move --apply` | `PUT /api/groups/{id}` built from the plan-time snapshot | **X6** | as N10; the PUT re-reads the group first | `netbird-x6-group-remove-at-a-edit-at-b` |
| N12 | `lifecycle-sweep.py:1371` (hold record to `/var/lib/peerview/hold-records.jsonl`, host-local `flock`), `:1373 → 705`, `:1385,1447 → 723` | `lifecycle-sweep.py --apply --lane quarantine` (manual) | JSONL append; `PUT /api/groups/{id}` into `<SITE>-PROBATION-<role>` and out of every prior group | **X6** | The hold is ledger kind `netbird.hold` (peer, prior groups); the record moves to Postgres (design §5 B3) | `cross-c3-hold-at-a-release-at-b` |
| N13 | `lifecycle-sweep.py:1580,1698,1702` (groups), `:1585,1695,1707` (JSONL) | `--lane release`, run by the onboard-runner `ops/onboard-runner/runner.go:448` from console C17 | `PUT /api/groups/{id}` back into prior groups; JSONL rewrite | **3** | A `reinstate` row naming the hold (X14) | `cross-c3-hold-at-a-release-at-b` |
| N14 | `ops/scripts/external-service-finish.py:612` | `external-service.yml` | `PUT /api/groups/{id}`, fresh read | **5** | — | `netbird-c5-declared-state-hub-only` |
| N15 | `ops/ansible/netbird-acl-cutover.yml:603,647,674,776,795,1149,1159` | playbook | `POST`, `PUT` (full body), `DELETE /api/policies…`; `DELETE /api/groups/{id}` on teardown | **5** | A cut-off site may `PUT` existing policy ids only; `POST` and `DELETE` wait for heal (§9 D8) | `netbird-c5-policy-regenerate-no-duplicates` |
| N16 | `ops/ansible/netbird-acl-bootstrap.yml:165`; `zagros-bootstrap.yml:151`; `zagros-prod-bootstrap.yml:288` | standup | `POST /api/groups` | **5** | — (hub only) | `netbird-c5-declared-state-hub-only` |
| N17 | `goat-coredns.yml:246,271,346,370`, `mesh-coredns.yml:119,146`, `mgmt-host-dns.yml:132`, `nethost-dns.yml:66`; `ops/relay/deploy-relay.sh:234,240` | playbooks; relay deploy | `POST`, `PUT /api/dns/nameservers[/{id}]` | **5** | as N15 | `netbird-c5-declared-state-hub-only` |
| N18 | `ops/scripts/netbird-service-identities.py:356,361` | `--mint`, at standup | `POST /api/users` (service user), `POST /api/users/{id}/tokens` | **1** | — | `netbird-c1-service-user-both-sides` |
| N19 | `netbird up --setup-key` on every enrolling device, from a key goat minted (N1, N2, P10); `admin.rs:3207` PQC toggle re-login | client | gRPC Login | **X15** | Serial repair after heal; usage-limit check in `value_collisions` | `netbird-x15-enrol-both-sides-serial` |

Not in the table: test harnesses that write NetBird SQL directly (`ops/wave3/*`,
`ops/wave-33/tier-4b/inchworm.sh`, `ops/ansible/cgnat-validation.yml:223,311`) and
`ops/time-hygiene/cmd/ntp-admit-sidecar`, which gates requests and writes nothing itself.

---

## 6. Kratos and Hydra writes made by goat outside the console

The portal makes no Kratos or Hydra write (`ops/goat-zenoh-portal/internal/kratosadmin/client.go:123,155`
are GETs). goat-cli reaches Kratos with curl over SSH; `goat admin role set|deactivate|reactivate`
and `goat admin platform-admin revoke` call the console machine API (C43, C44).

Replicated, per `ops/ansible/roles/ory-narrow-repset/defaults/main.yml:65-77`: Kratos
`identities`, `identity_credentials`, `identity_credential_identifiers`, `identity_credential_types`,
`identity_recovery_addresses`, `identity_verifiable_addresses`, `networks`; Hydra `hydra_client`,
`hydra_jwk`, `networks`. Excluded (`:86-113`): every self-service flow table, `sessions`,
`session_devices`, `courier_messages`, and every `hydra_oauth2_*` table. Recovery and verification
tokens and codes are on neither list and are not replicated.

| # | Surface (file:line) | Trigger | Call | Tables | Class | Change | Scenario |
|---|---|---|---|---|---|---|---|
| K1 | `crates/goat-cli/src/cmd/admin.rs:6981` (`provision_ory_native` `:6797`) | `goat admin onboard <h> --native` | `POST /admin/identities` | Kratos identity tables | **4** | as C41 | `kratos-c4-same-email-both-sides` |
| K2 | `admin.rs:6876 → 6910` (`identity_resync_patch` `:6742`) | same verb, identity exists | GET, then JSON `PATCH` of traits and metadata | `identities` | **2** | owner as C46 | `kratos-c2-trait-edit-owner-only` |
| K3 | `admin.rs:6688` `mint_recovery_link` | same verb with a passkey | `POST /admin/recovery/link` | recovery flow and token (local) | **X13** | as C42 | `kratos-x13-recovery-link-other-site` |
| K4 | `admin.rs:8131 → 8170` (`revoke_ory_identity` `:8111`, from `revoke_one` `:7912`); `ops/scripts/offboard.sh:122` | `goat admin onboard <h> --revoke --native` | JSON `PATCH /state` = `inactive` | `identities.state` | **3** | ADR 1193 `kratos.identity` | `kratos-c3-deactivate-at-a-edit-at-b` |
| K5 | `admin.rs:8156` | same verb, `--purge` | `DELETE /admin/identities/{id}` | `identities` + cascades | **3** | ADR 1193 `kratos.identity.deleted` | `kratos-c3-delete-at-a-edit-at-b` |
| K6 | `crates/goat-cli/src/cmd/members.rs:1166 → 1217` (`apply_identity_leg` `:1158`) | `goat members move` | GET, then JSON `PATCH` swapping groups and markers | `identities.metadata_public`, `traits` | **X6** | as C43 | `kratos-x6-group-remove-at-a-edit-at-b` |
| K7 | `ops/scripts/ory-group-mirror.sh:19,38,44` | manual, `APPLY=1` | Bulk JSON `PATCH` of the whole `metadata_public`, `test`-guarded | `identities.metadata_public` | **5** | — (hub only, after heal) | `kratos-c5-bulk-backfill-hub-only` |
| K8 | `ops/identity/backfill-aff-trait.py:266`, `backfill-verified-addresses.py:194`, `backfill-authz-traits-to-metadata.py:158`, `backfill-vendor-to-org-trait.py:134` | manual one-shots | JSON `PATCH` | `identities`, `identity_verifiable_addresses` | **5** | — | `kratos-c5-bulk-backfill-hub-only` |
| K9 | `ops/ansible/operator-onboard.yml:1262` | Ansible onboard | `POST /admin/identities` | Kratos identity tables | **4** | as C41 | `kratos-c4-same-email-both-sides` |
| K10 | `operator-onboard.yml:1396` | Ansible onboard | `POST /admin/recovery/link` | recovery flow and token (local) | **X13** | as C42 | `kratos-x13-recovery-link-other-site` |
| K11 | `operator-onboard.yml:2290,2309` | Ansible revoke, `--purge` | `PATCH /state`; `DELETE /admin/identities/{id}` | `identities` | **3** | ADR 1193 | `kratos-c3-deactivate-at-a-edit-at-b` |
| K12 | `ops/ansible/operator-tier-onboard.yml:331` | playbook | JSON `PATCH` adding to `metadata_public/groups` | `identities.metadata_public` | **2** | owner as C46 | `kratos-c2-trait-edit-owner-only` |
| K13 | `ops/ansible/roles/ory/tasks/main.yml:523` | ory role, bootstrap admin | `POST /admin/identities` | Kratos identity tables | **5** | — (standup, hub only) | `kratos-c5-bulk-backfill-hub-only` |
| K14 | `ops/scripts/revoke-stale-consent.sh:65-73,87-90`; `roles/ory/tasks/main.yml:656` | manual; ory role | `DELETE /admin/oauth2/auth/sessions/consent?subject=…`; `DELETE /admin/oauth2/tokens?client_id=…` | `hydra_oauth2_flow`, `_access`, `_refresh` (not replicated) | **X11** | Ledger kind `hydra.consent` in the `hydra` database | `hydra-x11-consent-revoke-reaches-b` |
| K15 | `ops/ansible/external-service.yml:402,412,441,460`; `zagros-prod-oidc-clients.yml:164,215`; `trust-sandbox-oidc-client.yml:104`; `roles/panoscope/tasks/main.yml:365`; the `*-oidc-client` roles (`netvis`, `microscope`, `grafana`, `minio`, `oauth2-proxy-id-console`, `openobserve`, `herd`, `forgejo`, `oauth2-proxy-dashboard`, `isitup`), `oauth2-proxy-surface`, `netbird-dashboard`, `relay-health` | ansible, per deploy | GET, then `POST /admin/clients` or full `PUT /admin/clients/{id}`; `PUT …/lifespans`; `DELETE` | `hydra_client` | **5** | — (declared in group_vars; one site, replicates) | `hydra-c5-client-declared-state` |

`ops/scripts/operator-idp-enroll.sh:134,162` writes the separate operator IdP Kratos (port 4436,
own Postgres on 5436), which is not replicated and not in scope.

---

## 7. Scripts that open the SQLite files directly

Three scripts write and eight read. After step A each one reads Postgres or calls the portal or
console API; a script never writes a replicated table directly, because it would bypass the
`owner_site` check.

| # | Script | R/W | Tables | Trigger | Class | After step A |
|---|---|---|---|---|---|---|
| S1 | `ops/scripts/backfill-invite-recipients.py:148` (write `:203-206`) | write | `onboard_invites` | manual, `--apply` | **5** | One-shot SQL at the hub |
| S2 | `ops/goat-id-console/scripts/retro-backfill.py:265` (writes `:303-304`, `:376-381`) | write | `onboard_proposals` | manual, `--apply` | **5** | as S1 |
| S3 | `ops/goat-zenoh-portal/cmd/pubsub-export/main.go:33,45` | opens read-write; `registry.Open` runs the schema statements | all | manual | **5** | Read-only DSN; no migration from a CLI |
| — | `ops/scripts/reap-orphans.py:87,111` | read | `onboard_proposals`, `slots` | manual | — | Postgres or API read |
| — | `ops/scripts/sync-slot-anchors.py:62` | read | `slots` | manual (writes `group_vars/*/zenoh-lazy-storage.yml`) | — | as above |
| — | `ops/goat-id-console/scripts/slot-identity-backfill.py:81` | read | `slots`, `onboard_proposals` | manual | — | as above |
| — | `ops/zenoh-trial-peers/budget-notify/budget-notify.py:92` | read | `slots` | timer (`roles/budget-notify`) | — | as above |
| — | `ops/identity/register-drift.sh:92,97` | read | `onboard_proposals` | manual | — | as above |
| — | `ops/ansible/roles/goat-zenoh/tasks/main.yml:193` | read | `slots` | ansible preflight | — | as above |
| — | `ops/identity/identity-usage.py:52,74-80` | read | `onboard_proposals` | manual | — | as above |
| — | `ops/scripts/rosenpass_revoke_and_reinvite.py:341-346` | read | `onboard_proposals` | manual | — | as above |

`ops/relay/health/registry-wal-signals.py` (`os.stat` at `:72-89`) and
`ops/compose/goat-obs/prometheus/rules/registry-wal.yml` open no database; both are deleted with
step A (design §5 A1).

---

## 8. Paths that fit no class, and the rule for each

**Plain:** each shape below, placed in the nearest class, would still lose a write or bring a
revoked thing back after a partition. X2, X4 and X14 affect paths classed 2 or 3 above; the
other twelve are the class of the 31 paths marked X. Each rule below turns the shape into one of
the five classes, or adds a rule the design does not yet state.

| # | Shape | Paths | What goes wrong across a partition | Rule |
|---|---|---|---|---|
| X1 | A credential stored in a column and rotated in place | P6 | A rotates a leaked key; B, cut off, writes any column of the same `vendors` row (P3-P5, P34, P35). Spock replicates the whole new row, so B's later commit puts the old key hash back at both sites | Credentials are rows: `vendor_api_keys` insert-only (class 1); retiring a key is a class 3 ledger row. The `vendors` row holds no secret |
| X2 | A revoked slot returns to the pool by an in-place `UPDATE`, and reuse clears `revoked_at` | P8, P11 (`slots.go:413,435`, `:229`) | The revoke clears `assigned_to` on a row the owner keeps stamping (P7, P14-P16), so last-write-wins undoes it. Reuse hands the same anchor, which is the certificate CN, to a new tenant while the old leaf is still valid | Each claim gets an `assignment_id` (xid); the revocation is a ledger row naming the `assignment_id`. Only the range owner reuses a slot, and only after the revocation is in its ledger and the old leaf has expired or is on the CRL |
| X3 | An invariant across rows, checked at write time | P9 quota (`machine_enroll.go:234`), P30 last admin (`govern.go:142`), C16 one pending action (`actions.go:171`), C33 no parent cycle (`orgstore/store.go:552`) | Each site checks only its own rows; together they exceed a quota, remove every admin, queue two reissues or form a cycle | Keep the local check. Add each invariant as a query in `value_collisions`, so a violation after heal is in the resolution queue. Never auto-undo |
| X4 | Partner slot blocks are fixed `idx` ranges | P1 partner path (`internal/api/handlers.go:472-490`, `zenoh_partner_slot_blocks`) | Per-site ranges and partner blocks cut the same `idx` space; a block no single site owns is claimed from both sides | Split each partner block into per-site sub-ranges: `slot_ranges(site, lane_set, lo, hi)`. The router render is unchanged because it pre-renders every slot in the block |
| X5 | A revoke that acts only on what this site can list | P11 `CutMachine` (`netbirdkeys.go:353`); N5 `revoke_one` (`admin.rs:7872,7898`); the design §6 case "a revoke at A while B reissues the key" (N2) | A key minted or a peer enrolled at the other site during the partition has a different NetBird id. ADR 1193 keys its ledger on row ids, so those rows survive the heal | Record the revocation against the subject (`kind = netbird.subject`, subject = handle or Kratos id) and re-run the cut when the ledger row arrives at each site, as ADR 1193 decision 4 does for sessions. The guard refuses a new key whose name matches a revoked subject |
| X6 | A privilege removed by editing a row or list other writes also rewrite | P12 `network_admin`; C43, K6 `metadata_public.groups`; N6 `users.role`; N10, N11 group membership; N12 a hold, which is a group removal | Removing a privilege is a revocation stored as a field edit; a concurrent write of the same row at the other site restores it. NetBird's `SaveUserLastLogin` is a full-row save on every login (ADR 1193 context), so a demotion is undone by the next login at B. ADR 1193 covers `setup_keys.revoked`, `users.blocked`, `identities.state` and deletes of peers, keys and identities only | Privilege removals are class 3: ledger kinds `portal.network_admin`, `kratos.identity.group`, `netbird.user.role`, `netbird.group_peer`, `netbird.hold`, each with a guard that refuses re-adding without a `reinstate`. An extension of ADR 1193 |
| X7 | A record edited by its subject, not by a site | P18, P20, P21, P23: the org's own API key, accepted at whichever site it reaches | Class 2 with a site owner refuses the org's own edits at the other site | Class 2 ownership may be held by a principal. A subject-owned row accepts the subject's writes at any site; a conflict needs the same credential used on both sides at once, and lands in `spock.resolutions` |
| X8 | Writes made automatically at every site with random ids | P38 govern reconcile; P28 `EnsureOrgMultipubPool` | Both sites nominate the same first member under two `gadm-<hex>` ids, or mint two pool handles for one org; the losing pool's topics have no pool after heal | Ids of derived rows are a hash of their inputs (`gadm-` = hash(org, identity); `pool_handle` = hash(salt, org)), so both sites write the same row |
| X9 | A runtime write to a row that also holds an admin decision | P35 digest backfill; C24 installer reports; P34 until it moves | The F-573 shape inside portal and console: a routine write at the cut-off site carries the whole row and undoes the other site's admin write | Runtime writes go to per-site tables (`portal_local`, `console_local`); replicated admin rows are written only by admin paths |
| X10 | State changed by a read or a timer at every site | C15, C19, C22 | Every site makes the transition on its own, and a GET at a non-owner site writes an owned row | Time-derived status is computed at read (`expires_at < now()`), never stored; reapers act only on `owner_site = current_site()` |
| X11 | A revocation whose effect lives in an unreplicated table | C47 Kratos sessions; K14 Hydra consent and tokens | The delete runs at one site; sessions and OAuth2 tokens at the other site stay valid. ADR 1193 ends sessions only as a side effect of `kratos.identity` | Ledger kinds `kratos.sessions` and `hydra.consent` whose arrival re-runs the delete at that site |
| X12 | A merge that rewrites many rows | C35 | Merging X into Y at A and Y into X at B leaves a cycle of absorbed orgs; unmerge deletes aliases the other site still uses | Org merge and unmerge are refused while any `orgs` subscription is not `replicating`. A merge resolves a class 4 collision, which exists only after heal, so a cut-off site loses nothing it needs. This is the one write not available to a cut-off site |
| X13 | A credential handed to a person that only one site can redeem | C42, K3, K10 | A welcome link minted at A fails at B's Kratos, before and after heal | The link URL names the issuing site's host; the outbound mail queue (step 7) records the site; `GET /onboarding/{id}/welcome` stops minting on read |
| X14 | Revoke and reinstate at two sites, ordered by clock | Every class 3 path, including C17 and N13: `goat_revoked` takes the latest event by `ORDER BY at DESC, id DESC` (PR #1538, `ops/postgres/spock/revocation-guard/netbird.sql:67-71`) | A revoke at A and a reinstate at B while cut off are ordered by the two sites' clocks | A `reinstate` row names the revoke event ids it cancels. A subject is revoked while any revoke event is not named by a reinstate. Set membership, no clock **Resolved in code** 2026-10-29 by PR #2077: a reinstate row carries `cancels uuid[]`, and `goat_revoked` is set membership with no timestamp. |
| X15 | A counter on a shared row | N19: `accounts.network_serial` (bumped by every group, policy, nameserver and peer change and every enrolment; `IncrementNetworkSerial` `sql_store.go:3378`); `setup_keys.used_times` | Both sites increment from the same value and last-write-wins keeps one. The serial can then go backwards for peers at the other site, and the client ignores any network map whose serial is below the one it holds (`client/internal/engine.go:1225`). A one-off key can enrol a peer at each site | After heal the hub sets `network_serial` above the highest value either site served. Peers enrolled from one key above its `usage_limit` appear in `value_collisions` |

Measured by the partition harness (PR #2083), not covered by a rule above: Kratos rewrites an
identity's credential rows with fresh ids on every admin write, so C41, C44, K4 and K11 at two
cut-off sites never converge (F-590);
N15 POSTs a missing warrant policy at each cut-off site (F-591).

Found in passing, not an island-mode issue: `update_group_peers`
(`crates/panoscope-core/src/netbird.rs:564`) sends only `{name, peers}`, and NetBird sets
`resources` to empty when the field is absent (fork
`management/server/http/handlers/groups/groups_handler.go:152`), so N6, N9, N10 and N11
erase any network resources attached to the group they edit.

---

## 9. What the design doc stated wrongly

All twelve were applied to `island-mode-multi-writer.md` §2.1, §3, §6, §7 and to ADR 1165 on 2026-10-29. The table stays as the record of what changed.

| # | Design doc says | Code shows | Change to the design |
|---|---|---|---|
| D1 | §3 class 1: slots are new rows from a per-site range | Slots are pre-baked; a claim is an `UPDATE` (`slots.go:76,229`) | Slot claim is class 2 with the range owner as owner; pool initialisation is class 5 (P1, P36) |
| D2 | §3 class 3, NetBird: "disable/delete peer, remove group membership"; console: "mesh hold" | ADR 1193 has no kind for group membership, user role or holds; a hold is a group removal (N12) | Add the X6 ledger kinds to ADR 1193's scope before class 3 is turned on |
| D3 | §3 class 4: `topics(vendor, key_pattern)` | The pair is owned by one org; a hashed id makes two registrations one row | Topic create is class 1 with a derived id (P19) |
| D4 | §3 class 2 lists "runner claims and leases"; §3.2 lists "runner lease columns" as local | A runner at each site would each win the CAS on its own copy and mint twice (C7) | Leases stay on the replicated row; what prevents a double mint is the `owner_site` filter on every claim |
| D5 | §3.2: `govern_meta` local | Its marker gates a one-time migration of `govern_admins` (P38); local, every site runs it once | `govern_meta` replicates, and the migration's ids are derived (X8) |
| D6 | §3.2: `vendor_progress` and `vendor_topic_activity` local | Both have `REFERENCES vendors(prefix) ON DELETE CASCADE` (`internal/registry/schema.sql:184,205`); a foreign key cannot cross databases. `vendor_topic_activity` has had no writer since `internal/registry/progress.go:233` | Drop the FK on `vendor_progress`; drop `vendor_topic_activity` instead of moving it |
| D7 | §7: "Classes 1-3 and 5 do not depend on timestamps" | Class 3 orders by `at` (X14); class 5's `network_serial` is a counter (X15) | Adopt the X14 and X15 rules, or state the dependency |
| D8 | §3 class 5: warrant ACL policies "regenerate locally while cut off" | `netbird-acl-cutover.yml:603` POSTs a missing policy and NetBird assigns the id; two sites regenerating create two copies | While cut off, regeneration only `PUT`s existing ids; `POST` and `DELETE` run at the hub after heal |
| D9 | §2.1: 10 scripts open the files, 2 write | 11 open them and 3 write (`pubsub-export` runs migrations); 2 of the 10 open nothing | §7 of this document |
| D10 | ADR 1165 Evidence: onboarding writes a NetBird setup key from the console (`onboard_api.go`, `onboard_actions.go`) | The console never writes NetBird; the onboard-runner runs `goat admin onboard --native`, which does (N1) | The harness (design §6) runs the onboard-runner and goat-cli as the NetBird writers |
| D11 | §3 has no Hydra row | `hydra_client` is replicated and declared (K15); consent and tokens are not replicated (K14) | Add Hydra: clients class 5, consent revoke X11 |
| D12 | §6 harness: portal, console, NetBird management and Kratos on each node | The double-claim case (C7) needs two onboard-runners | Add an onboard-runner per node |

---

## 10. Harness scenarios

Each name below is one case in the partition harness (design §6). Every case writes at site A
and site B with the subscriptions disabled, re-enables them, and passes when drift-watch reports
every table equal, every exception is in the resolution queue, and no slot, key or identity is
issued to two subjects.

| Scenario | Proves rows | Pass condition beyond the common one |
|---|---|---|
| `portal-c1-topic-create-both-sides` | P19 | One `topics` row |
| `portal-c1-two-topics-both-sides-loss-reported` | P19 | Where last-write-wins replaced a row on heal (a `spock.resolutions` `insert_exists` or `update_update` row whose two versions differ), drift-watch reports a loss for that table at each site, though its compare reports the sites equal |
| `portal-c1-schema-upload-both-sides` | P22 | Both versions present; schema bytes readable at both sites |
| `portal-c1-access-request-both-sides` | P24 | Two request rows |
| `portal-c1-grant-both-sides` | P26 | Two grant rows |
| `portal-c1-govern-nominate-both-sides` | P29 | Both nominations present |
| `portal-c2-slot-claim-both-sides` | P1, P14-P16 | Each claim is from its own site's range; no anchor assigned twice |
| `portal-c2-vendor-edit-owner-only` | P3-P5 | The non-owner edit is refused |
| `portal-c2-collect-once-both-sides` | P7 | One collection; the second site refuses |
| `portal-c2-machine-enroll-both-sides` | P9 | Two machines on two anchors |
| `portal-c2-budget-edit-owner-only` | P17 | The non-owner edit is refused |
| `portal-c2-decide-off-owner-refused` | P25, P32 | The non-owner decision is refused |
| `portal-c3-offboard-at-a-edit-at-b` | P8 | Org revoked at both sites after B's edit |
| `portal-c3-grant-revoke-both-sides` | P27 | Grant revoked once in effect, two ledger rows |
| `portal-c4-alias-both-sides` | P1 | One queue entry for the alias |
| `portal-c4-prefix-both-sides` | P2 | One queue entry |
| `portal-c4-probation-hold-both-sides` | P31 | One queue entry |
| `portal-c5-slot-pool-init-once` | P36 | Pool rows identical; no exception |
| `portal-c5-boot-migration-once` | P37, C30, C37 | Migration applied once |
| `portal-c5-affiliation-repair-hub-only` | P13 | Repair runs at the hub only |
| `portal-c5-acl-render-after-heal` | P42 | Router config equal at both sites |
| `portal-x1-rotate-at-a-edit-at-b` | P6 | Old key refused at both sites |
| `portal-x3-quota-both-sides` | P9 | Over-quota custodian in the queue |
| `portal-x3-remove-last-admins-both-sides` | P30 | Zero-admin org in the queue |
| `portal-x5-revoke-at-a-mint-at-b` | P11 | B's key and peer cut after heal |
| `portal-x6-network-admin-remove-at-a-edit-at-b` | P12 | `network_admin` off at both sites |
| `portal-x7-subject-edit-either-side` | P18, P20, P21, P23 | The org's edit accepted at the non-creating site |
| `portal-x8-first-multipub-both-sides` | P28 | One pool handle |
| `portal-x8-govern-reconcile-both-sides` | P38 | One nomination row |
| `portal-x9-runtime-write-vs-rekey` | P34, P35 | Rekey survives B's runtime writes |
| `console-c1-invite-mint-both-sides` | C12, C38 | Two invites |
| `console-c1-reactivation-request-both-sides` | C21 | One request row |
| `console-c1-negative-pair-both-sides` | C36 | One override row |
| `console-c2-decide-off-owner-refused` | C4-C6, C9, C20 | The non-owner transition is refused |
| `console-c2-claim-both-runners` | C7, C8, C18 | One runner claims; one setup key and one identity minted |
| `console-c2-invite-edit-owner-only` | C13 | The non-owner edit is refused |
| `console-c2-operator-key-approve-owner-only` | C27 | One secret issued |
| `console-c2-org-edit-owner-only` | C32 | The non-owner edit is refused |
| `console-c3-offboard-runs-at-both-sites` | C11 | Both sites' runners finish; no error from the second |
| `console-c3-invite-revoke-vs-consume` | C14, C40 | Token refused at both sites after heal |
| `console-c3-operator-key-revoke-both-sides` | C28 | Key revoked; key directory without it at both sites |
| `console-c4-same-email-both-sides` | C1, C3 | One queue entry for the email |
| `console-c4-invite-token-both-sides` | C2, C39 | One queue entry for the token |
| `console-c4-ssh-key-both-sides` | C23 | One queue entry |
| `console-c4-operator-key-both-sides` | C26 | One queue entry |
| `console-c4-org-slug-both-sides` | C31, C34 | One queue entry, resolved by org merge after heal |
| `console-c5-operator-key-dir-converges` | C29 | Key directory equal at both sites |
| `console-x3-two-reissues-both-sides` | C16 | Duplicate pending action in the queue |
| `console-x3-parent-cycle-both-sides` | C33 | Cycle in the queue |
| `console-x9-installer-report-vs-revoke` | C24, C25 | SSH key revoked at both sites |
| `console-x10-expiry-is-derived` | C15, C22 | No write at the non-owner site |
| `console-x10-lease-reaper-owner-only` | C19 | Only the owner's reaper acts |
| `console-x12-merge-refused-while-cut-off` | C35 | Merge refused while cut off; accepted after heal |
| `cross-c3-offboard-at-a-reissue-at-b` | C10, N2 | Person offboarded; B's reissued key revoked at both sites |
| `cross-c3-hold-at-a-release-at-b` | C17, N12, N13 | Release names the hold; result does not depend on site clocks |
| `cross-x5-revoke-at-a-mint-at-b` | N5 | Keys and peers minted at B for the handle are cut |
| `netbird-c1-setup-key-both-sides` | P10, N1 | Two keys |
| `netbird-c1-service-user-both-sides` | N18 | Two users |
| `netbird-c3-key-revoke-at-a-use-at-b` | N3 | Key revoked at both sites (PR #1538 `sk-late`, `sk-del`) |
| `netbird-c3-peer-delete-at-a-connect-at-b` | N4 | Peer absent at both sites (PR #1538 `p-del`) |
| `netbird-c5-declared-state-hub-only` | N7, N8, N14, N16, N17 | Equal at both sites after the hub run |
| `netbird-c5-warrant-membership-both-sides` | N9 | Membership equal at both sites |
| `netbird-c5-policy-regenerate-no-duplicates` | N15 | One policy per warrant rule |
| `netbird-x6-role-demote-at-a-login-at-b` | N6 | Role stays demoted after B's login |
| `netbird-x6-group-remove-at-a-edit-at-b` | N10, N11 | Peer stays out of the group |
| `netbird-x15-enrol-both-sides-serial` | N19 | Every peer receives a network map after heal |
| `kratos-c2-trait-edit-owner-only` | K2, K12 | The non-owner edit is refused |
| `kratos-c2-reset-2fa-owner-only` | C46 | The non-owner reset is refused |
| `kratos-c2-hub-edit-carried-out-by-owner` | ADR 1205 D3 | The hub's edit, issued while cut off, is carried out by the owner after heal and is the value at both sites |
| `kratos-c2-transfer-owner-by-hub` | ADR 1205 D5 | The hub's `transfer_owner` is carried out by the current owner; both sites then hold the new `owner_site`, the login identifier still resolves, and the new owner's edit is the value at both |
| `hub-directive-from-non-hub-refused` | ADR 1205 D2 | A directive committed at a non-hub site is refused at every site; nothing severed, nothing edited |
| `hub-sever-site-stops-its-writes` | ADR 1205 D4 | After the sever nothing the severed site writes reaches the hub; after re-seed and `restore_site` replication resumes without replaying the severed writes |
| `kratos-c3-deactivate-at-a-edit-at-b` | C44, K4, K11 | PR #1538 identity `late` |
| `kratos-c3-delete-at-a-edit-at-b` | C45, K5 | Identity absent at both sites |
| `kratos-c4-same-email-both-sides` | C41, K1, K9 | One queue entry |
| `kratos-c5-bulk-backfill-hub-only` | K7, K8, K13 | Runs at the hub only |
| `kratos-x6-group-remove-at-a-edit-at-b` | C43, K6 | Group stays removed |
| `kratos-x11-session-revoke-reaches-b` | C47 | No session at B |
| `kratos-x13-recovery-link-other-site` | C42, K3, K10 | Link names A's host; redeems at A |
| `hydra-c5-client-declared-state` | K15 | `hydra_client` equal at both sites |
| `hydra-x11-consent-revoke-reaches-b` | K14 | No consent or token for the subject at B |

## Related

- [`island-mode-multi-writer.md`](island-mode-multi-writer.md): the design this inventory applies
- `island-mode-program.md`: step 5
- [ADR 1165](../adr/1165-island-mode-multi-writer-every-site-admits.md); ADR 1193 (PR #1538); [ADR 1084](../adr/1084-block-64r-mgmt-repset-and-two-node-federation.md) D3
- [`hub-spoke-admit-split.md`](hub-spoke-admit-split.md) §2: the admit/revoke taxonomy class 3 adopts
