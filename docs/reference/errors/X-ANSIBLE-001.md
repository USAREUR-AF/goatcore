# X-ANSIBLE-001: a host was unreachable

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-ANSIBLE-001` is an error of `ansible` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-ANSIBLE-001` or `goatbuilder explain X-ANSIBLE-001`.

## What it means

ansible could not open a connection to the host over the transport it was given (ssh over the mesh, or the door).

## Why it happens

- this laptop is not on the goatnet's mesh profile and no door was opened
- the host is down
- the SSH key is not accepted (see the ssh entries)

## What to do

One line per step or alternative:

```text
goatbuilder doctor --env <env> --all (the transport row)
ssh -o ConnectTimeout=5 <ssh user>@<mesh ip> true
```

## Read more

- [`docs/design/env-doors.md`](../../../docs/design/env-doors.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
