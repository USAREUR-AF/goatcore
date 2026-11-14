# goatbuilder 1.0 — what it promises and the tests that decide it

| Field | Value |
|---|---|
| Status | **Designed.** This document sets the gate. The gate is not yet met; the table in "The gate" gives the status of each requirement |
| Design | [goatbuilder.md](goatbuilder.md) · [goatbuilder-conformance.md](goatbuilder-conformance.md) · [goatbuilder-speed.md](goatbuilder-speed.md) · [ADR 1198](../adr/1198-goatbuilder-config-driven-deploy-release-cli.md) · [ADR 1225](../adr/1225-federation-belongs-to-goatbuilder-one-command-one-decision.md) |
| Audience | whoever decides that a `goatbuilder` build may be tagged 1.0, and any team that plans to depend on it |

## What this is

`goatbuilder` 1.0 is the first release that carries a promise: a deployment
that works on 1.0 keeps working on every later 1.x release without a change
to its files or to the programs that call the binary. Releases before 1.0
carry no such promise.

A build is tagged 1.0 when every requirement in "The gate" has passed its
test. Each requirement names one test, so that "ready for 1.0" is a result
a person can read and not an opinion. Nothing is added to the gate, and
nothing is removed from it, except by a change to this file.

## What 1.0 promises

Four surfaces are what other people build on. From 1.0, each one changes
only by the rule beside it.

| Surface | Where it is defined | Rule from 1.0 |
|---|---|---|
| The configuration: `goatbuilder.yaml` and the peering declaration inside it | `ops/goatbuilder/schema/goatbuilder-v1.schema.json` | A file that validates on 1.0 validates on every 1.x and means the same thing. New fields are optional and their absence means the 1.0 behavior |
| Commands, flags and exit codes | [command reference](../reference/goatbuilder/command-reference.md); [the contract, "Exit codes"](../reference/goatbuilder/06-contract.md) | No command or flag is removed or changes meaning in 1.x. Exit codes 0 to 4 keep their meaning |
| The `-o json` and `-o yaml` documents | [goatbuilder-output.md](goatbuilder-output.md) | Each document carries its version. Fields are added, never removed or retyped, within one version |
| Error codes `GB-nnn` | `ops/errors/catalog.yaml` | A code is never reused for a different fault. A retired code stays in the catalog, marked retired |

**Deprecation (proposed).** A command, flag or field that is to be removed
keeps working, with a warning that names its replacement, for at least two
minor releases, and is removed only in the next major version. The period is
still open: ADR 1228 sets out the options and leaves the choice to the operator.

**What 1.0 does not promise.** The text of tables printed for a person. The
shell scripts beside the binary, which stay documented and unsupported
([ADR 1201](../adr/1201-two-avenues-scripts-documented-cli-supported.md)).
The internals of a converger. The `goat` client, which has its own releases.

## The gate

Status words: **built** (the code is on `main`), **designed** (decided, not
implemented), **not built**, **in review** (a pull request is open),
**measured** (a named run with a record passed), **not measured** (exists and has not been exercised by the named test).

### A. The promise is written and tested

