# X-AWS-001: AWS rejected the credentials

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-AWS-001` is an error of `aws` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-AWS-001` or `goatbuilder explain X-AWS-001`.

## What it means

the access key is unknown to AWS, or a session token was sent where none is accepted.

## Why it happens

- aws-vault run without --no-session (STS tokens are rejected by this account)
- a rotated key still in the keychain

## What to do

One line per step or alternative:

```text
aws-vault exec <profile> --no-session -- aws sts get-caller-identity
aws-vault add <profile> (re-enter the current key)
```

## Read more

- [`docs/design/cloud-infrastructure-discipline.md`](../../../docs/design/cloud-infrastructure-discipline.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
