# ADR 1224 — Released data goes one hop, unless its owner marks it for general release, which travels to every federated environment

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-11-08 |
| Deciders | operator |
| Repo scope | goat core (federation, goat-zenoh-edge, goat-fed-republisher, grant-crypto) |
| Amends | [ADR 1157](1157-cross-sovereign-federation-edge-terminated.md) §8 (transit) · [ADR 1170](1170-federation-growth-by-corroborated-party-roster.md) decision 7 (transitive reachability) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 1143](1143-unilateral-release-declaration-and-accession.md) (release declaration) · [ADR 1208](1208-sovereign-environments-release-namespaces-and-grant-encryption.md) · [`partner-data-exchange.md`](../design/partner-data-exchange.md) (the marking lives in the key) · [`zenoh-data-fabric.md`](../design/zenoh-data-fabric.md) §"Routing at scale" · [`federation-at-scale.md`](../design/federation-at-scale.md) §10 |

## TL;DR

Data a nation releases to a partner goes to that partner and stops there. It moves further only
if the receiving nation deliberately releases it again, under its own name. There is one
exception, and only the data's owner can choose it: the owner can mark a release as a
**general release**. A general release travels on its own to every federated environment,
through any chain of peerings, and Zenoh's routing delivers it once to each, without loops.
Grant encryption is not required for any of this.

## Context

ADR 1157 §8 permits hierarchical transit and requires sealing for anything that crosses more
than one link. ADR 1170 decision 7 refuses transitive reachability outright. The two disagree,
and neither matches the operator's position, stated in reply to partner questions on 8 November
2026: data traverses only through deliberate release; a one-hop share is not sent further; data
the owner intends for the widest use is labelled as such and shared as far as it is useful.

What exists today:

- An owner releases by publishing to `release/<set>/**`. Its republisher
  (`ops/ansible/roles/goat-fed-republisher/`) lifts that to `from/<its code>/<set>/**`. The key is
  not renamed in transit (ADR 1157 §6).
- Each edge facing a partner admits only `from/<that partner's code>/` (rule `partner-land`,
  `ops/ansible/roles/goat-zenoh-edge/templates/config.json5.j2`). So data from C that reaches B
  cannot pass B's edge towards A. One hop is already what the code enforces.
- Across a boundary, the only attribution is the ingress prefix and the peering certificate name
  (ADR 1157 §6). Beyond the first hop those no longer agree, so a prefix alone cannot prove
  origin.
- Zenoh routing is interest-scoped and loop-free by construction
  ([`zenoh-data-fabric.md`](../design/zenoh-data-fabric.md) §"Routing at scale"). Federated edge
  routers join the environments' router graphs, which is why ADR 1157 §8 sizes the link-state
  domain across nations.

## Decision

**1. One hop is the default.** A release reaches the environments its owner peers with and goes
no further. The edge rule that admits only `from/<partner's code>/` stays as it is.

**2. Further movement is a deliberate re-release.** A receiving nation that wants to pass data on
publishes it to one of its own release sets. It then leaves as `from/<receiver's code>/…`,
attributed to the re-releasing nation, under the owner's onward-sharing term (release
declaration term 2) and with the origin kept attached (term 3).

**3. An owner may mark a release as general.** Publishing to `release/general/**` marks it.
The republisher lifts it to `from/<owner's code>/general/**`, and the key keeps the owner's code
at every hop. Only the owner's environment can write under its own `from/<code>/`, so only the
owner can make a general release. No other environment can turn a one-hop release into a general
one.

**4. Every federated edge admits and forwards `from/*/general/**` from every peer.** This is a
second edge rule beside `partner-land`. A general release therefore reaches every environment
connected by any chain of peerings, with no action by the environments in between.

**5. Zenoh's routing prevents loops and duplicate delivery.** In a cycle of peerings (A–B, B–C,
C–A), a general release from A reaches C once. goat adds no loop-prevention mechanism of its own;
the acceptance tests in the plan prove the property on a triangle and on a line before the rule
is enabled.

**6. Origin is proven by signature, not by prefix.** A general release is signed by the owner's
SPIFFE identity, using the ECDSA P-256 signing that grant-crypto already uses for key bundles
(`ops/grant-crypto/WIRE-FORMAT.md`, `ops/grant-crypto/envelope/`). A
receiver verifies it against the owner's trust bundle: from its own peering, or from a framework
roster ([ADR 1170](1170-federation-growth-by-corroborated-party-roster.md)) for an owner it does
not peer with. A message that does not verify is not presented as the owner's. Routers forward
without checking signatures, and the edge's existing rate and size caps bound what a dishonest
middle environment can inject.

**7. Grant encryption is not required for transit, one hop or general.** An owner may still seal
a release to a named audience; a sealed general release is readable only by that audience.

**8. A general release is the owner's statement, recorded in its release declaration.** The
declaration template gains a "general release" option under the onward-sharing term. Withdrawing
a general release stops new data; data already delivered stays where it is, as with any release.

## Plan

Each step is separate work, done in order. Status: designed, not built. No date is set.

| # | Work | Where |
|---|---|---|
| 1 | Republisher maps `release/general/**` to `from/<ours>/general/**` | `ops/ansible/roles/goat-fed-republisher/` |
| 2 | Edge admits and forwards `from/*/general/**` on every peering; the peering validator accepts it | `ops/ansible/roles/goat-zenoh-edge/templates/config.json5.j2`, `ops/federation/validate-partners.py` |
| 3 | Core storage and access rules for `from/*/general/**`; no local identity writes under another code's `from/` | `ops/ansible/roles/goat-zenoh/` |
| 4 | Owner signs general releases; a receiver library verifies against peering bundles and roster bundles | `ops/grant-crypto/envelope/`, `ops/grant-crypto/sdk/` |
| 5 | Release declaration template: "general release" option under onward sharing | goatdocs `docs/partner-templates/release-declaration-and-accession.md` |
| 6 | Runbook and design text updated | `docs/operations/sovereign-federation-runbook.md`, `docs/design/federation-at-scale.md` §10 |
| 7 | Acceptance across three environments, as a triangle and as a line: a general release from A reaches C once in both; a one-hop release from A does not reach C; B's re-release reaches C as `from/B/…`; a forged `from/A/general/…` published by B fails verification at C | a recorded run, kept by each environment |

## Alternatives rejected

- **No transit (ADR 1170 decision 7 as written):** forces every nation to re-release by hand
  data its owner wanted shared everywhere.
- **Hierarchical transit, sealed (ADR 1157 §8 as written):** ties transit to region layout and to
  encryption, neither of which decides who may receive the data.
- **A marking in message metadata only:** cannot be enforced by key-prefix rules, against the
  rule that the marking lives in the key ([`partner-data-exchange.md`](../design/partner-data-exchange.md)).
- **A goat-run hub that forwards general releases:** adds a party every nation must trust, against
  ADR 1208.

## Consequences

- One-hop sharing stays what the code enforces today; the change adds one opt-in lane.
- An owner can reach every federated environment with one marking, and the middle environments
  do no work.
- Each environment carries general-release traffic for others. The existing edge caps limit it,
  and each nation sets its own retention for what it stores.
