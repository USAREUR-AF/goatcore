# Audit metadata and storage TLS cutover

This procedure requires separate release and deployment authorization. The local
change does not activate a site, inspect stored objects or change retention.
Replay capture and ordinary body reads remain gated on all tiers.

## Operator audit confirmation over HTTPS

Audit metadata travels through the portal's authenticated HTTPS route. The portal
reads MinIO with its own storage credential; an operator laptop needs mesh access
to the portal on TCP 443, with no direct access to MinIO on TCP 9000. Keep the
existing backend-port restrictions in place.

With compatible portal and CLI releases, confirm a publish using its key, time,
payload length and SHA-256:

```text
goat replay '<active-key>' --since 10m --metadata
goat sub '<active-key>' --since 10m --metadata
goat narrate --since 10m
```

`--metadata` explicitly requests records without payload bodies. `narrate` and
the audit tier of `flow` use metadata directly. Payload replay without
`--metadata` remains gated; the CLI reports that gate and names the metadata
alternative. The new mode preserves key/time filtering and refuses a window
with unread objects. A capped listing is still reported as truncated.

Metadata requests never follow redirects or send the operator token to an
object-store origin. An HTTP 401/403 from the projection is an authorization
failure, not an expired presigned URL. HTTP 502 points to the portal's storage
read or projection; a transport failure to the portal requires checking its DNS,
TLS and mesh reachability on 443.

Acceptance on each goatnet requires an operator profile on a non-admin peer:
publish a known sample, retrieve its matching hash and length with the metadata
command, and verify direct MinIO TCP 9000 remains blocked. Local fixture tests
model this network boundary; they do not establish live mesh acceptance.

## Prepare the coordinated change

1. Select compatible subscriber, portal, CLI and inspection-surface releases.
2. Identify every client of the shared MinIO endpoint: initialization and
   lifecycle jobs, audit subscriber, portal, OpenObserve, snapshots, cold-storage
   tooling, console proxy, health probes and Prometheus.
3. Assign an explicit storage service hostname resolvable from each host and
   container. Issue a server certificate for that hostname under the organization
   root. Supply `minio_service_host`, `minio_tls_cert_src`, `minio_tls_key_src`
   and `minio_tls_ca_src` through the owning certificate workflow.
4. Validate the leaf's chain, expiry, server purpose, SAN and private-key match.
   Keep key material out of command arguments and logs. The role performs these
   checks before installation and recreates MinIO when material changes.
5. Prepare `minio_tls_enabled: true` and the common `minio_s3_endpoint`. Set the
   durable writer's `audit_subscriber_s3_endpoint` to that HTTPS endpoint, or leave
   it unset: it defaults to `minio_s3_endpoint`, and the role refuses any other
   name (a mesh-IP endpoint fails host-name verification on every upload). The
   audit host must see `minio_tls_enabled` and `minio_s3_endpoint` (or
   `minio_service_host`) in its group_vars: `audit-subscriber-deploy.yml` does not
   load the minio role, and a V2 writer with no endpoint is refused whatever
   `minio_tls_enabled` says. To run it without durable storage on purpose, set
   `audit_subscriber_s3_endpoint: ""` and `audit_subscriber_observe_only: true`.
   Subscriber
   and portal require explicit CA files. OpenObserve's combined trust bundle
   preserves host public roots and adds the organization root. Check that its
   OIDC/JWKS and other HTTPS integrations still work.
6. Ensure each history reader has its own operator-capable portal service token.
   Readers use the portal projection, not direct audit-bucket credentials.
7. Keep `audit_subscriber_replay_enabled`, `portal_replay_enabled` and
   `minio_replay_enabled` false. The roles reject true on every tier.
8. Prepare revocation of old raw-reader grants and rotation of old presigning
   credentials. Removing a route does not invalidate previously signed URLs.
   Coordinate revocation with active readers; preserve the new portal reader's
   narrowly scoped access. Do not print credentials or raw alias configuration.

## Change storage and readers

1. Schedule the shared storage outage and stop affected writers cleanly.
2. Install native MinIO TLS and migrate every client in the same maintenance
   window. Check HTTPS reachability using the configured hostname and trusted CA.
3. Verify that HTTP, wrong-name, expired and untrusted certificates fail.
4. Re-run `audit-fifo-evict.yml` on every audit host in the same window. With
   `minio_tls_enabled: true` the evictor uses `minio_s3_endpoint` and verifies the
   site CA at `minio_client_ca_path`; the role refuses an `http://` endpoint. Run
   `/opt/audit-fifo-evict/goat-audit-fifo-evict.sh` once and confirm it measures
   the bucket (no `AuditEvictorCannotMeasure`), or audit eviction stops and the
   bucket fills to its quota.
5. Verify MinIO initialization, lifecycle, snapshots, OpenObserve storage,
   Prometheus targets and the console upstream. Refresh/recreate clients after
   CA changes; a mounted file does not prove an existing connection pool reloaded
   trust. Cold-storage aliases must also use verified HTTPS.
6. Activate compatible metadata readers. Confirm unauthorized requests fail,
   `/api/replay/presign` returns unavailable for authorized callers, and old audit
   clients without `projection=1` receive upgrade-required.
