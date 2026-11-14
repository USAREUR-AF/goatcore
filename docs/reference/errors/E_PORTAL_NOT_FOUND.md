# E_PORTAL_NOT_FOUND: the portal answered that what was named is not registered

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `E_PORTAL_NOT_FOUND` is printed by `goat`. The same text offline: `goat explain E_PORTAL_NOT_FOUND` or `goatbuilder explain E_PORTAL_NOT_FOUND`.

## What it means

the portal is up and answered HTTP 404 for the request path shown: the org, topic, schema, machine or person the command named does not exist there. An answer, not a network fault.

## Why it happens

- a typo in the org prefix, topic key or handle
- the thing is registered on another goatnet than the active context's
- it was removed, or never created

## What to do

One line per step or alternative:

```text
goat vendor ls (the orgs that exist)
goat topic ls <org>/ (the topics under one org)
goat context show (which goatnet this context talks to)
```

## Read more

- [`docs/design/goat-cli-error-codes.md`](../../../docs/design/goat-cli-error-codes.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

## Exit code

`goat` exits with 7.
