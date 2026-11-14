# X-SECRETS-003: refusing to write secret values to a file git would commit

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-SECRETS-003` is an error of `goatbuilder` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-SECRETS-003` or `goatbuilder explain X-SECRETS-003`.

## What it means

`secrets init` writes plaintext values, so it refuses a path that is not gitignored.

## Why it happens

- --file names a path inside the repository that .gitignore does not cover

## What to do

One line per step or alternative:

```text
add the path to .gitignore, or use the default .envrc.local.d/secrets.env
```

## Read more

- [`docs/design/pluggable-dependencies.md`](../../../docs/design/pluggable-dependencies.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
