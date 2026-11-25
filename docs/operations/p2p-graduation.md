# P2P graduation — granting (and revoking) clique membership

| Field | Value |
|---|---|
| Status | Manual process on rails. Console-based rails (request → approval → apply) ride the goat-id-console release train — hardening plan §2c. |
| What it grants | Membership in `ZAGROS-P2P`: the peer can **initiate to the other members of that group**, on any port, one-way. Nothing else. |
| Who may grant | A platform admin, via the mgmt API / dashboard. Every grant is a deliberate act — that is the point of the capability group. |

## What P2P means, and what it does not

P2P is **lateral connectivity between the members of the group**. Read the name
literally: members go peer-to-peer *with each other*.

It is **not** a licence to reach the population. A P2P member has no path to a
`DEVELOPER`, `MONITOR` or `OBSERVER` peer, to gear, or to a pod on the strength
of this membership.

## Where reach actually comes from

The point of the contraction is that **administration derives from the role, not
from this group**. If you are reaching for P2P to solve an access problem, one
of these is probably the right answer instead:

| You need to reach | The rule that grants it | Anchored on |
|---|---|---|
| The substrate on any port (SSH, diagnostics) | `platform-admin → infra` | `PLATFORM-ADMIN` |
| The substrate's service ports (web, DNS, zenoh) | rule A, `all → infra` | any role |
| Our gear (sensors, effectors, control) | `platform-admin → gear` | `PLATFORM-ADMIN` |
| Pod appliances at a site | `platform-admin → pod` | `PLATFORM-ADMIN` |
| Externally-hosted services | rule C, `all → external-infra` | any role |
| A participant's machine, for diagnostics | `infra → all` — **run it from the substrate host** | `INFRA` |
| Site data paths between systems | ADR 1124 `gear → gear`, port-scoped | a gear class |
| Other members of the clique | `p2p ↔ p2p` — this document | `P2P` |

The last row of the "reach a participant" case is the one people ask about. It
is deliberate: admin diagnostics toward somebody's laptop runs from an operated,
audited substrate host, not from whichever laptop happens to carry a group. A
`platform-admin → participants` rule is **not** the fix — it would have the same
shape as `platform-admin → opsystem`, which was added and withdrawn on
2026-08-08 because it "handed site admins any-port reach into an apparent
colleague's workstation". Re-adding it against the whole participant population
would be that mistake at larger scale.

## When to graduate a peer — three reasons, and no other

P2P is granted per device for one of three reasons. Each has its own scope
and its own mechanism; a request that names none of them is refused.

| Reason | Scope | What the request names |
|---|---|---|
| **1. Development clique.** The person is past the basics and does device-to-device work with others who are already in the clique | the host goatnet's `ZAGROS-P2P` | the device, and the counterparts it will reach |
| **2. Tactical site continuity.** Nodes at a site must keep reaching each other after WAN loss | the site scope's P2P, never the host's | the site, and the site nodes that form the set; the site's own INFRA node must exist so rule A survives the drop |
| **3. Denied or disconnected autonomous systems.** A gear set must keep communicating with nothing upstream | the site scope's P2P, among that gear set only, and only where the port-scoped `gear → gear` rules (ADR 1124) cannot carry the protocol | the gear set, the protocol, and why it cannot be port-scoped |

Not a reason: reaching a service (rule C, `EXTERNAL-INFRA`), reaching a
colleague's machine for diagnostics (the substrate does that, audited),
"might need it someday", or a device that cannot otherwise reach something
(that is a membership fault in a role group, fixed there).

Graduation also assumes the device's mesh exposure is under control. The owner
confirms the mesh exposure baseline
in the request; for gear, the enrolling admin confirms the class's port set is
what the device actually listens on.

## Grant — request, approve, apply, record

1. **Request.** The owner posts on the work bus: device (peer name and mesh
   IP), reason (1, 2 or 3), the counterparts / site set / gear set, the
   hygiene confirmation, and a review date at most 90 days out.