| # | Requirement | Status | Proven by |
|---|---|---|---|
| A1 | The compatibility and deprecation rules above are a decision record | built: ADR 1228 (`docs/adr/1228-goatbuilder-keeps-its-four-surfaces-and-removes-only-in-a-major-release.md`) is Accepted, with the operator's deprecation period of 2026-11-13: a deprecated item keeps working, with its warning, for at least two minor releases and at least 90 days from the release that first warns, whichever is later. The check that enforces the period is designed, not built (ADR 1228, "The deprecation period") | ADR 1228 is accepted, with the operator's choice of the period, and this section cites it |
| A2 | Configuration compatibility is a test | built: `crates/goatbuilder/tests/compat/configs/` keeps every configuration file of each published release since `goatbuilder/v0.4.1` (0.4.1, 0.4.2, 0.4.3, 0.5.1, 0.5.3, 0.6.0; 0.5.0 and 0.5.2 were never published), with the outcome that release's own binary gave it. `tests/config_compat.rs` runs them through `validate` in `goatbuilder-pr` on every pull request, and each refusal prints the code its release printed or one the documents name: GB-025, or GB-245 for a file that is not YAML. No accepted file may come to be refused. `goatnet.toml` is the goat client's file and is not covered | A kept set of configuration files from each release since `goatbuilder/v0.4.1`, the first tagged release (ADR 1228: no released binary or tagged tree exists for 0.1.0 to 0.4.0), run through `validate` on every pull request. Each file validates, or is refused with the catalog error the release notes named |
| A3 | The command surface is a test | built: `ops/ci/check-goatbuilder-compat.sh`, in `goatbuilder-pr`, compares the build's commands, flags, positional arguments and the values each flag accepts with those of the last release (`goatbuilder/v0.6.0`, recorded from its published binary). It fails on a removal or a renamed flag without an entry in `ops/goatbuilder/compat/deprecations.yaml`. The warning a deprecated flag prints is not built | A check that compares the commands and flags of the build with those of the last release and fails on a removal or a renamed flag without a deprecation entry |
| A4 | The machine-readable documents are a test | built for the current version: `ops/ci/check-goatbuilder-compat.sh` keeps the `-o json` and `-o yaml` documents of the last release, at least one for every verb or a reason in `not_kept:`, and fails unless each is still produced as a superset with the same version and exit code. A changed kept exit code needs its reason in `exit_changed:`. The set from 1.0 is recorded when 1.0 is published | A kept set of `-o json` outputs from 1.0 that every later build must still produce a superset of |
| A5 | Error codes are never reused | built: `ops/ci/check-error-codes-kept.py` (workflow `error-catalog.yml`) reads the catalog at the last goatbuilder and goat-cli releases (the newest `goatbuilder/v<x.y.z>` and `goat-cli-v<x.y.z>` tags) with `git show` and fails when one of its codes is absent from the tree's catalog, or has a new title without a `retired` mark (defined in `docs/design/error-interpretation.md`, "The catalog"); its self-test holds both failures and both passes. It passes against `goatbuilder/v0.5.3` and `goat-cli-v0.5.19`. The row closes when 1.0 is tagged and the gate runs against it | A check that fails when a code present in the last release is absent, or its title has changed, without a "retired" mark |

### B. Federation is complete

Federation is one command with one decision
([ADR 1225](../adr/1225-federation-belongs-to-goatbuilder-one-command-one-decision.md)).
The first release in which `goatbuilder federate <partner>` runs both sides
with nothing written by hand is 0.5.0. The requirements below are what
remains after it.

