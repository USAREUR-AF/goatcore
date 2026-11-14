# X-SECRETS-005: a git-crypt file is still encrypted

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-SECRETS-005` is an error of `with-secrets` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-SECRETS-005` or `goatbuilder explain X-SECRETS-005`.

## What it means

the declared store is git-crypt and its file has not been unlocked on this laptop, so its values cannot be read.

## Why it happens

- this clone was never unlocked

## What to do

One line per step or alternative:

```text
git-crypt unlock
```

## Read more

- [`docs/design/pluggable-dependencies.md`](../../../docs/design/pluggable-dependencies.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