2. **Approve.** The scope's platform admin replies on the bus. For the host
   goatnet that is an `ZAGROS-PLATFORM-ADMIN` holder; for a site scope, the
   admin named for that scope (ADR 1158). Nobody approves their own device.
3. **Apply.** In an enforcing warrant scope, add the P2P grant to the
   device's reviewed [membership declaration](warrant-runbook.md#4a-membership--which-peer-holds-which-group)
   and include the required expiry. Where direct membership writes are
   authorized, use `PUT /api/groups/<p2p-group-id>` with the **existing list
   plus the new ID** because the PUT replaces the list. Verify that the peer
   retains its role group. This is an additive grant. The console's
   **Move Group** replaces a source membership.
4. **Record.** Add a `group: <SCOPE>-P2P` entry for the peer in
   `ops/ci/membership-declared-deviations.yaml` with `decided_by`,
   `decided_on`, `reason` quoting the request and approval, and
   `revisit_when` set to the review date. Without this entry the daily
   reconcile on the mgmt host reports the membership as undeclared.
5. **Verify both directions**, because they are governed by different rules
   and only one of them changed:
   - *Outbound (what you granted)*: from the graduated peer, reach another
     member (`ping <member-mesh-ip>`). Reaching a non-member must still fail;
     if it succeeds, the clique has been widened somewhere.
   - *Inbound (what you did not grant, and must not have broken)*: from the
     mgmt host, reach the graduated peer. If this fails, the peer has lost or
     never had a role group. Fix the membership, never add a rule.

## Revoke — and the sweep that keeps the clique honest

Remove the peer from `<SCOPE>-P2P` (and `P2P` from its key if present).
Effect is immediate at the WireGuard layer. Reversible-by-default per
ADR 1031: membership is the whole grant; there is nothing else to clean up.
Because reach is role-derived, revoking P2P never strips somebody's ability to
administer gear or the substrate.

Three things trigger a revoke without a request:

- **An undeclared membership.** The daily reconcile report is read weekly. A
  P2P membership with no entry in the deviations file is removed, not
  declared after the fact. The setup key's auto-groups are what the person
  was granted; the dashboard is not a grant.
- **A lapsed review date.** A `revisit_when` that has passed is removed unless
  the owner renews the request. Renewal is the same five steps.
- **A reason that no longer holds.** The counterpart left, the site stood
  down, the gear was re-classed. The admin who approved it removes it.

Record a revoke as `absent: <SCOPE>-P2P` on the peer's deviations entry when
it must stay removed; a hand re-add then prints REVERTED in the reconcile.

## Invariants

- P2P is a **capability**, never a role: no onboarding template mints it; no
  role implies it. If a template ever starts minting `P2P`, that is a regression.
- The rule stays **one-way** (`bidirectional: false`): membership lets you
  initiate toward other members; responses flow on the established connection.
- **P2P is additive. A peer must never hold it as its ONLY group.** A peer whose
  sole group is `P2P` can reach the other members and be reached by *nothing*,
  including the mgmt host nominally responsible for it — `P2P` is a destination
  in no rule but `p2p ↔ p2p` itself. Its role group is what makes it reachable.
  Check with
  `goat warrant why-blocked --scope <env> --from INFRA --to <the peer's group> --port 22`.
- **A one-way path is the designed shape here, not evidence of a missing rule.**
  The fix for an unreachable graduated peer is a **membership** change (restore
  its role group), never a new rule.
- Two assertions in `ops/warrant/rule-intents/netbird-acl.yaml` hold the line:
  one proves a member reaches the clique, one proves it reaches no participant
  outside it. A change that re-widens the rule fails the second.
- Every membership has a request, an approval and a deviations entry with a
  review date. One without all three is removed at the next weekly read.
- Exercises stamp their own `EX-<NAME>-P2P` with the same semantics
  (exercise-group-sets).
