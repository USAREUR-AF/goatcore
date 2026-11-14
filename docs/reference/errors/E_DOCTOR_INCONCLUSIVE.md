# E_DOCTOR_INCONCLUSIVE: doctor saw a failure it cannot narrow

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `E_DOCTOR_INCONCLUSIVE` is printed by `goat`. The same text offline: `goat explain E_DOCTOR_INCONCLUSIVE` or `goatbuilder explain E_DOCTOR_INCONCLUSIVE`.

## What it means

a local TLS or configuration failure, or a session rejection, without enough evidence for one cause.

## Why it happens

- router-side admission refused this identity
- a local TLS setup problem the probe cannot distinguish from the router's

## What to do

One line per step or alternative:

```text
follow the hint on the failing doctor step
if the local certificate is valid, send the step output to the goatnet operator
```

## Read more

- [`docs/design/goat-cli-error-codes.md`](../../../docs/design/goat-cli-error-codes.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

## Exit code

`goat` exits with 1.
