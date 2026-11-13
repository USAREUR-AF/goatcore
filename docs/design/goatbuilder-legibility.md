# goatbuilder legibility: the tool shows its work (contract clause C11)

| Field | Value |
|---|---|
| Status | **Designed.** Step 1, the conformance test, is **built** (2026-11-12). Step 2a, the runner, and step 4, the run record, are **built** with unit tests only (2026-11-12): `crates/goatbuilder/tests/legibility.rs`, with the checks that fail today listed in `crates/goatbuilder/tests/legibility-known-failing.txt`. Every feature of steps 2 to 9 is **designed**; each feature's pull request changes its row below. Step 3 is **built** for the output pages (2026-11-12); its probes and federate's page are designed. |
| Design | [`goatbuilder.md`](goatbuilder.md) (the contract, C1 to C10) · [`goatbuilder-conformance.md`](goatbuilder-conformance.md) (the contract as tests) · [`goatbuilder-output.md`](goatbuilder-output.md) (the output standard) |
| Audience | whoever builds or reviews goatbuilder's runner, dry runs, run records, phase catalog and failure messages |

## What this is

goatbuilder stands up a goatnet, adds a node, deploys a component and federates two goatnets. It takes a lot of complexity away from the person running it. It must not hide that complexity where nobody can read it. This document is the design of record for that rule: contract clause C11, the conformance test that proves it, and the nine steps that build it.

## The rule

Nothing goatbuilder does or decides is hidden from the person running it. A person can answer four questions from the tool itself, without opening its source:

1. What will it do?
2. What is it doing now?
3. What did it do?
4. Why? Why this step exists, why this value was used, and why it stopped.

## C11, legibility

**C11.** For every action the tool takes, its command, its inputs with their sources, and its reason can be read from the tool before, during and after the run.

C11 is a new group of the 1.0 gate. Its proof is the conformance test below: C11 holds when all four checks pass for all four verb families and the known-failing file is empty. Step 8 records C11 beside C1 to C10 in [`goatbuilder.md`](goatbuilder.md) and in [`06-contract.md`](../reference/goatbuilder/06-contract.md).

Decided with the clause:

- Default output is three lines for each phase that succeeds: what the phase is, what it runs, and where its log is. Detail is one flag or one command away. Legibility must not slow the tool or bury the person in output.
- Run records live beside the existing state directory. They are kept until the operator removes them.
- No run record and no output ever contains a secret value. A secret shows its source only.
- This work changes what the tool says and records. It does not change what any phase does.

## What the tool does today

Surveyed on main at 26f9fd2a (goatbuilder 0.5.1). Each claim is checked against the code it cites.

| Question | Today |
|---|---|
| What will it do? | A dry run prints every phase. At a terminal, join-node and teardown print a plan page instead of every row (`crates/goatbuilder/src/output.rs:69-70`), and the page says "add -o json to see every phase's command" (`crates/goatbuilder/src/output.rs:774`). federate prints its own plan page whenever the output is a table (`crates/goatbuilder/src/federate/mod.rs:840-844`), with the same advice (`crates/goatbuilder/src/federate/mod.rs:505`). deploy's terminal page says "-o json lists them" in place of its commands (`crates/goatbuilder/src/output.rs:1050`). standup's terminal page does print each command, reflowed to the terminal width, and the page rewrites " — " to ": " inside the text it prints, commands included (`plain`, `crates/goatbuilder/src/output.rs:274`). |
| What is it doing now? | Each verb collects rows into a report that main.rs prints after the verb returns (`crates/goatbuilder/src/main.rs:1503-1504`). During a run the terminal shows only the children's own output; phase headings and command rows arrive at the end. `-v` logs a command through `trace::cmd` (`crates/goatbuilder/src/trace.rs:38`), which only `crates/goatbuilder/src/push.rs:369`, `crates/goatbuilder/src/release.rs:145`, `crates/goatbuilder/src/release.rs:156`, `crates/goatbuilder/src/release.rs:232`, `crates/goatbuilder/src/federate/measure.rs:460` and `crates/goatbuilder/src/federate/measure.rs:493` call. The phase runners of standup (`crates/goatbuilder/src/standup.rs:786`, `crates/goatbuilder/src/standup.rs:952`), teardown (`crates/goatbuilder/src/standup.rs:1397`), join-node (`crates/goatbuilder/src/joinnode.rs:317`, `crates/goatbuilder/src/joinnode.rs:340`) and federate (`crates/goatbuilder/src/federate/mod.rs:800`) never call it. |
| What did it do? | join-node leaves no record. standup leaves a timings file (`crates/goatbuilder/src/standup.rs:608`). deploy leaves host ledgers, and writes each play's output to `$GOAT_DEPLOY_LOG_DIR`, else `$TMPDIR`, else `/tmp` (`crates/goatbuilder/src/push.rs:527`), not beside the state directory. federate leaves `state.json` and evidence in its working directory. |
| Why this value? | Sources are shown only for federate's `FED_*` inputs, and only as `-o json` rows of a dry run (`crates/goatbuilder/src/federate/preflight.rs:539-545`, `crates/goatbuilder/src/federate/mod.rs:846`). Nothing shows a source for standup's profile, for join-node, for deploy, or for federate's site, host and allocated values. |
| Why did it stop? | An error that ends the program is matched against `ops/errors/catalog.yaml` (`crates/goatbuilder/src/main.rs:1511-1520`). The output of a failed phase is not. |

