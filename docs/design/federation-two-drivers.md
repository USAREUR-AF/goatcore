# Federating from two drivers: the contract

| Field | Value |
|---|---|
| Status | **Designed; the hand-off stop is built.** A peering declares two drivers with `drivers: two` ("Which peerings have two drivers"), and the controller's run stops at the first phase that needs the edge with the hand-off object (GB-263; a checkout with changes is refused, GB-264). The halves, the edge driver's `--side edge` run, `import` and the rest are not built; the phase-3 halves are in review. It is the product contract goatbuilder 1.0 gate row B10 is proven against |
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

## Which peerings have two drivers

A peering opts in on its own `peerings:` entry in the deployment's `goatbuilder.yaml`, read like
every other peering value, so nothing about a deployment is in the binary:

```yaml
peerings:
  - with: xab
    environment: zagros
    site: alpha
    edge_class: edge
    drivers: two
```

| Value | Meaning |
|---|---|
| `one`, or absent (the default) | one machine reaches both the edge host and the core host and runs every phase, as before |
| `two` | an edge driver and a controller, as this document describes: the controller stops at each phase that needs the edge |

The schema (`ops/goatbuilder/schema/goatbuilder-v1.schema.json`, `peering.drivers`) allows only
those two values. `crates/goatbuilder/src/federate/handoff.rs` reads it (`two`) from the peering
entry the run resolves, and it marks each phase's side (`side_of`) as the table above does. The
choice is the deployment's, not the machine's: a reachability probe would hand off on a passing
SSH failure, and an environment variable is easy to forget on one of the two machines.

Status: built (`federate_two_drivers.rs`).

**Phase 1 passes only when both sides' results are ready.** A phase 1 result written by one
side's checks (`preflight.py --side edge|core`, its `side` member) is never ready on its own,
however its own checks went: `crates/goatbuilder/src/federate/plan.rs` (`read_readiness`) reads it
as not ready and adds a BLOCKED `sides` row that says why. Combining it with the other side's
result is the import, which is not built. Status: built (the test
`a_one_sided_phase_1_result_is_never_ready_on_its_own`).

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
everything that command reads from the other machine, the file it writes, and
the two commands the controller runs after it. The sentence a person reads
names the same commands.

