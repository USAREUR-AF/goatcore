# E_CARRIAGE_WINDOW_PASSED: the rule expired too long ago to reactivate

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `E_CARRIAGE_WINDOW_PASSED` is printed by `goat`. The same text offline: `goat explain E_CARRIAGE_WINDOW_PASSED` or `goatbuilder explain E_CARRIAGE_WINDOW_PASSED`.

## What it means

`goat carriage reactivate` or `goat pod rules reactivate` named a rule that expired longer ago than the goatnet's reactivation window (six months by default). After the window a rule is approved like a new one (ADR 1251). Nothing was changed.

## Why it happens

- the rule ended more than the reactivation window ago

## What to do

One line per step or alternative:

```text
goat carriage allow <source> --pod <pod> --until <time> --reason "…" (a new rule, granted and accepted again)
```

## Read more

- [`docs/design/pod-carriage-and-provenance.md`](../../../docs/design/pod-carriage-and-provenance.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

## Exit code

`goat` exits with 1.
