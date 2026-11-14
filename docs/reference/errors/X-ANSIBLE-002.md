# X-ANSIBLE-002: sudo on the host wants a password

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-ANSIBLE-002` is an error of `ansible` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-ANSIBLE-002` or `goatbuilder explain X-ANSIBLE-002`.

## What it means

a task needed privilege escalation and the host's operator user is not NOPASSWD.

## Why it happens

- the host was not baselined (mgmt-host-baseline.yml sets NOPASSWD sudo)
- the wrong remote user

## What to do

One line per step or alternative:

```text
run ops/ansible/mgmt-host-baseline.yml against the host once
```

## Read more

- [`docs/operations/goatnet-standup.md`](../../../docs/operations/goatnet-standup.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
