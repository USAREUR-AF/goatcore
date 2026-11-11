# Audit data — tiered retention + cold-storage offload

| Field | Value |
|---|---|
| Status | **Design of record** as of 2026-05-31. Canonical for new deployments + the rollout pattern for existing ones. |
| ADR | [ADR 0811](../adr/0811-audit-log-integrity.md) — this doc names the concrete substrate for 0811's "1 year online + 6 years archive" tier split. |
| Block | 95 (new — supersedes the implicit ADR 0811 single-tier model where it landed at all) |

## TL;DR

Three tiers of audit-data retention, each with a substrate that's already in
place (or has a named, finite implementation step):

```
zenoh router rocksdb  →  MinIO audit-integrity   →  cold-storage bucket
       (live cache)       (durable, hot+warm)         (long-term archive)
       7 days TTL          30 days Object Lock          milestone-triggered
       hourly write-rate   write-rate × 30 days         operator op
       per-vendor sized    bounded by quota             per-dataset bundle
```

After the cold-storage tier, **goatnet does not hold the data**.
Whatever is needed beyond the 30-day MinIO window has been deliberately
offloaded by an operator and lives in a manifest-indexed bucket that's
external to goatnet's hot/warm flow.

> **Per-goatnet wiring is not automatic**: the middle-tier
> lifecycle is applied by `roles/audit-subscriber/tasks/lifecycle.yml` on every
> `audit-subscriber-deploy.yml` run — a goatnet whose bucket predates that task
> retains *forever* until the playbook is re-run. Do not read a clean run as proof:
> for any goatnet, the check that settles it is
> `mc ilm rule list <alias>/audit-integrity`, not the play recap.

## Quota pressure + FIFO-eviction (middle tier)

The middle tier is **GOVERNANCE-mode Object Lock**, not COMPLIANCE.
The substrate decision picks the more common operational case over
the rarer adversarial case:

| Scenario | Frequency | If middle tier is COMPLIANCE (refuse new writes) | If middle tier is GOVERNANCE (FIFO-evict oldest) |
|---|---|---|---|
| Real audit-data overflow (new vendor onboarded, producer spike, sustained growth past planning) | Common | **Bad** — operators lose live visibility into what's happening *now* | Good — keep current, drop oldest (which is presumably already cold-archived) |
| Attacker fills bucket with garbage to overwrite real records | Rare; signals deeper breach | Good — preserves evidence | **Bad** — attacker squeezes out legitimate records |

The attacker case is real but is layered-defense territory:

- audit-subscriber's S3 user is bucket-scoped append-only (per
  `ops/ansible/group_vars/<env>/audit-subscriber.yml` — no list, no
  delete, no governance-bypass)
- the FIFO-eviction service holds the bypass-governance permission
  on a separate identity, with its actions audit-logged into the same
  bucket (and ultimately into cold storage)
- the cold-storage tier is COMPLIANCE — once a milestone offload has
  run, that data is genuinely immutable for the dataset's full
  retention horizon, no bypass possible
- a write-rate spike alert (`AuditAnomalousWriteRate`) catches the
  garbage-flood attacker shape before FIFO-eviction has dropped much

So FIFO-eviction is the right policy for the working tier. The
substrate fails over to "we have less recent history" rather than "we
have no recent history" under pressure.

**Eviction policy** — built as the `audit-fifo-evict` role
(`ops/ansible/roles/audit-fifo-evict`, playbook `ops/ansible/audit-fifo-evict.yml`).
Without it, GOVERNANCE mode gives the substrate the weaker guarantee AND
COMPLIANCE's failure mode: a bucket that reaches its hard quota refuses every
new audit write while nothing makes room. The implementation follows the policy
below, with the quota read from the host's own declaration rather than the
server (reading a quota is an admin API, and the service deliberately holds no
admin right):

- Measure all stored versions with `mc du --versions`, using the host's
  declared quota. Ordinary current-key size can hide retained versions.
- At or above the high-water mark, enumerate version IDs and sort hour
  prefixes by their timestamp across sites. Before touching the next oldest
  hour, count every snapshotted version against the remaining byte budget.
  An oversized hour stops the pass; it does not justify deleting newer data.
