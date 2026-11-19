# ADR 1020 — Zenoh federated peering trust model: canonical goat↔goat + third-party quarantine

**Status:** Proposed
**Date:** 2026-06-01
**Repo scope:** goat core
**Amended by:** [ADR 1251](1251-every-source-keeps-its-identity-through-a-pod-carriage-rules-provenance-at-entry-and-pod-local-authority.md) — a pod's session may write the slots its carriage rules cover
**Related:** [0844](0844-zenoh-deployment-topology-and-acl-v1.md), [0216](0216-goat-net-federation-and-site-planes.md), [0821](0821-mission-partner-sharing-pattern.md), [0811](0811-audit-log-integrity.md); design doc [`zenoh-federated-peering-trust.md`](../design/zenoh-federated-peering-trust.md)

## TL;DR (for a layperson)

When another organization's data-router connects to ours and relays its feeds in, our system trusts the *connection*, not the *original sender* — so that data arrives with no verified identity, no agreed format, nowhere to be stored, and free to use any name it likes. This ADR says: the **blessed** way to connect routers is *ours to ours*, where everything is already trusted end-to-end; and any **outside** connection is **quarantined** — accepted but penned into a clearly-labeled area named after who relayed it — rather than thrown away or let loose.

## Context

[ADR 0844](0844-zenoh-deployment-topology-and-acl-v1.md) established router-to-router federation over the NetBird mesh, URI-SAN cert identity, and a default-deny + per-prefix ACL. But the v1 trial fabric authorizes the **directly-connected session**, not the **origin publisher**, and a universal-read rule that also grants `put` on `**` for `ingress` to every CA-rooted peer defeats per-vendor write scoping. A peered router therefore acts as a transitive trust bridge: anything it forwards enters under its authorization, under any namespace, with the origin's identity lost at the hop.

Relayed traffic can therefore enter under a namespace with no ACL subject, no storage, and no schema — unattributable, unretained, unrecoverable. The fabric has no concept of "un-onboarded peered ingress," so such data is neither properly authorized nor deliberately handled.

We need an explicit trust posture for federated ingress before partner peering (Block 84) and multi-site goat federation scale up.

## Decision

**1. Canonical peering is goat↔goat.** The first-class, blessed peering is our routers federating with our routers (the ADR 0844 model). Both ends share one trust root, identity grammar, ACL, schema registry, and audit trail; native keys keep their native namespace across the federation with no rewrite. Authorization, attribution, decode, and retention hold end-to-end.

**2. Tier-1 depends on all schemas living in Zenoh.** Goat↔goat auto-propagation requires schemas to ride `attest/schema/**` (subscribable across federation), not a single portal instance. The [`portal-schema-publishing.md`](../design/portal-schema-publishing.md) §Long-term migration (portal replays descriptors into Zenoh; vendors then publish their own with their cert) is a **named prerequisite** of Tier 1.

**3. Third-party peering is quarantined, not discarded.** Any ingress from an entity not onboarded end-to-end is accepted and **rewritten at the boundary** to `<peering-entity>/<peering-id>/<original-namespace…>`, where `<peering-entity>` is the *verified* identity of the directly-connected peer, `<peering-id>` is a stable per-relationship identifier, and the original key is preserved verbatim. Provenance is in the key path; the data is isolated, retained in its own bounded storage, audited against the verified peering identity, and honestly labeled as un-onboarded.

**4. Third-party ingress goes through a goat ingress gateway, not direct core-router federation.** Because Zenoh has no native ingress key-rewrite, a goat-operated gateway authenticates the third party, assigns the `<peering-id>`, and re-publishes its traffic into the core fabric under the quarantine prefix using the gateway's own identity scoped to that prefix. Core routers only ever see goat-CA-rooted, origin-scoped writers. The gateway composes with the partner-peering-portal (Block 84) as its registration front end.

**5. Fix the ingress-write over-grant.** Split the ACL so the universal rule grants `put`/`delete` on `**` for `egress` only (subscriber delivery); `ingress` writes are granted solely by per-vendor scoped rules plus the gateway's quarantine-prefix rule. After the split, un-onboarded prefixes arriving directly at a core router are denied by default-deny.

## Alternatives considered

- **Discard un-onboarded peered ingress.** Rejected: loses signal we may want, and the loss is silent and unrecoverable. Quarantine preserves it under explicit provenance.
- **Admit it raw under its claimed namespace.** This is the status quo — the bug. Allows namespace squatting, breaks identity-bound audit ([ADR 0811](0811-audit-log-integrity.md)), and leaves data unretained/undecodable.
- **Let third parties peer directly into core routers with a per-partner ACL subject.** Insufficient: ACL authorizes the link, not relayed origins, so a partner router still bridges arbitrary upstreams in. The gateway is what makes rewrite + origin-scoping possible.
- **Trust key-prefix as identity for attribution.** Rejected: an unverified, spoofable string; the only authenticatable identity across a peering hop is the directly-connected peer's cert.

## Consequences

- **Positive:** First-class namespaces are protected by default-deny; un-onboarded ingress is preserved with verified provenance, bounded retention, and honest UX; the active universal-ingress-write hole is closed; partner peering gets a clear, auditable contract before it scales.
- **Cost / new surface:** A new ingress-gateway component to build and operate (rate-limit chokepoint, peering-id registry, provenance metadata). The Tier-1 schema-in-Zenoh migration must land for goat↔goat auto-propagation. The ACL split needs live verification against Zenoh 1.9.0 `egress`/`ingress` semantics.
- **Migration:** Existing onboarded vendors are unaffected (they already have scoped subjects + storage + schema). The ACL `put`-split is the one change touching the running router config and must be verified by a cross-prefix-`put` test before rollout.
- **Open decisions** (tracked in the design doc): peering-id reservation convention to avoid colliding with an onboarded `<entity>/**`; quarantine retention window; gateway trust domain (own X.509 / token / portal-minted cert); whether direct third-party federation is ever permitted.

This ADR stays **Proposed** until the design doc's build plan is picked up; the first landing item is the ACL `egress`/`ingress` `put` split.

## Implementation note (2026-08-27)

Decision **5** (the ACL `put`-split) landed on a running production router config. Decision **1** (canonical goat↔goat) is being **built** for the same-goatnet, two-site case — a goatnet's founder site ↔ its joiner site (e.g. zagros alpha ↔ zagros beta). It ships in the `goat-zenoh` role gated on `goat_zenoh_connect_endpoints`: an outbound `connect:` block, an outbound-dial client cert, and a `router-peer` ACL subject whose federation ingress is **scoped to `ops/**` + `attest/schema/**`** (never `**`, preserving decision 5). Decisions **3–4** (third-party quarantine + the ingress gateway) stay **Proposed** — not exercised by same-goatnet peering, where both ends share one CA, ACL, and audit trail. Design + resume state: [`zenoh-federated-peering-trust.md`](../design/zenoh-federated-peering-trust.md).
