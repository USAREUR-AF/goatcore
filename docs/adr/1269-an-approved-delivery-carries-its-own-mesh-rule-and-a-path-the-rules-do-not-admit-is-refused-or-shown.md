# ADR 1269 — An approved delivery carries its own mesh rule, a tactical site's working set reaches each other on tactical data ports, and a path the rules do not admit is refused or shown

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-11-26 |
| Deciders | operator |
| Repo scope | goat core (this decision; `ops/warrant/`: port sets, generator, assertions; `docs/design/warrant.md`, `fabric-feeds.md`, `mesh-change-control.md`), goatstuff (`goat-feeds`: the check at approval and resume, the link health record), private overlays (rule intents, per-site assertions, feed warrants) |
| Amends | [ADR 1259](1259-fabric-feeds-self-service-approved-delivery-of-fabric-topics-to-mesh-systems.md) (an approval now also authorizes the network path), [ADR 1260](1260-every-tactical-link-in-one-registry-with-one-health-record.md) (the `unreachable` state gains a declared cause) |
| Supersedes | — |
| Superseded by | — |
| Related | ADR 1124 (scope-qualified destinations and cross-scope reach), [ADR 1126](1126-acl-assertions-rechecked-when-nothing-is-written.md) (ACL assertions and their one evaluator), [ADR 1129](1129-warrant-scoped-expiring-network-authorizations.md) (warrants: scoped, expiring network authorizations), [ADR 1266](1266-a-tactical-data-chain-is-declared-source-to-receiver-judged-hop-by-hop-and-pages-when-data-stops-between-two-hops.md) (the chain judge), [ADR 1168](1168-self-hosted-paging-and-automatic-oncall-enrolment.md) (pager), designs [`warrant.md`](../design/warrant.md), [`fabric-feeds.md`](../design/fabric-feeds.md), [`mesh-change-control.md`](../design/mesh-change-control.md) |

## TL;DR (for a layperson)

Two separate systems decide whether data reaches a machine. One is the approval: a tactical admin
agrees that a feed may be delivered to a console. The other is the mesh's access rules: the list of
which machines may send to which, on which ports. Until now nothing connected them. A feed could be
approved, show as healthy and sending, and still never arrive, because no access rule let it through,
and the receiving machine discarded it without telling anyone.

This decision joins them. Approving a delivery creates the access rule for exactly that path, and
removing the delivery removes the rule. Machines that work together at one tactical site can reach
each other on tactical data ports without a hand-written exception for each pair. And when a path is
not allowed, the system says so at the moment the feed is made, on the feed's page, and to the pager.
It never fails silently.

## Context

A delivery from Fieldkit Feeds (ADR 1259) is a small process on a core node that sends a fabric
topic to one mesh system at an address, protocol and port. A tactical admin approves it. The mesh ACL
is generated from the warrant registry (ADR 1129) and enforced on the receiving peer. The two have no
shared record:

| Fact | Where it lives today | What reads it |
|---|---|---|
| "This feed may go to this console on this port" | the feeds registry | the delivery process |
| "This source group may reach this destination group on this port" | the warrant registry's rule intents | the mesh ACL generator |

Three properties of the mesh make the gap invisible:

- **Enforcement is at the receiver.** The sender's datagrams leave; the receiving peer's firewall drops
  them. The sender's counters rise.
- **"Connected" is not "admitted".** Two peers show as connected when any rule links them in either
  direction. A console that may reach the core's fabric port is connected to the core even when
  nothing lets the core send to the console.
- **UDP gives no answer.** A dropped datagram and a delivered one look the same from the sender.

The registry's scope model adds a second way to fall in the gap. Gear at a tactical site is enrolled
in that site's groups, and the core node holds the goatnet's infrastructure group. A rule written for
"infrastructure to gear" inside one scope does not cover the other scope's gear. ADR 1124 already
allows port-scoped service reach between a host site and a tactical site in either direction, and
forbids any-port administrative reach from the narrower scope into the broader one. The delivery path
is service reach, so it is permitted in principle; it was simply not declared.

## Decision

1. **An approved delivery carries its own network authorization.** Approving a feed to a destination
   creates a warrant for exactly that path: source, the group of the host that runs the delivery;
   destination, the group that holds the destination peer; the feed's protocol; a named port set that
   holds its port. The warrant is scoped to that path and expires with the feed's own end date
   (ADR 1259 feeds already carry one). The warrant registry generates it into the ACL like any other
   warrant. Pausing a feed leaves the warrant; removing or expiring the feed removes it, and the ACL
   rule goes with it at the next generation. Approval and reachability are one act, recorded once.

