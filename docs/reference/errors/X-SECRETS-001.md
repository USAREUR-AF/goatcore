# X-SECRETS-001: the declared secrets store is unknown

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-SECRETS-001` is an error of `with-secrets` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-SECRETS-001` or `goatbuilder explain X-SECRETS-001`.

## What it means

GOAT_SECRET_STORE or `secrets: store:` in goatbuilder.yaml names a store goat does not have, so no command can get its secrets.

## Why it happens

- a typo in GOAT_SECRET_STORE or in goatbuilder.yaml
- a custom store whose ops/secrets/&lt;name>/run is not on this checkout

## What to do

One line per step or alternative:

```text
goatbuilder secrets store (prints the store in force and where it came from)
use one of: 1password, openbao, custom, sops, git-crypt, env-file, env
```

## Read more

- [`docs/design/pluggable-dependencies.md`](../../../docs/design/pluggable-dependencies.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
