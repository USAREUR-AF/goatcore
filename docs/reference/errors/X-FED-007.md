# X-FED-007: federate apply cannot tell which party it is

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-FED-007` is an error of `goat-federation` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-FED-007` or `goatbuilder explain X-FED-007`.

## What it means

federate apply renders one party's side of an agreement. Without --as it reads the party's sovereign id from this site's declaration, and there is no declaration to read here (no context directory, no declaration file, or one that does not parse). Nothing was rendered.

## Why it happens

- --as was left out on a machine that has no federation declaration (federate init writes it)
- the context directory named is not this site's

## What to do

One line per step or alternative:

```text
pass --as <sovereign_id>, the party this side is
or run federate init first, or name this site's context with --context-dir
```

## Read more

- [`docs/design/goatbuilder-conformance.md`](../../../docs/design/goatbuilder-conformance.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
