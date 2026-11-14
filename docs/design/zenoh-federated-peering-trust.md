# Zenoh federated peering — trust model

**Status: goat↔goat intra-goatnet slice BUILDING (2026-08-27); third-party quarantine still designed-not-built.** Decision recorded in [ADR 1020](../adr/1020-zenoh-federated-peering-trust-model.md); this doc is the canonical design. Builds directly on the federation + ACL baseline in [ADR 0844](../adr/0844-zenoh-deployment-topology-and-acl-v1.md).

> **Build note (S0).** The canonical goat↔goat peering (ADR 1020 decision 1) is being implemented for the same-goatnet, two-site case. Mechanism in the `goat-zenoh` role, gated on `goat_zenoh_connect_endpoints` (empty = single-router deploy byte-unchanged): an outbound `connect:` block + outbound-dial client cert, plus a `router-peer` ACL subject (`cert_common_names: ["goat-zenoh-router"]`) granting read + delivery + a **scoped** federation ingress (`router-peer-w-federated`: `put`/`delete` on `ops/**` + `attest/schema/**` only — never `**`, so decision 5's ingress split holds). Decision 5 (the ACL put-split) already landed 2026-06-01. Decisions 3–4 (third-party quarantine + ingress gateway) remain Proposed — not needed here because both routers are the same goatnet, one CA, one ACL.

## The problem in one sentence

When data reaches our Zenoh fabric over a **router-to-router peering** rather than from a directly-onboarded publisher, the entire onboarding contract (per-vendor cert identity, write-scope ACL, storage registration, schema declaration, audit attribution) is bypassed at the session boundary — so the data is unauthenticated at the publisher level, unattributable, unretained, and undecodable, and it can squat any namespace.

## Why the current model lets this happen

Two properties of the v1 trial fabric combine (both verified against the live nova-a `config.json5` 2026-05-31):

1. **Authorization is link-scoped, not origin-scoped.** Our router authenticates the *directly-connected* peer's mTLS cert. When that peer is itself a router relaying other publishers, the forwarded samples present *the relay's* cert to us — the origin publisher's identity never crosses the hop. Zenoh carries no per-sample origin identity across a federation link. So a peered router is a transitive trust bridge: everything it forwards rides its authorization.

2. **Per-vendor write scoping is not actually enforced.** The ACL subject `trial-peer-read` is defined by `link_protocols: ["tls"]` alone (any CA-rooted peer) and is bound to a rule allowing `put`/`delete` on `key_exprs: ["**"]` with `flows: ["egress","ingress"]`. ACL rules are additive allows over default-deny, so this single rule grants **every CA-rooted peer ingress-write on every key**. The per-vendor `trial-peer-w-<vendor> → <vendor>/**` rules add nothing restrictive. (The `put` was added to make subscriber *delivery* — egress — work; `flows:[egress,ingress]` also granted ingress-write as a side effect.)

The net effective policy today is "hold any fabric-CA-rooted cert → read everything + write any prefix." Onboarding's per-vendor scoping is cosmetic for ingress.

## The model: two tiers of peering

### Tier 1 — canonical: goat ↔ goat

The blessed, first-class peering is **our routers federating with our routers** (the inter-site router-to-router federation of [ADR 0844](../adr/0844-zenoh-deployment-topology-and-acl-v1.md), riding the NetBird mesh). Both ends share one trust root, one identity grammar (the URI-SAN `urn:goat:peer:site=…,role=…,echelon=…`), one ACL stack, one schema registry, one audit trail. Because the substrate is symmetric, native keys stay in their native namespace across the federation — no rewrite — and authorization, attribution, decode, and retention all hold end-to-end with no special handling.

**This tier works only if all schemas live in Zenoh.** See the prerequisite below — today they do not, for real vendors.

### Tier 2 — everything else: quarantine, do not discard

A peering from any entity that is **not** part of our trust domain (a partner router, a vendor's own aggregator relaying third parties, anything we did not onboard end-to-end) is **accepted but quarantined**, never admitted to a first-class namespace and never silently dropped. Discarding it loses signal we may want; admitting it raw is the bug above. Quarantine makes the provenance explicit and the data isolated, retained, and inspectable on our terms.

#### Quarantine namespace shape

Un-onboarded peered ingress is **rewritten at the boundary** to:

```
<peering-entity>/<peering-id>/<original-namespace…>
```

- **`<peering-entity>`** — the *verified* identity of the directly-connected peer that did the peering (its cert URI-SAN / CN). This is the one identity we can actually authenticate.
- **`<peering-id>`** — a stable identifier for the specific peering relationship/session, so multiple peerings from the same entity are distinguishable and every quarantined sample is traceable to one peering event in the audit log.
- **`<original-namespace…>`** — the key exactly as the peer presented it, preserved verbatim (e.g. `example-vendor/entity/track/v1`).

Worked example — a relayed third-party feed, quarantined:

```
org-1/peering-7f3a/example-vendor/entity/track/v1
```

**Collision note.** `<peering-entity>` may itself be an onboarded vendor with a first-class `<entity>/**` namespace (e.g. `org-1/**`). To keep relayed third-party traffic from colliding with the entity's own onboarded topics, the `<peering-id>` segment must be drawn from a reserved space that cannot be confused with the entity's real topic systems (e.g. a `peering-` prefix on the id, or an explicit reserved infix such as `<entity>/_peered/<peering-id>/…`). The exact reservation convention is an open decision below.

#### Mechanism: an ingress gateway, not direct federation

Zenoh has no native key-rewrite-on-ingress. The clean implementation is a **goat-operated ingress gateway** — a dedicated process holding our own CA identity — rather than letting third parties peer directly into a core router:

1. The third party connects to the gateway (its own trust domain / cert), not to the core router.
2. The gateway assigns and records a `<peering-id>` for the session.
3. The gateway subscribes to what the third party publishes and **re-publishes** each sample under `<peering-entity>/<peering-id>/<original-key>` into the core fabric, using the gateway's own identity scoped to write *only* the quarantine prefix.
4. The gateway emits provenance metadata per sample: verified peering identity, original key, receive + claimed-publish timestamps.

This keeps the **core router's ACL simple** — it only ever sees goat-CA-rooted writers (the gateway among them), each origin-scoped. No third party ever writes a first-class namespace, and the quarantine boundary is a real component we control and can rate-limit, not an emergent property of federation. It composes with the partner-peering-portal (Block 84) as the onboarding/registration front end for the gateway.

## Required ACL change (applies to both tiers)

Fix the ingress-write over-grant so authorization is origin-scoped by default-deny:

- The universal rule keeps `put`/`delete` on `**` for **`egress` only** (subscriber delivery — the original, legitimate need).
- **Ingress** `put`/`delete` is granted *solely* by per-vendor scoped rules (`<vendor>/**`, bound to the vendor's cert subject) plus the gateway's quarantine-prefix rule.

After this split, an un-onboarded prefix arriving directly at a core router is denied at ingress by default-deny — closing both the namespace-squatting surface and the accidental-universal-write. Verify against Zenoh 1.9.0 `egress`/`ingress` semantics with a live cross-prefix `put` test before landing.

## Prerequisite: all schemas in Zenoh

Tier 1's "it automatically clicks" depends on schemas being **on the fabric**, so a peered goat router receives them by subscribing to `attest/schema/**` rather than reaching a single portal instance.

**Today this is only half true** (verified 2026-05-31): synth-vendors publish schema declarations to `attest/schema/<vendor>/<system>/v1` every 30s (rocksdb-backed, 7-day TTL), but **real / portal-minted vendors do not** — their schemas live on the portal's local disk + SQLite, fetched via `GET /api/schemas/…`. The live `attest/schema/**` store holds essentially only `notional`. The v2 migration in [`portal-schema-publishing.md`](portal-schema-publishing.md) §"Long-term evolution" (portal walks its `schemas/` dir and `zenoh put`s each descriptor to the canonical key; vendors thereafter publish their own with their cert) is the path to closing this, and is a **named dependency of Tier 1**.

## Shape of issues this model addresses

| Issue | Raw un-onboarded ingress | Under this model |
|---|---|---|
| Authorization | Link-scoped; relay is a transitive trust bridge | Origin-scoped writes (ACL fix); third parties only via gateway, scoped to quarantine prefix |
| Attribution | Origin identity lost across hop; audit falls back to spoofable key-prefix string | Audit binds the *verified* peering identity + records claimed original namespace |
| Retention | No matching `<prefix>/**` storage → ephemeral, unrecoverable | Quarantine prefix gets its own bounded storage |
| Decode | No schema → opaque bytes, DECODER OFF, `vendor=None` | Honestly labeled "received via peering, not onboarded"; schema absence is expected, not a failure |
| Namespace integrity | Can squat any vendor's or `notional/**` prefix | Confined to `<entity>/<peering-id>/**`; first-class namespaces protected by default-deny |
| Volume | No rate/scope contract (13k-singleton floods) | Gateway is a rate-limit + scope chokepoint |
| Visibility | Appears/vanishes with no expected-vs-actual | Provenance is in the key path itself; surfaces (panoscope) show the peering origin |

## What this deliberately does not cover

- **Cross-trust-domain command authority.** Quarantined ingress is data only; it never grants `cmd/`/`task/` authority (ADR 0844's authority model is unchanged).
- **Promoting quarantined data to first-class.** If a relayed source should become a real vendor, it goes through normal onboarding (cert + ACL subject + storage + schema). Quarantine is a holding pattern, not a back door.
- **The portal-schema → Zenoh migration itself.** That is its own work item ([`portal-schema-publishing.md`](portal-schema-publishing.md) §Long-term); this doc only names it as a Tier-1 dependency.

## Build plan (when picked up)

1. **✅ file-prepped (pending live nova-a test) — ACL `egress`/`ingress` `put` split.** In `ops/ansible/roles/goat-zenoh/templates/config.json5.j2`: `trial-peer-r-all` is now read-only (`declare_subscriber`/`query`/`declare_queryable`/`reply`, both flows); a new `trial-peer-deliver-all` keeps `put`/`delete` on `**` for **egress only** (subscriber delivery); `trial-peer-w-attest-schema`, `operator-w-system`, and the per-vendor `trial-peer-w-<vendor>` rules are now **ingress-only**, scoped to their prefixes — so ingress write (publish) is granted *solely* by scoped rules. Policies bind `trial-peer-read` to read + deliver, never ingress write. `add-vendor.sh` injects the same ingress-only shape. Rendered template verified brace/bracket-balanced. **STILL REQUIRED before "done": operator-run ansible apply + a live cross-prefix-`put` confirmation test on nova-a** (assert an un-onboarded-prefix `put` is denied at ingress and that subscriber delivery still works). Landed on the `goat-moon-pod-mvp` track. The runbook for that operator test is `docs/operations/nova-a-acl-ingress-egress-split-test.md`.

   **✅ nova-lan parity (2026-06-02, `zenoh-acl-ingress-write-enforcement` track).** The sibling `nova-lan-zenoh` role still carried the identical unsplit catch-all (it was the only other instance — verified repo-wide). The same split is now applied to `ops/ansible/roles/nova-lan-zenoh/templates/config.json5.j2` (incl. the Tier-L anonymous publish rule, now `ingress`-only) and `ops/ansible/roles/nova-lan-zenoh/files/add-vendor.sh`. `delete-vendor.sh` regexes are structure-matched and unchanged. The nova-lan split inherits the same Zenoh 1.9.0 `egress`/`ingress` semantics the nova-a live test confirms — no separate nova-lan live-fire is required beyond that semantics confirmation.
2. Add the expected-vendor manifest + an alert when traffic appears under a prefix with no onboarding record (no ACL subject, no storage, no schema). **This is the recommended next operator-side build step per [ADR 1022](../adr/1022-zenoh-denial-observability.md)** (denial observability): it catches *accepted* un-onboarded traffic without touching the just-shipped enforcement. (The complementary *denied-write* signal — the silent-denial gap of F-177 — is covered publisher-side today by `goat pub --confirm`, and operator-side later by the DLQ `policy_deny` path once the DLQ substrate exists.)
3. **🟡 in progress — ingress gateway** (namespace rewrite, peering-id assignment, provenance metadata, quarantine-prefix storage) — composing with the partner-peering-portal (Block 84). v0 crate landed at `ops/zenoh-ingress-gateway/` (quarantine + passthrough admission, provenance NDJSON, 5 unit tests). TODO: derive peering identity from the live peer cert, quarantine-prefix storage + retention, rate limiting, deployment wiring.
4. Close the Tier-1 schema-in-Zenoh prerequisite (portal-schema v2 migration).

## Reconciliation with the uuid write-anchor ([ADR 1025](../adr/1025-zenoh-dynamic-tenant-authz-slot-pool.md))

This doc was written for the old `<vendor>/**` namespace. ADR 1025 moves writers to a pre-cut, identity-bound **`<uuid>/**` slot anchor** (`uuid = H(salt, k)`). That changes federation in three ways, staged sandbox-flexible → operational-strict:

- **Internal goat↔goat — strict router parity.** Peering between goat's own routers requires a **parity contract**: identical namespace grammar + ACL scaffold (slot-pool shape) + schema/contract registry + goat trust root + pinned zenoh version, gated by a **pre-peering parity check**. Crucially, the slot **salt is per-goatnet** — identical *within* a goatnet (so its sites share one pool) but **distinct across goatnets**, so two goat goatnets can peer without `uuid_k` colliding. Because origin identity is lost at the hop, cross-hop admission is a **coarse trusted-parity-peer write rule** (the peer router may write the shared namespaces), distinct from the per-publisher slot rules — the point where the slot model and federation meet.
- **External sovereign goat partner** (runs the goat stack but a *separate sovereign trust root* — moon-pod / standalone-goatnet). This is **parity-of-grammar, not parity-of-trust**: it federates with the same grammar but a distinct CA, so its traffic is **namespace-prefixed by its goatnet/sovereign id** (which also tags its origin — answering "is this from our router or a peer's") and trust is established explicitly. This is the partner case that most needs build-out; it composes with the partner-peering-portal and the [moon-pod](https://github.com/usareur-af/goatstuff/blob/main/docs/design/goat-moon-pod.md).
- **Non-goat third party** — **unchanged**: the gateway/quarantine path above (`<peering-entity>/<peering-id>/<original-key>`, gateway-scoped writer). The uuid anchor doesn't affect it, because third-party data is namespaced verbatim under the quarantine prefix and so can't collide with our `<uuid>/**` slots.

**Sandbox stage: stay flexible** — none of the strict-parity machinery is enforced yet; the quarantine-by-default floor is the only invariant.

## Sovereign-partner federation — the releasability model (decided 2026-07-16)

A sovereign running its own goat-pattern network federates by **scoped router-to-router peering**, not by republishing each topic across the boundary. The *releasability model* at that boundary is settled here, except the labeling rule for forwarding, which is a NATO community decision; the trust-root and key mechanics also remain open (below).

- **Scope the peering to shared `REL-` namespaces.** A network mirrors the releasable slice of its data into `REL-` namespaces aligned with what the receiver expects, and the peering's ingress/egress ACL admits only those — so the whole releasable slice syncs over one link. Zenoh routing is interest-scoped natively; the per-prefix `egress`/`ingress` ACL (above) enforces it. Advantage: far less machine-level sync than per-topic bridging. Cost: the topics, the rules, and any encryption must be **pre-aligned on both sides** before the peering carries them.
- **Reach is what the sender forwarded — there is no alliance-wide environment and no alliance-wide default.** Each sovereign runs its own separate environment ([ADR 1208](../adr/1208-sovereign-environments-release-namespaces-and-grant-encryption.md)), so data arrives over a peering only because the sender deliberately forwarded it, into the `REL-` namespace the sender chose. Whether data is labeled to forward or labeled to hold back is a decision for the NATO community. The active forces and sovereign nations that operate these networks will make it over time; this document does not settle it, and no rule here is offered as a cure for every data-sharing problem. Label to forward is the likely choice, and label to hold back is also possible. A common environment run by a multinational organization may set an open default for its own members (ADR 1208, decision 7).
- **Finer bilateral/multilateral control = pre-encrypt before republish, then peer direct.** For control finer than a release namespace (e.g. releasable to one specific nation), the sender **encrypts to the recipient's key, places the ciphertext under `REL-`, then peers** — the content is sealed before the sync. Delivery is by **peering directly to the intended recipient's network** (to reach one nation, peer to that nation's backbone).
- **Transit is native and wanted — the guardrail is releasability-scoping, not no-transit.** Zenoh routes each message *transitively* toward wherever there is an interested, authorized subscriber, so a network reaches others **through** the ones it is peered to (network A peers network B; B is peered onward; releasable data flows A→B→the rest), with no direct link per pair. It is not flooding: *routers forward only data a downstream subscriber has declared interest in* (zenoh-data-fabric.md — structural filtering at the protocol level), so a message goes where it is wanted, not to all nodes. What each link's ingress/egress ACL scopes is **releasability**: it admits only the `REL-` channels that network is entitled to, so an unreleasable `REL-<other>` never crosses into a network that may not read it. We do **not** build a custom content router — Zenoh's keyexpr routing is the mechanism; what we deliberately avoid is a bespoke partitioned content-router layered on top of it.
- **Scale is a topology + class problem, not a peering-model problem.** At many-nation, hundreds-of-instances, live-telemetry scale: high-rate firehoses ride the loss-tolerant `volatile/` class ([ADR 1032](../adr/1032-zenoh-volatile-non-persisted-namespace-class.md), dropped under congestion, never stored) so telemetry cannot swamp transport; and the topology is **hierarchical/regional broker tiers — configuration, not code** ([ADR 0844](../adr/0844-zenoh-deployment-topology-and-acl-v1.md)), never a naive N-way full mesh. Loop-freedom is a native property of Zenoh's inter-router routing. **Not yet validated at that fan-out** — scaling is discovered, not asserted.
- **Origin across the hop** is the peer router's identity plus the sovereign-id namespace prefix; per-sample publisher identity is not carried across a Zenoh federation link. Where per-publisher identity matters, the publisher encodes it in the payload and the topic name.

**Still open (the labeling rule, the trust/key mechanics + scale validation):** the labeling rule for forwarding, which is a NATO community decision (above); cross-trust-root establishment between distinct sovereign CAs; cross-sovereign key-alignment so a recipient can decrypt what a sender pre-encrypted (M2 grant-keyed keys crossing trust roots); and validated loop-free, flood-controlled behavior at many-nation live-telemetry scale.

## Open decisions

- **Sovereign-goat-partner trust + key mechanics** — the releasability model is decided above; what remains is the namespace-prefix-by-goatnet-id scheme, cross-trust-root establishment between distinct sovereign CAs, and cross-sovereign key-alignment for the pre-encrypted bilateral case. The partner mechanics that still need specification.
- **Per-goatnet salt vs. an explicit `<goatnet>/` prefix** — the salt gives disjoint uuid spaces cheaply; an explicit goatnet/instance prefix additionally tags canonical-peer origin in the key. Decide which (or both) at the operational definition.
- **Peering-id reservation convention** — `peering-<id>` segment vs. a reserved `_peered` infix vs. a dedicated top-level quarantine root, to avoid colliding with an onboarded entity's own `<entity>/**` namespace.
- **Quarantine retention** — same 7-day TTL as onboarded vendors, or shorter (it's untrusted) / longer (it's the only copy)?
- **Gateway trust domain** — do third parties present their own X.509 (separate CA), an API token, or a portal-minted short-lived cert? Ties to Block 84.
- **Whether direct third-party router federation is ever allowed** at all, or whether the gateway is mandatory for all non-goat ingress (this doc assumes gateway-mandatory).

## Related

- [ADR 1020](../adr/1020-zenoh-federated-peering-trust-model.md) — the decision.
- [ADR 0844](../adr/0844-zenoh-deployment-topology-and-acl-v1.md) — federation topology + ACL v1 baseline (incl. the `telemetry/<format>/` wire-faithful prefix, the onboarded sibling of quarantine).
- [ADR 0216](../adr/0216-goat-net-federation-and-site-planes.md) — federation + site planes; schema decls in the federation plane.
- [ADR 0821](../adr/0821-mission-partner-sharing-pattern.md) — mission-partner sharing.
- `partner-peering-portal.md` (Block 84) — onboarding front end the gateway composes with.
- [`portal-schema-publishing.md`](portal-schema-publishing.md) — schema registry; §Long-term is the Tier-1 prerequisite.
- [`zenoh-trial-fabric.md`](zenoh-trial-fabric.md) — the v1 fabric whose wide-open posture this tightens.
- [ADR 1022](../adr/1022-zenoh-denial-observability.md) + F-177 — write-denial observability: the enforcement here denies silently; `goat pub --confirm` is the publisher-side signal, and ADR 1022 sequences the operator-side capture.
