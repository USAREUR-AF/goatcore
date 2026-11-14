# 6. The contract and exit codes

Every declarative verb makes ten guarantees, C1 to C10. When a guarantee cannot
be met, the verb refuses, names the reason and prints the recovery command. It
never proceeds on a guess. The contract is enforced in one module of the binary
that every declarative verb calls. An eleventh clause, C11, says that what the
tool does can always be read from the tool; it is written and tested, and does
not hold yet (below).

| Field | Value |
|---|---|
| Canon | [goatbuilder.md](../../design/goatbuilder.md) §"The deploy contract" |
| Tests | [goatbuilder-conformance.md](../../design/goatbuilder-conformance.md); every clause, C1 to C10, is built as a test on a fixture; C11's test is `crates/goatbuilder/tests/legibility.rs` |

## The ten guarantees

**Declare**

| | Guarantee | What you see |
|---|---|---|
| **C1** | **Immutable ref.** A deploy applies exactly the digest the declaration names. Never a mutable tag, never a value from the operator's working tree | a declaration of `:latest` is refused: "is a mutable tag; a deploy declares a digest (C1)" |
| **C2** | **Signed and ordered.** Each declaration is signed at the source and verified where it is read. A consumer never accepts one older than the last it accepted, so a revert is a new declaration at an old digest | `pins --check` refuses an unsigned or tampered intent; a fixture publication with a bad seal or a serial not above the last is refused (GB-045) |
| **C3** | **Fresh source.** The declaration read is the current one on origin. A stale checkout is fast-forwarded or refused with the command to fix it | before the first host, `ops/scripts/fresh-source.sh` fetches the trunk and refuses a checkout behind it in `ops/ansible` or `ops/pins` (GB-064), or a trunk it cannot fetch (GB-065); `--offline` judges against the cached trunk. On a fixture, a source ahead or diverged is refused (GB-046); `--frozen` refuses any fast-forward |
| **C4** | **Attributed.** Every declaration names the person who made it | `--ack <name>` on `deploy`, `release`, `fastlane`, `standup`; a deploy, rollback or standup without it is refused, on a fixture too (GB-047) |

**Converge**

| | Guarantee | What you see |
|---|---|---|
| **C5** | **Idempotent and resumable.** Re-running the same verb on the same declaration changes nothing. A run that stopped resumes where it stopped | `--from <step>` on `release`, `standup`, `join-node`, `federate`; a second fixture deploy writes no record |
| **C6** | **Least-privilege transport.** Each action opens only the credential and transport it needs | the transport order in chapter 5; doors per goatnet |

**Verify**

| | Guarantee | What you see |
|---|---|---|
| **C7** | **Verified per machine; unknown is not done.** A machine is converged only when its own ledger names the declared digest and marks the run finished. Unreadable is *unknown*; not yet changed is *pending* | `deploy`, `verify` and `status` exit 3 while any machine is pending or unknown |
| **C8** | **Inspectable.** Declared state, actual state and the drift between them can be read at any time by verbs that change nothing | `diff`, `history`, `verify`, `status` |

**Cross-cutting**

| | Guarantee | What you see |
|---|---|---|
| **C9** | **Reversible.** A rollback is a new declaration of the prior digest and travels the same converge-and-verify path | `rollback` writes a row with `action: rollback` and the prior ref |
| **C10** | **Machine contract.** Every verb is non-interactive when asked, has a typed exit code, and renders a table, JSON or YAML from one flag | `-o json`; `--yes`; GB-008 without a terminal |

**Legibility**

| | Guarantee | What you see |
|---|---|---|
| **C11** | **Legible.** For every action the tool takes, its command, its inputs with their sources, and its reason can be read from the tool before, during and after the run | the dry run prints each command the real run will start; each run of a verb that drives hosts writes `<state>/runs/<id>/run.json`; `goatbuilder runs` and `goatbuilder show <run>` read it |

C11's test, `crates/goatbuilder/tests/legibility.rs`, runs standup, join-node,
deploy and federate on stand-ins that reach no host and checks four things for
each:

- (a) every process the tool started is in the run record, with its command and
  a log file;
- (b) every value in a command line is in the record's inputs, with where it
  came from;
- (c) the commands the dry run prints at a terminal are the ones the real run
  starts, in order;
- (d) every phase has an entry in the phase catalog, and every document path
  the tool prints exists.

C11 holds when all four pass for all four verbs. Today some do not: the ones
that fail are listed in `crates/goatbuilder/tests/legibility-known-failing.txt`,
and that file, not this page, is the current list. The design is
[goatbuilder-legibility.md](../../design/goatbuilder-legibility.md).

## Declarative and read-only verbs

| Read-only: safe anywhere, change nothing | Declarative: enforce C1 to C10 |
|---|---|
| `validate`, `doctor`, `status`, `verify`, `diff`, `history`, `explain`, `plan`, `promote status/review/readiness/check`, `pins --check`, `secrets store/available/missing/hint/doctor` | `deploy`, `release`, `rollback`, `fastlane`, `standup`, `teardown`, `join-node`, `federate`, `apply`, `adopt --yes`, `pins`, `promote open/graduate/graduate-set/hotfix/abandon`, `secrets init` |

A declarative verb asks for confirmation at a terminal. With no terminal it
needs `--yes` and refuses otherwise (GB-008). Every declarative verb that
reaches hosts has `--dry-run`, which decides everything, prints it and touches
nothing.

## Exit codes

| Code | Meaning |
|---|---|
| **0** | ok: every check passed and every machine converged |
| **1** | refused: a precondition failed. The report names it and the fix |
| **2** | usage: the command line itself was wrong, or names something the config does not declare |
| **3** | not converged: `status`, `deploy`, `verify` or `diff` found a machine pending, unknown or drifted. For `rollback` and `teardown`: refused as unsafe |
| **4** | guided: `rollback` of a `config` or `acl` artifact prints the manual steps instead of acting |

A refusal keeps every step that ran as the last rows of the report, so you can
see how far the run got.

## The output shape

A table for a person is one row per check: `ok` or `FAIL`, the check's name and
its detail, then a summary line `<verb>: <outcome> (<n> checks)`. A failing row
may carry a `fix:` line.

*Fixture output, `goatbuilder deploy --mock basic --env dev --ack reader --yes`,
with image names shortened to fit the page:*

```text
FAIL  dev/netvis/declaration  netvis:latest is a mutable tag; a deploy declares a digest (C1)
                              fix: pin the declaration to <image>@sha256:<digest>
ok    dev/portal/publish      serial 1: portal:v1.4.0@sha256:0b2a6c1d3e4f… by reader
ok    dev/dev-core-a/portal   converged on portal:v1.4.0@sha256:0b2a6c1d3e4f…
ok    dev/dev-core-b/portal   converged on portal:v1.4.0@sha256:0b2a6c1d3e4f…
FAIL  dev/dev-edge-a/portal   unknown: dev-edge-a: record unreadable
FAIL  dev/dev-obs-a/portal    pending (not yet converged)
FAIL  dev/dev-obs-b/portal    pending (not yet converged)
deploy: refused (7 checks)
```

With `-o json` the same report is a versioned document a program reads without
parsing prose:

```json
{
  "schema": "goatbuilder.output/v1",
  "verb": "status",
  "ok": false,
  "exit": 3,
  "checks": [
    { "name": "dev/dev-core-a/netvis", "ok": false,
      "detail": "pending: no record yet; declared …/netvis:latest" }
  ]
}
```
