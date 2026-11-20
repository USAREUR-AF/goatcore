# goatbuilder on hardware — what the clock measures, what the binary records, what the tutorials prove

| Field | Value |
|---|---|
| Status | **Designed.** The per-step timing file and its summary line are built for `standup`, `join-node` and `teardown`, each row with its cause; both standup drivers record phase 11 as a row of its own, and `goatbuilder standup`'s summary reads the founder run's numbers, the slowest phases across resumes among them, from the file (below). The row C3 procedure ("`status` on a standing goatnet", below) and the phone procedures for rows H7 and H8 ("A phone node on hardware", below) use only built verbs and scripts. Everything else on this page is designed and not built. No run on hardware has been measured against it |
| Proves | Gate rows C1 (a founder site in 60 minutes or less, a node in 15, a burn-down in 15) C2 (tutorials 2 and 3 on real machines), C3 (`status` agrees with `verify` and `diff` on a standing goatnet), H7 (a phone built by one command) and H8 (a phone node for seven days) of the goatbuilder 1.0 gate |
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
and the resume being started. `ops/scripts/goat-standup.sh` writes the same
open row at the operator step, and a resumed run of the script closes it with
`ops/scripts/standup-timings-close.py` (the same rule, tested by
`tests/test_standup_timings_close.py`), so a run driven by either, or by both
in turn, reads back the same way. The script prints no summary line.

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
| A summary at the end of the process | `total <s>s (<m> min) over <k> phases; slowest: ...`, the five slowest steps of this process, then the founder run's numbers read from the file: `since phase 1: <s>s (<m> min), phase 11 <s>s, without phase 11 <s>s (<m> min); <k> stops (...); slowest phases: ...` |
| A failure | `!! Phase <n> ("<label>") FAILED — exit <rc> (GB-331)` and the resume command |

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
only the process summary. The founder numbers end with the three slowest
phases across every resume, phase 11 left out because it is reported on its
own: `slowest phases: <n> <s>s, ...`. A phase's time is the wall time of each
stretch of its consecutive rows, from the earliest step's start to the latest
row, summed over its stretches. Steps run side by side in phase 12's waves
count once. A failed step or an open row ends a stretch, so the time until the
resume is no phase's.

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
   three slowest phases with their durations. Built for `standup` (above).
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

**Before the run.** Everything is in the tutorial's "Before the real run":
- a founder whose `goatbuilder assay <founder-site>` is green;
- the joiner's group_vars, with `goatnet_founder: false` and `join_founder_site`;
- both sites' groups under `[mgmt_hosts:children]`;
- the joiner's member group in `ops/pins/fleet-pins.yaml`;
- signal federation declared on both sides, passing
  `ops/scripts/signal-federation-check.py <joiner-site> --resolve`;
- reach to both sites.

The machine that runs the assay holds a goat context named after each site:
`<joiner-site>` and `<founder-site>`. The cross-site round trip runs `goat`
under those two names; other names are passed with
`--var goat_context=<name> --var goat_context_peer=<name>`. You also need a
release binary and the goatnet's checkout at the commit it deploys from.

**The run.** From the root of the checkout:

```text
goatbuilder --version
goatbuilder doctor --env <env> --credentials
goatbuilder standup <env> --site <joiner-site> --dry-run
ops/scripts/with-secrets goatbuilder standup <env> --site <joiner-site> --ack "<your name>" --yes
goatbuilder assay <joiner-site>
goatbuilder assay <founder-site>
goatbuilder status --env <env>
```

`standup --site <joiner-site>` runs the signal federation steps in its phase
9, the same rows `join-node` runs in its phase 13
([signal-federation-pair.md](signal-federation-pair.md)). Each further machine
of the joiner runs
`goatbuilder join-node <env> --site <joiner-site> --node <name> --class <class> --address <LAN address> --yes`,
after its `--dry-run`. Compare every command's output with the tutorial's
text as you go. A difference is a finding even when the command exits 0.

