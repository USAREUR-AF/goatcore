# X-FED-003: --now is not a YYYY-MM-DD date

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-FED-003` is an error of `goat-federation` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-FED-003` or `goatbuilder explain X-FED-003`.

## What it means

federate compose evaluates the two packages on the date --now gives, a calendar date in UTC written YYYY-MM-DD. The value given is not one. Nothing was composed. Without --now, compose uses today's date in UTC.

## Why it happens

- the date has another shape (15/11/2026, 2026-11-15T10:00Z) or is not a real date

## What to do

One line per step or alternative:

```text
pass --now as YYYY-MM-DD, for example --now 2026-11-15, or leave it out to use today in UTC
```

## Read more

- [`docs/design/goatbuilder-conformance.md`](../../../docs/design/goatbuilder-conformance.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
