# X-AWS-004: the AWS CLI fell back to an expired aws login session

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-AWS-004` is an error of `aws` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-AWS-004` or `goatbuilder explain X-AWS-004`.

## What it means

the aws-vault credential for the profile is empty or malformed, so the CLI used an expired `aws login` session instead and asked to reauthenticate.

## Why it happens

- the aws-vault credential is empty or malformed, so the CLI fell back to an expired aws login session
- AWS_VAULT_KEYCHAIN_NAME in the environment (a direnv export) points aws-vault at the wrong keychain

## What to do

One line per step or alternative:

```text
aws login (when the error names it)
env -u AWS_VAULT_KEYCHAIN_NAME aws-vault exec <profile> --no-session -- aws sts get-caller-identity (when the keychain name is set)
aws-vault add <profile> (when the key really is bad; it replaces the stored key without deleting it first)
```

## Read more

- [`docs/design/cloud-infrastructure-discipline.md`](../../../docs/design/cloud-infrastructure-discipline.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
