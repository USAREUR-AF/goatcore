# ADR 1170 — Federation grows by a roster of corroborated parties, and a roster may never be the source of a first-contact binding

| Field | Value |
|---|---|
| Status | **Proposed — UNREVIEWED. Not ratified, and not a basis for building.** No platform engineer has reviewed this ADR and the commissioning operator has not read it; it was authored in a single session. Treat every decision below as a proposal to be argued with. Nothing in a live goatnet changes on merge — no goatnet has a peering with anything, and `spire_federation_enabled: false` on all three. |
| Date | 2026-09-06 |
| Deciders | operator |
| Design of record | [`../design/federation-at-scale.md`](../design/federation-at-scale.md) |
| Tracking | `../project/federation-at-scale-plan.md` |
| Evidence | F-424 — the access layer cannot see the issuer, so per-partner isolation is by process. [`../design/federation-package-spec.md`](../design/federation-package-spec.md) §5.1 — the two values needing an authenticated channel are established once per party. |
| Related | [ADR 1157](1157-cross-sovereign-federation-edge-terminated.md) · [ADR 1143](1143-unilateral-release-declaration-and-accession.md) · [ADR 1138](1138-pre-cut-partner-slot-blocks.md) · [ADR 1094](1094-grant-keyed-payload-encryption.md) · [ADR 1027](1027-partner-release-set-governance.md) |

## Before acting on this

**This ADR has not been reviewed by a platform engineer.** The operator who commissioned it has not
read it. It is recorded so the reasoning is not lost, not because it is settled.

Whoever picks it up reviews it first, and reviews the *judgement* — whether the problem is framed
right and whether decision 2, the refusal that the whole design rests on, is the right control.
[`../design/federation-at-scale.md`](../design/federation-at-scale.md) §12 lists the claims to
attack first and records the two status errors already found in this work after it merged.

**Do not treat a merged ADR as a ratified one.** This one merged because the reasoning was worth
keeping in the repository, on the explicit basis that it was unreviewed.

## TL;DR (for a layperson)

Two organisations that want to exchange data must first confirm two short values with each
other — a signing key and a fingerprint — over a channel where each knows who it is talking to.
A phone call between people who recognise each other's voices does. Today that call happens
once for every *pair* of organisations, so twenty organisations means one hundred and ninety
calls, and the group never forms.

The call only ever needed to happen once per *organisation*. This decision records that
confirmation as a durable thing an organisation keeps and reuses, and lets a group of
organisations share a signed list of who has been confirmed, so a newcomer confirms itself with
a few members rather than with all of them. The list is a convenience and grants nothing: an
organisation still decides, separately and for itself, what any other organisation may do.

The rule that keeps that true is one refusal. **You may not set up a link with a party you have
never confirmed for yourself.** A list you were handed is not a confirmation.

## Context

Cross-sovereign federation is pairwise by construction, and that part is not in question. The
fabric's access layer matches a certificate Subject CN by exact string and cannot see which
authority issued it, so one certificate authority per trust store is the only separation
available and the process is the only unit of isolation. Two partners can
never share an edge router, a trust store or a tunnel.

What is in question is the growth story. `N` parties means `N(N-1)/2` peerings — ten at five,
one hundred and ninety at twenty — and each one today carries a negotiation and an out-of-band
confirmation as well as its containers and config. Growth has been grassroots and ad hoc, and
there is no story for a party joining an existing group or for withdrawing one from it.

The costs are not one thing. Processes, trust stores, tunnels and rendered config are
genuinely quadratic and genuinely forced. The out-of-band confirmation is neither, and the
specification already says so — `federation-package-spec.md` §5.1, line 604: *"They are
established in the same act, once per party, and again only if that party changes them."* No
artifact in the repository records a corroborated party, so a bound the specification set has
never been collected.

## Decision

**1. A corroborated party is a durable, per-party record.** One file per party at
`ops/federation/parties/<sovereign_id>.yaml`, holding that party's Ed25519 verifying key, its
`identity.ca_fingerprint`, and the act that established them — date, channel, and who
performed it. It is written once per party and again only when that party changes a value. It
is not per peering.

**2. `goat federate apply` refuses a counterparty with no local corroboration record, or one
whose package disagrees with it.** This is the decision's load-bearing half. It makes the
specification's rule — *a directory is a cache of this binding, never its source* — a refusal
in the tool rather than a sentence in a document.

**3. A framework may publish a roster of its members' corroborated identities.** The roster
carries `sovereign_id`, the two §5.1 values, trust domain and bundle endpoint, and the
corroboration date and method for each entry. It carries no `transport_offer`, no `data_offer`
and no `cut` — those are sovereign operational detail and move directly between two parties.

**4. A roster is valid only on a threshold of member signatures.** A framework of `n` members
accepts a roster revision carrying at least `k` member signatures, so the body that hosts the
file cannot add, remove or alter a party by itself.

**5. Every party keeps local overrides, and they outrank the roster.** Refuse this member; pin
this fingerprint regardless of what the roster says. Overrides are applied after the roster,
never before.

**6. Peerings stay bilateral and stay one process per partner.** The roster changes what a
party knows, never what it permits. Composing an agreement, comparing a digest and running
`apply` are unchanged, and every peering still gets its own edge container, its own trust store
holding exactly one certificate authority, and its own tunnel.

