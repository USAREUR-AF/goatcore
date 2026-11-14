# E_RECIPIENT_KEY_UNPUBLISHED: this anchor's recipient key is not in the recipient-cert store

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `E_RECIPIENT_KEY_UNPUBLISHED` is printed by `goat`. The same text offline: `goat explain E_RECIPIENT_KEY_UNPUBLISHED` or `goatbuilder explain E_RECIPIENT_KEY_UNPUBLISHED`.

## What it means

`goat access key` asked the portal's recipient-cert store for the anchor's public leaf cert and the portal answered 404, so the key service cannot wrap sealed data to that anchor. The text after the dash is the portal's own reason.

## Why it happens

- the anchor's leaf was minted before the recipient-cert store existed, so its public cert was never stored
- the anchor is not the one meant (a typo, or an anchor from another goatnet)

## What to do

One line per step or alternative:

```text
goat access key --anchor <anchor> (check the anchor; with no --anchor it checks your own profile's)
goat admin rotate <anchor> (an operator re-issues the leaf in place, ADR 1044, which stores its public cert; install the new bundle's cert and key on the peer)
goat access key --anchor <anchor> (run again: it reports PUBLISHED)
```

## Read more

- [`docs/design/goat-cli-error-codes.md`](../../../docs/design/goat-cli-error-codes.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

## Exit code

`goat` exits with 1.
