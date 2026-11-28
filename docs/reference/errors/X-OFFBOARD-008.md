# X-OFFBOARD-008: an external-account plug-in failed or left access in place

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-OFFBOARD-008` is an error of `offboard.sh` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-OFFBOARD-008` or `goatbuilder explain X-OFFBOARD-008`.

## What it means

a plug-in exited non-zero, printed something other than the answer the contract defines, or reported that the person still holds access after removal. The row is FAILED and the departure record stays open.

## Why it happens

- the service refused the credential or could not be reached
- the service refused the removal (for example the person still owns something there)
- the plug-in does not follow ops/offboard/README.md

## What to do

One line per step or alternative:

```text
read the row's reason in the result table, fix the cause in the service, and run the same command again
ops/offboard/plugins/<type> check < request.json (run the plug-in alone)
```

## Read more

- [`ops/offboard/README.md`](../../../ops/offboard/README.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