| # | Requirement | Status | Proven by |
|---|---|---|---|
| B1 | `goatbuilder federate <partner>` federates two sovereign environments, with the operator writing only the peering entry and the offer and answering two prompts | built (0.5.0); measured against `main` (e7af99da) in the federation test environment's full run `e7af99dab800-20261113T013140-138132`: `nothing-hand-written`, strict (both sides declared from nothing, the command alone writing every value, strict acceptance on both sides), and `goatbuilder-only-both-sides` both passed. The earlier pass, run `up-6884e8354ba6-20261111T090853`, was of a pre-release integration commit and is not counted | The two-sovereign end-to-end job (`ops/federation-e2e/run.sh`) supplies no value the command should derive, and fails if one is needed |
| B2 | The command completes the partner's half of acceptance. Today each side proves its own half, and the two halves are joined by a bilateral session the command does not run | designed | A scenario in which each side's acceptance report includes the other side's signed result, with no step outside the command |
| B3 | Keys can be rotated. Every phase can be run again after a rotation and the peering keeps carrying data | designed (`docs/design/federation-key-rotation.md`); built: phase 3 measuring our package again after a recorded rotation, and rotating the leaves as a library (`crates/goatbuilder/src/federate/rotation.rs`, unit tests; not yet on the command line) | A scenario that rotates each key in turn, re-runs the command, and checks data arrives throughout |
| B4 | Data keeps arriving across a restart of either side, and a republisher passes only what its release set names | measured: persistent reception and the republisher boundary both passed in the federation test environment on `main` (d7e1f2a1) in two consecutive runs, `d7e1f2a1c5c3-20261112T201502-723879` and `d7e1f2a1c5c3-20261112T202644-1776904`; the scenario is on `main` (#489). Each side's core is restarted with its edge left running; a restart of an edge is not measured | Both checks pass there on two consecutive runs |
| B5 | An offer narrower than the deployment's release sets is carried out for one partner | built for a peering whose `federation.toml` the command generates; refused (GB-129) for a peering run from hand-written files. Measured: the scenario passed on `main` in runs `26f9fd2a7355-20261112T175442-893243` and `e70da8a7fff8-20261112T181143-2472121` (strict) | A scenario in which one partner receives a subset and a read of anything outside it is denied |
| B6 | One environment holds several peerings, each with its own directory package | built, on by default in `goatbuilder federate` (`FED_DIRECTORY_PER_COUNTERPARTY` set by the operator still decides; `directory.py` alone and `peering-standup.sh` stay off); tested through the binary for two peerings in one environment (`crates/goatbuilder/tests/federate_per_partner_package.rs`). Measured: the two-sovereign job passed with it in run `saltlick-050-per-partner-run4-64d29c4c-20261113T002213`, 210 passed and 0 failed, on the harness at 64d29c4c against saltlick v0.5.0 (`sha256:80b3c9fa850d498cf9b22e44750721996c4e6760b3aec5e47ea823f8ebbf7c31`) with `FED_DIRECTORY_PER_COUNTERPARTY` unset | The switch is on by default and the two-sovereign job passes with it |
| B7 | Backing a peering out removes everything the command created and nothing else | built (0.5.0, `goatbuilder federate backout`); built: the back-out follows the way admission placed the core's admission on both of the test environment's core shapes, a goat-zenoh role core and a core reached through `FED_CORE_*`, and takes back our core router's leaf on the driver by the earliest admission that touched it, and on a role core the host's copy, through the role's play when the leaf is put back and through the role's absent tasks when it is removed (`crates/goatbuilder/src/federate/backout.rs`, `ops/ansible/roles/goat-zenoh/tasks/federation-absent.yml`, unit tests). The back-out scenario is built (#452, `ops/federation-e2e/units/backout.sh`) and not measured. The partner's stored data, as decided (ADR 1234, a store per partner dropped whole when the peering ends): built on a role core, where the back-out takes the partner out of the role's `zenoh_sovereign_peerings` and the role's play restarts the router without `sovereign_from_<partner>`, then a play of its own deletes that store (`ops/ansible/zenoh-drop-partner-store.yml`); the role renders no shared `from/**` store (`docs/design/federation-backout-partner-data.md`, `roles/goat-zenoh/tasks/drop-partner-store.yml`, unit tests; not measured) | The back-out scenario, followed by a comparison of both hosts with their state before the peering |
| B8 | ADR 1225 is accepted, and the design document's federation sections carry no "WIP" status line | Proposed | The record's status and the document |
| B9 | Federation has run between two environments that are not the test laboratory | not measured. The procedure is written (`docs/operations/federation-between-two-standing-goatnets.md`) | One peering between two standing goatnets, set up with the released binary, with both sides' acceptance reports kept as a measurement |
| B10 | A peering can be driven from two machines when no single machine reaches both the edge host and the core host: an edge-side driver that holds no CA key and cannot reach the core, and a controller that can. The plan approval, the run journal and resume hold across the two | designed, and the row has not passed: the edge driver's run, `import` and the contract's end state are not built. The phase-1 hand-off is measured: in the federation test environment's two-driver scenario on 90ecd3d3 (run `90ecd3d39829-20261113T083718-2510969`, harness 159b1674), both controllers stopped at phase 1 with an object whose commands parse, the follower's controller run passed with its clean-checkout, edge-commit and carry checks, and the edge step was refused with GB-267 as built; 15 checks passed and the 7 that need the edge run failed. The design is `docs/design/federation-two-drivers.md`; the hand-off stop at phases 1 and 2 is built: a peering declared `drivers: two` stops the controller there with the contract's hand-off object (GB-265), and a checkout with changes is refused (GB-266); a two-driver run that would reach a later phase that needs the edge (3, 11 to 14), every `federate deploy` among them, and `--side edge` are refused as not built (GB-267) before the run acts, so a deploy never admits on our core and stops half done (`crates/goatbuilder/tests/federate_two_drivers.rs`, `a_two_driver_deploy_runs_nothing`); every command the phase 1 and 2 objects name parses with the binary, and the edge run and `import` refuse with GB-267 (`every_command_a_hand_off_names_parses_and_refuses_as_not_built`); phase 3's two halves are built at the `init` level; the halves inside the one command, the edge driver's run, `import`, the hand-offs at phases 3 and 11 to 14 and the rest are not built. The single command has been measured only from one machine | A scenario with two drivers per sovereign, in which the edge-side driver has no CA key and no route to the core |

### C. It is proven on real machines, by people who did not build it

| # | Requirement | Status | Proven by |
|---|---|---|---|
| C1 | A founder site goes from declared to verified in 60 minutes or less, a new node in 15, a burn-down in 15 ([goatbuilder-speed.md](goatbuilder-speed.md)) | built; not measured on hardware. What a timed run records is designed (`docs/design/goatbuilder-hardware-measurement.md`) | A timed run on hardware, kept as a measurement, with the per-phase times the binary prints |
| C2 | Tutorials 2 and 3 work on real machines ([goatnet-tutorials.md](goatnet-tutorials.md)) | built: both tutorials run on the fixture; not measured on hardware. What a run on hardware must show is designed (`docs/design/goatbuilder-hardware-measurement.md`) | Both tutorials followed on hardware from the published text, with every command's output matching the text |
| C3 | `status` reads real hosts | built as a library: `inspect::status` reads real hosts as `diff` reads and `verify` grades them, tested on a fixture of real-host answers (`status_agreement_tests` in `crates/goatbuilder/src/inspect.rs`, and `ops/goatbuilder/c3-agreement.py`); not built on the command line: `goatbuilder status` without `--mock` refuses with GB-026 until its `main.rs` arm calls it; not measured against a standing goatnet | `status` against a standing goatnet agrees with `verify` and `diff` for every machine |
| C4 | A person outside the team stands up a goatnet and federates it using only the published documents | not measured | A cold-read drill: the person has the public repositories and no help. Every place they stopped becomes a fix to the tool or the text, and the drill is repeated until a run has no stops |
| C5 | Every refusal the drills met is in the error catalog with its fix | built for known errors, and measured on the command-line corpus: `crates/goatbuilder/tests/refusals_coded.rs` runs every refusal case in `crates/goatbuilder/tests/cmd/` and requires the one code its catalog entry names. 30 of its 49 runnable refusal cases print that code (22 before this measure began); the other 19 are listed as pending, 18 in main.rs and 1 in federate/, whose owners' changes print them. A pending federate case that needs a declared deployment (`federate_undeclared_is_refused`) is skipped where none is declared. In the source, every refusal an operator can cause is raised with its code except main.rs's and federate/'s; what remains uncoded is four internal invariants. The drills themselves have not run | The drill logs contain no error without a `GB-nnn` code |

### D. Scope is decided

Each designed item is either in 1.0 or named as later. The answers below
are proposed; an item moves only by a change to this table.

| Item | Status | Proposed | Reason |
|---|---|---|---|
| `status` on real hosts | designed | In (C3) | A 1.0 tool reports real state |
| A pull converger | designed | **Open decision** | Fleet doctrine is that convergence is pulled ([fleet-management.md](fleet-management.md), P8). If 1.0 ships with the push converger only, the promise in "What 1.0 promises" must hold when the pull converger arrives: the configuration field `converger:` and the verbs stay as they are, and pull is added as a value. The decision is whether that is enough, or whether 1.0 waits for a pull converger so that its first promise is made on the intended model |
| Zero-SaaS standup, measured | not built | In, as part of C1 | "No outside account" is a stated property; it is measured once or not claimed |
| Forge adapter | designed | Later | The release path works on one forge; the adapter adds a value and breaks nothing |
| OpenBao injector | designed | Later | A store is a declared, pluggable dependency |
| Semantic error matcher | designed | Later | It adds a matcher after the existing ones |
| A `boundary` machine class | not built | Later | The boundary node is the router `goatbuilder federate` deploys; no tutorial needs the class |
| A `phone` machine class and `--via usb` | not built | In (H3 to H5) | A phone is built by the command that builds a computer, or it is not a goatbuilder node |
| What a phone node replicates | designed ([phone-node.md](phone-node.md)) | **Open decision** | A full joiner is a replication member; a phone that is off for days falls behind and must be seeded again. The alternative is a phone profile that is a mesh peer with its own router and is not a replication member. The class in H5 is declared either way; the decision is which one `--class phone` means by default |
| A launcher on the phone that starts the VM at boot | not built | Later | It is an Android app with its own release. Until it exists, H8's "without a person" is met by a computer on the USB cable and a phone with no screen lock |

### E. Upgrading is tested

| # | Requirement | Status | Proven by |
|---|---|---|---|
| E1 | A deployment made with the last 0.x release continues on 1.0 | built: `crates/goatbuilder/tests/upgrade.rs` passes against `main` for 0.4.3 and 0.5.1. The row closes when it runs against the 1.0 build | A fixture deployed with the last 0.x binary, then `validate`, `status`, `deploy` and `rollback` run with the 1.0 binary with no change to its files |
| E2 | A federation started on 0.x continues on 1.0 | the first half is built: a cargo test on the federate tests' stand-in world (`crates/goatbuilder/tests/upgrade_federation.rs` finishes with this build a peering 0.5.1 took to phase 10). The second half is measured against `main` (83604dba) in run `83604dba74dd-20261113T011437-3712674`: both sides stood up with the published 0.5.1 binary (sha256 checked) on checkouts at `goatbuilder/v0.5.1`, passing strict acceptance (25 passed, none failed, skipped or blocked); each checkout then moved to the build's tree with its own declarations kept and its goatbuilder replaced by the build's (sha256 checked); phase 14 alone, run by the build against 0.5.1's working directory, passed strict acceptance on both sides (25 passed, none failed, skipped or blocked), with the journal's records for phases 1 to 13 unchanged; a state.json of a schema the build does not read was refused (GB-182), the control. The row closes against the 1.0 build | A peering taken to a middle phase with the last 0.x binary and finished with 1.0; and a finished 0.x peering that passes 1.0's acceptance |
| E3 | `goatbuilder update` moves a 0.x install to 1.0 and back | built: `crates/goatbuilder/tests/upgrade_update.rs` passes against `main` on linux-amd64. A copy of the published 0.5.1 moves to the build of `main` with a plain `update`. A plain `update` back declines, "not older than the served one; nothing to do", exits 0 and changes nothing. `update --force` returns it to 0.5.1, byte for byte. A served binary that its checksums do not vouch for is refused (GB-074), with `--force` too, and the install is kept. Back on 0.5.1, a run journal holding a phase outcome that 0.5.1 does not know is refused (GB-182, "does not parse") and left as it was. The build of `main` verifies each release's signature in process before it installs it, refusing one that does not verify (GB-261). On the way back that is the published 0.5.1's own `SHA256SUMS.cosign-bundle`, against `goatbuilder-release.yml@refs/tags/goatbuilder/v0.5.1`. The way there is run by 0.5.1, which checks a signature only with cosign on PATH. Not measured: linux-arm64 and both macOS platforms. The row closes against the 1.0 build on each supported platform | The update and the return, on each supported platform |
| E4 | A surface that serves goatbuilder releases checks each release's signature before it serves it. The serving deploy runs the same checks as `goatbuilder update`'s built-in verifier, every one of them, on the keyless signature over `SHA256SUMS` (`SHA256SUMS.cosign-bundle`), all of these, each of which must pass: (1) the certificate chains to a Fulcio CA of the trusted root, carries the codeSigning extended key usage, is not a CA, has no critical extension the verifier does not handle, and is valid at the signature's `integratedTime`; (2) an SCT embedded in the certificate verifies under a trusted certificate-transparency log that was valid at the SCT's own time; (3) Rekor's SignedEntryTimestamp verifies under a trusted transparency-log key; (4) the hashedrekord entry names this `SHA256SUMS`'s sha256, this signature and this certificate; (5) the signature over `SHA256SUMS` verifies with the certificate's public key; (6) the certificate names the release's own identity: the issuer is GitHub Actions (`https://token.actions.githubusercontent.com`); the source repository is the release's repository; the source ref is exactly `refs/tags/goatbuilder/v<version>` for the version served; and the Build Config URI is `<repository>/.github/workflows/goatbuilder-release.yml@refs/tags/goatbuilder/v<version>`. A match on the repository alone is not enough: every workflow in the repository that calls the shared release workflow gets the same certificate identity. A serving implementation that skips any of these does not meet the row. The checks are the release design's ([goatbuilder-distribution.md](goatbuilder-distribution.md), "The release"). A release whose bundle does not verify is refused, and nothing of it is served | not built: the goat-get deploy requires the bundle to be present, as set-equality over the release's assets (`ops/ansible/roles/goat-get/tasks/deploy.yml`, line 842), and verifies nothing. The role's own comment leaves the verification to the person who downloads (line 780) | A test of the goat-get deploy, run against a stand-in release: a release whose bundle verifies is served; a release whose `SHA256SUMS` was changed after signing, one signed by another workflow identity, one whose transparency-log entry names another sha256, and one with no bundle are each refused, and the served directory is unchanged after each refusal |

### F. The tool shows its work

Nothing `goatbuilder` does or decides is hidden from the person running it: what it will do, what it is doing, what it did, and why can each be read from the tool, without opening its source. This becomes contract clause C11.

| # | Requirement | Status | Proven by |
|---|---|---|---|
| F1 | Every process the tool starts is shown when it starts (phase, purpose, command, host, log) and is in a run record afterwards, for every verb | built; tested through the conformance stand-ins; not measured on hosts. Every process `standup`, `join-node`, `deploy` and `federate` start goes through the runner (`crates/goatbuilder/src/runner.rs`; no `Command::new` outside it), is framed with its phase, the command and its log, and is in the run record with a log that exists; C11 check (a) passes for all four. federate's frame also gives each phase's purpose and an ssh probe's host (a play names no host: it starts here, and its hosts are the inventory's); standup's, join-node's and deploy's give the phase's label as its name, and no purpose until the phase catalog does. `-v` logs every command | The legibility conformance test, check (a): on the fixture, every process started is in the run record with its command and its log |
| F2 | Every value a run uses is listed with its source: a file and line, the declaration and rule it was derived from, an environment override, a default, or an allocation. A secret shows its source and never its value | built as a library, not yet wired: `crates/goatbuilder/src/inputs/` lists every value of federate, standup, join-node and deploy with its source, and a secret by its source alone; its tests `every_value_in_a_<verb>_command_has_a_sourced_input` pass for all four verbs. No run record or command prints the list yet, so check (b) still fails for each verb (`legibility-known-failing.txt`) | Check (b): every value in a command line has an entry in `inputs` with a source |
| F3 | A dry run prints the commands the real run executes, on a terminal as well as in JSON, and marks what cannot be known before the run | built for `standup`, `join-node`, `deploy` and `federate`: C11 check (c) passes for all four. join-node's and federate's terminal plans list each phase's command under the table, verbatim and unwrapped; standup prints every planned command verbatim; deploy lists its play line, built from the argv that runs. Each probe of standup and deploy is printed as the line its builder makes, under a heading that says whether this dry run ran it (its answer shaped the plan) or only a real run starts it. federate's plan marks what cannot be known before the run: phase 11 "depends on what phase 6 agrees", since the agreement chooses the connection's play. Not built for `release`, whose terminal lines are the plan's description of a command (step 3) | Check (c): the dry run's commands equal the real run's, in order |
| F4 | Every phase of every verb is described in one catalog (what it does, why, what it changes, how to tell it worked, the command by hand), `explain` answers for a phase as it does for an error, and the documents' phase tables are rendered from that catalog | built for federate: its 14 phases in `ops/phases/catalog.yaml`, held to the plan's phases and commands by unit tests, check (d) passing for federate, the runbook's three phase tables rendered from it and checked in CI; `goatbuilder explain <verb> [<phase>]` built for federate (`crates/goatbuilder/src/explain_phase.rs`, called from `explain`: one phase by number or id, as text or the catalog's fields under `-o json|yaml`; the verb's phases when none is given; GB-262 for a phase it does not have; pinned by unit tests and five golden command cases); designed: the catalogs of standup, join-node and deploy | Check (d): every phase that can run has a catalog entry, and every document path the tool prints exists |
| F5 | The output of a failed phase is matched against the error catalog | built for federate: a failed phase's logged output (its last 64 KiB) is matched against the error catalog, and its failure carries an `interpreted` row with the code, likely cause, fix, log path and the phase's command by hand from the phase catalog, or the phase's own guidance when no known error is found (`explain_phase::failed_phase`, called from federate's failure branch); proven by `crates/goatbuilder/tests/federate_failed_phase.rs`, phase 12 failing on an ssh key refusal; designed: standup, join-node and deploy | A phase made to fail with a known error carries that error's code and fix |
| F6 | Clause C11 is in the contract with its conformance test | the conformance test is built (`crates/goatbuilder/tests/legibility.rs`), with the checks that fail today listed in `crates/goatbuilder/tests/legibility-known-failing.txt`; the clause is not built | The clause in `goatbuilder.md` and the contract chapter, naming the test |

