# X-AWS-002: aws-vault has no credentials for the profile

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-AWS-002` is an error of `aws-vault` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-AWS-002` or `goatbuilder explain X-AWS-002`.

## What it means

the keychain holds no credentials under the profile name, or the keychain could not be opened (headless).

## Why it happens

- a fresh machine
- a headless session cannot prompt the keychain

## What to do

One line per step or alternative:

```text
aws-vault add <profile>
aws-vault list
```

## Read more

- [`docs/design/cloud-infrastructure-discipline.md`](../../../docs/design/cloud-infrastructure-discipline.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
