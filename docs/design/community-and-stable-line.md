# The community and the stable line — where work lands, what to build on, and what to expect

| Field | Value |
|---|---|
| Status | **Built: the `stable` workflow, its check and its guard, the release page with the versions that go together, and the `community` label. Designed: the parts marked so in "Status of each part".** |
| Anchor ADR | [ADR 1231](../adr/1231-one-development-branch-and-a-stable-line-that-follows-releases.md) |
| Composes with | [ADR 1228](../adr/1228-goatbuilder-keeps-its-four-surfaces-and-removes-only-in-a-major-release.md) (compatibility and deprecation), [ADR 1221](../adr/1221-outside-contributions-need-a-known-author-or-a-recorded-security-review.md) (`contributor-trust`), [release-candidate-validation.md](release-candidate-validation.md) |
| Audience | anyone who builds on goat or contributes to it, and the maintainers |

## The approach in one paragraph

goat already ships by releases and tags. This design makes that pattern
consistent, so that `main` keeps moving fast and everyone knows which
version to build on and what will not change under them. The maintainers also aim
to answer validated outside contributors within one working day.

## Three names, three uses

| Name | What it is | Use it when |
|---|---|---|
| `main` | The one development branch. Every change lands here, many times a day | You are contributing. Open your pull request against `main` |
| `goatcore/vX.Y.Z` | A release of the whole repository: a signed tag at the commit of the goatbuilder release with the same version, cut after that commit has run in a standing environment | You want a fixed reference. Pin this tag |
| `stable` | A branch that always points at the newest `goatcore/v` tag. It takes no pull requests | You want the newest release by a name that does not change |

The component tags (`goatbuilder/vX.Y.Z`, `goat-cli-vX.Y.Z` and the others)
stay as they are: each publishes one component's binaries. The whole-tree
release is cut at a goatbuilder minor release, so the two share a commit and
a version.

## How the stable line moves

1. A goatbuilder minor release `X.Y.0` is tagged from `main` and published.
2. The maintainers run it in a standing environment.
3. A maintainer cuts the signed tag `goatcore/vX.Y.0` at the same commit.
4. `.github/workflows/stable.yml` runs `ops/release/stable.py check`. The
   check refuses the tag unless it names a commit on `main`, a tag
   `goatbuilder/vX.Y.0` names the same commit, and `stable` is an ancestor
   of it.
5. The workflow fast-forwards `stable` to that commit and publishes the
   release page.

Nothing else moves `stable`. `.github/workflows/stable-guard.yml` closes a
pull request opened against it and fails when a person pushes a commit that
is not a whole-tree release. A ruleset refuses deleting the branch and
moving it backwards.

A patch release moves the stable line only when a maintainer cuts
`goatcore/vX.Y.Z` for it, which they do when the patch carries a fix that
people on the stable line need.

## Which versions go together

The release page of each `goatcore/v` tag lists the newest release of each
component contained in that commit, written by
`ops/release/stable.py table`, and the versions of other projects the
release works with, kept in `ops/release/works-with.md`.

## What stays compatible

From goatbuilder 1.0, its configuration, commands and flags, machine-readable
documents and error codes change only by additions. A command, flag or field
that is to be removed keeps working, with a warning that names its
replacement, for at least two minor releases and at least 90 days, and is
removed only in the next major release ([ADR 1228](../adr/1228-goatbuilder-keeps-its-four-surfaces-and-removes-only-in-a-major-release.md)).
Before 1.0, a minor release may change an interface, and the release notes
say so. `docs/RELEASE-CONTENTS.md` lists each component and its status.

## What an outside contributor can expect

| What to expect | How it is kept |
|---|---|
| Once validated, a first reply within one working day, as an aim and not a guarantee | A contributor is validated when registered in a goat community environment with a linked GitHub account, or with an earlier merged pull request (the tier `known` of `contributor-trust`). Validation takes work from both sides and has no fixed time. The reply is in words: who has it, and what happens next |
| Outside pull requests are worked first | The `contributor-trust` check labels them `community`; maintainers work that list before their own |
| Maintainers carry the rebase | A pull request does not have to be up to date with `main` to merge. When it conflicts, a maintainer rebases it |
| Work is not replaced without a word | A maintainer's pull request that carries the same change says so in the contributor's pull request before that one is closed, and credits the commits |

