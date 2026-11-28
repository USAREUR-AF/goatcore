# Netbird build artifacts (multi-master HA + AutoPeerName + offline service-user token)

Patched build of the Netbird management server, from upstream v0.83.2. Three
**management-server** deltas vs upstream, applied in this order by the
[`Dockerfile`](Dockerfile):

1. **`patches/skip-migrations.patch`** — gates schema changes via the
   `NETBIRD_SKIP_MIGRATIONS` env var. Required for multi-master Spock
   replication, where uncoordinated AutoMigrate across sites breaks logical
   replication. It gates the main store's AutoMigrate (`store.NewSqlStore`),
   the Postgres activity store's migration, and, from v0.83.2, the access log
   store's AutoMigrate (`accesslogs/manager.OpenConn`), which moved out of the
   main store's model list but still runs against the main Postgres database
   unless `NB_ACCESS_LOG_POSTGRES_DSN` points elsewhere. SQLite stores are per
   node and always migrate. Context: ADR 0301 and
   [`docs/design/upstream-upgrade-process.md`](../../docs/design/upstream-upgrade-process.md).

2. **`patches/auto-peer-name.patch`** — adds an optional `AutoPeerName` field on
   `SetupKey` so an operator-assigned label is combined with the device-reported
   hostname at registration time: `Name = "<auto_peer_name>-<hostname>"`. Both
   halves of the identity are preserved so operators can match the mint-intent
   label back to the physical device. Origin: merged PR #1 on
   [USAREUR-AF/netbird](https://github.com/USAREUR-AF/netbird/tree/feat/setupkey-auto-peer-name) at commit
   `cb223726` (combined diff of `970d9506a..0e976e85e` scoped to
   `management/` + `shared/management/`, excluding `*_test.go`). Used by
   `goat admin` and the operator-onboard pipeline.

3. **`patches/service-user-token-bootstrap.patch`** — adds
   `netbird-mgmt service-user bootstrap-token --name <n> [--role admin]
   [--expires-in-days 365] [--account-id <id>] [--datadir <dir>]`, which mints a
   service user and an API token directly against the store, with no dashboard
   and no OIDC login, for the standup's first scripted API call. Re-running it
   upserts the same service user (`svc-<name>`). From v0.83.2 it opens the store
   through upstream's admin command openers (`withAdminStoreOnly`), which
   replaced the token command's store helper.

**Not carried at v0.83.2:**

- `peer-id-bound-relay-tokens.patch` (ADR 1021) — dropped. It was off on every
  goatnet, and upstream #8493 (v0.84.0-rc.1) is the upstream form of peer-bound
  relay tokens. `Relay.PeerBoundTokens` in `management.json` is now an unknown
  key that management ignores. The relay-side fuse
  (`ops/ci/check-relay-peer-bound-tokens-gate.sh`) stays: `goat_relay_require_peer_bound`
  must stay false, because this management mints no peer-bound tokens.
- `setupkey-user-id-stamp.patch` (F-237) — dropped. No caller ever set
  `user_id` on a setup key: NetBird validates `peer.UserID` against its own
  `users` table, so the stamp cannot carry an external identity, and person to
  peer attribution is done by name and registry
  ([`docs/design/peer-identity-resolution.md`](../../docs/design/peer-identity-resolution.md)
  Option B). The `setup_keys.user_id` column it added stays on databases that ran
  it; a database created at v0.83.2 does not have it.

**Client patches.** [`client-patches/`](client-patches/) holds the two ADR 1081
Layer 1 client changes (signal backoff jitter, offer circuit breaker), written
against the v0.71.4 client source. No build in this repository applies them: the
management build never compiled client code, so they left it at v0.83.2. They are
the source for a client build (the goat-client fork branch) and are rebased when
that build moves.

## Three-pronged approach

1. **Local patches** (this directory) — applied during `docker build` against
   the pinned upstream commit. **Immediate value; in use until each patch is
   merged upstream or rendered redundant.**
2. **Upstream PRs**:
   - skip-migrations: drafted at `docs/upstream-prs/netbird-skip-migrations.md`.
   - auto-peer-name: currently fork-internal only; pending upstream submission.
3. **Custom entrypoint** (`entrypoint.sh`) — wraps the patched binary with
   marker-file orchestration. Lets Ansible playbooks designate which site is
   the migrator on any given upgrade. **Useful regardless of where the env-var
   support comes from (local patch or upstream merge).**

## Files

- [`Dockerfile`](Dockerfile) — multi-stage build: fetches upstream Netbird at
  the pinned `NETBIRD_REF`, applies the three patches in sequence, builds the
  binary, copies it into a minimal runtime image with our entrypoint.
- [`patches/skip-migrations.patch`](patches/skip-migrations.patch) — unified
  diff against upstream. Defaults preserve upstream behavior (env var unset =
  AutoMigrate runs).
- [`patches/auto-peer-name.patch`](patches/auto-peer-name.patch) — unified diff
  against upstream. Empty `AutoPeerName` preserves upstream behavior
  (`Name = peer.Meta.Hostname`); non-empty triggers the combined naming.
- [`patches/service-user-token-bootstrap.patch`](patches/service-user-token-bootstrap.patch)
  — unified diff against upstream; adds the `service-user` command tree.
- [`client-patches/client-signal-backoff-jitter.patch`](client-patches/client-signal-backoff-jitter.patch)
  and [`client-patches/client-offer-circuit-breaker.patch`](client-patches/client-offer-circuit-breaker.patch)
  — client-scoped (agent build), against the v0.71.4 client source; not applied
  by any build here.
- [`entrypoint.sh`](entrypoint.sh) — wrapper that sets `NETBIRD_SKIP_MIGRATIONS`
  based on a marker file or `NETBIRD_RUN_MIGRATIONS` env var.

## Build

```bash
# Single-arch (host arch). NETBIRD_REF can be a tag, a branch, or a commit SHA.
# The Dockerfile default tracks the SHA the current patches are rebased against.
docker build -t goat/netbird-mgmt:patched \
    --build-arg NETBIRD_REF=6d4814d22726c16961be79409dfbf6a2b2f7537b .

# Multi-arch (push to registry)
docker buildx build --platform linux/amd64,linux/arm64 \
    -t ghcr.io/usareur-af/netbird-mgmt:patched-6d4814d \
    --build-arg NETBIRD_REF=6d4814d22726c16961be79409dfbf6a2b2f7537b --push .
```

## Coordinated upgrade procedure (uses the entrypoint)

The Ansible playbook [`ops/ansible/coordinated-upgrade.yml`](../ansible/coordinated-upgrade.yml)
executes:

1. Stop management servers on **all** sites.
2. Pull the new image on **all** sites.
3. `touch /opt/netbird/migrations.allow` on the **designated migrator** node only.
4. Start the migrator node — entrypoint sees marker → AutoMigrate runs → schema
   updated locally.
5. Verify Spock replication health on the migrator
   (`spock.sub_show_status()`; no recent `CONFLICT:` lines in the Postgres log).
6. `rm /opt/netbird/migrations.allow` on the migrator node.
7. Migrate and start the remaining sites, **one at a time**: on each, touch the
   marker, start management (entrypoint sees marker → AutoMigrate migrates that
   site's own database), wait until it accepts connections, remove the marker,
   and wait until its Spock subscriptions replicate again before the next site
   starts. Spock replicates rows, not schema changes, so the coordinated
   upgrade migrates each site's database in turn before starting its services,
   so a schema change reaches every site and replication resumes. AutoMigrate
   is idempotent: a site whose schema is already current changes nothing.

Any start without the marker, outside this playbook, still exports
`NETBIRD_SKIP_MIGRATIONS=true`.

## When to drop the local patch

When the upstream PR
merges into a `netbirdio/netbird` release:

1. Switch the Dockerfile to use the published `netbirdio/management:<version>`
   image directly.
2. Delete `patches/skip-migrations.patch` and the patch-application step from
   the Dockerfile.
3. Keep `entrypoint.sh` — still useful for marker-file orchestration.

## Pinning policy

The pinned `NETBIRD_REF` in the Dockerfile must point at an upstream commit
**all** patches apply cleanly against. When bumping Netbird:

1. Verify every patch still applies cleanly (`git apply --check`) against the
   target ref. If not, rebase the offending patch in the user's fork and
   regenerate the patch file.
2. Decide repset membership for every table the new release adds
   (`spock_mesh_netbird_mgmt_tables` in `ops/ansible/roles/spock-mesh-peering/defaults/main.yml`,
   recorded in [ADR 1084](../../docs/adr/1084-block-64r-mgmt-repset-and-two-node-federation.md)).
3. Re-run any Phase 2 validation tests.
