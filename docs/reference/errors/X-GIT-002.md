# X-GIT-002: the branch cannot fast-forward

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-GIT-002` is an error of `git` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-GIT-002` or `goatbuilder explain X-GIT-002`.

## What it means

local and remote histories diverged, so a --ff-only pull refuses.

## Why it happens

- local commits on main

## What to do

One line per step or alternative:

```text
git log --oneline origin/main..HEAD (what is local); move it to a branch, then git pull --ff-only
```

## Read more

- [`CONTRIBUTING.md`](../../../CONTRIBUTING.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
