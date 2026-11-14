# X-GIT-001: not a git repository

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-GIT-001` is an error of `git` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-GIT-001` or `goatbuilder explain X-GIT-001`.

## What it means

the working directory is not inside a checkout.

## Why it happens

- run from the wrong directory

## What to do

One line per step or alternative:

```text
cd /path/to/goat (or pass -C)
```

## Read more

- [`CONTRIBUTING.md`](../../../CONTRIBUTING.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
