# ADR 1219 — A peering edge holds `PEERING-EDGE`: it reaches only the core router's fabric TLS port and mesh DNS, and only platform admins reach it

| Field | Value |
|---|---|
| Status | Accepted. No live effect on merge: the groups exist only once a scope that declares them is re-applied, and no peer is declared into them here |
| Date | 2026-11-06 |
| Deciders | operator |
| Repo scope | goat core (node-class vocabulary, live-membership guards) |
| Amends | — |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 1157](1157-cross-sovereign-federation-edge-terminated.md) (edge-terminated federation), [ADR 1218](1218-peering-forwarder-when-neither-edge-accepts-inbound.md) (peering forwarder), ADR 1116 §7 (live-membership guard), [ADR 1126](1126-acl-assertions-rechecked-when-nothing-is-written.md) (assertions), [ADR 1129](1129-warrant-scoped-expiring-network-authorizations.md) (warrant), [ADR 1189](1189-netbird-read-only-dashboard-and-warrant-owned-membership.md), [ADR 1216](1216-net-engineer-group-and-edge-server-class.md) (the class-plus-rules shape this follows) |

## TL;DR (for a layperson)

A federation edge is the one machine that holds a partner's certificate authority. The peering
tunnel joins neither party's network (ADR 1157); the edge itself is a peer of its own goatnet's
mesh, because that is how it reaches its own core router. Since it trusts a foreign root, it must
reach nothing beyond that one connection. The edge sits in `PEERING-EDGE`
alone; it may open one connection, to the core Zenoh router's TLS port, and look up mesh names
on the mesh nameserver; platform admins may log in to it; nothing else on the mesh can reach it,
and it can reach nothing else.

## Context

The edge router (`roles/goat-zenoh-edge`) dials the core router over `goat_zenoh_edge_core_endpoints`
(`tls/<core>:7447`, where `<core>` is the router's mesh name) and carries the partner link over its
own tunnel (`roles/goat-peering-wg`, or the forwarder carrier of ADR 1218). Beyond the core link it
needs one thing from the mesh: resolving that name.

An edge enrolled as ordinary gear (for example `KID`) holds the gear rules: tcp 443/53/5353/7447 and
udp 53/5353/7448 to every `INFRA` peer, and any-port reach from `INFRA` and `PLATFORM-ADMIN`. That is
the reach of a trusted publisher, held by the one machine that also trusts a foreign root.

No existing class has the needed shape. `NETWORK` and `EDGE-SERVER` reach the management plane and
have no fabric port. The gear classes reach the whole substrate surface. And every `INFRA` peer is a
destination of any rule written against `INFRA`, so "the core router" cannot be named by `INFRA`:
on a goatnet with several substrate hosts, `INFRA:7447` is several routers.

## Decision

1. **`PEERING-EDGE` — a NODE CLASS** for a federation edge router, a peer of its own
   goatnet's mesh. Class name and group name are the same. A peer holding it holds it alone.
   - It initiates toward `FABRIC-CORE` on port-set `fabric` tcp (7447, the TLS listener), and
     toward `INFRA` on port-set `mesh-dns` udp (5353, the mesh nameserver), and nothing else.
     No udp 7448, no web, no management plane, no lateral reach, never P2P.
   - Only `PLATFORM-ADMIN` reaches it, on port-set `shell` (tcp 22). `INFRA` does not.
   - Placed by the membership declaration (`exact: [<P>-PEERING-EDGE]`), never minted: the portal
     enrollment enum does not carry it, so re-deriving the box's groups from its enrollment class
     is a widening the declaration would show.
   - Added to the closed enums that classify live peers: the node-class membership guard
     (`ops/ci/check-node-class-membership.sh`, as a non-gear class beside `EDGE-SERVER`), the
     membership reconciler's machine suffixes (`ops/ci/check-membership-reconcile.py`), and
     `ops/identity/grammar.yaml`.
2. **`FABRIC-CORE` — a CAPABILITY GROUP**, destination only, held beside `INFRA` by the substrate
   host(s) that run the goatnet's core fabric router and nothing else. It grants its members
   nothing; it exists so a rule can name the core router rather than the whole substrate. Same
   pattern as the per-cluster `BACKEND-<CLUSTER>` groups.
3. **Rules, one way, `bidirectional: false`**, in the deployment's rule intents for each scope
   whose edge is a mesh peer:
   - `peering-edge → fabric-core` tcp, port-set `fabric`;
   - `peering-edge → infra (mesh dns only)` udp, port-set `mesh-dns`, the shape of
     `breakglass → infra (mesh dns only)`;
   - `platform-admin → peering-edge` tcp, port-set `shell`.
   No new port set: all three arms exist.
4. **The edge dials the core router by its mesh name**, resolved through the mesh nameserver on
   udp 5353. Resolving a name reaches no service: the only service port the class opens is
   `FABRIC-CORE:7447`.
5. **Assertions** (ADR 1126) per scope that declares the class: the edge reaches `FABRIC-CORE:7447`
   and `INFRA` udp 5353; it reaches nothing else on `FABRIC-CORE` (tcp 443, 53, 5353, 22; udp 7448,
   53), no tcp port, no other udp port and no icmp on `INFRA`, and nothing on any gear class, any
   person group, `P2P`, `POD` or `MOON`; `PLATFORM-ADMIN` reaches it on 22 and
   on nothing else; no other group (including `INFRA` and `P2P`) reaches it.

## Alternatives considered

- **Keep the edge in a gear class.** Rejected: substrate-wide reach on six ports, and any-port
  inbound from every substrate host, on the machine that trusts a foreign root.
- **`peering-edge → INFRA` on tcp 7447.** Rejected: on a goatnet with several substrate hosts it
  reaches every one of their routers, which is not "the core router".
- **A rule whose destination is one peer resource.** Rejected: the generator and the assertion
  evaluator are group-to-group, and a peer id is a fact of one management server that a
  re-enrollment changes.
- **Reuse `EDGE-SERVER`.** Rejected: management-plane reach and no fabric port, the opposite of
  what an edge router needs.

## Consequences

**Positive.** The edge reaches its core over its own mesh under a declared, asserted rule. A
compromised edge reaches only the port its legitimate traffic uses. The live guards classify the
edge rather than reporting it unclassified.

**Negative.** Two groups per scope. A goatnet whose core router moves host must move
`FABRIC-CORE` membership with it, or the edge loses its core link. `check-warrant-narrowing.py`
grades the rules a widening, because each is new; for the peer that moves out of a gear class,
the change is a narrowing.

## Assessor view (SCA-R)

Least privilege for the one host trusting an external root: one outbound service port to one
named destination, name resolution on the mesh nameserver, one administrative inbound port for privileged users (AC-4, AC-6, AC-17,
CM-7). Declared in version control, generated, asserted on each change and against live state
(CM-3, CA-7).

## Trigger conditions for revisit

- The edge leaves the mesh (ADR 1157 step 1 placement): retire the deployment's rules and
  membership entries.
- The core router gains a second listener the edge must use (for example QUIC on udp 7448).
- NetBird gains per-rule peer destinations that the generator and evaluator can model.