Two more facts the conformance test found:

- A dry run can start processes. deploy's dry run runs `ops/scripts/served-payloads.py` to say what a surface serves (`crates/goatbuilder/src/push.rs:695`). standup and deploy start probes on a real run that no dry run prints: the secrets posture (`ops/secrets/posture.py available`) and the fresh-source check (`ops/scripts/fresh-source.sh`).
- `-o ndjson` and `--narrate` are described in [`goatbuilder.md`](goatbuilder.md) (`docs/design/goatbuilder.md:517`, `docs/design/goatbuilder.md:540-541`) and are not built.

## The conformance test (step 1)

**Built.** `crates/goatbuilder/tests/legibility.rs`. Run it with:

```bash
cd crates && cargo test -p goatbuilder --test legibility -- --nocapture
```

`--nocapture` prints one line per check: passes, or known failing with the first line of why. `GB_LEGIBILITY_DETAIL=1` prints every failure in full and each terminal dry run the test read.

### What it runs

The C1 to C10 tests run on the `--mock` fixtures (`crates/goatbuilder/src/verbs.rs`, `crates/goatbuilder/tests/flows.rs`). A `--mock` run starts no process, so it cannot show which processes a real run starts. The C11 test therefore runs the real code paths on two stand-ins that reach no host:

| Verb family | Stand-in | Real run | Dry run read |
|---|---|---|---|
| standup | a checkout whose `ops/goatbuilder/goatbuilder.yaml` is the standup fixture's (`ops/goatbuilder/fixtures/standup`), site `alpha` on environment `dev` | `standup dev --site alpha`, phases 1 to 11 (11 is the first manual step) | the same, `--dry-run` |
| join-node | the same checkout, with `deploy.ssh_user` declared | `join-node dev --site alpha --node node-x --class core --address 192.0.2.10 --from 2`, phases 2 to 8 (1 and 8 are manual) | the same with `--to 8 --dry-run`; the catalog check reads the full plan, phases 1 to 15 |
| deploy | the same checkout, with one goatnet, one play, one inventory and one pin for `portal` | `deploy --env dev --component portal` | the same, `--dry-run` |
| federate | the stand-in repository of `crates/goatbuilder/tests/federate_world/mod.rs`, peering `hello-ally` between `xaa` and `xab` | `federate hello-ally` (phases 1 to 5), then `--from 6 --to 9` | the same two ranges, `--dry-run` |

### The recorder

No code in goatbuilder changed to make the recorder possible. It works below the tool, so it does not trust what the tool says about itself:

- The tool's `PATH` is one directory, and every name in it is a link to the recorder: every program name on the test machine's `PATH` (a symbolic link counts when it leads to an executable file, as `sh`, `env` and `awk` usually do), plus `sh` and the names the tool may start that a test machine may lack (`ansible-playbook`, `netbird`, `aws-vault` and others). So every program the tool starts by name is recorded, and none slips past to the real one. In the standup checkout every file under `ops/scripts/` is the recorder too, so a script started by its path is recorded.
- Each invocation writes its parent's process id, its own process id and its arguments to a file, and appends the file's name to an order file. The test reads the processes back in the order they started.
- Only a process whose parent is the goatbuilder process counts. Those are the processes the tool started. A shell given one simple command replaces itself with that command under the same process id; the first record of a process id is kept.
- For standup, join-node and deploy, the recorder stands in for the program and exits 0, because those commands reach hosts. For federate it records the program and then runs the real one of the same name from the test machine's `PATH`, because the stand-in repository's stubs write the files the next phase reads. A name with no real program fails loudly (exit 127, and a message naming it). The recorder's own tools (`mktemp`, `od`, `tr`) come from the saved `PATH`, not the tool's.
- Check (c) reads the dry run as a person sees it at a terminal: the dry run runs under `script(1)`, which gives it a pseudo-terminal. Colour codes are removed before the comparison.

### The four checks

