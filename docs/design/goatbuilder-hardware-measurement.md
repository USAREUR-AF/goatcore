# goatbuilder on hardware — what the clock measures, what the binary records, what the tutorials prove

| Field | Value |
|---|---|
| Status | **Designed.** The per-step timing file and its summary line are built for `standup`, `join-node` and `teardown`, each row with its cause; standup records phase 11 as a row of its own and its summary reads the founder run's numbers from the file (below). The row C3 procedure ("`status` on a standing goatnet", below) uses only built verbs and a built script. Everything else on this page is designed and not built. No run on hardware has been measured against it |
| Proves | Gate rows C1 (a founder site in 60 minutes or less, a node in 15, a burn-down in 15) C2 (tutorials 2 and 3 on real machines) and C3 (`status` agrees with `verify` and `diff` on a standing goatnet) of the goatbuilder 1.0 gate |
| Design | [goatbuilder-speed.md](goatbuilder-speed.md) (the targets and the modelled budget) · [goatnet-tutorials.md](goatnet-tutorials.md) (the tutorials and their checkpoints) · [goatnet-standup.md](../operations/goatnet-standup.md) (the prerequisites and the phases) |

A timed run is worth something only if a second person can repeat it and get a
comparable number. This page fixes what is inside the clock and what is
before it, which one step inside it waits on a person, what the binary prints
and keeps for each phase, and what each tutorial run on hardware has to show.

## The three clocks

Gate row C1 is three measurements, each with its own start and end.

| Clock | Starts | Ends | Target |
|---|---|---|---|
| Founder site | the declaration is committed and every host of the site is at its baseline: the operating system installed and the host reachable over SSH from the operator machine | `goatbuilder standup <env> --site <site>` prints its result after phase 14 | 60 min |
| One node | the node's row is committed and its host is at the same baseline | `goatbuilder join-node <env> --site <site> --node <n> --class <c>` prints its result | 15 min |
| Burn-down | `goatbuilder teardown <env> --site <site> --services` starts | it prints its result. The steps teardown refuses (the NetBird account, the identity store, the database) are a person's decision and outside the clock | 15 min |

The founder clock runs across every stop. If the run stops on a defect, is
fixed and resumes with `--from N`, the time between the stop and the resume is
inside the clock. A run that stops is still a measurement; it is reported with
its stops counted (below).

## Before the clock

Everything that has to exist before phase 1 can start is before the clock. A
measurement lists each item and says whether it was ready when the clock
started. An item that was not ready makes the run a rehearsal, not a
measurement.