- Delete each selected version with `mc rm --bypass --version-id ID
  ALIAS/BUCKET/KEY`. This removes retained versions rather than creating
  delete markers. The mc client needs bucket-scoped `GetObjectVersion` for
  its versioned metadata check as well as delete and governance-bypass rights.
- Bound each pass by both the configured byte percentage and hour count.
  Exit 2 means a bounded incomplete pass. Measurement errors exit 1;
  deletion, state or trail errors exit 3 and fail the unit. A measure-only
  pass that finds the bucket at or over the high-water mark exits 4 and
  fails the unit loudly: observing a breached threshold is not a success.
  The textfile publishes the same value (`last_result` 4) and raises
  `over_high_water`, so the exit code, the metrics and the alerts agree
  (built).
- Journal each attempted version on local durable storage before removal.
  Confirmed outcomes are uploaded beneath `ops/eviction/`; failed trail
  uploads remain pending and block the next destructive pass. The evictor's
  policy (`ops/minio/policies/audit-evict.json`) explicitly denies deletion of
  its own trail. Metrics remeasure usage after publishing the trail.
- Persist an actual cumulative hour-prefix counter. Alerting uses its
  `increase` over six hours, so repeated scrapes do not multiply a single pass.

Installation and the default timer are measurement-only. Set
`audit_fifo_evict_enabled: true` only as an explicit rollout decision after
checking the observed quota, retention posture, first pass scope and client
artifact. The installation probe always passes `--measure-only` and accepts
exit 0 or 4: either one proves the credential, and 4 means the bucket is
already over high-water. The timer is always started so metrics and alerting
stay live, and the converge then refuses to finish only when the probe saw 4
with eviction disabled, because a measure-only timer will never relieve that
bucket. The warning alert `AuditEvictionDisabledOverHighWater` watches the
same condition between converges, days before `AuditBucketAtQuota` goes
critical (built). This flag does not replace the functional eviction path;
`--evict` performs the bounded pass described above.

The role refuses to install until `audit_fifo_evict_mc_image` names an
immutable client image (`<image>@sha256:<64 hex>`); its default is an explicit
`@sha256:PENDING` declaration, recorded as non-rollable pending metadata.
Replace it with a verified multi-architecture reference and a matching ledger
row together. The integration fixture builds the exact upstream release source
into disposable local images; this proves version and retention behaviour but
does not publish a runtime artifact. The rollback ledger classifies eviction as
forward-only: reverting code cannot restore removed versions.

Reproduce the source contracts and the isolated real-version test:

```sh
python3 tests/test_audit_fifo_evict.py
bash tests/audit_fifo_source_integration.sh
promtool test rules ops/compose/goat-obs/prometheus/tests/audit-fifo-evict.test.yml
```

The integration script needs Docker, Git, Python 3 and Go. It selects the Docker
architecture, verifies the two upstream commit IDs, uses Go 1.26.10, and removes
only its unique test images and container. It contacts no live audit endpoint.

## Why three tiers (not one)

Single-tier 1-year retention (the ADR 0811 v1 implicit shape) made every
deployment carry a "1 year × p95 event rate × p99 byte size" quota
forever. On a single trial host (allocated 12 GiB → 100 GiB under
F-169)
that's substrate-cost the trial fabric can't afford.

The three-tier shape:

- **Router rocksdb (7d)** — already in place per-vendor at
  `/var/lib/goat-zenoh/rocksdb/<vendor>/`
  (`time_to_live: 604800000`, `garbage_collect_period: 86400`).
  Bounded by vendor-write rate; bounded by GC; survives router restart;
  not authoritative (cache for replay-on-late-subscribe + REST GET).
