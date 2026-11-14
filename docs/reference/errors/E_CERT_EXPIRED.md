# E_CERT_EXPIRED: the mTLS certificate is outside its validity

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `E_CERT_EXPIRED` is printed by `goat`. The same text offline: `goat explain E_CERT_EXPIRED` or `goatbuilder explain E_CERT_EXPIRED`.

## What it means

the local certificate is expired, not yet valid, or was rejected by the router.

## Why it happens

- the certificate's validity interval has passed
- the device clock is wrong (see E_CLOCK_SKEW)

## What to do

One line per step or alternative:

```text
goat profile show
goat profile rotate <new-bundle.cbor> (after the operator re-issues it)
```

## Read more

- [`docs/design/internal-tls-ca.md`](../../../docs/design/internal-tls-ca.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

## Exit code

`goat` exits with 6.