**7. Transitive reachability is refused.** A party reaches only parties it has peered with. No
roster membership, and no chain of peerings, creates a path between two parties that have not
peered. *Amended by [ADR 1224](1224-transit-is-one-hop-unless-the-owner-marks-a-general-release.md) (Proposed): data its owner marks as a general release travels to every federated environment, and a roster supplies the trust bundle that verifies its origin.*

**8. Withdrawing a party from a framework is a roster revision, and each member converges
through the retirement path that already exists** — moving the peering's entry from
`spire_trust_sync_federated_edges` to `spire_trust_sync_federated_edges_retired`
(`ops/ansible/roles/spire/defaults/main.yml:90,110`), which reconciles that edge's trust file to
our own bundle alone on the next tick and restarts the edge router. The immediate cut remains
removing the far side from the tunnel on your own edge host.

## What this does not decide

**It does not decide that a first framework may carry data whose retained history matters.**
Removing a member closes the forward direction — a new key generation excludes them, built and
fired end-to-end on real portal-minted certificates. It does not
close the past, and the act meant to close the past is withdrawal of a per-release-set read
scope on the key topic. F-482 measured that scope absent on 2026-09-06; **#790 built and
deployed it the next day**, giving each member a `keys/<set>/**` read lane on the live ZAGROS router
and removing that lane when the member leaves the set. So the forward direction is closed and the
onward-following direction is now closed too. What remains open is the backward window — bundles a
removed party could already have pulled — and `grant-keyed-encryption.md`
§4b **accepts** it at a seven-day bound set by the router's data TTL rather than closing it.
**A first framework is sized against that bound**, which is the same constraint [ADR 1169](1169-partner-audience-membership-via-spire-federation.md)
places on a first federated partner, for the same reason.

## Alternatives rejected

**A coalition certificate authority every member's core router trusts.** Rejected: it is the
definition of F-424. Any member could mint `goat-zenoh-router` and inherit the `router-peer`
subject, which binds read and delivery on `**`.

**A shared fabric all parties join.** Rejected: a broker, in a system whose fabric is brokerless
by root decision. A shared router re-adds a central QoS point, a central ACL and a central
failure.

**Transitive trust — A peers with B, B peers with C, therefore A trusts C.** Rejected: SPIFFE
federation is one-way and non-transitive precisely to prevent trust-laundering, and under F-424
a transitive path is not a weaker grant than a direct one. It is the identical grant, obtained
without the grantor's act.

**Trust chains to a common anchor, in the OpenID Federation shape.** Rejected: metadata policy
inherited down a chain from a top-level authority is a central trust anchor. It resolves
freshness well and is worth reading; it decides the wrong thing here.

**A roster that vouches for a party nobody has corroborated.** Rejected, and this is the
alternative the design will be under pressure to adopt. A key learned only from a roster is a
key learned from whoever assembles the roster, which is the RPKI coercion problem and the
eIDAS List of Trusted Lists in miniature. Decision 2 is what refuses it.

**Reducing `N` by making organisations share a certificate authority.** Not rejected — recorded
as available and never the default. A group that shares an authority is one party, which is
the state of goatnets run by one operator. Inside such a group F-424's collapse is
total and deliberate: any member can mint any other member's names. That is a legitimate choice
between environments with one risk owner and the wrong one between sovereigns.

**Sealed objects forwarded over any path, with the audience rather than the link as the access
boundary.** Not rejected on its mechanism, which is sound and preserves the grant-keyed
property that the producer is a member of its own release-set. Gated: with no admission layer to
cut and no ACL lane on the object's path, a party cannot be withdrawn from a sealed-object
distribution at all — the lever every other candidate uses is the per-set key lane, and candidate 3
is the one shape that puts objects where no lane sits. It also weakens provenance, because the
`from/<sovereign>/` prefix is trustworthy only on a hop an ACL enforced.

## Consequences

**Good.** The out-of-band cost becomes `O(N)` — a party joining a twenty-member framework
corroborates with `k` members, not nineteen. A newcomer has a join story, and a departing party
has a leave story that runs on machinery already built and already CI-guarded. Nothing in the
package format changes, so no package or digest in circulation is invalidated — which matters,
because D-1 records that changing the signature envelope would invalidate all of them.

**Costly.** Three things do not exist: the party record, the roster and its threshold
verification, and the `apply` refusal. The refusal is the one that must land first; a roster
built before it is the rejected alternative wearing this ADR's name.

**Contentious.** A threshold-signed roster looks like a central trust anchor, and the
resemblance is not superficial. The difference is entirely decision 2. A reviewer who reads the
roster as authoritative has read it correctly for every system in the survey except PeeringDB,
and the design is only distinguishable from those systems while the refusal holds.

**Residual risk, stated plainly.** This design's safety rests on a refusal in a tool holding
under operational pressure. Nothing cryptographic enforces it, the pressure to remove it will
arrive framed as convenience, and a party that never exercises its local override has delegated
its recognition decision in practice whatever this document says.

**Unresolved.** What `k` should be, and whether it varies with framework size. A member
co-signing because four others already did adds a signature rather than a check, which is an
operating question no threshold settles — and the reason a first framework should be small.

---

*See `GLOSSARY.md` for terms and acronyms.*
