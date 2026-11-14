# X-ANSIBLE-004: a task failed on a host

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-ANSIBLE-004` is an error of `ansible` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-ANSIBLE-004` or `goatbuilder explain X-ANSIBLE-004`.

## What it means

ansible marks the task fatal; its msg field says what the module could not do.

## Why it happens

- see the msg in the fatal line

## What to do

One line per step or alternative:

```text
grep -n -A3 'fatal:' <log>
```

## Read more

- [`docs/operations/goatnet-standup.md`](../../../docs/operations/goatnet-standup.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
