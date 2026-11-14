# X-HTTP-001: the service rejected the credential (401)

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-HTTP-001` is an error of `http` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-HTTP-001` or `goatbuilder explain X-HTTP-001`.

## What it means

the request reached the service and the token or cookie was not accepted.

## Why it happens

- an expired or wrong token

## What to do

One line per step or alternative:

```text
check which token the command is using; for goat, E_TOKEN_INVALID says how to rotate it
```

## Read more

- [`docs/design/goat-cli-error-codes.md`](../../../docs/design/goat-cli-error-codes.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
