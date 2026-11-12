# What a back-out does with a partner's stored data

| Field | Value |
|---|---|
| Status | **Open decision.** The options are designed; none is built. Today's behavior is stated under "What happens today" |
| Decides | What `goatbuilder federate backout <partner>` does with the core's receive stores for that partner, above all when it is the last peering |
| Gate row | B7 of the goatbuilder 1.0 gate: "Backing a peering out removes everything the command created and nothing else", proven by the back-out scenario and a comparison of both hosts with their state before the peering |
| Related | [sovereign-federation-runbook.md](../operations/sovereign-federation-runbook.md), Step 9 · [federation-package-spec.md](federation-package-spec.md), "Receiving storage interests" · [ADR 1157](../adr/1157-cross-sovereign-federation-edge-terminated.md) |

A partner's data is the one class of data on our core that we did not
generate. Admission makes the core store it; a back-out today does not remove
that store or the data in it. Gate row B7 says a back-out removes everything
the command created and nothing else. This paper sets out three ways to make
the back-out and that row agree, and what each costs.

## Where the store comes from

**Plain:** admitting a partner on a core that the goat-zenoh role configures
adds that partner to a list, and the list makes the core render storage for
data received from partners.

- Phase 10 writes the partner's entry into the role's `zenoh_sovereign_peerings`
  (`merge_peerings` in `crates/goatbuilder/src/federate/admit_role.rs`, line 360,
  into the file `peerings_file` names, line 349).
- While that list is not empty, the core's configuration template renders two
  kinds of store (`ops/ansible/roles/goat-zenoh/templates/config.json5.j2`,
  lines 324 to 363):
  - **`sovereign_from`**, one store on `from/**`, shared by every partner, in
    `<storage path>/sovereign_from` (line 335). Its comment says partner data
    "MUST persist here" so that what a partner sent can be audited, and that
    partner data "may be obliged to delete on request or on peering
    termination" (lines 326 and 332).
  - **`sovereign_from_<partner>`**, one per partner, on `from/<partner>/**`, in
    `<storage path>/sovereign_from_<partner>` (line 350). A storage interest on
    `from/**` does not fit an edge grant on `from/<partner>/**`, so each
    partner needs its own ([federation-package-spec.md](federation-package-spec.md),
    "Receiving storage interests").
- Both are RocksDB volumes with `on_closure: "do_nothing"` and
  `create_db: true`. A sample from a partner is written to both, because the
  router writes a sample to every storage whose key covers it.

**Retention.** Both stores set `time_to_live` to `zenoh_from_retention_days`
days in milliseconds (lines 345 and 360), with `garbage_collect_period: 86400`
(once a day). The default is 7 days
(`ops/ansible/roles/goat-zenoh/defaults/main.yml`, line 648), kept separate from
the fabric's own retention because of the deletion obligation above. Retention
is a property of a **configured** store: a directory that the configuration
no longer declares has no `time_to_live` applied to it. This repository has no
measurement of the pinned router dropping samples by `time_to_live`; the
options below that rely on it inherit that gap.

## What happens today

**Plain:** a back-out takes the partner's access rules out of the core, but the
partner's entry in the role's list and both stores stay. Removing the entry by
hand later leaves the data on disk with no retention.

1. `federate backout` removes the partner's subjects, rules and policies from
   the core's spliced org fragment and restarts the router
   (`ops/ansible/roles/goat-zenoh/tasks/federation-absent.yml`). The fragment
   carries no store: its storages region is empty
   (`crates/goat-federation/src/render.rs`, line 876).
2. The role's entry for the partner is reported and kept: "that is the
   deployment's configuration and is reported, never edited"
   (`federation-absent.yml`, lines 18 to 20 and 104). The back-out ends with
   GB-227, "the core still admits the partner through its own configuration"
   (`ops/errors/catalog.yaml`). The federation test environment found the
   store still configured after a back-out for this reason.
3. While the entry stays, both stores stay configured. They receive nothing
   more, because the partner's edge is gone, and their samples age out under
   retention.
4. When a person removes the entry and runs the core play:
   - If other peerings remain, `sovereign_from` stays, and the partner's
     samples in it age out under retention. `sovereign_from_<partner>` leaves
     the configuration.
   - If it was the last peering, both stores leave the configuration.
   - A directory that leaves the configuration stays on disk. The role lists
     it in `<storage path>/.undeclared-stores` and never deletes it (F-447,
     `defaults/main.yml` lines 650 to 659; `tasks/main.yml` from line 677).
     It has no retention, so its samples are kept until a person disposes of
     them. With `zenoh_orphan_stores_fail: true`, the next deploy refuses
     until that is done.

