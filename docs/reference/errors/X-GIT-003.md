# X-GIT-003: the remote refused access

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-GIT-003` is an error of `git` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-GIT-003` or `goatbuilder explain X-GIT-003`.

## What it means

the push or fetch was refused: the SSH key is not authorised for the repository, or the remote does not exist.

## Why it happens

- the wrong GitHub account for this remote
- the key is not loaded

## What to do

One line per step or alternative:

```text
ssh -T git@github.com
gh auth status
```

## Read more

- [`CONTRIBUTING.md`](../../../CONTRIBUTING.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
