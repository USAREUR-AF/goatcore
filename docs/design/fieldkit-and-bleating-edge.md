# Fieldkit and the Bleating Edge design system

| Field | Value |
|---|---|
| Status | Built: Bleating Edge, Fieldkit Gatepost (tested against a local mock of the pod API), the scope guard and the console ratification queue; Gatepost against a real pod is not built until the pod API serves it; usability with operators unmeasured ([ADR 1251](../adr/1251-every-source-keeps-its-identity-through-a-pod-carriage-rules-provenance-at-entry-and-pod-local-authority.md), Accepted) |
| Scope | Small offline pages that let an operator who is not a developer do one job a developer would otherwise do with the CLI |
| Related | [`pod-carriage-and-provenance.md`](pod-carriage-and-provenance.md) (Gatepost's actions), [`operator-action-audit.md`](operator-action-audit.md) (one door, one audit) |

## 1. The rule

The API and the `goat` CLI are primary. A Fieldkit tool is a small, single-purpose page over that same API: a fixed list of
actions, no logic or state of its own, and nothing it can do that the CLI cannot. Both faces enforce the same limits and write
the same audit record.

## 2. Names

| Layer | Name | Seen by |
|---|---|---|
| Design system | Bleating Edge | Developers building Fieldkit tools |
| Tool family | Fieldkit | Operators |
| Tools | Fieldkit plus a scope word, for example Fieldkit Gatepost | Operators |

Operator-facing names are plain words, because operators from many nations read them in a second language. A new tool is added
only for a real, recurring job.

## 3. Every Fieldkit tool

- One file, shipped in the image of the host that serves it. No framework, no build step, no outside assets: it works with no
  backhaul.
- At most eight actions. Each states its result before it runs and shows the CLI command it equals.
- Signs in with the person's goat certificate or card; the serving host checks it against cached CA roots.
- A CI scope guard fails the build when a tool gains an action with no CLI equivalent. Each action is declared once, on
  its element: `data-action` (a name), `data-cli` (the `goat` verb it equals) and `data-api` (`METHOD /route`). The guard
  checks each against the allow list, refuses more than eight, and refuses any other way of reaching the network; a tool
  calls its API only through Bleating Edge's `BE.call`, which reaches only declared routes. CI also checks every verb on
  the allow list against the CLI's command tree, read from the committed `goat docs --markdown`
  (`docs/goat-cli-book/src/reference.md`, which goat-cli's tests hold equal to the clap tree), so a Gatepost action whose
  `goat pod` verb is renamed or removed fails the build.

## 4. Bleating Edge

- **Tokens:** a high-contrast palette with sunlight and dark modes; one type scale; touch targets of at least 44 px.
- **Components:** connected or cut-off banner; list row for a source or rule; action button with a result preview; duration
  picker (4, 24, 72 hours); confirm sheet; "same as" CLI strip; waiting list; error with what-to-do text.
- **Delivery:** one CSS file and one small script, included into each tool at build time, plus a starter template so a new tool
  is its API and an action list. "Build time" is `ops/fieldkit/inline.py`, run by the developer: the committed tool file
  carries the copy, so the host serves it with no build step, and CI fails when a copy is stale.

## 5. Who builds it, and the boundary

Fieldkit and Bleating Edge are work for software developers, full-stack developers, and UX researchers and designers: the
design system, each tool, field usability testing with operators who are not developers, and the wording of actions and
errors. The same people fit the portal and console experience, onboarding flows and the plain-language error catalog. That
work is opened to them deliberately, with its own labeled issues and a clear definition of done (a usability test with
operators passes; the design system is followed).

The boundary that keeps it away from load-bearing parts of goat:

| Mechanism | What it guarantees |
|---|---|
| Contract first | The platform team owns the API, the CLI and their response schema. Tools build against that contract and a local mock in CI, with no goatnet needed |
| No new authority | Every action maps to an existing CLI command (the scope guard); a tool holds no state or logic, so a defect can show something wrong but never grant something |
| Path ownership | Tool code lives in its own directories with its own code owners. Router templates, portal authorization, certificate minting and ACL generators stay on the sensitive-paths list, so changes there need platform review whoever proposes them |

### Build approach: a complete v0.1, then room to grow

Each piece ships first as a minimal but complete v0.1: it works end to end, is tested and is documented, with no gaps left for
someone else to fill. The pieces marked **open for refinement** are then handed to the software and UX roles to refactor,
rebuild and mature. Load-bearing pieces stay with the platform team.

| Piece | v0.1 (built first) | Open for refinement by | Stays with platform |
|---|---|---|---|
| Bleating Edge | Tokens, the eight components, one CSS and one script, a starter template | UX and design: research-led refinement, accessibility review, localization, component documentation | — |
| Fieldkit Gatepost | The six actions working against a mock pod and a real one, with the scope guard | UX research and full-stack: field usability study, layout and wording, tablet sign-in | The action list and its mapping to the CLI |
| Ratification queue in the console | List, ratify, let lapse | Full-stack and UX | The ratification rules |
| `goat pod` CLI and the pod API | Every verb in the design doc | — | All of it |
| Provenance signing and the ingress adapter | ASTERIX adapter, three grades, CLI verification | Software: further feed decoders and display of provenance grades | Signing, key handling, grades |
| Carriage rules and ACL rendering | Rules, rendering, drift check | — | All of it |

What is built, and where:

| Piece | Where | Status |
|---|---|---|
| Bleating Edge | `ops/fieldkit/bleating-edge/` (`bleating-edge.css`, `bleating-edge.js`, `starter.html`); `ops/fieldkit/inline.py` copies it into each tool | built |
| Fieldkit Gatepost | `ops/fieldkit/gatepost/gatepost.html`; local mock `mock_pod_api.py`; browser checks `ops/goat-zenoh-portal/web/e2e/fieldkit/` (config `playwright.fieldkit.config.ts`, the portal's Playwright install) | built against the mock; served by the pod API (`ops/goat-pod-api`) |
| Scope guard | `ops/ci/check-fieldkit-scope.py` with the allow list `ops/ci/fieldkit-cli-allowlist.txt`, cross-checked with `--docs` against the CLI's command tree; runs in `docs-guards.yml` and `fieldkit.yml` | built |
| Ratification queue | Portal page `ops/goat-zenoh-portal/web/static/carriage-pending.html` ("Pod rules waiting", linked from Data Access); each decision states its result and its `goat carriage` command in an in-page confirm sheet | built |
| Field usability test with operators | — | unmeasured |

The ratification queue is in the portal web UI, not the identity console, because org leads already decide their org's
data requests there (Data Access, the `data.govern.org` role) and the portal serves the carriage routes it calls.

Issues for refinement work carry the label `open-for-refinement` and a role label, so the work is visible and owned.

## 6. Fieldkit Gatepost

Who may use this pod, managed on site with no backhaul.

| Action | Same as |
|---|---|
| Status | `goat pod status` (its rule list is `goat pod rules ls`) |
| Let in (a source, for 4, 24 or 72 hours, with a reason) | `goat pod rules allow <source> --until … --reason …` |
| Shut out | `goat pod rules revoke <source> --reason …` |
| Extend | `goat pod rules extend <rule> --until … --reason …` |
| Bring back | `goat pod rules reactivate <rule> --until … --reason …` |
| Waiting | `goat pod pending` |

The reason, and the end time on Bring back, are on every change because the pod API requires them (§7.4 of the pod
carriage design). Gatepost signs in by the browser presenting the person's goat certificate or card to the pod.

Open in the build: how a tablet that holds neither a goat certificate nor a card signs in, without adding a password system.
