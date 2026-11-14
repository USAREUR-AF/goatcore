# E_MESH_PEER_UNRESOLVED: the enrolled machine has no single mesh peer

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `E_MESH_PEER_UNRESOLVED` is printed by `goat`. The same text offline: `goat explain E_MESH_PEER_UNRESOLVED` or `goatbuilder explain E_MESH_PEER_UNRESOLVED`.

## What it means

the machine is enrolled, but no NetBird peer carries its name, or several do. A machine's peer is named `machine-<handle>-<host>` by its setup key, or `<handle>` when renamed by hand. Nothing was changed.

## Why it happens

- the machine never joined the mesh with its setup key
- the peer was renamed off its handle in the NetBird dashboard
- the machine joined twice, leaving a stale peer beside the live one

## What to do

One line per step or alternative:

```text
goat admin peer ls (find the machine's peer and its name)
goat machine mesh-key <machine> (when it never joined)
rename the peer back to `<handle>`, or remove the stale peer with goat admin peer cut <peer-id>
```

## Read more

- [`docs/design/goat-cli-error-codes.md`](../../../docs/design/goat-cli-error-codes.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

## Exit code

`goat` exits with 1.
