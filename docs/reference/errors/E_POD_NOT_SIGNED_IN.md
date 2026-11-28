# E_POD_NOT_SIGNED_IN: the pod received no client certificate it accepts

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `E_POD_NOT_SIGNED_IN` is printed by `goat`. The same text offline: `goat explain E_POD_NOT_SIGNED_IN` or `goatbuilder explain E_POD_NOT_SIGNED_IN`.

## What it means

The pod answered HTTP 401 (`not_signed_in`): it signs people in by client certificate and received none it accepts. Nothing was changed (ADR 1251).

## Why it happens

- the context holds no goat certificate, or one from another goatnet
- the pod was reached over http://, which sends no certificate

## What to do

One line per step or alternative:

```text
goat context ls (pick the context holding this goatnet's certificate)
goat --context <name> pod --pod https://<pod>:<port> status
```

## Read more

- [`docs/design/pod-carriage-and-provenance.md`](../../../docs/design/pod-carriage-and-provenance.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

## Exit code

`goat` exits with 1.
