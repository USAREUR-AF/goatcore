# X-NETBIRD-001: no such mesh profile

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-NETBIRD-001` is an error of `netbird` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-NETBIRD-001` or `goatbuilder explain X-NETBIRD-001`.

## What it means

the profile named does not exist on this laptop; profiles are per goatnet.

## Why it happens

- the profile was never added
- the hyphen/underscore form differs from the profile name

## What to do

One line per step or alternative:

```text
netbird profile list
```

## Read more

- [`docs/design/env-doors.md`](../../../docs/design/env-doors.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