| Check | Holds when | How the test decides |
|---|---|---|
| (a) | every process the tool started is in the run record, with its command and its log path | each recorded process matches a `commands` entry of a `runs/<id>/run.json` written under the test's own temporary root, by `argv` or by its command line, and that entry's `log` is a file that exists |
| (b) | every value in a command line has an entry in `inputs`, with a source | each value in each recorded command line has an `inputs` entry with a non-empty `source`. A value is, by a fixed rule: the right side of `-e name=value` and of `--flag=value`; the file of `-e @file`; the profile after `with-aws`; and the word after `-l`, `--limit`, `-i`, `--inventory`, `--site`, `--profile`, `--tags`, `--skip-tags`, `--repo`, `--address`, `--node`, `--class`, `--env`, `--component`, `--out` or `--with`. A `;` the shell ends a word with is not part of its value. A value read from the environment (`$(printenv NAME)`, `$NAME`, `${NAME}`, or a play's `{{ lookup("env", "NAME") }}`) needs an entry named `NAME`, whose value may be withheld as secret |
| (c) | the commands a dry run prints are the commands the real run executes, in order | each recorded command line appears in the terminal dry run, after the one before it, once quoting, line continuations and runs of whitespace are folded away. A parallel step (a phase marked `∥`; today only standup's image pull, `Image pull · A ∥`, whose plan row is `crates/goatbuilder/src/standup.rs:877` and which runs in the background) starts when the runner reaches it, which races the steps beside it. So a command that is exactly one of that step's commands, as the `-o json` plan lists them, must be printed but not in order; every other command keeps its order, even where its text also appears inside the parallel one |
| (d) | every phase that can run has a phase-catalog entry, and every document path the tool prints exists | each phase and observability leg in the `-o json` dry run's plan (its `phase` and `leg` rows: `=== Phase 5 [A] ...` is phase `5`; a titled step is its words before `[` or `(`, lowercased and hyphenated, quotes dropped, so `=== Image pull [...] ...` is `image-pull` and `=== Observability leg "host" [...] ...` is `observability-leg-host`) has an entry for its verb in `ops/phases/catalog.yaml` with every field below filled and its `read` path present; and every `docs/...md` path in anything the verb printed (the real run, both dry runs, `--help`) is a file in the repository. A path is checked in every form it is printed in: `docs/x.md`, `./docs/x.md`, `../docs/x.md`, an absolute path into a checkout, `goatcore/docs/x.md`, and this repository's own `blob`, `tree` or `raw` URL. Only a path inside a URL to another repository is left to that repository's link checks. deploy has no numbered phases yet, so the test names its six steps: prep, fresh-source, transport, playbook, deploy, verify (the runner records five; playbook is a row, not a phase) |

One more test, `c11_each_check_can_pass_when_the_tool_does_what_it_asks`, gives each check a run record, a page and a catalog that hold what C11 asks, and asserts each check passes; then gives each the shape it exists to catch, and asserts each fails. A feature is judged by a check that something can satisfy.

### Known failing, and how the list only shrinks

`crates/goatbuilder/tests/legibility-known-failing.txt` lists each check that fails today, one line per check and verb family: `<check> <verb> <why>`. The test reads it:

- A listed check that fails is reported as known failing, and the test passes. CI stays green while the features are built.
- A listed check that passes fails the test, with a message naming the line to remove. The pull request that makes a check pass must remove its line.
- An unlisted check that fails, fails the test. Nothing new can start failing quietly.
- `c11_known_failing_list_is_well_formed` refuses a line that names no check or verb, gives no reason, or repeats another.

A harness failure is never read as a known failure. The test fails whatever the list says if:
- a real run exits with an unexpected code, or the recorder saw no process it started;
- a dry run at a terminal, the `-o json` plan or `--help` exits other than 0 (`script -e` hands back the dry run's own code), or the plan is not a JSON report;
- `script(1)` is missing;
- a `run.json` under the test's root is not JSON.

The self-test holds the terminal case to it: a dry run with an unknown flag fails the harness.

### What fails today

Checks (a) and (c) pass for all four verb families, from steps 2b and 3, and check (d) for federate, from its phase catalog: 7 lines.

| Check | standup | join-node | deploy | federate |
|---|---|---|---|---|
| (a) | passes | passes | passes | passes |
| (b) | the record's `inputs` is empty | the record's `inputs` is empty | the record's `inputs` is empty | the record's `inputs` is empty |
| (c) | passes: each command prints verbatim (step 3), and each probe under its heading | passes (step 3) | passes: the play is listed (step 3), and each probe under its heading | passes: the plan page lists each phase's command |
| (d) | no phase catalog | no phase catalog | no phase catalog | passes: the phase catalog has its 14 phases, and the dry-run footer names the staged-delivery pattern by its address in goatdocs |

### What the test does not cover

- It runs one stretch of each verb, up to the first manual step (standup 1 to 11, join-node 2 to 8, federate 1 to 9). Phases after that are covered by check (d), which reads the whole plan, but not by (a), (b) or (c).
- A phase that the fixture's declaration skips (an obs leg, a break-glass node) is not planned, so (d) does not ask for its entry, and (c) never runs it. The standup fixture declares no obs host, so the observability legs (`leg` rows, read by (c) as parallel steps and by (d) as steps) are covered by the self-test only.
- A program started by an absolute path that is not a recorder is not seen, for every verb. In the standup checkout, a path under `ops/scripts/` is a recorder, and any other path into the checkout does not exist, so starting it fails the run loudly. Outside the checkout, `endstate::python()` can return the interpreter named on the first line of `ansible-playbook` (`crates/goatbuilder/src/endstate.rs:41`), and `$GOATBUILDER_PYTHON` can name any path; neither happens on these stand-ins, where `python3` is a recorder. In the federate stand-in, the repository's own stubs are started through `bash` today, which is recorded; a stub started directly by its path would not be.
- Check (c) asks that every command that ran was printed. It does not ask that every printed command ran; step 3 builds both from the same code.
- The value rule of (b) is a floor. It does not read positional arguments.
- The macOS form of `script(1)` (`script -q /dev/null <command>`) is written but has not been run; the test was run on Linux.

## The runner

**Built** (step 2a), unit tests only: `crates/goatbuilder/src/runner.rs`. Step 2b moves the call sites to it; until then no verb's command goes through it except main.rs's own.

- `runner::Cmd` is built as `std::process::Command` is (`arg`, `args`, `env`, `envs`, `env_remove`, `env_clear`, `current_dir`, `stdin`, `stdout`, `stderr`), plus `host`. It runs a command three ways: `status` streams the child's output and waits (inside a run, under `-o json` or `-o yaml`, the child's stdout streams to stderr, so stdout stays the one report a script parses; its log is written either way. Outside a run, as under `--mock` or in a verb main.rs does not begin a run for, and for a command that keeps the terminal, the child inherits stdout as `Command` gives it, so a child that prints there still reaches stdout. federate's real runs are always inside a run), `output` captures it, `spawn` starts it in the background. It has no way to hand out the `Command` inside it, so nothing can run one around the door.
- `runner::begin` starts a run: main.rs calls it for every verb that drives hosts, except under `--mock`. Inside a run, every command is logged under `-v`, framed, copied to a log at `<state>/runs/<id>/logs/<nnn>-<phase>.log` (made by the first command, mode 0600), and kept as a command entry: phase, command, argv, host, log, start, end, exit, seconds. `runner::end` hands the entries back; step 4 writes them to `run.json`. Outside a run, `Cmd` does exactly what `Command` does.
- `runner::phase(id, name, purpose)` says which phase the next commands belong to. Until the phase catalog exists, a phase's purpose is its existing label.
- The frame goes to stderr, so `-o json` stays a clean document. A phase that succeeds takes three lines; its first two print before the child can print anything (the child's output reaches the terminal only through the runner):

  ```text
  Phase 5 · mgmt-stack: site-bootstrap up to the account seed
    $ cd ops/ansible && ../scripts/with-aws default -- ../scripts/ansible-preflight.sh ...   (on alpha_hosts; log ~/.goat/state/runs/<id>/logs/007-5.log)
    ok in 12s
  ```

  A failure reads `FAILED: exit 2 in 12s; log <path>`. A command outside a phase (a probe: the fresh-source check, the secrets posture, release's `git` reads) prints nothing when it succeeds, so a verb's probes do not bury its phases: it is in the record, and logged under `-v`. When a probe fails it prints one line naming it, `· FAILED: <command>: exit 2 in 0.1s; log <path>`.
- To log while streaming, `status` and `spawn` pipe the child's stdout and stderr and copy each to the terminal and the log as it arrives; stdin stays inherited, so a prompt still works. The trade-off: the child no longer sees a terminal on its output, so a tool that colours or animates only at a terminal (ansible's colour, for one) prints plainly. `GOATBUILDER_CHILD_TTY=1` gives a streamed command the terminal as before, and its entry then says no log was written. That is decided before the frame prints, so a command that keeps the terminal (this, or a program on the interactive list) is framed `no log: the command keeps the terminal`, and neither the frame nor the record names a log. A captured command's log is made before the command starts, so one that cannot start still names a file that exists. A caller that sets `stdout` or `stderr` itself keeps that stream; the runner neither copies nor logs it, and the entry's `log_note` says which stream is the caller's. When both are, the entry names no log and the frame says `no log: the caller's own streams`.
- A command that cannot start is recorded with no exit code and the note "did not start".
- The door covers `crates/goatbuilder/src`. A library goatbuilder runs in its own process is outside it: goat-federation, for one, starts a process of its own during federate's phase 3 (`crates/goat-federation/src/derive.rs`). Such a process is not framed or recorded until that library takes a runner too.
- Two gates hold the door. `crates/goatbuilder/tests/legibility_one_door.rs` counts the ways of building std's `Command` in each file outside its test modules (`Command::new`, `std::process::Command::new`, `<std::process::Command>::new`, the constructor taken without a call, and any name the file gives it with `use … Command as X` or `type X = Command;`), with strings and comments removed first, and holds each file to its count in `crates/goatbuilder/tests/legibility-one-door-allow.txt`. `ops/ci/check-one-door-allow-shrinks.py` holds that list to the pull request's base: a count may only go down, and a new file may not appear. A base it cannot read as a commit fails the gate rather than passing it. A call that moves to a new file is not an exception: the change that moves it starts it through `runner::Cmd`. The count sees constructions, not callers: a helper that builds std's `Command` and hands it back (`federate::runenv::command`, which every federate call site goes through since #498) counts once, in its own file, however many places call it. Step 2b's federate group makes it return a `runner::Cmd`.

**Designed**, for timed runs (from the timed hardware run of contract clause C1): the frame and the record carry what a measurement needs, so a run can be timed from the tool's own output.

- Each phase prints its number and name, its wall-clock start and end, its duration and its outcome. The three-line frame carries the start on its first line and the end on its last.
- A run ends with one summary line: the total time and the three slowest phases.
- The run record keeps the same per phase, and every stop records its cause as a code (an error catalog code, or `manual` for a step the operator does), so a measurement can count the stops by cause.
- A step that waits on a person (standup phase 11's browser login, federate's confirmations) records the wait apart from the phase's own time.

### Children that need a terminal

Piping a child's output (§"The runner") is safe for a child that prompts through `/dev/tty` or through its stdin, because the runner leaves the controlling terminal and stdin as they were. It is not safe for a child that decides whether to prompt from whether its stdout or stderr is a terminal: piped, it would stop prompting, and its phase would do something else. A lost colour is acceptable; a prompt that silently disappears is a changed phase.

Such a program goes on a reviewed list, `crates/goatbuilder/tests/legibility-interactive.txt`, one line per program with why it needs the terminal. The runner reads the list (built in): a streamed command whose line names a listed program keeps the terminal, writes no log, and its entry's `log_note` reads `not captured (interactive): <why>`. Check (a) accepts a command with no log only when its line names a listed program and its entry says so. Any other command with no log fails the check, whatever its note says, including a run with `GOATBUILDER_CHILD_TTY=1`. Adding a line is a reviewed change, like the known-failing list.

The survey (2026-11-12) looked at every child a verb that drives hosts starts, and at what those children start in turn. **No child needs the terminal, so the list is empty.** This is designed, not measured; the first live run of each verb after step 2b measures it.

| Child | How it asks for anything | Piped |
|---|---|---|
| `ssh` (join-node phases 2 and 4, rollback's host reads) | a host-key confirmation or a password, through `/dev/tty` | unchanged |
| `ops/scripts/seed-ssh-key.sh` | the password comes from the environment variable `--pass-var` names, handed to `sshpass`, which makes its own terminal | unchanged |
| `sudo` on a host, through `ssh ... sudo` | no prompt: the plays rely on NOPASSWD sudo, and without `ssh -t` sudo has no terminal today either | unchanged |
| `ansible-playbook` (every play, through `ops/scripts/with-aws` and `ansible-preflight.sh`) | no `-K`, `--ask-become-pass` or `--ask-vault-pass` is passed; ansible's own prompts use `getpass`, through `/dev/tty`. Its colour follows stdout, so it prints plainly | colour lost |
| `aws-vault exec` (inside `ops/scripts/with-aws`, `GOAT_AWS_MODE=aws-vault`) | an MFA code or a keyring password, written to stderr and read from stdin; on macOS a keychain dialog. The prompt reaches the terminal through the runner's copy, which forwards each read as it arrives, without waiting for a newline | unchanged; unmeasured |
| the signing tools (`go run ./cmd/name-snapshot-sign`, standup phase 4; `intent-sign`) | no prompt: the age identity is a file named by `--age-identity`, read with `age.ParseIdentities` (`ops/enrollment/cmd/name-snapshot-sign/main.go:83-87`) | unchanged |
| `goat federate init`, `lint`, `compose`, `verify`, `apply` (`FED_VERBS=goat`) | no prompt; `--confirm` is passed. goat's colour and its update notice follow stdout and stderr | colour and notice lost |
| `ops/scripts/pr-land.sh` (fastlane) | every `gh` call is given all its values, so `gh` never prompts; its stdout already goes nowhere | unchanged |
| `git push` (release, fastlane) | credentials through a credential helper or `/dev/tty` | unchanged |
| federate phase 10 (admitting the edge) | the prompt is goatbuilder's own, in the same process, not a child | not a child |
| the manual phases (standup 11, join-node 1, 8 and 10, federate 5 and 10) | not run by the tool | not a child |
| `goatbuilder secrets exec -- <command>` | the operator's own command, which can be anything | not in a run: `secrets` is not a verb that drives hosts, so its command inherits everything as before |
| `ops/goatbuilder/endstate/endstate.py` (under `assay`) | no prompt; it prints a table only when its stdout is a terminal | output form changes; no prompt is lost |

Programs examined and not started by any verb: `ops/scripts/external-service-finish.py` (prompts with `input()`), `ops/scripts/secrets-session` (prompts when stdin is a terminal), `ops/scripts/ssh-key-backfill.sh` (reads `/dev/tty`).

## Run record

**Built** (step 4), unit tests only: `crates/goatbuilder/src/runs.rs` writes it from what the runner kept, and checks (a) and (b) read it. One record for each run of a verb that drives hosts (the verbs `long_running` names in main.rs), at `<state>/runs/<id>/run.json`, where `<state>` is the state directory the project declares (`deploy.state_dir`, else `~/.goat/state`), for every verb. The id is the UTC start time, the verb and the process id. The record is written when the verb returns, by an atomic rename, mode 0600, and kept until the operator removes it. Every run that reached its first phase, or started a process, writes one, a refusal or failure inside a phase included: that is why it stopped. A refusal of the arguments before any phase writes none, because its reason is already printed, and a run that changes nothing begins no run and writes none: every dry run, and every mode that only lists, reports or reads (`rollback --list` and `--status`, `deploy --verify-only`, `assay` with or without `--plan`, `federate plan` and the six federation verbs). `writes_no_record` in main.rs is that rule; a mode that changes anything, on a host or in the checkout, writes a record. Until step 2b moves the call sites to the runner and the phase loops call `runner::phase`, no verb's run reaches either, so no verb writes a record yet.

The tests never write into the real state directory. Every test that runs a verb points `HOME` at a directory of its own: `crates/goatbuilder/tests/common/test_home.rs` (under cargo's per-target scratch directory, keeping the real user's Python packages and git configuration), or a temporary root (the federate stand-in, the C11 test). `ops/ci/check-goatbuilder-tests-leave-home.sh` fails if anything under `~/.goat/state` is newer than a marker made before the tests ran; `goatbuilder-pr.yml` runs it after its test steps.

`goatbuilder runs [--verb V] [--last N]` lists the records, newest first. `goatbuilder show <run>` prints one, phase by phase and command by command, each with its exit code and log, then its inputs; `-o json` prints the record itself. `goatbuilder runs diff <a> <b>` compares two: arguments, inputs (value and source; a secret shows as `(secret)`), the commands in order, and each phase's exit code. A run is named by its id, a unique prefix of it, or `last`. The tree's commit is read from the checkout's git files, not by starting `git`, so the record names it without a process outside the record.

| Field | Holds | Read by |
|---|---|---|
| `schema`, `id` | `goatbuilder.run/v1`; the version changes when a field changes meaning | |
| `tool_version`, `tree_commit` | the binary's version and the checkout's commit | |
| `verb`, `args` | the verb and its arguments as typed (no verb that drives hosts takes a secret on its command line; secrets come from the environment) | the test finds a verb's records by `verb` |
| `started`, `ended`, `exit`, `error` | the run's times, its exit code, and the error that ended it, as printed | |
| `phases[]` | each phase: `id`, `name`, `purpose`, `started`, `ended`, `exit` (the first non-zero exit of its commands, else 0; null when it ran none, or one did not start), `commands` (how many), and `verdict` when the verb gave one for a phase that started no command (deploy's `verify:<group>` for a group whose deploy failed reads `not checked: the deploy failed`); `runs diff` compares a phase's exit, or its verdict when it has no exit | |
| `commands[]` | each process: `phase`, `command` (the line as a shell reads it), `argv`, `host`, `log` (its log file) or `log_note` (why there is none), `started`, `ended`, `exit`, `secs` | check (a) |
| `inputs[]` | each value: `name`, `value`, `secret`, `source`, `origin` (below) | check (b) |

One `inputs` entry is the same in `run.json` and in `goatbuilder inputs -o json` (step 5 writes the list; the run record carries it):

```json
{ "name": "FED_WG_LISTEN_PORT",
  "value": "51821",
  "secret": false,
  "source": "allocated: peering-values.yaml xab.wg_listen_port",
  "origin": { "kind": "allocated", "record": "peering-values.yaml", "field": "wg_listen_port" } }
```

- `name`: an environment variable, a flag (`--node`) or a play variable (`target`).
- `value`: null when `secret` is true, always; also null when nothing gave a value.
- `source`: one rendered line, never empty.
- `origin.kind` is one of: `file` (`path`, `line`, `key`); `derived` (`from`, the declaration, and `rule`); `environment` (`var`); `default` (`rule`: why the default belongs to no deployment); `allocated` (`record`, `field`, `recorded`: whether it is in the record yet, and `line`, absent when allocated afresh and not yet recorded); `argument` (`flag`: the operator typed it); `missing` (nothing gave a value: `value` is null, `source` reads `missing: <where to declare it>`, and `declare` names where).
- `unset`: present when an entry read from the environment found nothing set. The entry keeps its real origin (the environment, or the store as `derived`) and `unset` says where to set it.

Check (b) reads `name`, `value`, `secret` and `source`. It also fails an entry whose `secret` is true and whose `value` is not null.

## Phase catalog

The interface step 6 builds, and that check (d) reads: `ops/phases/catalog.yaml`, one file for every phase of every verb.

```yaml
schema: goat-phases/v1
phases:
  - verb: standup
    phase: 5            # an integer; a titled step is a string, such as image-pull
    what: the mgmt stack, up to the account seed
    why: ...
    changes: ...        # what it changes, and where
    verify: ...         # how to tell it worked
    runs: ...           # the script or play it runs
    by_hand: ...        # the command to run it by hand
    read: docs/operations/goatnet-standup.md   # one repository path, optional #anchor
```

Check (d) needs `schema: goat-phases/v1` and, for each planned phase, an entry with the seven fields above non-empty and `read` naming a path that exists. Other fields (`title`, `kind`, `stage`, and `failed_because` and `then` for the runbook's failure table) are the catalog's own; the check ignores them. The rendered runbook tables also read `verify` (the "Worked when" column) and `what` (the deploy table's column). The phase ids the test asks for are the plan's numbers, plus titled steps hyphenated: standup's `image-pull` and `join-the-image-pull-before-phase-5`; and deploy's steps `prep`, `fresh-source`, `transport`, `playbook`, `deploy`, `verify`.

## The steps

One pull request for each step, or for each feature within a step, each small and each with a "Reverting this" section. A feature is done when its check in the conformance test passes, not when its own unit test passes. Step 1 merges first, alone.

| Step | What it builds | Removes from the known-failing file | Status |
|---|---|---|---|
| 1 | the conformance test and the known-failing file | nothing | **built** |
| 2a | the runner, `crates/goatbuilder/src/runner.rs` (§"The runner"): `runner::Cmd`, the frame, the per-command log, the command entry; a run begun in main.rs for every verb that drives hosts; the test `crates/goatbuilder/tests/legibility_one_door.rs`, which fails on a `Command::new` outside the runner beyond the counts in `crates/goatbuilder/tests/legibility-one-door-allow.txt` | nothing | **built**, unit tests only: no call site of a verb moves in 2a, so no verb prints a frame yet |
| 2b | every call site moved to `runner::Cmd`, and `runner::phase` called where each verb's phase loop starts a phase; the allow-list emptied | with step 4: `a standup`, `a join-node`, `a deploy`, `a federate`; with step 3, the probe lines of `c standup` and `c deploy` | **built**: every call site goes through `runner::Cmd` (the one-door allow-list is empty), and each verb names its phases for the runner: standup's phases, its image pull, observability legs and phase-12 waves (whose own log files are their logs, `Cmd::logged_at`), join-node's phases and door, deploy's five steps (prep, fresh-source, transport, and `deploy:<group>` and `verify:<group>` for each group, so `runs diff` keeps each group's outcome), federate's phases. `a standup`, `a join-node`, `a deploy` and `a federate` pass. The interactive list exists, empty |
| 3 | commands in the terminal dry run, built by the same code that runs them; anything not knowable before the run is marked as such | `c join-node`, `c federate`, `c standup` and `c deploy` (removed: built) | **built** for the pages in `crates/goatbuilder/src/output.rs` (2026-11-12): the plan page (join-node, teardown) lists each phase's command under the table, verbatim and unwrapped; the deploy and release page lists each dry-run line in place of a count; standup's page prints each command as it runs, verbatim and on one line, with no word rewritten, no image reference shortened and no wrap that would change what a pasted line runs. Deploy's play line is built from the argv that runs (`ansible_cmd` in `crates/goatbuilder/src/push.rs`, through `runner::Cmd::line_from`). **Built** for federate: its terminal plan page (`plan_view` in `crates/goatbuilder/src/federate/mod.rs`) lists each phase's command under the table, verbatim and unwrapped, so a line pastes as the command it runs; the width check holds every other line of the page to 80 columns. **Built** for the probes of standup and deploy: each dry run prints each probe's line, made by the builder the probe runs (`command` in `crates/goatbuilder/src/fresh.rs`, `posture_cmd` in `crates/goatbuilder/src/secrets.rs`, `netbird_status_cmd` and `ledger_read_cmd` in `crates/goatbuilder/src/push.rs`), under one of two headings: the probes the dry run ran itself, whose answers shaped the plan (the secrets posture, and deploy's mesh status), and those only a real run starts (the fresh-source check, and deploy's ledger read, printed with the directory it runs in). **Designed**: release's lines built from the argv that runs; they are still the plan's own words for each command (`crates/goatbuilder/src/release.rs:237`, `crates/goatbuilder/src/release.rs:584`) |
| 4 | the run record for every verb (§"Run record"); `goatbuilder runs`, `goatbuilder show <run>`, `goatbuilder runs diff <a> <b>` | with step 2: the four `a` lines | **built**, unit tests only: no verb writes a record until 2b moves its call sites |
| 5 | every value with its source, verb by verb: federate's remaining values first (site values, site hosts, allocations), then standup, join-node, deploy. `goatbuilder inputs <verb> ...` and `goatbuilder why <value>`; the same list in every dry run, as a table and as JSON | `b federate`, then `b standup`, `b join-node`, `b deploy` | designed |
| 6 | the phase catalog (§"Phase catalog"); `goatbuilder explain <verb> <phase>`; the frame's "what it is for" line reads it; the runbook's phase tables, the command reference's stage text and appendix B are rendered from it and checked in CI; every document path the tool prints exists | `d standup`, `d join-node`, `d deploy`, `d federate` | built for federate: its catalog entries, the runbook's three phase tables rendered from them and checked in CI, and the document paths it prints (`d federate` removed); built for federate: `goatbuilder explain <verb> [<phase>]` (`crates/goatbuilder/src/explain_phase.rs`, called from `explain` in main.rs) answers one phase by number or id (text, or the catalog's fields under `-o json|yaml`), lists the verb's phases when none is given, and refuses a phase the verb does not have (GB-262); unit tests and five `tests/cmd/` golden cases pin it. Designed: the other verbs, the frame's line, the command reference and appendix B |
| 7 | failed phases interpreted: a failed phase's output is matched against the error catalog, and the failure row carries the code, cause, fix, log path and the command to retry that one phase by hand | none; its own check is `crates/goatbuilder/tests/federate_failed_phase.rs` | built for federate: a failed phase's log, read through the runner's command record, is matched against the error catalog (code and pattern hits only, the one nearest the end first), and the failure carries an `interpreted` row with code, cause, fix, log path and the phase catalog's command by hand; with no known error, the phase catalog's `failed_because` and `then`. Designed: the other verbs |
| 8 | C11 recorded in [`goatbuilder.md`](goatbuilder.md) and [`06-contract.md`](../reference/goatbuilder/06-contract.md) with its test named; then `-o ndjson`, the run record written as events | none | designed |
| 9 | `--step`: pause before each phase, show its catalog entry, command and inputs, and continue on a key; built on the tutor mode | none | designed |

### Step 6: the document paths that do not exist

Recorded here; fixed by step 6, not before.

- `docs/design/staged-delivery-pattern.md` is not in this repository; it is in goatdocs. federate's dry-run footer (`crates/goatbuilder/src/federate/mod.rs:848`) and `ops/scripts/peering-standup.sh` now give its goatdocs address, and check (d) for federate passes. Comments in `ops/validation/run-acceptance.py:196` and `ops/validation/acceptance-tests.yaml:807` still name it by the bare path; the tool does not print them.
- `docs/operations/node-addition.md` does not exist. goatbuilder does not print it: join-node's help and plan name `docs/tutorials/02-first-federated-node.md`, which exists. It is named by `crates/goatbuilder/CHANGELOG.md:480`, by `ops/mgmt-host-bootstrap/main.go:487` and `ops/mgmt-host-bootstrap/main.go:529`, and by `ops/validation/generate-values.py:52`, `ops/validation/generate-values.py:387` and `ops/validation/acceptance-tests.yaml:152`. Check (d) does not see it, because the tool does not print it. Appendix B now points at the tutorial; the references above are not changed yet.

## Reverting this

Step 1 adds a test, a list and this document, and changes no behaviour. Reverting its pull request removes `crates/goatbuilder/tests/legibility.rs`, `crates/goatbuilder/tests/legibility-known-failing.txt` and this document, and the `--test legibility` and `ops/phases/catalog.yaml` lines of `.github/workflows/goatbuilder-pr.yml`. Every later step depends on the test; revert them first.