| The run ends with | It means |
|---|---|
| `standup` exit 0, with phase 9's `[signal federation] proven from both nodes` row passing | The joiner's three passes, the founder-side mirror and the federation proof ran: the federation half of checkpoint 3 holds |
| `standup` exit 0, with phase 9's `[signal federation]` gate passing and its SKIP line (`declared off at <joiner-site>`) | The joiner declares `signal_federation_absent_reason` and the gate proved no side expects federation. Checkpoint 3's federation half does not apply; record the declared reason |
| `assay <joiner-site>` exit 0, every row of the `federation` layer PASS, including `goat-xsite-roundtrip` (`xsite: PASS …`) | Checkpoint 2 holds, and the joiner's half of checkpoint 3 |
| `assay <founder-site>` exit 0, with `fed-founder-netbird-subscriptions-replicating` (and, with Ory on both sites, `fed-founder-kratos-subscriptions-replicating` and `fed-founder-hydra-subscriptions-replicating`) PASS | The founder's half of checkpoint 3: each subscription it must hold from the joiner exists by name and replicates |
| `status --env <env>` lists both sites' machines, every one `converged` | Checkpoint 4 holds |
| a phase fails, or `assay` exits 3 | Not yet a measurement. Note the phase, the code and the output. Fix the tool or the text, then resume with `--from <phase>` |

Checkpoint 3 is not fully covered by a command today. `assay` at the joiner
reads the joiner's inbound subscriptions (netbird, kratos and hydra) and the
founder's rows present on the joiner. `assay` at the founder reads the
founder's subscriptions from the joiner. Nothing yet writes a row at the
founder and deletes it at the joiner. Until a command does, the run records
that leg as not measured.

**What to keep.**

| Field | Content |
|---|---|
| Build | the goatcore commit and the `goatbuilder --version` output |
| Hardware | each joiner machine's node class and model |
| Result | the exit code of `standup`, of `assay <joiner-site>` and of `status` |
| Findings | each command whose output differed from the text, with the phase, the code and the difference |
| Times | `standup`'s summary line, and each `join-node` run's total |
| Reports | `assay`'s rows (`--rows-out`), `status -o json`, the timings file |

The reports name a real deployment's hosts, so they go to the operator's
private record. The gate row records the version, the commit, the number of
machines per site, the exit codes and the number of findings.

### Tutorial 3, a boundary node

Tutorial 3 peers a goatnet with a counterparty that runs its own goatnet under
its own authority. Row C2 is met by that form: the counterparty is a separate
goatnet with its own certificate authority, its own hosts, and a different
person running its side. That person answers its prompts at phases 5, 6 and
10, reads back the key in the exchange, and runs its half of phase 14's
acceptance. The run is done when phase 14's acceptance is green on both sides:
a sample carries in the agreed direction, a write outside the agreed prefix is
refused, and a nonce round-trips through the counterparty's own router.

That run cannot pass yet. Each side holds only its own surfaces. The refusal
rows need probe sessions holding the partner's identity, and
`spire/svid-chains` needs the partner's SPIRE server. So phase 14 at each side
ends INCOMPLETE, exit 3, with those rows BLOCKED or SKIPPED. The fix is
[federation-acceptance.md](federation-acceptance.md) §6.4, which moves each
row to the side that owns its surface; it is designed, not built. Until §6.4 is
built, a two-party run is a rehearsal of every phase but the last.

A peering between two goatnets that one party runs is a weaker run. It shows
that the phases work on real machines. It does not show the exchange between
two people, or that the counterparty's side works from the published text
alone. It can pass today, because that party can give each side the other's
probe launcher and SPIRE command; [tutorial 4](../tutorials/04-two-sovereigns.md)
runs it that way. Accepting it for row C2 is an operator decision, recorded in
the gate row as "both goatnets run by one party". Without that decision, it is a
rehearsal.

**Before the run.** Everything in the tutorial's "Before the real run":
- a goatnet whose gate is green;
- an edge host in node class `PEERING-EDGE`;
- the peering declared under `peerings:`, its files written by
  `goatbuilder federate allocate <peering>`, reviewed and committed;
- a peering directory both sides reach;
- the signing key and CA declared under `secrets.federation`;
- a FIPS-mode fabric image on both edge routers, or the waiver recorded.