### G. A first goatnet can grow

A goatlite stays quick to start: one file of four facts, no outside account, and no step added to prepare for later. Growing it into a full environment is a separate, later act that carries its values forward. Growth proven on machines, including the identity root and the database, comes after 1.0.

| # | Requirement | Status | Proven by |
|---|---|---|---|
| G1 | A lite configuration is valid input to the full preset: the tool rewrites it into a full configuration with the same values and shows the difference | designed (`docs/design/goatlite-growth.md`); the rewrite is in review | On the lite fixture, the rewritten configuration passes validation and a standup plan from it changes nothing that already exists |
| G2 | The first run of a goatlite does not grow: the count of facts the learner supplies, of prompts and of phases is held by a test | built: `crates/goatbuilder/tests/goatlite_budget.rs` | The budget test on the lite fixture, and tutorial 00 gains no step before the goatnet works |

### H. A node on a phone

A site node can be built on an Android phone with the command that builds one on a computer ([phone-node.md](phone-node.md)). One phone has joined a standing goatnet as a `core` joiner with `join-node` 0.5.1; that run left the tool in the places H1 to H6 close. H1, H2 and H6 are not about phones: they are faults of `join-node` on any new node, found by that run.

| # | Requirement | Status | Proven by |
|---|---|---|---|
| H1 | `join-node` runs phases 1 to 15 in order on a node that is not yet enrolled, at a goatnet that already has a joiner: the gates of phases 3 and 7 accept the mesh-address placeholder that phase 12 fills, pass 1 of phase 13 does not check a service that the next step starts, and a site that declares signal federation off passes the federation gate | built (`crates/goatbuilder/tests/join_node_third_site.rs`, on the fixture `ops/goatbuilder/fixtures/join-node-third-site`); not measured on hardware; each of the three refused in the measured run | A `join-node` run on the fixture from phase 1 to 15 with no `--from`, for a third site that declares federation off |
| H2 | `join-node` has no manual phase after "reach the node": it mints the setup key into the declared secret store, enrols the node from a key file, and resolves every secret reference it passes to a node | not built: phases 8 and 10 are manual, and phase 9 passed a reference unresolved | The same fixture run, with no operator step between phase 2 and phase 15 and no secret in any argument list |
| H3 | `--via usb`: the tool reaches a node through the Android debug bridge, with no SSH configuration written by hand | not built; measured by hand (Ansible ran the host baseline through an `adb` proxy command) | A fixture run in which a stand-in `adb` carries the connection and no file outside the checkout is read |
| H4 | For a phone, the first phase prepares the phone's Linux VM (kernel, container engine, deploy user) and refuses to continue unless the integrity checks pass | a script outside this repository does it, measured on one phone; not built in the tool | The phase on the fixture, and its refusal when a check is made to fail |
| H5 | A machine class `phone` carries the declared differences once: no LAN address, no public ingress, no disk encryption inside the VM, memory limits for 4 GB, no observability host | not built: the first site states them in its own group_vars | A site declared with `--class phone` passes the config preflight with none of those values written by hand |
| H6 | The plays a joiner runs work on Debian as on Ubuntu, and Postgres memory settings and every container memory limit are variables whose defaults are today's values | Docker's apt repository follows the distribution (built). Every role that writes a crontab entry installs `cron` first (built). Postgres `shared_buffers` and `max_connections` and every container memory limit and reservation are variables whose defaults are today's values, and `ops/ci/check-compose-memory-is-variable.sh` refuses a literal (built). The hermetic container-host job on a Debian image (not built) | The hermetic container-host job on a Debian image; a check that fails on a memory limit with no variable |
| H7 | One phone is built from a reset VM to a verified node by one command, with the released binary, in one uninterrupted run, and the record is kept | not measured: the first run was resumed several times | A timed run on a phone, kept as a measurement, as C1 is for a computer |
| H8 | A phone node stays a node: it runs for seven days with the checks of H4 still passing, and comes back after a phone reboot without a person | not measured: about two hours on power; after a reboot the VM stays down until a person unlocks the phone | The seven-day record, and a reboot during it |

