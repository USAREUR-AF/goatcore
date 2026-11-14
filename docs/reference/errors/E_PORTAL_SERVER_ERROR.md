# E_PORTAL_SERVER_ERROR: the portal failed on its own side

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `E_PORTAL_SERVER_ERROR` is printed by `goat`. The same text offline: `goat explain E_PORTAL_SERVER_ERROR` or `goatbuilder explain E_PORTAL_SERVER_ERROR`.

## What it means

the portal is up and answered HTTP 5xx for the request path shown. Nothing on this device's network or credential is at fault.

## Why it happens

- the portal or a service behind it (its database, the registry) is failing
- the portal is restarting during a deploy (often a 502 or 503)

## What to do

One line per step or alternative:

```text
retry after the Retry-After the hint names, or in a minute
if it persists, send the code, status and path to a platform admin; the portal's log on the goatnet's management host names the cause
```

## Read more

- [`docs/design/goat-cli-error-codes.md`](../../../docs/design/goat-cli-error-codes.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

## Exit code

`goat` exits with 9.