You also need the counterparty on a call for phases 5, 6 and 10, a release
binary on each side, and each side's checkout at the commit it deploys from.

**The run.** Each side, from the root of its own checkout:

```text
goatbuilder --version
goatbuilder federate <peering> --env <env> --as <ours> --dry-run
ops/scripts/with-secrets goatbuilder federate <peering> --env <env> --as <ours>
goatbuilder federate status <peering> --env <env>
```

The run asks at phases 5, 6 and 10. It stops at phase 5 until the
counterparty's package is in the directory; run the same command again to
resume. Without a terminal, run the four stage commands of the tutorial's "Run
it on your hardware" instead. Phase 14 alone runs again with
`goatbuilder federate test <peering> --env <env>`.

| Phase 14, at each side, ends with | It means |
|---|---|
| exit 0, verdict PASS: carries, refuses and round-trips all PASS | Checkpoint 2 holds at this side. Row C2 needs it at both sides |
| exit 3, INCOMPLETE, with only the `refusal/*` and `spire/svid-chains` rows open | Expected until §6.4 is built. Every other phase is measured, phase 14 is not |
| exit 3, INCOMPLETE, with any other row open | Not yet a measurement. Fix the surface the row names, then run `federate test` again |
| exit 1, FAIL | A defect. File the row with both sides' result bundles |

**What to keep.**

| Field | Content |
|---|---|
| Build | each side's goatcore commit and `goatbuilder --version` output |
| Parties | one party or two, and the transport the agreement chose |
| Result | each side's phase 14 exit code and verdict, and every row not PASS |
| Findings | each command whose output differed from the text, with the phase and the difference |
| Reports | each side's acceptance result bundle and the agreement digest |

The bundles and the agreement name both goatnets, so they go to each party's
private record. The gate row records the versions, one party or two, the
transport, and each side's verdict.

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

## A phone node on hardware (rows H7 and H8)

Row H7 is one phone built from a reset VM to a verified node by one command,
with the released binary, in one run that is not resumed. Row H8 is that node
still a node seven days later, with a phone reboot during the week that it
comes back from without a person. The phone, the VM and what the tool does to
them are in [phone-node.md](phone-node.md).

### Before the clock (H7)

- **The phone.** An Android phone whose Linux Terminal app runs a Debian VM
  (Android 16 on a recent Pixel), with USB debugging allowed. Allowing USB
  debugging is done by hand on each phone. Switch off the app store's
  automatic updates: an update of Android's WebView stops the VM.
- **A reset VM.** In the Terminal app: settings, Recovery, "Reset to
  initial version". The tool installs the image again if the app asks for
  it, inside the clock.
- **The computer.** The released `goatbuilder`, `adb` on the `PATH`, the
  phone on a USB cable and unlocked. With more than one device attached, set
  `ANDROID_SERIAL`.
- **The site.** The phone node declared in the site's inventory with no
  address, under class `phone`, as site `delta` of
  `ops/goatbuilder/fixtures/join-node-third-site` declares its phone. The site
  states none of the values the class carries (gate row H5).
- **A clean record.** Move aside any earlier
  `<state_dir>/join-node-<site>-<node>-timings.tsv` and the node's SSH files
  `<state_dir>/join-node-<site>-<node>-usb.*` (a reset VM has new host keys). `state_dir` is `deploy.state_dir` of
  `goatbuilder.yaml`, `~/.goat/state` when unset.
- **A rehearsal.** `--dry-run` prints every phase and runs nothing.

### The run (H7)

```bash
goatbuilder --version
goatbuilder join-node <environment> --site <site> --node <node> --class phone --via usb --dry-run
time goatbuilder join-node <environment> --site <site> --node <node> --class phone --via usb --yes
```

`--yes` answers the run's confirmations, so no prompt waits inside the clock.
The clock starts when the command starts and stops when it exits. Phase 2
prepares the VM and runs its integrity checks, and phases 3 to 15 join the
node as any `join-node` does.

