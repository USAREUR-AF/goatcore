# Identity, Access & Onboarding — authoritative reference

**Status:** ground-truth reference, built by reading the code + SoT (not inference).
Every load-bearing claim cites `path:line`. When this doc and your intuition
disagree, **read the cited file** — the code is the source of truth, and this doc
is only current as of its citations. Drifts + unverified corners are listed at the
end; do not silently trust a claim that isn't cited.

**Why this exists:** identity/access here spans several *independent* credential
planes that look interchangeable and are not. Guessing which plane drives a given
surface is the single biggest source of wasted time and wrong changes (e.g. "make
someone platform-admin on the CLI" is *not* a cert/`.cbor` change, *not* an Ory
group change, *not* a data-role grant — it is one specific bearer key). This doc
maps the planes so nobody has to infer.

---

## 0. BLUF — the one thing to internalize

There are **six credential planes**. They are distinct by design and do **not**
cross-authenticate. A principal can hold several; each surface reads exactly one.

| Plane | The credential | Where it lives | What reads it |
|---|---|---|---|
| **Ory human** | Kratos identity + `groups[]` trait → JWT `groups` claim | Kratos (Postgres); presented as an oauth2-proxy cookie/JWT | Web consoles + panoscope RBAC |
| **NetBird machine** | WireGuard identity; setup-key (enrol) + `nbp_` PAT (admin API) | host WG keys; PAT in `<SITE>_NB_API_TOKEN` env | the mesh; `goat admin peer` |
| **Zenoh fabric** | X.509 client cert (the `mtls_cert` in the `.cbor`) | `~/.goat/<ctx>/mtls.*.pem` | the Zenoh router wire ACL (keys on **cert CN**) |
| **Portal API** | a Bearer token (`portal_token` in the `.cbor`) | `~/.goat/<ctx>/portal.token` | the portal `/api/*` — **operator vs vendor** |
| **Workload (SPIFFE)** | X.509-SVID | SPIRE agent; Workload API socket | the Zenoh router (admitted by SVID **CN**) |
| **Collab (off-mesh)** | a person account in the collab IdP (Dex + Kanidm) | the collab Kanidm, reachable without mesh transport | the collab apps (Outline / filedrop / chat) — **nothing on the mesh** |

The collab plane is the one that is deliberately **outside** the mesh boundary, so the
mesh cut (ADR 1031) does not reach it. What was not deliberate — F-274 —
is that the mesh recorded *nothing* about who held collab access. It now records it
(§2.9) and can reconcile it, without either plane governing the other.

**The portal API token and the Zenoh cert are deliberately distinct credentials**
— "the cert authenticates to the fabric, the API key only to the portal"
(`ops/goat-zenoh-portal/internal/api/auth.go:9-11`). Both travel inside the same
`.cbor` bundle but authenticate different things.

**Two "admin" group names on two different axes — never conflate them**
(`ops/ci/check-ory-groups-enum.sh:4-11`):
- `ADMIN` = a **NetBird mesh-role** auto_group (SoT `ops/ansible/netbird_acl_taxonomy.yml`).
- `ADMINS` = an **Ory service-permission** group (SoT `ops/permissions/group-permissions.yml`).
They differ only by a trailing `s`. Both have since been renamed to `PLATFORM-ADMIN` in
their SoTs (ADR 1073 / `admins-to-platform-admin-rename-plan.md`); on the Ory side `ADMINS`
is still accepted as the legacy alias, and panoscope's compiled-in `full_groups` default is
still `{"ADMINS"}` (`ops/panoscope/src/viewer.rs:135`); see §8.

---

## 1. What the `.cbor` actually is

`goat profile init <bundle>.cbor` consumes a **signed CBOR bundle** (ASN.1 DER
ECDSA P-256 over the canonical CBOR; the verifying CA root is operator-supplied at runtime, not pinned at build time — F-406)
(`crates/goat-cli/src/bundle.rs:1-7, 371-389, 245-262`). The load-bearing payload is
the `goat_cli_extras` map — six required fields plus an optional `onboard_endpoint`
(`crates/goat-cli/src/bundle.rs:58-70`):

```
portal_endpoint   text   the portal HTTPS URL
portal_token      bytes  the portal Bearer token        ← THIS decides CLI powers (§4)
router_endpoint   text   e.g. tls/router:7447
mtls_cert         bytes  the Zenoh client cert PEM       ← THIS decides fabric writes (§5)
mtls_key          bytes  the Zenoh client private key PEM
ca_roots          bytes  CA roots PEM
```

On `init` these split onto disk under `$GOAT_PROFILE_DIR`/`$HOME/.goat`
per-context (`crates/goat-cli/src/profile.rs:62-115`): `profile.toml`
(`portal_endpoint`, optional `operator_handle`), `mtls.cert.pem`, `mtls.key.pem`,
`ca-roots.pem`, `portal.token`, `bundle.cbor`. Bundle lifecycle gates
(`activation_deadline`, `expires_at`) are enforced at init
(`crates/goat-cli/src/bundle.rs:280-286, 337-353`).

**Consequence:** the `.cbor` is a *participant/org* client bundle. Its `portal_token`
is normally a **vendor/org** key → org-scoped. It does **not** carry operator
power, and re-cutting it does not add operator power (§7). Operator power is a
*different token* that is not distributed in the participant `.cbor`.

---

## 2. Credential inventory (what / where / presented / authenticates)

### 2.1 Ory/Kratos human identity + `groups` claim
- Groups live in the Kratos identity trait `traits.groups` (`[]string`)
  (`ops/goat-id-console/internal/kratos/client.go:146-162`), written by JSON-Patch
  `{"op":"add","path":"/traits/groups",...}` (`client.go:340-342`).
- The schema admits **two value shapes** via one regex: UPPERCASE platform groups
  (`ADMINS`, `IAM-ADMIN`, …) and lowercase data-grammar values
  (`data.view.all`, `data.view.org:<p>`, `data.write.producer:<k>`)
  (`ops/ansible/roles/ory/templates/ory/identity-schema.json.j2:45`).
