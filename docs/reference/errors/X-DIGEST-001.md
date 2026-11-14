# X-DIGEST-001: no tool could read the image's digest

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-DIGEST-001` is an error of `image-digest` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-DIGEST-001` or `goatbuilder explain X-DIGEST-001`.

## What it means

ops/scripts/image-digest tried crane, skopeo and docker in turn; none is installed, or each refused, and each tool's own error is printed beneath.

## Why it happens

- none of crane, skopeo or docker is installed
- the registry refused this laptop's credentials (each tool's error says which)
- the tag does not exist yet (its image build has not finished)

## What to do

One line per step or alternative:

```text
brew install crane
gh auth token | crane auth login ghcr.io -u <user> --password-stdin
```

## Read more

- [`docs/design/pluggable-dependencies.md`](../../../docs/design/pluggable-dependencies.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
