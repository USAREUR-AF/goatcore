# Fieldkit

Small offline pages that let an operator who is not a developer do one job the `goat` CLI does. Each tool is a fixed list
of actions over the same API the CLI calls, with no logic, state or authority of its own. Design:
[`docs/design/fieldkit-and-bleating-edge.md`](../../docs/design/fieldkit-and-bleating-edge.md).

| Path | What | Status |
|---|---|---|
| [`bleating-edge/`](bleating-edge/README.md) | Design system: tokens, eight components, one CSS file, one script, a starter template | built |
| [`gatepost/`](gatepost/README.md) | Fieldkit Gatepost: who may use this pod, on site with no backhaul | built; served by `ops/goat-pod-api` |
| `inline.py` | Copies Bleating Edge into each tool; `--check` in CI | built |
| `ops/goat-zenoh-portal/web/e2e/fieldkit/` | Playwright checks of each tool against its local mock, using the portal's Playwright install (config `playwright.fieldkit.config.ts`) | built |

## Checks

| Command | Checks |
|---|---|
| `python3 ops/ci/check-fieldkit-scope.py` | Every action equals an allowed `goat` verb (`ops/ci/fieldkit-cli-allowlist.txt`), at most eight, no other network call |
| `python3 ops/ci/check-fieldkit-scope.py --self-test` | The guard refuses what it should |
| `python3 ops/fieldkit/inline.py --check` | Each tool carries the current Bleating Edge |
| `python3 ops/fieldkit/gatepost/test_mock_pod_api.py` | The mock enforces the pod's limits |
| `cd ops/goat-zenoh-portal/web/e2e && npm ci && npx playwright install chromium && npx playwright test -c playwright.fieldkit.config.ts` | Each tool in a browser against its mock |

CI: `.github/workflows/fieldkit.yml` runs all of these; `docs-guards.yml` also runs the scope guard.

## Adding a tool

A tool is added only for a real, recurring job. Copy `bleating-edge/starter.html` to `<tool>/<tool>.html`, declare each
action with `data-action`, `data-cli` and `data-api`, add one row per action to `ops/ci/fieldkit-cli-allowlist.txt`
(platform review), write a mock of its API and a Playwright spec under `ops/goat-zenoh-portal/web/e2e/fieldkit/`, and run `inline.py`.
