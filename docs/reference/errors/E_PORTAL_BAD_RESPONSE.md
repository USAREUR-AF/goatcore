# E_PORTAL_BAD_RESPONSE: the portal answered in a shape this goat build does not understand

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `E_PORTAL_BAD_RESPONSE` is printed by `goat`. The same text offline: `goat explain E_PORTAL_BAD_RESPONSE` or `goatbuilder explain E_PORTAL_BAD_RESPONSE`.

## What it means

the portal returned 2xx but the body did not deserialize. The portal is healthy; the CLI and the portal are at different versions.

## Why it happens

- goat is behind the portal
- the portal deployed ahead of the CLI

## What to do

One line per step or alternative:

```text
goat update (or re-install from get.<site>)
goat --version, and report the endpoint if already current
```

## Read more

- [`docs/design/goat-cli-error-codes.md`](../../../docs/design/goat-cli-error-codes.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

## Exit code

`goat` exits with 4.
