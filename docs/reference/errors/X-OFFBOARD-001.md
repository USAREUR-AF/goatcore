# X-OFFBOARD-001: an offboarding needs the reason the person left

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-OFFBOARD-001` is an error of `offboard.sh` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-OFFBOARD-001` or `goatbuilder explain X-OFFBOARD-001`.

## What it means

ops/scripts/offboard.sh --apply writes a departure record before it removes anything, and the record needs a reason class. With --reason other it also needs one line of text.

## Why it happens

- --apply was given without --reason
- --reason other was given without --reason-text

## What to do

One line per step or alternative:

```text
ops/scripts/offboard.sh <handle> --all-goatnets --apply --reason new-position|left-service --confirmed-by <handle>
ops/scripts/offboard.sh <handle> --all-goatnets --apply --reason other --reason-text '<why>' --confirmed-by <handle>
```

## Read more

- [`docs/design/offboarding-one-driver.md`](../../../docs/design/offboarding-one-driver.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
