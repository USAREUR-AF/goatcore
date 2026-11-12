# ADR 1228 — From 1.0, goatbuilder's configuration, commands, documents and error codes change only by additions, and a deprecated one is removed only in a major release

| Field | Value |
|---|---|
| Status | Proposed — the deprecation period is open for the operator to decide (see "The decision left open") |
| Date | 2026-11-12 |
| Repo scope | goat core (`goatbuilder`) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 1198](1198-goatbuilder-config-driven-deploy-release-cli.md) (goatbuilder and its deploy contract), [ADR 1199](1199-one-error-catalog-interpreted-by-both-clis.md) (one error catalog), [ADR 1201](1201-two-avenues-scripts-documented-cli-supported.md) (the scripts are documented, the binary is supported), [ADR 1225](1225-federation-belongs-to-goatbuilder-one-command-one-decision.md) (federation is one goatbuilder command) |

## TL;DR (for a layperson)

People write files for goatbuilder, call its commands from their own scripts,
read what it prints for programs, and look up its error codes in runbooks.
This record says that from version 1.0 none of that breaks within the 1.x
releases: things are added, never taken away or changed in meaning. When
something has to go, it first keeps working while it prints a warning that
names what to use instead, and it is taken away only in version 2.0. A check
on every change compares the tool with its last release, so a break is caught
before it merges, not by a user. How long a warning must be shown before the
removal is the one choice this record leaves to the operator.

## Context

goatbuilder has four surfaces that other people build on. A deployment's
overlay repository holds a `goatbuilder.yaml` with its peering declarations;
its automation calls goatbuilder's commands and flags and branches on their exit
codes; its pipelines read the `-o json` and `-o yaml` documents; its runbooks
and alerts name `GB-nnn` error codes. Each of these lives outside this
repository, in places a goatbuilder change cannot update.

Before 1.0 no rule protects them. Measured from the release tags, goatbuilder
has had five tagged releases in eight days, and none removed anything.
Versions 0.1.0 to 0.4.0 were never tagged; they exist only as changelog
entries (`crates/goatbuilder/CHANGELOG.md`, 2026-10-28 to 2026-11-02).
`goatbuilder/v0.5.0` was tagged and never published, so it has no binary. The
dates below are the tags'; the changelog dates 0.4.1 to 2026-11-02, a day
before its tag.

| Release | Date | Commands | Flags | GB codes | Removed or retitled since the previous release |
|---|---|---|---|---|---|
| `goatbuilder/v0.4.1` | 2026-11-03 | 39 | 128 | 72 | — |
| `goatbuilder/v0.4.2` | 2026-11-06 | 39 | 129 | 73 | none |
| `goatbuilder/v0.4.3` | 2026-11-08 | 39 | 130 | 74 | none |
| `goatbuilder/v0.5.0` | 2026-11-11 | 47 | 178 | 157 | none |
| `goatbuilder/v0.5.1` | 2026-11-11 | 47 | 178 | 157 | none |

Every configuration file shipped in those releases (15 files and four
documentation examples, one of them in two versions, kept in
`ops/goatbuilder/compat/configs/`) still gets
the outcome its own release gave it. The record so far is additions only, by
habit. A 1.0 needs it by rule, with a check that holds it.

The kept configuration set starts at `goatbuilder/v0.4.1`, the first tagged
release, while the gate's row A2 says "since 0.1.0". No released binary or
tagged tree exists for 0.1.0 to 0.4.0, so nothing can be kept from them with
the outcome a release gave it. This record takes 0.4.1, the first tagged
release, as the floor, and the pull request for row A2 changes the gate's
wording to match.

The rows of the goatbuilder 1.0 gate (`docs/design/goatbuilder-v1-gate.md`,
group A, in review) name the rules and the checks. Rows A2 to A5 are built as
`ops/goatbuilder/compat/` and `ops/ci/check-goatbuilder-compat.sh` (in
review). This record is row A1.

## Decision

### The four surfaces and their rules

This record is the full statement of the rules. The gate's table "What 1.0
promises" is a shorter summary of them, and the rules below are a consistent
superset of it: where the gate says less (positional arguments, a refused
file accepted later, a document with no version, a retired code), this record
decides. The pull request for row A1 changes the gate to cite this record.

| Surface | Defined in | Rule from 1.0 |
|---|---|---|
| The configuration: `goatbuilder.yaml`, including its `peerings:` declarations | `ops/goatbuilder/schema/goatbuilder-v1.schema.json` | A file that validates on 1.0 validates on every 1.x and means the same thing. A new field is optional, and leaving it out gives the 1.0 behavior. A file 1.0 refuses may be accepted later, when a field is added; the pull request that does so changes that file's kept outcome, so the change is reviewed |
| Commands, flags and exit codes | [the command reference](../reference/goatbuilder/command-reference.md); [the contract, "Exit codes"](../reference/goatbuilder/06-contract.md) | No command, flag or positional argument is removed, renamed or changed in meaning in 1.x. A flag that takes a value keeps taking one. An optional argument does not become required. Exit codes 0 to 4 keep their meaning |
| The `-o json` and `-o yaml` documents | [goatbuilder-output.md](../design/goatbuilder-output.md) | Each document carries its version in `schema`; one printed without a version gains one as an added field. Within one version, fields are added and never removed or retyped. A change that would remove or retype a field is a new version, and a new version waits for the next major release |
| Error codes `GB-nnn` | `ops/errors/catalog.yaml` | A code is never reused for a different fault, and its title does not change. A code that is no longer printed stays in the catalog with `retired: goatbuilder/vX.Y.Z`; a new fault gets a new code |

### What is not promised