### I. A joined site runs cut off

A site added with `join-node` is a whole site: it runs every service it declares, and when it is cut off from the other sites it keeps admitting people and enrolling machines, then agrees with them again when the link returns, with no repair by hand. The design is [island-mode-multi-writer.md](island-mode-multi-writer.md) ([ADR 1165](../adr/1165-island-mode-multi-writer-every-site-admits.md)). I3 is `goatbuilder` work; I1, I2, I4 and I5 are work in the portal, the id console and the onboarding engine, and 1.0 waits for them.

| # | Requirement | Status | Proven by |
|---|---|---|---|
| I1 | The portal and id console stores are at the multi-writer schema of the design's section 3 | not built: both stores run on Postgres with their earlier tables, in no replication set | The design's gate G2 |
| I2 | The partition harness passes for all five write classes, with the portal and the id console among its stores | built for NetBird, Kratos and Hydra; the portal and console scenarios are not built | Every scenario of the harness passes, with none listed as expected to fail |
| I3 | `join-node` alone builds every service a joining site declares: the data router, its certificates and health publishers, the front door, and, once I4 allows them, the portal, the id console and the onboarding engine. It states which founder-only services it leaves out and why | not built: the router, certificates and publishers are hand steps after `join-node`; the front door is set up only inside signal federation; no verb deploys the id console | A two-site join on the fixture with no step outside the command, and the declared service set equal to the deployed one |
| I4 | Each write class is turned on at every site, and the rule that keeps admin surfaces at the founding site is lifted for it | not built | The design's gate G3, per class |
| I5 | The partition drill passes on a standing two-site goatnet, and then on a second one: the sites are cut apart, a person is admitted and a machine enrolled on each side, and the sites agree after the link returns | not measured | The design's gate G4, with each drill kept as a measurement |

