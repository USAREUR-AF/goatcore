# ops/ory-consent/Dockerfile
#
# Patches oryd/kratos-selfservice-ui-node's consent handler with three
# in-place transformations:
#
#   1. patches/0001-forward-traits-groups-to-id-token.patch
#      Upstream only forwards email / username / website / name from the
#      Kratos identity; the `groups` trait (which every downstream OIDC
#      consumer in goat-net needs for RBAC) drops on the floor. Patch 1
#      adds groups-forwarding into session.id_token.groups.
#
#   2. patches/0002-fallback-audience-to-client-audience.patch
#      Upstream echoes Hydra's body.requested_access_token_audience back
#      verbatim as grant_access_token_audience on the consent accept.
#      Observed empirically: Hydra delivers that field empty even
#      when the SPA requests an audience, so tokens land with aud:[] and
#      every JWT validator rejects them. Patch 2 falls back to the
#      client's whitelisted audience array when the requested value is
#      empty.
#
#   3. patches/0003-fallback-scope-to-client-scope.patch
#      Same shape as patch 2, different field. For OAuth2 clients with
#      `skip_consent: true`, Hydra delivers an empty
#      body.requested_scope, the upstream code lands grant_scope: [] in
#      the stored consent session, and Hydra strips every claim from
#      /userinfo for scopes that weren't granted — including the groups
#      claim patch 1 puts there. Patch 3 falls back to client.scope when
#      grantScope is empty. See F-161 (Grafana "user not a member" gap).
#
#   5. patches/0005-prevent-double-submit-of-the-login-form.patch
#      A login form submitted TWICE orphans the OAuth2 login challenge: the
#      second POST lands on an already-consumed flow, Kratos answers it with a
#      redirect to default_browser_return_url, and that navigates the browser
#      off the challenge-bearing step-up URL the first submission had correctly
#      returned. The operator lands on Ory's stock welcome page instead of the
#      app. Reproduced deterministically. Unlike 1-4 this
#      patch targets a VIEW (views/partials/scripts.hbs), so it is applied by a
#      separate script.
#
# Strategy: overlay-only. FROM the published upstream image at a pinned
# tag (sha-digest baked into mgmt-host compose default), apply all three
# patches to /usr/src/app/lib/routes/consent.js in-place. No npm rebuild,
# no node toolchain in the final image — keeps the diff against upstream
# minimal and easy to reason about.
#
# Source-of-truth for the actual transformations: ./apply-patch.sh
# (idempotent — re-running on an already-patched file is a no-op for
# all three patches via marker-comment detection). Companion test:
# ./tests/verify-patched.sh greps the running container's consent.js to
# confirm ALL THREE patch markers are present.
#
# Build (local arm64 — Thor is aarch64):
#   docker buildx build --platform linux/arm64 \
#     --tag ghcr.io/usareur-af/kratos-selfservice-ui-node-patched:v1.3.2-groups-aud-scope \
#     --push \
#     ops/ory-consent/
#
# Build (multi-arch for future amd64 obs hosts):
#   docker buildx build --platform linux/arm64,linux/amd64 \
#     --tag ghcr.io/usareur-af/kratos-selfservice-ui-node-patched:v1.3.2-groups-aud-scope \
#     --push \
#     ops/ory-consent/
#
# Consume (mgmt-host compose .env):
#   KRATOS_SELFSERVICE_UI_IMAGE=ghcr.io/usareur-af/kratos-selfservice-ui-node-patched:v1.3.2-groups-aud-scope@sha256:<digest>

ARG UPSTREAM_IMAGE=oryd/kratos-selfservice-ui-node:v1.3.1@sha256:2d6f9a84456c0e6797664e65f2a743f695794788f1e276f915d247f9e35684a3
FROM ${UPSTREAM_IMAGE}

USER root

# Same Alpine release and application; update only affected runtime libraries.
RUN apk add --no-cache --upgrade \
      libcrypto3=3.0.19-r0 \
      libssl3=3.0.19-r0 \
      musl=1.2.3-r6 \
      musl-utils=1.2.3-r6

COPY apply-patch.sh /usr/local/bin/apply-patch.sh
COPY apply-view-patch.sh /usr/local/bin/apply-view-patch.sh
RUN chmod +x /usr/local/bin/apply-patch.sh /usr/local/bin/apply-view-patch.sh \
 && /usr/local/bin/apply-patch.sh /usr/src/app/lib/routes/consent.js \
 && /usr/local/bin/apply-view-patch.sh /usr/src/app/views/partials/scripts.hbs \
 && rm /usr/local/bin/apply-patch.sh /usr/local/bin/apply-view-patch.sh

# Restore the upstream image's runtime user.
USER node
