# X-OFFBOARD-009: the person's handle in an external account is not declared

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-OFFBOARD-009` is an error of `offboard.sh` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-OFFBOARD-009` or `goatbuilder explain X-OFFBOARD-009`.

## What it means

an account that matches by declared handle needs to be told who the person is there. The driver never guesses from a name.

## Why it happens

- the person's manifest has no identity.accounts.&lt;id>
- --account-handle was malformed or names an account that is not declared

## What to do

One line per step or alternative:

```text
add identity.accounts.<id>: <handle> to the person's manifest
--account-handle <id>=<handle> (repeat for more than one)
--account-handle <id>=- (states that the person holds no account there)
```

## Read more

- [`docs/design/offboarding-one-driver.md`](../../../docs/design/offboarding-one-driver.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
