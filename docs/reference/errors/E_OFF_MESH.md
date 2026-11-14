# E_OFF_MESH: not connected to the mesh

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `E_OFF_MESH` is printed by `goat`. The same text offline: `goat explain E_OFF_MESH` or `goatbuilder explain E_OFF_MESH`.

## What it means

the local mesh client reports no connected tunnel, so nothing on the goatnet is reachable.

## Why it happens

- the mesh client is stopped or logged out
- this device was never enrolled
- the device is on a network that blocks the mesh

## What to do

One line per step or alternative:

```text
netbird status -d
goat heal && goat doctor
```

## Read more

- [`docs/design/goat-cli-error-codes.md`](../../../docs/design/goat-cli-error-codes.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

## Exit code

`goat` exits with 5.