- **MinIO `audit-integrity` (30d, GOVERNANCE)** — Object Lock
  GOVERNANCE + matching lifecycle expire. Each PUT carries N-day
  retention; default delete is locked for everyone, but an identity
  with `s3:BypassGovernanceRetention` can release oldest-first for
  quota-relief FIFO-eviction (see §"Quota pressure + FIFO-eviction"
  below). Bounded by the substrate quota + the FIFO-eviction service
  rather than by hard-fail. The
  F-169
  alert rules + F-171
  noncurrent-version expiration close the silent-fill modes. This is
  the hot+warm working tier — operators query the last 30 days here;
  the COMPLIANCE-grade preservation bar lives in the cold-storage
  tier instead.
- **Cold-storage bucket (operator-triggered)** — terminal store for
  data that needs to outlive 30 days. The operator names a
  *dataset* (typically a deployment milestone like `trial-1.0`),
  the offload op mirrors the relevant MinIO contents into a
  `<dataset>-cold-storage` bucket, and writes a manifest sidecar
  recording sha256, date range, vendor coverage, event count.

## Cold-storage substrate

### Bucket naming + lock semantics

```
<dataset-id>-cold-storage
```

where `<dataset-id>` is operator-supplied, kebab-case, and references a
deployment milestone the operator wants to preserve (`trial-1.0`,
`site-handoff-2026-Q3`, `site-go-live`, etc.). One bucket per dataset.

**Object Lock**: COMPLIANCE on each bucket. Default retention per dataset
chosen by operator at create time. Recommended:

- Trial datasets: 1 year (bounded learning-data horizon).
- Production milestones / compliance-cited events: 7 years
  (matches NIST 800-53 AU-11).
- Forensic-capture-style datasets (post-incident): until the named
  incident's investigation closes, then operator decides
  retention-extend vs unlock.

### Storage location (current + future)

| Phase | Location | Trade-off |
|---|---|---|
| Today (2026-05-31) | **Same MinIO instance** as the source bucket. | Local — operator-laptop-mintable, fast, no inter-site dependency. Same-host failure = total loss. |
| Future | **TSI v3** (the inter-site Zenoh-router-attached storage substrate; not yet built). | Survives single-host loss; multi-site replication; operator unaware which physical host holds the bytes. Cold-storage data flows through TSI v3 instead of staying on the originating MinIO. |

The cold-storage substrate is location-agnostic by design — same bucket
naming + lock semantics + manifest contract on either substrate. The
**operator op** is the swap-out point: `offload-to-cold.sh` takes a
`--target` flag specifying where the cold-storage bucket lives, and the
local-MinIO-today case is just one valid `--target`. When TSI v3 ships,
the same script grows a `tsi-v3` target with a different `mc alias`
pointing at the federated endpoint.

### Manifest contract

Each offload writes one manifest sidecar at
`s3://<dataset-id>-cold-storage/manifests/<dataset-id>.json`:

```json
{
  "schema_version": 1,
  "dataset_id": "trial-1.0",
  "source": {
    "site_id": "site-a",
    "bucket": "audit-integrity",
    "key_prefix": "",
    "object_count": 174168,
    "version_count": 275515,
    "delete_marker_count": 101347
  },
  "coverage": {
    "first_event_ts": "2026-05-18T...",
    "last_event_ts":  "2026-05-31T...",
    "vendor_keys":    ["example-vendor/**", "notional/**", ...]
  },
  "archive": {
    "bucket": "trial-1.0-cold-storage",
    "object_count": 174168,
    "total_bytes": 8245612096,
    "sha256_of_manifest_of_objects": "abc123..."
  },
  "operator": {
    "handle": "<operator-handle>",
    "operator_id_jwt_sub": "...",
    "trigger_reason": "trial-1.0 milestone close — relaxing 7d MinIO retention to 30d going forward; preserving the trial's full audit trail for replay + analysis."
  },
  "timestamps": {
    "offload_started":  "2026-05-31T19:55:00Z",
    "offload_completed": "2026-05-31T20:12:00Z",
    "manifest_signed":   "2026-05-31T20:12:00Z"
  },
  "object_lock": {
    "mode": "COMPLIANCE",
    "retention_days": 365,
    "retain_until":  "2027-05-31T20:12:00Z"
  }
}
```

