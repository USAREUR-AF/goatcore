# E_PORTAL_FORBIDDEN: the portal refused this identity

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `E_PORTAL_FORBIDDEN` is printed by `goat`. The same text offline: `goat explain E_PORTAL_FORBIDDEN` or `goatbuilder explain E_PORTAL_FORBIDDEN`.

## What it means

HTTP 403 to the probe. The token was accepted; the identity lacks the access.

## Why it happens

- the active context is a different identity than you meant
- the grant for this surface was never made

## What to do

One line per step or alternative:

```text
goat context show
ask the goatnet operator for the access grant; rotating the token will not grant it
```

## Read more

- [`docs/design/goat-cli-error-codes.md`](../../../docs/design/goat-cli-error-codes.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

## Exit code

`goat` exits with 4.
