# ADR 0821 — Mission partner sharing — `release/` namespace + bring-your-own-OIDC + four-facet expectations

> **Read with [ADR 1208](1208-sovereign-environments-release-namespaces-and-grant-encryption.md)**, which replaced the single goat-run fabric this ADR describes with sovereign environments that release across federated boundary nodes (default instrument: [ADR 1143](1143-unilateral-release-declaration-and-accession.md)), **and with [ADR 1239](1239-a-moon-pod-is-a-learning-tool-and-not-a-sovereignty-guarantee.md)**, which states what a moon pod is.


| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-04-26 |
| Repo scope | goat core |
| Supersedes | — |
| Superseded by | — |
| Related | [`0203-bridge-vpses-separate-from-relays.md`](0203-bridge-vpses-separate-from-relays.md), [`0309-bridge-cidr-allowlist.md`](0309-bridge-cidr-allowlist.md), [`0405-bridge-access-split.md`](0405-bridge-access-split.md), [`0411-ory-idp-mvp.md`](0411-ory-idp-mvp.md), [`0500-zenoh-data-fabric.md`](0500-zenoh-data-fabric.md), [`0504-cds-conditioning.md`](0504-cds-conditioning.md), [`0511-publisher-trust-model.md`](0511-publisher-trust-model.md), [`0512-rapid-adaptation-with-ai.md`](0512-rapid-adaptation-with-ai.md), [`0805-policy-engine-active-passive.md`](0805-policy-engine-active-passive.md), [`0806-egress-proxy-cascade.md`](0806-egress-proxy-cascade.md), [`0811-audit-log-integrity.md`](0811-audit-log-integrity.md), `0819-tak-federation.md` |

## TL;DR (for a layperson)

Sharing data with mission partners at unclassified needs a deliberate, secure pattern — not a string of partner-specific bridges that each got built differently. We reserve a `release/<partner>/...` namespace on the fabric with a default-deny baseline. Partners authenticate with **whatever standards-compliant OIDC IdP they already run** — we don't prescribe their stack. Four-facet expectations apply to any data that flows: **security, functionality, performance, reciprocity**. We don't *prescribe* how partners meet those, but we offer a **Partner Reference Architecture (PRA)** — a minimal Zenoh-on-ZTOL recipe — that partners can take or leave; many will take our lead. A separately-issued **Partner Data Sharing Charter** (template at `docs/partner-templates/data-sharing-charter.md`) provides the formal memo basis: a framework that is *necessary* to enable sharing but does *not* compel either side to share specific data.

## Maturity caveat

GOAT-Net (the project's externally-facing name for the data backbone goat realizes, used in this ADR's partner-facing context) is at proof-of-concept stage. We are in uncharted territory — sovereign tactical-edge data backbones at this shape do not have established operational precedent at theater scale. The direction is sound and anchored in mainstream distributed-systems patterns; specific scaling, performance, and operational characteristics will be discovered, not asserted. A viable scaling plan exists (ADR [`0216-goat-net-federation-and-site-planes.md`](0216-goat-net-federation-and-site-planes.md)) and becomes operative *if* GOAT-Net earns it through the early phases. Partners signing the Partner Data Sharing Charter are entering an early-phase relationship — the discipline is sound, but specific operational behaviors will be measured rather than promised.

## Context

A bridge built for one partner, such as a TAK federation bridge to a partner's federation hub (`0819-tak-federation.md`), is partner-specific. Without a generalized pattern, the second partner produces a second bespoke bridge, the third a third — accumulated cruft with no shared release-trust story, no shared audit symmetry, no shared identity-federation discipline. The same kind of cruft the phoenix-native peer discipline ([`0208`](0208-phoenix-native-peer-discipline.md)) was authored to avoid.

What is **already in this repo** the pattern can build on: bridge VPSes ([`0203`](0203-bridge-vpses-separate-from-relays.md)), partner-CIDR allowlist + default-deny ([`0309`](0309-bridge-cidr-allowlist.md)), bridge access split ([`0405`](0405-bridge-access-split.md)), Ory IdP MVP ([`0411`](0411-ory-idp-mvp.md)) for our side of OIDC, per-site policy engine ([`0805`](0805-policy-engine-active-passive.md)), egress-proxy shape ([`0806`](0806-egress-proxy-cascade.md)) the partner boundary should mirror, audit-log integrity ([`0811`](0811-audit-log-integrity.md)), CDS conditioning ([`0504`](0504-cds-conditioning.md)) for the cross-classification case the unclass pattern composes with.

What is **missing**:

