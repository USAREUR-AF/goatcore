# X-SECRETS-002: the env-file store has no file

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-SECRETS-002` is an error of `with-secrets` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-SECRETS-002` or `goatbuilder explain X-SECRETS-002`.

## What it means

the declared store is env-file, and its file of values does not exist on this laptop.

## Why it happens

- the quickstart's env file was never generated
- a goatlite whose phase 1 has not run yet (phase 1 renders the goatnet, then creates this file)
- GOAT_SECRET_ENV_FILE points somewhere else

## What to do

One line per step or alternative:

```text
on a goatlite: goatbuilder standup <name> --to 1 (renders the goatnet, then creates the file)
on any other goatnet: goatbuilder secrets init --env <goatnet> (strong random values, 0600, gitignored)
```

## Read more

- [`docs/design/pluggable-dependencies.md`](../../../docs/design/pluggable-dependencies.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
