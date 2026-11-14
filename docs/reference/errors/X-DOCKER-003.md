# X-DOCKER-003: the Docker daemon is not running

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-DOCKER-003` is an error of `docker` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-DOCKER-003` or `goatbuilder explain X-DOCKER-003`.

## What it means

the socket exists but nothing answers, or does not exist at all.

## Why it happens

- docker.service stopped on the host
- a local Docker Desktop that is not launched (never launch it for goat work)

## What to do

One line per step or alternative:

```text
on the host: sudo systemctl status docker
```

## Read more

- [`docs/operations/site-node-troubleshooting.md`](../../../docs/operations/site-node-troubleshooting.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
