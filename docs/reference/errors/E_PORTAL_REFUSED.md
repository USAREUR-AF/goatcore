# E_PORTAL_REFUSED: the portal refused the request

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `E_PORTAL_REFUSED` is printed by `goat`. The same text offline: `goat explain E_PORTAL_REFUSED` or `goatbuilder explain E_PORTAL_REFUSED`.

## What it means

the portal is up and answered a non-2xx status no command curates (a 400, 405, 410, 413 or 422) for the request path shown.

## Why it happens

- this goat build and the portal are at different API versions
- a value the command sent is one the portal does not accept

## What to do

One line per step or alternative:

```text
goat doctor (shows whether goat is behind the served version)
goat update
```

## Read more

- [`docs/design/goat-cli-error-codes.md`](../../../docs/design/goat-cli-error-codes.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

## Exit code

`goat` exits with 4.
