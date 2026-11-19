# CI discipline: workflow rules, and a guard that nothing runs is not a guard

| Field | Value |
|---|---|
| Status | Both checks built, and run on every pull request by the `ops-ci-guards` job in `ci.yml`. The allowlisted guards are not built into CI; each section of the allowlist says what wiring it waits for. Rules W1 to W4 for writing a workflow are written here and no check enforces them. |
| Related | [ci-results-follow-inputs.md](ci-results-follow-inputs.md) · [ci-redundant-rebuild-guard.md](ci-redundant-rebuild-guard.md) · [openssf-scorecard.md](openssf-scorecard.md) · [ADR 1246](../adr/1246-workflows-pin-actions-and-start-read-only-releases-carry-signatures-and-provenance.md) |

## Workflow rules

`ops/ci/check-workflow-discipline.py` reads every file in `.github/workflows/` and refuses any that breaks one of these rules:

| Rule | What it requires | Why |
|---|---|---|
| R1 | Every job that runs steps declares `timeout-minutes`. | A hung job otherwise holds a runner for six hours. |
| R2 | Every workflow triggered by `pull_request`, `pull_request_target` or `push` declares a `concurrency` group, on the workflow or on every job. | A push that has been superseded is cancelled instead of finished. |
| R3 | No schedule fires more than once an hour: the minute field of every cron is one literal minute. | A frequent schedule spends runner time without new inputs. |
| R4 | A pull-request workflow with no `paths` or `paths-ignore` filter is listed in `ops/ci/unfiltered-pr-workflows.txt` with its reason, and every listed file exists. | A workflow that runs on every pull request is a decision, so it is written down. The list shrinks; it does not rot. |
| R5 | Every export to the GitHub Actions cache (`cache-to: type=gha...` or a `--cache-to type=gha...` argument) carries `ignore-error=true`. A workflow not yet changed is listed in `ops/ci/gha-cache-export-exceptions.txt` with its reason; a listed file that complies, or that no longer exists, fails. | The cache service can lose a blob between listing and reading it. Buildx then fails the whole build on the export with `404 BlobNotFound`, though the image built. A cache is an optimisation, so losing it must not fail a build. Import already tolerates it: buildx logs a failed `cache-from` and builds without that cache. |

Eight workflows are listed under R4 today. `leak-gate` and `contributor-trust` are required checks on `main` and must report on every pull request. `ci`, `agent-references` and `slot-wal-cap` scan whatever a change touches. `codeql` analyzes every pull request and every push to `main`, so that every merged commit carries a code-scanning result; a run takes about three minutes. `acl-assertions` always reports and decides inside its jobs whether to build. `stable-guard` is filtered by branch instead of by path.

## Path-filtered workflows run weekly on main

A workflow with a `paths` filter runs only when a change touches those paths, so an upstream change (a package withdrawn from a mirror, an image tag removed) leaves it broken until an unrelated pull request touches them. `weekly-path-filtered.yml` starts every path-filtered workflow on `main` each Monday through `workflow_dispatch` (`ops/ci/weekly-path-filtered.py`), opens one issue per red workflow and closes it when the workflow is green again. A path-filtered workflow therefore declares `workflow_dispatch`; one that reads pull-request context and cannot run on `main` is listed with its reason in `ops/ci/weekly-path-filtered-exceptions.txt` (`tests/test_weekly_path_filtered.py` enforces this).

## Rules for writing a workflow

A new workflow, and every line you add to an existing one, follows these four rules ([ADR 1246](../adr/1246-workflows-pin-actions-and-start-read-only-releases-carry-signatures-and-provenance.md)). OpenSSF Scorecard reads each of them; [openssf-scorecard.md](openssf-scorecard.md) has the result per check.

| Rule | What it requires | Why |
|---|---|---|
| W1 | Every external action is referenced by its full 40-character commit SHA, with the version in a trailing comment: `uses: actions/checkout@<40-character SHA> # v4.2.2`. A local action or reusable workflow (`uses: ./...`) needs no pin. | A tag or a branch can be moved to different code after review. A commit SHA cannot. |
| W2 | Every workflow declares `permissions` at the top level. The default is `contents: read`. | A workflow with no declaration receives the repository's default token, which can hold write scopes that no job uses. |
| W3 | A write scope (`contents: write`, `packages: write`, `id-token: write`, `attestations: write`, `pull-requests: write`, `security-events: write`) is declared on the one job that needs it, and never at the top level. | A step in another job of the same workflow then cannot use that scope. |
| W4 | Under `pull_request_target`, a checkout step takes no `ref` from an event expression (`github.event.pull_request.*`). Check out with no `ref`, which gives the base branch, and never build or run the pull request's code. | A `pull_request_target` run holds the base repository's token and secrets. A `ref` from the event can point that run at code its author controls. |

How to follow each rule:

- **W1.** Find the commit a version tag points at with `gh api repos/<owner>/<action>/commits/<tag> --jq .sha`, and write the tag in the comment. Dependabot moves the SHA and the comment together when the action publishes a new version.
- **W2 and W3.** A top-level `permissions` block sets every scope it does not name to `none`. A job that needs more declares its own `permissions` with every scope it uses, `contents: read` included. A job that pulls from the package registry, for example through `.github/actions/apt-failover`, declares `packages: read`. A job that calls a reusable workflow grants every scope the called workflow asks for; `ops/ci/check-reusable-workflow-permissions.py` refuses a caller that grants less.
- **W4.** `leak-gate.yml` and `agent-references.yml` check out with no `ref`, pass `github.event.pull_request.base.sha` to a run step through `env:`, and run `git checkout --detach "$BASE_SHA"` to use the recorded base's scripts. If that base predates the commit-message scanner, `agent-references.yml` reads the scanner from the commit that supplied the workflow (`GITHUB_WORKFLOW_SHA`). `contributor-trust.yml` checks out `github.base_ref`, the base branch by name. The pull request's head is fetched as data for the scan and is never executed.

Status of the rules on `main`:

| Rule | Status |
|---|---|
| W1 | Built in 392 of 446 external action references (#915), and `.github/dependabot.yml` keeps the pins current through its `github-actions` entry. The other references are in four workflow files not yet converted. See [`.github/ACTION-PINS.md`](../../.github/ACTION-PINS.md). |
| W2, W3 | Built in 96 of 98 workflows (#915). Two workflow files are not yet converted. |
| W4 | Built in the three `pull_request_target` workflows that check out code: `leak-gate.yml`, `agent-references.yml` and `contributor-trust.yml`. `stable-guard.yml` runs no checkout under `pull_request_target`. |

No check refuses a workflow that breaks W1 to W4. Before you push a workflow change, run Scorecard on your checkout:

```bash
scorecard --local=. --checks=Pinned-Dependencies,Token-Permissions,Dangerous-Workflow --show-details
```

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
- A check for rules W1 to W4. A reviewer and Scorecard read them.