A pull request from an author who is not yet known to the project also
needs a recorded security review before it merges
([CONTRIBUTING.md](../../CONTRIBUTING.md), "Outside contributions").

## Large programs

A program of many pull requests is built on a branch `integration/<name>`
and reaches `main` as a small number of reviewed changes. A program whose
releases are tagged from `main` every few days stays on `main`.

## Fixing a released version

Fixes land on `main` and are released by tag. A branch `release/X.Y` is
created from a tag only when a supported release needs a fix that cannot
wait for the next release; the fix merges to `main` first and is
cherry-picked to the branch.

## The changelog, one file per entry

goatbuilder's changelog, `crates/goatbuilder/CHANGELOG.md`, is rendered
from one file per entry, so a pull request that adds an entry edits no
shared file and two such pull requests never conflict.

**Where entries live.** Each entry is a file
`crates/goatbuilder/changelog.d/<slug>.md`, its name in lower case letters,
digits and hyphens. The directory's `README.md` tells a contributor how to
write one and is not an entry.

**What an entry declares.** A header between two `---` lines, then the body:

- `heading`: one of the three headings every release uses, "For someone
  deploying", "For someone contributing" or "To be removed";
- `title`: the entry's one-line heading, for example "Fixed: a federate run
  killed in phase 11 resumes and brings the link up";
- the body, in Markdown, with no heading of level 1 to 3, ending with the
  pull request's number.

**The Unreleased section.** `ops/release/changelog.py write` renders the
entries between two marker lines under `## Unreleased`: each heading that
has an entry, as `###`, and under it each entry's title, as `####`, then its
body, in file-name order. A heading with no entry is left out.

**The check.** `ops/release/changelog.py check` fails when an entry is
malformed, when the text between the markers differs from the render, or
when there is text between `## Unreleased` and the first marker or between
the last marker and the newest release. It runs in the `docs-guards`
workflow after its own self-test (`--self-test`), which proves each refusal
and the release step on a copy.

**A release.** The version pull request runs `ops/release/changelog.py
release <version> <date>`. It writes `## <version> — <date>` above the
newest release, with all three headings, and "Nothing." under a heading with
no entry; it removes the entry files and empties the Unreleased section. The
release's opening summary, and any grouping such as "Not in <version>", are
then added by hand in the same pull request. A released section is never
rendered again: after its release it is ordinary text, and the check reads
only the Unreleased section.

The phase catalog's tables are rendered the same way: one source, a
generated region between marker lines, a check that the region equals the
render, and `--write` to rewrite it (`ops/ci/check-phase-catalog.py`).

## Status of each part

| Part | Status |
|---|---|
| `stable` workflow and `ops/release/stable.py`, with tests in `tests/test_stable_line.py` | built |
| Release page with the versions that go together | built |
| The branch `stable` | created by the first `goatcore/v` tag cut at a goatbuilder release |
| Guard: a pull request against `stable` is closed, and a push by a person is checked (`.github/workflows/stable-guard.yml`) | built |
| Ruleset `stable-follows-releases`: `stable` cannot be deleted or moved backwards | built |
| `community` label applied by `contributor-trust` | built |
| First reply to a validated contributor within one working day | an aim, kept by the maintainers |
| Release notes with the headings "for someone deploying", "for someone contributing", "to be removed" | designed |
| The example environments move to each new `goatcore/v` tag by a pull request the release opens | designed |
| goatbuilder's changelog rendered from one file per entry (`crates/goatbuilder/changelog.d/`, `ops/release/changelog.py`), checked in `docs-guards` | built, self-test only |
| ADR index and error-code reference rendered from one file per entry, so that a new entry edits no shared file | designed |
| A column in `docs/RELEASE-CONTENTS.md` saying whether each component's interface is kept between releases | designed |
