# E_SLOT_SCOPE: the request names a slot this profile does not own

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `E_SLOT_SCOPE` is printed by `goat`. The same text offline: `goat explain E_SLOT_SCOPE` or `goatbuilder explain E_SLOT_SCOPE`.

## What it means

the portal refused a write under a slot (HTTP 403 scope mismatch): the topic key's first segment is not this profile's slot, and this profile is not that machine's recorded custodian. Nothing changed.

## Why it happens

- the topic key starts with a machine's slot and the command ran in the person's or org's own context
- the slot belongs to someone else

## What to do

One line per step or alternative:

```text
goat profile show (the publish anchor is this profile's slot)
goat profile init <machine .cbor> --context <machine>, then goat --context <machine> topic create …
ask the slot's owner or custodian; an operator key is not the fix
```

## Read more

- [`docs/adr/1096-delegated-machine-enrollment.md`](../../../docs/adr/1096-delegated-machine-enrollment.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

## Exit code

`goat` exits with 4.