- The patched Ory consent UI forwards `traits.groups` → the JWT `groups` claim
  (`ops/ory-consent/apply-patch.sh:5-7`). Web surfaces authorize on that claim.
- UPPERCASE group SoT: `ops/permissions/group-permissions.yml` (CI-fenced,
  `ops/ci/check-ory-groups-enum.sh:14-27`). Lowercase `data.*` SoT:
  `ops/permissions/data-access-roles.yaml`.

### 2.2 NetBird machine identity
- WireGuard identity + enrolment provenance in the netbird-mgmt Postgres (the
  netbird source is a sibling repo, not this tree — asserted from prose
  `docs/design/identity-architecture.md:30,144`, **not** code-verified here).
- **Setup key** (enrol): `sudo netbird up --setup-key $NETBIRD_SETUP_KEY`
  (`docs/claudeoverflow.md:296`).
- **PAT** (`nbp_…`, admin API): `Authorization: Bearer <PAT>`
  (`ops/goat-zenoh-portal/internal/netbirdpoll/netbirdpoll.go:57,108`); goat-cli
  reads it env-only (`GOAT_NB_API_TOKEN` then `<CTX>_NB_API_TOKEN`), never from the
  profile (`crates/goat-cli/src/profile.rs:383-395`).
- Mesh auto_groups (the role axis, e.g. `ADMIN`/`DEVELOPER`/`MONITOR`/`OBSERVER`):
  SoT `ops/ansible/netbird_acl_taxonomy.yml`.
- **Interactive SSO (PKCE)**. `netbird login` /
  `netbird up` without a setup key, and the JWT `netbird ssh` needs, come from the
  `PKCEAuthorizationFlow` block in `ops/ansible/templates/management.json.j2`,
  switched on per-goatnet by `netbird_pkce_client_id`. It points at the site Hydra
  through a PUBLIC OAuth2 client (`netbird-cli`, `token_endpoint_auth_method: none`)
  registered by `ops/ansible/zagros-prod-oidc-clients.yml`. Both halves are load-bearing:
  the client alone changes nothing, the config alone fails every login at Hydra.
  Deploy a change to that block with `ops/ansible/mgmt-config-refresh.yml`, never the
  full bootstrap play.
  - The flow is PKCE (RFC 7636) and not device-code (RFC 8628) because our Hydra pin
    `v2.3.0` ships the device grant **disabled** — its discovery doc advertises no
    `device_authorization_endpoint`. Enable it in Hydra before anyone reaches for
    `DeviceAuthorizationFlow`.
  - The client's whitelisted audience is `auth_audience` (`netbird-dashboard`),
    deliberately not its own `client_id`: management validates API tokens against
    `HttpConfig.AuthAudience`, and its SSH server derives the SSH JWT audience list
    from that same field, so one audience satisfies both.

### 2.3 Zenoh fabric client cert
- Minted by the portal certmint with URI-SAN `urn:goat:<env>:org=<anchor>` (ZAGROS) /
  `urn:goat-nova:vendor=<vendor>` (nova) and **CN `<prefix>-trial-peer`** (slot mode:
  CN = the bare `<anchor>` uuid)
  (`ops/goat-zenoh-portal/internal/certmint/certmint.go:53,155-156,193`).
- **The wire ACL keys on the CN, NOT the URI-SAN** — Zenoh 1.9.0 has no URI-SAN
  matcher (`ops/ansible/roles/goat-zenoh/templates/config.json5.j2:532-533,1299-1304`).
  The URI-SAN is the frozen audit/identity stamp, not the match axis.
- Presented as the mTLS client cert on the router TLS handshake (`tls/…:7447`).

### 2.4 Portal operator key (`PORTAL_OPERATOR_API_KEYS`)
- Per-person super-user Bearer keys. Config
  `OperatorAPIKeys map[string]string` (handle→key), env form
  `handle1:key1,handle2:key2` (`ops/goat-zenoh-portal/internal/config/config.go:193-201,432`).
  A legacy single `PORTAL_OPERATOR_API_KEY` → handle `"operator"` (`goat-zenoh-portal/internal/config/config.go:184-191`).
- A persistent on-host dir (`PORTAL_OPERATOR_API_KEYS_DIR`) merges keys so a laptop
  redeploy can't drop them, and is **re-read while the portal runs** (default 30s,
  `PORTAL_OPERATOR_API_KEYS_RELOAD`;
  `ops/goat-zenoh-portal/internal/config/operator_key_store.go`). Adding a file
  grants, removing one revokes; env keys stay frozen at boot and the directory can
  neither redefine nor revoke them. A read FAILURE keeps the last good set, while a
  successful smaller read is honoured as revocation.
- **This is no longer a service-only tier.** It used to be, on the reasoning that
  human keys must stay env-managed "so they remain removable" — sound while the
  directory was startup-only, since a file nobody notices the removal of is a
  credential nobody can retire. With the directory live it is the right home for
  human keys, which is what makes self-service possible.
- Issued and revoked through goat-id-console — `/operator-key` (request),
  `/onboarding/operator-keys` (approve/reject/revoke), `/api/onboard/operator-keys`
  for the machine path, `goat admin operator-key` on the CLI. Approving writes the
  file and rolls back if the write fails; revoking removes the file BEFORE the
  record changes. Requesting mints nothing; the value is delivered once and then
  dropped from the console database, leaving only a fingerprint.
- Presented as `Authorization: Bearer <key>` to the portal `/api/*`. **This is the
  operator lever** (§4).

### 2.5 Console `ONBOARD_OPERATOR_API_KEYS` (goat-id-console)
- The machine-realm per-handle operator-key map for the console onboard API
  (`ops/goat-id-console/internal/config/config.go:108-118,368`). A presented key that
  is *not* an operator key is delegated read-only to the portal `GET /api/whoami`
  to resolve a participant's own key + ADR 1071 sponsorship cap
  (`ops/goat-id-console/internal/handlers/onboard_api.go:93-174`). Fail-closed: no
  keys + no portal delegation → 503; unknown → 401 (`onboard_api.go:188-205`).

