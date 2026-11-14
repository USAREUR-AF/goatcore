# goatbuilder on hardware — what the clock measures, what the binary records, what the tutorials prove

| Field | Value |
|---|---|
| Status | **Designed.** The per-step timing file and its summary line are built for `standup`, and a per-component time line for `teardown` (below); everything else on this page is designed and not built. No run on hardware has been measured against it |
| Proves | Gate rows C1 (a founder site in 60 minutes or less, a node in 15, a burn-down in 15) and C2 (tutorials 2 and 3 on real machines) of the goatbuilder 1.0 gate |
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

Today the run cannot record phase 11 by itself. At phase 11 `standup` prints
`OPERATOR STEP — run the above, then re-run with --from 12` and exits. The
person's work happens between two processes, and the timing file has no row
for it. Its wall time can be read from the file's timestamps: the last row
written before the stop, and the first row after the resume less that row's
own duration.

**Designed:** the run writes a row for phase 11 when it stops, with outcome
`operator`, and the resumed run closes that row with its end time. Phase 11's
wall time is then read from the record, not derived.

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
| A row per such step in `<state_dir>/standup-<site>-timings.tsv` | time written, phase, step label, seconds, exit code |
| A summary at the end of the process | `total <s>s (<m> min) over <k> phases; slowest: ...`, the five slowest steps |
| A failure | `!! Phase <n> ("<label>") FAILED — exit <rc>` and the resume command |

The rows are per step, not per phase: a phase of several steps writes several
rows. A stop names no cause code; the file keeps the exit code only. The
summary covers one process, so a run resumed after phase 11 or after a defect
prints two summaries and neither is the run's total.

Two kinds of work are not timed on their own. The image pull and the
observability host's legs run in the background (`Event::PullStart` and
`Event::LegStart` in `standup.rs`), and their joins print no `took` line and
write no row. Their time appears only in the total, and the time the run waits
at each join is credited to no phase.

`teardown` prints `teardown <component> took <s>s (exit <rc>)` for each
component, and writes no timing file and no summary. `join-node` records no
timing.

**Designed**, for `standup`, `join-node` and `teardown`, built with gate group F
(the tool shows its work):

1. **Per phase**, one line when the phase ends: its number out of the total, its
   name, start and end in UTC, duration, and outcome (`ok`, `failed`,
   `operator` or `stopped`). For example:
   `phase 7/14 public TLS and relay  10:21:04Z → 10:25:40Z  4m36s  ok`.
2. **At the end of the run**, one summary line: the total since phase 1 began,
   across every resume; phase 11's wall time; the number of stops; and the
   three slowest phases with their durations.
3. **The run record** keeps the same rows per phase and the summary row, in
   the timing file both drivers write, so a run driven by either reads back
   the same way. The step rows stay, and the image pull, each observability
   leg and the wait at each join get rows of their own.
4. **Every stop records its cause code**: the `GB-nnn` code from
   `ops/errors/catalog.yaml` for a refusal or a known failure, `operator` for
   phase 11, and `uncatalogued` for an exit no code covers. Gate row C5
   treats each `uncatalogued` stop as a missing catalog entry.
5. **The measurement counts the stops.** A stop is any exit before the last
   phase: a failure, the operator step, or a deliberate stop after a phase.
   A founder run with phase 11 as its only stop is a run with no defect
   stops.

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
