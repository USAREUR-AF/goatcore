# X-SMOKE-001: the live rows' browser toolchain is not on this machine

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-SMOKE-001` is an error of `onboard-intake-smoke` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-SMOKE-001` or `goatbuilder explain X-SMOKE-001`.

## What it means

the live acceptance rows (intake-form-browser-submit, review-gate-accepts-open-field-invite) run ops/scripts/onboard-intake-smoke.sh from the deployer's laptop, and it exits 2 before touching the goatnet when ops/goat-id-console/web/e2e has no node_modules. The release did not fail on the goatnet; it failed here.

## Why it happens

- a laptop that never installed the console's Playwright suite

## What to do

One line per step or alternative:

```text
(cd ops/goat-id-console/web/e2e && npm ci && npx playwright install chromium)
goatbuilder doctor --browser-tests (the tool:live-rows row fails until this machine is ready)
```

## Read more

- [`docs/design/release-candidate-validation.md`](../../../docs/design/release-candidate-validation.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