| Item | What is ready | Where it is described |
|---|---|---|
| The cloud tree and its relays (internet sites) | `terraform-<goatnet>/` applied: the relay hosts running, their `relays` output readable by `generate-relays.sh`. The standup has no Terraform phase | [The cloud tree](#the-cloud-tree) |
| The DNS zone | the public zone delegated and its records applied from the same tree; the zone verified for reputation before first enrolment | [goatnet-standup.md](../operations/goatnet-standup.md), prerequisites 3 and 4 |
| Secret references | every secret the plan reads is declared under the secrets posture, and each value exists in its store. `goatbuilder doctor --site <site>` passes | [goatnet-standup.md](../operations/goatnet-standup.md), prerequisite 2 |
| The configuration surface | inventory group, group_vars file set, certificate manifest, pins row and door row committed | [goatnet-standup.md](../operations/goatnet-standup.md), prerequisites 5 to 9 |
| Pre-staged certificates | the goatnet's certificate authority key available to the run; the certificate manifest from `ops/enrollment/manifests/TEMPLATE-certs.yaml` | [goatnet-standup.md](../operations/goatnet-standup.md), prerequisites 2 and 7 |
| Pre-staged images | the registry read credential in place. A site cache that is already warm, or images already on the host, are allowed and the measurement says so; the first standup of a site fills the cache inside the clock | [goatbuilder-speed.md](goatbuilder-speed.md), sink 6 |
| The host baseline | the operating system installed, either from the stock installer or from an autoinstall seed | below |

**The operating system install is outside the clock, seeded or not.** The
autoinstall seed (`ops/scripts/node-seed.py`) moves disk encryption and package
installation from phase 2 into the installer, so a seeded host reaches phase 3
sooner. The two installs differ by tens of minutes per host
([goatbuilder-speed.md](goatbuilder-speed.md), sink 5). A measurement therefore
names its installer, and the two are never averaged or substituted:

- A run on stock-installed hosts meets row C1 on its own.
- A run on seeded hosts counts toward row C1 only after the seed's own gate in
  [goatbuilder-speed.md](goatbuilder-speed.md) (a seeded host's baseline timed
  against a stock one) is a measurement, and the row then reads "seeded".

## The one person inside the clock

Phase 11 is the only step inside the clock that waits on a person. It is the
first login to the NetBird dashboard in a browser, the promotion of that login
to admin, and the creation of the service user and token that phase 12 reads.
Its wall time is recorded on its own, and every founder measurement reports
three numbers: the total, phase 11, and the total without phase 11.

At phase 11 `standup` prints
`OPERATOR STEP — run the above, then re-run with --from 12` and exits. The
person's work happens between two processes.

**Built**, in `standup` (`record_stop` and `close_operator_stop` in
`crates/goatbuilder/src/standup.rs`): the run writes a row for phase 11 when
it stops, with `-` for its seconds and exit code and cause `operator`. The
run resumed with `--from 12` (or any later phase) first closes that row: it
writes a second phase-11 row whose seconds are the wall time from the stop to
the resume, exit code 0, cause `operator`, and prints
`phase 11 (the operator step) took <s>s`. Phase 11's wall time is read from
the record, not derived. It includes any time between the person finishing
and the resume being started. `ops/scripts/goat-standup.sh` writes no
phase-11 row.

## What the binary prints and keeps

**Plain:** for every phase, the person running the tool sees which phase it is,
when it started and ended, how long it took and how it ended. At the end they
see the total and the three slowest phases. The run record on disk holds the
same, so the measurement is copied from the record, not from a screen.

**Built today**, in `standup` (`record_timing` and `timing_summary` in
`crates/goatbuilder/src/standup.rs`; `ops/scripts/goat-standup.sh` writes the
same file):

| What | Form |
|---|---|
| A line per foreground step, and per phase-12 step run in `--parallel` waves | `phase <n> took <s>s (exit <rc>)`; a wave step prints `phase 12 <step id> took <s>s (exit <rc>)` |
| A row per such step in `<state_dir>/standup-<site>-timings.tsv` | time written, phase, step label, seconds, exit code, cause |
| A summary at the end of the process | `total <s>s (<m> min) over <k> phases; slowest: ...`, the five slowest steps of this process, then the founder run's numbers read from the file: `since phase 1: <s>s (<m> min), phase 11 <s>s, without phase 11 <s>s (<m> min); <k> stops (...)` |
| A failure | `!! Phase <n> ("<label>") FAILED — exit <rc>` and the resume command |

The rows are per step, not per phase: a phase of several steps writes several
rows. The cause is `ok` for a step that passed. For a failed step it is the
catalog code the step's logged output carries or matches, the same match as
the `interpreted` row (gate row F5, `explain_phase::cause`), or `uncatalogued`
when no code covers it. `operator` marks phase 11's rows, and `stopped` an open
row written when the run stops on purpose after a phase (the pubsub bot step
when its token cannot be loaded). `ops/scripts/goat-standup.sh` writes the
same six columns; it does not match output against the catalog, so its failed
rows say `failed`.

The founder numbers (`run_evidence`) cover the rows from the latest phase-1
row to the end of the process that prints them: the total since phase 1
began (that row's time less its seconds), across every resume; phase 11's
closed row; the total without it; and the stops. A stop is an open row (the
operator step, or a stop on purpose) or a failed step; each failed step is a
defect stop, named by its cause. A run with no phase-1 row in the file prints
only the process summary. The three slowest phases across resumes are not
built: the slowest list covers one process.

Two kinds of work are not timed on their own. The image pull and the
observability host's legs run in the background (`Event::PullStart` and
`Event::LegStart` in `standup.rs`), and their joins print no `took` line and
write no row. Their time appears only in the total, and the time the run waits
at each join is credited to no phase.

**Built**, in `join-node` and `teardown`, with the same `record_timing` and
`timing_summary` (`crates/goatbuilder/src/joinnode.rs`; `run_teardown_steps`
in `crates/goatbuilder/src/standup.rs`). `join-node` prints
`phase <n> took <s>s (exit <rc>)` for each step it runs and writes a row per
step to `<state_dir>/join-node-<site>-<node>-timings.tsv`. `teardown` prints
`teardown <component> took <s>s (exit <rc>)` for each component it stops and
writes a row per stop to `<state_dir>/teardown-<site>-timings.tsv`, with the
phase and label of the step the component came from. Both files have
standup's columns, the cause included (join-node matches a failed step's
logged output as standup does; teardown keeps no log of a stop, so a failed
stop's cause is `uncatalogued`), and both runs end with standup's process
summary line. A dry run writes no row; a failed step is the last row, with
its exit code. No script avenue writes these two files.

**Designed**, for `standup`, `join-node` and `teardown`, built with gate group F
(the tool shows its work):

1. **Per phase**, one line when the phase ends: its number out of the total, its
   name, start and end in UTC, duration, and outcome (`ok`, `failed`,
   `operator` or `stopped`). For example:
   `phase 7/14 public TLS and relay  10:21:04Z → 10:25:40Z  4m36s  ok`.
2. **At the end of the run**, one summary line: the total since phase 1 began,
   across every resume; phase 11's wall time; the number of stops; and the
   three slowest phases with their durations. Built for `standup` except the
   slowest phases across resumes (above).
3. **The run record** keeps the same rows per phase and the summary row, in
   the timing file both drivers write, so a run driven by either reads back
   the same way. The step rows stay, and the image pull, each observability
   leg and the wait at each join get rows of their own.
4. **Every stop records its cause code**: the `GB-nnn` code from
   `ops/errors/catalog.yaml` for a refusal or a known failure, `operator` for
   phase 11, and `uncatalogued` for an exit no code covers. Gate row C5
   treats each `uncatalogued` stop as a missing catalog entry. Built for a
   failed step of `standup`, `join-node` and `teardown` (above); a refusal
   before the first step writes no row.
5. **The measurement counts the stops.** A stop is any exit before the last
   phase: a failure, the operator step, or a deliberate stop after a phase.
   A founder run with phase 11 as its only stop is a run with no defect
   stops. Built for `standup`'s summary (above).

## The cloud tree

Generating a full goatnet's cloud tree is **not built**. An operator copies
an existing `terraform-<goatnet>/` tree and changes its names; that copy is
work before the clock. A goatlite that declares `lite.internet` is the exception:
`ops/scripts/goatlite-render.py` writes `terraform-<name>/main.tf` with the
relays, their public names and the outputs `generate-relays.sh` reads
([goatlite.md](goatlite.md), "What the render writes").

**Build item, not built:** a generator that writes `terraform-<goatnet>/` for
a full goatnet from its declaration, with the same modules, outputs and
cross-account fuse that `ops/ci/check-env-coverage.sh` checks. Until it
exists, a C1 measurement lists the tree copy as a before-the-clock item and
records how long it took, as a separate number.

## The tutorials on hardware

Gate row C2 is each tutorial followed from its published text on real
machines, with every command's output matching the text. A command whose
output differs is a fix to the tool or to the text, and the tutorial is
followed again from that step.

### Tutorial 2, a first federated node

The founder and the joiner are two sites of one goatnet, so one party runs
both. The run is done when the tutorial's checkpoints 2 to 4 hold on hardware
([goatnet-tutorials.md](goatnet-tutorials.md)): the joiner's gate green from
the joiner's network, every subscription replicating both ways, and `status`
across both sites.

### Tutorial 3, a boundary node

Tutorial 3 peers a goatnet with a counterparty that runs its own goatnet under
its own authority. Row C2 is met by that form: the counterparty is a separate
goatnet with its own certificate authority, its own hosts, and a different
person running its side. That person answers its prompts at phases 5, 6 and
10, reads back the key in the exchange, and runs its half of phase 14's
acceptance. The run is done when phase 14's acceptance is green on both sides:
a sample carries in the agreed direction, a write outside the agreed prefix is
refused, and a nonce round-trips through the counterparty's own router.

A peering between two goatnets that one party runs is a weaker run. It shows
that the phases work on real machines. It does not show the exchange between
two people, or that the counterparty's side works from the published text
alone. Accepting it for row C2 is an operator decision, recorded in the gate
row as "both goatnets run by one party". Without that decision, it is a
rehearsal.

## `status` on a standing goatnet (row C3)

Gate row C3 asks that `status`, `diff` and `verify` give the same verdict for
every machine of a standing goatnet. It does not ask that the goatnet is
converged: a goatnet with drift or unreachable machines is a valid
measurement. The three verbs only read. No play runs and nothing is written on
a host. This is the shortest of the operator's runs, about fifteen minutes.

**Before the run.** A release binary, the goatnet's checkout at the commit it
deploys from, `jq` and `python3`, and the same credentials and reach `diff`
needs. `goatbuilder doctor --env <goatnet>` prints the transport the verbs will
use.

**The run.** From the root of the checkout, run the block in
[goatbuilder.md](goatbuilder.md), "`status` on real hosts (gate row C3)", with
`E` set to the goatnet's key. It runs `status` and `diff` once each with
`-o json`, then `verify` once for each component `status` names, then
`ops/goatbuilder/c3-agreement.py` over the reports. Time each verb.

| The script ends with | It means |
|---|---|
| exit 0, `c3: transport: …` and `c3: agree on N machine(s), M component(s): … reached R of N machine(s)` | C3 is measured for the R machines reached |
| exit 3 and `c3: DISAGREE <host>/<component>: status …, verify …, diff …` | C3 fails. Each line is a defect in goatbuilder, filed with the three reports' rows for that machine |
| exit 3 and `c3: status names component(s) verify was not run for: …` | The loop missed a component. Run `verify` for it and compare again |
| any other exit | Not a measurement. Fix what it names and run the sequence again |

A transport that reaches only part of the goatnet, such as a door to one site,
leaves the other machines `unknown` under all three verbs. That is still a C3
measurement, but only of the R machines reached.

**What to keep.**

| Field | Content |
|---|---|
| Build | the goatcore commit and the `goatbuilder --version` output |
| Transport | `doctor`'s transport line and the script's `c3: transport:` line |
| Reach | `reached R of N machine(s)` |
| Result | the script's exit code and its `c3: agree` line, verbatim |
| Times | the wall time of `status`, `diff` and each `verify` |
| Reports | `status.json`, `diff.json` and every `verify-<component>.json` |

The reports and the goatnet's key name a real deployment's hosts, so they go
to the operator's private record. The gate row records only the version, the
commit, the kind of transport, R of N, the number of components and the
verdict.

The same three verbs run through the binary on a stand-in transport in
`crates/goatbuilder/tests/status_real_hosts.rs`, so the procedure can be
rehearsed without a goatnet.

## What a measurement records

Each run kept as a measurement records:

| Field | Content |
|---|---|
| Clock | founder site, one node or burn-down |
| Build | the goatcore commit and the `goatbuilder --version` output |
| Hardware | each host's node class and model |
| Installer | stock or seeded, per host |
| Before the clock | each item above, ready or not; a warm cache or pre-pulled images, if any; the time the cloud tree copy took |
| Result | the total, phase 11's wall time and the total without it (founder clock only) |
| Phases | the per-phase rows from the run record |
| Stops | the count, and each stop's phase and cause code |
| Outside accounts | every outside account the run reached; the 1.0 gate measures "no outside account" as part of row C1 |

The founder measurement replaces the modelled rows of the budget in
[goatbuilder-speed.md](goatbuilder-speed.md), "The budget". The three slowest
phases are the next three levers.

## What is not built

| Item | Status |
|---|---|
| Per-phase lines, the run summary and the run record as described above | designed |
| Cause codes on every stop, and the stop count | designed |
| Phase 11 recorded as a row | designed |
| The image pull, the observability legs and their joins as rows | designed |
| A timing file and summary for `teardown` | not built |
| Timing for `join-node` | not built |
| A generator for a full goatnet's cloud tree | not built |

None of the three clocks has been measured on hardware, and neither tutorial
has been followed on hardware.
