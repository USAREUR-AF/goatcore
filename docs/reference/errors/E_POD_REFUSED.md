# E_POD_REFUSED: the pod refused the request

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `E_POD_REFUSED` is printed by `goat`. The same text offline: `goat explain E_POD_REFUSED` or `goatbuilder explain E_POD_REFUSED`.

## What it means

The pod answered with a refusal no narrower code names, such as `invalid_request` (HTTP 400). The pod's own message, and its suggested fix when it gave one, follow the summary. Nothing was changed (ADR 1251).

## Why it happens

- an argument the pod does not accept (a source that is not a slot, a malformed time)
- a goat build and a pod API of different versions

## What to do

One line per step or alternative:

```text
read the pod's message and fix shown under the error
goat pod status (shows the pod's limits: longest rule length and reactivation window)
```

## Read more

- [`docs/design/pod-carriage-and-provenance.md`](../../../docs/design/pod-carriage-and-provenance.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

## Exit code

`goat` exits with 1.
