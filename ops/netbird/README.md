# Netbird build artifacts (multi-master HA + AutoPeerName + peer-bound relay tokens + user-id stamp + client storm resilience)

Patched build of Netbird. Four **management-server** deltas plus two
**client** deltas vs upstream:

1. **`patches/skip-migrations.patch`** — gates `gorm.AutoMigrate()` via env var.
   Required for multi-master Spock replication, where uncoordinated AutoMigrate
   across sites breaks logical replication. Full context:
   `docs/design/ha-multi-site.md` §5.8.1.

2. **`patches/auto-peer-name.patch`** — adds an optional `AutoPeerName` field on
   `SetupKey` so an operator-assigned label is combined with the device-reported
   hostname at registration time: `Name = "<auto_peer_name>-<hostname>"`. Both
   halves of the identity are preserved so operators can match the mint-intent
   label back to the physical device. Origin: merged PR #1 on
   [USAREUR-AF/netbird](https://github.com/USAREUR-AF/netbird/tree/feat/setupkey-auto-peer-name) at commit
   `cb223726` (combined diff of `970d9506a..0e976e85e` scoped to
   `management/` + `shared/management/`, excluding `*_test.go`). Used downstream
   by the operator-onboard pipeline (queued track
   `netbird-fork-setupkey-auto-peer-name`).

3. **`patches/peer-id-bound-relay-tokens.patch`** — binds the management-issued
   relay auth token to the peer's relay `peer_id` (goat ADR 1021 Phase B).
   Adds `Relay.PeerBoundTokens` (default-off): when set, `GenerateRelayToken`
   emits an `AuthAlgo 2` token = `HMAC(sha256(relay_secret), peer_id ‖ expiry)`
   instead of the unbound algo-1 token, so a peer's relay token cannot be
   replayed to impersonate another peer at the relay. Pairs with the Rust `goat-relay` Phase A change (dual-accept). Carries the
   `peer_id`-preimage invariant test (mgmt↔relay lockout guard). Origin: merged
   PR #4 on [USAREUR-AF/netbird](https://github.com/USAREUR-AF/netbird/tree/feature/relay-peer-id-bound-tokens)
   (commits `4ec8f44fb..fd722b847`); full design
   ADR 1021. Default-off, so
   no behavior change until `Relay.PeerBoundTokens=true` is set and every relay
   tier accepts peer-bound tokens (ADR 1021 Phase C cutover).

4. **`patches/setupkey-user-id-stamp.patch`** — adds an optional `UserID` on
   `SetupKey` (a user-bound key) so peers enrolled with that key carry the owner
   identity on `peer.UserID` at registration, instead of the blank owner that
   setup-key (non-interactive) enrollment normally produces. Upstream only stamps
   `peer.UserID` on interactive JWT enrollment (the `addedByUser` gate), so every
   setup-key peer is unattributed at the DB layer; a user-bound key closes that for
   forward peers. Mirrors the AutoPeerName wiring exactly (struct field + `Copy` +
   `GenerateSetupKey`/`CreateSetupKey` param + `api.CreateSetupKeyRequest.user_id`
   + `handleSetupKeyAddedPeer`→`peerAddConfig.SetupKeyUserID`→`AddPeer`). It does
   NOT set `addedByUser`, so login/inactivity-expiration and every other
   user-added semantic stay as the setup-key path defines them — only the owner
   attribution changes. Empty `user_id` preserves upstream behavior, so it is
   additive + default-off (no onboarding sets it until user-bound keys are minted).
   **Constraint (F-237):** `user_id` MUST be a real NetBird user id — NetBird validates `peer.UserID` against its own `users` table, so feeding an external identity id (e.g. an Ory/Kratos id) *breaks* the peer's sync (`user … not found`). The setup-key participant model has no person-scoped NetBird users, so this stays a dormant capability; person↔peer attribution is done by name+registry (Option B), not this stamp.
   The person↔peer resolution design + Option-A rationale + dead-end finding:
   [`docs/design/peer-identity-resolution.md`](../../docs/design/peer-identity-resolution.md).

5. **`patches/client-signal-backoff-jitter.patch`** (goat ADR 1081 Layer 1,
   client-scoped) — replaces the fixed-cadence retry on the NetBird agent's
   two control-plane retry paths with AWS-style full-jitter exponential
   backoff. (a) The reconnection guard's signal-offer retry ticker
   (`client/internal/peer/guard/guard.go`) goes to base 1 s → cap 60 s, full
   jitter; (b) the signal-stream re-registration backoff
   (`shared/signal/client/grpc.go` `defaultBackoff`) goes to base 2 s → cap
   120 s, full jitter, preserving upstream's 3-month give-up bound. Both draw
   from a new shared calculator `shared/backoff` (`FullJitter`, implements
   `cenkalti/backoff/v4.BackOff`; unit-tested). Closes the self-amplifying
   retry loop of clients retrying offers + re-registration at a fixed pace
   into a degraded uplink. Full design
   [`docs/design/control-plane-storm-resilience.md`](../../docs/design/control-plane-storm-resilience.md)
   §"Layer 1" + [ADR 1081](../../docs/adr/1081-control-plane-storm-resilience.md).

6. **`patches/client-offer-circuit-breaker.patch`** (goat ADR 1081 Layer 1,
   client-scoped) — adds a per-peer-pair circuit breaker on offer sends
   (`client/internal/peer/breaker.go` + wiring in `conn.go`/`engine.go`).
   After 5 consecutive failed offer sends to one peer the breaker opens and
   `onGuardEvent` stops blind-retrying that pair; it half-opens for a single
   probe when management's network map reports that peer online again (the
   positive event the client already receives on every map update), with a
   5-minute fallback probe if that event is lost, and closes on a successful
   send. Breaker state machine is unit-tested. Turns retry from a poll into
   the void into an event-driven re-dial.

