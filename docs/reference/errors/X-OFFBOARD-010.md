# X-OFFBOARD-010: the audit entry cannot be written, so nothing was removed

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-OFFBOARD-010` is an error of `offboard.sh` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-OFFBOARD-010` or `goatbuilder explain X-OFFBOARD-010`.

## What it means

the last preflight step writes one operator-action entry on each selected goatnet. The write failed, so the driver stopped before the first removal rather than remove access it could not attribute.

## Why it happens

- this computer does not trust the goatnet router's certificate, or cannot reach the router
- the goat context &lt;goatnet>-op has no router_endpoint
- the operator handle is not allowed to publish operator-action entries

## What to do

One line per step or alternative:

```text
goat --context <goatnet>-op audit emit --actor <handle> --surface offboard-driver --action offboard.preflight --target test --env <goatnet> --outcome ok (the same write, alone, to see its error)
goat --context <goatnet>-op health (is the router reachable and trusted from here)
```

## Read more

- [`docs/design/offboarding-one-driver.md`](../../../docs/design/offboarding-one-driver.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