The runbook's Step 9 item 4 describes the same end state and leaves disposal
to "the agreed retention/deletion procedure"
([sovereign-federation-runbook.md](../operations/sovereign-federation-runbook.md),
Step 9).

## The options

Each option is described for the last peering, where the shared store is at
stake, and for a back-out that leaves other peerings in place.

### (a) The back-out removes the store and the data

The back-out removes the partner's role entry and runs the core play. It then
deletes `sovereign_from_<partner>`. On the last peering it also deletes
`sovereign_from`. A directory is deleted only after the router has restarted
without the store.

| | |
|---|---|
| Data obligations | Meets an obligation to delete on termination at once. Removes the audit copy of what the partner sent; anything needed later has to be exported first. On a back-out that leaves other peerings, the partner's samples in the shared `sovereign_from` remain until retention removes them, because that store holds every partner's data |
| A later re-peering | Starts with empty stores |
| The lab's check | A plain comparison with the state before the peering: no store in the configuration, no `sovereign_from*` directory, no `.undeclared-stores` entry |
| What it costs | Reverses two rules of today: the back-out edits the deployment's configuration, which GB-227 says it does not, and the role deletes a store directory, which F-447 says it never does. The deletion is bound to one partner's name and comes after the back-out's typed confirmation. It is irreversible |

### (b) The back-out keeps the store and the data under retention, as a named exception

The back-out stops admitting the partner, but the stores stay configured until
their retention has passed, so the data ages out and is not orphaned. The
role's list would need a second state for a partner that is no longer admitted
but whose data is retained, with no subject and no grant, and the template
would render its stores from that state.

| | |
|---|---|
| Data obligations | Keeps the partner's data for up to `zenoh_from_retention_days` after termination, and keeps the audit copy for that long. An agreement that requires deletion at termination is not met without a separate deletion step |
| A later re-peering | Within the retention window, the per-partner store reopens its existing directory (`create_db: true`), so samples from the earlier peering are served beside the new ones, under what may be a different agreement |
| The lab's check | B7's comparison with a named exception: the two store blocks and the two directories may differ from the state before the peering. The check then asserts the stores are configured with the declared retention and that no subject or grant for the partner remains. Expiry cannot be observed in a run: retention is whole days, with a minimum of one |
| What it costs | A template and declaration change, and a later step that removes the stores once the window has passed. Relies on `time_to_live` being enforced, which is not measured here |

### (c) The back-out keeps what other peerings need and deletes only this partner's data

The back-out deletes the partner's samples by key, `from/<partner>/**`,
through the router before the role entry is removed. That removes them from
both `sovereign_from` and `sovereign_from_<partner>`, and leaves other
partners' data, which lives under other keys, untouched. Then the per-partner
store and its directory are removed. The shared `sovereign_from` stays
configured while other peerings remain; on the last peering it stays as an
empty store, or is removed as in (a).

| | |
|---|---|
| Data obligations | Meets an obligation to delete on termination for this partner on any back-out, including one that leaves other peerings. Removes the audit copy, as in (a) |
| A later re-peering | Starts with no data from the earlier peering |
| The lab's check | B7's comparison, with the shared store's block as a named exception while it stays; plus a query on `from/<partner>/**` on our core that returns nothing |
| What it costs | A deletion through the router needs an operator identity allowed to delete under `from/<partner>/**` on our own core; whether the core's access rules grant one is not established here. A deletion leaves tombstones in each store until they are collected. The per-partner directory is still deleted by the role, so the F-447 change in (a) applies to it |

## Across the options

| | (a) remove | (b) retain | (c) delete by key |
|---|---|---|---|
| Partner's data after back-out | gone, except in the shared store when other peerings remain | kept until retention passes | gone |
| Audit copy | gone | kept for the retention window | gone |
| Re-peering sees old data | no | yes, within the window | no |
| B7 comparison | exact | with a named exception | with a named exception while the shared store stays |
| Changes to today's rules | back-out edits configuration; role deletes a directory | new declaration state; template change | new delete grant; role deletes a directory |

## The question

When the last peering is backed out, should the back-out remove the partner
store and its data (a), keep both under the retention policy as a named
exception to B7 (b), or delete the partner's data by key and keep only what
other peerings still need (c)?