Patches 5–6 change the **client** (agent) build path, not the management
server; `go build ./management` in the Dockerfile does not compile them (they
are applied there only to keep the whole patch set verified-appliable against
the pinned `NETBIRD_REF` — see the Dockerfile comment). They are additive:
absent a storm, behavior is the same aggressive-but-bounded reconnection, just
with wider jitter and a breaker that only trips on a sustained run of failures.

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

- [`Dockerfile`](Dockerfile) — multi-stage build: clones upstream Netbird at
  pinned version, applies all three patches in sequence, builds the binary,
  copies into a minimal runtime image with our entrypoint.
- [`patches/skip-migrations.patch`](patches/skip-migrations.patch) — unified
  diff against upstream. Defaults preserve upstream behavior (env var unset =
  AutoMigrate runs).
- [`patches/auto-peer-name.patch`](patches/auto-peer-name.patch) — unified diff
  against upstream. Empty `AutoPeerName` preserves upstream behavior
  (`Name = peer.Meta.Hostname`); non-empty triggers the combined naming.
- [`patches/peer-id-bound-relay-tokens.patch`](patches/peer-id-bound-relay-tokens.patch)
  — unified diff against upstream. `Relay.PeerBoundTokens` unset/false preserves
  upstream behavior (unbound algo-1 relay tokens); true issues peer-`peer_id`-bound
  algo-2 tokens. Applies last; disjoint from the other two patches' files.
- [`patches/setupkey-user-id-stamp.patch`](patches/setupkey-user-id-stamp.patch)
  — unified diff against upstream, applied after auto-peer-name (shares its files:
  `types/setupkey.go`, `peer.go`, the setup-key handler + API types). Empty
  `user_id` preserves upstream behavior (peer.UserID blank for setup keys);
  non-empty stamps the owner on enrolled peers. Excludes `*_test.go` (same
  convention as auto-peer-name).
- [`patches/client-signal-backoff-jitter.patch`](patches/client-signal-backoff-jitter.patch)
  — unified diff against upstream. Client-scoped (agent build). Adds
  `shared/backoff/FullJitter` + rewires the guard offer-retry and signal
  re-registration backoffs to full jitter. Includes the calculator's unit
  test.
- [`patches/client-offer-circuit-breaker.patch`](patches/client-offer-circuit-breaker.patch)
  — unified diff against upstream. Client-scoped (agent build). Adds the
  per-peer-pair offer breaker (`client/internal/peer/breaker.go`) + guard/engine
  wiring + unit test. Applies after the backoff patch; disjoint files.
- [`entrypoint.sh`](entrypoint.sh) — wrapper that sets `NETBIRD_SKIP_MIGRATIONS`
  based on a marker file or `NETBIRD_RUN_MIGRATIONS` env var.

## Build

```bash
# Single-arch (host arch). NETBIRD_REF can be a tag, a branch, or a commit SHA.
# The Dockerfile default tracks the SHA the current patches are rebased against.
docker build -t goat/netbird-mgmt:patched \
    --build-arg NETBIRD_REF=0358be23136da50e829ba99a83e54ef555071a7f .

# Multi-arch (push to registry)
docker buildx build --platform linux/amd64,linux/arm64 \
    -t ghcr.io/usareur-af/netbird-mgmt:patched-0358be2 \
    --build-arg NETBIRD_REF=0358be23136da50e829ba99a83e54ef555071a7f --push .
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
2. Re-run the schema audit (`docs/design/ha-multi-site.md` §5.2.2).
3. Re-run any Phase 2 validation tests.
