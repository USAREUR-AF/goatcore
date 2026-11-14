# X-SECRETS-004: bao:// references need the OpenBao injector

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-SECRETS-004` is an error of `with-secrets` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-SECRETS-004` or `goatbuilder explain X-SECRETS-004`.

## What it means

the environment holds bao:// references, and ops/secrets/openbao/run (the OpenBao store's injector, ADR 1203) is not on this checkout.

## Why it happens

- a .refs file holds bao:// lines on a checkout that predates the OpenBao store

## What to do

One line per step or alternative:

```text
git pull --ff-only origin main (the injector lands with the OpenBao store)
or remove the bao:// lines and use op:// references meanwhile
```

## Read more

- [`docs/design/pluggable-dependencies.md`](../../../docs/design/pluggable-dependencies.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
