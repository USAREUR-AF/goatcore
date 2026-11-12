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
   `goatbuilder federate <partner> --side edge --from 3 --to 3`, bring its result file here,
   import it, then run `goatbuilder federate <partner> --from 3` here". An edge driver that reaches
   a *core* phase stops the same way, naming the controller's command.
3. **The controller imports the edge result, then carries on with `--from N`.** For a *both*
   phase N, the controller first imports the file the edge driver wrote for phase N's edge half.
   The import checks its digest and records it in the controller's journal as an operator step on
   phase N, bound to that digest. `goatbuilder federate <partner> --from N` then runs phase N's core
   half, which reads the imported edge result, and the phases after it. `--from N` alone does not
   carry the hand-off: as built it records phase N−1 as done only when N−1 is a by-hand phase (5,
   10, or phase 3's re-exchange) whose last journal record is its stop (`federate/run.rs`, the
   module comment). In the example, `--from 3` without the import would record nothing about the
   edge's phase 3 and would treat phase 2 as the phase before. Resume, which is on by default for a
   run from phase 1, skips only what this machine's own journal holds; it stays a one-machine
   convenience and does not see the other machine's work.
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

## The hand-off object

**Plain:** every hand-off stop is also printed as one JSON object, so a script
or a test reads the next command instead of parsing a sentence. The object
names the phase, the machine that acts next, the exact command to run there,
the file that command writes, and the two commands the controller runs after
it. The sentence a person reads names the same commands.

### Where it appears

- **At the stop.** With `-o json`, a run that stops at a hand-off prints its
  report with one added member, `handoff`. The stop exits 0, as a by-hand stop
  does today (GB-192).
- **While the run waits.** `goatbuilder federate status <partner> -o json`
  shows the same object for that peering until the next run of the command on
  that machine. The run writes it to the journal as the phase's last record,
  and `status` reads it from there.
- **At the end of an edge run.** The edge driver's `--side edge` run prints
  the object for the way back (`run_on: "controller"`), with the result it
  wrote and its confirm code.
- **In the text.** The person's text prints each argv of the object, quoted
  for a POSIX shell, on its own line. A test renders each argv the same way
  and checks that the text holds it.

Both reports are additions to existing documents (`goatbuilder.output/v1` and
`goatbuilder.federate.status/v1`), so neither needs a new schema version.

### The members

```json
{"handoff": {
  "phase": 3,
  "code": "GB-NNN",
  "run_on": "edge",
  "command": ["goatbuilder", "federate", "xab", "--side", "edge", "--from", "3", "--to", "3"],
  "carry": [],
  "result": "handoff/3/edge-half.json",
  "import": ["goatbuilder", "federate", "import", "xab", "3", "{result}", "--sha256", "{code}"],
  "then": ["goatbuilder", "federate", "xab", "--from", "3"]
}}
```

| Member | Meaning |
|---|---|
| `phase` | the phase the hand-off belongs to |
| `code` | one new catalog code, "the run is waiting on the other driver", the same for every hand-off; its number is assigned when it is built |
| `run_on` | `edge` or `controller`: the machine that acts next |
| `command` | what to run on `run_on` now, as an argv. `null` in the edge's way-back object, where nothing more runs on the edge |
| `carry` | the files the controller produced that must be on the edge driver before `command` runs, each `{"file": "<path under <work>>", "sha256": "<digest>"}`; `[]` when there are none. `command` names each one with `--expect <file>=<sha256>`, so the edge driver refuses a file changed in transit |
| `result` | the file `command` writes, as a path under the writer's `<work>`. Each driver has its own `<work>` (`--work`, else `$FED_WORK`, else `/var/lib/goat-federation/<partner>`), so the controller cannot name the edge's absolute path; the edge's way-back object adds `result_path`, the absolute path it wrote, and `result_sha256` |
| `import` | what the controller runs to take the result in, as an argv |
| `then` | what the controller runs after the import, as an argv: `--from N` for a *both* phase, which runs phase N's core half against the imported result (contract step 3) |

Every argv element is literal except two tokens, each standing for a whole
element:

- `{result}`: where the operator put the result file on the controller;
- `{code}`: the confirm code.

The controller's own objects (the stop and `status`) carry both tokens, because
the controller knows neither. The edge's way-back object fills `{code}` and
keeps `{result}`.

### The confirm code

**Plain:** the confirm code reaches the controller only through a person, or a
test, reading the edge driver's own output. It is never in the file and never
in anything that travels with the file.

- The edge run prints the confirm code: the first 32 hex digits of the
  result's sha256, in groups of four. In its JSON the code is the member
  `confirm_code` of the way-back object, and it is filled into `import`. That
  JSON is the edge driver's screen, in another form.
- The code never appears in `result`, in the result file, or in `carry`. A
  code carried beside a changed file would confirm the change.
- `import` refuses a file whose sha256 does not begin with the code given, and
  records the full sha256 in the controller's journal as an `operator-step`
  confirmation on phase N. `then` uses that recorded digest; the operator does
  not type it again.

A scenario that reads the code from the edge driver's JSON is reading the edge
driver's own output, which is exactly what the binding rests on. It must not
read the code from the controller, from the result file, or from anything
copied with it.

### Phase 3, the hand-off already built

