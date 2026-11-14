# X-NETBIRD-002: the mesh daemon is not running

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-NETBIRD-002` is an error of `netbird` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-NETBIRD-002` or `goatbuilder explain X-NETBIRD-002`.

## What it means

the netbird client could not reach its own daemon.

## Why it happens

- the service is stopped

## What to do

One line per step or alternative:

```text
sudo netbird service start (or restart the desktop client)
```

## Read more

- [`docs/operations/battle-drills/02-mesh-client.md`](../../../docs/operations/battle-drills/02-mesh-client.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
