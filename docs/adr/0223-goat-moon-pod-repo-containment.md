# ADR 0223 — goat-moon-pod repo containment + naming pattern + vendor extensibility

> **Scope: a moon pod is a learning tool.** Partners use it in an exercise environment to learn how goat works. It is not the operational path for sharing data between nations. Operationally, each sovereign runs its own environment and manages its own pods, and data passes between sovereigns only across boundary nodes that federate their environments ([ADR 1157](1157-cross-sovereign-federation-edge-terminated.md), [ADR 1208](1208-sovereign-environments-release-namespaces-and-grant-encryption.md)). Where this document describes a pod joining another nation's environment, read it as the learning setup.

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-05-04 |
| Repo scope | Multi |
| Supersedes | — |
| Superseded by | — |
| Related | [`0821-mission-partner-sharing-pattern.md`](0821-mission-partner-sharing-pattern.md), [`0217-shim-repo-containment.md`](0217-shim-repo-containment.md), [`0111-vendor-friendly-license-posture.md`](0111-vendor-friendly-license-posture.md), [`https://github.com/usareur-af/goatstuff/blob/main/docs/design/moon-pod-conformance.md`](https://github.com/usareur-af/goatstuff/blob/main/docs/design/moon-pod-conformance.md), [`../design/goat-moon-pod.md`](https://github.com/usareur-af/goatstuff/blob/main/docs/design/goat-moon-pod.md), [`../design/goat-moon-pod-builder-guide.md`](https://github.com/usareur-af/goatstuff/blob/main/docs/design/goat-moon-pod-builder-guide.md) |

## TL;DR (for a layperson)

Most plausible mission partners cannot stand up our full Partner Reference Architecture in any reasonable time — they don't have the engineering staff for it. We need a turnkey "easy button" they can deploy in 24 hours that still preserves their data sovereignty. We will publish a *requirements contract* that any conformant easy-button implementation must meet, ship our own reference implementation called `goat-moon-open-pod` in a separate public repo, and explicitly invite vendors to build their own conformant moon-pods. The goat trunk owns the contract and the boundary enforcement; the separate pod repo owns the implementation and the partner-facing artifacts; vendor pods live in vendor-controlled repos. Sovereignty is preserved because the partner runs the pod on their own hardware, with their own identity database, against their own audit log — we just provide the easy button.

## Context

[ADR 0821](0821-mission-partner-sharing-pattern.md) established the mission-partner-sharing pattern (release-namespace, BYOIDC, four-facet expectations, Partner Reference Architecture). [`partner-reference-architecture.md`](https://github.com/usareur-af/goatdocs/blob/main/docs/design/partner-reference-architecture.md) defined an eight-piece minimal stack a partner can stand up "in days." Two things forced this ADR:

1. **The PRA still presumes engineering capacity at the partner side.** A partner who can't run zenohd + WireGuard + an OIDC IdP + an audit sink + a release-namespace allowlist cannot adopt the PRA in any real timeframe. For the realistic majority of plausible partners, "in days" means "with several engineer-days of integration work on their side that they don't have."
2. **Multiple commercial products claim to provide capability in this space.** Several commercial data and command-and-control platforms have public API surfaces that could plausibly host conformant moon-pods. We need a posture that does not entangle goat with any single vendor and does not block any vendor from building.

Both pressures point in the same direction: **publish the requirements as the contract, ship one reference implementation, invite the larger community to build.** Don't try to own every implementation; don't pick winners.

Three options were weighed for repo containment:

1. **Receiver pod in trunk.** Add a new top-level `goat-moon-open-pod/` directory in this repo. Pros: zero new repo to provision; tight coupling to substrate. Cons: trunk grows by another product; the pod has its own release cadence that doesn't match trunk's; partner-facing README + quickstart + hardware BOM bloat trunk; vendor moon-pods would have no parallel structure to mirror.
2. **Separate pod repo, software-only.** Pod bundle in a sibling repo; if hardware-imaging happens, third repo. Pros: software / hardware concerns separated. Cons: artificially splits one product into two repos; both have to release together; the "pre-imaged hardware tier" is a delivery shape of the same product, not a different product.
3. **Separate pod repo, software + hardware as one product.** The pod lives in `USAREUR-AF/zagros-moon-pod` as a single Apache-2.0 repo (private during MVP, public at GA); software bundle is the primary artifact; `hardware/` subtree holds BOM, imaging pipeline, and pre-imaged delivery shape. Pros: matches existing pattern from [ADR 0217](0217-shim-repo-containment.md); discoverable from `../integrations/INDEX.md`; independently versioned; severable; vendor pods follow the same shape in their own repos.

## Decision

Adopt **option 3: separate pod repo, software + hardware as one product, with vendor-extensibility bones in trunk.**

### Repo containment

| Class | Repo home | Examples |
|---|---|---|
| Requirements contract | Trunk: [goatstuff `docs/design/moon-pod-conformance.md`](https://github.com/usareur-af/goatstuff/blob/main/docs/design/moon-pod-conformance.md) | Authoritative seven-requirement contract; cited by every implementation |
| Boundary enforcement | Trunk: `zenoh-plugins/release-bridge/` + Ansible roles | Schema-enforcer, allowlist, conditioning, audit emission. *We* enforce the contract at the goat boundary; the pod implements its side |
| Coordination plane | Trunk: `terraform/` + `ops/ansible/` (partner-net netbird-mgmt instance) | Distinct from main fabric mgmt-plane; isolated blast radius |
| Reference implementation | Separate repo: [`USAREUR-AF/zagros-moon-pod`](https://github.com/USAREUR-AF/zagros-moon-pod) | Apache 2.0 (private during MVP, public at GA); bundle + per-IdP overlays + hardware BOM + imaging + first-boot scripts |
| Vendor implementations | Vendor-controlled repos: `goat-moon-<product>-pod` | Any commercial platform — vendor decides license + hosting |
| Vendor cookbooks | Trunk: `docs/integrations/moon-pods/` | Per-vendor mapping sketches citing public-API material only |
| Conformance smoke-test | Trunk: [`tests/moon-pod-conformance/`](../../tests/moon-pod-conformance/) | Walks 24-hour deployment path, asserts seven requirements at boundary; runs against any candidate moon-pod |
| Builder guide | Trunk: [`docs/design/goat-moon-pod-builder-guide.md`](https://github.com/usareur-af/goatstuff/blob/main/docs/design/goat-moon-pod-builder-guide.md) | "Want to build your own goat-moon-pod? Here's the contract + test suite + worked-example pointer" |

### Naming pattern

`goat-<plane>-<provider>-<service>` where:

- `goat` — project
- `<plane>` — relationship plane. `moon` for sovereign-orbital partner relationships.
- `<provider>` — `open` for our reference implementation; vendor-product-name otherwise (`<product>`).
- `<service>` — service type. `pod` for a deployable instance.

The abstract spec is "**goat-moon-pod**" (a concept, not a repo). Concrete implementations are `goat-moon-<provider>-pod`. The full definition of the moon metaphor lives once in [`moon-pod-conformance.md`](https://github.com/usareur-af/goatstuff/blob/main/docs/design/moon-pod-conformance.md) §"The 'moon' metaphor"; everywhere else cites it.

Future relationship planes (if any emerge) follow the same pattern with a different `<plane>` term. We do not pre-create those slots.

### Vendor extensibility bones (minimum, no over-build)

We leave just enough scaffolding for vendors to build their own conformant moon-pods. Per the "don't overdo preoptimization" guidance:

- ✅ **Conformance contract** — [`moon-pod-conformance.md`](https://github.com/usareur-af/goatstuff/blob/main/docs/design/moon-pod-conformance.md). Authoritative; cited by every implementation.
- ✅ **Conformance smoke-test suite** — [`tests/moon-pod-conformance/`](../../tests/moon-pod-conformance/). Vendor runs locally before requesting evaluation; CI runs continuously against `goat-moon-open-pod`.
- ✅ **Builder guide** — [`goat-moon-pod-builder-guide.md`](https://github.com/usareur-af/goatstuff/blob/main/docs/design/goat-moon-pod-builder-guide.md). Explains the contract, the test suite, and points at `goat-moon-open-pod` as a worked example.
- ✅ **Per-vendor cookbooks** — `docs/integrations/moon-pods/`. One short doc per named-vendor candidate sketching how their public API maps to `release/<partner>/...`.
- ✅ **Discoverability surface** — `docs/integrations/INDEX.md` gains a "goat-moon-pods" section listing the open one + cookbooks + reserved slots.
- ❌ **Per-vendor SDK / `goat-moon init` scaffolder** — deferred until at least one external vendor actually starts building.
- ❌ **Vendor-conformance-certification process** — deferred until at least one external vendor actually ships a conformant moon-pod.
- ❌ **Cross-moon-pod federation / peering** — out of scope; each moon is sovereign-orbital, no inter-moon relationships at this layer.

## Why this and not in-tree

Three load-bearing reasons:

1. **Release-cadence severability.** The pod ships partner-facing software with its own quickstart, troubleshooting, and upgrade documentation. Its release cadence is partner-driven, not trunk-driven. Bundling into trunk forces every pod release to ride a trunk commit; splitting them lets each evolve cleanly.
2. **Vendor symmetry.** A vendor moon-pod lives in the vendor's own repo. Our reference implementation living in *its* own repo creates a symmetric structure: one trunk-owned contract, N independently-released pod repos. In-tree breaks symmetry.
3. **Discoverability and forkability.** A partner who wants to fork the pod (the entire point of "unleash the larger technological community") forks one repo, not a subdirectory of trunk. Vendors evaluating "should I build a goat-moon-pod?" land on `goat-moon-open-pod` as a complete worked example, not a subdirectory.

## Why software + hardware in the same repo

The pre-imaged hardware tier is a **delivery shape** of the same product, not a different product. Same first-boot scripts, same bundle, same IdP overlays, same web UI, same conformance posture. The only difference is whether the partner runs `install.sh` on their own Linux box or plugs in a device we shipped. Splitting hardware into a third repo would force two-repo synchronization for what is logically one artifact.

The `hardware/` subtree contains BOM (OnLogic / Lenovo Tiny / NUC / RPi 5 8GB candidates), Packer templates, imaging pipeline, and tamper-evident packaging spec. Whether we ever actually image and ship a unit is per-partnership; the pipeline is available as soon as it's built.

## Why we name vendor candidates explicitly

Naming vendor candidates in [`moon-pod-conformance.md`](https://github.com/usareur-af/goatstuff/blob/main/docs/design/moon-pod-conformance.md) and providing per-vendor cookbooks **does not endorse** their products and **does not commit** us to evaluating them. It does:

- Signal that we know they have public API surfaces and could plausibly build conformant moon-pods.
- Provide a worked starting point for any of them or any partner using their products.
- Make clear that the door is open — to them and to anyone else. The list is named, not exclusive.

The cookbooks cite public-API material only. Any vendor preview material stays in a private per-vendor shim repo per the [ADR 0217](0217-shim-repo-containment.md) shim-containment rule; trunk references stay meta-level.

## Edge cases

- **Vendor builds a goat-moon-pod that fails conformance.** We document the gap in the cookbook; we do not "endorse" the failed implementation. Vendor decides whether to fix or fork.
- **Vendor's product is a hosted SaaS that violates requirement 6 (sovereignty floor).** It cannot be a goat-moon-pod, because the contract requires partner-controlled custody of data, identity, and audit. The vendor can publish a self-hosted variant that satisfies the floor, or the partner picks a different path.
- **Two vendors build conflicting moon-pods (e.g., both claim `goat-moon-<product>-pod`).** We do not arbitrate the name. Whoever publishes first owns the slot in our cookbook; the second vendor picks a differentiating suffix or accepts that they're forking the first.
- **A partner uses a vendor moon-pod that subsequently goes out of support.** Partner can migrate to another moon-pod (or `goat-moon-open-pod`) under requirement 7 (reversibility). We do not rescue partners stranded by vendor decisions.

## Consequences

- **Trunk grows three docs (requirements + abstract spec + builder guide), one ADR, one cookbook subdir, one conformance-test subdir, and updates to four existing docs (PRA, mission-partner-sharing, charter, integrations INDEX).** Manageable; standard docs-discipline pattern.
- **A new repo (`USAREUR-AF/zagros-moon-pod`) requires standing up.** ✅ Done 2026-06-01 (Block 60C): `USAREUR-AF/zagros-moon-pod` created (private during MVP, Apache-2.0) with the live-validated v0 MVP as the initial commit, extracted from the monorepo `incubator/goat-moon-open-pod/` tree.
- **Partner collaboration surface split out.** ✅ Done 2026-06-10: a *separate, scrubbed* repo (private, Apache-2.0) was created where partners co-develop the pod openly. Rationale: a shared repo with internal collaborators is exactly where us-internal content leaks; a clean repo gives blast-radius separation. It is a fresh repo (not a GitHub fork — a fork keeps an upstream network link + shared issues, defeating the separation), defaulted for that partner audience, and scrubbed of all goat-internal references (private-monorepo links, internal registry coupling, internal hostnames/mesh-IPs, the operator-provisioning runbook); the boundary contract is vendored self-contained. The same-class **no-stack-over-specification** correction (bundle is the source of truth for the router endpoint; the bilateral inbound namespace is parameterized) was applied to the canonical repo, the incubator mirror, and the partner repo together. Naming note: this is an *instance/audience* split, not a vendor implementation — the canonical `open` reference still owns the contract; the partner repo is the same reference pod profiled + scrubbed for one partner audience.
- **Vendor relationship work** is deliberately *not* taken on by goat. We provide the contract and the cookbook; vendors decide whether to build. If a partner shows up running a conformant vendor moon-pod, we evaluate it against the contract at partnership-onboarding time.
- **Block 60** (implementation-plan.md) tracks all the implementation work as a parallel track in the two-month cadence (sub-blocks 60A-H).
- **Severability is enforced by structure.** Deprecating `goat-moon-open-pod` is a repo-level decision; it does not entangle with trunk or with vendor moon-pods.

## Acronyms / project-specific terms

See `GLOSSARY.md`.