Phase 3's two halves are built at the `init` level, in review: `goatbuilder
federate init --over-ssh <peering> --side edge --half-out <file>` writes the
edge half (schema `goatbuilder.phase3-edge-half/v1`) and prints its sha256, its
confirm code and the line `goatbuilder federate init --over-ssh <peering> --side
core --edge-half <file> --edge-half-sha256 <code>`. The core half refuses a
changed or stale half with GB-114 ([federation-router-discovery.md](federation-router-discovery.md),
once that change is on main).

The hand-off object for phase 3 is the one command wrapped around those two
halves:

- **`command`** (`--side edge --from 3 --to 3`) runs exactly the built edge
  half, `init --over-ssh xab --side edge --half-out <work>/handoff/3/edge-half.json`,
  with the init flags the plan gives. The file, its schema and its confirm code
  are the built ones.
- **`import`** checks the file with the built core half's rules (the confirm
  code, the schema, the peering, the edge host, the 24-hour window, the
  interface; GB-114), copies it to the controller's
  `<work>/handoff/3/edge-half.json`, and journals its full sha256.
- **`then`** (`--from 3`) runs exactly the built core half: `init --over-ssh xab
  --side core --edge-half <work>/handoff/3/edge-half.json --edge-half-sha256
  <the journaled sha256>`, with the plan's init flags. It joins, builds and
  signs the package as the built core half does.

What changes in the built code is one thing: when the edge half runs inside
the one command, its last line names the one command's `import` and `then`
instead of the `init --side core` line, and it prints the way-back object with
`-o json`. `init` run on its own keeps printing what it prints today.

### Each hand-off, filled in

For a peering `xab` seen from `xaa`, with default working directories. Every
`import` is `["goatbuilder", "federate", "import", "xab", "<N>", "{result}",
"--sha256", "{code}"]` with `<N>` the phase. `carry` lists the files the
controller made that the edge's step reads.

| Phase | `run_on` | `command` | `carry` | `result` | `then` |
|---|---|---|---|---|---|
| 1 | edge | `["goatbuilder","federate","xab","--side","edge","--from","1","--to","1"]`: preflight's edge checks only | `[]` | `handoff/1/edge.json` (the edge checks' results) | `["goatbuilder","federate","xab","--from","1"]`: the core checks and `signing-ca`, then the join |
| 2 | edge | `["goatbuilder","federate","xab","--side","edge","--from","2","--to","2"]`: the federation interface and its key; the edge's leaf keys and their signing requests | `[]` | `handoff/2/edge.json` (the interface's public key, and one signing request per edge leaf; no private key) | `["goatbuilder","federate","xab","--from","2"]`: the SPIRE bundle export on the SPIRE server host |
| 3 | edge | `["goatbuilder","federate","xab","--side","edge","--from","3","--to","3"]`: the built edge half | `[]` | `handoff/3/edge-half.json` (`goatbuilder.phase3-edge-half/v1`) | `["goatbuilder","federate","xab","--from","3"]`: the built core half, then phases 4 to 10 on the controller |
| 11 | edge | `["goatbuilder","federate","xab","--side","edge","--from","11","--to","11","--expect","agreement.json=<sha256>","--expect","applied/transport.json=<sha256>"]`: the transport | the agreement and the rendered transport files | `handoff/11/edge.json` (the interface's state and its peer) | `["goatbuilder","federate","xab","--from","12"]`: phase 11 has no core half, so the controller goes on to 12, which stops for 12's edge half |
| 12 | edge | `["goatbuilder","federate","xab","--side","edge","--from","12","--to","12","--expect","pki/outward.pem=<sha256>", …]`: the edge router, with the leaves phase 8 signed from phase 2's requests | the edge's certificates from phase 8 and the rendered edge router files | `handoff/12/edge.json` (the edge router's running state and its configuration digest) | `["goatbuilder","federate","xab","--from","12"]`: the republisher on the core |
| 13 | edge | `["goatbuilder","federate","xab","--side","edge","--from","13","--to","13"]`: the edge's SPIRE federation tasks | `[]` | `handoff/13/edge.json` | `["goatbuilder","federate","xab","--from","13"]`: the tasks delegated to the SPIRE server host |
| 14 | edge | `["goatbuilder","federate","xab","--side","edge","--from","14","--to","14","--window","<start>/<end>"]`: the edge's log read for the refusal checks | `[]` | `handoff/14/edge.json` (the edge's log lines in the window) | `["goatbuilder","federate","test","xab"]`: joins the edge's lines with the core half's result and decides |

Phase 14 is the one hand-off that comes after the controller has acted. The
refusal checks need the edge's log for the time the core half fired its
samples, so the controller runs its half first, records the window, and then
stops with the object above, whose `command` names that window. Its `then` is
`test`, the stage that runs phase 14 alone. Finding the imported edge lines for
the window its own core half recorded, it joins the two and fires nothing
again.

One hand-off covers one phase. Batching consecutive edge halves (1 to 3) into
one is not designed.

## A single hand-off at phase 8, against the code

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
- The import step on the controller: it checks the edge result's digest and records an
  `operator-step` confirmation on phase N, bound to that digest, so that the following `--from N`
  runs phase N's core half against it. Today `operator-step` is recorded only for a by-hand
  phase's stop (5, 10, phase 3's re-exchange); it has to be extended to the edge half of a *both*
  phase (1, 2, 3, 12, 13, 14).
- The hand-off stops, with one new catalog code, and the hand-off object of "The hand-off
  object": in the run's `-o json` report, in `status -o json` while the run waits, and on the
  edge driver's way back, with the human text naming the same argvs and a test checking it.
- The `import` stage word, which checks a result against its confirm code, copies it under
  `<work>/handoff/<N>/`, and journals its sha256.
- `--side edge` runs ending with the way-back object and its confirm code, and `--expect`
  on the edge driver refusing a carried file whose digest differs.
- Phase 14 split by a window: the controller's half first, then the edge's log read for that
  window, then `test` joining the two.
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
- each side is driven only from the hand-off objects: the scenario runs `command` on `run_on`,
  carries `result`, reads the confirm code from the edge driver's own JSON, runs `import` and
  `then`, and checks that the text of every stop names the same argvs.
