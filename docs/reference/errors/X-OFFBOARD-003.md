# X-OFFBOARD-003: the removal was not confirmed, so nothing was removed

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-OFFBOARD-003` is an error of `offboard.sh` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-OFFBOARD-003` or `goatbuilder explain X-OFFBOARD-003`.

## What it means

before the first removal the driver names the person and the counts and takes the handle typed back. The run had no terminal to ask on, or the handle typed did not match.

## Why it happens

- the driver ran from a script, a pipe or CI without --yes
- the handle typed at the question was not the person's handle

## What to do

One line per step or alternative:

```text
run it again in a terminal and type the handle shown
add --yes (when the run has no terminal and the plan was already read)
```

## Read more

- [`docs/design/offboarding-one-driver.md`](../../../docs/design/offboarding-one-driver.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
