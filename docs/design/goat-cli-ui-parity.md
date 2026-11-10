# goat-cli ↔ web-UI parity matrix

| Field | Value |
|---|---|
| Status | **Current audit for [goat#239](https://github.com/usareur-af/goatcore/issues/239).** Audited 2026-10-20 against the `goat` command tree on `main` (146 operator-facing leaf verbs). Deployment-qualified rows use the declared pins in `ops/pins/fleet-pins.yaml`; they are not claims about unverified host state. |
| Sources | CLI: `goat docs --markdown` from the branch build, backed by `crates/goat-cli/src/main.rs` + `crates/goat-cli/src/cmd/`. General web surfaces: `ops/goat-id-console/internal/handlers/handlers.go` + `web/templates/`; `ops/goat-zenoh-portal/internal/api/server.go` + `web/static/`. Purpose-built surfaces are enumerated below. |
| Related | goat-cli.md, id-admin-console.md, [goat-cli-error-codes.md](goat-cli-error-codes.md), [operator-action-audit.md](operator-action-audit.md), [onboarding-rails.md](../operations/onboarding-rails.md), [ADR 1019](../adr/1019-goat-cli-unified-operator-binary.md) |

## Why this exists

goat has a command-line operator surface, two general web surfaces (the id-console and portal),
and several purpose-built inspection pages. They are meant to stay complementary: an equivalent
web action should use the same backend contract, while a purpose-built page may cover a read-only
question without duplicating it in a general console (ADR 1019). In practice the CLI has grown
faster than the web UI, and the gaps kept surfacing as individual bug reports (goat#214,
goat#229–231).

This document is the full audit behind goat#239. It lists every operator-facing CLI command and
answers one question for each: **can you do this from the web UI?** Every "no" either links a
child issue with an owner, or states the reason the command is CLI-only on purpose.

**What counts as "the web UI".** The id-console and the portal are the two general operator web
surfaces. goat also runs purpose-built inspection pages — panoscope, stethoscope, flashlight,
microscope, netvis, the netbird dashboard, the per-site isitup pages. If one of those already
answers the same question as a CLI command, that counts as covered: building a second copy into
the console would just create the same drift problem in reverse. The matrix names which surface
covers each command.

### Web surfaces audited

An API route by itself does not count. Each qualifying surface below has a rendered, reachable
page or action; the source column is the reproduction path used for this audit.

| Surface | Rendered coverage checked | Source of truth |
|---|---|---|
| id-console | Identity, onboarding, SSH/operator keys, holds, offboard verification, and audit pages | `ops/goat-id-console/internal/handlers/handlers.go` and `ops/goat-id-console/web/templates/` |
| portal | Org/topic/schema, live inspector, and data-access pages | `ops/goat-zenoh-portal/internal/api/server.go` and `ops/goat-zenoh-portal/web/static/` |
| panoscope | Fabric COP plus `/top`, `/who`, and `/access` | `ops/panoscope/src/main.rs`, `ops/panoscope/src/embed.rs`, and `ops/panoscope/frontend/` |
| stethoscope | Interactive target diagnosis | [stethoscope.md](https://github.com/usareur-af/goatstuff/blob/main/docs/design/stethoscope.md) and `ops/ansible/roles/stethoscope/` |
| flashlight | Live topic tail | `ops/flashlight/src/main.rs` and `ops/flashlight/src/assets/` |
| microscope | Per-entity cross-layer timeline | microscope.md and `ops/ansible/roles/microscope/` |
| netvis | Live mesh reachability and peer ownership | netvis.md and `ops/ansible/roles/netvis/` |
| isitup | Per-site hosted-service status | `ops/ansible/roles/isitup/templates/` |
| NetBird dashboard | Peer roster and peer deletion | [mesh-change-control.md](mesh-change-control.md) (read-only and admin dashboard split) |

**What is left out.** Hidden and internal commands (`goat _complete`, the four easter eggs, and
`goat audit emit`, which is a forwarding hook for host-side services, not something an operator
runs). Also left out: comparing the `goat client` group against the goat-client desktop app —
that is a different surface with its own contract, and goat#239 rules it out of scope.

## Legend

| Status | Meaning |
|---|---|
| **UI:console** / **UI:portal** | You can do the same thing in that web surface today |
| **SIBLING(x)** | A purpose-built web surface *x* already answers this — counts as covered |
| **PARTIAL** | Part of it is in the web UI, the rest is not — the note says which part |
| **ABSENT** | The backend supports it (or reasonably could), but no web page offers it — needs an owner |
| **CLI-ONLY** | Kept out of the web UI on purpose — the row says why |

## 1. Local tooling — CLI-ONLY by definition

These commands manage files on the operator's own machine or teach the operator how to use the
tool. There is no server-side action for a web page to mirror, so the whole group is CLI-only
for the same reason: **they act on the local machine**.

| Verb | Note |
|---|---|
| `profile init\|show\|rotate\|path`, `context ls\|use\|show\|rm\|refresh` | Local credential/profile store (`~/.goat/`) |
| `init`, `tutor`, `hello`, `demo`, `mock` | Learning + synthetic-traffic tools; web analogs are the guide and the isolated zenoh-sandbox |
| `update`, `completion`, `docs`, `plugin list\|which`, `explain`, `compose`, `ui` | Self-update (the `get.<site>` page is the web half), shell integration, hosted-docs browser, TUI hops |
| `ca ls\|add\|rm` | The CA roots *this machine* trusts. Fetching roots for the operator would be deciding trust on their behalf, so it stays a deliberate local act — no server-side mirror |

## 2. Connectivity diagnostics from the operator's vantage — CLI-ONLY

These commands answer **"can MY machine reach X?"**. A web page is rendered on a server, so it
can only ever test the server's connectivity, not yours. And some of these (`admin lan`) exist
precisely for the moment the mesh is down — when no mesh-served web page would load anyway.

| Verb | Status | Note |
|---|---|---|
| `doctor` (incl. `--topic` write-precheck) | CLI-ONLY | Stepped local probes ZTOL → router → cert → portal |
| `status`, `heal` | CLI-ONLY | Local probe synthesis; `heal` actuates the local mesh client |
| `services` | SIBLING(isitup) | Same health endpoints, probed from the operator's vantage vs. the site prober |
| `admin lan scan\|doctor` | CLI-ONLY | Zero-dependency LAN triage — must work with no mesh |
| `names resolve\|publish-claim\|collect` | CLI-ONLY | Device-local fallback-resolver diagnostics; `collect` is infra-side |
| `client *` (status/up/down/bundle/mode/mesh/diagnostics/trust-roots/pqc) | CLI-ONLY | Wraps the local goat-clientd IPC socket; the GUI equivalent is goat-client itself (out of scope) |

## 3. Fabric observation

| Verb | Status | Note |
|---|---|---|
| `health` | SIBLING(panoscope) | Env-health surface renders the same `EnvHealthV1` envelopes ([environment-health-surface.md](environment-health-surface.md)) |
| `top` | SIBLING(panoscope) | Same health/footprint topics |
| `flow` | SIBLING(panoscope/flashlight) | `flow` itself consumes panoscope `GET /api/snapshot` |
| `presence` | SIBLING(panoscope) | Native liveliness (ADR 1040) |
| `inspect` | SIBLING(stethoscope) | Full parity — identical `GET /api/diagnose` SSE, identical verdicts |
| `microscope` | SIBLING(microscope) | CLI renders the service's `GET /api/entity` |
| `who` | SIBLING(panoscope) | Panoscope `/who` renders the cross-layer roster through `frontend/src/who.ts` and `GET /api/who/roster` + `/api/who/detail` (`src/main.rs`) |
| `narrate` | **ABSENT** | No web cascade-reflection view; flashlight is live-only. Owned by goat#1903 |
| `replay`, `sub --since` | **ABSENT** | Audit-tail history read (portal `GET /api/audit/presign`) has no web consumer; portal inspector shows latest/recent live only. Owned by goat#1903 |

## 4. Catalog + data plane (portal-backed)

| Verb | Status | Note |
|---|---|---|
| `org ls\|show` | UI:portal | `vendor.html` |
| `topic ls\|show\|create\|delete` | UI:portal | `topics.html` (incl. purge) |
| `topic export\|import` | UI:portal | `topics.html` with dry-run diff |
| `schema upload\|ls\|show` | UI:portal | Upload on `topics.html`; render on `schema.html` |
| `schema diff` | **ABSENT** | Portal renders one version at a time; the only web diff is the topic-*import* dry-run (`topics.js:411`), not a schema-version diff. Owned by goat#1904 |
| `sub` (live) | UI:portal | `inspector.html` live stream + decode |
| `pub` | CLI-ONLY | A browser holds no native mTLS fabric identity; the sanctioned non-Rust path is [rest-pub-bridge](zenoh-rest-pub-bridge.md) (ADR 1023), play path is the isolated sandbox |
| `pub --sealed` / `sub --open`, `access key`, `access key rotate` | CLI-ONLY | grant-crypto leaf keys are device-held by design (ADR 1106); a web backend that could seal/open on your behalf would hold your keys |
| `blob send\|get`, `pub --volatile-stream` | CLI-ONLY | Native Zenoh chunk protocol; the person-facing file path is goatdrop (goat-client) |

## 5. Access / COP / grants

| Verb | Status | Note |
|---|---|---|
| `access grant\|revoke` + requests loop | UI:portal | `data-access.html` (requests, grants, directory) |
| `access who\|why` | SIBLING(panoscope) | Panoscope `/access` renders identity access and the why-trace through `frontend/src/access.ts`; its `/api/who/detail` and `/api/access/query/why` handlers proxy the portal contracts |
| `access topic` | SIBLING(panoscope) | Panoscope `/access` exposes the reverse-topic query through `frontend/src/access.ts` and `GET /api/access/query/topic` |
| `access strays` (roster) | PARTIAL | The console's `/offboard-verify` page (goat#319, completed) renders receipt + four-plane presence from `GET /api/lookup/person-presence`, one person per call. No rendered surface offers the fleet-wide strays roster; goat#1906 owns that gap |
| `access audience` | **ABSENT** | API exists (`GET /api/access/audience`), no page. Owned by goat#1905 |
| `access govern list\|nominate\|remove` | **ABSENT** | Governance-admin nomination (ADR 1102 U9), API-only. Owned by goat#1905 |
| `access co-publish create-topic\|list-topics\|grant\|revoke` | **ABSENT** | Shared-topic co-publish (ADR 1105), API-only. Owned by goat#1905 |
| `access co-publish export` | CLI-ONLY | Materializes a group_var for ansible — an infra artifact, not an operator page |
| `access probation list\|hold\|decide` | **ABSENT** | The portal exposes only the API. The console's completed hold-status/list slices (goat#1041/#1042) read mesh hold records, not this probation ledger; member explanation and guarded release remain in goat#1038/#1044. No page renders or invokes these three portal actions; goat#1906 owns that gap |
| `access repo policy\|preview\|add\|remove` | UI:console | Identity card repo-access panel |
| `acl test` | CLI-ONLY | Compiled-in assertions, no network — a pre-deploy/CI tool; netvis is gated *by* it (ADR 1126) |
| `warrant port-sets` | SIBLING(netvis) | netvis already renders the declared port sets through the same shared code |
| `warrant why-blocked` | PARTIAL | netvis answers the *live* reachability half; the declared-plane "why is A→B:p blocked" query has no web form. Owned by goat#1909 |
| `warrant apply\|pause\|resume` | CLI-ONLY | Mesh-membership converger (mesh-change-control.md): runs on the scope's management host as root against the NetBird API; `pause`/`resume` write an audited, expiring hold file. Infra actuation, not an operator page |

## 6. Machines

| Verb | Status | Note |
|---|---|---|
| `machine affiliation repair` | CLI-ONLY | Operator-controlled repair of approved legacy spellings, with an explicit preview and revision-bound apply; intentionally no web repair form. See [the repair runbook](../operations/machine-affiliation-repair.md) |
| `machine enroll[--bulk]\|mesh-key\|revoke\|ls\|grant` | **ABSENT** | No web surface calls `/api/machines/*` at all; machine-enrollment.md already names a console "set machine quota" action as the intended future. Owned by goat#1128 |
| `machine ssh enable` | SIBLING(netbird-dashboard) | The dashboard's peer page toggles SSH; the CLI resolves the machine through the portal (custody-scoped), changes only the SSH flag, and records the action in the operator-action audit (ADR 1216) |

## 7. Identity + onboarding admin (console-backed)

The `goat admin` commands and the console call the same APIs (`/api/onboard/*`, `/api/users/*`,
`/api/org-registry/*`), so most rows here genuinely work from both surfaces.

| Verb | Status | Note |
|---|---|---|
| `admin invite` | UI:console | `/onboarding/invite` |
| `admin queue ls\|approve\|reject\|sponsor-*` | UI:console | `/onboarding` review queue |
| `admin invites ls\|revoke` | UI:console | |
| `admin ssh-key ls\|approve\|reject\|revoke` | UI:console | Queue and review actions at `/onboarding/ssh-keys`; installation is a separate console-to-shell handoff (goat#229/#231; see next row). goat#230's skip-reason view and goat#214's resubmit repair (PR #235) are complete |
| `admin ssh-key install` | CLI-ONLY | The console shows queue depth and preview/install commands for a shell-holding admin to run; it does not execute installation. The CLI wraps the Ansible installer for approved keys on the mgmt hosts, preserving the shell authority boundary (goat#229/#231) |
| `admin ssh-key submit <handle>` | CLI-ONLY | Submit-on-behalf exists to backfill old keys. The web flow requires each person to submit their own key from their own session, which proves the key is theirs |
| `admin operator-key request\|list\|approve\|reject\|revoke\|collect` | UI:console | Self-service `/operator-key` + admin queue. `request` (no handle) and `collect` are self-service on the person's own `vp_` token from console F-571 / goat-cli 0.5.11; before that the CLI door was operator-only and a first key was browser-only (#1373) |
| `admin operator-key reconcile` | CLI-ONLY | Drift-repair sweep between records and the portal key dir — an operator hygiene tool, not a page |
| `admin role set\|reactivate\|deactivate` | UI:console | Users page groups + state (same `/api/users/*`) |
| `admin onboard <handle>` | UI:console | The rails: queue approve → runner mints; CLI direct-mint remains the operator fast path |
| `admin onboard --revoke [--purge]` | PARTIAL (`--purge` CLI-only) | Main contains the step-up-gated Offboard action and runner claim flow from goat#372; goat#260 owns deployment and live-fire. Sibling legs goat#370, goat#371, and verification view goat#319 are complete |
| `admin rotate` | UI:console | In-place cert re-issue (ADR 1044). "Rotate fabric credential" on the user detail page (goat#320): identity-keyed — the console names the person, the portal resolves their slot (`POST /api/orgs/rotate`, the collect trust boundary); fresh key shown once; per-admin attribution via `X-Goat-Operator-Actor`. Anchor-keyed CLI path unchanged |
| `admin sponsorship` | **ABSENT** | Grant/revoke sponsor authorization (ADR 1071); API exists, no UI. Owned by goat#1907 |
| `admin org create` | UI:portal | Mint on portal landing page |
| `admin org delete` | CLI-ONLY | Deletes a portal slot directly and skips the revoke steps that offboarding runs, so a web button would invite hasty cuts |
| `admin org suggest` (+ review attach/new) | UI:console | Review-gate org box |
| `admin org tree\|register\|set-parent\|update\|alias\|merge\|unmerge` | **ABSENT** | Org-registry stewardship (ADR 1112) is CLI-only beyond the review-gate slice. Owned by goat#1907 |
| `admin members move` | **ABSENT (designed)** | Console mesh-membership lane is explicitly planned in id-admin-console.md — same engine, `surface: portal`. Dormancy release is owned by goat#1044; the general lane is owned by goat#1908 |
| `admin peer ls\|cut` | SIBLING(netbird-dashboard) | Mesh roster + peer delete exist there; the CLI adds the selector safety + audit |
| `admin pqc on\|off` | CLI-ONLY | SSH actuation that bounces the target's tunnel; the durable fleet path is the `rosenpass_enabled` group_var |
| `admin client onboard` | CLI-ONLY | Offline-CA bundle mint — CA material never touches a server |
| `admin operator onboard` | CLI-ONLY | Shells an ansible playbook; infra provisioning, not a web action |
| `admin platform-admin status\|grant\|revoke` | CLI-ONLY | Makes/unmakes a platform admin across every plane at once — ory, mesh, operator-key, sponsorship, and via ansible the ssh-key union + 1Password team (ADR 1182, [platform-admin-lifecycle.md](platform-admin-lifecycle.md)). Shells ansible and touches 1Password, so like `admin operator onboard` it is not a single web action; `status` is read-only |
| `admin storm brake` | CLI-ONLY | Break-glass during control-plane storm — the moment the web stack may itself be degraded |

## 8. Audit

| Verb | Status | Note |
|---|---|---|
| `audit ops` | CLI-ONLY | `OperatorActionV1` events are age-encrypted to the operator roster; a web backend able to decrypt them would hold the roster's keys and defeat the design ([operator-action-audit.md](operator-action-audit.md)). Surface-local audit *is* on the web: console `/audit`, portal audit events |

## 9. Federation — CLI-ONLY by design

> **Moving out of `goat`.** [ADR 1225](../adr/1225-federation-belongs-to-goatbuilder-one-command-one-decision.md) (Proposed) moves all federation code into the
> `goat-federation` library and `goatbuilder federate`. Until a goatbuilder release carrying the
> verbs is available to partner environments, `goat federate <verb>` runs the same library and
> prints one line naming the goatbuilder command; then it is removed from `goat`. The
> classification below holds for the goatbuilder verbs: they stay CLI-only.
> Status: WIP — gate: the federation end-to-end test drives both sides with goatbuilder federate alone

`goat federate` builds and checks the federation package two parties exchange to peer. The
`lint`/`compose`/`diff` package-processing subset is designed to run with **no network, no
credentials and no ambient host access**: inputs are explicitly supplied files and public
verifying keys; `compose` can also write an explicitly named output file. This subset can run in
an empty container. The other verbs inspect local state or write generated artifacts, as detailed
below. The group remains CLI-only as local package tooling and host-specific preparation;
[federation-package-spec.md](federation-package-spec.md) is the authority.

| Verb | Status | Note |
|---|---|---|
| `federate lint\|compose\|diff` | CLI-ONLY | Isolated package processing from explicitly supplied inputs; parties compose independently and compare agreement digests over any channel |
| `federate init\|verify` | CLI-ONLY | Require local declarations and host state: `init` reads the profile certificate and CA roots and measures capabilities; `verify` checks local conformance. Signing requires an explicitly supplied signing key; `init --out` can write the generated package |
| `federate apply` | CLI-ONLY | Renders configuration artifacts from the agreement for the selected local party; previews by default and writes directories/files only with `--confirm --out <dir>` |

## Cross-cutting parity defects (the #239 acceptance criteria beyond the matrix)

### Audit-record parity (goat#1910)

An action taken through the CLI is recorded as one `OperatorActionV1` event that names the person
The rule for every action both
surfaces share is the same event from the web path, tagged `surface: portal`; the planned console
members-move lane is the pattern.

### Error-semantics parity (goat#1911)

When something fails, the CLI translates the server's HTTP response into a named error code
with a remediation hint (the `E_*` codes in [goat-cli-error-codes.md](goat-cli-error-codes.md),
mapped in `crates/goat-cli/src/errors.rs`). But that translation lives inside the CLI. The web
UI shows whatever the server happened to return — sometimes a success page for an action that
silently did nothing, which is exactly the goat#214 bug (resubmitting a rejected SSH key looked
like it worked).

The durable fix is to move the error contract into the API responses themselves: the server
returns the code and the remediation text in the body, and both the CLI and the web templates
render it. Then the two surfaces cannot disagree. Individual symptoms keep being fixed one at a
time (goat#214 via PR #235, goat#414, goat#408); goat#1911 owns the shared contract.

## Keeping this matrix honest (CI guard)

`ops/ci/check-cli-ui-parity.py`, run by the `goat-cli-ui-parity` workflow, first checks this table
directly and fails when any ABSENT or PARTIAL row lacks an owning issue. This check does not need
the CLI binary. The guard then builds the branch's CLI, obtains its inventory from
`goat docs --markdown`, and fails when an operator-facing leaf verb has no recognized row in this
matrix. The inventory check excludes only the hidden and internal commands, `audit emit`, and the
`client` group named in the scope above.

## Maintenance rule

A PR that adds or renames an operator-facing `goat` verb updates this matrix in the same change
(same discipline as the error-codes doc). Rows marked ABSENT or PARTIAL must each name an owning
issue as `goat#<number>` or `#<number>`. CI enforces both rules, so no gap is recorded and left
unowned.