### 2.6 Zenoh OPERATOR cert (`operator-cert.pem`/`operator-key.pem`)
- CN `operator-trial-peer`; grants ingress (publish) into `ops/**`
  (`ops/ansible/roles/goat-zenoh/templates/config.json5.j2:911-917,1359-1360`).
  Least-privilege health leaves exist per namespace (`ops/fabric/**`, `ops/time/**`,
  …) (`config.json5.j2:994-1173`). `ops/audit/operator/**` is deny-tree'd for the
  universal-read subject (`config.json5.j2:783-787`).
- This is the fabric-write side of operator enablement (§7); it is **not** what
  makes `goat admin` work — that is the portal operator key (§4).

### 2.7 Vendor portal API key
- Per-vendor Bearer to the portal `/api/*`, stored bcrypt-hashed as `api_key_hash` and,
  since F-371, as the indexed digest `api_key_sha256` that authentication looks up
  (`ops/goat-zenoh-portal/internal/api/auth.go:6-7,203-245`). Resolves to
  `Principal{Vendor: prefix}` (org-scoped). This is the token a normal participant
  `.cbor` carries.

### 2.8 SPIFFE/SPIRE SVID
- The SPIRE trust domain comes from mandatory deployment values
  (`ops/ansible/roles/spire/defaults/main.yml:14`).
- **Composes with the fabric by CN**: SPIRE's first `-dns` becomes the SVID Subject
  CN (`ops/ansible/roles/spire/tasks/main.yml:420-425`). The router folds configured
  extra trust roots into its CA bundle
  (`ops/ansible/roles/goat-zenoh/tasks/main.yml:474-490`).
- Image upgrades use the [staged upgrade and restore contract](spire-upgrades.md).
  Persisted enrollment and Workload API health are separate checks in the role.

### 2.9 Collab account (off-mesh, recorded but never governed)
- A person account in the collab IdP (Dex + Kanidm), provisioned by the onboard-runner's
  collab leg. Default OFF per goatnet (`onboard_runner_collab_enabled`).
  Detail: [`collab-onboarding-provisioner.md`](collab-onboarding-provisioner.md).
- **The mesh-side record is the manifest's `collab` block** — `enabled` / `provider` /
  `activated` / `activated_at` / `account`, schema-defined at
  `ops/operators/schema/operator-manifest.schema.json:671-700` and carried through the
  Rust manifest type (`crates/goat-cli/src/cmd/admin.rs:2932-2937`). Read-only awareness:
  no enforcement path consults it.
- **Reconcile, don't govern:** `onboard-runner -collab reconcile` joins the provider's
  live account list against the manifests plus that goatnet's `docs/operators/<goatnet>/roster.json` (ADR 1172) and reports a
  five-class delta (`ops/onboard-runner/collabrecon.go:273-285`) — the loudest being
  `revoked-identity-live-account`. It reports; it never cuts. `-emit-backfill` writes the
  observed accounts as an appliable patch set for `-collab backfill`.
- **The cut is an explicit act.** `onboard-runner -collab revoke -handle <h> -yes` is the
  only thing that disables an account; `goat admin onboard --revoke` makes no collab call.
  The operator's decision is carried by `--collab keep|revoke`
  (`crates/goat-cli/src/cmd/admin.rs:406-418, 4385-4399`) and lands on the revoke receipt
  + the `admin.revoke-native` audit event; omitting the flag for someone who has collab
  records `collab=undecided` rather than silently resolving to "keep".

---

## 3. Onboarding — the three-stage candidate flow

> **There is exactly one live enrollment path, and it is this one.**
> `ops/ansible/operator-onboard.yml`'s **enroll** leg is RETIRED (July 2026) and
> now refuses at its "refuse the retired enroll leg" step. It hand-minted every surface and emitted an
> out-of-band bundle containing the person's **private key** for an operator to
> carry by hand — the live path never puts someone's key in an operator's hands,
> and never skips the sponsor. Its **revoke** leg (`-e action=revoke`) is still
> the live offboarding mechanism (ADR 1031) until native revoke lands.
>
> **`ops/operators/<env>/<handle>.yml` is an OUTPUT of this flow**, written by
> `ops/scripts/sync-onboarded-manifests.py` from the portal/console DB after the
> runner mints, so the declared roster matches reality. It is not an input you
> author. Every one of those files says so in its own first line. **Copying one
> as a template is the regression** — see the retired-path section of
> [three-stage-candidate-onboarding.md](three-stage-candidate-onboarding.md).


The onboarding **engine lives in `goat-id-console`** (extracted from the portal;
`onboarding-extraction-plan.md`). Stages + state machine (`onboarding.go:35-55`,
`decideProposal` `onboarding.go:1404-1573`):

1. **Front desk / submit** — public, invite-token-gated (no ADMINS): the enrollee
   submits via `GET /onboard.html` → `POST /api/onboard/submit`, creating
   an **inert `proposed` proposal** (`ops/goat-id-console/internal/handlers/onboard_public.go:27,406`).
   The public front desk `intake.<zone>` is a thin nginx collector that forwards to
   the console's public endpoints
   (`ops/ansible/roles/onboard-intake-collector/`). An enrollee-supplied escalated
   tier is refused fail-closed (`onboard_effective.go:41-78`).
2. **Sponsor vouch** — `proposed → sponsor_approved` (`sponsor-approve`); a sponsor
   acts only on their own invitees within their tier cap (`onboarding.go:1039-1060,1474-1499`).
3. **Admin mint approval** — `sponsor_approved → approved` (`approve`), re-lints
   first (`onboarding.go:1509-1559`). The admin does **not** mint; `approved` is a
   queue state (`onboarding.go:25-27`).

