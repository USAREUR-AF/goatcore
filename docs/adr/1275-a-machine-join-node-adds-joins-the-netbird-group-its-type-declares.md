# ADR 1275 — A machine join-node adds joins the NetBird group its type declares, and the rules that group already has

| Field | Value |
|---|---|
| Status | Proposed (2026-11-28). Designed, not built: no type file declares its group yet, and `join-node` still refuses to enrol every class but core and phone (GB-430). The operator rules on each row of Decision 2 |
| Date | 2026-11-28 |
| Repo scope | goat core (`ops/goatbuilder/targets/`, `crates/goatbuilder/src/joinnode.rs`, `ops/ansible/mesh-enroll-host.yml`); each deployment's rule intents hold the rules of the groups it uses |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 1257](1257-every-machine-kind-is-a-goatbuilder-build-target-type-declared-as-data.md) (target types as data), [ADR 1141](1141-four-node-host-site-baseline.md) (the obs pair), [ADR 1092](1092-moon-pod-peer-classes.md) (`POD`), [ADR 1219](1219-peering-edge-on-the-mesh-reaches-only-the-core-router.md) (`PEERING-EDGE`), [ADR 1216](1216-net-engineer-group-and-edge-server-class.md) (`EDGE-SERVER`), [ADR 1253](1253-simple-boundary-node-pseudo-boundary-node-and-one-boundary-monitor.md) (the simple boundary), [ADR 1122](1122-scale-envelope-systems-vs-peers-pod-tier.md) (the pod tier), [ADR 1114](1114-breakglass-node-class-and-two-path-reach.md) and [ADR 1131](1131-network-jump-class-and-herd-gear-scope.md) (unchanged here), [ADR 1129](1129-warrant-scoped-expiring-network-authorizations.md) (rule intents) |

## TL;DR (for a layperson)

When goatbuilder adds a new machine to a goatnet, the machine joins the mesh, the private network every machine shares. On the mesh, a machine can reach only what its group allows. Today goatbuilder knows the right group for two kinds of machine only, the core servers and phones, so it refuses to add the other kinds rather than guess. This decision names the group for each of seven more kinds: observability servers, the two kinds of boundary machine, the edge pod, the processing server, the network tap and the message decoder. In every case it reuses a group that an earlier decision already defined, or, where none fits, it gives the options and recommends one for the operator to choose. If we had chosen differently, a new machine could land in a group that lets it reach more than it should, or a group that lets nothing reach it, so it could never be finished.

## Context

`join-node` phase 10 enrols a machine on the mesh with a single-use setup key. The key carries the NetBird groups the machine joins, and the groups decide everything it may reach and everything that may reach it. Today the key takes the groups of the founder's own peer, which are the site's `INFRA` group. That is right for a core node and for a phone node, which converges as one. For any other class it would put the machine in the substrate tier, which reaches every peer on every port, so `join-node` refuses the enrolment (GB-430) at the run's first phase.

Seven of the target types that ADR 1257 made into files are joined by `join-node` and are not core: obs, federation-boundary, simple-boundary, octo-pod, goat-processors, network-tap and first-bite. Each needs a group, and each group's rules must already exist, so that joining the machine grants nothing that a reviewed decision did not. Groups are written per scope as `PREFIX-<SUFFIX>` (for the synthetic goatnet `zagros`, `ZAGROS-POD`), from `ops/ansible/netbird_acl_taxonomy.yml`. Their rules are rule intents ([ADR 1129](1129-warrant-scoped-expiring-network-authorizations.md)): the public matrix for `zagros` carries only the substrate rules (`infra → all`, `all → infra` on web, DNS and fabric, `platform-admin → infra`), and a deployment's own intents carry the rules of the classes it uses.

`join-node` reaches the machine over the mesh after enrolment (phases 11 to 14), as the operator's own peer or through `INFRA`. A group that nothing may reach therefore also needs another way in: that is why the network-gear jump box is finished at its declared LAN or out-of-band address (GB-432).

## Decision

