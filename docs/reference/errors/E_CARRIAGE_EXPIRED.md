# E_CARRIAGE_EXPIRED: the carriage rule has expired and is archived

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `E_CARRIAGE_EXPIRED` is printed by `goat`. The same text offline: `goat explain E_CARRIAGE_EXPIRED` or `goatbuilder explain E_CARRIAGE_EXPIRED`.

## What it means

`goat carriage extend` or `goat pod rules extend` named a rule that has passed its end date or was revoked (`carriage_rule_expired`, `carriage_rule_archived`, `local_rule_expired`). An expired rule is archived, not deleted, and is brought back by reactivating it, not by extending it (ADR 1251). Nothing was changed.

## Why it happens

- the rule reached its end date before it was extended
- the rule was revoked, which archives it

## What to do

One line per step or alternative:

```text
goat carriage reactivate <rule> (inside the goatnet's reactivation window)
goat pod rules reactivate <rule> --until <time> --reason "…" (on a cut-off pod)
goat carriage ls --state archived (shows the rule and when it ended)
```

## Read more

- [`docs/design/pod-carriage-and-provenance.md`](../../../docs/design/pod-carriage-and-provenance.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

## Exit code

`goat` exits with 1.
