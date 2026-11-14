# CI redundant-rebuild guard for image-building workflows

| Field | Value |
|---|---|
| Status | Authored 2026-05-17. Applied to all image-building workflows in `.github/workflows/` (11 as of 2026-07-27). **Amended 2026-07-27:** the delta check now runs on `pull_request` events only — `push` events build unconditionally (see "Push events always build" below), after the delta silently false-skipped a real `main`-push image build (`panoscope-image` on the #1807 merge: the changed-file list plainly contained `crates/panoscope-core/**` + `ops/panoscope/**`, yet the job emitted `relevant=false`; unreproducible from the logged inputs). |
| Related | [ci-results-follow-inputs.md](ci-results-follow-inputs.md) (the general rule: a check's result follows its inputs), [ADR 0708](../adr/0708-build-zenoh-from-source.md), [ADR 0858](../adr/0858-zenoh-image-upstream-pivot.md) (drops two of these guards once we pivot off the custom zenoh build), F-122 |

## What the guard does

Every image-building workflow (`zenoh-base-image`, `zenoh-image`, `goat-zenoh-portal-image`, `synth-vendor-image`, `panoscope-image`, `brief-image`, `ory-consent-image`) starts with a thin `detect-relevant-changes` job that runs a small shell script computing `git diff --name-only <parent> <head>` against the **latest push's diff** (`head.sha^..head.sha`) on `pull_request` syncs. (As of the 2026-07-27 amendment, `push` events — branch + tags — skip this diff and build unconditionally; see "Push events always build" below.) If a PR sync didn't touch any of the workflow's relevant paths, every other job in the workflow skips via `if: needs.detect-relevant-changes.outputs.relevant == 'true' || github.event_name == 'workflow_dispatch'`.

The job uses raw `git diff` (not `dorny/paths-filter@v3`) because dorny v3 **ignores** the `base` / `ref` inputs on `pull_request` events — it always falls back to GitHub's listFiles API, which returns the PR's full diff vs base. That is exactly the semantic we are trying to escape. We tried dorny first; the runs failed with:

```
##[warning]'ref' input parameter is ignored when 'base' is set to HEAD
##[warning]'base' input parameter is ignored when action is triggered by pull request event
##[error]Resource not accessible by integration
```

(`Resource not accessible` is the secondary symptom — the API path needs `pull-requests: read` permission. But fixing that just gives us the wrong-semantic full-PR-diff. The fundamental fix is to bypass dorny on PR events.)

## Why we need it

GitHub's native `paths:` trigger filter on `pull_request` events evaluates against the **PR's full diff vs base**, not against the latest push's delta. Consequences:

- **Merge commits re-fire image builds.** Merging `main` into a PR brings unrelated files into the PR's diff. If any of them happen to fall under a path-filtered workflow's pattern, that workflow re-runs — even when the merged content is identical to what just merged to `main` (where the workflow already ran and passed).
- **Rebases re-fire image builds.** Same shape — new commit SHAs are evaluated against the PR's whole diff, every image build a commit anywhere in the PR ever touched gets re-fired.
- **Real-world cost.** During the 2026-05-17 close-out of PR #137 we observed ~4-5 PR syncs each re-firing `zenoh-base-image` (~50 min cold per arch, ~100 min runner-time) plus the other image workflows. ~3 runner-hours per redundant sync; 9-30 hours per active PR's lifetime; dozens of hours per week across the repo.

The guard cuts that to ~5 seconds per redundant fire — the detect job spins up, runs `git diff` (or dorny's equivalent), exits with `relevant=false`, and every downstream job skips.

## What does NOT change

- The workflows' top-level `paths:` triggers stay in place. GitHub still won't *start* the workflow unless the path-filter at the trigger level matches.
- For relevant-path changes (real Dockerfile edits, real cargo dep bumps), the detect job emits `relevant=true` and the build runs as before.
- `workflow_dispatch` always runs (operator override).
- Push events to `main` (and release-tag pushes) **always build** — see below.

## Push events always build (2026-07-27 amendment)

The delta check now short-circuits to `relevant=true` on any `push` event and only runs its `git diff` logic on `pull_request` syncs. Two reasons this is strictly correct:

- **The native `on.push.paths` trigger already filters push events.** The workflow only *starts* on a push when a relevant path changed — so on push there is nothing left for the delta job to filter; it can only ever agree ("build") or wrongly disagree ("skip"). There is no upside, only the false-skip risk.
- **A false-skip on push loses a real deployable image; on PR it costs nothing.** PR builds never publish (`push-by-digest` is gated on `github.event_name != 'pull_request'`), so the delta dedup's whole value — and its only safe blast radius — is the PR-sync case it was written for. On push→main the same code silently dropped the `panoscope-image` build for #1807 (root cause never reproduced from the logged inputs: the changed-file list contained the panoscope paths and the identical grep matches locally), leaving no B1 image until an operator ran `workflow_dispatch`.

So `push` (branch + tags) is now unconditional-build; the delta logic is retained verbatim for `pull_request` events, where it still cuts the merge-commit/rebase re-fire cost described above.

## Trade-offs

- **Adds one runner allocation per workflow.** The detect job runs on `ubuntu-latest` for ~5-10s. Cost: negligible vs the saved build time.
- **First commit on a fresh PR branch has no HEAD~1.** If `git cat-file -e $BASE_SHA` fails and `git fetch --depth=1 origin $BASE_SHA` also fails, the script fails open (`relevant=true`). Same for branch-push events where `github.event.before` is the all-zeros SHA. No regression vs the pre-guard behavior.
- **Force-pushed rebases re-fire** if the rebased commit's diff still touches relevant paths. This is correct — rebase replaces commits, the new commits genuinely need to be re-validated. The guard catches merge commits + path-irrelevant amendments, not rebase-rewrites.

## Pattern

Every image-building workflow gets this block at the top of `jobs:`:

```yaml
jobs:
  detect-relevant-changes:
    runs-on: ubuntu-latest
    permissions:
      contents: read
    outputs:
      relevant: ${{ steps.filter.outputs.relevant }}
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 2
          ref: ${{ github.event.pull_request.head.sha || github.sha }}
      - id: filter
        env:
          BASE_SHA: ${{ github.event_name == 'pull_request' && format('{0}^', github.event.pull_request.head.sha) || github.event.before }}
          HEAD_SHA: ${{ github.event.pull_request.head.sha || github.sha }}
        run: |
          set -euo pipefail
          if [ -z "$BASE_SHA" ] || [ "$BASE_SHA" = "0000000000000000000000000000000000000000" ]; then
            echo "relevant=true" >> "$GITHUB_OUTPUT"
            exit 0
          fi
          if ! git cat-file -e "$BASE_SHA" 2>/dev/null; then
            git fetch --depth=1 origin "$BASE_SHA" 2>/dev/null || {
              echo "relevant=true" >> "$GITHUB_OUTPUT"
              exit 0
            }
          fi
          CHANGED=$(git diff --name-only "$BASE_SHA" "$HEAD_SHA")
          if echo "$CHANGED" | grep -qE '<workflow-specific regex>'; then
            echo "relevant=true" >> "$GITHUB_OUTPUT"
          else
            echo "relevant=false" >> "$GITHUB_OUTPUT"
          fi
```

Notes on the shape:
- Checkout uses `ref: ${{ github.event.pull_request.head.sha || github.sha }}` because PR events default to a checkout of the `refs/pull/N/merge` *merge commit* (PR-head merged with base). For our diff we want the PR's actual head, whose `HEAD^` is the previous PR-branch tip.
- `fetch-depth: 2` is enough for the common case (HEAD + parent). If `$BASE_SHA` isn't in the local pack (rare — covered branch movements, force-pushes), the script does a single-commit `git fetch --depth=1 origin $BASE_SHA` to materialize it.
- The grep regex is workflow-specific (one entry per path the workflow's trigger-level `paths:` filter lists). Per-workflow values live in the workflow file itself; this doc shows the shape.
- Fail-open at three points (no parent, can't fetch parent, etc.) keeps the guard from blocking a real build if something unexpected happens. Worst case = pre-guard behavior.

And every other job in the workflow adds:

```yaml
    needs: detect-relevant-changes
    if: needs.detect-relevant-changes.outputs.relevant == 'true' || github.event_name == 'workflow_dispatch'
```

The `|| workflow_dispatch` branch lets operators always force a run via `gh workflow run`.

## Future cleanup

`zenoh-base-image.yml` and `zenoh-image.yml` were deleted in the ADR 0858 step 5 change-set (2026-05-18). The substrate now pulls upstream `eclipse/zenoh:1.9.0` directly + bind-mounts the upstream-published rocksdb backend `.so`. Five image workflows remain.

If we end up with > 10 image workflows, factor `detect-relevant-changes` into a reusable workflow at `.github/workflows/_path-relevance.yml` to deduplicate the boilerplate. Five workflows is below the bar; inline copy is more readable.

## goatbuilder-pr: a pull request of documents only (2026-11-14)

**Built.** `goatbuilder-pr.yml` runs on any change under `docs/`, because `crates/goatbuilder/tests/doc_configs.rs` validates every whole `goatbuilder.yaml` example in any document. For a pull request that changes nothing else, that test is the only gate there that reads the change, so the workflow now runs only it.

- **Which change counts.** A first job, `scope`, reads the pull request's whole diff against the base it merges into (`git diff --name-status HEAD^1 HEAD` on the merge commit). This is not the latest push's delta that the image guard above reads: the question here is what the pull request changes, not what one push changed. `ops/ci/goatbuilder-pr-scope.sh` answers `docs-only` only when every changed path is an added or edited document under `docs/`, or a `*.md` file outside `crates/`, `ops/`, `tests/` and `.github/`. It answers `full` for anything else:
  - a path under `docs/tutorials/`: the tutorials' rehearsals run as written, and `tests/goatlite_budget.rs` reads `00-goatlite.md`;
  - a path under `docs/reference/goatbuilder/`: the command reference is regenerated and compared;
  - a document deleted, renamed, copied or changed in type, because an error or phase catalog entry may name its path;
  - an empty diff.

  Its `--self-test` runs first in the same step.
- **The required checks still report.** `check + clippy + test` runs for every pull request. Each of its steps is conditioned on the scope, so for documents only it restores the cargo caches (restore only, never saving a partial build under the full run's key) and runs `cargo test -p goatbuilder --test doc_configs`. `macOS test (unit, golden, flows)` is skipped by its job-level `if`, which GitHub counts as passed for a required check. Both jobs use `!cancelled()`, and anything other than exactly `docs-only` means every gate. So a `scope` job that fails or prints nothing runs everything; it never skips a check.
- **CodeQL is not part of this.** `codeql.yml` already starts only on Go, Python, JavaScript and TypeScript paths. The `Analyze (go)`, `Analyze (python)` and `Analyze (javascript-typescript)` runs a documents-only pull request shows come from GitHub's managed Code Quality scan (workflow name `CodeQL - Code Quality`), which no file in this repository configures. Only a repository administrator can turn it off or filter it.
- **Measured.** Before: on #681 (an ADR and `INDEX.md`), each of the two `goatbuilder-pr` jobs took 5.2 minutes. After: not measured yet; the first documents-only pull request that runs this workflow measures it.
