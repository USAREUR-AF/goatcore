# Federating from two drivers: the contract

| Field | Value |
|---|---|
| Status | **Designed.** Nothing in this document is built. It is the product contract goatbuilder 1.0 gate row B10 is proven against |
| Design | [goatbuilder.md](goatbuilder.md) (the federate sections) · [federation-plan.md](federation-plan.md) · [federation-preflight.md](federation-preflight.md) · [ADR 1225](../adr/1225-federation-belongs-to-goatbuilder-one-command-one-decision.md) |
| Audience | whoever builds the two-driver run, writes its scenario, or runs a peering where no one machine reaches both the edge host and the core host |

## What this is

Some sites have no single machine that reaches both their peering edge host and their core
host. They have an **edge driver**, a machine that reaches the edge host, holds no CA key and has
no route to the core; and a **controller**, a machine that reaches the core host and holds the CA
key. Gate row B10 asks that `goatbuilder federate` can be driven from the two, with the plan
approval, the run journal and resume holding across them. This document says how, and what has to
be built for it, because today the command assumes one machine reaches both.

Status: WIP — gate: a scenario with two drivers per sovereign, in which the edge driver holds no
CA key and has no route to the core, passes every required acceptance check (goatbuilder 1.0 gate
row B10).

## Where each phase needs to run, today

The 14 phases are not split at one point between the edge and the core: the core and the CA key
are needed as early as phase 1, and the edge as late as phase 14. Every phase command below is
from `plan_verbs` in `crates/goatbuilder/src/federate/mod.rs`; the plays run with
`--limit <edge group>` unless the table says otherwise.

| Phase | What it does | Reaches the edge | Reaches the core | Needs the CA key | Source |
|---|---|---|---|---|---|
| 1 | preflight | yes: edge ports | yes: core ports, the core router, its receive stores; the slot registry | yes: check `signing-ca` resolves the CA key file and checks it matches our CA certificate (it reads the key and signs nothing) | `ops/federation/standup/preflight_live.py` `CHECKS`; `ops/ansible/peering-preflight.yml` runs on `peering_edges:peering_cores` |
| 2 | the federation interface, and the SPIRE bundle export | yes | yes, when this side runs SPIRE: the export is delegated to `peering_spire_server_host` | no | `ops/ansible/peering-prepare.yml`; `roles/goat-peering-spire/tasks/export.yml` (`delegate_to`) |
| 3 | `init`, our half measured | yes: the interface, transports | yes: the core router, the core's SPIRE trust domain | no; the package is signed with the signing key, resolved where the run is | `crates/goatbuilder/src/federate/measure.rs` module doc |
| 4 | `lint` | no | no | no | local |
| 5 | the exchange through the directory, and the key read-back | no | no | no; the signing key signs the pointer | `ops/federation/standup/directory.py` |
| 6 | `compose` | no | no | no | local |
| 7 | `verify` | no | no | no | `crates/goat-federation/src/verbs/verify.rs`: reads local state |
| 8 | mint the peering leaves | no | no | **yes** | `ops/federation/standup/mint-peering-leaves.sh`; `peering_leaf_material.py` makes each leaf's key and signs it in one place |
| 9 | `apply`, the rendered files | no | no | no | local |
| 10 | admission on our core | no | yes | no | `crates/goatbuilder/src/federate/admit.rs` |
| 11 | the transport | yes | no | no | `peering-wg.yml` or `peering-edge.yml` |
| 12 | the edge router, then the republisher | yes: `peering-zenoh-edge.yml` | yes: `peering-republisher.yml` with `--limit <with>_core` | no | `plan_verbs`, phase 12 |
| 13 | SPIRE federation | yes | yes: tasks delegated to `peering_spire_server_host` | no | `roles/goat-peering-spire/tasks/main.yml` (`delegate_to`) |
| 14 | acceptance | yes: the edge's log for the refusal checks | yes: our core's REST surface | no | `ops/federation/standup/acceptance.sh` |

Two consequences shape the contract:

- **No single hand-off point works.** An edge driver cannot run phases 1 to 7 and hand over at 8:
  phase 1 needs the core and the CA key, and phases 2 and 3 need the core. The edge's work
  continues after phase 8, in phases 11 to 14.
