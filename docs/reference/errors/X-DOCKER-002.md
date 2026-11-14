# X-DOCKER-002: the image could not be pulled

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-DOCKER-002` is an error of `docker` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-DOCKER-002` or `goatbuilder explain X-DOCKER-002`.

## What it means

the registry answered that the manifest is unknown or the pull is unauthorised.

## Why it happens

- a tag that was never pushed, or a digest from the other GHCR org
- the host's registry login has expired

## What to do

One line per step or alternative:

```text
crane digest <repo.image_namespace>/<image>:<tag>
on the host: sudo docker pull <ref>
```

## Read more

- [`docs/design/base-images.md`](../../../docs/design/base-images.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
