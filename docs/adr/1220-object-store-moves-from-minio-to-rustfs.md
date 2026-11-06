# ADR 1220 — goat's S3 object store moves from MinIO to RustFS, proven by a conformance job before any goatnet moves

| Field | Value |
|---|---|
| Status | Proposed. No live effect on merge: no role, pin or goatnet changes until the work in [issue #47](https://github.com/USAREUR-AF/goatcore/issues/47) lands |
| Date | 2026-11-06 |
| Deciders | operator |
| Repo scope | goat core (`ops/ansible/roles/minio` and its consumers) |
| Tracking | [USAREUR-AF/goatcore#47](https://github.com/USAREUR-AF/goatcore/issues/47) |
| Amends | [ADR 0811](0811-audit-log-integrity.md) (storage substrate line) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 0110](0110-license-posture-agplv3-forked-components.md) (AGPL components as separate processes), [ADR 0111](0111-vendor-friendly-license-posture.md) (Apache-2.0 default, license tiers) |

## TL;DR (for a layperson)

goat keeps its tamper-proof audit records, telemetry and state snapshots in an object store
called MinIO. MinIO's free edition is no longer maintained: its code repository was frozen in
April 2026, so security holes found from now on will not be fixed. This decision replaces it
with RustFS, a maintained store under the same permissive license as goat, which speaks the
same storage protocol. Nothing changes on a running goatnet until a test job proves RustFS
does everything goat relies on, including refusing to delete locked audit records.

## Context

**MinIO's open-source edition has ended.** `minio/minio` was archived read-only on 2026-04-24;
its last community release is `RELEASE.2025-10-15T17-29-55Z`. goat runs it as
`cgr.dev/chainguard/minio` pinned by digest (`ops/ansible/roles/minio/defaults/main.yml:39`).
Chainguard rebuilds from that frozen source, so the pin keeps working, but no upstream fix
will arrive for a vulnerability found after October 2025.

**The license was never the problem.** MinIO is AGPL-3.0, run unmodified as its own container
and reached only over the S3 API (`rusty-s3` in `crates/panoscope-core`, a stdlib SigV4 signer
in `ops/goat-zenoh-portal/internal/audittail`). That is the separate-process pattern ADR 0110
permits; goat's Apache-2.0 code is unaffected. The replacement is chosen for upstream health,
and a permissive license is a preference under ADR 0111, not the driver.

**What goat needs from the store** (inventory of goatcore at `4f3fb11`):

| Class | Capabilities | Where |
|---|---|---|
| Plain S3 | Object Lock at bucket creation; GOVERNANCE and COMPLIANCE default retention; per-object retention; governance bypass; versioning and delete markers; lifecycle `Expiration`, `NoncurrentVersionExpiration`, `ExpiredObjectDeleteMarker`, `AbortIncompleteMultipartUpload`; ListObjectsV2; SigV4 header and presigned query; path-style URLs; bucket tags | `roles/minio/templates/mc-init.sh.j2`, `roles/audit-subscriber/tasks/lifecycle.yml:165-198`, `roles/cold-storage-bucket`, `crates/panoscope-core/src/audit_tail.rs`, portal `audittail` |
| MinIO admin API | Bucket quotas (`mc quota`), IAM users and policies (`mc admin user/policy`, built-in `readwrite` / `consoleAdmin`), `mc admin info`, scanner config | `mc-init.sh.j2:136-232`, `lifecycle.yml:344-379`, `ops/loadtest/benchmark-ceiling.sh` |
| MinIO server config | `MINIO_ROOT_*`, `MINIO_IDENTITY_OPENID_*` console SSO with `groups` matched to policy names, console on `:9001` behind Traefik | `roles/minio/templates/docker-compose.yml.j2:32-86`, `roles/minio-oidc-client` |
| MinIO endpoints | `/minio/health/live`; `/minio/v2/metrics/{cluster,bucket}` and about twelve `minio_*` metric names behind twelve alerts and three dashboards | compose healthcheck `:86`, `ops/compose/goat-obs/prometheus/`, `roles/minio/templates/prom-rules-minio.yml.j2` |

Unused: SSE/KMS, bucket notifications, replication, service accounts, MinIO audit logging.
No CI job runs MinIO today; every MinIO assertion runs at deploy time on a live host.

## Decision

1. **RustFS replaces MinIO** as goat's S3 object store. RustFS (`rustfs/rustfs`, Apache-2.0)
   reached 1.0.0 on 2026-09-16; 1.0.4 is current (2026-10-25). Its feature table lists every
   capability in the table above as available: Object Lock, versioning, lifecycle, bucket
   quota, IAM and policies, OIDC console sign-in with a groups claim
   (`RUSTFS_IDENTITY_OPENID_*`), web console, and audit logging. Its TLS stack is rustls on
   aws-lc-rs, which has a FIPS-validated module, so a FIPS build is a build-flag question rather
   than a re-platform.
2. **Conformance first.** Before any role changes, a CI job starts the candidate server in a
   container and exercises every plain-S3 capability above, with one hard assertion: a
   COMPLIANCE-locked object version cannot be deleted, including by the root credential, before
   its retain-until date. The same job runs against the current MinIO pin so the two results sit
   side by side. RustFS is adopted only if it passes; otherwise this ADR is revised with the
   failing capability named.
3. **Clients stay generic S3.** `mc` from the `minio-client` image remains the S3 client for
   plain verbs, addressed by `MC_HOST_<alias>`. Nothing runs `mc` inside the server container
   (`docker exec minio mc …` is removed from `roles/audit-subscriber`, `roles/cold-storage-bucket`,
   `ops/scripts/offload-to-cold.sh`, `ops/loadtest/benchmark-ceiling.sh`). MinIO-admin verbs
   (quota, IAM, server info) move to RustFS's admin interface in the server role, which is the
   one place allowed to know which server it runs.
4. **Monitoring is ported, and guarded.** Scrape paths and metric names are re-pointed to
   RustFS's surface, and each storage alert gains an `absent()` companion so a renamed metric
   pages instead of going silent.
5. **Migration is a copy over S3.** RustFS does not read MinIO's on-disk format. Each goatnet
   moves by `mc mirror --preserve` (which carries per-object retention) into a new RustFS
   deployment, then switches endpoints. The MinIO role stays deployable as the rollback until
   every goatnet has moved; then it is removed with its pins.
6. **Image:** Chainguard-first, per goat's supply-chain policy; otherwise the upstream image
   pinned by digest.

The work list, with file and line references, is [issue #47](https://github.com/USAREUR-AF/goatcore/issues/47).

## Alternatives considered

| Option | License | Why not chosen |
|---|---|---|
| Stay on MinIO at the frozen pin | AGPL-3.0 | No security fixes after October 2025 |
| Silo (`pgsty/silo`, maintained MinIO fork) | AGPL-3.0 | Zero migration cost, but a small fork with one maintaining organisation; kept as the fallback if conformance fails for RustFS |
| MinIO AIStor (commercial) | Proprietary | A per-deployment commercial license on core substrate contradicts ADR 0111 |
| SeaweedFS | Apache-2.0 | Master/volume/filer architecture adds moving parts; an open issue (seaweedfs#11333) reports COMPLIANCE retention accepted but not enforced on delete |
| Garage | AGPL-3.0 | No Object Lock, so it cannot hold the ADR 0811 audit tier |
| Versity S3 Gateway | Apache-2.0 | S3 over a POSIX filesystem; no quotas, IAM or replication of its own |
| Ceph RGW | LGPL | Too heavy for host-site mini PCs |

## Consequences

- goat's object store returns to a maintained upstream under the same license tier as goat.
- The store's admin surface (quotas, IAM, console SSO, metrics) is re-implemented once, in the
  server role; the readers and writers do not change, because they already speak plain S3.
- goat gains its first CI job that runs the object store, so the audit tier's WORM guarantee is
  tested on every change rather than only at deploy time on a live host.
- RustFS is a 1.0 project. The conformance job and the per-goatnet rollback are what make that
  acceptable; the release ladder soaks it on a development goatnet before production.

## Verification items for the implementer

These are open facts the conformance work settles. Each is listed on issue #47.

- Health endpoint path and readiness semantics on RustFS 1.0.x.
- Prometheus metrics: paths, names and auth mode.
- Whether a Chainguard RustFS image exists; whether the published build enables the aws-lc-rs
  FIPS module.
- Whether the external audit writer (`goat-audit-subscriber`) sends per-object retention headers.