**It passes** when the command exits 0 after phase 15, and the timing file
shows one run. Since #869, phase 1 of a phone run is a check that the phone
answers over USB (`adb get-state`), recorded as a row like every other phase,
and the run goes on to phase 15 with no operator step (on the fixture:
`crates/goatbuilder/tests/join_node_phone.rs`,
`a_phone_joins_over_usb_from_phase_1_to_15`). A phone that does not answer
stops the run at phase 1 with GB-314. The file holds exactly one phase-1 row, every row exits 0, and
no step label appears twice. A `--from` resume, or a step run again after a
failure, adds a second row for a step, and that run is not H7: fix the cause
and start again from a reset VM.

### What to keep (H7)

| Field | Content |
|---|---|
| Build | the goatcore commit and the `goatbuilder --version` output |
| Phone | the model, the Android version, and the VM's kernel after the run (`uname -r` in the VM) |
| Result | the exit code, the wall time from `time`, and the process summary line `join-node` prints at the end |
| Phases | the timing file, with phase 2's rows (the VM's preparation and checks) named |
| Before the clock | whether the Terminal app had to install its image again, and how long the reset took |

The timing file and the run's output name the site, the node and the goatnet's
hosts, so they go to the operator's private record. The gate row records only
the version, the commit, the phone model and Android version, the total time,
phase 2's time and "one run, no resume".

### The seven days (H8)

H8 starts when H7's run ends, with the phone on power, the screen off and the
launcher installed (`ops/phone/vm-launcher/README.md`). Once a day, from the
operator's computer:

```bash
# 1. the H4 checks, in the VM, over the mesh (no USB cable needed after enrolment)
ssh <deploy user>@<node mesh address> 'bash -s' < ops/scripts/phone-node-integrity.sh
# 2. the node as goatbuilder reads it
goatbuilder status --env <environment> -o json > status-day<N>.json
```

Check 1 writes 512 MiB and drops the page cache, a few seconds of load on a
running node. Each day keeps the check's exit code (0, or its one
`integrity: <what>` line), the node's `status` rows, and the time.

**The reboot.** Once during the week, reboot the phone and record three
times: the reboot, the VM running again (`adb shell` finds the VM's address,
or the node answers on the mesh), and the node's rows converged in `status`
again. "Without a person" is met only when nobody touches the phone. The
launcher starts the VM at the first unlock, and Android delivers its start
signal at boot only on a phone with no lock screen. A node phone with no lock
screen is a decision about the phone's own security. Operator decision,
2026-11-15: the phone of the seven-day record may run with no lock screen, and
only a dedicated test phone may. With a lock screen, the run records the one
unlock, and H8 is not met as written.

**It passes** when every day's integrity check exits 0, the node's `status`
rows are converged on every day, and the node came back after the reboot.

### How long a phone may be off

