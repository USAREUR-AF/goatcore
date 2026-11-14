# E_PORTAL_UNREACHABLE: the portal API did not answer

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `E_PORTAL_UNREACHABLE` is printed by `goat`. The same text offline: `goat explain E_PORTAL_UNREACHABLE` or `goatbuilder explain E_PORTAL_UNREACHABLE`.

## What it means

the portal was not reachable for network reasons (DNS, TCP or TLS), not an authorization failure.

## Why it happens

- the portal host is not reachable over the mesh from this device
- mesh DNS is not resolving the portal name

## What to do

One line per step or alternative:

```text
goat doctor --all
check from another peer: if it answers there, the problem is local
```

## Read more

- [`docs/design/goat-cli-error-codes.md`](../../../docs/design/goat-cli-error-codes.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

## Exit code

`goat` exits with 4.
