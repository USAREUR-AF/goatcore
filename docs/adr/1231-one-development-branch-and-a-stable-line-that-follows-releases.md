# ADR 1231 — One development branch; a stable line that follows releases; outside contributors answered within one working day

| Field | Value |
|---|---|
| Status | Accepted — decided by the operator on 2026-11-13 |
| Date | 2026-11-13 |
| Repo scope | Multi (`goatcore`, and the public repositories that follow it) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 0109](0109-git-collaboration-trunk-based.md) (trunk-based collaboration), [ADR 1221](1221-outside-contributions-need-a-known-author-or-a-recorded-security-review.md) (the `contributor-trust` check), [ADR 1228](1228-goatbuilder-keeps-its-four-surfaces-and-removes-only-in-a-major-release.md) (what goatbuilder keeps compatible) |

## TL;DR (for a layperson)

goat changes many times a day, and people outside the team build on it and
contribute to it. This record keeps one place where all work lands, the
branch `main`, and gives everyone else a fixed thing to build on: a release
of the whole repository, and a branch named `stable` that always points at
the newest one. Nobody develops on `stable`. The project already released by
tags; this makes the tags consistent. It also commits the maintainers to
answer every outside pull request and issue within one working day, because
a contributor who hears nothing does not come back.

## Context

In its first ten days as a public repository, `goatcore` merged 345 pull
requests, up to 67 in a day. Thirteen came from forks. One of the thirteen
was blocked by a conflict with `main`; the ruleset does not require a pull
request to be up to date before it merges. The median time from opening to
merging was 0.8 hours for the most active maintainer account and 23.1 hours
for an outside author, and 17 of 25 issues from outside authors had a label
and an assignee and no written reply.

The repository carried 18 tags in five families (`goatbuilder/v`,
`goat-cli-v`, `goatcore/v`, and three others). The example environment
pinned `goatcore/v0.1.0`, and `GOVERNANCE.md` did not say what a
`goatcore/v` tag promised or when the next would come. No page said which
versions go together.

## Decision

1. **All work lands on `main`.** Maintainers and outside contributors open
   pull requests against `main`. No other branch accepts original work.
2. **A whole-tree release is the tag `goatcore/vX.Y.Z`.** A maintainer cuts
   it, signed, at the commit of the goatbuilder release with the same
   version, after that commit has run in a standing environment. There is
   one release train: the whole-tree release and the goatbuilder release
   are one commit and one version.
3. **The stable line follows minor releases.** A whole-tree release is cut
   at each goatbuilder minor release (`X.Y.0`). It is cut at a patch release
   only when that patch carries a fix that people on the stable line need.
4. **`stable` is a pointer.** The branch `stable` is fast-forwarded to each
   whole-tree release by `.github/workflows/stable.yml`, which runs
   `ops/release/stable.py check` first. A ruleset refuses every other
   update. `stable` takes no pull requests.
5. **Each whole-tree release says which versions go together.** Its release
   page lists the newest release of each component contained in that
   commit, and the versions of other projects it works with
   (`ops/release/works-with.md`).
6. **A released version is fixed on `main` first.** A branch `release/X.Y`
   is created only when a supported release needs a fix that cannot wait for
   the next release; the fix merges to `main` and is cherry-picked.
7. **Outside contributors get a first reply within one working day.** The
   reply is in words: who has it and what happens next. Every pull request
   from outside the maintainers carries the label `community`, maintainers
   work that list first, and a maintainer rebases an outside pull request
   that conflicts with `main`.
8. **A large program is built on an integration branch.** A program of many
   pull requests uses `integration/<name>` and reaches `main` as a small
   number of reviewed changes. A program whose releases are cut from `main`
   every few days stays on `main`.

The full description, with the status of each part, is
[community-and-stable-line.md](../design/community-and-stable-line.md).

## Alternatives considered

**A second development branch, `stable-main`, that accepts pull requests.**
Every change merged there must also be applied to `main`, which by then
differs by hundreds of commits, and the two branches drift. Rejected.

**A calm default branch and a fast `next` branch.** The same promotion work,
and every workflow, pin and document that names `main` changes. Rejected.

**A whole-tree release on its own weekly schedule.** A second release train
beside goatbuilder's, with two version numbers for one commit. Rejected for
one train.

## Consequences

**Positive.** A user has one answer to "what do I build on": `stable`, or
the tag it points at. A contributor has one place to send work and knows
when to expect an answer. `main` keeps its pace.

**Negative.** `stable` is as old as the last minor release, so a fix on
`main` reaches the stable line only at the next release. The reply
commitment is kept by people; nothing enforces it. Cutting the whole-tree
tag is one more signed act per minor release.

## Assessor view (SCA-R)

Configuration management (NIST SP 800-53 CM-2, CM-3): the baseline a
deployment builds on is a signed tag, and the branch that names it moves
only by a workflow that checks the tag against `main` and against the
goatbuilder release. No person pushes to `stable`.

## Trigger conditions for revisit

- A supported release needs a fix and `main` can no longer supply it by a
  new release: the first `release/X.Y` branch.
- The median first reply to an outside author exceeds one working day for a
  month.
- Ruleset "require branches to be up to date" is proposed for `main`: at
  this merge rate it needs a merge queue first.
