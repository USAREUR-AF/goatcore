# X-SSH-002: no route to the host

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-SSH-002` is an error of `ssh` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-SSH-002` or `goatbuilder explain X-SSH-002`.

## What it means

the connection never reached the host: timed out, no route, or refused on port 22.

## Why it happens

- the address is on a network this laptop is not on (a mesh IP without the mesh, a Tailscale IP without the tailnet)
- the host is down

## What to do

One line per step or alternative:

```text
netbird status | head -5
goatbuilder doctor --env <env>
```

## Read more

- [`docs/design/env-doors.md`](../../../docs/design/env-doors.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