The `sha256_of_manifest_of_objects` is the sha256 over a
sorted-by-key NDJSON list of `{"key": "...", "sha256": "...", "bytes": N}`
for every object in the archive — stored alongside the manifest at
`s3://<bucket>/manifests/<dataset-id>.objects.ndjson`. This makes the
archive cryptographically queryable without reading every blob: an
operator can ask "is this exact bag of bytes still intact" against the
ledger.

### Offload op — `ops/scripts/offload-to-cold.sh`

Single shell script driving `mc` + a small Python manifest-emit. Idempotent
under operator-redo (re-running on the same dataset reads the existing
manifest, computes a fresh sha256 over the live archive, refuses to
overwrite if the source bucket has new objects since the last manifest
unless `--allow-extend` is passed).

```
offload-to-cold.sh \
  --source-alias    local \
  --source-bucket   audit-integrity \
  --target-alias    local \
  --target-bucket   trial-1.0-cold-storage \
  --dataset-id      trial-1.0 \
  --retention-days  365 \
  --operator        <operator-handle> \
  --reason          "trial-1.0 milestone close ..."
```

Steps:

1. Verify target bucket exists with Object Lock enabled; create if absent
   (uses `mc mb --with-lock`).
2. `mc mirror --preserve` source → target. (`--preserve` keeps the
   per-object retention timestamps; objects under their original
   COMPLIANCE retention stay under that retention in the archive.)
3. Walk the target, emit `<dataset-id>.objects.ndjson` with one row per
   object (sorted by key).
4. Compute `sha256_of_manifest_of_objects` over the file.
5. Emit `<dataset-id>.json` manifest.
6. PUT manifest + objects-list to `s3://<target>/manifests/`.
7. Apply object-lock retention to the two manifest files (separate
   call — `mc retention set` for COMPLIANCE + the named retention
   horizon).

## Ansible-ization

### New role: `ops/ansible/roles/cold-storage-bucket/`

Renders a parameterized cold-storage bucket on a named MinIO endpoint.
Idempotent (no-op when the bucket already exists, with a `force_recheck`
knob that re-verifies Object Lock settings without recreating).

| Variable | Default | Purpose |
|---|---|---|
| `cold_storage_dataset_id` | (required) | The dataset name |
| `cold_storage_target_alias` | `local` | `mc` alias of the destination MinIO |
| `cold_storage_target_bucket_suffix` | `-cold-storage` | Suffix; bucket = `<dataset_id><suffix>` |
| `cold_storage_object_lock` | `COMPLIANCE` | Lock mode |
| `cold_storage_retention_days` | `365` | Default lock retention |
| `cold_storage_minio_creds_envvars` | `[MINIO_ROOT_USER, MINIO_ROOT_PASSWORD]` | Source env vars for `mc alias set` |

### Source-side retention knobs — two-role split

The middle tier is configured across two roles:

| Concern | Role | Var(s) | Set when |
|---|---|---|---|
| Bucket Object Lock + default retention (per-PUT lock window) | `minio` | `minio_audit_retention_days` (default 365 / trial override 30) | Every minio-role apply |
| Bucket lifecycle policy — TWO rules (see below) | `audit-subscriber` | `audit_subscriber_minio_expiration_days` (30), `audit_subscriber_minio_noncurrent_days` (1), `audit_subscriber_minio_expire_delete_markers` (true), `audit_subscriber_minio_multipart_abort_days` (1, **requested but not stored — see below**) | Every audit-subscriber-role apply, via `tasks/lifecycle.yml`, when `audit_subscriber_apply_lifecycle: true` (default). Re-appliable alone with `--tags lifecycle`. |

The audit-subscriber role's lifecycle task uses `mc ilm rule import`
(atomic replace from a rendered JSON file) under rule ID
`audit-integrity-three-tier` — so any prior `expire-audit-after-Nd`
rule emitted by the legacy minio-role template is replaced on first
apply. To prevent the minio role from re-emitting a competing rule,
its lifecycle-on-audit-bucket clause stays gated on
`minio_audit_expiration_days` being defined — leave it unset
fleet-wide (the audit-subscriber role owns the lifecycle now).

### The policy is TWO rules, and both halves are load-bearing

