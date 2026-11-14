# X-COSIGN-001: the signature did not verify

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-COSIGN-001` is an error of `cosign` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-COSIGN-001` or `goatbuilder explain X-COSIGN-001`.

## What it means

cosign found no signature for the artifact under the expected identity, or the signature does not match.

## Why it happens

- the artifact was rebuilt without signing
- the wrong public key or identity

## What to do

One line per step or alternative:

```text
cosign verify --key <org key> <image@digest>
```

## Read more

- [`docs/reference/goatbuilder/02-install.md`](../../../docs/reference/goatbuilder/02-install.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
