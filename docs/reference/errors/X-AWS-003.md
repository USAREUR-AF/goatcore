# X-AWS-003: the SSM target is not connected

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-AWS-003` is an error of `aws` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-AWS-003` or `goatbuilder explain X-AWS-003`.

## What it means

Session Manager has no live agent for the instance, so the door cannot open.

## Why it happens

- the host is down or has no route to the SSM endpoint
- the instance id in doors.yaml is stale

## What to do

One line per step or alternative:

```text
aws-vault exec <profile> --no-session -- aws ssm describe-instance-information --region <region>
```

## Read more

- [`docs/design/env-doors.md`](../../../docs/design/env-doors.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