7. Confirm `/api/audit/object` returns allowlisted metadata only, rejects foreign
   subscriber keys, records access and fails a malformed object as a whole.
8. Revoke former raw-reader grants and rotate the former signing credentials.
   Verify that old presigned links and retired reader credentials no longer work.

## Initialize the metadata writer

Use one subscriber ID and durable state directory for each writer. A file lock
cannot coordinate different hosts or independent state directories.

1. Stop the legacy subscriber with SIGINT and confirm it exited after draining.
2. Using authorized metadata-only inventory, identify its final stored object key
   and the greatest batch millisecond value. Compare it with the reader cursor.
   For a new empty prefix use zero; never initialize from wall time alone.
3. Create the durable state directory with mode `0700`, owned by UID `65532`.
4. Run the V2 binary once under that UID:

```text
goat-audit-subscriber --endpoint <configured-zenoh-endpoint> \
  --receiving-site <site-id> --state-dir <durable-state-directory> \
  --audit-subscriber-id <existing-subscriber-id> \
  --initialize-state-after <final-batch-milliseconds>
```

This writes a new watermark and exits without subscribing. Existing state is an
error. Preserve state across upgrades. Protect the directory from other writers.

5. Configure `audit_subscriber_v2_enabled: true`, the released image,
   `audit_subscriber_receiving_site` and the durable state mount. Mount the
   rendered router policy read-only if policy inference is required. A digest
   identifies that file, not the configuration installed on a router.
6. Start the writer after storage and reader checks pass. Confirm the first V2
   key sorts after the final legacy key.
7. Publish synthetic put and delete events. Check kind, receiving site, observed
   outcome, length and known SHA-256, with no body field in objects or local tails.
8. Check explicit unknown attribution for ambiguous/shared/federated paths.
   SourceInfo is publisher-asserted, and slot attribution is only ACL-inferred.
9. Confirm only V2 puts advance publication/onboarding state. Legacy unknown-kind
   records and deletes must not promote it. Existing registry progress remains.
10. Check drop, upload, local-tail and session error counters before acceptance.

The roles refuse the released images until this cutover is configured, before
rendering or restarting anything. A subscriber image labelled
`io.goat.audit-subscriber.cutover=v2` requires `audit_subscriber_v2_enabled: true`,
and `audit_subscriber_v2_enabled: true` requires that label: an older image
would ignore the V2 settings and keep storing bodies.
A portal image labelled `io.goat.zenoh-portal.storage-tls=required` with a
provisioned audit reader requires `minio_tls_enabled: true` and an `https://`
`minio_s3_endpoint`. The portal Postgres cutover and rollback playbooks ask the
portal question in phase 0, before they stop the portal. `zenoh-loadtest.yml`
refuses a labelled subscriber image, because its second subscriber renders only
the legacy writer configuration. Until then, keep the previous image pins.

In V2 mode the subscriber role refuses a storage CA bundle that is missing or
not a regular file before it touches the running writer, mounts the rendered
router policy only when that file exists, and fails the play when the new
container is not running, or has restarted, `audit_subscriber_settle_seconds`
after `docker compose up`. With storage TLS the writer's endpoint must equal
`minio_s3_endpoint`, its default. When it writes to storage, the play also
fails if its metrics show failed uploads and none stored within one batch
interval.

The process has a 60-second drain deadline; Compose allows 70 seconds. Forced
termination or exhausted queues can lose records. Preserve evidence of any gap.

## Recovery and historical handling

Keep HTTPS, the metadata-only writer and durable state. Do not restore a legacy
body writer or HTTP endpoints. If that state cannot be maintained, stop the
writer and record the audit gap explicitly. Fix trust or client configuration
before restarting. Preserve failed state; recover from trusted evidence and
allocate beyond every known stored or previously allocated batch timestamp.
Never reset a watermark in place.

Historical bodies remain unchanged. Have the data owners inventory affected
prefixes, versions, replicas, local tails and cold archives under their existing
access controls. Determine classification, handling, retention and legal holds
before any new archival or disposal. Do not alter or bypass locks as part of this
cutover. Body capture needs a separate governance decision before its gate can
be removed; this implementation supplies no such authorization.

## Local evidence

Synthetic Rust and Go tests cover metadata serialization, known hashes, sample
kind, conservative provenance, authenticated projection, redirects, access logs,
limits and onboarding. The template fixture renders configuration and exercises
role gates without deployment tasks:

Both fixtures live in goatstuff, beside the subscriber; the roles, compose
files and policies they read live in goatcore. Run them from a goatstuff
checkout and name a goatcore checkout carrying the same change:

```text
python3 ops/zenoh-trial-peers/audit-subscriber/tests/template_fixture.py \
  --goatcore <goatcore-checkout> --evidence <new-empty-directory>
```

`GOATCORE_ROOT` may stand in for `--goatcore`. The storage fixture also needs
`ops/minio/policies/audit-read.json` and `audit-write.json` in that checkout and
stops with their names when they are absent.

The storage fixture uses disposable local MinIO data and synthetic credentials;
its native TLS mode also tests the actual subscriber SDK. It retains evidence
and stops its MinIO process. Read its `--help` for local binary/TLS options.
Local success does not establish live deployment, historical content or the
runtime behavior of a consumer image that was not exercised.
