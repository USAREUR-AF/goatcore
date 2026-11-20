# E_CARRIAGE_END_INVALID: the carriage rule's end date was refused

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `E_CARRIAGE_END_INVALID` is printed by `goat`. The same text offline: `goat explain E_CARRIAGE_END_INVALID` or `goatbuilder explain E_CARRIAGE_END_INVALID`.

## What it means

The portal or pod refused a carriage rule's end date: it is already past by the server's clock (`carriage_until_past`), it was set on an org rule, which has no end date (`carriage_org_rule_no_end`), or an extend or reactivate named an exclusion, a ratified local revoke that has no end date and is lifted only by revoking it (`carriage_exclusion_no_end`) (ADR 1251). Nothing was changed.

## Why it happens

- --until names a time already past, or this machine's clock is wrong
- an end date was asked for on an org rule
- extend or reactivate was run on an exclusion

## What to do

One line per step or alternative:

```text
pass a later --until (an RFC 3339 time, or a length such as 72h or 30d)
goat carriage opt-out <source> (to stop an org's pods carrying one source)
goat carriage revoke <exclusion> (the source's custodian lifts an exclusion)
```

## Read more

- [`docs/design/pod-carriage-and-provenance.md`](../../../docs/design/pod-carriage-and-provenance.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

## Exit code

`goat` exits with 1.
