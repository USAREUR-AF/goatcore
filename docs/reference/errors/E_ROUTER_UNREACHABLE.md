# E_ROUTER_UNREACHABLE: the Zenoh router did not answer

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `E_ROUTER_UNREACHABLE` is printed by `goat`. The same text offline: `goat explain E_ROUTER_UNREACHABLE` or `goatbuilder explain E_ROUTER_UNREACHABLE`.

## What it means

the configured router's TCP path or Zenoh session timed out or could not be reached.

## Why it happens

- the mesh tunnel is up but the route to the router is not
- the router service is down on its host
- the endpoint in the profile is stale

## What to do

One line per step or alternative:

```text
goat doctor --all
compare from another enrolled peer before changing anything local
```

## Read more

- [`docs/design/goat-cli-error-codes.md`](../../../docs/design/goat-cli-error-codes.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

## Exit code

`goat` exits with 5.
