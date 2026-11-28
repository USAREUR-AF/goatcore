# Operator onboarding intake — webform, linter-as-a-service, admin approval queue

**Status:** designed-not-built (this doc) · ADR [1061](../adr/1061-operator-onboard-intake-system.md)
**Owner surface:** the ZAGROS portal (`goat-zenoh-portal`)
**Related:** operator-onboard-utilities.md · [admin-operator-guide](../operations/admin-operator-guide.md) · identity-grammar.md (ADR 1043) · [operator-action-audit.md](operator-action-audit.md)

## BLUF

Today onboarding starts from a **hand-authored YAML manifest** (`ops/operators/<env>/<handle>.yml`) that an operator copies from a sibling, edits by guesswork, and discovers is wrong only when `goat admin onboard` rejects it. A manifest copied from a day-old template can fail validation because a grammar cutover now *requires* a block it lacks — guesswork the operator shouldn't have to do.

This system removes the guesswork by making the **schema self-serve**:

1. **Intake webform** — a grammar-native form *generated from the schema*, so every field shows its type (enum dropdown / free text), whether it's required or optional, and a plain-English description. The **sponsor is bound automatically to the authenticated submitter** — nobody types it.
2. **Linter-as-a-service** — paste or upload a manifest, get back pass/fail plus exactly which field is wrong and why. One linter (`validate.py`), shared with the `goat admin onboard` Phase-0 gate — no second implementation to drift.
3. **Proposed-onboarding queue** — submissions land as *proposals*, not live credentials. They are stored, not executed.
4. **Admin approval view** — an operator reviews the queue and chooses which proposals to run; approving emits the validated `<handle>.yml` and runs `goat admin onboard`.
5. **Batch upload** — drop a file of many records; each is linted independently; valid ones become proposals, invalid ones report per-record errors.

It is built around the **future identity grammar** (role · affiliation · echelon · node-class · site — ADR 1043), not the historical "onboard a vendor to nova" shape. The form speaks `DEVELOPER` / `partner` / `COMPONENT`, not `VENDOR`.

## The problem it solves

The manifest schema ([`ops/operators/schema/operator-manifest.schema.json`](../../ops/operators/schema/operator-manifest.schema.json)) is rich and correct — five blocks, closed enums, conditional requirements, grammar-cutover gates. But it is only *machine*-readable. A human onboarding someone must:

- know which of ~8 blocks apply to the tier they want (dev vs browse vs operator);
- remember post-cutover rules (affiliation required, deprecated groups forbidden) that change over time;
- get enum spellings exactly right (`DEVELOPER` not `developer`, `partner` not `POL`);
- discover all of the above *one error at a time* by re-running the onboard.

Every one of those is information the schema already encodes. The intake system **surfaces the schema to the human** instead of making them reverse-engineer it.

## Architecture

```
                    ┌──────────────── ZAGROS portal (goat-zenoh-portal) ─────────────────┐
  sponsor  ─POST─▶  │  /api/onboard/lint   (linter-as-a-service, shells validate.py)   │
  (browser)         │  /api/onboard/intake (store a proposal; sponsor = submitter)     │
                    │  /api/onboard/intake/batch (file → N proposals, each linted)     │
  admin    ─GET──▶  │  /api/onboard/proposals      (the queue; operator-gated)         │
  (browser)         │  /api/onboard/proposals/{id}/approve → emit yaml + run onboard   │
                    └──────────────────────────────────────────────────────────────────┘
                                  │ approve                      ▲ lint
                                  ▼                              │
              ops/operators/<env>/<handle>.yml  ──▶  validate.py --json  ◀── (same SoT)
                                  │                              ▲
                                  ▼                              │
                       goat admin onboard <handle> --env <env> (native fast path, ~3s, 0.4.13)
```

### 1. Schema-as-SoT field metadata + the form

The form is **generated**, not hand-coded, so it can never drift from the schema. The JSON Schema already carries `type`, `enum`, `pattern`, `required`, and `description` for every field. We add a thin **UI-hints sidecar** (`ops/operators/schema/intake-form.json`) for the few things JSON Schema can't express: field grouping/order, which tier presets exist, and one-line "what this means" help where the schema description is terse. The form renderer reads schema ⊕ hints and emits:

- **enum fields → dropdowns** (role family, affiliation kind, echelon, node-class, autonomy, site) sourced from the grammar's closed value sets;
- **pattern/string fields → text inputs** with the regex surfaced as a hint (handle, email, unit slug);
- **required vs optional** marked from the schema `required` arrays + conditional rules;
- **tier presets** — four, mapped onto the ADR 1043 grammar (decided in [onboard-tier-role-model.md](onboard-tier-role-model.md)): **Developer** (`DEVELOPER`+Zenoh slot), **Browse** (`OBSERVER`, web only), **IAM Admin** (`MONITOR` + Ory `IAM-ADMIN` + diagnostics-read; identity/access management + security monitoring, no dev cert, mutate-creds out-of-band), **Platform Admin** (`ADMIN`+`DEVELOPER` — full backend *plus* a Zenoh slot for fabric test/troubleshoot). They pre-fill the group/Ory/zenoh blocks so common cases are one click, every field still editable (progressive disclosure, not a ceiling).

> The tier↔grammar role model (and what's deferred — the IAM-Admin L7 fence wiring + delegated machine-enrollment) is settled in [onboard-tier-role-model.md](onboard-tier-role-model.md).

The grammar dimensions are the spine of the form (see identity-grammar.md §1): **role** {ADMIN, MONITOR, DEVELOPER, OPERATOR, OBSERVER} · **affiliation.kind** {vendor, organic, partner, civil} · **echelon** {COMMAND, COMPONENT, OPERATIONAL, OPSCTR, COMMAND-POST, MFT} · **node-class** {SENSOR, EFFECTOR, PROCESSOR, PLATFORM, …infra} · **site** {zagros, nova, …}. Capability is named by **role**, never by affiliation — the form enforces this by construction (affiliation is its own section, never a group picker). (`affiliation.kind: vendor` here is the *commercial-company* affiliation value — orthogonal to role, distinct from the deprecated role-sense `VENDOR`.)

**Display discipline — no vendor vocabulary reaches the user.** The form is grammar-native at the display layer too, even where a reused machine surface still carries the legacy noun (the machine-vs-display split of [ADR 1052](../adr/1052-vendor-to-org-machine-rename.md) / the portal lexicon). Concretely: the schema's `vendor` block is **labelled "Organization"** in the form (the key stays `vendor` until ADR 1052 flips it); the linter's reused `classify_shape` returns `vendor-primary`/`vendor-sibling`, which the form **relexes to tier names** ("developer tier") rather than surfacing — the raw `shape` stays only on the machine `/api/onboard/lint` payload and the shared Phase-0 gate, never on screen. The form vocabulary the operator/sponsor actually sees is role/affiliation/echelon/node-class/site and the tier presets, full stop.

### 2. Sponsor binding

"Tie in a sponsor (the individual that submits)" maps directly onto the portal's auth model. Every request carries a `Principal` (`internal/api/auth.go`); for an authenticated submitter `actorOf(r)` already resolves the per-person handle. The intake handler stamps `sponsor = actorOf(r)` (operator) or the non-operator principal's identity (a sponsoring participant) onto the proposal **server-side** — the field is not in the form and cannot be forged from the client. This also records `identity.enrolled_by` / `added_by` provenance the schema already has.

**Who may submit vs who may run.** Submission is the *proposal* step and is open to any authenticated principal (a sponsor proposing a teammate). Approval-and-run is operator-gated (`requireOperator`). This is the whole point of the queue: decouple "I vouch for this person" (sponsor) from "I mint their credentials" (admin).

### 3. Linter-as-a-service

`POST /api/onboard/lint` mirrors the existing `POST /api/peering/lint` workbench (`internal/api/peering_lint.go`): body `{content, format}`, response `{ok, findings:[{severity, field, message}], shape}`.

**Validation is Go-native, against the *shared* schema JSON — not a shell-out to `validate.py`.** The portal runtime is a Chainguard **distroless** single-binary image (ADR 0700): no `python3`, no shell, no package manager. Baking a Python runtime + `validate.py` + `jsonschema` into it to run a linter would multiply the attack surface the distroless image exists to eliminate — the wrong trade. Instead:

- The bulk of validation **is** the JSON Schema (`operator-manifest.schema.json`): required blocks, enum membership, patterns, and the conditional `if/then` rules. That file is the **single source of truth** and is consumed *unchanged* by a Go Draft-7 validator (`internal/onboardlint`, embedding a build-synced copy of the canonical schema). Same rules, same file — no drift.
- Only the two small **grammar-regression** rules (affiliation-required-after-cutover, deprecated-groups — `validate.py::_grammar_regression_errors`) and the display `classify_shape` are hand-ported to Go (~40 lines, stable).
- **Drift is prevented by two fail-closed CI guards, not by hope:** (a) a schema-sync check (`ops/ci/check-onboard-schema-sync.sh`) asserts the portal's embedded copy is byte-identical to the canonical `ops/operators/schema/operator-manifest.schema.json`; (b) a parity test runs *both* the Go linter and `validate.py --json` over a fixture corpus and asserts identical pass/fail verdicts. CI has Python; the portal image does not.
- `validate.py` keeps its additive `--json` mode (built, Phase A) — it is the canonical CLI linter, the `goat admin onboard` Phase-0 gate, **and** the parity-test reference the Go linter is held equal to.

**Result:** the webform, batch upload, and onboard Phase-0 gate run verifiably-equal validation. A manifest green in the form is guaranteed to pass the onboard gate (CI proves it). Findings are field-addressed (`loc` like `netbird.auto_groups.0`) so the form highlights the offending input inline.

### 4. Proposed-onboarding store + admin view

Proposals persist as JSON records (reusing the portal's existing on-disk registry pattern under `internal/registry`; a `proposals/` collection). Record shape:

```
{ id, created_at, sponsor, env, status: proposed|approved|rejected|onboarded|failed,
  manifest: <the validated manifest object>, lint: <last lint result>,
  decided_by, decided_at, onboard_receipt? }
```

The admin view is a new operator-gated page under `web/static/admin/` (sibling of `vendor-activity.html`): a queue table (sponsor · handle · env · tier · lint-status · age), a detail pane rendering the manifest + lint findings, and **Approve / Reject** actions. Approve:

1. re-lints (guard against a schema bump between submit and approve);
2. writes `ops/operators/<env>/<handle>.yml` (the gitignored manifest, the existing onboard input);
3. runs `goat admin onboard <handle> --env <env>` — **v1: surfaced as the exact command for the admin to run** (semi-automated, see Decisions); the receipt/bundle path is recorded back on the proposal.

### 5. Batch upload

`POST /api/onboard/intake/batch` accepts a multi-record file (YAML list or JSON array of manifests, or a CSV mapped to the tier presets). Each record is linted independently via the same `validate.py --json`; valid records become `proposed` rows, invalid records return a per-record findings report so the uploader can fix and re-submit. Mirrors the existing topic bulk-import semantics (`internal/api/bulk.go`: dry-run + per-entry diff) — same "tell me what's wrong before it lands" posture.

## Decisions

- **Home = the ZAGROS portal.** It already has operator/sponsor auth, an admin frontend area, the registry persistence pattern, the `certmint`/`kratosadmin`/`access` packages, and a lint-workbench precedent. A standalone service would duplicate all of it.
- **One linter (`validate.py`), shelled-to, not reimplemented.** The schema check the uploader sees is byte-identical to the onboard gate. Cost: a Python runtime in the portal image — accepted.
- **v1 is semi-automated: approve → emit manifest + run `goat admin onboard`.** The mint needs the NetBird API token + portal operator key + SSH to the site Thor — credentials that live with the operator, not the portal. v1 emits the validated manifest and the exact onboard command; the operator runs it. As of goat-cli 0.4.13 that command takes the **native fast path** (~3s, all three surfaces, no Ansible — see [`onboard-rotate-revoke-streamlining.md`](onboard-rotate-revoke-streamlining.md) §"Phase 2"), which is what the fast-follow auto-executor runs. **Fully portal-triggered onboarding** (the credentials + mint capability move to a runner, not the portal) is a deliberate fast-follow phase, gated on its own security review — a materially bigger credential surface, out of scope for v1.
- **Grammar-native, not vendor→nova.** Form vocabulary, presets, and enums are the ADR 1043 dimensions. The legacy `vendor` noun survives only where it is still machine-load-bearing (per [ADR 1052](../adr/1052-vendor-to-org-machine-rename.md)).
- **Proposals are inert.** A stored proposal mints nothing. The queue/approve split is the safety boundary: sponsors can propose freely; only an operator turns a proposal into credentials.

## Security posture

- No new secret crosses the intake surface — a proposal is pre-mint draft identity (name/email/affiliation), the same data the manifest already holds; treated at the manifest's sensitivity (the rendered bundle with the one-shot api_key is still produced only by the existing onboard, out-of-band).
- Manifests remain **gitignored** identity material; the approve step writes them to the existing `ops/operators/<env>/` location, not into git.
- Submission is authenticated (no anonymous proposals); approval/run is operator-gated.
- The approve action is an operator action and emits an `OperatorActionV1` audit event (see [operator-action-audit.md](operator-action-audit.md)) — who approved which onboarding.

## Build phases

| Phase | Deliverable | Gate | Status |
|---|---|---|---|
| **A** | `validate.py --json` + design/ADR | this doc | **built** |
| **B** | `POST /api/onboard/lint` (linter-as-a-service) | mirror peering/lint; CI parity + schema-sync | **built** — `internal/onboardlint` + endpoint + 2 CI guards; unit/parity/httptest green |
| **C** | Schema field-metadata sidecar + generated intake form (single-record) | form lints green → onboard accepts | **built + browser-verified** — `GET /api/onboard/schema` + `web/static/admin/onboard-intake-form.json` (hints) + `onboard-intake.html` (generated form, live-lint) + nav link; data contract httptested AND driven end-to-end in a real browser (Playwright: form renders from schema⊕hints, live-lint round-trips, valid→Submit→proposal stored, shape relexed to "developer tier"). The browser pass caught + fixed a real bug — the form left `ory_site.sites` unset (schema-required when `ory_site.enabled`), so a fully-filled form would have failed lint; now derived from the site field |
| **D** | Proposal store + `POST /api/onboard/intake` (sponsor binding) | proposal round-trips | **built** — `onboard_proposals` table + `registry` CRUD + `POST /api/onboard/intake` (sponsor server-bound, re-lints, inert) + `GET /api/onboard/proposals` (operator) + form Submit wired; httptest round-trip green |
| **E** | Admin approval view + approve→emit+command | operator approves a real proposal on ZAGROS | **built** — operator-gated `GET {id}` + `approve`/`reject`/`onboarded` endpoints (approve re-lints, records the decision, emits manifest YAML + `goat admin onboard` command; portal can't run it — no repo/creds in the distroless container) + `onboard-queue.html` master-detail admin view + audit-logged; httptests green; interactive render operator-gated |
| **F** | Batch upload | multi-record file → N proposals | **built** — `POST /api/onboard/intake/batch` (JSON array / YAML sequence; each record linted independently; valid→proposals, invalid→per-record findings; `dry_run` validates-only; one bad record never blocks the good ones) + `onboard-batch.html` (paste/file upload, per-record results) + nav link; httptests (mixed validity, dry-run, YAML) green |

## Fast-follow: fully-automated onboarding (`onboard-runner`) — credential-surface review + impl plan

v1 is **semi-automated**: approve records the decision and emits the manifest YAML + the `goat admin onboard` command; the operator runs it. The fast-follow is **full automation** — the system mints on approve, with no human command-running. This is deferred not because it's hard to wire, but because it is the single largest credential-surface decision in this design, and getting the boundary wrong turns the onboarding portal into a mint-anything oracle. This section is that review + a concrete implementation plan; **the conclusion is a recommended architecture, gated on an explicit decision to accept a new mint-capable component — not yet an approval to build.**

### What "run the onboard" actually requires (native-path, goat-cli 0.4.13)

`goat admin onboard <handle> --env <env>` now defaults to the **native fast path** (~3 s, all three surfaces, no Ansible — [`onboard-rotate-revoke-streamlining.md`](onboard-rotate-revoke-streamlining.md) §"Phase 2"). This *shrinks* the executor's footprint versus the old Ansible orchestrator, which is what makes the runner tractable. The native mint needs:

| Needs | For | Was also needed by the old Ansible path? |
|---|---|---|
| the `goat` binary | the executor itself | no (ansible + a repo checkout of the playbook) |
| per-env **NetBird API token** | mint the mesh setup-key | yes |
| **portal operator key** | mint the Zenoh slot/cert via the portal | yes |
| **SSH key to the site Thor** | create the Kratos login (the Ory leg SSHes to the Thor's localhost admin API) | yes (but for the *whole* delegated play, not one curl) |
| the **manifest** + the **schema file** (for enum validation) | the input + the `ory_site.groups` check | the manifest + the full role-defaults tree |

So native removes the two heaviest asks the old analysis assumed — **no `ansible`, and no full repo/role-defaults checkout** (just the one schema JSON). The portal still holds *none* of these and runs in a **Chainguard distroless** image (ADR 0700) with no shell, ssh, or `goat` binary — so the mint capability still cannot live on the web tier.

### Options

- **A — portal shells `goat` directly.** Requires baking `goat` + ssh + the three credentials into the distroless portal image. **Rejected:** it collapses the onboarding credential set into the most-exposed component and guts the distroless attack-surface posture ADR 0700 exists for. (Native makes this *lighter* than the ansible version, but the objection is the credential locus, not the payload size — still rejected.)
- **B — privileged `onboard-runner` sidecar, portal-pushed.** A separate service holds the credentials and exposes a narrow authenticated "run onboard for approved proposal `<id>`" API; the portal calls it on approve. Better — the blast radius leaves the portal — but the portal now makes an outbound privileged call, and a portal compromise can drive the runner.
- **C — `onboard-runner` worker, pull-model (recommended).** A worker on a **trusted sovereign host** (the site Thor / operator infra) holds the credentials and **polls the portal** for `approved` proposals over an authenticated read, runs the native `goat admin onboard`, and writes the result back (`onboarded`/`failed`). **The portal never holds a credential and never initiates a privileged call** — it stays a credential-free read surface; the runner reaches *in*. This is the cleanest boundary: the mint capability lives exactly where the offline-CA / NetBird-token trust already lives, not on the web tier — and native's ~3 s mint makes a poll-run-writeback loop cheap.

### The boundary the review fixes (gates on implementation)

1. **Credential locus.** Mint credentials live only on the runner's sovereign host — never on the portal, never in its image, never in its env. The portal's role is unchanged from v1: validate + queue + record decisions.
2. **Authorization.** Only an **operator**-approved proposal is eligible; the runner refuses anything not in `approved` state. Approval stays a human operator action (the queue), audited.
3. **Idempotency / no double-mint.** The runner dedups by **email** (not handle) before executing — composing with the onboarding-collision-safety contract (concurrent onboards of the same person silently double-issue unless reconciled by email): a proposal whose principal is already minted is a no-op + flag, never a second issuance. The proposal `id` is the execution key; a proposal transitions `approved → onboarded` exactly once (claimed via an atomic `approved → onboarding` state flip so two runners can't race the same proposal).
4. **Attribution.** Every auto-run emits `OperatorActionV1` ([operator-action-audit.md](operator-action-audit.md)) carrying **both** the approving operator and the runner identity — "who approved" and "what executed" are distinct and both recorded. The native path already emits this event from the `goat admin` chokepoint; the runner sets `GOAT_OPERATOR_HANDLE` to a distinct runner identity so the audit shows runner-executed, operator-approved.
5. **Runner blast radius.** The runner *is* a mint-anything capability; a runner compromise = forge-any-identity. It must run on a tightly-controlled sovereign host, with its own attested identity, least-privilege per-env tokens, and its actions fully audited. Its compromise model is reviewed as equivalent to an operator-laptop compromise, not a web-service compromise.
6. **Failure + partial-mint.** A failed mint sets the proposal `failed` with the native onboarder's error (the native path already surfaces a per-surface `partial` outcome — e.g. Zenoh+NetBird minted but Ory failed — rather than an all-or-nothing crash). Partial mints compose with the mesh-first revocation cut ([ADR 1031](../adr/1031-credential-revocation-mesh-first-offboarding.md)); the runner does not invent its own teardown, it records the partial state for an operator to reconcile or re-run.

### Concrete implementation plan (Option C)

The runner is a small Go (or `goat`-subcommand) daemon on the site Thor. It reuses, not reinvents, the native onboarder + the existing proposal store + audit trail.

**Portal side (additive, still credential-free):**
- `GET /api/onboard/proposals?status=approved` — already exists (the admin queue read); the runner authenticates as an operator-tier principal (its own key) and reads the approved set + each proposal's `manifest_json`.
- `POST /api/onboard/proposals/{id}/claim` — new: atomic `approved → onboarding` transition (optimistic, `WHERE status='approved'`), returns 409 if already claimed. Prevents two runners double-minting.
- `POST /api/onboard/proposals/{id}/onboarded` and `/failed` — already exist (mark-onboarded / the `failed` state); the runner writes the terminal state + the native onboarder's stdout/err digest back.

**Runner side (holds the credentials):**
1. **Poll** the approved queue on an interval (or long-poll); for each, `claim` it (skip on 409).
2. **Dedup by email** against the live registry (`access who` / roster) — already-minted principal → mark `onboarded` with a `duplicate` flag, no mint.
3. **Materialize the manifest** — render `manifest_json` → YAML → write to a scratch `ops/operators/<env>/<handle>.yml` under a repo checkout the runner owns (or pass `--repo-root` at a minimal tree carrying just the manifest + `ops/operators/schema/operator-manifest.schema.json` for the enum check).
4. **Run** `goat admin onboard <handle> --env <env>` (native default) with `GOAT_OPERATOR_HANDLE=<runner-id>` + the three credentials in its env. ~3 s.
5. **Write back** `onboarded` (+ the one-time bundle location / api_key handling per the OOB-delivery rule) or `failed`/`partial` with the error. The runner never persists the minted secrets beyond the OOB hand-off the existing onboarder already governs.

**Gates before build:** an explicit decision to stand up a mint-capable component; the runner's attested identity + least-privilege per-env token model; a security review of the runner host; and the `claim` endpoint's atomicity test. This is the natural home for the mint capability — the same sovereign host that already holds the offline-CA and NetBird trust — and native's ~3 s, no-Ansible executor is what makes the poll-run-writeback loop cheap enough to be boring.

### Conclusion

The recommended architecture is **Option C (pull-model `onboard-runner` on a sovereign host)**, which keeps the portal credential-free and co-locates the mint capability with the existing offline-CA/NetBird trust. It is *more* tractable now that the executor is the native ~3 s path (no Ansible, no role-defaults checkout).

### Status: built + live-fired on ZAGROS (2026-07-01)

Option C is built and proven end-to-end:

- **Portal contract** — the atomic `POST …/claim` (`approved → onboarding`, `WHERE status='approved'`, 409 on loss) + `POST …/result` (`onboarded`|`failed`) endpoints in `ops/goat-zenoh-portal` (#1373), operator-gated, credential-free.
- **The runner** — `ops/onboard-runner/` (#1376): poll approved → claim → dedup-by-email → materialize `manifest_json` → YAML → run native `goat admin onboard` → write result back. `--dry-run`/`--once` safe modes, `--only-env` per-goatnet filter, injected-mint tests.
- **Local-Kratos** — goat 0.4.14 (#1384): `GOAT_ONBOARD_KRATOS_LOCAL=1` runs the Ory leg's curl locally against `127.0.0.1:4434` instead of SSHing to the Thor — so the on-Thor runner needs **no SSH key**.
- **Deploy role** — `ops/ansible/roles/onboard-runner/` + `onboard-runner-deploy.yml`: renders a compose stack + a `0600` `.env`, a container-uid-owned schema/operators tree, a `goat >= 0.4.15` assert, and a **default-off go-live gate** (`onboard_runner_enabled`, since it's a mint-anything capability). Rollback-ledgered (ADR 1060, `image`-class). The role was first a host systemd unit (below) and is now a container with its own identity (§"Container reshape").
- **Default-off teardown** — built and measured in a disposable Docker project.
  The production task handles an absent stack, removes a running stack and
  converges on repeat. Stand-in Compose output verifies that the task accepts
  an exit-zero warning and propagates an unrelated error accompanied by the
  same warning. Coverage is limited to the teardown task.

**Live-fire** (laptop-driven runner against the live ZAGROS portal, a notional candidate through the real intake→approve flow): from one admin approval the runner claimed, minted **all three surfaces in 2.6 s** — Zenoh slot + URI-SAN + api_key + alias, a NetBird setup key, and an Ory/Kratos identity — wrote back `onboarded`, then the notional identity was torn down across all three. No grammar-cutover 400. Two findings, both fixed/recorded:

1. **`--native` is forced in the runner's exec.** `native_eligibility` reads *only* `zenoh.trial.org`; a legacy `trial.vendor` manifest (or an operator-IdP/observer manifest) falls back to the Ansible path, which the runner **cannot** execute (no aws-vault, no interactive sudo). Forcing `--native` makes a non-serviceable manifest fail fast with the CLI's eligibility error, recorded as the proposal's `failed` result, instead of a confusing hang. Portal-assembled nomination manifests must therefore emit `zenoh.trial.org`.
2. **Dedup + "mark onboarded" interact** — retiring an approved proposal by marking it `onboarded` makes a same-email resubmit dedup-skip (correct: the email is already onboarded). Operator nuance, not a bug.

**Standing daemon LIVE on the ZAGROS Thor (2026-07-01).** goat-cli-v0.4.14 released → `get.zagros` bumped + goat installed on the Thor → `onboard-runner-deploy.yml` deployed the role → `onboard_runner_enabled=true`. A second live-fire, this time by the **standing daemon** (not a laptop run), claimed `21:13:48` → minted `21:13:50` (**2.6 s**), all three surfaces up, `onboarded` written back, then torn down. Bringing the on-Thor daemon up surfaced four host-integration gaps, all fixed in the role:

- **No full repo clone** — the private repo can't be cloned unauthenticated; the runner needs only the schema (read) + a writable `ops/operators/<env>/` dir, so the role places the schema from the controller + creates the dirs.
- **Host resolver can't reach mesh CoreDNS** — the Thor's systemd-resolved doesn't forward the mesh zone, so a host process can't resolve the portal FQDN even though the portal runs on the Thor. The role maps the portal FQDN → the local Thor mesh IP in `/etc/hosts` (service-FQDN→own-host, valid LE cert).
- **`ProtectHome=true` hid the goat profile** — the native mint's portal client reads its endpoint + operator mTLS/token from `~/.goat`; `ProtectHome` made it invisible (`E_PROFILE_MISSING`). Turned off + `~/.goat` added to `ReadWritePaths`.
- **Profile is operator-provisioned** — the role now fails fast (assert) when the runner user lacks a goat default context, rather than failing every proposal at mint time. (On ZAGROS the site operator context was placed by hand.)

### Container reshape: own identity, no operator login on the Thor

The standing daemon above worked, but it was the wrong shape: a **host process that reused the operator's login** — a copy of the operator's goat profile on the Thor, with full operator power, not independently revocable. Being a host process is also what caused three of the four gaps above (mesh DNS, `ProtectHome`, the hand-placed profile). The role now deploys the runner as a **container with its own scoped identity** (source `ops/onboard-runner/Dockerfile` + the `onboard-runner-image` workflow; image `ghcr.io/usareur-af/onboard-runner`):

- **Own portal key, no operator key.** The runner carries a dedicated per-handle portal key (`onboard-runner-<site>:<key>`, added to the portal's `PORTAL_OPERATOR_API_KEYS` — F-207), independently revocable. The verified fact that makes this clean: the native onboard's portal client is **bearer-only** — it uses only the portal token + endpoint, never the operator mTLS key (and the `goat admin` gate passes on an initialized profile alone). So the runner needs no private key of the operator's. The entrypoint materializes a minimal goat profile (`profile.toml` endpoint + `portal.token`) from the deploy env at startup — no identity material in the image, no mTLS on disk.
- **Reaches by container name, not mesh DNS.** The container joins the shared `mgmt` Docker network, so it reaches the portal at `http://goat-zenoh-portal:8090` and Kratos at `http://mgmt-host-kratos-1:4434` by name — retiring the `/etc/hosts` mesh-DNS hack. The Ory leg runs `curl` in-container (`GOAT_ONBOARD_KRATOS_LOCAL=1` + `GOAT_ONBOARD_KRATOS_ADMIN_URL`, the override released in goat 0.4.15). The NetBird leg uses the public mgmt API (internet-reachable). The `ProtectHome`/`~/.goat`-visibility and hand-placed-profile gaps are gone (no systemd, profile is materialized).
- **Bind-mounts the host goat.** The site `goat` binary stays a single version source: mounted read-only from `/usr/local/bin/goat`. wolfi-base's glibc runs the host's `aarch64-unknown-linux-gnu` binary (verified on the Thor). The role asserts host `goat >= 0.4.15`.

**Live-fired + host daemon decommissioned on ZAGROS 2026-07-07 (M-077):** the container authenticates with its own `onboard-runner-zagros` key (200 with it / 401 without), and both tiers minted autonomously — developer (all three surfaces, 2.6 s) and browse (NetBird + Ory only, 2.4 s) — then torn down. The host-process daemon, its `/etc/hosts` line, and the hand-copied operator profile `/home/onboard-runner/.goat` are removed; the container is the sole executor and the operator's login is off the Thor. The live-fire surfaced + fixed one gap: the role chowned only the leaf operators dirs, so the container uid couldn't traverse the prior-owned repo-root (`mkdir /repo/ops: permission denied` after a successful claim) — fixed by owning the full chain.

### Pipeline robustness — closing the silent-strand paths

The end-to-end path had several ways for a proposal to strand invisibly or finalize
broken. Hardened (console `goat-id-console` + `onboard-runner`):

- **`env` is required at mint** (machine `POST /api/onboard/invite` + the console invite form). A blank env yields a blank-env proposal that a scoped runner (`--only-env <env>`) skips **forever** — the mint now fails closed (`env_required`) instead of creating an un-runnable invite.
- **Runner WARNs on env-skip.** When the runner skips an approved proposal on env mismatch it logs a WARN (was silent), so a stranded proposal surfaces in the log instead of vanishing.
- **Incomplete drafts can't advance.** An incomplete draft (blank tier/affiliation → manifest never assembled) may be **rejected** but not sponsor-approved/approved; advancing one let the runner claim it, then the native mint hard-failed on the unusable manifest with a misleading error. The decide gate now refuses with a clear "complete it or reject it" message.
- **Submit-time duplicate-email guard.** A second submission for an email already in-flight or onboarded is refused (`409 duplicate_email`) **before** the one-time invite token is consumed — a prior rejected/failed attempt still allows a retry. The refusal names the blocking row — `detail: "blocked by proposal <id> (status: <status>)"` in the error envelope — so an operator can act on it without reading `onboard.db` over SSH (#334); the candidate-facing `message` is unchanged (`onboard.html` renders only `error.message`). The runner's dedup backstops the concurrent-submit race and now finalizes the duplicate carrying the **original's** setup key + bundle path (was: a keyless `onboarded` row).
- **Failure reason is kept.** `POST …/result` persists the mint failure reason as the proposal `note`; the admin queue shows a **Provisioning** (onboarding) and **Failed** tab so stuck/failed proposals are visible, not just `approved`.
- **Keyless enrollee isn't dead-ended.** The welcome package's mesh-join step surfaces the mgmt enrollment URL even with no setup key in hand, so a keyless enrollee has a target.

### The management-API path fails closed

The runner reaches its goatnet's NetBird management API at the public
management name, mapped inside the container by `extra_hosts` to
`host-gateway` (the host, where traefik publishes :443). That mapping is the
one value the deploy renders that the mint depends on and nothing else
exercises, so it is verified at every layer that can see it rather than at the
next person's onboarding:

- **Deploy gate.** After the container is up the role curls
  `https://<public_mgmt_fqdn>/api/groups` from inside it and fails the play on
  anything but 200/401/403, naming the address it mapped. No converge path
  bypasses the role.
- **Runtime preflight.** With approved work in the queue, the runner probes
  the same URL before claiming. Unreachable means nothing is claimed and the
  proposals stay `approved`; one ERROR line carries the URL and the error.
  Claiming first would flip each row to `failed` and hand the recovery to a
  person.
- **Health.** The container healthcheck carries the same probe, so the row
  reads `unhealthy` and the existing container-health alert fires.
- **Release ladder.** Acceptance row `onboard-runner-reaches-mgmt-api` is a
  live row of the `onboarding-review-and-mint` flow; the runner's Go tests
  are its cheap suite. CI separately refuses a literal docker-bridge address
  in shared deploy artifacts (#959).

Status: built; deploy gate and healthcheck measured on zagros-dev 2026-09-09
(red on a wrong address, green on the right one); runtime preflight covered
by Go tests, deployed with the next runner image.

## Built-vs-designed

**All six phases (A–F) are built in code** (see the phase table for the per-phase detail + test coverage). Backend logic — the linter (`internal/onboardlint`), the proposal store + CRUD (`internal/registry`), and the intake/approve/batch endpoints — is covered by unit, parity, and httptest suites that pass in CI. The three admin pages (`onboard-intake.html`, `onboard-queue.html`, `onboard-batch.html`) were **driven end-to-end in a real browser** (Playwright against a locally-run portal): the intake form renders from schema⊕hints with live-lint and a valid submit storing a proposal; the queue renders it and approve emits the manifest YAML + `goat admin onboard` command; batch lints each record (valid/rejected/dry-run). That pass found + fixed one real bug (the form left `ory_site.sites` unset). The fast-follow **fully-automated onboarding** — the pull-model `onboard-runner` executing `goat admin onboard` off the approved queue rather than the operator running the emitted command — is now **built + deployed + live on the ZAGROS Thor** (2026-07-01; see §"Status" above): the standing daemon auto-mints operator-approved proposals end to end in ~3 s.

Reused, already-built substrate it stands on: the manifest schema + `validate.py` linter, the `goat admin onboard` orchestrator, the portal auth/`Principal`/`actorOf` model, the `peering/lint` workbench pattern, the `web/static/admin/` frontend area, the `internal/registry` persistence pattern, and the portal `audit_events` trail.