1. **A type file names the group its machine joins.** A new key, `mesh_group: <SUFFIX>`, in the type file, one suffix of the taxonomy. `join-node` phase 10 mints the setup key with exactly `PREFIX-<SUFFIX>` for the machine's scope, and with no other group: never the founder peer's groups, never `P2P`. A type without the key keeps today's behaviour: refused before the machine changes (GB-430). Core and phone keep the founder peer's groups. The key changes no rule; it only places the machine.
2. **The group of each type.** Each row follows the decision it names. Where no earlier decision fixes the group, the row gives the options and a recommendation, and the operator rules.

   | Type | Group | Follows | Reached for phases 11 to 14 |
   |---|---|---|---|
   | obs | `INFRA` | ADR 1141 (the obs pair is part of the host site); `standup` phase 12 already puts a founder site's obs hosts in `INFRA` | over the mesh, as core is |
   | federation-boundary | `PEERING-EDGE`, alone | ADR 1219 | `PLATFORM-ADMIN` on tcp 22, ADR 1219 §3 |
   | simple-boundary | open: see 2a | ADR 1253 names no group | depends on the choice |
   | octo-pod | `POD` | ADR 1092, and ADR 1122, which puts the octo-pod in the pod tier | `INFRA` any port and `PLATFORM-ADMIN` through `P2P`, ADR 1092 |
   | goat-processors | `FUSION` | the gear classes of the taxonomy: `FUSION` publishes derived data, not observation, which is what correlation and fusion produce | open: see 2d |
   | network-tap | open: see 2b | no decision names the tap | depends on the choice |
   | first-bite | open: see 2c | no accepted decision names it | depends on the choice |

   **2a. simple-boundary.** The machine sends chosen fabric data out to one partner receiver, outside NetBird (ADR 1253 §3), and serves its boundary monitor on its own mesh address behind `door: group-gated` (ADR 1253 §5), so `INFRA` must reach the monitor's port. Options:
   - (i) `PEERING-EDGE`. It is the other boundary's class, and its outbound shape (the core router's fabric port and the mesh nameserver) is what the sender needs. But nothing on the mesh but `PLATFORM-ADMIN` on tcp 22 may reach it, so the monitor's door could not.
   - (ii) `EDGE-SERVER` (ADR 1216). `INFRA` and `PLATFORM-ADMIN` reach it on any port, so the monitor's door works. But it opens no fabric port, so the sender could not subscribe. It is also delegable, so a site network administrator could enrol a machine that sends data out of the goatnet.
   - (iii) A new non-delegable class, `SIMPLE-BOUNDARY`: `PEERING-EDGE`'s outbound rules, plus `INFRA` reaching it on the monitor's port only, and `PLATFORM-ADMIN` on tcp 22. It needs its own decision and its rule intents.

   **Recommendation: (iii).** A machine whose purpose is to send data out of the goatnet should hold neither a delegable class nor reach it does not need. Until (iii) is decided, the type keeps GB-430.

   **2b. network-tap.** The tap records one Ethernet link and may hand chosen traffic to a decoder; it decodes nothing and publishes nothing on the fabric. Options:
   - (i) `EDGE-SERVER` as it stands (ADR 1216): `INFRA` and `PLATFORM-ADMIN` reach it, and it initiates only to `INFRA` on the management plane. But it is delegable, and a tap holds the recording of every frame on its link.
   - (ii) A new class with `EDGE-SERVER`'s shape that is not delegable, as ADR 1114 and ADR 1131 keep their classes operator-enrolled.

   **Recommendation: (ii).** It has the same reach as (i), and only the operator may enrol a machine that sees a whole link. If the operator accepts delegation, (i) needs no new class.

   **2c. first-bite.** It decodes captured messages against a declared dictionary and hands them to authorized software. An open proposal (ADR 1270, not accepted) would make first-bite the home of capture. Options:
   - (i) `KID`, the taxonomy's appliance that fronts devices that do not speak fabric, with `SENSOR-FEED`'s reach plus its local sensor side: right when first-bite publishes what it decodes onto the fabric.
   - (ii) The tap's class from 2b: right when it only hands decoded messages to software on its own machine or link.

   **Recommendation: (i)** when it publishes onto the fabric, which is how the build-targets design describes the edge kinds. The row waits for ADR 1270's decision on capture, and must not contradict it.

   **2d. goat-processors and phases 11 to 14.** The gear classes' rules are each deployment's intents. `join-node` can finish a `FUSION` machine over the mesh only where `INFRA` or `PLATFORM-ADMIN` reaches it on tcp 22. `infra → all` gives that today wherever the substrate rules apply. A deployment whose intents narrow it declares the machine's LAN address and `reach: declared`, as the jump box does.
