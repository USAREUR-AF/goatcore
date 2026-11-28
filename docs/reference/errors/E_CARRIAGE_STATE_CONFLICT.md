# E_CARRIAGE_STATE_CONFLICT: the carriage rule is not in a state that allows this change

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `E_CARRIAGE_STATE_CONFLICT` is printed by `goat`. The same text offline: `goat explain E_CARRIAGE_STATE_CONFLICT` or `goatbuilder explain E_CARRIAGE_STATE_CONFLICT`.

## What it means

The portal or pod answered HTTP 409: the rule or local rule is not in the state the change needs, or changed since it was read (the pod's `conflict`). Reactivating a rule that is not archived, reactivating one already reactivated, accepting a rule that is not proposed, ratifying or letting lapse a local rule already decided, a pod registered for another org, a source whose identity was revoked (goat's revocations win), or a pod ledger that conflicts with the portal's (ADR 1251). Nothing was changed.

## Why it happens

- the rule changed state since it was last listed
- another person already acted on it
- the source's identity was revoked

## What to do

One line per step or alternative:

```text
goat carriage ls --state archived (the rule's current state)
goat carriage pending --state all (local rules and their decisions)
goat carriage pod ls (which org each pod is registered for)
```

## Read more

- [`docs/design/pod-carriage-and-provenance.md`](../../../docs/design/pod-carriage-and-provenance.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

## Exit code

`goat` exits with 1.