**The onboard-runner** polls `approved`, atomically claims (`approved → onboarding`),
writes the manifest to `ops/operators/<env>/<handle>.yml`, and execs the native
mint `goat admin onboard <handle> --native --env <env>` (`ops/onboard-runner/runner.go:100-436`).
`--native` is forced (`runner.go:407-418`).

**What a completed mint produces** (`crates/goat-cli/src/cmd/admin.rs`,
`onboard_native` `admin.rs:4758`; the leg ORDER is documented at `admin.rs:4749-4757`
— NetBird first, Zenoh second, Ory last, so an Ory failure still leaves a usable
bundle):
1. **NetBird setup key** (first leg) via the mgmt API (`provision_netbird_native`
   `admin.rs:3318`, `create_setup_key` `admin.rs:3397-3410`); needs the goatnet's
   `token_env_var` (`admin.rs:3367-3375`; `ZAGROS_NB_API_TOKEN` for ZAGROS, `admin.rs:3115`).
2. **Zenoh slot + cert + key + portal api_key** (only if `zenoh.enabled` — browse skips
   it, `admin.rs:4808`) via `POST /api/orgs` (`portal.onboard_org` `admin.rs:4976`).
3. **Ory Kratos login identity** (last leg) over SSH to the site Kratos admin API
   (`provision_ory_native` `admin.rs:3759`; the admin-URL resolution + SSH hop at
   `admin.rs:3435-3457, 3515-3519`).