1. A reserved key-expression namespace for partner-shareable data with a default-deny baseline.
2. A documented **four-facet expectations set** (security + functionality + performance + reciprocity) that does not prescribe partner architecture.
3. A documented **Partner Reference Architecture** — a minimal Zenoh-on-ZTOL recipe that partners can adopt without inheriting our active-active multi-site complexity.
4. A bridge-side data-conditioning checkpoint that mirrors the egress-proxy shape.
5. A formal **Partner Data Sharing Charter** template that establishes the framework for any partnership without obligating data flow on either side.

This ADR codifies all five.

## Decision

goat ships a generalized **mission partner sharing pattern** for unclassified data, in five parts.

### 1. Reserved key-expression prefix `release/`

Add `release/` as a reserved top-level prefix in [`0500-zenoh-data-fabric.md`](0500-zenoh-data-fabric.md) §"Top-level key-expression prefixes":

| Prefix | Purpose | Example |
|---|---|---|
| `release/` | Data deliberately released to a named mission partner. Default-deny baseline; every entry under `release/<partner>/<topic>` is a policy-gated, audit-logged release decision. | `release/partner-a/tak/cot/<event-id>`, `release/<partner>/sensor/track/<id>` |

The convention stops at the second segment — `release/<partner>/` — to give partner-specific shape freedom while keeping the release boundary a clean grep target. Reciprocity uses the **same** convention from the partner direction: a partner publishes their releasable topics under `release/<partner>/...` and we may subscribe.

### 2. Bring-your-own-OIDC partner identity federation

**Partners may use any standards-compliant OIDC IdP.** We do not prescribe Ory, Kanidm, Dex, Zitadel, Keycloak, or any other specific stack on the partner side. The integration surface is OIDC discovery + standards-compliant token issuance.

We provide working integration examples for Dex, Kanidm, Zitadel, Keycloak, and Ory itself. These examples are **not a list of approved IdPs** — any partner running a standards-compliant OIDC IdP whose discovery URL we can reach can federate. The examples exist so a partner integrator has a working starting point regardless of their stack.

Our side stays Ory Kratos with the OIDC connector pointing at the partner's discovery URL. Partner-issued identity assertions are accepted within the partner's `release/<partner>/` namespace scope — a partner-issued identity has no authority outside its partner's scope, by construction.

### 3. Four-facet expectations (security + functionality + performance + reciprocity)

Expectations apply to any data that flows under this pattern. **Expectations, not prescriptions** — we describe the bar the partnership commits to; partners decide how to meet it.

#### 3a. Security

- **Encryption in transit.** TLS 1.3 minimum on every partner-side hop touching `release/<partner>/...` data. No plaintext.
- **Identity assurance.** Partner-side end-user identity equivalent to NIST SP 800-63 IAL2 / AAL2 or above, federated via OIDC.
- **Audit retention.** 90 days minimum on partner-side audit logs covering accesses to `release/<partner>/...` data.
- **Incident notification SLA.** Partner-side incidents involving shared data — notification within an agreed window (default: 24h for confirmed, 72h for suspected).

#### 3b. Functionality

- **OIDC-compliant IdP** with discovery endpoint we can reach.
- **Zenoh wire protocol** routability — partner peer can speak Zenoh natively, or shim through a vendor protocol per the shim pattern ([`0209-shim-pattern.md`](0209-shim-pattern.md)).
- **Partner identity assertion** at the release-namespace boundary — every release event is bound to a partner-side identity, not just a network address.
- **Optional inbound subscription capability** for our outbound topics — the partner may subscribe to topics we expose under `release/zagros/...`.

#### 3c. Performance

Calibrated to **commercial transport-based internet** realities, *not* our internal SLOs. The partner pattern explicitly assumes commercial internet — no MPLS, no fiat-government wires, no SCS expected.

- Round-trip latency to the mutual relay: **typical < 500ms**, episodic spikes acceptable.
- Sustainable throughput: **typed per partnership** (declared in the Partner Data Sharing Charter Appendix A — see template); not absolute.
- Monthly availability: **≥ 99%**, calibrated to commercial-internet realism.

These are **expectations, not SLAs.** Hard SLAs require a contractual basis the charter does not provide; the expectations exist so both sides know what "working" looks like.

#### 3d. Reciprocity

The release-namespace pattern is bilateral. If we expose `release/zagros/<topic>/...` to partner X, partner X *may* expose `release/<partner>/<topic>/...` to us under the **same convention**:

- Same default-deny baseline at their boundary.
- Same identity-bound, policy-gated, audit-logged release discipline.
- Same four-facet expectations applied symmetrically.

