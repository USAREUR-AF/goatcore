# X-ANSIBLE-005: the preflight refused a stale or unsafe checkout

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-ANSIBLE-005` is an error of `ansible-preflight` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-ANSIBLE-005` or `goatbuilder explain X-ANSIBLE-005`.

## What it means

ops/scripts/ansible-preflight.sh refuses to run a play from a tree that differs from origin/main in a safety-critical path, or from a stale trunk; ansible reads role defaults from the working tree, so a stale tree templates stale pins.

## Why it happens

- local main behind origin
- a feature branch that touches group_vars or roles

## What to do

One line per step or alternative:

```text
git pull --ff-only origin main (the message prints the exact command)
```

## Read more

- [`docs/operations/goatnet-standup.md`](../../../docs/operations/goatnet-standup.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
