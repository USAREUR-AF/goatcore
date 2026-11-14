# CI discipline: workflow rules, and a guard that nothing runs is not a guard

| Field | Value |
|---|---|
| Status | Both checks built, and run on every pull request by the `ops-ci-guards` job in `ci.yml`. The allowlisted guards are not built into CI; each section of the allowlist says what wiring it waits for. |
| Related | [ci-results-follow-inputs.md](ci-results-follow-inputs.md) · [ci-redundant-rebuild-guard.md](ci-redundant-rebuild-guard.md) |

## Workflow rules

`ops/ci/check-workflow-discipline.py` reads every file in `.github/workflows/` and refuses any that breaks one of these rules:

| Rule | What it requires | Why |
|---|---|---|
| R1 | Every job that runs steps declares `timeout-minutes`. | A hung job otherwise holds a runner for six hours. |
| R2 | Every workflow triggered by `pull_request`, `pull_request_target` or `push` declares a `concurrency` group, on the workflow or on every job. | A push that has been superseded is cancelled instead of finished. |
| R3 | No schedule fires more than once an hour: the minute field of every cron is one literal minute. | A frequent schedule spends runner time without new inputs. |
| R4 | A pull-request workflow with no `paths` or `paths-ignore` filter is listed in `ops/ci/unfiltered-pr-workflows.txt` with its reason, and every listed file exists. | A workflow that runs on every pull request is a decision, so it is written down. The list shrinks; it does not rot. |

Six workflows are listed under R4 today. `leak-gate` and `contributor-trust` are required checks on `main` and must report on every pull request. `ci` and `agent-references` scan whatever a change touches. `acl-assertions` always reports and decides inside its jobs whether to build. `stable-guard` is filtered by branch instead of by path.

## Every ops/ci script is run by something

`ops/ci/check-ci-scripts-wired.py` refuses an `ops/ci/check-*`, `ops/ci/test-*` or `ops/ci/test_*` file whose name appears in nothing that runs: `.github/workflows/`, `tests/` (run by `ci.yml` through `bats tests/` and pytest), `ops/scripts/`, `ops/ansible/roles/`, the `Makefile`, or git and session hooks. A mention inside `ops/ci` itself does not count.

`ops/ci/ci-scripts-wired-allowlist.txt` lists the files that cannot run in this repository's CI today, grouped by reason:

- **Needs a live host or a NetBird API token.** These belong on a management-host timer.
- **Runs against a goatnet's environment repository.** Its group_vars, doors, Terraform roots or assembled tree are never in this repository. On this tree each of these refuses as vacuous instead of passing.
- **Checks the operator's local secret values**, and skips without them.
- **Needs a base and a head revision.**
- **Has its subject in another repository.**
- **Fails on `main`.** Each is fixed before it is wired.

The list is shrink-only. Wiring a listed file makes its line stale, and the check refuses until the line is removed. A new script that nothing runs fails on its own pull request.

## Where they run

The `ops-ci-guards` job in `ci.yml` runs on every pull request and on every push to `main`. It runs both checks with their `--self-test`, then each guard that judges the tree from a plain checkout. The step runs every guard even after one fails and names each failure.

## Not built

- Wiring the allowlisted guards. Each needs the fix or the environment its section names.
- Running `check-warrant-narrowing.py` on each pull request's own diff.
