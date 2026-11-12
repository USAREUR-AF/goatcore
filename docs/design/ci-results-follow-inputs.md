# CI results follow inputs — fixed inputs, kept stages, and a pass that is not run twice

| Field | Value |
|---|---|
| Status | **Designed.** First instance: the two-sovereign end-to-end job (`.github/workflows/federation-e2e.yml`), in work. No workflow follows all three rules yet |
| Related | [ci-redundant-rebuild-guard.md](ci-redundant-rebuild-guard.md) (the same idea for image builds, by changed paths) · [deployment-e2e.md](deployment-e2e.md) · [goatbuilder.md](goatbuilder.md), contract clause C1 (a deploy applies a content-addressed ref) |
| Audience | whoever writes or changes a CI workflow, and whoever decides whether a check may be skipped |

## What this is

A check's result should depend only on its inputs. When that holds, three
things follow: the check gives the same answer every time it runs on the same
content, a failed run can continue from the stage that failed, and content
that has already passed does not need to run again.

This document states those three rules, what may never be reused, and the
test that shows a workflow follows them. `goatbuilder` already applies the
same idea to deployments: a deploy is tied to a content digest, and the same
digest is not converged twice.

## The three rules

### Rule 1. Inputs are fixed

Everything a job reads is either in the checkout or named by a digest.

- A job does not download packages, images or tools by a floating name while
  it runs. A package the test hosts need is installed when their image is
  built. A tool the job needs comes from a pinned action, a pinned image or a
  lock file.
- A base image is named by digest and pulled from a registry the project
  controls or mirrors.
- A compiler toolchain is named by its exact release, never by a channel.
  Every workflow's Rust toolchain is an exact version
  (`dtolnay/rust-toolchain@1.100.0`, or `toolchain: '1.100.0'`), and moving
  it is a change of its own. Built and checked: `ops/ci/check-rust-toolchain-pinned.sh`
  refuses `@stable`, `@beta`, `@nightly` and a `toolchain:` set to a channel,
  and runs in the coupled-fact-guards workflow on every workflow change.
- When a job cannot avoid a network source, that source is named in the
  workflow as an input, and the job reports a failure to reach it as
  `environment`, with a distinct exit, so it is never read as a failure of the
  change.

A job that follows this rule passes with outbound network access limited to
the registries its pinned inputs come from.

### Rule 2. Stages keep their results

A long job is split into stages, each a job of its own, that hand their
outputs forward as artifacts: build, then each scenario.

- A failure in one stage does not discard the stages before it.
- Running the failed jobs again repeats only the stage that failed.
- Scenarios that do not depend on one another run side by side.

### Rule 3. A recorded pass is reused when the inputs are identical

Each job declares its inputs. A key is computed from them. If a pass is
recorded for that key, the job reports success in seconds and links to the
run that earned it.

**The key covers:**

- the content of every path the job reads (for a workflow with a `paths:`
  trigger, at least those paths);
- the workflow file itself and every script it calls;
- the lock files and toolchain versions;
- the digest of every image the job uses.

**The record:**

- is written only after the job has passed in full, by that job;
- names the key, the run that passed, and the commit it ran on;
- can be read from any branch, because the same content reaches `main` on a
  different commit than the one it was tested on;
- cannot be written by a pull request from a fork, and is verified before it
  is trusted. A record is a claim that a check passed, so it is protected the
  way a release artifact is.

## What is never reused

| Check | Why |
|---|---|
| Every commit is signed and verified; the contributor check; the leak gate on the diff | They are statements about this pull request, not about content |
| Vulnerability scans | Their input includes the advisory list of the day. They run on a schedule and when dependencies change, and a new advisory opens an issue instead of failing unrelated pull requests |
| Anything that reads a clock, a live service or a goatnet | Its input is not in the key |
| A push to `main` and a release tag | The full run is repeated there without reuse, so a wrong input declaration is found within one merge |

## The risk, and the two guards

Rule 3 is only as correct as the declared inputs. A job that reads a file it
did not declare will reuse a pass it has not earned.

1. **Declare broadly.** The key covers directories, not hand-picked files,
   and always the workflow and its scripts. A wider key costs an extra run; a
   narrower one costs a false pass.
2. **One full run that never reuses.** The run on `main`, and a nightly run,
   execute every stage with no reuse. A pass that was reused and then fails
   there is a defect in the declaration, and is fixed by widening the key.

## How a workflow is shown to follow the rules

A workflow claims this document only when these four cases pass, and the
workflow's header comment says which run showed each.

| Case | Expected |
|---|---|
| Run with outbound network limited to the pinned registries | Passes (rule 1) |
| Fail one scenario on purpose, then run the failed jobs again | Only that scenario runs again (rule 2) |
| Run twice on identical inputs | The second run reports the recorded pass and links to the first (rule 3) |
| Change one declared input; change one path outside the declaration | The first runs in full; the second reuses the pass (rule 3) |

## First instance: the two-sovereign end-to-end job

Status: WIP — gate: the four cases above pass on `.github/workflows/federation-e2e.yml`.

The job takes about 50 minutes and runs four scenarios in one job. On one
day it failed twice for a reason outside the change: its hosts fetch a
package from a distribution mirror while the job runs, and its setup step
installs tools from the same mirror. The same content had passed an hour
earlier on another branch.

| Rule | Change to this job |
|---|---|
| 1 | The host image (`ops/federation-e2e/host/Dockerfile`) installs every package a scenario needs when it is built. The setup step's tools come from the same pinned image or a pinned action. No `apt-get` at run time |
| 2 | One build job produces the two binaries and the host image as artifacts. Each scenario is a job that takes them |
| 3 | The key is the content of the workflow's `paths:` list, the Cargo lock file, the toolchain version and the host image digest. A pass is recorded per scenario |

## Not decided here

- Where the records are kept. The store must be readable from every branch,
  writable only by the project's own workflow runs, and verifiable. A cache
  that is private to one branch does not meet the first requirement.
- Which other workflows follow next. The image builds already skip by
  changed paths ([ci-redundant-rebuild-guard.md](ci-redundant-rebuild-guard.md));
  the hermetic deployment job is the next longest.
