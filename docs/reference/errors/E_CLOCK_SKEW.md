# E_CLOCK_SKEW: the device clock is wrong

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `E_CLOCK_SKEW` is printed by `goat`. The same text offline: `goat explain E_CLOCK_SKEW` or `goatbuilder explain E_CLOCK_SKEW`.

## What it means

the device clock differs from the management server's authenticated HTTPS Date by more than 60 seconds, so certificates cannot be judged valid.

## Why it happens

- the clock drifted while the device was off or offline

## What to do

One line per step or alternative:

```text
synchronise the clock (chrony/ntp), then: goat doctor
```

## Read more

- [`docs/design/time-hygiene-trusted-time.md`](../../../docs/design/time-hygiene-trusted-time.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

## Exit code

`goat` exits with 6.