## Order of work

The requirements depend on one another in this order. No dates are set here.

1. **0.5.0.** B1 and B7.
2. **Federation completed.** B2 to B6 and B10, each its own release in the 0.x line, each behind a switch until its scenario passes.
3. **Federation on standing goatnets.** B9, then B8.
4. **Real machines.** C3, then C1 and C2 on hardware. H7 and H8 on a phone.
5. **The tool shows its work.** F1 to F6, in 0.x releases, beside the steps above.
   **A node on a phone.** H1, H2 and H6 first, because they are faults on every new node; then H3 to H5.
   **A joined site runs cut off.** I3 beside H1 and H2, in `join-node`. I1 and I2 before I4; I4 before I5.
6. **The promise.** A1 to A5 and the decision in D on the pull converger. From this point the surfaces are frozen.
7. **Upgrade tests.** E1 to E3, against the release that froze the surfaces.
8. **The cold-read drill.** C4 and C5, on a release candidate `goatbuilder/v1.0.0-rc.N`.
9. **1.0.** Every row of the gate reads "passed", with a link to its test run or measurement.

## Release plan

Three releases after 0.5.2 reach 1.0, with the 0.7.x releases that carry group I between the second and the third, or four if the release candidate needs a second round. No dates are set; a release is cut when its rows pass.

