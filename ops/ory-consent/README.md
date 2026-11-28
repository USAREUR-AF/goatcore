# `ops/ory-consent/` — patched Kratos selfservice-ui consent handler

Overlays `oryd/kratos-selfservice-ui-node` with three additive in-place patches:

1. **groups-forwarding** — forwards the admin-managed Kratos `metadata_public.groups` (never self-editable `traits.groups`, GHSA-9v3v) into the OIDC JWT `id_token.groups` claim so downstream consumers (Grafana, netbird-dashboard, future relying parties) can do real groups-based RBAC instead of every-authenticated-user-is-admin pragmas.
2. **audience-fallback** — falls back to `body.client.audience` when Hydra delivers an empty `requested_access_token_audience` in the consent challenge body, so minted tokens carry the `aud` claim every JWT validator requires. (F-144)
3. **scope-fallback** — falls back to `body.client.scope.split(' ')` when `grantScope` is empty on `acceptOAuth2ConsentRequest`. Same shape as patch 2: for clients with `skip_consent: true`, Hydra delivers an empty `body.requested_scope`, upstream lands `grant_scope: []` in the stored session, and Hydra then strips every claim (including `groups` injected by patch 1) from `/userinfo`. Downstream consumers see an empty userinfo and reject the login. (F-161)

## Why this exists

### Patch 1: groups-forwarding

`oryd/kratos-selfservice-ui-node` is the reference consent UI for Hydra. Its `extractSession()` function hardcodes a small whitelist of identity traits to forward into the JWT — `email`, `username`, `website`, `name`. The `groups` trait is omitted, so every JWT minted via Hydra-Kratos drops the groups claim before it reaches Grafana / dashboard / any other relying party.

Patch 1 adds a 3-line block to `extractSession()` that forwards `identity.traits.groups` into `session.id_token.groups` when present and non-empty. Additive only — identities without a `groups` trait see no change.

### Patch 2: audience-fallback

The upstream consent handler accepts a consent request via PUT `/admin/oauth2/auth/requests/consent/accept` and echoes Hydra's `body.requested_access_token_audience` back as `grant_access_token_audience` verbatim. The intended chain is: SPA passes `audience=netbird-dashboard` on /oauth2/auth → Hydra captures it in the login + consent challenge bodies → consent UI echoes it back → Hydra mints token with that `aud`.

Observed empirically on nova-a: even when the SPA correctly sends `audience=netbird-dashboard` (verified at the Traefik access log), Hydra's consent-challenge body delivers `requested_access_token_audience: []` to the consent UI. Result: tokens land with `aud: []`. mgmt-a's JWT validator (and every other consumer that requires the `aud` claim) rejects every browser-issued token with "token invalid"; SPA retries → user sees 10+ second slow loads with empty data.

Patch 2 wraps the audience-echo with a fallback: when `body.requested_access_token_audience` is empty, use `body.client.audience` — the client's whitelisted-audience array from its OIDC client registration. For our first-party clients this array is exactly the audience we want minted into tokens. Additive — when Hydra DOES populate the field correctly (upstream regression fixed), the original value wins.

### Patch 3: scope-fallback

Same shape as patch 2, different field. For OAuth2 clients with `skip_consent: true` (the F-077 mitigation for first-party clients to auto-grant consent and avoid the "consent UI default-untickts every scope" footgun), Hydra delivers `body.requested_scope` empty in the consent challenge. The upstream code lands `grantScope = body.requested_scope || []` = `[]` and passes that empty array to `acceptOAuth2ConsentRequest` as `grant_scope: []`.

The downstream effect is silent and ugly: the stored consent session in Hydra correctly carries the groups claim patch 1 injects into `session.id_token`. BUT Hydra's `/userinfo` endpoint strips ALL claims for scopes that weren't granted — so with `granted_scope: null` in the stored session, NO claims survive the userinfo round-trip. Grafana's `GF_AUTH_GENERIC_OAUTH_ALLOWED_GROUPS` check sees an empty `groups` claim and rejects with `user not a member of one of the required groups`.

