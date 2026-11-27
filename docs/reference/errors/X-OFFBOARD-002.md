# X-OFFBOARD-002: an offboarding needs the person who confirmed the departure

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-OFFBOARD-002` is an error of `offboard.sh` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-OFFBOARD-002` or `goatbuilder explain X-OFFBOARD-002`.

## What it means

the departure record names who confirmed that the person left. The handle is missing, or the checkout declares goat_offboard_confirmers and the handle is not one of them.

## Why it happens

- --apply was given without --confirmed-by
- the handle given is not in goat_offboard_confirmers in ops/ansible/group_vars/all.yml

## What to do

One line per step or alternative:

```text
add --confirmed-by <handle> (kebab-case handle of the person who confirmed it)
grep goat_offboard_confirmers ops/ansible/group_vars/all.yml (the handles this checkout accepts)
```

## Read more

- [`docs/design/offboarding-one-driver.md`](../../../docs/design/offboarding-one-driver.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
