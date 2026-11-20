# Fieldkit and the Bleating Edge design system

| Field | Value |
|---|---|
| Status | Designed, not built ([ADR 1251](../adr/1251-every-source-keeps-its-identity-through-a-pod-carriage-rules-provenance-at-entry-and-pod-local-authority.md), Accepted) |
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
- A CI scope guard fails the build when a tool gains an action with no CLI equivalent.

## 4. Bleating Edge

- **Tokens:** a high-contrast palette with sunlight and dark modes; one type scale; touch targets of at least 44 px.
- **Components:** connected or cut-off banner; list row for a source or rule; action button with a result preview; duration
  picker (4, 24, 72 hours); confirm sheet; "same as" CLI strip; waiting list; error with what-to-do text.
- **Delivery:** one CSS file and one small script, included into each tool at build time, plus a starter template so a new tool
  is its API and an action list.

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

Issues for refinement work carry the label `open-for-refinement` and a role label, so the work is visible and owned.

## 6. Fieldkit Gatepost

Who may use this pod, managed on site with no backhaul.

| Action | Same as |
|---|---|
| Status | `goat pod status` |
| Let in (a source, for 4, 24 or 72 hours, with a reason) | `goat pod rules allow <source> --until …` |
| Shut out | `goat pod rules revoke <source>` |
| Extend | `goat pod rules extend <rule> --until …` |
| Bring back | `goat pod rules reactivate <rule>` |
| Waiting | `goat pod pending` |

Open in the build: how a tablet that holds neither a goat certificate nor a card signs in, without adding a password system.
