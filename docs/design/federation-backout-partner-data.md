# What a back-out does with a partner's stored data

| Field | Value |
|---|---|
| Status | **Decided, 13 November 2026: option (a)** ([ADR 1234](../adr/1234-a-partners-data-lives-in-its-own-store-dropped-whole-when-the-peering-ends.md)), under the rule in "The decision". To be reviewed later; no date is set. The back-out that carries it out is built on a goat-zenoh role core and not measured; "What happened before the build" is the behavior before it
| Decides | What `goatbuilder federate backout <partner>` does with the core's receive stores for that partner, above all when it is the last peering |
| Gate row | B7 of the goatbuilder 1.0 gate: "Backing a peering out removes everything the command created and nothing else", proven by the back-out scenario and a comparison of both hosts with their state before the peering |
| Related | [sovereign-federation-runbook.md](../operations/sovereign-federation-runbook.md), Step 9 · [federation-package-spec.md](federation-package-spec.md), "Receiving storage interests" · [ADR 1157](../adr/1157-cross-sovereign-federation-edge-terminated.md) |

A partner's data is the one class of data on our core that we did not
generate. Admission makes the core store it; a back-out today does not remove
that store or the data in it. Gate row B7 says a back-out removes everything
the command created and nothing else. This paper set out three ways to make
the back-out and that row agree, and what each costs. Option (a) was chosen.

## The decision

**Plain:** a partner's data is kept in a store of its own. While the peering
runs, the only thing that deletes it is the 7-day retention every store has.
When the peering ends, the back-out drops that store whole. We never delete
one partner's records one by one.

The rule:

  1. A partner's data lives in its own store, separated by structure.
  2. While a peering is live, the only deletion is the 7-day retention, the same for everyone.
  3. When a peering ends, the partner's store is dropped whole.
  4. We never build deletion by record, by key or by person. Agreements state what the mechanism does; the mechanism does not bend to each agreement.

The operator's words, recorded with it: "okay let's do this and note that it
warrants a review and consideration at some point but we move out with this
design to get somewhere working for now".

- **To be reviewed.** The decision is to be reviewed and reconsidered later.
  No date is set.
- **Why.** Machinery that finds and cleanses one customer's data, once it is
  started for one contractual case, grows until it is as large as the data
  system itself. A store per partner, dropped whole, keeps deletion a property
  of the structure instead.
- **Option (c) is not built**, now or as a follow-up. Deleting a partner's
  samples by key is rule 4's deletion by key.
