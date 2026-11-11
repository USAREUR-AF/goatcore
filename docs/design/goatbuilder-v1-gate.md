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
minor releases, and is removed only in the next major version.

**What 1.0 does not promise.** The text of tables printed for a person. The
shell scripts beside the binary, which stay documented and unsupported
([ADR 1201](../adr/1201-two-avenues-scripts-documented-cli-supported.md)).
The internals of a converger. The `goat` client, which has its own releases.

## The gate

Status words: **built** (the code is on `main`), **designed** (decided, not
implemented), **not built**, **in review** (a pull request is open),
**not measured** (exists and has not been exercised by the named test).

### A. The promise is written and tested

| # | Requirement | Status | Proven by |
|---|---|---|---|
| A1 | The compatibility and deprecation rules above are a decision record | not built | The record is accepted and this section cites it |
| A2 | Configuration compatibility is a test | The schema and its gate are built. The compatibility test is not built | A kept set of configuration files from each release since 0.1.0, run through `validate` on every pull request. Each file validates, or is refused with the catalog error the release notes named |
| A3 | The command surface is a test | The `--help` output of every command is already checked in `crates/goatbuilder/tests/cmd/`. The rule "no removal" is not built | A check that compares the commands and flags of the build with those of the last release and fails on a removal or a renamed flag without a deprecation entry |
| A4 | The machine-readable documents are a test | built for the current version | A kept set of `-o json` outputs from 1.0 that every later build must still produce a superset of |
| A5 | Error codes are never reused | not built | A check that fails when a code present in the last release is absent, or its title has changed, without a "retired" mark |

### B. Federation is complete

Federation is one command with one decision
([ADR 1225](../adr/1225-federation-belongs-to-goatbuilder-one-command-one-decision.md)).
The first release in which `goatbuilder federate <partner>` runs both sides
with nothing written by hand is 0.5.0. The requirements below are what
remains after it.

| # | Requirement | Status | Proven by |
|---|---|---|---|
| B1 | `goatbuilder federate <partner>` federates two sovereign environments, with the operator writing only the peering entry and the offer and answering two prompts | in review; lands with 0.5.0 | The two-sovereign end-to-end job (`ops/federation-e2e/run.sh`) supplies no value the command should derive, and fails if one is needed |
| B2 | The command completes the partner's half of acceptance. Today each side proves its own half, and the two halves are joined by a bilateral session the command does not run | designed | A scenario in which each side's acceptance report includes the other side's signed result, with no step outside the command |
| B3 | Keys can be rotated. Every phase can be run again after a rotation and the peering keeps carrying data | not built | A scenario that rotates each key in turn, re-runs the command, and checks data arrives throughout |
| B4 | Data keeps arriving across a restart of either side, and a republisher passes only what its release set names | The acceptance checks for persistent reception and the republisher boundary ([federation-acceptance.md](federation-acceptance.md)) fail in the federation test environment on `main` | Both checks pass there on two consecutive runs |
| B5 | An offer narrower than the deployment's release sets is carried out for one partner | built for a peering whose `federation.toml` the command generates: `offer:` narrows the generated file and phase 3 reads it. Refused (GB-129) for a peering run from hand-written files. Not measured end to end | A scenario in which one partner receives a subset and a read of anything outside it is denied |
| B6 | One environment holds several peerings, each with its own directory package | in review, behind a switch that is off by default | The switch is on by default and the two-sovereign job passes with it |
| B7 | Backing a peering out removes everything the command created and nothing else | in review; lands with 0.5.0 | The back-out scenario, followed by a comparison of both hosts with their state before the peering |
| B8 | ADR 1225 is accepted, and the design document's federation sections carry no "WIP" status line | Proposed | The record's status and the document |
| B9 | Federation has run between two environments that are not the test laboratory | not measured | One peering between two standing goatnets, set up with the released binary, with both sides' acceptance reports kept as a measurement |
| B10 | A peering can be driven from two machines when no single machine reaches both the edge host and the core host: an edge-side driver that holds no CA key and cannot reach the core, and a controller that can. The plan approval, the run journal and resume hold across the two | not measured. The phases run by number (`--from`, `--to`) on separate machines today; the single command has been measured only from one machine | A scenario with two drivers per sovereign, in which the edge-side driver has no CA key and no route to the core |

### C. It is proven on real machines, by people who did not build it

| # | Requirement | Status | Proven by |
|---|---|---|---|
| C1 | A founder site goes from declared to verified in 60 minutes or less, a new node in 15, a burn-down in 15 ([goatbuilder-speed.md](goatbuilder-speed.md)) | built; not measured on hardware | A timed run on hardware, kept as a measurement, with the per-phase times the binary prints |
| C2 | Tutorials 2 and 3 work on real machines ([goatnet-tutorials.md](goatnet-tutorials.md)) | rehearsed on the fixture; not run on hardware | Both tutorials followed on hardware from the published text, with every command's output matching the text |
| C3 | `status` reads real hosts | designed | `status` against a standing goatnet agrees with `verify` and `diff` for every machine |
| C4 | A person outside the team stands up a goatnet and federates it using only the published documents | not measured | A cold-read drill: the person has the public repositories and no help. Every place they stopped becomes a fix to the tool or the text, and the drill is repeated until a run has no stops |
| C5 | Every refusal the drills met is in the error catalog with its fix | built for known errors | The drill logs contain no error without a `GB-nnn` code |

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

### E. Upgrading is tested

| # | Requirement | Status | Proven by |
|---|---|---|---|
| E1 | A deployment made with the last 0.x release continues on 1.0 | not built | A fixture deployed with the last 0.x binary, then `validate`, `status`, `deploy` and `rollback` run with the 1.0 binary with no change to its files |
| E2 | A federation started on 0.x continues on 1.0 | not built | A peering taken to a middle phase with the last 0.x binary and finished with 1.0; and a finished 0.x peering that passes 1.0's acceptance |
| E3 | `goatbuilder update` moves a 0.x install to 1.0 and back | `update` is built; the move to 1.0 and back is not measured | The update and the return, on each supported platform |

## Order of work

The requirements depend on one another in this order. No dates are set here.

1. **0.5.0.** B1 and B7.
2. **Federation completed.** B2 to B6 and B10, each its own release in the 0.x line, each behind a switch until its scenario passes.
3. **Federation on standing goatnets.** B9, then B8.
4. **Real machines.** C3, then C1 and C2 on hardware.
5. **The promise.** A1 to A5 and the decision in D on the pull converger. From this point the surfaces are frozen.
6. **Upgrade tests.** E1 to E3, against the release that froze the surfaces.
7. **The cold-read drill.** C4 and C5, on a release candidate `goatbuilder/v1.0.0-rc.N`.
8. **1.0.** Every row of the gate reads "passed", with a link to its test run or measurement.

## How the gate is kept

- Each row's status is changed in the pull request that makes its test pass, and the row then links to the test.
- A requirement is added or dropped only by a pull request that changes this file and says why.
- The release notes of each 0.x release name the rows it closed.
