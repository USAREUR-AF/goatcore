# CI: a pull request runs the workflows a pull request into main runs

| Field | Value |
|---|---|
| Status | Trigger change built. Guard script built. Per-pull-request verdict as a required check on `main`: not built. |
| Related | [ci-results-follow-inputs.md](ci-results-follow-inputs.md) (a check's result follows its inputs) · issue #741 |

## The problem

GitHub decides whether a `pull_request` workflow runs from the event, the pull request's base branch and its changed files. Six workflows also had `branches: [main]` in their `pull_request` trigger: `mermaid-renders`, `rust-audit`, `trivy-advisory`, `peer-name-grammar`, `roster-registry` and `sovereign-pki-registry`.

A pull request based on another pull request's branch (a stacked pull request) ran none of them. When the lower pull request merged, GitHub retargeted the upper one to `main`. A retarget is an `edited` event, which none of these workflows listens for, so they still did not run. None of the six is a required check on `main`, so the upper pull request could merge without them ever having run on its code.

A retarget also changes which files GitHub compares. Path filters are evaluated against the diff from the base at the time of the push. A pull request pushed while based on a stacked branch has a smaller diff than the same commits against `main`, so path-filtered workflows without a branch filter can be missing too. Removing the branch filter fixes the first case only; the guard below catches both.

## What is built

1. **No branch filter on `pull_request` triggers.** The six workflows keep their `paths` filters and drop `branches: [main]`, so a stacked pull request runs them whenever its changed files match. They do not listen for `edited`: the checks have already run on the head commit.
2. **`ops/ci/check-expected-workflows-ran.py <pr>`.** It reads every workflow's `on.pull_request` and `on.pull_request_target` filters (`branches`, `branches-ignore`, `paths`, `paths-ignore`, `types`) from `origin/main`. It evaluates them against the pull request's changed files as if its base were `main`, which gives the workflows the change should have run. A workflow the changed files do not trigger is not expected, so a paths filter never causes a refusal. Each expected workflow needs a completed run, from a pull-request event, on the head commit, matched by workflow file path through the Actions runs API. Otherwise the script refuses (exit 1) and names the workflow. Whoever merges runs it before merging. An unknown trigger filter is an error (exit 2), never a pass.
3. **`--tree` and `--self-test` in CI.** `coupled-fact-guards` runs both. `--tree` refuses a `pull_request` trigger that a `branches` filter limits to `main`, unless the workflow file says why on a line containing `expected-workflows: main-only`. `--self-test` covers GitHub's pattern syntax and three cases from the issue: a stacked pull request that changes a `*.md` file and has no `render` run (refused, named); a pull request into `main` that has it (passes); and a change no filter matches.

## Limits

- The filters come from `main`, while GitHub uses the workflow files in the pull request's merge commit. A pull request that changes a workflow's triggers is judged by `main`'s version.
- GitHub evaluates path filters against at most 300 changed files. Beyond that the script may expect a workflow GitHub skipped, which causes a refusal, never a missed one. The script prints a note when this applies.
- The script checks that a run completed, not that it passed. The merge rules decide whether a failed run blocks.

## Not built

Making the per-pull-request verdict a required check on `main`. That is a separate decision: the check would have to wait for every other workflow on the head before it could judge.