```
audit-integrity-three-tier              30 days  EXPIRE DELETEMARKER false
  + NoncurrentVersionExpiration          1 day
audit-integrity-zombie-delete-markers    0 days  EXPIRE DELETEMARKER true
```

For eleven weeks it was one rule, and this document said the
`audit_subscriber_minio_noncurrent_days: 1` clause "is the F-171 fix". It is
half of it. Getting this wrong is what let the other half stay invisible, so
the distinction is spelled out rather than assumed:

- **`NoncurrentVersionExpiration`** prunes the noncurrent VERSIONS. It works.
- **`ExpiredObjectDeleteMarker`** removes the DELETE MARKERS those versions
  leave behind. On a versioned bucket, expiring a current version does not
  hard-delete it — it writes a marker. Prune the versions underneath and the
  marker is the only thing left at that key, and nothing else in the policy
  removes it.

With only the first clause the bucket **stops growing in bytes and keeps
growing in keys**. That is why quota and bucket-size alerting stayed quiet:
a marker with nothing beneath it stores nothing. The cost is CPU: MinIO's scanner walks every marker continuously
(F-373).

`ExpiredObjectDeleteMarker` is **mutually exclusive with `Expiration.Days` in
the same S3 rule**, which is why it renders as a second rule rather than a
fourth clause on the first. MinIO's own name for the condition it clears is
*zombie delete markers*.

Set `audit_subscriber_minio_expire_delete_markers: false` only to reproduce the
pre-fix state. There is no case where keeping the markers is useful.

### `AbortIncompleteMultipartUpload` is requested and NOT stored

`mc ilm rule import` accepts the clause, exits 0, and stores nothing.
Re-exporting after a clean apply returns only `Expiration` and
`NoncurrentVersionExpiration`. Measured on mc `RELEASE.2024-10-08` + MinIO
`RELEASE.2024-10-29`.

The role printed it as applied on every run for months. It now re-exports after
applying and **asserts** the three F-171 clauses actually landed, and reports
the multipart clause honestly rather than asserting it — failing every audit
deploy fleet-wide over a substrate quirk is the wrong trade, but printing a
policy you did not apply is worse than printing nothing, because it ends the
investigation.

**Stalled multipart uploads are therefore not cleaned up by this policy.**
Not fatal, not F-171, but do not assume it is running.

### Alerting

Two rules watch `minio_bucket_usage_deletemarker_total`, which MinIO exports
natively and which was already being scraped throughout — the metric was never
missing, only the rule was:

- **`AuditBucketDeleteMarkersNeverReclaimed`** — the marker/version ratio
  floored over 48h. A healthy bucket's ratio is a sawtooth with a ~24h period
  (scanner `speed=default`), so the question is not how high it peaks but
  whether the floor ever comes down.
- **`AuditBucketLifecycleMissing`** — zero markers on a bucket over 10,000
  objects, sustained 7d. There is no metric for "a lifecycle rule exists", so
  this infers it from the consequence. It is the rule that would have named
  F-372.

Both live in `ops/compose/goat-obs/prometheus/rules/audit-subscriber.yml` AND
`roles/minio/templates/prom-rules-minio.yml.j2`. That duplication is
deliberate: the two Prometheus stacks have separate rule-delivery paths and
neither reads the other's directory, so a one-sided change leaves a goatnet
uncovered.

**Shipping a rule file is a third, separate step**, and it is the one that was
missing. `obs-host` installs an EXPLICIT list (`obs_rule_files`), not a glob,
because a shared rule carrying an `absent()` guard for a service that host does
not run fires immediately and forever. `audit-subscriber.yml` was not in that
list, so on that stack these alerts were structurally unable to fire no matter
what the file said. It is listed now.

The bar for being in that list is a test, not an argument: the file must stay
silent on a Prometheus that scrapes none of its metrics.
`tests/audit-delete-markers.test.yml` feeds the rules no input series at all
and requires silence from all six alerts. `tls-cert-renewal.yml` is excluded
from the list precisely because it would fail that test.

