# X-FED-006: apply --confirm needs --out

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-FED-006` is an error of `goat-federation` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-FED-006` or `goatbuilder explain X-FED-006`.

## What it means

federate apply writes only with --confirm, and only under the directory --out names: it will not choose where to write. --confirm was given without --out, so nothing was written. Without --confirm, apply prints what it would write.

## Why it happens

- --out was left out of a command that writes

## What to do

One line per step or alternative:

```text
goatbuilder federate apply <agreement> --confirm --out <dir>
or leave out --confirm to see what apply would write
```

## Read more

- [`docs/design/goatbuilder-conformance.md`](../../../docs/design/goatbuilder-conformance.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
