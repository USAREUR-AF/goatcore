# X-DIRENV-001: direnv exec was given an assignment, not a command

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-DIRENV-001` is an error of `direnv` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-DIRENV-001` or `goatbuilder explain X-DIRENV-001`.

## What it means

`direnv exec <dir> VAR=value cmd` looks VAR=value up on PATH as the command and fails; direnv exec takes a command.

## Why it happens

- an environment assignment placed where direnv exec expects the command

## What to do

One line per step or alternative:

```text
direnv exec . env VAR=value cmd
```

## Read more

- [`docs/operations/admin-operator-guide.md`](../../../docs/operations/admin-operator-guide.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