One trap worth knowing if you add a file there: the role's did-they-load
assertion used to derive expected group names from the FILE STEM. That is a
convention, not a rule — `audit-subscriber.yml` declares
`audit-subscriber.writes` and `audit-integrity.bucket`, so listing it under the
old form would have failed the assertion on every obs host for a file that
loads correctly. The assertion now parses group names out of the files it
ships.

### Applying the lifecycle needs MinIO ROOT credentials

Lifecycle is an admin-plane operation; the audit-subscriber's own S3 user is
deliberately not privileged enough for it. The task mints an mc alias per run
from `minio_root_user_envvar` / `minio_root_password_envvar` — the same env-var
names the minio role uses for the host — via `MC_HOST_<alias>`, delivered
through an 0600 env file that `docker exec --env-file` reads, so the secret
never reaches a command line and is removed in an `always:` block.

It used to assume a persisted `local` alias already existed inside the
long-running `minio` container; where it did not exist, the resulting `Access Denied` was swallowed by
`failed_when: false` and reported as "bucket does not exist". A host that genuinely runs no MinIO should set
`audit_subscriber_apply_lifecycle: false` on purpose.

### Per-deployment overrides

```yaml
# group_vars/<trial>_hosts/minio.yml — trial fabric
minio_audit_retention_days: 30   # bucket Object Lock default, set by minio role
# minio_audit_expiration_days intentionally unset — audit-subscriber
# role owns the lifecycle now per Block 95.

# group_vars/<prod>_hosts/minio.yml — production
# (Same 7d/30d/cold-archive shape; the cold-storage bucket takes the
# long-term role. ADR 0811's "1y online + 6y archive" decomposes into:
#   online   = 30d MinIO,
#   archive  = cold-storage bucket with Object Lock 7y per NIST 800-53 AU-11.)
minio_audit_retention_days: 30
# audit-subscriber role defaults already match (30 / 1 / 1); per-deployment
# overrides only change values if a site's posture diverges.
```

The production posture is the **same substrate shape**, with one
difference: production sites are expected to do a cold-storage offload
on a scheduled cadence (e.g., monthly), not just at deployment-milestone
events, because every month of audit data needs to land in the long-term
archive to satisfy AU-11.

## First canonical run

(omitted)

## Anti-goals

This substrate is **not**:

- A real-time replication target. It's a milestone-triggered snapshot.
  Hot reads still go through the audit-subscriber + MinIO path.
- A backup-of-everything store. Only audit data + manifests live here;
  the substrate name reflects that explicitly.
- A queryable analytics warehouse. Objects are NDJSON-preserved in their
  source bucket layout; consumers wanting Parquet should repack
  separately.
- The terminal store for non-audit data. Postgres backups, image
  registry contents, configuration backups all have their own
  substrates (pgbackrest, OCI registry, ansible role state).

## What stops the substrate from drifting

1. **One naming convention.** `<dataset-id>-cold-storage`. Catalog query
   = "list buckets matching `*-cold-storage`."
2. **One manifest schema.** `schema_version: 1` + `sha256_of_manifest_of_objects`
   means later tools (a `cold-storage-list` CLI, a Grafana panel,
   stethoscope checks) can read every dataset uniformly.
3. **One role + one script.** Both are operator-touchable. No service
   runs continuously. Operator op invokes the script, script writes the
   manifest, role can be re-run idempotently.
4. **One ADR pointer.** ADR 0811's `Implementation note 2026-05-31`
   names this doc as the substrate for the "archive" tier the ADR
   originally posited. Future ADR authors see one canonical pointer.

## Related

- [ADR 0811](../adr/0811-audit-log-integrity.md) — the compliance baseline.
- [`docs/design/zenoh-audit-external-subscriber.md`](zenoh-audit-external-subscriber.md) — the audit-subscriber whose flush this substrate consumes.
- F-169 — silent-quota-fill; closed by the 30d retention + monitoring landed by this design.
- F-170 — digest-pinning; orthogonal.
- F-171 — lifecycle-vs-Object-Lock slack; the `noncurrent-expire-days: 1` clause this design ansible-izes is the fix.
- `docs/design/zenoh-data-fabric.md` — per-vendor router rocksdb (the 7d cache tier).
