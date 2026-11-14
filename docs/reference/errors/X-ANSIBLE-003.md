# X-ANSIBLE-003: a play assertion failed on the host

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-ANSIBLE-003` is an error of `ansible` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-ANSIBLE-003` or `goatbuilder explain X-ANSIBLE-003`.

## What it means

a task asserted a precondition, usually that a templated secret is non-empty, and it was false on the host.

## Why it happens

- a secret the play templates was empty in the deployer's environment (see GB-007)
- a group_vars fact missing for this goatnet

## What to do

One line per step or alternative:

```text
goatbuilder deploy --env <env> --component <c> --dry-run (names every variable the play needs)
```

## Read more

- [`docs/operations/goatnet-standup.md`](../../../docs/operations/goatnet-standup.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