3. **Break-glass and the network-gear jump box stay as they are.** `BREAKGLASS` (ADR 1114) and `NETWORK-JUMP` (ADR 1131) are operator-enrolled classes whose rules those decisions fix. `join-node` finishes the jump box at its declared address (`reach: declared`, GB-432). This decision does not change either.
4. **`PEERING-EDGE` is placed, not minted.** ADR 1219 places the class by the membership declaration (`exact: [PREFIX-PEERING-EDGE]`) and never mints it. For federation-boundary, `join-node` phase 10 therefore mints a key with no group, and a peer in no group reaches nothing. Before phase 11, a gate waits until the declaration has placed the machine in `PREFIX-PEERING-EDGE` and in no other group, and refuses if it has not.
5. **Nothing new is granted by joining.** A row is built only once its group's rules exist in the scope's intents. `check-warrant-narrowing.py` and the ACL assertions (ADR 1126) then hold the machine to them. The machine's acceptance rows check it is in exactly its group.

## Alternatives considered

- **Keep refusing every class but core and phone.** Safe, but `join-node` can then build only two of the nine types it lists, and a person enrols the rest by hand, choosing the groups in the dashboard. That hand step is how ADR 1216's one-device-every-port policies arose. Rejected.
- **Give every machine the founder peer's groups (`INFRA`).** It needs no new key. But every edge, boundary and tap machine would then reach every peer on every port, which the classes named above exist to prevent. Rejected.
- **Let the operator pass the groups on the command line.** Flexible, but the group would then be a per-run choice that no reviewed file records, and two runs could place two machines of one type differently. Rejected in favour of the type file.
- **Derive the group from the machine's identity class at the portal.** That is how a person's device is placed. A machine `join-node` builds has no portal enrolment, and ADR 1219 forbids minting `PEERING-EDGE` from an enrolment class. Rejected.

## Consequences

**Positive.**
- Each type's group is recorded once, in its reviewed type file, beside its plays and checks.
- Every row reuses a group whose rules an earlier decision fixed, or waits for one. `join-node` enrols nothing into a group with no rules.
- GB-430 lifts one type at a time, as each row is decided and built.

**Negative.**
- Three rows (simple-boundary, network-tap, first-bite) stay refused until the operator rules. Two of the recommendations need a new class, with its own decision and rule intents.
- A federation boundary's join depends on the membership declaration being applied before phase 11, so a run can wait on another change.
- `obs` in `INFRA` gives an observability host the substrate tier's reach. This matches what `standup` does for the founder's obs hosts today, and a narrower obs class would be its own decision.

## Assessor view (SCA-R)

This is access control by group membership (AC-3, AC-4, AC-6): a new machine's network reach comes from one declared group per type, never from a person's choice at enrolment or from the founder's own groups. Configuration management (CM-2, CM-3) holds the group in a reviewed file, and the existing narrowing check and ACL assertions (CA-7) show that joining grants nothing beyond the group's reviewed rules. The residual risk is a type file that names the wrong suffix. A reviewer catches it in the pull request, and the machine's acceptance rows catch it on the machine, because they check that it is in exactly its group. No waiver is needed.

## Trigger conditions for revisit

- The operator rules on 2a, 2b or 2c, or ADR 1270 is decided: the row is settled, by amendment.
- A deployment's rule intents for `FUSION` or `POD` stop letting `INFRA` or `PLATFORM-ADMIN` reach the machine on tcp 22.
- A type gains a second group, or a machine of one type needs two groups at once.
- An obs class narrower than `INFRA` is proposed.
