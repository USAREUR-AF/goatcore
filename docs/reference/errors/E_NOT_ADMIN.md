# E_NOT_ADMIN: an operator-only action without operator authority

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `E_NOT_ADMIN` is printed by `goat`. The same text offline: `goat explain E_NOT_ADMIN` or `goatbuilder explain E_NOT_ADMIN`.

## What it means

a `goat admin` action was refused, client-side (no operator credentials in the profile) or by the portal/NetBird API (HTTP 403, nothing changed).

## Why it happens

- the profile is using the vp_ participant token, not the op_ operator key
- the role is not authorized for this action

## What to do

One line per step or alternative:

```text
GOAT_PORTAL_TOKEN=op_… goat admin …
goat admin operator-key request --reason … (when you hold no op_ key)
```

## Read more

- [`docs/design/platform-admin-lifecycle.md`](../../../docs/design/platform-admin-lifecycle.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

## Exit code

`goat` exits with 4.
