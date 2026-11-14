# X-HTTP-003: the service is up but its backend is not (5xx)

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-HTTP-003` is an error of `http` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-HTTP-003` or `goatbuilder explain X-HTTP-003`.

## What it means

the front door answered with a gateway error: Traefik or Caddy is up and the service behind it is not.

## Why it happens

- the container is restarting or down

## What to do

One line per step or alternative:

```text
on the host: sudo docker ps --format '{{.Names}} {{.Status}}' | grep -v healthy
```

## Read more

- [`docs/operations/site-node-troubleshooting.md`](../../../docs/operations/site-node-troubleshooting.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