Status: built at the stop: the controller's run with `-o json` carries `handoff`, its text names
every argv on its own line, quoted for a POSIX shell, and a checkout with changes gives no object
(GB-264), naming each file (`crates/goatbuilder/src/federate/handoff.rs`, test
`federate_two_drivers.rs`). Not built: the object in `status`, the edge's way-back object, and
the `import` and `then` it names. Phase 3's two halves, which the phase 3 object wraps, are
in review; this section says "built" for them once they are on main.

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
  "command": ["goatbuilder", "federate", "xab", "--side", "edge", "--from", "3", "--to", "3",
              "--expect-commit", "<commit>"],
  "commit": "<commit>",
  "carry": [],
  "result": "handoff/3/edge-half.json",
  "import": ["goatbuilder", "federate", "import", "xab", "3", "{result}", "--sha256", "{code}"],
  "then": ["goatbuilder", "federate", "xab", "--from", "3"]
}}
```

| Member | Meaning |
|---|---|
| `phase` | the phase the hand-off belongs to |
| `code` | `GB-263`, "the run is waiting on the other driver", the same for every hand-off |
| `run_on` | `edge` or `controller`: the machine that acts next |
| `command` | what to run on `run_on` now, as an argv. `null` in the edge's way-back object, where nothing more runs on the edge |
| `commit` | the commit of the deployment's checkout on the controller. The edge's plays read the checkout: the inventory, the group_vars that `allocate` renders and that phase 10 places, and the roles. `command` carries `--expect-commit <commit>`, and the edge driver refuses to run unless its checkout's `HEAD` is that commit and its tree is clean. The controller emits a hand-off object only when its own tree is clean, so that `commit` holds every `in: checkout` file: phase 10 places rendered group_vars in the checkout and admission writes `peerings.yml`, and until they are committed the controller stops instead, naming each file to commit. A file the checkout ignores is never read from the checkout by an edge step; a file the edge reads is either committed or carried as `in: work` |
| `carry` | every file the edge step reads that the controller produced, each `{"file": "<path>", "in": "checkout" \| "work", "sha256": "<digest>"}`. A `checkout` file is pinned by `commit` as well; a `work` file exists only under the controller's `<work>` and is copied to the same path under the edge driver's. `command` names each with `--expect <file>=<sha256>`, and the edge driver refuses a file whose digest differs. `[]` when the step reads nothing but the pinned checkout |
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

### The confirm code and the imported file

**Plain:** the confirm code reaches the controller only through a person, or a
test, reading the edge driver's own output. It is never in the file, and never
in anything that travels with the file. The controller checks the file against
it once on import, and again each time it reads the file.

- The edge run prints the confirm code: the first 32 hex digits of the
  result's sha256, in groups of four. In its JSON the code is the member
  `confirm_code` of the way-back object, and it is filled into `import`. That
  JSON is the edge driver's screen, in another form.
- The code never appears in `result`, in the result file, or in `carry`. The
  way-back JSON, with `confirm_code` and `result_sha256`, never travels with
  the file: a code carried beside a changed file would confirm the change.
- `import` refuses a file whose sha256 does not begin with the code given. It
  copies the file to the controller's `<work>/handoff/<N>/` and records the
  full sha256 in the journal as an `operator-step` confirmation on phase N.
- **Every `then` hashes the file again before reading it.** Phase N's core
  half reads `<work>/handoff/<N>/…` only after its sha256 equals the one the
  journal holds, and it stops with the hand-off code when it does not. The
  operator does not type the digest again.

A scenario that reads the code from the edge driver's JSON is reading the edge
driver's own output, which is exactly what the binding rests on. It must not
read the code from the controller, from the result file, or from anything
copied with it.

### Phase 3: the hand-off whose halves are in review

Phase 3's two halves are written at the `init` level and are in review:
`goatbuilder federate init --over-ssh <peering> --side edge --half-out <file>`
writes the edge half (schema `goatbuilder.phase3-edge-half/v1`) and prints its
sha256, its confirm code and the line `goatbuilder federate init --over-ssh
<peering> --side core --edge-half <file> --edge-half-sha256 <code>`
([federation-router-discovery.md](federation-router-discovery.md), once that
change is on main). The phase 3 object wraps those two halves as they are:

- **`command`** (`--side edge --from 3 --to 3`) runs the edge half,
  `init --over-ssh xab --side edge --half-out <work>/handoff/3/edge-half.json`,
  with the init flags the plan gives. The file, its schema and its confirm code
  are the edge half's own.
- **`import`** runs the core half's file checks, nothing more:
  - the file's sha256 begins with the code;
  - its schema and peering are an edge half's for this peering;
  - it was measured no more than 24 hours ago and no more than 5 minutes in the
    future;
  - its edge host is the one this checkout's inventory names, checked only when
    the inventory names exactly one edge host; otherwise the import says the
    host was not checked.

  Each failure is GB-114. It then copies the file under `<work>/handoff/3/` and
  journals its sha256.
- **`then`** (`--from 3`) hashes the file again (above), then runs the core
  half: `init --over-ssh xab --side core --edge-half
  <work>/handoff/3/edge-half.json --edge-half-sha256 <the journaled sha256>`,
  with the plan's init flags. The core half runs every file check again,
  including the 24-hour bound at that moment. At measure time it also checks
  that the half was measured for the interface the init flags ask for. It then
  joins, builds and signs the package.

One thing changes in the halves as written: when the edge half runs inside the
one command, its last line names the one command's `import` and `then` instead
of the `init --side core` line, and it prints the way-back object with `-o
json`. `init` run on its own keeps printing what it prints.

### Each hand-off, filled in

For a peering `xab` seen from `xaa`, with default working directories and an
edge group `<edge>`. Every `command` begins `["goatbuilder", "federate", "xab",
"--side", "edge", "--from", "<N>", "--to", "<N>", "--expect-commit",
"<commit>"]` and ends with one `--expect <file>=<sha256>` pair per `carry`
entry. Every `import` is `["goatbuilder", "federate", "import", "xab", "<N>",
"{result}", "--sha256", "{code}"]`. Every `then` hashes its file first.

| Phase | What `command` runs on the edge | `carry`, beyond the pinned checkout | `result` | `then` |
|---|---|---|---|---|
| 1 | preflight's edge checks only | none | `handoff/1/edge.json` (the edge checks' results) | `--from 1`: the core checks and `signing-ca`, then the join |
| 2 | the federation interface and its key; the edge's leaf keys and their signing requests | none | `handoff/2/edge.json` (the interface's public key, one signing request per edge leaf; no private key) | `--from 2`: the SPIRE bundle export on the SPIRE server host |
| 3 | the edge half above | none | `handoff/3/edge-half.json` (`goatbuilder.phase3-edge-half/v1`) | `--from 3`: the core half, then phases 4 to 10 on the controller |
| 11 | the transport | `ops/ansible/group_vars/<edge>/federation-xab-transport.yml` (`checkout`; phase 9 renders it at `<work>/applied/ops/ansible/group_vars/<edge>/federation-xab-transport.yml`, `crates/goat-federation/src/render.rs`, and phase 10 places it) | `handoff/11/edge.json` (the interface's state and its one peer) | `--from 12`: phase 11 has no core half, so the controller goes on to 12, which stops for 12's edge half |
| 12 | the edge router | `pki/peering-outward.pem` and `pki/peering-inward.pem` (`work`: the two edge certificates phase 8 signed from phase 2's requests; the edge's keys never left it); `pki/our-ca.pem` (`work`); `theirs-ca.pem` (`work`: the partner CA, `goat_zenoh_edge_partner_ca_src`, installed at `goat_zenoh_edge_partner_ca_path`); `ops/ansible/group_vars/<edge>/federation-xab.yml` and `…/federation-xab-transport.yml` (`checkout`: the rendered edge files) | `handoff/12/edge.json` (the edge router's running state and its configuration digest) | `--from 12`: the republisher on the core, from `…/federation-xab-republish.yml` |
| 13 | the edge's SPIRE forwards (`goat-peering-bundle-in-xab`, `goat-peering-bundle-out-xab`) | `ops/ansible/group_vars/<edge>/federation-xab-transport.yml` (`checkout`): the partner's trust domain and the two link addresses (`goat_peering_partner_trust_domain`, `goat_peering_local_link_ip`, `goat_peering_remote_link_ip`, `crates/goat-federation/src/render.rs`; the SPIRE role reads them through its own `peering_spire_*` defaults). The bundle endpoint (`peering_spire_server_addr`, `peering_spire_bundle_port`) is in the edge group's own group_vars, pinned by `commit`. The bundle files `ours.spiffe-bundle` and `theirs.spiffe-bundle` are not carried: only the tasks delegated to the SPIRE server host read them, and those run in the controller's half | `handoff/13/edge.json` | `--from 13`: the tasks delegated to the SPIRE server host |
| 14 | the edge's log, filtered as below | none | `handoff/14/edge.json` (only the edge's log lines that carry the core half's ids) | `["goatbuilder","federate","test","xab"]`: matches the edge's lines to the core half's result by id, and decides |

**Phase 14 binds by id, not by time.** It is the one hand-off that comes
after the controller has acted. The controller runs its half first and records
the ids it fired: the session id of each probe and the nonce of the
round trip. Its `command` names them (`--ids <id>,<id>,…`) and a window
(`--window <start>/<end>`). The edge returns only log lines that carry one of
those ids and fall in the window widened by 5 minutes on each side, the clock
skew phase 3 also allows. This is the filter `acceptance.sh` applies today
when it keeps only the trace lines of the probe's own session (`CZID`).
`test`, the stage that runs phase 14 alone, matches each returned line to the
probe that fired it by id, uses the window only as an outer bound, and fires
nothing again.

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
- The controller's clean-tree rule: no hand-off object while its checkout has uncommitted changes;
  the stop names each file to commit.
- `--side edge` runs ending with the way-back object and its confirm code; `--expect-commit`
  and `--expect` on the edge driver, refusing a checkout at another commit or with changes, and
  a carried file whose digest differs.
- Every `then` hashing its imported file against the journaled sha256 before reading it.
- Phase 14 split by id: the controller's half first, recording the ids it fired; the edge's log
  lines carrying those ids, within the window and 5 minutes either side; `test` matching them by
  id.
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
