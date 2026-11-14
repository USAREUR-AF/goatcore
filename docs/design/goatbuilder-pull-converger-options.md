# goatbuilder 1.0 and the pull converger: the options

| Field | Value |
|---|---|
| Status | **Written.** Options for a decision the operator has not yet made. It decides one open row of the 1.0 gate, group D, "A pull converger" |
| Design | [goatbuilder.md](goatbuilder.md) (the converger interface) · [fleet-management.md](fleet-management.md) (P8 and §5, convergence is pulled) · [ADR 1198](../adr/1198-goatbuilder-config-driven-deploy-release-cli.md) · [ADR 1109](../adr/1109-fleet-management-dimension-and-pull-convergence.md) |
| Audience | the operator, who decides; anyone who later needs to know why 1.0 was cut the way it was |

## The decision

goatbuilder 1.0 is the first release that promises its configuration file, its commands, its
`-o json` documents and its error codes stay compatible across every 1.x release. The project's
fleet doctrine says convergence is pulled: each machine fetches what it should run and applies it
itself. goatbuilder today reaches machines only by pushing to them from the operator's machine.
The decision is whether 1.0 makes its promise with push alone, with pull to be added later inside
that promise, or waits until a pull converger exists.

Either way, the promise covers what other people build on: `goatbuilder.yaml`, the verbs and
flags, the exit codes, the JSON documents and the `GB-nnn` codes. The two options differ in what
must be true of those surfaces at 1.0 for pull to arrive later without breaking them, and in how
much has to be built before the tag.

## What exists today

**The configuration already names both convergers.** The schema's `converger` field accepts
`push-ansible` and `pull-agent` (`ops/goatbuilder/schema/goatbuilder-v1.schema.json`, definition
`converger`). It is required on every environment and may be overridden per site and per machine
class (`crates/goatbuilder/src/config.rs`, `Environment::converger`, `Site::converger`,
`Class::converger`), because the doctrine moves one class at a time. A file declaring
`converger: pull-agent` validates today.

**The verbs are designed to be converger-agnostic, and are so only on the fixture.** The design
fixes one interface of three calls, `publish`, `observe` and `read_record`
([goatbuilder.md](goatbuilder.md), "The converger interface"), and the code has it as a trait
(`crates/goatbuilder/src/converger.rs`, `trait Converger`). Its one implementation is the
scripted converger on a fixture directory (`impl Converger for Scripted`), which the conformance
suite, the golden tests and `tutor` run on. On the fixture, a `pull-agent` environment converges
with no transport opened (`crates/goatbuilder/src/verbs.rs`, test `c6_least_privilege`).

**On real hosts, every verb pushes, whatever `converger:` says.** `goatbuilder deploy` on real
hosts calls the push converger directly (`crates/goatbuilder/src/main.rs`, the `Cmd::Deploy` arm,
`push::deploy`), and so does `release` for its deploy rungs (`crates/goatbuilder/src/release.rs`,
`push::deploy`). The push converger reads the pins and the inventory, not `goatbuilder.yaml`'s
`converger:` (`crates/goatbuilder/src/push.rs`); it is not behind the trait. `doctor` does read the
field: for a `pull-agent` environment it skips the reachability check and reports "no transport:
the agent reaches out" (`crates/goatbuilder/src/doctor.rs`). So today a `pull-agent` environment
passes `doctor` without a reach check and is then deployed by push.

