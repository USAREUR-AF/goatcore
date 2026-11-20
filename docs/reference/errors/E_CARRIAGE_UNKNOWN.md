# E_CARRIAGE_UNKNOWN: no such carriage rule, pod or live slot

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `E_CARRIAGE_UNKNOWN` is printed by `goat`. The same text offline: `goat explain E_CARRIAGE_UNKNOWN` or `goatbuilder explain E_CARRIAGE_UNKNOWN`.

## What it means

The portal answered HTTP 404 for a carriage change: the rule or local rule id does not exist (`not_found`), the pod is not registered for carriage (`pod_not_registered`), or a source or pod slot is not a live enrolled identity (`source_not_live`, `pod_slot_not_live`) (ADR 1251). Nothing was changed.

## Why it happens

- a typo in the rule id or slot
- the pod was never registered with `goat carriage pod add`
- the slot was never enrolled, or was retired

## What to do

One line per step or alternative:

```text
goat carriage ls (rule ids)
goat carriage pod ls (registered pods)
goat carriage pod add <pod-slot> --org <org> (an operator registers the pod)
goat machine ls (enrolled slots)
```

## Read more

- [`docs/design/pod-carriage-and-provenance.md`](../../../docs/design/pod-carriage-and-provenance.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

## Exit code

`goat` exits with 1.