- **Phase 8 makes the edge's private keys on the controller.** `peering_leaf_material.py` creates
  each leaf's key and certificate request and signs it with the CA key, in one process. When the
  edge group has `goat_zenoh_edge_install_certs` on (allocate renders it on for its groups; the
  role's default is off), phase 12 installs the edge's leaves from `<work>/pki` on the machine
  running the play; with it off, the leaves are placed on the edge by hand. A two-driver run
  must move those keys to the edge driver, or have the edge make its own keys and send only a
  signing request.

## Where state lives, today

- **The journal** is `<work>/state.json` on the machine that runs the command, under the lock
  `<work>/.lock` (`federate/state.rs`, `federate/lock.rs`); host-level allocation takes a second
  lock, `<root>/.allocation.lock`. `<work>` defaults to
  `/var/lib/goat-federation/<partner>`.
- **The plan** is `<work>/plan.json`. A plan is accepted only from its own working directory:
  `check_binding` refuses a plan whose directory is not the `work` it records (GB-201,
  `federate/plan.rs`). Before deploy changes anything, two checks run, each refusing with GB-188:
  the plan's inputs are derived again on the machine running deploy, from its files and
  declarations and without reading any host, and compared with the plan (`deploy::inputs_hold`);
  and the live prerequisites are checked again on the hosts (`plan::recheck_live_prerequisites`,
  called from `deploy`). A plan made on one machine and deployed from another is refused when the
  two derive different inputs.
- **The approvals** (the keys, the agreement digest, the plan's hash, the admission digest) are
  confirmations in that machine's `state.json`, each bound to what it approved
  (`federate/state.rs`, `confirm`).
- **`--from N`** runs phases N to the end, skips nothing, and, when the journal's last word on
  phase N−1 is a by-hand stop with unchanged inputs, records that phase as done by the operator
  (`federate/run.rs`, `Run::begin`). Resume is on by default (`--no-resume` turns it off) and applies
  only to a run from phase 1 or one given `--only`: it skips a phase this journal holds passed with
  unchanged inputs (the same function). With `--from N` above 1 and no `--only`, nothing is skipped,
  and the run says so.
- **`status`** reads the journal and the evidence in the working directory of each declared
  peering on the machine it runs on (`federate/status.rs`). It merges nothing from another machine.

## The contract

**The controller is the run of record.** It holds the CA key, the plan, every approval and the
journal. The edge driver holds no approval: it runs only the edge-side work the controller hands
it, and what it produces comes back to the controller before the run goes on.

1. **Which phases each driver runs.** Every phase is marked with the side it needs: *edge*,
   *core* (which includes the CA key), *both* or *either*, as in the table above. The controller
   runs every *core* and *either* phase, and the core half of each *both* phase. The edge driver
   runs the edge half of each *both* phase (1, 2, 3, 12, 13, 14) and every *edge* phase (11).
2. **At the first phase that needs the other side,** the driver stops before acting, with a
   named hand-off and a catalog code. The stop says which phase, what the other machine runs,
   and the exact command, for example: "phase 3 needs the edge: on the edge driver run
   `goatbuilder federate <partner> --side edge --from 3 --to 3`, then bring back its result and run
   `goatbuilder federate <partner> --from 3` here". An edge driver that reaches a *core* phase stops
   the same way, naming the controller's command.
3. **The controller carries on with `--from N`.** `--from N` is the existing way to say "the phase
   before this one was done elsewhere": the controller's journal records the edge's part of phase
   N−1 as an operator step, bound to the result the edge driver produced. Resume, which is on by
   default for a run from phase 1, skips only what this machine's own journal holds; it stays a
   one-machine convenience and does not see the other machine's work.
4. **What moves between the machines.** From the edge driver to the controller: the edge half of
   each *both* phase's result (phase 1's edge checks, phase 2's interface and key, phase 3's edge
   measurement, phase 14's edge log), as files under `<work>` with their digests. From the
   controller to the edge driver: the agreement, the rendered edge files, and the edge's leaves.
   The edge's private keys are made on the edge driver, which sends a signing request; the
   controller signs it in phase 8 and returns only certificates. No private key crosses between
   the machines.
5. **The plan and its approval** stay on the controller, in its `<work>`, where `check_binding`
   and `inputs_hold` already expect them. They are not committed to the checkout: the approval is
   bound to inputs derived on the machine that deploys, which is the controller.
6. **`status`** on the controller shows the whole peering: its own journal, and for each edge
   result it imported, the phase, the time and the digest. On the edge driver it shows what that
   driver ran and the hand-off it is waiting on.

## Window 6's proposal, against the code

| Proposal | What the code says | In the contract |
|---|---|---|
| The edge driver stops at phase 8, the CA key, saying "continue on the controller: `goatbuilder federate <partner> --from 8`" | Phase 1 already needs the core and the CA key (`signing-ca`), and phases 2 and 3 need the core | Hand-offs at every phase that needs the other side, not one at phase 8 |
| The plan and its approval digest are committed to the checkout both machines pull | A plan is accepted only from its own `<work>` (GB-201), and its inputs are rechecked on the deploying machine (GB-188) | The plan and approvals stay on the controller, the only machine that deploys |
| The journal is per machine | It is, today: `<work>/state.json` | Kept |
| `status` merges the two | It reads one machine's working directories | The controller's `status` shows the edge results it imported; no merge of two journals |

## What has to be built

- Each phase's side, in the phase list, and a `--side edge` mode that runs only the edge half of
  a *both* phase and only *edge* phases, refusing anything else with a hand-off.
- The halves of phases 1, 2, 3, 12, 13 and 14 as separate steps: preflight's checks selected by
  side (`preflight.py --only` exists and selects by check); phase 3 measuring the edge and the core
  separately and joining the two before signing. As built, phase 3's over-SSH measurement reaches
  the edge and the core from one machine in one call (`federate/measure.rs`, `over_ssh`), and it is
  the default (`FED_MEASURE=here` measures on the host instead, which reaches the core through
  `DOCKER_HOST`), so phase 3 cannot run on either driver alone until its two halves are separate:
  the *both* marking of phase 3 depends on that split; the SPIRE tasks that today delegate from the edge
  play to the SPIRE server host, run from the controller.
- Leaf issuance from a signing request: the edge's keys made on the edge driver, signed in
  phase 8.
- The carrier for an edge result: a file the edge driver writes and the controller imports,
  with its digest, and the journal entry recording it.
- The hand-off stops, each with a new catalog code and the next command.
- `status` showing imported edge results.

## The scenario that proves it

Two sovereigns, each with two drivers. On each side the edge driver holds no CA key and has no
route to the core host (enforced in the test environment by the network, not by convention), and
the controller has no route to the edge host. Each side follows the hand-offs from phase 1 to
phase 14. The scenario passes when:

- every required acceptance check passes on both sides;
- no private key is present on a machine other than the one that made it;
- the controller's journal holds every phase, with each edge result's digest;
- a run killed during an edge half and started again with `--from` on the same machine finishes;
- `status` on each controller shows the peering at phase 14 with its evidence.
