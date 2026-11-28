# E_POD_UNREACHABLE: the pod API could not be reached or its certificate was refused

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `E_POD_UNREACHABLE` is printed by `goat`. The same text offline: `goat explain E_POD_UNREACHABLE` or `goatbuilder explain E_POD_UNREACHABLE`.

## What it means

`goat pod` could not complete a request to the pod's API: no answer at the address, or the TLS handshake failed. goat accepts the pod's certificate only when it chains to the context's CA roots and its CN is the pod's slot (`<slot>` or `<slot>.g<N>`); the text after the summary carries the cause (ADR 1251).

## Why it happens

- the pod is off, or this machine is not on the pod's network or the mesh
- --pod-slot names another pod than the one at that address
- the context's CA roots are another goatnet's

## What to do

One line per step or alternative:

```text
check the URL and that the pod answers (on the pod's network, or over the mesh)
goat pod --pod-slot <the pod's slot> status
goat --context <the goatnet's context> pod status
```

## Read more

- [`docs/design/pod-carriage-and-provenance.md`](../../../docs/design/pod-carriage-and-provenance.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

## Exit code

`goat` exits with 1.
