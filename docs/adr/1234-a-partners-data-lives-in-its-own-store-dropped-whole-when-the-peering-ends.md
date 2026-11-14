# ADR 1234 — A partner's data lives in its own store, dropped whole when the peering ends

| Field | Value |
|---|---|
| Status | **Accepted**; to be reviewed later, no date set |
| Date | 2026-11-13 |
| Deciders | operator |
| Repo scope | goat core (goatbuilder, the goat-zenoh role) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 1157](1157-cross-sovereign-federation-edge-terminated.md) (cross-sovereign federation), [ADR 1101](1101-zenoh-lazy-per-slot-storage.md) (store retention), [`federation-backout-partner-data.md`](../design/federation-backout-partner-data.md) (the options and the check on the shared store), gate row B7 of [`goatbuilder-v1-gate.md`](../design/goatbuilder-v1-gate.md), GB-227, F-447 |

## TL;DR (for a layperson)

When we federate with a partner, our core keeps the data that partner sends
us. This decision says how that data is kept and how it goes away. Each
partner's data sits in a store of its own. While the partnership runs, the
only thing that deletes it is the same 7-day expiry every partner gets. When
the partnership ends, that partner's store is removed in one piece. We do not
build tools that hunt down and delete one partner's, one key's or one
person's records, because such tools, once started, grow as large as the data
system itself.

## Context

`goatbuilder federate backout <partner>` takes a partner's access rules out
of our core, but until now left the partner's entry in the goat-zenoh role's
`zenoh_sovereign_peerings` list, and with it the stores that list renders: a
store per partner, `sovereign_from_<partner>` on `from/<partner>/**`, and a
shared store, `sovereign_from` on `from/**`, that holds every partner's
samples a second time. Gate row B7 says a back-out removes everything the
command created and nothing else; the stores and their data were the gap.

Two standing rules kept them. GB-227 says a back-out never edits the
deployment's configuration: the role's entry is reported, not removed. F-447
says the role never deletes a store directory that the configuration stops
declaring; it lists it in `.undeclared-stores`.

The design document set out three options: (a) the back-out removes the store
and the data, (b) it keeps them under retention as a named exception to B7,
(c) it deletes the partner's samples by key.

## Decision

Option (a), under this rule:

  1. A partner's data lives in its own store, separated by structure.
  2. While a peering is live, the only deletion is the 7-day retention, the same for everyone.
  3. When a peering ends, the partner's store is dropped whole.
  4. We never build deletion by record, by key or by person. Agreements state what the mechanism does; the mechanism does not bend to each agreement.

The operator's words, recorded with it: "okay let's do this and note that it
warrants a review and consideration at some point but we move out with this
design to get somewhere working for now".

- **To be reviewed.** This decision is to be reviewed and reconsidered later.
  No date is set.
- **The back-out** removes the partner's entry from `zenoh_sovereign_peerings`,
  runs the core play, and deletes `sovereign_from_<partner>` only after the
  router has restarted without that store, behind the back-out's existing
  typed confirmation.
- **The shared `sovereign_from` store leaves the role**, so that rule 1 holds
  and a back-out leaves no residue. Every partner sample in it is also in the
  partner's own store, and nothing reads it (the check is in the design
  document).
- **GB-227 and F-447 change** when the back-out that carries this out lands,
  in that change: the back-out edits the role's list for the partner it backs
  out, and the role deletes that partner's store directory. Until then both
  rules stand as written.
- **A store directory nothing declares** (an entry removed by hand, outside a
  back-out) is reported by the role, as today. No automatic deletion is added
  for it.
- **Option (c) is not built**, now or as a follow-up.

## Alternatives considered

**(b) Keep the stores under retention, as a named exception to B7.** It keeps
an audit copy for the retention window and needs no deletion. Rejected: it
needs a second declaration state for a partner no longer admitted, it does not
meet an agreement that requires deletion at termination, and a re-peering
within the window serves the earlier peering's samples beside the new ones.

**(c) Delete the partner's samples by key.** It is the only option that
removes one partner's samples from a store shared with others. Rejected:
per-customer find-and-cleanse machinery, once started for one contractual
case, grows as large as the data system itself. It also needs a temporary
delete grant in the access rules B7 compares, or an on-host deletion inside
each RocksDB store. With the shared store gone, there is no store shared
between partners for it to clean.

## Consequences

**Positive.** Deletion is a property of the structure: one directory per
partner, removed whole. A back-out compares exactly with the state before the
peering. No per-agreement deletion code exists to maintain or to get wrong.

**Negative.** The back-out edits the deployment's configuration and the role
deletes a directory, both irreversible; the typed confirmation is the only
guard. Removing the store removes the audit copy of what the partner sent;
anything needed later must be exported before the back-out. Our core no
longer keeps a copy of our own outbound wire name, which the shared store held
and nothing read. On a core that has the shared store today, its directory
stays on disk with no retention once the role stops declaring it, and
disposing of it is a person's step. An agreement that needs deletion other
than at the end, or by record, cannot be met by this mechanism; the agreement
has to say so. Retention by `time_to_live` is not measured in this
repository.

## Assessor view (SCA-R)

Partner data is separated by structure (SC-4, information in shared system
resources; AC-4, information flow enforcement through the per-partner access
rules), and its disposal at the end of a peering is the removal of one store
(MP-6 and SI-12, information retention and disposal, at the granularity of a
partner). Retention while a peering runs is one fixed period for every
partner, not set per agreement. The residual risk is that a partner's data
persists up to the retention period after a request to delete one record,
because record-level deletion is excluded; the compensating control is that
agreements state what the mechanism does before a peering starts. The
deletion is irreversible and gated by the back-out's typed confirmation.

## Trigger conditions for revisit

- The review the operator asked for, at a date not yet set.
- An agreement that a partner requires and this mechanism cannot meet, such
  as deletion of named records during a live peering.
- A measurement showing the pinned router does not enforce `time_to_live`, so
  rule 2's retention does not hold.
- Partner data that must be kept past the end of a peering, for audit or a
  legal hold.
