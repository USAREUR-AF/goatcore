# E_CARRIAGE_TOO_LONG: the rule's end date is missing or too far away

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `E_CARRIAGE_TOO_LONG` is printed by `goat`. The same text offline: `goat explain E_CARRIAGE_TOO_LONG` or `goatbuilder explain E_CARRIAGE_TOO_LONG`.

## What it means

A named, temporary or local carriage rule (`carriage_until_required`, `carriage_over_max_length`) needs an end date no later than the longest rule length: 90 days by default, a goatnet setting, or the pod's own limit for a local rule (ADR 1251). The portal or pod refused the change. Nothing was changed.

## Why it happens

- --until is later than the longest rule length
- the goatnet or the pod sets a shorter limit than 90 days

## What to do

One line per step or alternative:

```text
pass an earlier --until (an RFC 3339 time, or a length such as 72h or 30d)
goat carriage extend <rule> --until <time> (move the end date later, when the rule nears it)
```

## Read more

- [`docs/design/pod-carriage-and-provenance.md`](../../../docs/design/pod-carriage-and-provenance.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

## Exit code

`goat` exits with 1.