The text of the tables printed for a person. The shell scripts beside the
binary, which stay documented and unsupported (ADR 1201). The internals of a
converger. The `goat` client, which has its own releases. Each of these may
change in any release.

### Deprecation

- A command, flag, positional argument or configuration field that is to be
  removed is first **deprecated**: an entry in
  `ops/goatbuilder/compat/deprecations.yaml` names it, its replacement and the
  first release that warns.
- From that release, using it still works and prints a warning on stderr that
  names the replacement and the release that will remove it. (Designed, not
  built: the warning, and a `deprecations` field added to the run's `-o json`
  document so a pipeline sees it without reading stderr.)
- It is removed only in the next major release, and only after the
  deprecation period below has passed.
- An error code is never deprecated and removed; it is retired and stays.

### The check that holds the rules

On every goatbuilder pull request, `ops/ci/check-goatbuilder-compat.sh` compares
the build with the last release (`ops/goatbuilder/compat/baseline.yaml` names
it). It refuses a configuration file whose outcome changed, a command or flag
that is gone without a deprecation entry, a kept document that lost or retyped
a field, and a code that is gone or retitled without `retired:`. After each
release is published, one pull request moves the baseline to it with
`compat.py refresh`, recorded from the released binary. A release's own pull
request is checked against the release before it.

### When it starts

The check runs from now, on 0.x. In 0.x a deprecation entry excuses a removal
at once; the release notes say what was removed. From `goatbuilder/v1.0.0` the
rules above hold in full, and removal waits for 2.0.

### The decision left open: the deprecation period

How long must a deprecated command, flag or field keep working, with its
warning, before a major release may remove it? The gate proposes "at least two
minor releases". The options, with what each costs:

| Option | Rule | Who it protects | What it costs | How the check enforces it |
|---|---|---|---|---|
| A | At least two minor releases | A deployment that updates at least once every two minor releases | At the cadence above (a release every two to three days in November 2026), two minor releases can be under a week, so a deployment that updates monthly can meet the removal without having seen a warning | `deprecated_in` must be at least two minor versions below the major that removes it |
| B | At least two minor releases **and** 90 days from the first warning release | A deployment that updates quarterly | A major release waits up to 90 days for anything deprecated late in the line | As A, plus the tag date of `deprecated_in`, read from the tag, at least 90 days before the major's release pull request |
| C | One whole major line: deprecated in 1.x, removed no earlier than 3.0 | Every deployment that is on any 2.x release | Deprecated code stays in the binary for a whole major line; the binary and its tests carry both forms longer | `deprecated_in` must name a major below the one before the removal |
| D | One minor release | Deployments whose callers are all in repositories the same team updates in step | A deployment outside that team can see the warning for one release only | `deprecated_in` must be a minor version below the major that removes it |

The evidence that would decide between them is how often a deployment updates
its pinned goatcore commit and its goatbuilder binary, and how many callers
of goatbuilder live outside the repositories this team updates. Neither is
measured. The release cadence above is measured; it is the cadence of 0.x,
and a 1.x line may be slower.

When the operator chooses, this record is amended with the chosen option, the
check gains the rule in the last column, and the record becomes Accepted.

Which of A to D sets the deprecation period?

## Alternatives considered

**No written promise, additions by habit.** This is the state through 0.5.1,
and it has held: nothing has been removed. It depends on every author knowing
the habit, and it gives a deployment no basis to update without reading every
change. Rejected for 1.0: a version number that promises nothing is not a
1.0.

**Promise the human tables too.** A script that parses a table would then be
safe. It would freeze the wording of every row, which the output standard
(`goatbuilder-output.md`) is still improving, and scripts already have
`-o json`. Rejected; the tables stay free to improve.

**Allow a new document version within 1.x, produced beside the old one and
chosen by a flag.** This lets a document be restructured without waiting for
2.0. It doubles the documents the binary produces and the documents the check
must keep, for a need no document has shown yet. Not chosen now; a later
record can add it if a document needs it.

**Long-term-support releases instead of a deprecation period.** A named 1.x
release would receive fixes while later releases move on. It needs a team to
backport fixes, which goat does not have, and it does not stop the later
releases from breaking callers. Rejected.

## Consequences

**Positive.** A deployment can take any 1.x release without changing its
files, scripts or pipelines, and the check, not a reader, finds a break.
Runbooks and alerts that name a `GB-nnn` code keep meaning the same fault.
A removal is announced by the tool itself, at the point of use, before it
happens.

**Negative.** A mistake in a 1.0 command or field name is carried to 2.0,
next to its replacement. Every release adds one step, moving the baseline
after publication. The check compares names and the shape of documents, not
behavior: a flag that keeps its name and changes what it does passes the
check and breaks the rule, so review must still catch it. The deprecation
warning and its `-o json` field are designed, not built; until they are, a
deprecation entry is the only notice.

## Assessor view (SCA-R)

This is configuration and change control for an operator tool (NIST SP 800-53
CM-2 baseline configuration, CM-3 configuration change control, SA-10
developer configuration management). The baseline is the last released
binary's behavior, recorded with that binary's SHA-256 and kept under version
control; every change is compared with it before it merges. Stable error codes
keep audit records and incident runbooks interpretable across releases (AU-6).
The residual risk is a change of meaning under an unchanged name, which the
check cannot see; the compensating control is review, and the release notes
that name each deprecation.

## Trigger conditions for revisit

- The operator chooses the deprecation period (this record is then amended and
  accepted).
- A document needs a field removed or retyped within 1.x.
- A deployment reports a break that the check did not catch.
- A pull converger is added to `converger:` (the open decision in the gate's
  group D): it must arrive as a new value without changing the existing ones.
