# ADR 0857 — Overlay-patch the Ory consent UI to forward Kratos `traits.groups` into the JWT

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-05-17 |
| Repo scope | goat core |
| Supersedes | — |
| Superseded by | — |
| Related | ADR 0411 (ory-kratos-hydra-mvp), ADR 0842 (dedicated-operator-idp-for-observability), F-132 (services do not gate by Kratos groups claim) |

## TL;DR (for a layperson)

We log people in with Ory. Ory hands each service a small token that says "this is the user, here's their email and name." We need the token to also carry the list of groups the person belongs to ("admin", "viewer", etc.) so services like Grafana and netbird-dashboard can decide who gets what. The default Ory login UI doesn't include groups — it only forwards email, name, username, and website. We chose to ship a tiny modified version of that UI: same upstream image, plus three injected patches that (1) copy the groups across, (2) make sure the token's "audience" field gets populated correctly, and (3) make sure the token's "scope" field gets populated correctly. We picked this over building a custom login UI or running a separate token-rewrite service because it's the smallest possible change with the smallest possible blast radius, and it stays trivially recoverable if the upstream changes shape.

## Context

Site Hydra (Ory's OAuth2 server) mints JWTs by way of a consent flow: when a user authenticates, Hydra calls a separate "consent UI" (the `oryd/kratos-selfservice-ui-node` reference app) to read the user's identity from Kratos and decide what claims to put in the token. The upstream consent UI's `extractSession()` function hardcodes a whitelist of identity traits to forward — `email`, `email_verified`, `preferred_username`, `name` (with `given_name` / `family_name`), `website`, and `updated_at`. It does **not** forward `traits.groups`, the array we use across the project for every RBAC mapping (`ALL-ADMIN`, `GRAFANA-ADMIN`, `DASHBOARD-ADMIN`, future per-consumer groups under the operator-onboard utility per Block 83).

Concrete consequences before this decision: an observability goatnet's Grafana (Block 82B) had to ship a `GF_AUTH_GENERIC_OAUTH_ROLE_ATTRIBUTE_PATH="'Admin'"` literal pragma — every successful OIDC sign-in granted Grafana Admin — because the JMESPath had nothing to discriminate on. The netbird-dashboard role-based authorization (21D.2.f) was blocked on the same gap; operator-IdP per-consumer claim mapping (21H.3/4) had it as an unresolved "open question" pointed at this same decision. F-132 catalogs the broader pattern.

Identity schema is fine — the site Kratos schema at `ops/ansible/roles/ory/templates/ory/identity-schema.json.j2` already declares `traits.groups` as an array of uppercase-hyphenated tokens (`^[A-Z][A-Z0-9-]{0,40}$`). The operator-onboard utility (Block 83C.1+) sets the trait correctly. The break in the pipeline is between Kratos and the JWT — specifically, the consent UI's claim-construction step.

The decision in front of us was *how* to close that gap with the smallest defensible footprint, given that the same fix must work for every present and future site-Hydra OIDC consumer.

## Decision

We ship a small overlay-only Docker image, `ghcr.io/usareur-af/kratos-selfservice-ui-node-patched`, sourced from [`ops/ory-consent/`](../../ops/ory-consent/) and built by [`.github/workflows/ory-consent-image.yml`](../../.github/workflows/ory-consent-image.yml). The image:

- `FROM` the upstream `oryd/kratos-selfservice-ui-node` at a pinned digest, the same version as Kratos (v1.3.1 at adoption, v26.2.0 from the 26.2.0 upgrade; [ory-identity-upgrades.md](../design/ory-identity-upgrades.md)).
- Runs `apply-patch.sh` as `root` once at build time to inject a 3-line block into `/usr/src/app/lib/routes/consent.js` immediately before the existing `if (identity.updated_at)` handler. The block reads:

      if (Array.isArray(identity.traits.groups) && identity.traits.groups.length > 0) {
        session.id_token.groups = identity.traits.groups;
      }

- Restores the upstream image's `USER node` for runtime.
- Idempotency guard: the patch script greps for a marker comment (`goat-maintainer-groups-forwarding-patch`) and bails if already present — safe to re-run.

Consume:

- The `ory` Ansible role ([`ops/ansible/roles/ory/`](../../ops/ansible/roles/ory/)) renders `KRATOS_SELFSERVICE_UI_IMAGE` into `/opt/mgmt-host/.env` from the per-site `kratos_selfservice_ui_image` group_var. Pin-bump workflow = edit the var to a `<tag>@sha256:<digest>` form + re-run `<site>-ory.yml`; the existing `docker compose up -d` recreates the container on image change. No `.env` hand-editing.
- Already-affected users (stored consent sessions with broken contents from a pre-bump deploy — see F-144 / F-161) are flushed via the role's `ory_consent_revoke_subjects` list var: add `{ subject, client }` pairs to group_vars and the role DELETEs each via Hydra admin loopback at the end of the run. Idempotent on Hydra's side; leaving entries in group_vars on subsequent runs is harmless.
- Downstream consumers (Grafana, netbird-dashboard, future OIDC relying parties) use the standard `GF_AUTH_GENERIC_OAUTH_ROLE_ATTRIBUTE_PATH` / equivalent JMESPath against the `groups` claim — no per-consumer integration plumbing beyond standard OIDC config.
- Group taxonomy is uppercase + hyphens per the site Kratos identity schema (`^[A-Z][A-Z0-9-]{0,40}$`).

CI: per-arch native build (linux/amd64 + linux/arm64), multi-arch manifest combine, smoke-test gate via `tests/verify-patched.sh` that pulls the just-built image and confirms the marker comment is present in the compiled JS.

### Subsequent extensions

The original decision (May 2026) was for the single groups-forwarding patch. The same overlay-image substrate has since grown to carry two additional patches in the same shape — `apply-patch.sh` is idempotent across all three, and the smoke-test asserts each marker independently. The ADR remains accepted; the additions are downstream consequences of the same architectural choice (overlay-patch the compiled JS rather than build from source / run a webhook).

- **Patch 2 — audience-fallback** (2026-05-19, F-144). Wraps `grant_access_token_audience: body.requested_access_token_audience` at two call sites (skip-consent + full-consent handlers) with a fallback to `body.client.audience` when Hydra delivers the requested field empty. Without it, tokens are minted with `aud: []` and every JWT validator rejects them. Image tag bumped to `:v1.3.1-groups-aud`.
- **Patch 3 — scope-fallback** (2026-05-28, F-161). Same shape as patch 2, different field. Wraps `grant_scope: grantScope` at the same two call sites with a fallback to `body.client.scope.split(' ')` when grantScope is empty. The bug: for clients with `skip_consent: true`, Hydra delivers empty `body.requested_scope` in the consent challenge, upstream lands `grant_scope: []`, Hydra's `/userinfo` then strips every claim (including the groups claim patch 1 puts there). Grafana saw an empty groups claim and rejected logins despite the user having the right Kratos traits. Image tag bumped to `:v1.3.2-groups-aud-scope`. Companion helper: [`ops/scripts/revoke-stale-consent.sh`](../../ops/scripts/revoke-stale-consent.sh) for the one-shot flush of already-affected users' stale consent sessions post-deploy.

Both extensions inherit the same residual-risk and compensating-control story as the original — the patches widen the set of values that reach the JWT, and the operator-onboard utility plus the Kratos identity schema regex bound what values can land there.

## Alternatives considered

| Alternative | Reason rejected |
|---|---|
| **Hydra `token_hook` webhook** (run a small HTTP service that Hydra calls at token-mint time to inject custom claims) | Adds a long-lived service to the mgmt-stack with its own lifecycle, image, configuration, health-check, failure mode. The thing we want to add — a single field forwarded from Kratos identity to JWT — does not justify the operational surface. |
| **Fork `kratos-selfservice-ui-node` and rebuild from source** (custom TypeScript repo, npm dependency tree, our own release cycle) | We would inherit upstream's full release surface and have to track CVEs in `kratos-selfservice-ui-node` + every transitive npm dep. The change we need is 3 lines. The cost-benefit is upside-down. |
| **Custom consent app written from scratch** (any language, our own Hydra integration) | Largest blast radius. We would be re-implementing the Ory consent contract — every claim, every flow type, every error path — to add 3 lines. We would lose Ory upstream fixes to the consent flow itself. |
| **Carry the patch as a sidecar that rewrites the consent response in flight** (HTTP middleware) | Adds an in-band intermediary on the consent path that can fail. Same operational-cost issue as the webhook but worse because the failure mode is harder to reason about (consent-flow timeouts). |
| **Inline JMESPath workaround on every consumer** ("if user authenticated, assume Admin" as we did pre-decision) | Doesn't solve the problem. Closes the trial-fabric Grafana use case only; leaves dashboard RBAC + operator-IdP claim mapping permanently blocked; teaches consumers to drop their auth-discrimination capability. |

## Consequences

**Positive.**

- Smallest possible diff against upstream — three lines of injected JS in a compiled file with a marker-comment idempotency guard. A future maintainer can read [`apply-patch.sh`](../../ops/ory-consent/apply-patch.sh) and understand the entire delta in two minutes.
- Reusable across every site-Hydra OIDC consumer. The observability Grafana closed out under Block 82B-rbac. Block 21D.2.f netbird-dashboard role-based authz and Block 21H.3/4 operator-IdP per-consumer claim mapping are unblocked at the substrate layer; consumer-side work is JMESPath config only.
- Rollback is trivial: revert the `KRATOS_SELFSERVICE_UI_IMAGE` pin to the upstream digest and recreate the container. The downstream consumers can fall back to a literal-Admin JMESPath pragma during rollback if needed.
- Build is fully reproducible — no npm tree, no Node runtime invoked at build time, no transitive dep surface beyond what the upstream image already carries.
- The patch surface is small enough that upstream version tracking is mechanical: spawn a shell into the new image, confirm the anchor (`if (identity.updated_at)`) is still present in the compiled JS, bump the `UPSTREAM_IMAGE` digest, re-tag, re-publish. README covers the procedure.

**Negative.**

- We are now responsible for tracking upstream changes to the consent UI's compiled output. If `oryd/kratos-selfservice-ui-node` refactors `extractSession()` between releases such that the anchor line moves or its surrounding context changes, the awk patch will need re-authoring. Mitigation: idempotency guard means a bad patch is detectable at build time, and the smoke-test in CI greps for the marker so a silently-broken patch fails the build.
- The patch operates on compiled JavaScript, not TypeScript source. A future Ory release that ships a minified or differently-compiled output would invalidate the byte-level approach and force us to switch to building from source (a la the alternative we rejected). Likelihood: low — Ory has not historically minified the reference UI image — but worth flagging.
- The patch does not propagate to the upstream Ory project. Retirement follows the trigger conditions below; an upstream feature that forwards self-editable traits does not qualify.

**Residual risk and compensating control.** The patch widens the set of identity attributes that reach the JWT — if an operator inadvertently assigns sensitive groups to a Kratos identity, those leak via the JWT to every consumer using the patched UI. Compensating control: identity-schema groups are uppercase-only short tokens (no PII shape), every consumer's JMESPath is explicit about which group values it acts on, and group assignment goes through the operator-onboard utility (Block 83) with auditable manifests rather than ad-hoc admin-API calls.

## Assessor view (SCA-R)

The decision narrows, not widens, the implementation surface for OIDC role-based authorization across the system, and makes RBAC mapping consistent and auditable across every site-Hydra relying party. It does not introduce new authentication paths, new key material, or new external integrations. It does not change the FIPS/PQC posture (no new crypto), nor does it change the audit pipeline (Hydra's existing `token.issued` audit event continues to capture token claims). Mapping to NIST SP 800-53: AC-2 (account management — the trait that drives role assignment is canonical at the Kratos identity record), AC-3 (access enforcement — consumers now have a discriminator to enforce against), AC-6 (least privilege — `STRICT=true` JMESPath rejects unmapped identities outright rather than defaulting to a permissive role), AU-2 / AU-3 (audit content — JWT claims are part of the existing audit trail). No waiver required; no compliance-relevant external posture change.

## Trigger conditions for revisit

- Ory upstream forwards the admin-managed `metadata_public.groups` and `metadata_public.aff` into the ID token, from metadata only — retire patches 0001 and 0004. A feature that forwards identity **traits** is not a trigger: traits are self-editable through the settings flow, so forwarding them lets a user mint their own groups (F-572, GHSA-9v3v). Upstream's `SESSION_EXTRA_TRAITS_ID_TOKEN` (ory/kratos-selfservice-ui-node commit `08f01849a6`, not in a release as of 26.2.0) is that kind of feature; patches 0001 and 0004 stay. Patches 0002, 0003 and 0005 retire each on its own when upstream ships the equivalent behaviour.
- A future upstream release of `oryd/kratos-selfservice-ui-node` moves or removes the `if (identity.updated_at)` anchor line — re-author the patch with a different anchor, or switch to a source rebuild if the layout has changed substantially.
- The set of identity-derived claims we need to forward grows past what an in-image patch can cleanly express (e.g., complex aggregate claims, claim values computed from cross-identity lookups) — promote to a real claim-mapping service or the Hydra `token_hook` webhook we rejected here.
- A second cluster operator adopts the consent UI and needs different forwarding behavior — generalize the patch to read from a config file rather than hardcoding which traits to forward.
