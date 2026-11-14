# X-TLS-001: the TLS handshake failed

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-TLS-001` is an error of `tls` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-TLS-001` or `goatbuilder explain X-TLS-001`.

## What it means

the certificate presented was not trusted, did not match, or the handshake ended early.

## Why it happens

- a CA this machine does not trust (goat ca add)
- a clock far enough off that the certificate reads as invalid

## What to do

One line per step or alternative:

```text
goat ca ls
date -u (compare with a known-good clock)
```

## Read more

- [`docs/design/internal-tls-ca.md`](../../../docs/design/internal-tls-ca.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
