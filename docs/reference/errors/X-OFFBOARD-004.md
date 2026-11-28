# X-OFFBOARD-004: the departure record gives a different reason or cannot be used

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-OFFBOARD-004` is an error of `offboard.sh` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-OFFBOARD-004` or `goatbuilder explain X-OFFBOARD-004`.

## What it means

an open departure record for this person already exists with another reason class, or the record file cannot be read or written. The driver stops before the first removal.

## Why it happens

- an earlier run recorded a different --reason for the same departure
- this is a new departure of a person who returned, and --effective was not given
- ops/departures/ is not writable, or a record file is not valid YAML

## What to do

One line per step or alternative:

```text
python3 ops/scripts/offboard_departure.py show <handle> (the recorded reason and date)
pass the recorded --reason, or --effective YYYY-MM-DD for a new departure
repair or restore the record file named in the message
```

## Read more

- [`docs/design/offboarding-one-driver.md`](../../../docs/design/offboarding-one-driver.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
