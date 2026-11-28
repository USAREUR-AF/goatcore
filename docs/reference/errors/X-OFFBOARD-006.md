# X-OFFBOARD-006: the external-accounts declaration cannot be used

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-OFFBOARD-006` is an error of `offboard.sh` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-OFFBOARD-006` or `goatbuilder explain X-OFFBOARD-006`.

## What it means

ops/offboard/accounts.yaml is not a valid goat-offboard-accounts/v1 document. The driver refuses the whole run rather than read part of the list.

## Why it happens

- an account has no id, a repeated id, or no type
- a manual account has no `what`
- credential_env holds something other than variable names
- match is not email, github or declared

## What to do

One line per step or alternative:

```text
compare the file with ops/offboard/accounts.yaml.example
ops/scripts/offboard.sh <handle> --goatnet <g> (plan mode lists every account once the file loads)
```

## Read more

- [`ops/offboard/README.md`](../../../ops/offboard/README.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
