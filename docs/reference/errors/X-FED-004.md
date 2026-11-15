# X-FED-004: a bound key from --key or --keyring cannot be used

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-FED-004` is an error of `goat-federation` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-FED-004` or `goatbuilder explain X-FED-004`.

## What it means

federate compose checks each package's signature with the key bound to its sovereign id, given as --key &lt;sovereign_id>=&lt;base64> or in a --keyring file, a JSON object of sovereign_id to base64 key. One of them could not be used: a --key without `=`, a --keyring that is not such an object, or a key that is not a 32-byte Ed25519 public key in base64. Nothing was composed.

## Why it happens

- the --key value is missing its &lt;sovereign_id>= part, or its key part was cut short when copied
- the --keyring file is not a JSON object of strings
- the key is not the partner's package key (for example a fingerprint or a private key)

## What to do

One line per step or alternative:

```text
pass --key <sovereign_id>=<base64>, with the key the partner read aloud at the exchange
check the --keyring file: one JSON object, each value a base64 Ed25519 public key
```

## Read more

- [`docs/design/goatbuilder-conformance.md`](../../../docs/design/goatbuilder-conformance.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
