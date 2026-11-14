# E_PROFILE_MISSING: no profile for this context

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `E_PROFILE_MISSING` is printed by `goat`. The same text offline: `goat explain E_PROFILE_MISSING` or `goatbuilder explain E_PROFILE_MISSING`.

## What it means

goat was run before `goat profile init`, or the context named has no profile.toml, certificate or key.

## Why it happens

- the bundle was never installed on this machine
- the bundle was installed into a different context than the one in use

## What to do

One line per step or alternative:

```text
goat profile init <bundle.cbor>
goat --context <name> … (when the bundle was installed under another context)
```

## Read more

- [`docs/design/goat-cli-error-codes.md`](../../../docs/design/goat-cli-error-codes.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

## Exit code

`goat` exits with 3.
