# X-OFFBOARD-005: no such departure record or by-hand row

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-OFFBOARD-005` is an error of `offboard.sh` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-OFFBOARD-005` or `goatbuilder explain X-OFFBOARD-005`.

## What it means

offboard_departure.py was asked to show or close a record that is not in the records directory, or a by-hand row the record does not have.

## Why it happens

- the handle has no record under ops/departures/ in this checkout
- the row id is misspelled
- close was given without a valid --by handle

## What to do

One line per step or alternative:

```text
python3 ops/scripts/offboard_departure.py list (every record and its open rows)
python3 ops/scripts/offboard_departure.py show <handle> (the row ids)
```

## Read more

- [`docs/design/offboarding-one-driver.md`](../../../docs/design/offboarding-one-driver.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