We do not **require** reciprocity (some partnerships are intrinsically one-directional — e.g., outbound TAK to a partner's federation hub) but the pattern supports it cleanly. Where reciprocity exists, the charter covers both sides under one document.

### 4. Bridge data-conditioning checkpoint (mirrors egress proxy)

The bridge VPS data path for `release/<partner>/...` runs through a **default-deny data-conditioning checkpoint** that mirrors the egress proxy ([`0806`](0806-egress-proxy-cascade.md)):

- **Default-deny baseline.** No `release/<partner>/<topic>` crosses until an explicit per-partner per-topic allowlist entry exists.
- **Per-partner per-topic allowlist.** Allowlist entries are policy-engine-gated ([`0805`](0805-policy-engine-active-passive.md)). "Releasable to partner X" is a deliberate, auditable decision, not an emergent property of network configuration.
- **Identity-bound.** Every release event is recorded against the partner identity that received it; partner-side identity assertions are validated against the partner's federated IdP at release time.
- **Audit-logged.** Every release event lands in the audit-log integrity tier ([`0811`](0811-audit-log-integrity.md)) with the same immutability + retention guarantees as internal egress.

The shape mirroring is deliberate: when CDS ([`0504`](0504-cds-conditioning.md)) eventually lifts the pattern across classifications, the operational shape transfers — only the conditioning logic at the boundary changes.

### 5. Partner Reference Architecture (recommended, not required)

We publish a **Partner Reference Architecture** at [`../design/partner-reference-architecture.md`](https://github.com/usareur-af/goatdocs/blob/main/docs/design/partner-reference-architecture.md) — a minimal Zenoh-on-ZTOL stack a partner can stand up in days, deliberately stripped of every complication that exists for *our* reasons (active-active, multi-site, island-mode-by-construction, paired pgBackRest, three-tier NTP, phoenix-native rebuild discipline). Sketch:

1. One Zenoh router at the partner side. Single node. No replication.
2. WireGuard ZTOL over commercial internet.
3. One OIDC IdP (any standards-compliant).
4. TLS 1.3 on non-WireGuard hops.
5. `release/<partner>/...` namespace discipline, mirroring ours.
6. One audit sink (90d-capable).
7. Health endpoint at `health/partner/<partner>/...`.
8. Optional phoenix-native discipline.

The PRA is a **recommendation**. Partners running a more elaborate stack adopt as little or as much as they want; partners with no existing fabric infrastructure get a working starting point. We commit to maintaining the PRA as our complexity evolves.

### 6. Partner Data Sharing Charter (formal memo basis)

We publish a **Partner Data Sharing Charter** template at [`../partner-templates/data-sharing-charter.md`](https://github.com/usareur-af/goatdocs/blob/main/docs/partner-templates/data-sharing-charter.md). The charter is the formal artifact a partnership signs. Critical framing: **necessary but not compelling.** Signing the charter:

- Establishes the framework that *enables* data sharing — governance, identity, audit shape, release boundaries, four-facet expectations, incident SLA, termination clause.
- Does **not** obligate either party to share any specific data. Specific data flows remain bilateral, case-by-case decisions.

Without a signed charter, no `release/<partner>/...` allowlist entries are issued. With a signed charter, allowlist entries are *possible* — each requires a per-topic policy-engine decision, gated and audited like everything else. The charter unlocks the shape; data discretion stays absolute.

The template carries placeholder sections (Appendix A: data type & operational requirement; Appendix B: per-partner technical configuration) so each partnership instantiates the framework with concrete content while preserving the framework's bilateral-non-obligating character.

## Alternatives considered

- **Prescribe a specific partner IdP / stack.** Partners will not adopt non-sovereign or migration-burdening stacks just to federate. Rejected.
- **One-bridge-per-partner with per-partner code paths.** Doesn't scale; produces partner-specific cruft. Rejected.
- **Prescribe partner network architecture.** Architecture-shaped relationships rather than contract-shaped. Brittle, inhibits partnership formation. Rejected.
- **No release namespace; route partner topics through bridge ACLs by IP.** Doesn't compose with audit; doesn't survive partner-IP changes. Rejected.
- **Push our active-active multi-site discipline on partners.** Inappropriate — our complexity exists for *our* reasons (island mode, scope envelope, sovereignty). Rejected; replaced by the PRA recommendation.
- **Mandatory reciprocity.** Some partnerships are intrinsically one-directional. Pattern supports reciprocity cleanly but does not require it. Rejected as mandate.
- **Strict performance SLAs.** Commercial internet doesn't support them; pretending otherwise produces unenforceable agreements. Rejected; replaced by expectations + per-partnership typed targets in the charter.
- **Use a vendor data-sharing platform.** Sovereignty + island-mode incompatible. Rejected for the secure plane.
- **Wait for CDS to be in place and run partner sharing through it.** CDS lifts to higher classifications; not the right shape for unclass-to-unclass. The two patterns coexist. Rejected as substitute.
- **Charter that obligates data sharing.** Wrong contract shape. Both parties retain absolute discretion over what flows. Rejected; replaced by "necessary but not compelling" framing.

## Consequences

**Positive.**

- Generalized release pattern. Second and subsequent partners reuse the convention.
- Partner architecture autonomy. BYOIDC respects partner sovereignty.
- PRA gives partners a working starting point without inheriting our complexity.
- Charter establishes the framework formally without obligating data flow — partnerships can form before specific datasets are decided.
- Audit shape mirrors internal egress; transfers cleanly when CDS is layered on.
- A TAK federation bridge fits cleanly under `release/<partner>/tak/...`.
- Reciprocity is supported, not required — covers both bilateral and one-directional cases.
- Performance expectations calibrated to commercial internet — honest, achievable.

**Negative.**

- Partner-IdP heterogeneity is real maintenance burden. Mitigation: bounded example set; standards compliance is the contract.
- Attestation-based posture trusts partner attestations to be honest. Mitigation: standard coalition partnership shape; alternative does not scale.
- Default-deny baseline at the bridge requires each new partner-topic combination to flow through the policy engine. Operationally heavier; the friction is the feature.
- PRA maintenance is ongoing. Mitigation: PRA changes only when our complexity changes in ways that affect partner interop.
- Charter template needs legal review when first instantiated. Mitigation: template is the engineering contribution; legal sign-off is per-partnership.

**Out of scope for this ADR.**

- Cross-domain (CDS) conditioning — separate ADR ([`0504`](0504-cds-conditioning.md)).
- Prescribing partner architecture — explicitly rejected.
- Symmetric audit federation as a *requirement* — partner-by-partner negotiation determines whether it happens.
- Higher-than-unclassified release.
- Real-time partner-revocation primitives — today's revocation is allowlist-removal + trust-revocation; push-based revocation is a follow-on once a partner needs it.
- Any specific signed partnership — the ADR provides the pattern + template, not the agreements.

## Assessor view (SCA-R)

This decision serves AC-3 (access enforcement), AC-4 (information flow enforcement), AC-6 (least privilege), AU-2/AU-12 (audit), and IA-8 (identification and authentication, non-organizational users). Making "data released to mission partner X" a policy-gated, identity-bound, audit-logged decision per topic — not a network-configuration emergent property — is the load-bearing posture.

The bring-your-own-OIDC stance aligns with NIST SP 800-63-3 federated-identity guidance. Standards compliance (OIDC, TLS 1.3) is the bar; specific stacks are not.

The Partner Data Sharing Charter formalizes the partnership shape as a contract artifact. Its "necessary but not compelling" character is deliberate — it provides the framework needed for any data flow without committing either side to flows not yet decided. Per-flow decisions remain auditable through the policy-engine workflow.

The PRA is an engineering recommendation, not an accreditation requirement. Partners who adopt it inherit conventions that compose cleanly with our boundary; partners who don't still meet the four-facet expectations through their own architecture, attested in their charter signature.

The release pattern composes with the CDS pattern ([`0504`](0504-cds-conditioning.md)) without replacing it. Cross-classification release continues to require CDS conditioning; this ADR addresses unclass-to-unclass mission-partner release only.

## Trigger conditions for revisit

- **First partner that cannot meet a four-facet expectation.** Revisit calibration or scope (observer-only, restricted topic set).
- **Symmetric audit federation becomes a partner requirement.** Formalize the symmetric-audit shape.
- **Real-time partner-revocation primitive becomes load-bearing.** Design push-based revocation.
- **More than ~5 partners.** Revisit convention for collisions, audit volume, IdP-trust drift.
- **CDS conditioning lands.** Revisit composition.
- **Partner-side compromise.** Revisit IdP-trust-revocation primitives + allowlist tear-down speed.
- **Performance expectations breach pattern.** If commercial internet stops being viable, revisit transport assumptions in the PRA.
- **Charter instantiation surfaces missing clauses.** Revise template; do not patch per-partnership.

---

*See `GLOSSARY.md` for terms and acronyms.*
