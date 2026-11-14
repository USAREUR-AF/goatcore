# E_BUNDLE_INVALID: the bundle did not verify

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `E_BUNDLE_INVALID` is printed by `goat`. The same text offline: `goat explain E_BUNDLE_INVALID` or `goatbuilder explain E_BUNDLE_INVALID`.

## What it means

at `goat profile init` the bundle's structure, signature or expiry failed. The help line says which of the three.

## Why it happens

- a truncated or wrong download (malformed CBOR)
- a signing CA this machine does not trust
- an expired bundle

## What to do

One line per step or alternative:

```text
goat ca ls (what this machine trusts)
goat ca add <root.pem> (when the CA is the problem)
ask the operator for a re-issued bundle when it has expired
```

## Read more

- [`docs/design/goat-cli-error-codes.md`](../../../docs/design/goat-cli-error-codes.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

## Exit code

`goat` exits with 3.