4. **The `.cbor` bundle** — minted by the portal certmint (not the runner) via the
   `bundle-create` binary (`ops/goat-zenoh-portal/internal/certmint/certmint.go:217-262`;
   a goatnet wires `portal_bundle_create_bin` in its environment repository's `group_vars/<site>_hosts/vars.yml`).
5. **Bundle markdown** `ops/operators/out/<handle>-bundle.md` (setup key + api_key +
   bundle URL) (`admin.rs:4893`, `render_bundle_md` `admin.rs:3211`), which the runner
   scrapes to send the welcome email.

Idempotency: runner dedups by email (`runner.go:369-387`); the portal 409s a taken
slot (`admin.rs:4979-4986`).

### 3.1 What the registry records about a slot — the person linkage

A slot carries the **Kratos identity UUID of the person it was onboarded for**:
`slots.identity_id` (`ops/goat-zenoh-portal/internal/registry/schema.sql:324-333`,
NULL-able, added by migration in the SQLite build and part of the version-1 Postgres schema). It exists because the
onboard flow holds that UUID and used to discard it, leaving every downstream consumer
to re-derive ownership by matching names — the mis-attribution class the 2026-08-01
fragility audit named, and ownership is what revocation targets.

- **Key discipline:** a UUID and nothing else — an email or a handle in this column
  would recreate the mutable-key join it exists to retire (`ValidIdentityID`
  `registry/registry.go:310`, regex `registry/registry.go:304`).
- **Two write paths:** inline on the slot-pool onboard (`identity_id` on the
  `POST /api/vendors` body, `internal/api/handlers.go:187-194`, validated before a slot
  is claimed) and after the fact via `PUT /api/slots/{anchor}/identity`
  (operator-gated + audited, `internal/api/server.go:332`; `StampSlotIdentity`
  `internal/registry/slots.go:260`). The second exists because the native
  `goat admin onboard` runs its Ory leg AFTER the slot mint, and because pre-existing
  slots predate the column. An empty value CLEARS the stamp — a wrong linkage is
  withdrawn to "not recorded", never left as a confident wrong owner.
- **Re-claim clears it** (`slots.go:224-247`), so a recycled slot never carries the
  prior tenant's person.
- **NULL is honest, not "unowned":** a free slot, a machine slot (machines are
  WireGuard identities, their person link is `enrolled_by`), or any slot minted before
  the column. Consumers fall back to the name joins and must say so — see §4.6.
- **Populated since 2026-08-07, by BOTH mint paths.** The console stamps it on its
  `/result` writeback (`ops/goat-id-console/internal/handlers/onboard_slot_identity.go`,
  #2212) and `goat admin onboard` stamps it after its Ory leg
  (`PortalClient::stamp_slot_identity`, #2220). Two paths on purpose: the console covers
  console-driven onboards, the CLI covers an operator running the mint directly — fixing
  only one leaves the other silently producing unlinked slots, which is how the original
  gap accumulated. `PUT` is idempotent, so both firing is a no-op.
  Both are fail-soft (a miss degrades to the name join, never fails an onboard) and both
  REFUSE when an email resolves to more than one identity: a wrong binding asserts that
  one person owns another's namespace, and ownership is what revocation targets.
- **Historic slots are still name-joined, and that needs a human.** The dry-run tool
  (`ops/goat-id-console/scripts/slot-identity-backfill.py`) proposes none of them,
  which is the right answer: most are machines, services, fixtures or org slots that have
  no person, and the ones that ARE people predate both the console's proposal store and
  the operator manifests, so no source records which email owns the handle (Kratos cannot
  answer — it carries no handle trait). Treat an unlinked anchor as name-joined, and never
  link one by recognising the name.

**Custodian keys are normalized to the anchor.** `machine_grants.custodian` used to be
polymorphic — an anchor, a slot alias, or operator free text — while the machine-quota
join keyed on it, so a grant written under one spelling and an enrollment stamped under
another silently read quota 0 or a doubled quota. Every read and write now funnels
through the slot ANCHOR (`normalizeCustodian`
`ops/goat-zenoh-portal/internal/registry/machines.go:383`), and legacy rows are
rewritten once at `Open()` on BOTH sides of the join — the grant rows and the
`slots.enrolled_by` stamps (`normalizeMachineCustodians` `machines.go:36`, wired
`registry/registry.go:188`). A custodian naming no live slot is left as written, so an operator
may still pre-grant against a handle that has no slot yet; it just does not get the
join guarantee until it does.

---

## 4. Access resolution per surface — **who has what power, and why**

### 4.1 The goat CLI (`goat access`, `goat admin`, `goat top`) — THE headline
The CLI holds **no** client-side notion of "admin". Its powers are entirely a
function of the single Bearer token it reads off disk (`portal.token`) and how the
**portal** classifies it. Every call is `bearer_auth(&inner.api_key)`
(`crates/panoscope-core/src/portal.rs:3861`).

The portal `resolvePrincipal` (`ops/goat-zenoh-portal/internal/api/auth.go:178-246`):
- constant-time-compares the token against **every** `OperatorAPIKeys` value (and
  the legacy key) → match ⇒ `Principal{Operator:true, Actor:handle}` (super-user,
  no tier cap);
- else shape-checks the token (`vp_<prefix>_<64 hex>`) and looks up its sha256 digest in
  the vendor table, with at most one bcrypt for a row minted before the digest column
  → `Principal{Vendor:prefix}` (org-scoped);
- no match ⇒ 401.

Operator-only endpoints enforce `p.Operator` server-side (`auth.go:293-302`).
`GOAT_OPERATOR_HANDLE` / `operator_handle` only sets the `X-Goat-Operator-Actor`
attribution header — it changes *who an action is attributed to*, never *whether
it's allowed*, and is honored only because the token already resolved to an
operator (`crates/panoscope-core/src/portal.rs:1490-1511`; `auth.go:80-97`).

> **So: the goat CLI acts with platform-admin/operator powers iff its `portal.token`
> value matches a `PORTAL_OPERATOR_API_KEYS` entry. Nothing else — not the cert, not
> the `.cbor`, not Ory groups, not a data role — grants CLI operator power.**

`goat access who/why me` resolves the caller's identity via `GET /api/whoami`, which
returns `{operator, actor, vendor, display_name, alias}` (a *selector*, not an
authorization) (`crates/goat-cli/src/cmd/access.rs:1410-1453`;
`ops/goat-zenoh-portal/internal/api/whoami.go:53-88`).

### 4.2 Portal `/api/*` auth
Same `resolvePrincipal`. Operator ⇒ any vendor scope; vendor ⇒ its own prefix only
(`requireVendorScope`, `auth.go:307-324`).

`GET /api/health` is unauthenticated and **probes** rather than asserting: it runs one
1-row read against the registry with a 3 s bound and answers `503` + a `reason` when
that fails (`internal/api/handlers.go:31,40`; `Registry.Ping`
`internal/registry/registry.go:239`). It used to be a constant `"ok"` that no failure
could change, which is why a portal with an unreadable registry read healthy. An empty
registry is still healthy — the probe does not depend on any row existing. The runtime
image is distroless, so the container probe is the binary probing itself:
`goat-zenoh-portal healthcheck [--url …] [--timeout …]`, exit 0 only on a 200 whose
body says `status: "ok"` (`cmd/portal/main.go:312`).

### 3.2 Whose nation is on a record — the one that gets read wrong (ADR 1171)

The cross-plane answer, because this is the reference people check before inferring.

| Field | Whose | Where it lives | Who sets it |
|---|---|---|---|
| `identity.nationality` | **the person's** citizenship | the person's manifest / registry row | the person, at intake |
| `affiliation.nation` | **the organisation's** controlling nation | the org registry | an admin, at org registration |
| `affiliation.relationship` | how the two relate — `constituent` \| `assigned` \| `liaison` | the membership | an admin, at approval |

Neither nation is derived from the other and **the two disagreeing is normal** — a liaison officer, or an internationally recruited member. Alliance standing and the derived organic test read `affiliation.nation` **only**.

Both of the couplings that used to make this unreadable are gone (2026-09-07):

- **A member's nationality is their own.** `access.RosterEntry` carries `Nationality` from the holder's own manifest, beside the org's `Nation`. An undeclared nationality stays **empty** — the projection never fills it in from the organisation.
- **An org's facts come from the catalog, not from whoever onboarded first.** `gen-roster.py` defaults to `--org-source store`, so kind/nation/parent are the registry's and the manifests supply membership only. Nation codes are folded to alpha-3 on both sides, so `US` and `USA` no longer read as two countries.

**What is empty rather than wrong:** `identity.nationality` post-dates every manifest in the store, so every member's `Nationality` is blank today and fills in one candidate at a time. Read it as *undeclared*, never as *same as the org*.

Status and sequence: `org-affiliation-nationality-plan.md`.

### 4.3 Data-access roles resolver `GET /api/access/data-roles`
Operator-bearer or console-service-token gated; internal consumers only
(`ops/goat-zenoh-portal/internal/api/data_roles.go:121-125`). Serves **registry-derived
roles only** — org membership → `data.view.org:<O>`, own producer →
`data.view.producer:<K>`, plus ADR 1075 grant rows merged in
(`data_roles.go:212-234`; `dataroles.go:278-307`). It **explicitly does NOT serve
claim-carried grants or platform groups**:

> "It carries ONLY registry-derived roles — no platform groups, no claim-carried
> extras … (those ride the Ory claim …)" (`ops/goat-zenoh-portal/internal/api/data_roles.go:98-100`;
> `ops/goat-zenoh-portal/internal/access/dataroles.go:11-16`).

**Trap:** granting `data.view.all` in an Ory identity's `groups` will NOT appear in
this resolver — by design. This resolver is a *directory lookup*, not the
enforcement path for `data.view.all`.

### 4.4 Web console + panoscope RBAC (the Ory JWT `groups` claim)
Panoscope derives a `ViewerScope` from the JWT groups
(`ops/panoscope/src/viewer.rs:225-257`):
- group in `full_groups` (default `{"ADMINS"}`) ⇒ `Full` (everything incl.
  platform telemetry) (`viewer.rs:100-141,235-239`);
- `data.view.all` claim ⇒ `DataAll` (every producer namespace, **excludes**
  `ops/audit/`) (`viewer.rs:193,241-243,306`);
- `data.view.org:<p>` / `data.view.producer:<a>` ⇒ `Scoped{anchors}`;
- nothing qualifying (enforce mode) ⇒ the bare floor, fail-closed.
- It trusts **`x-forwarded-groups` only** — reading `x-auth-request-groups` is a
  forgery hole (`viewer.rs:50-62`).
- `PLATFORM-ADMIN → Full` and `PLATFORM-ADMIN → data.view.all` (the group formerly named
  `ADMINS`, §8) is the *one* declared cross-axis
  implication (`ops/permissions/group-permissions.yml:269-297`;
  `ops/permissions/data-access-roles.yaml:269`). `IAM-ADMIN` implies **no** data
  access (`data-access-roles.yaml:262-266`; `viewer.rs:105-107`).

### 4.5 Zenoh wire ACL
- **Writes** are per-producer, matched by the cert **CN**: one write rule per
  vendor/slot, `<prefix>/**` + `volatile/<prefix>/**`
  (`ops/goat-zenoh-portal/internal/acl/acl.go:365-385`;
  `config.json5.j2:1286-1291`). No per-vendor read rule is rendered unless the partner
  read partition is on (next bullet).
- **Reads are flat by default** (ADR 1025): a universal read rule admits any mTLS peer with a
  goatnet-CA cert to subscribe/query any non-carved key
  (`config.json5.j2:694-700,1311-1352`; `data-access-roles.yaml:80-96`). The one wire
  read-carve is `ops/audit/operator/**` (`config.json5.j2:752-788`). Where
  `zenoh_partner_read_partition` is on, a partner anchor leaves the
  universal-read subject and reads only its own anchor plus its granted release lanes
  (`config.json5.j2:852-867`).
- View-scoping is a *surface* concern (panoscope §4.4), not a wire concern.

### 4.6 How the COP attributes an anchor to a person — and how it says so
Ownership drives revocation targeting, so the access COP distinguishes a **recorded**
owner from a **name that happened to match**. Three resolution sources, strongest
first (`ops/goat-zenoh-portal/internal/access/graph.go:204-212`):

| `anchor_linkage` | Source | Strength |
|---|---|---|
| `recorded` | `slots.identity_id` — the linkage stamped at onboard (§3.1) | ground truth |
| `tag-join` | the slot alias equals the identity's org short-tag | inference — coarse: org-mates sharing a tag share the anchor, right for a shared system and wrong for a person |

**There is deliberately no alias↔handle join.** A handle is not a slot alias — the
onboard sets the alias from `zenoh.trial.vendor` and drift-checks it against
`org.short_tag` — so a handle lookup can only "succeed" by colliding with some *other*
member's tag alias. goat-cli's rotate/revoke cascade has refused exactly that as a
mis-rotation since 2026-06 (`resolve_member_anchors`); the portal made the join anyway,
so the two components disagreed about ownership and the surface people READ was the
looser one. Removed 2026-08-03. A person the tag join cannot place now reports **no
anchor** and an explicit `writes unattributed` note (`internal/access/query.go:237`) —
an honest miss the recorded linkage fills, never a confident wrong owner. Read that note
as "can't know", not "known unable to publish": writes are cert-gated at the router, and
grants outside the vendor/slot registry (the static operator `ops/**` rule) are invisible
to the graph.

A weaker join never overwrites a stronger one (`setPersonAnchor` `graph.go:228`). The
label rides on both `GET /api/access/who` and every roster row
(`internal/access/query.go:52,299`), and the per-assembly fallback counts are logged
plus returned as a warn-severity note (`assembleAccessGraph`
`internal/api/access.go:241`) — that count is the backfill's progress meter and is
expected to be **every person** until the linkage is actually written (§3.1).

**Roster notes are severity-graded.** `notes[]` mixes failure notes ("a layer did not
answer, this picture is incomplete") with standing doctrinal text emitted on every
healthy response, and consumers were string-matching the prose to tell them apart.
`note_details[]` now carries a stable `kind` + `severity` (`error` | `warn` | `info`)
on each entry, index-aligned with `notes[]` and carrying the identical strings
(`query.go:318-370`; the notes are tagged where they are written,
`internal/api/access.go:37`). Read `severity: error` to mean **a layer is dark and the
COP under-reports** — absence in the answer is not evidence of absence in the world.

---

## 5. Tiers → what they actually set

Tier presets are the SoT for the tier→plane mapping, in
`ops/goat-id-console/web/static/admin/onboard-intake-form.json` (read by
`loadIntakeHints()` `onboard_support.go:232-242`; `assembleManifest`
`onboard_support.go:521-739`). Verbatim `set` blocks:

| Tier (rank) | NetBird `auto_groups` | Zenoh | Ory `groups` | Sponsorship |
|---|---|---|---|---|
| **browse** (1) | `[OBSERVER]` | off | — | — |
| **developer** (2) | `[DEVELOPER, OBSERVER]` | on, slot `@handle` | — | — |
| **iam-admin** (3) | `[MONITOR, OBSERVER]` | off | `[IAM-ADMIN, GRAFANA-READONLY]` | ≤ iam-admin |
| **platform-admin** (4) | `[PLATFORM-ADMIN, DEVELOPER]` | on, slot `@handle` | `[PLATFORM-ADMIN]` | ≤ platform-admin |

Two facts from the tier help text that resolve most confusion:
- **iam-admin / platform-admin tiers do NOT grant IAM-mutate (operator) power** —
  "IAM-mutate credentials are still granted out-of-band" (tier help,
  `onboard-intake-form.json:30,44`). The tier grants console `PLATFORM-ADMIN` + a mesh role
  + (platform-admin) a Zenoh dev slot. **Operator power is the separate ceremony (§7).**
- `iam-admin`/`platform-admin` are the escalated carve-out: sponsor-offered only,
  never enrollee-selected, flagged at both gates (`onboard_support.go:118,248-252`;
  `onboard_effective.go:43-45`).

**Changing a tier after onboard is not an in-band flow** — no "change tier"
endpoint exists; it is a re-onboard / manifest edit (UNVERIFIED as a supported
path). The Ory-plane slice of a role change (the console group patch,
`handleUserPatchGroups`) does exist and, on an effective change, emails the
person an upgrade-framed complete reference for their new role — explicitly
not a new-provisioning welcome
(`ops/goat-id-console/internal/handlers/access_change.go`;
`ops/goat-id-console/internal/welcome/welcome.go` `RenderUpgrade`). Sponsorship
(one attribute) is revocable via `goat admin sponsorship --revoke`.

---

## 6. Operator (platform-admin on the CLI) enablement — the SEPARATE ceremony

Becoming a **backend/CLI operator** is separate from candidate onboarding. Playbook
`ops/ansible/operator-tier-onboard.yml` ("NEW playbook, separate from
operator-onboard.yml … never touches the live participant path"
`operator-tier-onboard.yml:19-21`); CLI wrapper `goat admin operator onboard <handle>`
(`AdminOperatorCmd::Onboard` `admin.rs:568-571`, `operator_onboard` `admin.rs:5167`). It creates, for a person who already has a participant identity:
1. an **operator Zenoh leaf** (`operator-cert.pem`/`operator-key.pem`, CN
   `operator-trial-peer`, writes `ops/**`) (`operator-tier-onboard.yml:77,148-193`);
2. an SSH key on the mgmt host;
3. each Ory admin group the manifest's `operator.groups` names (`ADMINS`, or
   `PLATFORM-ADMIN` after the rename, §8) PATCHed onto the identity (`:317-332`);
4. a bundle `NOTES.md`.

**The playbook does not mint the portal operator key — it prints the follow-up, never
auto-edits** (`operator-tier-onboard.yml:398-428`). The key is no longer a hand edit plus a
redeploy: the person runs `goat admin operator-key request` on their own profile (no
handle — the console binds the request to the person the `vp_` token belongs to, F-571), a
DIFFERENT admin approves, the person collects the value once with `collect <id>` on the
same profile, and the portal re-reads its key directory while running (§2.4). That value
becomes `Principal{Operator:true}`
at `auth.go:201`.

Operator CLI env (`ops/ansible/templates/operator-tier/NOTES.md.j2:22-29`; `admin-operator-guide.md:48-95`):
| Env | Purpose |
|---|---|
| `GOAT_OPERATOR_HANDLE` | per-person attribution |
| `<ENV>_PORTAL_OPERATOR_API_KEY` | the portal operator Bearer (what makes `operator:true`) |
| `<ENV>_NB_API_TOKEN` | NetBird setup-key mint |
| `GOAT_OPERATOR_AGE_IDENTITY` | age key for `goat audit ops` decrypt |

**Verified nuance:** the *native* `goat admin onboard` reads its portal token from
`GOAT_PORTAL_TOKEN` when that is set, else from
the profile (`~/.goat/<ctx>/portal.token`, from the `.cbor`), not directly from
`<ENV>_PORTAL_OPERATOR_API_KEY` (`crates/goat-cli/src/profile.rs:281-302`); the env
var is documented as the value you *register* server-side and is consumed by the
Ansible onboard leg, which the CLI runs under `direnv exec` when the env is not already loaded (`admin.rs:2442-2453`). Whichever token the CLI presents, it is
the *portal's operator-key match* that grants power (§4.1).

---

## 7. The "I want X to have privilege Y" playbook

Pick the plane by the surface, not by intuition. Each row is the *only* lever for
that surface.

| Goal | Lever (the actual fix) | Not the fix |
|---|---|---|
| **Platform-admin on the goat CLI** (`goat admin`/operator) | Register their per-person key in `PORTAL_OPERATOR_API_KEYS` and give them that key (operator ceremony §6). Their CLI presents it → `operator:true`. | ❌ recut `.cbor` (crypto only) · ❌ Ory group · ❌ data-role grant |
| **Admin of web consoles** (dashboard / id-console / MinIO / panoscope Full) | Add `ADMINS` to their Kratos `traits.groups`; they re-login (JWT refresh) | ❌ operator key · ❌ `.cbor` |
| **Look at the NetBird dashboard** (peers, groups, policies) | Ory `PLATFORM-ADMIN` or `IAM-ADMIN`. On a goatnet under ADR 1189 `netbird-admin-reconcile` maps them to NetBird `auditor` (reads everything, writes nothing) via `netbird_mgmt_role_map` | ❌ NetBird `admin` for everyone |
| **Write in the NetBird dashboard** (break-glass) | Ory `NETBIRD-ADMIN` (→ NetBird `admin`), set with `goat admin role`; they write at `admin-dashboard.<site>` | ❌ the everyday `dashboard.<site>` (reads through `mgmt-read.<site>`, GET only) |
| **Change which groups a device holds** | a change to their operator manifest (default groups, `--generate-baseline`) or an entry in `ops/warrant/membership/<scope>/` (exceptions), merged and deployed; the converger applies it (warrant-runbook §4A) | ❌ a dashboard edit — reverted within one converger pass on a governed goatnet |
| **See all data on the web** (panoscope DataAll) | `ADMINS` (⇒ Full) or `data.view.all` in Kratos `groups` (⇒ DataAll) (`viewer.rs:235-243`) | ❌ the `/api/access/data-roles` resolver (doesn't serve claims) |
| **See a specific org/producer's data (web)** | a producer-approved grant row (ADR 1075) or `data.view.org:`/`producer:` claim | — |
| **Write to a fabric namespace** | a Zenoh cert whose **CN** matches an ACL write rule (participant slot, or operator `operator-trial-peer` for `ops/**`) | ❌ portal token · ❌ Ory group |
| **Onboard/revoke/rotate others** | an operator token (§6) — `goat admin onboard/rotate` | — |
| **Sponsor candidates** | sponsorship annotation on their org row (`goat admin sponsorship`), tier-capped | — |
| **New participant at tier T** | onboard with tier T (§5) — sets NetBird + Zenoh + Ory per the tier table | — |

---

## 8. Known drifts, gotchas & UNVERIFIED corners

Honest flags — do not treat these as settled:
- **`PLATFORM-ADMIN` rename is part-way through, no longer prose-only.** The SoT, the
  tier presets and the panoscope role default now say `PLATFORM-ADMIN`
  (`ops/permissions/group-permissions.yml:266-273`; `onboard-intake-form.json:46,49`;
  `ops/ansible/roles/panoscope/defaults/main.yml:297`); panoscope's compiled-in
  `full_groups` default is still `{"ADMINS"}` (overridable via
  `PANOSCOPE_RBAC_FULL_GROUPS`) (`viewer.rs:135,153-155`). Until the contract phase
  drops it, `ADMINS` stays accepted as the legacy alias
  (`ops/goat-id-console/internal/handlers/handlers.go:2212-2217`).
- **Cert ACL matches CN, not URI-SAN.** The URI-SAN is the frozen identity stamp;
  the wire ACL keys on `cert_common_names` (`config.json5.j2:532-533`). Older design
  prose sketched URI-SAN matching — the code is authoritative.
- **`.cbor` top-level `acl_groups`/`peer_pubkey`/`site`** ride opaque under the
  signature; the Rust verifier never parses them (`bundle.rs:685-712`). Their
  device-side consumer was not read — UNVERIFIED.
- **NetBird machine-identity internals** are asserted from prose
  (`identity-architecture.md`), not code (the fork is a sibling repo). The PAT
  prefix + Bearer presentation are code-verified.
- **In-band tier change** after onboard: no endpoint found; treat as re-onboard.
- **`slots.identity_id` is built but unpopulated.** The column, both write paths, the
  validation and the read-side honesty labels all exist (§3.1, §4.6); **no in-tree
  caller supplies the UUID yet** and no backfill has run, so in practice every person's
  anchor still resolves by a name join today. Read `anchor_linkage` before acting on an
  ownership claim; do not assume "recorded" because the column exists.
- **Peer-name parsing was single-sourced 2026-08-03 — the drift is CLOSED.** The grammar
  had five independent implementations across Go, Rust and an Ansible playbook with no
  fence between them, and both failure directions were silent: a matcher that drops the
  trailing separator attributes an org-mate's peer to the wrong principal, and
  a matcher that never fires reports an honest-looking zero. The SoT is
  `ops/identity/peer-name-grammar.yaml` — deliberately a **sibling of**
  `ops/identity/grammar.yaml` (ADR 1067's identity grammar), not a section inside it:
  they version independently (peer-name `1.1.0`, grammar `1.6.0`, both at line 38), and
  folding one into the other would couple two release cadences that have no reason to
  move together. `ops/identity/generate-peer-name.py` derives the
  Go (`ops/goat-zenoh-portal/internal/peername/peername_gen.go`) and Rust
  (`crates/panoscope-core/src/peername.rs`) implementations one-way, and
  `ops/ci/check-peer-name-grammar-sync.sh` fails the build on drift. **What is
  single-sourced:** the grammar values (prefix, separator, field order, optionality) and
  the conformance vectors. **What is not:** the per-language function bodies — Go and
  Rust cannot share code — so cross-language agreement is proven *behaviourally* by the
  vectors, generated into a table-driven test in both languages. A divergence therefore
  fails a test, not only a byte compare. Evolution is additive-only: changing the prefix,
  separator or field order renames every peer in the fleet and breaks every revoke
  selector.
- **A peer name is evidence about ownership, not a record of it.** The recorded linkage
  is `slots.identity_id` (§3.1). Every consumer that presents ownership must say which
  one it used (§4.6).
- **`civil` affiliation kind** — RESOLVED. Was a real drift: the served
  selectable-affiliations list (`affiliationKinds`, `onboard_support.go:132`) omitted
  `civil` even though the schema enum, both intake forms, the welcome help text, and
  the grammar canon (`aff-kind=<vendor|organic|partner|civil>`) all include it. The
  slice now matches the schema enum, locked by a parity test
  (`affiliation_kinds_parity_test.go`).

---

## 9. Sources & keeping this current

This doc was assembled by reading, not summarizing memory. Primary sources (open
these before trusting any claim above):
- CLI + bundle: `crates/goat-cli/src/{profile.rs,bundle.rs,cmd/access.rs,cmd/admin.rs}`,
  `crates/panoscope-core/src/portal.rs`.
- Portal auth + resolvers: `ops/goat-zenoh-portal/internal/api/{auth.go,whoami.go,data_roles.go}`,
  `ops/goat-zenoh-portal/internal/access/dataroles.go`, `.../internal/acl/acl.go`,
  `.../internal/certmint/certmint.go`, `.../internal/config/config.go`.
- Slot ownership + health (§3.1, §4.2, §4.6):
  `ops/goat-zenoh-portal/internal/registry/{schema.sql,registry.go,slots.go,machines.go}`,
  `.../internal/access/{graph.go,query.go}`, `.../internal/api/{access.go,handlers.go,budgets.go}`,
  `.../cmd/portal/main.go`.
- Console onboarding: `ops/goat-id-console/internal/handlers/onboard_*.go`,
  `.../internal/kratos/client.go`, `ops/onboard-runner/` (incl. `collabctl.go`,
  `collabrecon.go` for the collab plane, §2.9).
- SoT + config: `ops/permissions/group-permissions.yml`,
  `ops/permissions/data-access-roles.yaml`,
  `ops/goat-id-console/web/static/admin/onboard-intake-form.json`,
  `ops/ansible/roles/goat-zenoh/templates/config.json5.j2`,
  `ops/ansible/operator-tier-onboard.yml`, `ops/ansible/roles/ory/templates/ory/identity-schema.json.j2`.
- Related design docs (narrative, not authoritative for behavior):
  `identity-architecture.md`, `identity-grammar.md`, `data-access-roles.md`,
  `three-stage-candidate-onboarding.md`, `operator-onboard-intake.md`,
  `docs/operations/admin-operator-guide.md`.

When code moves, update the cited line ranges (or delete the claim) — a stale
citation is worse than none.