A phone node is a full replication member (the class's `converges_as: core`).
Every site that replicates with it keeps a replication slot for it, and
Postgres invalidates a slot that holds more WAL than `max_slot_wal_keep_size`.
After that the phone must be seeded again
([post-partition-convergence-runbook.md](../operations/post-partition-convergence-runbook.md),
"A. Slot rebuild — the destructive path"); the procedure and the `goatbuilder`
path for it, designed and not built, are below. The cap is
1 GB in the image's `postgresql.conf` and 6 GB where the
`postgres-streaming-replication` role sets it. Read the live value; never
assume either one.

H8 measures the budget. On each site that replicates, twice a day during the
week, in the convergence runbook's way (`psql -h 127.0.0.1 -p 5432 -U postgres
-d postgres` on the site's management host):

```sql
SELECT now(), pg_current_wal_lsn(), current_setting('max_slot_wal_keep_size');
SELECT slot_name, active, wal_status, safe_wal_size,
       pg_wal_lsn_diff(pg_current_wal_lsn(), restart_lsn) AS retained
  FROM pg_replication_slots ORDER BY slot_name;
```

The site's WAL rate is the difference of two `pg_current_wal_lsn()` readings
(`pg_wal_lsn_diff`) divided by the time between them. During the reboot, the
slots that read `active = f` while every other site is up are the phone's.
A slot can be off for about `safe_wal_size` divided by the WAL rate before it
is invalidated, and the phone's budget is the smallest such time over its
slots. For example, a 1 GiB cap and a rate of 2 MiB an hour allow about
21 days. That is arithmetic for the reader, not a measurement. Measured on lab
containers, a slot past the cap reads `unreserved` and is invalidated at the
next checkpoint, so the budget is the cap over the rate, with no allowance
(below). The H8 record
keeps the readings, each site's rate, and the budget computed from them.

#### Measured on lab containers

Status: the window and what a returning node sees are measured on two
postgres-spock containers (`ops/postgres/spock/tests/slot-offline-window/run.sh`,
four runs on 2026-11-15); the re-seed procedure is measured on the same
containers; the re-seed in `goatbuilder` is designed, not built. Nothing in
this part is measured on a phone or a live goatnet. Operator decision,
2026-11-15 (gate group D, D2): a phone stays a full `core` joiner for
goatbuilder 1.0, and these are the guards asked for before H8.

#### How it was measured

`ops/postgres/spock/tests/slot-offline-window/run.sh` starts two sites from the
postgres-spock image, a founder (A) and a phone (B), and replicates one table
both ways, as `spock-mesh-peering` does: no initial copy, no forwarded origins.
Then:

1. B leaves the network (`docker network disconnect`). It keeps running and
   writes 10 rows of its own, as a phone off its uplink does. A writes 50
   replicated rows that B does not see.
2. A writes WAL at a stated rate into a table it does not replicate. Any WAL
   counts against a slot, whatever table it comes from. Once a second, the run
   reads A's slot for B from `pg_replication_slots`: `wal_status`,
   `safe_wal_size`, the WAL it holds, and whether it is active.
3. When `wal_status` reads `lost`, B returns: by the network, or by a restart
   (`RETURN=restart`, a phone that rebooted). The run reads B's subscription
   status every 5 seconds and both sites' logs.
4. B is seeded again (below), and the run compares both sites' rows by count
   and digest.

The inputs were frozen by copy: the image was built from `ops/postgres/spock`
at main `e106d420`, and each run used a copy of the script. The image reported
the cap as 1 GiB, `max_wal_size` 1 GB, `checkpoint_timeout` 5 min and 16 MB WAL
segments.

| Run | Rate asked | Rate made | B returned by | Slot `unreserved` | Slot `lost` | B's status read `down` | Result |
|---|---|---|---|---|---|---|---|
| 1 | 16 MiB/s | 13 MiB/s | network | 78 s, 1,088,711,280 bytes | 115 s, 1,614,458,480 bytes | not within 45 s | a first version of the script; see "Two findings" |
| 2 | 16 MiB/s | 14 MiB/s | network | 70 s, 1,081,801,536 bytes | 103 s, 1,600,834,728 bytes | 21 s after B returned | PASS |
| 3 | 32 MiB/s | 14 MiB/s | restart | 72 s, 1,118,848,624 bytes | 105 s, 1,601,627,184 bytes | 6 s after B returned | PASS |
| 4 | 4 MiB/s | 4 MiB/s | network | 237 s, 1,083,541,640 bytes | 347 s, 1,600,192,720 bytes | 119 s after B returned | PASS |

The byte counts are the WAL A wrote after the cut. Run 3 asked for more than
the inserts could make; the rate made is read from A's WAL position.

#### What the measurement shows

**The slot is safe up to the cap, and lost at the next checkpoint after it.**
In every run the slot read `unreserved` once it held more than the cap, and
read `lost` only when a checkpoint then ran and removed the WAL. Between the
two the slot could still have been read, had B come back. How long that gap
lasts depends on when the next checkpoint runs: within `checkpoint_timeout`,
or sooner when enough WAL has been written. In these runs it came at about
1.6 GB in every case, whatever the rate. A phone's budget must not count on
that gap. **The budget is the cap divided by the founder's WAL rate.**

So a phone may be off for about the cap divided by the WAL rate of the busiest
site that keeps a slot for it. The rate is read on each site as
"How long a phone may be off", above, describes. The measurement here does not give
a goatnet's rate, which comes from H8's week. As arithmetic: at 2 MiB an hour,
a 1 GiB cap lasts about 21 days, and at the F-093 rate in the Spock README,
about three hours.

**What the phone sees when it comes back.**
- For a while, nothing is wrong. B's subscription from A read `replicating`
  for 21 s and 119 s after B returned by the network, and 6 s after a restart.
  That is how long the apply worker took to find its old connection dead.
- Then the subscription reads `down`, and every few seconds B's log shows
  `could not send replication command "START_REPLICATION SLOT ..."` with
  `ERROR: can no longer get changes from replication slot`. A's log shows the
  same error with `DETAIL: This slot has been invalidated because it exceeded
  the maximum reserved size.`
- **B's own writes are not lost.** The slot B keeps for A is on B, and B wrote
  little while away, so it was not invalidated. In every run all 10 of B's
  offline rows reached A after B returned.
- **B never receives what A wrote while it was away** (0 of the 50 rows in
  runs 2 to 4), nor anything later. The two sites diverge until B is seeded
  again.
- On the founder, the slot reads `wal_status = lost`, `active = f`, with no
  `restart_lsn`. `SpockSlotInvalidated` alerts on exactly that
  ([replication-consistency-observability.md](replication-consistency-observability.md)).

#### Two findings from the first run

1. **`sub_create` with `synchronize_data := true` does not seed a node that
   still holds rows.** Run 1 dropped B's subscription and created it again
   with the initial copy. The copy failed on B's existing rows (`duplicate key
   value violates unique constraint`), and the subscription still read
   `replicating` with the table not copied. The re-seed below copies each
   table again after a local truncate instead.
2. **A re-seed writes the copied data as WAL on the phone, and that WAL counts
   against the slot the phone keeps for the founder.** In run 1 the replicated
   table held about 1.4 GB, because the load went into it. The copy into B
   wrote that much WAL on B within about a minute, and B invalidated its own
   slot for A (`can no longer get changes from replication slot
   "spk_netbird_site_b_sub_b_to_a"`). So a re-seed is safe only when the data
   it copies is well under the phone's own cap. A goatnet's control-plane
   databases are far smaller than that. Runs 2 to 4 put the load in a table
   that is not replicated, and the copy wrote 30 KB to 60 KB of WAL.

#### Seeding a phone again

Measured on lab containers in runs 2 to 4. The order matters: the phone's own
writes reach the founder first, and only then is the phone's copy replaced.

1. **Confirm the slot is lost**, on the site that held it:
   `SELECT slot_name, active, wal_status FROM pg_replication_slots;` reads
   `lost` for the phone's slot, which is named
   `spk_<db>_<provider node>_sub_<provider node>_to_<phone node>`.
2. **Let the founder take the phone's own writes.** The founder's subscription
   from the phone must read `replicating` and have caught up to the phone's
   current WAL position. Truncating the phone's tables before then would lose
   whatever the phone wrote while it was away.
3. **Check the size.** The data to copy, in total, must be well under the
   phone's own cap (finding 2): check `pg_total_relation_size` of the
   replicated tables on the founder.
4. **On the phone, for each replicated database:** drop the subscription from
   the founder, which also drops the lost slot on the founder
   (`SELECT spock.sub_drop('sub_<founder>_to_<phone>');`). Create it again
   with the parameters `spock-mesh-peering` uses, without the initial copy.
   Then copy each table of its replication sets again, truncated first
   (`SELECT spock.sub_resync_table('sub_<founder>_to_<phone>', '<table>',
   true);`). The truncate happens on the phone only and is not replicated. In
   run 2, the founder's rows were untouched.
5. **Wait** until every table reads `r` in `spock.local_sync_status`, and the
   phone's slot for the founder still reads `reserved`.
6. **Check convergence**: run drift-watch, as the convergence runbook's step 4
   does. In runs 2 to 4 both sites held the same 161 rows with the same digest.

The operations form of this procedure is in
[post-partition-convergence-runbook.md](../operations/post-partition-convergence-runbook.md),
"A. Slot rebuild". The `pg_basebackup` rebuild there copies the provider's
whole data directory, and with it the provider's Spock node identity. It
applies to a streaming standby, not to a Spock member such as a phone node.

#### The re-seed in goatbuilder

**Designed, not built.** It is more than a small change: a new verb, run
against a node's database over the mesh, with refusals and a journal. It is
listed for gate row H8's prerequisites.

```
goatbuilder join-node <node> --reseed [--dry-run]
```

It runs from the operator's machine against the phone and every site that kept
a slot for it, over the same path `join-node` uses. Each step below is a
refusal with its own catalog code when its condition does not hold:

| Step | What it checks or does | Refuses when |
|---|---|---|
| 1 | Reads every replicating site's slots for the node | no slot for the node reads `lost`: nothing to re-seed, and a re-seed of a healthy node would only cost a copy |
| 2 | Reads each site's subscription from the node, and the node's WAL position | a site has not caught up with the node's own writes: they would be lost by the truncate |
| 3 | Sums the replicated tables' sizes on the site it copies from, and reads the node's live cap | the data is not well under the node's cap (finding 2): the copy would cost the founder its slot on the node |
| 4 | Drops and creates each subscription from the site whose slot was lost, with `spock-mesh-peering`'s parameters, and runs `sub_resync_table` with truncate for each table of its replication sets | |
| 5 | Waits for `spock.local_sync_status` to read `r` for every table, and for the node's slots to read `reserved` | a table fails to copy, or a slot on the node reads `lost` |
| 6 | Runs drift-watch between the node and the site | a table differs |

`--dry-run` stops after step 3 and prints what it would do. Each step is
journaled, as `federate` journals its phases, so a run that stops resumes at
the step it stopped at. The verb runs only when asked: it never re-seeds
because an alert fired, since an operator decides that a node is to be copied
again (runbook, "What this runbook does NOT do").

Not built: the verb, its catalog codes, the journal, and a test against the
harness above. Until it is built, the operator runs the procedure above by
hand.

### What to keep (H8)

| Field | Content |
|---|---|
| Days | each day's integrity exit code or line, and the node's `status` rows |
| Reboot | the three times, and whether a person touched the phone |
| Slots | each reading of the two queries, per site |
| Budget | each site's WAL rate and live cap, and the phone's off-time budget |
| Unmeasured | anything not watched: battery, Doze, heat, a mobile-data uplink (phone-node.md, "Staying up") |

The readings name slots, sites and hosts, so they go to the operator's private
record. The gate row records the seven daily verdicts, the reboot's recovery
time, whether a person acted, and the off-time budget with the cap and rate
it was computed from.

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
| The per-phase line of the designed form (`phase 7/14 <name>  <start> → <end>  <duration>  <outcome>`) | designed |
| A `took` line and a timing-file row per step, for `standup`, `join-node` and `teardown` | built (`record_timing`, `crates/goatbuilder/src/standup.rs:729`; `join-node` at `crates/goatbuilder/src/joinnode.rs:851`; `teardown` at `standup.rs:1823`) |
| The process summary line, for all three | built (`timing_summary`, `standup.rs:868`; `join-node` at `joinnode.rs:872`; `teardown` at `standup.rs:1831`) |
| The founder run's numbers across resumes: total since phase 1, phase 11, total without it, the stops and the slowest phases | built for `standup` (`run_evidence`, `standup.rs:777`) |
| Phase 11 recorded as a row | built (`record_stop`, `standup.rs:736`, and `close_operator_stop`, `standup.rs:759`) |
| A cause code on every stop | built for a failed step of all three verbs; a refusal before the first step writes no row |
| The image pull, the observability legs and their joins as rows | designed |
| A generator for a full goatnet's cloud tree | not built |

None of the three clocks has been measured on hardware, and neither tutorial
has been followed on hardware. Two parts of the tutorial runs are not built:
- a command for tutorial 2's checkpoint 3 write and deletion across the sites;
- federation-acceptance.md §6.4, which a two-party tutorial 3 run needs for a
  PASS.