2. **A tactical site's working set reaches each other on tactical data ports by default.** Each
   tactical-site scope carries one standing rule set, declared once in the scope template:
   - the site's sensors, sensor nodes and consoles reach each other on the named tactical data port
     sets;
   - the host site's feed host reaches the site's consoles on the feed-delivery port set.

   Administration is not part of it: shell, vendor consoles and any-port reach stay gated by role, as
   today. This keeps ADR 1124's invariant. The rules are port-scoped service reach, which may flow in
   either direction between scopes; any-port administrative reach still flows only from the broader
   scope into the narrower one, never the reverse.

3. **A path the declared rules do not admit is refused or shown, never silent.**
   - **At creation and resume.** Fieldkit Feeds evaluates the declared ACL for the feed's path with
     the one evaluator of ADR 1126 (the semantics of `goat acl test`: source peer, destination peer,
     protocol, port). If no rule admits it, and decision 1 cannot create one (the destination is
     outside every scope the approver may warrant), the feed is refused with the missing rule named:
     source group, destination group, protocol and port.
   - **While running.** The link's health record (ADR 1260) carries
     `unreachable: no mesh rule admits <source> -> <destination> <protocol>/<port>` whenever the
     declared ACL stops admitting the path, for example after a peer changes group. This is read from
     the declared ACL and live group membership, not inferred from traffic.
   - **To a person.** The chain judge (ADR 1266) grades the chain `broken at delivery` on that record
     and pages through the existing pager (ADR 1168). The page names the rule that is missing.

4. **Every delivery port is a named port set, and every declared delivery is asserted.** No rule
   carries a bare port number: a delivery port belongs to a named port set in the registry
   (`ops/warrant/port-sets.yaml`), with a purpose and an owner. Each site's ACL assertions (ADR 1126)
   include its declared deliveries: an `accept` for each delivery path, and a `deny` for a
   neighboring port and for a class that must not receive it, so a rule that is too narrow or too
   wide both fail the gate.

## Status of each part

| Part | Status |
|---|---|
| Decision 1: a warrant per approved delivery, generated and expired with the feed | Designed, not built |
| Decision 2: the standing tactical-site rule set | Designed. Its first rule is written: the named port set `feed-delivery` and one scope-template rule from the host site's infrastructure to a site's console classes on it, with accept and deny assertions (two pull requests, open) |
| Decision 3: refusal at creation, the `unreachable` cause on the health record, the page | Designed, not built. Fieldkit's existing "Check destination" pings and connects; it does not read the ACL |
| Decision 4: named port sets and per-delivery assertions | The registry and the assertion evaluator are built (ADR 1126, ADR 1129); the rule that every delivery has both is new |

## Consequences

- A tactical admin who approves a feed has done everything needed for it to arrive. Nobody has to know
  that a second registry exists.
- A feed whose path is not admitted cannot be created as "healthy". The gap this decision closes
  cannot recur without an error message that names it.
- The warrant registry gains a second writer: Fieldkit Feeds proposes feed warrants. They are still
  generated, reviewed by the registry's own gates and expired like every other warrant; the mesh
  dashboard stays read-only ([`mesh-change-control.md`](../design/mesh-change-control.md)).
- The standing rule set in a scope template applies to every tactical site that uses the template. A
  site that must be narrower declares the deviation in its own scope.
- Fieldkit Feeds needs read access to the declared ACL and to group membership for its site, and the
  evaluator as a library.
- The check is against the declared rules. A receiver's own host firewall can still drop an admitted
  datagram; that case stays `receipt-unconfirmed` (ADR 1266) until the receiver acknowledges.

## Alternatives considered

| Option | Why not |
|---|---|
| Give the core node any-port reach to every tactical site's gear | Wider than any delivery needs, and it turns a data path into administrative reach across scopes |
| Open a standing wide port range for deliveries | Every port in the range is open whether or not a feed uses it; nothing ties an open port to an approval |
| Keep approvals and rules separate, and document the second step | The second step is the one that is forgotten, and forgetting it produces no error |
| Detect the gap from traffic (sent but never acknowledged) | UDP receivers send nothing back; silence is also what a healthy one-way feed looks like |
| Let Fieldkit write mesh policies directly through the management API | Bypasses the registry, its review, its expiry and its assertions; the next generated apply would remove the policy |