**There is no pull converger.** Nothing in this repository is an on-host agent that fetches a
declaration and applies it, and nothing publishes the signed per-environment index that agent
would read ([goatbuilder.md](goatbuilder.md), "The model": "for a pull converger it is a small
**signed index** per environment"). What does exist is the typed, signed intent document the index
would be made from: `goatbuilder pins` renders `ops/consistency/intent/<goatnet>/intent.json`
(IntentV1) and verifies its detached signature (`crates/goatbuilder/src/pins.rs`).

**`status` on real hosts is not built either** (`GB-026`, "the push converger's status path is not
built yet"). It is in 1.0's scope as gate row C3, and its read path is the same `read_record` a
pull converger needs.

**The decisions behind both are proposed, not accepted.** [ADR 1109](../adr/1109-fleet-management-dimension-and-pull-convergence.md)
(convergence is pulled) and [ADR 1198](../adr/1198-goatbuilder-config-driven-deploy-release-cli.md)
(goatbuilder, with "push/pull-agnostic by construction, adopted per class") are both **Proposed**
in `docs/adr/INDEX.md`.

## What the push converger assumes that pull changes

The verbs only declare and verify. What pull changes is underneath them, in five places a reader
can check in the code.

| The push converger today | Under pull | Where it is in the code |
|---|---|---|
| **A deploy ends when the play ends.** `observe` runs the play and reads every host's ledger; the run is done when the newest row names the declared digest and says the deploy finished | The agent applies on its own jittered timer. `observe` watches for that, with a deadline derived from the poll interval and jitter, and a machine that has not yet pulled is *pending* | `push.rs` module comment, step 5; `goatbuilder.md`, "The converger interface" |
| **The operator's machine reaches the hosts**: the mesh, else the environment's door | Nothing reaches the hosts; the agent reaches out | `push.rs` `transport()`; `doctor.rs` already skips the reach check for `pull-agent` |
| **Secrets are resolved on the operator's machine at deploy time** and templated into the play by `ansible-preflight.sh` | The play runs on the host, so whatever it templates must be resolvable there | `push.rs` `plan()`, `secrets_needed`; the module comment's last paragraph |
| **Publishing is the merge.** `publish` does nothing beyond the merged pins | `publish` signs and pushes an index per environment, one version per class and a serial that only increases, into the registry chain | `goatbuilder.md`, "The model" and "The converger interface" |
| **A run carries per-run inputs to the hosts**, such as an accepted risk passed to the play as an extra variable and written into the ledger row | Only what is in the published index reaches the host | `push.rs` `deploy()`, the accepted-risk extra variable (ADR 1211) |

Two things do not change. The exit codes already have a meaning for a machine that has not
converged yet: exit 3, "not converged: a machine pending, unknown or drifted"
([the contract, "Exit codes"](../reference/goatbuilder/06-contract.md)). And the doctrine keeps
push for operations that need ordering across hosts ([fleet-management.md](fleet-management.md)
§5, "What push is retained for"). The one-time verbs, `standup`, `join-node` and `federate`, run
plays directly rather than through the converger interface, and nothing here proposes to change
them.

## Option A: 1.0 ships push only, and pull arrives later as a value of `converger:`

**What it is.** 1.0 is tagged with the push converger as the only one that runs on real hosts.
A later 1.x release adds the pull converger behind the same three calls, so a deployment moves a
class to pull by changing `converger:` on that class, with no change to the verbs or their flags.

**What the promise must hold at 1.0, so that pull can arrive without breaking it.**

- `converger: pull-agent` must mean the same thing on 1.0 as on the later release that builds
  it. Today a real deploy pushes to a `pull-agent` environment, and a release that later stopped
  doing so would change what an accepted file does. At 1.0 such an environment must therefore be
  refused on real hosts with a catalog error that names the value as not yet built. A refusal can
  later become "it works" without breaking anything a 1.0 file did.
- `deploy`, `release`, `rollback` and `status` on real hosts must reach machines only through the
  converger interface, so that the later converger is an implementation and not a change to the
  verbs. Today the push converger is called directly and is not behind the trait.
- The JSON documents of these verbs must already carry the states pull produces, *pending* and
  *unknown* per machine, as fields that 1.0 defines. Within one document version, a field is
  added and never retyped, so a field pull would need cannot first appear after 1.0 with a
  different shape.
- Exit code 3 keeps its present meaning, which already covers a machine that has not converged.

**The work before 1.0.** Put the push converger behind the trait for the real-host verbs; refuse
`pull-agent` on real hosts with a new catalog code and a test; make sure the per-machine states
are in the `-o json` documents and in the compatibility test that gate row A4 keeps. Gate row C3
(`status` on real hosts) builds `read_record` for push, which is the same call a pull converger
implements.

**What 1.0 then does not have.** A machine converges only while an operator runs a verb against
it; the properties [fleet-management.md](fleet-management.md) §5 argues for (no delivery
lifecycle needed, load that decays under failure, island mode) are not provided by goatbuilder
1.0.

## Option B: 1.0 waits until a pull converger exists

**What it is.** 1.0 is tagged only when the pull converger runs on real hosts behind the same
interface, so that the first promise is made on the model the doctrine intends.

**What the promise must hold at 1.0.** Everything in option A's list, and the pull converger's
own behavior becomes part of what 1.x keeps stable: what `publish` writes, the deadline `observe`
uses, and what `read_record` reads from a machine.

**The work before 1.0**, in addition to option A's:

- the on-host agent: on a jittered timer, fetch the index from the nearest registry that answers,
  verify its signature, read only its own class's version, and run the existing play against
  itself (fleet-management.md §5: "there is one convergence engine — Ansible — and the pull loop
  invokes it");
- the publication: the signed per-environment index built from IntentV1, and its distribution
  to a replica the site can reach with no wide-area link;
- `observe` and `read_record` for pull, from each machine's status line;
- where secrets come from when the play runs on the host;
- the conformance suite run against the pull converger, which is how the design admits a
  converger ([goatbuilder.md](goatbuilder.md), the conformance paragraph), and a measurement on
  real machines.

This adds gate rows: a pull converger built, conformance passed, measured on hardware. It moves
1.0 later by however long those take; no estimate is made here.

## What is the same under either option

The configuration field and its values are already in the schema and do not change. The verbs
do not change. Gate row C3 (`status` on real hosts) is needed by both. The ADRs behind pull
(1109) and behind goatbuilder's converger interface (1198) stay proposed until one of them is
accepted; neither option needs a new ADR to proceed, and either one is recorded by changing the
D row of the gate.

## The question

Does goatbuilder 1.0 ship with the push converger only, holding the conditions in option A so
that pull arrives later as a value of `converger:` with the verbs unchanged, or does 1.0 wait
until a pull converger is built, conformance-tested and measured on real machines?
