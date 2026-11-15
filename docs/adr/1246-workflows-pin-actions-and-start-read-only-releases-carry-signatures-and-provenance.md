# ADR 1246 — Workflows pin actions by commit and start read-only, releases carry signatures and provenance, and OpenSSF Scorecard measures both

| Field | Value |
|---|---|
| Status | Proposed. Decisions 3, 4 and 6 are built on `main`. Decisions 1 and 2 are built in part on `main`, and the rest is in review in pull request #915. Decision 5 is built for two of four release workflows. |
| Date | 2026-11-15 |
| Repo scope | goat core (`.github/workflows/`, `.github/dependabot.yml`, every `osv-scanner.toml`, `.github/release-notes/`) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 0703](0703-pinned-digests-and-pin-regression-ci.md) (container images are pinned by digest) · [ADR 0707](0707-rust-toolchain-pinning.md) (advisories that stay open are listed with a reason) · [ADR 0820](0820-continuous-vuln-scanning.md) (continuous vulnerability scanning) · [ADR 1221](1221-outside-contributions-need-a-known-author-or-a-recorded-security-review.md) (`contributor-trust`) · [ADR 1226](1226-every-commit-in-a-pull-request-is-signed-and-verified.md) (signed commits) · [`../design/ci-discipline.md`](../design/ci-discipline.md) · [`../design/openssf-scorecard.md`](../design/openssf-scorecard.md) |

## TL;DR (for a layperson)

goat is built, tested and released by automated jobs that run on GitHub. Those jobs use small programs written by other projects, and they hold a token that can change this repository. This decision fixes the exact version of each of those programs, gives each job only the access it needs, and attaches to each release a signature per file and a record of which job built it. A person who downloads a release can then check that the file came from this repository's release job. A public tool, OpenSSF Scorecard, scores these practices, and the project uses its result as the measurement.

## Context

ADR 0703 pins container images by digest. Nothing recorded the same rule for the GitHub Actions that workflows call, and on 15 November 2026 the workflows on `main` referenced actions by version tag. A tag can be moved to different code after a review.

A workflow with no `permissions` declaration receives the repository's default token. 41 of 98 workflows on `main` had no top-level declaration on that day.

Three gates run on `pull_request_target`, which holds the base repository's token. Each checked out only the base, and each named its `ref` through an expression read from the pull request event. A reader of the workflow, or a scanner, cannot tell that pattern from one that checks out the pull request's code.

Releases carried `SHA256SUMS` and one keyless signature over it. That proves the whole set. It gives no signature for a single file and no record of the build.

OpenSSF Scorecard v5.4.0 scored the repository 4.0 of 10 on that day. [`openssf-scorecard.md`](../design/openssf-scorecard.md) holds the result per check.

## Decision

1. **Every external action is pinned to a full commit SHA**, with the version in a trailing comment. Dependabot's `github-actions` ecosystem in `.github/dependabot.yml` proposes each new version as a pull request that moves the SHA and the comment together.
2. **Every workflow declares top-level `permissions`, and the default is `contents: read`.** A write scope is declared on the job that needs it and never at the top level.
3. **Under `pull_request_target`, no checkout step takes a `ref` from an event expression.** A gate that needs the pull request's recorded base commit moves to it in a run step, with the SHA passed through `env:`.
4. **A lock file on `main` carries no published advisory without a recorded reason.** A dependency with an advisory is updated. When no fix exists or the advisory does not apply, the file `osv-scanner.toml` beside that lock file records the advisory id and one factual sentence. ADR 0707 defines the list of advisories that stay open. `rust-audit.yml` keeps its `--ignore` list for the advisories `cargo audit` fails on, and `osv-scanner.toml` is a second place that list is read from.
5. **A release carries `<asset>.sigstore.json` per asset and one `<component>.intoto.jsonl` provenance file**, beside `SHA256SUMS` and `SHA256SUMS.cosign-bundle`. The job that writes the checksums signs each asset, writes the provenance with `actions/attest-build-provenance`, and verifies both before the release job attaches them.
6. **CodeQL analyzes every pull request and every push to `main`**, with no path filter ([`codeql-coverage.md`](../design/codeql-coverage.md)).
7. **OpenSSF Scorecard is the measurement.** [`openssf-scorecard.md`](../design/openssf-scorecard.md) records the result per check and is updated in the pull request that changes a result.

The rules a contributor follows are W1 to W4 in [`ci-discipline.md`](../design/ci-discipline.md#rules-for-writing-a-workflow) and the two sections "Dependency advisories" and "Release signatures and provenance" in [`openssf-scorecard.md`](../design/openssf-scorecard.md).

This ADR does not decide whether a pull request needs an approving review. The ruleset on `main` requires none, and Scorecard's Code-Review and Branch-Protection checks score that setting. Changing it is an owner decision.

## Alternatives considered

- **Reference actions by version tag and trust the publisher.** Rejected because the code behind a tag can change with no change in this repository, so a reviewed workflow can run unreviewed code.
- **Write the SLSA provenance with the slsa-github-generator reusable workflow.** Rejected because it is a second workflow that must be called by tag and needs `actions: read`. `actions/attest-build-provenance` runs as one step in the job that already signs, and adds one permission.
- **One `osv-scanner.toml` at the repository root.** Rejected because Scorecard and `osv-scanner` read the file in each lock file's own directory, so a root file covers the root lock file only.
- **Keep the event expression as the checkout `ref` in the three gates, because each checks out only the base.** Rejected because the safe use and the unsafe use look the same in the workflow file.

## Consequences

**Positive.** A workflow runs the action code that was reviewed. A token in a job that builds or tests cannot write to the repository. A downloader can verify one file and its build. A public, repeatable measurement shows the state of each practice.

**Negative.** A workflow change takes one more step, finding the commit behind a version. Dependabot opens a weekly pull request for action updates, and an owner reads it. A top-level `permissions` block sets unnamed scopes to `none`, so a job that relied on a default scope fails until it declares that scope. No check refuses a workflow that breaks decisions 1 to 3. A reviewer and Scorecard read them.

## Assessor view (SCA-R)

The decision maps to SR-3 and SR-4 (supply chain controls and provenance), SA-10 (developer configuration management), SI-7 (software integrity), AC-6 (least privilege, for the workflow token) and RA-5 (vulnerability monitoring and scanning). Evidence is the workflow files, each `osv-scanner.toml`, the files on a release page and a Scorecard run. Residual risk: a pinned action can still download other code when it runs, and a pin does not cover that code.

## Trigger conditions for revisit

- A check that enforces decisions 1 to 3 is built. This ADR's status and `ci-discipline.md` then name it.
- Scorecard changes what a check reads, so that a practice here stops matching it.
- The owners decide to require an approving review on `main`.
- GitHub offers immutable action references that make a commit SHA unnecessary.
