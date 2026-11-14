# E_MACHINE_UNRESOLVED: the machine named is not exactly one enrolled machine

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `E_MACHINE_UNRESOLVED` is printed by `goat`. The same text offline: `goat explain E_MACHINE_UNRESOLVED` or `goatbuilder explain E_MACHINE_UNRESOLVED`.

## What it means

a `goat machine` verb was given a handle or slot anchor that matches no enrolled machine this profile can see, or a handle that matches machines under more than one custodian. Nothing was changed.

## Why it happens

- a typo in the handle
- the machine is under another custodian's custody, which a non-operator profile cannot see
- the machine was revoked
- two custodians each enrolled a machine with the same handle

## What to do

One line per step or alternative:

```text
goat machine ls (shows each machine's handle, custodian and slot anchor)
pass the 32-hex slot anchor instead of the handle when the handle is ambiguous
```

## Read more

- [`docs/design/goat-cli-error-codes.md`](../../../docs/design/goat-cli-error-codes.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

## Exit code

`goat` exits with 1.
