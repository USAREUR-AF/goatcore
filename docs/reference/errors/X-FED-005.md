# X-FED-005: the signing key is not an Ed25519 seed

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-FED-005` is an error of `goat-federation` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-FED-005` or `goatbuilder explain X-FED-005`.

## What it means

federate init and verify sign with the file --signing-key names: an Ed25519 seed, as 64 hex characters or as its 32 bytes in base64, alone in the file. The file holds something else. Nothing was signed.

## Why it happens

- the file is a PEM key or another key format, not the bare seed
- the file holds a public key, a fingerprint or more than the seed

## What to do

One line per step or alternative:

```text
pass the file that holds this party's package signing seed, alone: 64 hex characters or 32 bytes base64
```

## Read more

- [`docs/design/goatbuilder-conformance.md`](../../../docs/design/goatbuilder-conformance.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