- **What the back-out does** (built, unit-tested, not measured): on a
  goat-zenoh role core with no route of its own, behind the back-out's typed
  confirmation and after the core step, it takes the partner's entry out of
  `zenoh_sovereign_peerings` in `group_vars/<core router group>/peerings.yml`
  (`admit_role::remove_peering`), and runs the role's play, which restarts the
  router on the configuration without `sovereign_from_<partner>`. Only then
  does a play of its own, `ops/ansible/zenoh-drop-partner-store.yml`, delete
  that directory (`roles/goat-zenoh/tasks/drop-partner-store.yml`). GB-227 ("a
  back-out never edits configuration") and the F-447 rule that the role never
  deletes a store directory changed with it: GB-227 now names an entry the
  back-out did not write or could not take out, and the role deletes the one
  store it is told to drop. A core admitted through the deployment's own route
  (`FED_CORE_*`) keeps its role entry, reported as GB-227.
- **A store directory nothing declares** (an entry removed by hand, outside a
  back-out) has no retention. The role reports it, as it does today
  (`.undeclared-stores`); no automatic deletion is added for it.

## Can the shared `sovereign_from` store go?

**Plain:** yes. Every partner's samples in the shared store are also in that
partner's own store, and nothing reads the shared one. Removing it from the
role is what makes rule 1 hold: today each partner's samples are kept twice,
once mixed with every other partner's.

Checked against `main` at 2133f4d4 by reading the role and rendering its
template; not measured on a running core.

- **Every write under `from/` on a role core is one of two kinds.** Rendering
  `config.json5.j2` with the role's own test helper
  (`ops/ansible/roles/goat-zenoh/tests/render-check.py`, `render`) for two
  partners, `xaa` with an outbound republisher and `xab` without, with and
  without `goat_zenoh_legacy_system_namespace`, and scanning all 50 rules,
  the only rules that allow an ingress `put` on a key under `from/` (or on
  `**`) are `sovereign-<partner>-w` on `from/<partner>/**`, one per partner,
  and `sovereign-<partner>-rp-w` on our own wire name (`from/<us>/<set>/**`,
  the republisher, the only writer of that name). The access rules deny by
  default.
- **So the shared store holds nothing of a partner's that its own store does
  not.** A sample from a partner is on `from/<partner>/**`, which
  `sovereign_from_<partner>` covers. The role has rendered a store per
  partner since 29c6ec8f (#235, 9 November 2026). A sample a core received
  before it first ran that role is in the shared store alone, until the
  deployment's retention (`zenoh_from_retention_days`, 7 days by default) has
  passed since then. That retention is not measured here (see "Retention").
- **The shared store also holds our own outbound wire name**, `from/<us>/**`,
  because `from/**` covers it. Nothing in this repository reads that copy: the
  federation test environment's checks read what the receiving core stored
  under the sender's name (`stored_at` in `ops/federation-e2e/units/pair.sh`).
  After the removal, a query that reaches our core for our own wire name is
  answered by no store on our core. A record of what we sent, if one is
  wanted, is a separate decision.
- **Nothing else names the shared store.** Outside the template, only the
  role's render test (`render-check.py`, "existing store path retained") and
  the storage-restart test's copy of the stores
  (`tests/zenoh-core-storage-restart.sh`) name `sovereign_from`; both change
  with the removal. The preflight and the edge's ingress check already treat a
  `from/**` store as insufficient on its own (GB-136).
- **What the removal leaves on a core that has the store today.** The
  directory `<storage path>/sovereign_from` stays on disk, with no retention.
  The role lists it in `.undeclared-stores` and never deletes it; disposing of
  it is a person's step, named in the runbook. A core built after the removal,
  such as the federation test environment's, never has it, so a back-out there
  leaves no residue.

The role no longer renders the shared store (built, `roles/goat-zenoh/tests/render-check.py`).

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
    `<storage path>/sovereign_from` (store at line 335, its directory at line 339). Its comment says partner data
    "MUST persist here" so that what a partner sent can be audited, and that
    partner data "may be obliged to delete on request or on peering
    termination" (lines 326 and 332).
  - **`sovereign_from_<partner>`**, one per partner, on `from/<partner>/**`, in
    `<storage path>/sovereign_from_<partner>` (store at line 350, its directory at line 354). A storage interest on
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

## What happened before the build

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

As decided, the shared `sovereign_from` is removed from the role instead (see
"Can the shared `sovereign_from` store go?"), so no back-out has a shared
store to deal with, and the partner's samples are only in its own store.

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
before the role entry is removed: either through the router, or on the host
inside each RocksDB store. That removes them from
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
| What it costs | A temporary delete grant, or an on-host deletion. No grant exists today: the core's access rules deny by default (`config.json5.j2`, line 586), and the only ingress put or delete on `from/<partner>/**` is `sovereign-<partner>-w` (lines 597 to 601), the partner's own edge, which the back-out removes. The operator subject (`operator-trial-peer`, line 1621) may write only `ops/**`, and `system/**` where the legacy namespace is on, through `operator-w-ops` (lines 1114 to 1119). A deletion through the router therefore needs a temporary operator rule allowing ingress delete on `from/<partner>/**`, which changes the access rules B7 compares. A deletion on the host inside each RocksDB store needs no grant. Whether the pinned router applies a wildcard delete to every stored key under it is not checked. A deletion leaves tombstones in each store until they are collected. The per-partner directory is still deleted by the role, so the F-447 change in (a) applies to it |

## Across the options

| | (a) remove | (b) retain | (c) delete by key |
|---|---|---|---|
| Partner's data after back-out | gone, except in the shared store when other peerings remain | kept until retention passes | gone |
| Audit copy | gone | kept for the retention window | gone |
| Re-peering sees old data | no | yes, within the window | no |
| B7 comparison | exact | with a named exception | with a named exception while the shared store stays |
| Changes to today's rules | back-out edits configuration; role deletes a directory | new declaration state; template change | a temporary delete grant or an on-host deletion; role deletes a directory |

## The question, as it was put

When the last peering is backed out, should the back-out remove the partner
store and its data (a), keep both under the retention policy as a named
exception to B7 (b), or delete the partner's data by key and keep only what
other peerings still need (c)?

Answered: (a), under the rule in "The decision".
