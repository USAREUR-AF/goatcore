# ADR 1251 — Every source keeps its identity through a pod: carriage rules, provenance at entry, and pod local authority

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-11-18 |
| Deciders | operator |
| Repo scope | goat core: portal registry, router ACL rendering, goat CLI, moon-pod |
| Amends | [ADR 1105](1105-pubsub-org-admin-co-publish-write-grants.md) (a single-owner root may also be written by a pod its carriage rules allow) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 1020](1020-zenoh-federated-peering-trust-model.md) (the receiver sees the forwarder), [ADR 1096](1096-delegated-machine-enrollment.md) (machines are their own principals), [ADR 1165](1165-island-mode-multi-writer-every-site-admits.md) (island mode), [ADR 1109](1109-fleet-management-dimension-and-pull-convergence.md) (pulled convergence), [ADR 1129](1129-warrant-scoped-expiring-network-authorizations.md) (generated ACLs), [ADR 1157](1157-cross-sovereign-federation-edge-terminated.md) (cross-sovereign), design [`pod-carriage-and-provenance.md`](../design/pod-carriage-and-provenance.md), design [`fieldkit-and-bleating-edge.md`](../design/fieldkit-and-bleating-edge.md), design [`pod-targeted-client-bundles.md`](../design/pod-targeted-client-bundles.md) |

## TL;DR (for a layperson)

A device that reaches goat through a moon-pod today can publish only under the pod's own identity, so every device behind a
pod looks like the pod. Under this decision every source keeps its own identity whatever path it takes: straight to the core,
through any pod it is allowed to use, or directly to a neighbour. A pod forwards a device's data only where carriage rules
allow, and those rules are written per org, not as a record of where each device is. Every sample carries a signed record of
where it entered goat, added by the device or by an adapter for systems that cannot sign. A pod cut off from goat keeps
working and can have its own rules changed on site by a named person, with the changes reconciled when the link returns.

## Context

Zenoh's access control checks only the certificate of the directly connected peer (ADR 1020; the x.509 plane note in
[`federated-trust-model.md`](../design/federated-trust-model.md)). A core router therefore sees a pod's certificate, not the
device behind it, and accepts only the pod's own slot from that connection. Operators have added device slots to a pod's ACL
by hand, which changes nothing at the core (denial is silent, [`zenoh-brokerless-consequences.md`](../design/zenoh-brokerless-consequences.md)),
and have carried the device's slot as a prefix in the key, which any client of the pod can forge. The pod's own rule also lets
any certificate from the goatnet CA write the pod's slot.

## Decision

1. **Four layers, kept apart.** Identity (slot, certificate, key) and catalog (topics) are set at enrollment and never depend on
   the path. Carriage (which pods may forward a source's data) is policy. The route (which router or peer a source uses now) is
   live state and is recorded nowhere central. A source can enroll against the core and use any allowed pod later; topics are
   never registered "against a pod".
2. **Carriage is written as rules.** The default rule, created when an org is onboarded, lets pods run by an org carry that
   org's devices. Named rules cover other cases (a partner's pod). Every named or temporary rule carries an end date. Expired
   rules are archived, never deleted, and can be reactivated by their owner in one step within a reactivation window
   (proposed: six months); older ones are approved again like a new rule. A sensitive device can opt out of the org rule.
3. **Rules are rendered, never hand-edited.** A generator renders the rules into each pod's ACL and the core router's ACL. A pod
   admits each covered device's own certificate under its own slot; only the pod's certificate writes the pod's slot; the core
   lets a pod's connection write exactly the slots its rules cover. Pods pull a signed rule generation. A drift check compares
   rendered ACLs with the rules.
4. **Direct links need no rule.** Where the receiving router or peer sees the source's own certificate, the source's own access
   applies. Bundles list preferred routers as hints, never a binding.
5. **Provenance at entry.** Every sample carries a signature in its per-sample attachment, added where it first enters goat and
   forwarded unchanged; pods never re-sign. Devices that hold a key sign their own data. A source that cannot (for example a
   radar feed over UDP) is enrolled as a keyless source with its own slot, and an ingress adapter, itself a principal, signs on
   its behalf, stating the source slot and how the data was received. Three grades: device-signed, adapter-attested,
   pod-vouched only. Unsigned data still flows with its grade visible; a topic may later require a minimum grade.
6. **Island mode with local authority.** A pod cut off from goat keeps enforcing its last signed rule generation and reports its
   age. A local authority, named in advance in the pod's signed rules, can add, extend, revoke or reactivate rules for that pod
   only, each with an end date, each signed locally and audited. Local authority cannot change another pod or the core, cannot
   grant without an end date, and cannot mint identities. On reconnection the pod reports its local changes; data sent under
   them reaches goat marked as not yet ratified; an org lead ratifies each or lets it lapse; goat's revocations win.
7. **Tooling.** The pod's local API and `goat pod` verbs are primary. Operators who are not developers use Fieldkit Gatepost, a
   six-action page over the same API, built from the Bleating Edge design system
   ([`fieldkit-and-bleating-edge.md`](../design/fieldkit-and-bleating-edge.md)).
8. **Boundaries.** A pod forwards only sources it authenticated itself (no pod behind a pod). A carriage rule never crosses a
   trust root; foreign sources use ADR 1157's forwarding router or ADR 1020's gateway.

## Alternatives

- **Keep everything under the pod's slot.** Rejected: devices behind a pod cannot be told apart, and the forgeable prefix
  workaround stays.
- **Record each device's current pod.** Rejected: stale as soon as a device moves, and unusable while a pod is cut off.
- **Require device signatures before any data passes a pod.** Rejected as a gate: existing feeds cannot sign. Kept as the
  strongest provenance grade.
- **Stop a cut-off pod after a time limit.** Rejected as the default: it halts local work during the outages pods exist for. Kept
  as a per-pod option for unusually sensitive pods.

## Consequences

- A compromised pod can forge only the sources its rules cover; provenance signatures expose forgery by grade.
- New surfaces: rule storage and rendering in the portal, the pod's local API and ledger, an ingress adapter, `goat pod`,
  Gatepost and the Bleating Edge design system.
- The core router's ACL template change lands after the security fixes now changing that template are disclosed.
- Measured before the core change ships: whether one publication can reach a subscriber twice over two live paths, whether an
  ACL change applies without a router restart, and signature overhead per sample.
