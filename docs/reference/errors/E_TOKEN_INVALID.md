# E_TOKEN_INVALID: the portal rejected the token

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `E_TOKEN_INVALID` is printed by `goat`. The same text offline: `goat explain E_TOKEN_INVALID` or `goatbuilder explain E_TOKEN_INVALID`.

## What it means

the portal accepted the connection and refused the bearer token (HTTP 401): the credential itself is expired or misissued.

## Why it happens

- the token in ~/.goat/contexts/&lt;name>/portal.token has expired
- the token belongs to another goatnet

## What to do

One line per step or alternative:

```text
goat profile rotate <new-bundle.cbor> (after re-running onboarding)
```

## Read more

- [`docs/design/goat-cli-error-codes.md`](../../../docs/design/goat-cli-error-codes.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

## Exit code

`goat` exits with 4.
