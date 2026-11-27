# X-OFFBOARD-007: a declared external account cannot run, so nothing was removed

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-OFFBOARD-007` is an error of `offboard.sh` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-OFFBOARD-007` or `goatbuilder explain X-OFFBOARD-007`.

## What it means

every account in ops/offboard/accounts.yaml must be able to run before the first removal. One has no executable plug-in, or a variable its credential arrives in is unset.

## Why it happens

- ops/offboard/plugins/&lt;type> does not exist or is not executable
- a variable named in the account's credential_env is not set in this shell

## What to do

One line per step or alternative:

```text
ops/scripts/with-secrets ops/scripts/offboard.sh <handle> ... (resolve the credential references for this command)
chmod +x ops/offboard/plugins/<type>
declare the account with type manual to do it by hand and keep it open in the record
```

## Read more

- [`docs/design/offboarding-one-driver.md`](../../../docs/design/offboarding-one-driver.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
