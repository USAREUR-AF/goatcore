# E_POD_NOT_NAMED: no pod named for goat pod

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `E_POD_NOT_NAMED` is printed by `goat`. The same text offline: `goat explain E_POD_NOT_NAMED` or `goatbuilder explain E_POD_NOT_NAMED`.

## What it means

`goat pod` needs the pod's API URL and, for https, the pod's slot: the pod serves its own goat certificate, which names its slot rather than the address dialled, and goat checks that slot (ADR 1251). One of them was missing, or the URL had no scheme. Nothing was sent.

## Why it happens

- neither --pod, GOAT_POD_URL nor pod_endpoint in the context's profile.toml is set
- neither --pod-slot, GOAT_POD_SLOT nor pod_slot in profile.toml is set
- the URL was given without https://

## What to do

One line per step or alternative:

```text
goat pod --pod https://<pod>:<port> --pod-slot <pod-slot> status
set pod_endpoint and pod_slot in ~/.goat/contexts/<name>/profile.toml
```

## Read more

- [`docs/design/pod-carriage-and-provenance.md`](../../../docs/design/pod-carriage-and-provenance.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

## Exit code

`goat` exits with 2.