| Release | What it is | Rows it closes |
|---|---|---|
| 0.5.2 | Document corrections; CI no longer depends on a distribution archive. Released and deployed with no exception at any stage | none; it proves the pipeline |
| 0.6.0 | The tool shows its work, and the federation rows that need no outside party | F1 to F6; B5, B6, B7, B10; C3; G2 |
| 0.7.0 | Federation complete and proven outside the laboratory; the promise written and under test; upgrade tested | B2, B3, B4, B8, B9; A1 to A5; E1 to E3; G1; H1 to H6; I3 |
| 0.7.x | A joined site runs cut off, one write class at a time, each behind its own switch | I1, I2, I4 |
| 1.0.0-rc.N, then 1.0.0 | Surfaces frozen; proof on hardware and by people who did not build it | C1, C2, C4, C5; D agreed; H7, H8; I5 |

Rows that need a person or a machine outside the build: B9 (a real partner), C1 and C2 (hardware), H7 and H8 (a phone), I5 (two standing two-site goatnets), C4 (a person outside the team), A1 and D (decisions). Each is prepared so that the act itself is short.

## How the gate is kept

- Each row's status is changed in the pull request that makes its test pass, and the row then links to the test.
- A requirement is added or dropped only by a pull request that changes this file and says why.
- The release notes of each 0.x release name the rows it closed.
