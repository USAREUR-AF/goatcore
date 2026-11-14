# E_NETBIRD_FAILED: the NetBird management API call failed

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `E_NETBIRD_FAILED` is printed by `goat`. The same text offline: `goat explain E_NETBIRD_FAILED` or `goatbuilder explain E_NETBIRD_FAILED`.

## What it means

a call to the goatnet's NetBird management API did not succeed for a reason other than authorization, so nothing was changed. The help line carries the status or the transport error.

## Why it happens

- the NetBird admin token was rejected (HTTP 401)
- the management server is unreachable from this machine
- the management server answered an unexpected status
- the peer record lacked a setting the update has to send back unchanged

## What to do

One line per step or alternative:

```text
check the token: export <CONTEXT>_NB_API_TOKEN (or GOAT_NB_API_TOKEN) with a current admin token
goat admin peer ls (proves the URL and token work)
retry once the management server answers
```

## Read more

- [`docs/design/goat-cli-error-codes.md`](../../../docs/design/goat-cli-error-codes.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

## Exit code

`goat` exits with 4.
