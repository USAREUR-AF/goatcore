# X-SSH-003: the host key changed

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-SSH-003` is an error of `ssh` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-SSH-003` or `goatbuilder explain X-SSH-003`.

## What it means

the host presented a different key than ~/.ssh/known_hosts holds for that address.

## Why it happens

- the host was rebuilt (expected after a re-image)
- an address was reused by another machine

## What to do

One line per step or alternative:

```text
confirm the rebuild with the operator, then: ssh-keygen -R <address>
```

## Read more

- [`docs/operations/site-node-troubleshooting.md`](../../../docs/operations/site-node-troubleshooting.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
