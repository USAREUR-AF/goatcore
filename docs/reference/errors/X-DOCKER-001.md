# X-DOCKER-001: no permission on the Docker socket

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-DOCKER-001` is an error of `docker` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-DOCKER-001` or `goatbuilder explain X-DOCKER-001`.

## What it means

the user is not in the docker group on that host, so every docker call is refused.

## Why it happens

- running docker without sudo on a host where the operator user is not in the docker group

## What to do

One line per step or alternative:

```text
sudo docker … (hosts run docker as root by baseline)
```

## Read more

- [`docs/operations/site-node-troubleshooting.md`](../../../docs/operations/site-node-troubleshooting.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