Patch 3 wraps the `grant_scope` echo on `acceptOAuth2ConsentRequest` with a fallback at both call sites: when `grantScope` is empty, use `body.client.scope.split(' ')` — the OAuth2 client's registered scope list from its Hydra client config. Additive in the same way as patch 2 — when grantScope is populated (non-skip-consent clients, or upstream regression fixed), the original value wins.

Note that this patch DOES NOT require running an additional flow for already-affected users: the renderer-local + in-RAM contract isn't relevant here — Hydra's stored consent sessions persist in Postgres and stay around. Already-affected users (whose first login wrote a session with empty granted_scope) need a one-time consent revoke on Hydra after the image bump so their next login re-runs through the new code. Helper at `ops/scripts/revoke-stale-consent.sh`.

## Tracking

- **Source-of-truth track:** Block 82B-rbac (this directory's PR closes out the literal-`'Admin'` JMESPath pragma in `ops/compose/nova-obs/docker-compose.yml` that landed under Block 82B).
- **Unblocks:** Block 21D.2.f (netbird-dashboard role-based authorization v2), Block 21H.3 / 21H.4 (operator-IdP per-consumer claim mapping).
- **Site Kratos identity schema** (`ops/ansible/roles/ory/templates/ory/identity-schema.json.j2`) already declares `traits.groups` as an array of `^[A-Z][A-Z0-9-]{0,40}$` strings. No schema change needed.

## Files

| File | Purpose |
|---|---|
| `Dockerfile` | Multi-arch overlay — `FROM oryd/kratos-selfservice-ui-node:v26.2.0@sha256:…` (same version as Kratos); runs `apply-patch.sh` against the in-image compiled JS as root, restores the upstream runtime user |
| `apply-patch.sh` | Idempotent awk patches — applies all three patches in sequence: anchors on `if (identity.updated_at)` to insert the groups-forwarding block, literal-string-replaces the two `grant_access_token_audience: body.requested_access_token_audience,` lines with the audience-fallback wrapper, and literal-string-replaces the two `grant_scope: grantScope,` lines with the scope-fallback wrapper. Re-running on an already-patched file is a no-op per patch (marker comment guards) |
| `patches/0001-forward-traits-groups-to-id-token.patch` | Reference patch 1 in standard git-am format. Documents intent; not applied directly by the build (the build uses `apply-patch.sh` against the compiled JS) |
| `patches/0002-fallback-audience-to-client-audience.patch` | Reference patch 2 in standard git-am format. Documents the audience-fallback transform. Same shape — not applied directly; `apply-patch.sh` performs the equivalent in-place rewrite |
| `patches/0003-fallback-scope-to-client-scope.patch` | Reference patch 3 in standard git-am format. Documents the scope-fallback transform. Same shape as patch 2; not applied directly |
| `tests/verify-patched.sh` | Spawns the built image and greps for all three marker comments in `/usr/src/app/lib/routes/consent.js`. Used by CI smoke test |

## Build (CI)

[`.github/workflows/ory-consent-image.yml`](../../.github/workflows/ory-consent-image.yml) builds per-arch on native GitHub runners (amd64 + arm64) and combines into a multi-arch manifest. Triggers:

- Push to `main` touching `ops/ory-consent/**`
- Tag `ory-consent/v*`
- PR touching `ops/ory-consent/**`
- `workflow_dispatch`

Published to `ghcr.io/usareur-af/kratos-selfservice-ui-node-patched`.

## Build (local, for iteration)

```bash
docker buildx build \
  --platform linux/arm64,linux/amd64 \
  --tag ghcr.io/usareur-af/kratos-selfservice-ui-node-patched:v1.3.2-groups-aud-scope \
  --push \
  ops/ory-consent/
```

Requires `docker login ghcr.io` (GHCR PAT with `write:packages`). Built tags are version-suffixed; the CI workflow additionally publishes `:latest` and `:sha-<short>`.

## Consume

Pin via the `kratos_selfservice_ui_image` var in the site's `group_vars/<site>_hosts/vars.yml`. The `ory` Ansible role renders it into `/opt/mgmt-host/.env`'s `KRATOS_SELFSERVICE_UI_IMAGE` line via its existing `lineinfile` task; the subsequent `docker compose up -d` recreates the container on image change. Pin-bump workflow:

```yaml
# group_vars/<site>_hosts/vars.yml
kratos_selfservice_ui_image: "ghcr.io/usareur-af/kratos-selfservice-ui-node-patched:sha-<short>@sha256:<digest>"
```

Then `ansible-playbook -i inventory/mgmt-hosts.ini <site>-ory.yml`. Always pin a digest (not just a tag) so deploys are deterministic across image promotions.

**Already-affected user flush.** After a pin-bump where the new image changes how consent sessions are written, users whose first login wrote a stored session under the OLD code keep hitting the same failure mode. List them under `ory_consent_revoke_subjects` in group_vars:

```yaml
ory_consent_revoke_subjects:
  - { subject: "<kratos-id>", client: "<oauth2-client-id>" }
```

The role DELETEs each via Hydra admin loopback at end of run. Idempotent — leaving entries here on subsequent runs is harmless.

After swap, downstream consumers can replace their literal-Admin pragmas with real JMESPath role mapping against `groups`. For Grafana:

```yaml
GF_AUTH_GENERIC_OAUTH_ROLE_ATTRIBUTE_PATH: "contains(groups, 'OBS-ADMIN') && 'Admin' || contains(groups, 'GRAFANA-ADMIN') && 'Admin' || contains(groups, 'OBS-READONLY') && 'Viewer' || contains(groups, 'GRAFANA-READONLY') && 'Viewer'"
GF_AUTH_GENERIC_OAUTH_ROLE_ATTRIBUTE_STRICT: "true"
```

(Uppercase per the site Kratos identity schema pattern `^[A-Z][A-Z0-9-]{0,40}$`.)

## Upstream tracking

When `oryd/kratos-selfservice-ui-node` cuts a new version:

1. Spawn an interactive shell into the new upstream image: `docker run --rm -it --entrypoint=/bin/sh oryd/kratos-selfservice-ui-node:<new-tag>`
2. Re-grep `/usr/src/app/lib/routes/consent.js` for all three anchors:
   - Patch 1: `if (identity.updated_at)`
   - Patch 2: `grant_access_token_audience: body.requested_access_token_audience,`
   - Patch 3: `grant_scope: grantScope,`
   If all three are still there, bump `UPSTREAM_IMAGE` arg in the Dockerfile and re-run CI — done.
3. If any anchor moved, update `apply-patch.sh` to use a new stable anchor for the affected patch.
4. If upstream fixes either round-trip (Hydra populates `requested_access_token_audience` or `requested_scope` correctly, OR the consent UI starts using `body.client.audience` / `body.client.scope` natively), the corresponding patch becomes load-free — the original-value-wins branch always fires. Worth re-validating against nova-a then; consider deprecating the affected patch once both halves of the round-trip work end-to-end without our wrapper.
5. Tag the result `ory-consent/v<upstream-tag>-groups-aud-scope`.

A standard upstream-tracking ADR pattern (similar to the zenoh image build at `ops/zenoh/image/`) applies here — re-pin the upstream digest on every consume bump, and document the anchor invariant.

## Rollback

Compose `.env` revert: drop the override line; mgmt-host compose defaults back to upstream `oryd/kratos-selfservice-ui-node:v26.2.0@sha256:46a7bac1…`. `docker compose up -d selfservice-ui` recreates the container with the upstream image. Downstream consumers (Grafana, dashboard) will then see JWTs without the `groups` claim — restore their pre-patch role mapping to keep them working.
