# Mission partner sharing — design (companion to ADR 0821)

> **Scope: a moon pod is a learning tool.** Partners use it in an exercise environment to learn how goat works. It is not the operational path for sharing data between nations. Operationally, each sovereign runs its own environment and manages its own pods, and data passes between sovereigns only across boundary nodes that federate their environments ([ADR 1157](../adr/1157-cross-sovereign-federation-edge-terminated.md), [ADR 1208](../adr/1208-sovereign-environments-release-namespaces-and-grant-encryption.md)). Where this document describes a pod joining another nation's environment, read it as the learning setup.

| Field | Value |
|---|---|
| Status | Draft (Phase 2D-late / Phase 3 commitment) |
| ADR anchor | [`../adr/0821-mission-partner-sharing-pattern.md`](../adr/0821-mission-partner-sharing-pattern.md), [`../adr/0223-goat-moon-pod-repo-containment.md`](../adr/0223-goat-moon-pod-repo-containment.md) |
| Companions | [`partner-reference-architecture.md`](https://github.com/usareur-af/goatdocs/blob/main/docs/design/partner-reference-architecture.md), [`moon-pod-conformance.md`](https://github.com/usareur-af/goatstuff/blob/main/docs/design/moon-pod-conformance.md), [`goat-moon-pod.md`](https://github.com/usareur-af/goatstuff/blob/main/docs/design/goat-moon-pod.md), [`goat-moon-pod-builder-guide.md`](https://github.com/usareur-af/goatstuff/blob/main/docs/design/goat-moon-pod-builder-guide.md), [`../partner-templates/data-sharing-charter.md`](https://github.com/usareur-af/goatdocs/blob/main/docs/partner-templates/data-sharing-charter.md) |
| Implementation tracker | `../project/implementation-plan.md` Block 24, Block 60 |

## Why this exists

The design-level companion to ADR [`0821`](../adr/0821-mission-partner-sharing-pattern.md). The decision (reserve `release/`, BYOIDC, four-facet expectations, bridge data-conditioning, PRA, charter) lives there. This document is the design doc that future contributors and partner integrators read alongside the ADR.

## Maturity caveat

GOAT-Net is at proof-of-concept stage. We are in uncharted territory — sovereign tactical-edge data backbones at this shape and scope do not have established operational precedent at theater scale. The direction is sound and anchored in mainstream distributed-systems patterns; specific scaling, performance, and operational characteristics will be discovered, not asserted. A viable scaling plan exists (ADR [`../adr/0216-goat-net-federation-and-site-planes.md`](../adr/0216-goat-net-federation-and-site-planes.md), design at `goat-net-scaling.md`) and becomes operative *if* GOAT-Net earns it through the early phases. Partner expectations in §"Performance" below are calibrated honestly — commercial-internet-realistic, not aspirational SLAs.

## The shape in one paragraph

Mission partner data sharing at unclassified rides on a dedicated key-expression namespace (`release/<partner>/...`) with a default-deny baseline. Partners federate identity via any standards-compliant OIDC IdP — we don't prescribe theirs. Data crossing the boundary runs through a conditioning checkpoint that mirrors the egress proxy in shape (default-deny, per-partner per-topic allowlist, identity-bound, audit-logged). Four-facet expectations cover what we hold the partnership to: security, functionality, performance, reciprocity. We publish a Partner Reference Architecture (PRA) that partners may adopt and a Partner Data Sharing Charter that formalizes the framework — the charter is *necessary* to enable sharing but does *not* compel either side to share specific data. The pattern is the project's externally-facing surface for **GOAT-Net** federation.

## The four-facet expectations

### Security

- **TLS 1.3** on every partner-side hop touching `release/<partner>/...` data. No plaintext.
- **Identity assurance** equivalent to NIST SP 800-63 IAL2 / AAL2 or above, federated via OIDC.
- **Audit retention** ≥ 90 days on partner-side audit logs covering accesses to `release/<partner>/...` data.
- **Incident notification SLA** — default 24h confirmed / 72h suspected.

### Functionality

- **OIDC-compliant IdP** with a discovery endpoint we can reach.
- **Zenoh wire protocol** routability (or shim through a vendor protocol per the shim pattern, ADR [`../adr/0209-shim-pattern.md`](../adr/0209-shim-pattern.md)).
- **Partner identity assertion** at the release-namespace boundary — every release event is bound to a partner-side identity, not just a network address.
- **Optional inbound subscription capability** — partner may subscribe to topics we expose under `release/goat/...`.

### Performance

Calibrated to commercial transport-based internet, *not* our internal SLOs:

- Round-trip latency to mutual relay: **typical < 500ms**, episodic spikes acceptable.
- Sustainable throughput: **typed per partnership** (declared in charter Appendix A); not absolute.
- Monthly availability: **≥ 99%**, commercial-internet-realistic.

These are **expectations, not SLAs.** Hard SLAs require a contractual basis the charter does not provide.

### Reciprocity

The release-namespace pattern is bilateral. If we expose `release/goat/<topic>/...` to partner X, X *may* expose `release/<partner>/<topic>/...` to us under the **same** convention — same default-deny baseline, identity-bound and policy-gated, audit-logged with symmetric four-facet expectations.

We do not **require** reciprocity (some partnerships are intrinsically one-directional) but the pattern supports it cleanly. Where reciprocity exists, the charter covers both sides under one document.

## Bring-your-own-OIDC partner identity federation

Partners use any standards-compliant OIDC IdP. We provide working integration examples for **Dex, Kanidm, Zitadel, Keycloak, and Ory** as a starting set — these are *not* a list of approved IdPs; any partner with a standards-compliant OIDC IdP whose discovery URL we can reach can federate.

### Our side

Ory Kratos (per ADR [`../adr/0411-ory-idp-mvp.md`](../adr/0411-ory-idp-mvp.md)) with an OIDC connector pointed at the partner's discovery URL. Each partnership gets a connector configuration in `ops/ansible/group_vars/partner-<partner>/oidc.yml`. Partner-issued identity assertions are accepted within the partner's `release/<partner>/` namespace scope only — no authority outside that scope.

### Per-IdP example configs

`docs/operations/partner-idp-integration/` contains one runbook per starting-set IdP:

- `dex.md` — minimal Dex config + Kratos connector snippet
- `kanidm.md` — Kanidm OAuth2 client setup + Kratos connector snippet
- `zitadel.md` — Zitadel project/application/client setup + Kratos connector snippet
- `keycloak.md` — Keycloak realm/client setup + Kratos connector snippet
- `ory.md` — partner running Ory; same-stack federation

Partners with a different IdP follow the OIDC-discovery convention and adapt; we will accept PRs that add new examples.

## Bridge data-conditioning checkpoint

Mirrors the egress proxy (ADR [`../adr/0806-egress-proxy-cascade.md`](../adr/0806-egress-proxy-cascade.md)) in shape. Per-partner ingress on bridge VPSes routes through:

1. **Default-deny baseline.** No `release/<partner>/<topic>` crosses until an explicit allowlist entry exists.
2. **Per-partner per-topic allowlist.** Allowlist entries are policy-engine-gated (ADR [`../adr/0805-policy-engine-active-passive.md`](../adr/0805-policy-engine-active-passive.md)). "Releasable to partner X" is a deliberate, auditable decision per topic.
3. **Identity validation.** Partner-side identity assertions validated against the partner's federated IdP at release time.
4. **Audit-log integration.** Every release event lands in the audit-log integrity tier (ADR [`../adr/0811-audit-log-integrity.md`](../adr/0811-audit-log-integrity.md)).
5. **Schema declaration check.** Released topics must have `attest/schema/...` or `attest/schema-declaration/...` entries per [`data-dictionary.md`](data-dictionary.md). The schema-enforcer interceptor applies at the boundary too.

The shape mirroring is deliberate — when CDS conditioning (ADR [`../adr/0504-cds-conditioning.md`](../adr/0504-cds-conditioning.md)) eventually layers on for cross-classification release, the operational shape transfers.

### Per-partner conditioning logic — WASM sandbox

Per-partner *transform / redaction / drop* logic (e.g. round position to 100m grid, strip `source_sensor_id`, drop tracks with `confidence < 0.6`) is the place external-author code legitimately needs to run inside a host process we own. Two not-good options — accumulating every partner's policy in our Go/Rust release-bridge (we become the bottleneck and own the semantics, contra the anti-platform posture) or loading partner-supplied native plugins (partner gets full process memory) — are both rejected.

The middle path is the **WASM conditioning sandbox**: the partner ships a signed `.wasm` module exposing a narrow `condition(input) -> verdict` function; the release-bridge embeds Wasmtime with no network / no filesystem / no env imports + fuel + memory + wall-clock caps; the module runs once per message in a fresh instance with audit-by-construction. Sandbox boundary is by-construction; blast radius is one message and one partner.

This is the **first WASM service** in goat and the only carve-out from ADR [`../adr/0501-rust-interceptors.md`](../adr/0501-rust-interceptors.md) (which still governs our own internal interceptors). Full design at [`wasm-conditioning-sandbox.md`](wasm-conditioning-sandbox.md). Anchor decision: ADR [`../adr/0513-wasm-conditioning-sandbox.md`](../adr/0513-wasm-conditioning-sandbox.md). Implementation tracker: `../project/implementation-plan.md` Block 28.

### Implementation surface

| Path | Purpose |
|---|---|
| `ops/ansible/roles/bridge-conditioning/` | Bridge VPS conditioning role |
| `ops/ansible/roles/release-bridge-wasm/` | Wasmtime install + per-partner module dirs + key-chain |
| `zenoh-plugins/release-bridge/` | Rust Zenoh interceptor; embeds Wasmtime; per-message `condition()` |
| `ops/wasm-modules/reference/conditioner-rs/` | Reference Rust partner module |
| `ops/wasm-modules/reference/conditioner-go/` | Reference TinyGo partner module |
| `ops/policy-engine/schema/release_policy.sql` | Spock-replicated `release_policy` table |
| `ops/policy-engine/schema/wasm_module.sql` | Spock-replicated module-version + activation-state table |
| `ops/policy-engine/release-approval/` | Partner-release approval workflow extension |

## Reciprocity in operation

Where a partnership is bilateral:

- **We expose** `release/goat/<topic>/...` (deliberately released to the partner) on our bridge.
- **Partner exposes** `release/<partner>/<topic>/...` (deliberately released to us) on their bridge.
- Both sides apply default-deny + per-topic allowlist + identity-bound + audit-logged.
- Each side runs their own conditioning checkpoint; identity federation is bidirectional (their IdP for their users reaching ours; our IdP for our users reaching theirs).
- The Partner Data Sharing Charter (when a partnership is reciprocal) covers both directions in one document.

The reciprocity *recommendation* is part of the four-facet expectations; the *requirement* is not. A one-directional partnership signs a reciprocity-free charter and operates accordingly.

## The Partner Reference Architecture (PRA)

Recommended-not-required minimal Zenoh-on-ZTOL stack. Partners may take it whole, take pieces, or ignore. Full design at [`partner-reference-architecture.md`](https://github.com/usareur-af/goatdocs/blob/main/docs/design/partner-reference-architecture.md). Sketch:

- One Zenoh router (single node).
- WireGuard ZTOL over commercial internet.
- One OIDC IdP (any standards-compliant).
- TLS 1.3 on non-WireGuard hops.
- `release/<partner>/...` namespace discipline.
- One audit sink (90d-capable).
- Health endpoint at `health/partner/<partner>/...`.

The PRA deliberately omits everything in our stack that exists for *our* reasons — active-active multi-site, paired pgBackRest, three-tier NTP, phoenix-native rebuild discipline, FIPS PQC boundary node. Partners adopting the PRA get a clean interop starting point in days.

## Tier-0 — goat-moon-pod easy button (added 2026-05-04)

The PRA still presumes engineering capacity at the partner side. For partners who don't have it, **Tier-0** is a turnkey containerized appliance — `goat-moon-pod` — that bundles all six required pieces and deploys in 24 hours by a non-engineer.

- **Contract** (the seven requirements any moon-pod must meet): [`moon-pod-conformance.md`](https://github.com/usareur-af/goatstuff/blob/main/docs/design/moon-pod-conformance.md).
- **Abstract spec**: [`goat-moon-pod.md`](https://github.com/usareur-af/goatstuff/blob/main/docs/design/goat-moon-pod.md).
- **Reference implementation**: [`USAREUR-AF/zagros-moon-pod`](https://github.com/USAREUR-AF/zagros-moon-pod) (separate repo, Apache 2.0 — private during MVP, public at GA, per [ADR 0223](../adr/0223-goat-moon-pod-repo-containment.md)).
- **Builder guide for vendors**: [`goat-moon-pod-builder-guide.md`](https://github.com/usareur-af/goatstuff/blob/main/docs/design/goat-moon-pod-builder-guide.md). Vendors explicitly invited to build their own conformant moon-pods. Per-vendor cookbooks at `../integrations/moon-pods/`.
- **Conformance smoke-test**: [`../../tests/moon-pod-conformance/`](../../tests/moon-pod-conformance/).

**Two delivery shapes:** software bundle (partner runs `install.sh` on their existing Linux host) or pre-imaged hardware (we ship a small SFF computer pre-flashed with the bundle; per-partnership decision).

**Sovereignty preservation:** partner's data on partner's hardware, partner's operators in partner's IdP, partner's audit in partner's storage — even though we ship the bundle and host the partner-net coordination plane. See [`moon-pod-conformance.md`](https://github.com/usareur-af/goatstuff/blob/main/docs/design/moon-pod-conformance.md) §"Sovereignty floor" for the four edge cases (partner-net coordination, in-flight bridge processing, shipped binaries, pre-imaged hardware).

Tier-0 and the PRA are both conformant against the same contract. Partner can transition between them without re-onboarding (requirement 7).

## The Partner Data Sharing Charter

> **Superseded as the default, not withdrawn.** [ADR 1143](../adr/1143-unilateral-release-declaration-and-accession.md) makes the **published release declaration + accession** the default instrument for releasing unclassified data — one document the owner writes once and any recipient accedes to, in place of an instrument negotiated per pair. Design of record: `release-declaration-and-accession.md`; template: [`../partner-templates/release-declaration-and-accession.md`](https://github.com/usareur-af/goatdocs/blob/main/docs/partner-templates/release-declaration-and-accession.md). The charter below stays for relationships that genuinely need **bilateral** terms — reciprocal obligations, jointly-owned data, negotiated liability. Never both over the same data. Where a partnership uses declarations, step 2 of the onboarding workflow below (charter exchange and signature) is replaced by the recipient acceding to the relevant declaration, which does not block the steps after it.

Formal memo basis for any partnership. Template at [`../partner-templates/data-sharing-charter.md`](https://github.com/usareur-af/goatdocs/blob/main/docs/partner-templates/data-sharing-charter.md). Critical framing: **necessary but not compelling.** Signing the charter:

- Establishes the framework that *enables* data sharing — governance, identity, audit shape, release boundaries, four-facet expectations.
- Does **not** obligate either party to share any specific data.

Without a signed charter, no `release/<partner>/...` allowlist entries are issued. With a signed charter, allowlist entries are *possible* — each requires a per-topic policy-engine decision, gated and audited.

## Onboarding workflow (end-to-end)

1. **Initial conversation** — partner expresses interest; we share the ADR, design doc, PRA, [requirements contract](https://github.com/usareur-af/goatstuff/blob/main/docs/design/moon-pod-conformance.md), [moon-pod spec](https://github.com/usareur-af/goatstuff/blob/main/docs/design/goat-moon-pod.md), charter template. Partner indicates which tier (PRA / Tier-0 moon-pod / vendor moon-pod / partner-built) they intend to operate at.
2. **Charter exchange + signature** — partner reviews charter, fills Appendix A (data type + operational requirement) and Appendix B (technical configuration, including tier declaration), signs. We countersign.
3. **OIDC federation** — partner publishes their IdP discovery URL; we configure the Kratos OIDC connector. Test login. (For Tier-0 moon-pod, the moon-pod's local IdP is the partner's IdP after first-boot enrollment.)
4. **Namespace allocation** — partner gets `release/<partner-slug>/`; we get `release/goat/` reciprocally where applicable.
5. **Bridge configuration** — partner-CIDR allowlist entries, conditioning role applied to bridge VPSes. (For Tier-0, also: partner-net netbird-mgmt setup-key issuance.)
6. **Conformance evaluation** (Tier-0 only) — for moon-pods (ours, vendor's, or partner-built), run [`../../tests/moon-pod-conformance/`](../../tests/moon-pod-conformance/) against the deployed pod. All seven requirements must PASS.
7. **First per-topic allowlist** — partner requests release of topic X to themselves (or vice versa); the request flows through the policy engine for explicit approval.
8. **Smoke test** — actual data flow over the boundary, audit log entries verified on both sides.
9. **Operational** — ongoing per-topic decisions follow the same policy-engine workflow.

## Out of scope

- Cross-domain (CDS) conditioning — separate ADR [`../adr/0504-cds-conditioning.md`](../adr/0504-cds-conditioning.md).
- Prescribing partner architecture — explicitly rejected.
- Symmetric audit federation as a *requirement* — partner-by-partner negotiation.
- Higher-than-unclass release.
- Real-time push-based partner revocation — current revocation is allowlist-removal + trust-revocation; push-based deferred until needed.

## Test plan

- **Smoke test (CI).** Mock partner IdP + mock bridge + signed charter fixture; assert namespace allocation, OIDC federation success, allowlist entry created via policy engine, data flow with audit log entry on both sides.
- **Integration soak.** First-greenfield-partner onboarding (when scheduled) is the real validation; protocol exists for it in the onboarding workflow above.
- **Drill** — quarterly tabletop with simulated partner-side compromise: revoke partner trust, tear down allowlist entries, validate that bridge conditioning blocks subsequent release attempts within minutes.

## References

- ADR [`0821`](../adr/0821-mission-partner-sharing-pattern.md) — anchor decision
- ADR [`0223`](../adr/0223-goat-moon-pod-repo-containment.md) — goat-moon-pod repo-containment + naming
- [`partner-reference-architecture.md`](https://github.com/usareur-af/goatdocs/blob/main/docs/design/partner-reference-architecture.md) — recommended minimal stack (PRA tier)
- [`moon-pod-conformance.md`](https://github.com/usareur-af/goatstuff/blob/main/docs/design/moon-pod-conformance.md) — Tier-0 seven-requirement contract
- [`goat-moon-pod.md`](https://github.com/usareur-af/goatstuff/blob/main/docs/design/goat-moon-pod.md) — abstract spec for moon-pods
- [`goat-moon-pod-builder-guide.md`](https://github.com/usareur-af/goatstuff/blob/main/docs/design/goat-moon-pod-builder-guide.md) — for vendors building their own moon-pods
- [`../partner-templates/data-sharing-charter.md`](https://github.com/usareur-af/goatdocs/blob/main/docs/partner-templates/data-sharing-charter.md) — formal memo template
- [`../vendor-templates/data-services-provider-charter.md`](https://github.com/usareur-af/goatdocs/blob/main/docs/vendor-templates/data-services-provider-charter.md) — vendor counterpart
- ADR [`0411`](../adr/0411-ory-idp-mvp.md) — Ory IdP MVP
- ADR [`0805`](../adr/0805-policy-engine-active-passive.md) — policy engine
- ADR [`0806`](../adr/0806-egress-proxy-cascade.md) — egress proxy (shape mirrored at boundary)
- ADR [`0811`](../adr/0811-audit-log-integrity.md) — audit-log integrity
- ADR `0819` — existing TAK federation (rebases under this pattern)
- ADR [`0309`](../adr/0309-bridge-cidr-allowlist.md) — bridge CIDR allowlist
- `zenoh-data-fabric.md` — keyexpr conventions
- [`data-dictionary.md`](data-dictionary.md) — schema declaration applies at boundary
- `../integrations/moon-pods/` — per-vendor moon-pod cookbooks
- [`../../tests/moon-pod-conformance/`](../../tests/moon-pod-conformance/) — Tier-0 conformance smoke-test suite
- `../project/implementation-plan.md` Block 24, Block 60
