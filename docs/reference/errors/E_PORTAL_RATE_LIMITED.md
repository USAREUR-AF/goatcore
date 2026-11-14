# E_PORTAL_RATE_LIMITED: the portal is rate-limiting this client

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `E_PORTAL_RATE_LIMITED` is printed by `goat`. The same text offline: `goat explain E_PORTAL_RATE_LIMITED` or `goatbuilder explain E_PORTAL_RATE_LIMITED`.

## What it means

the portal is up and answered HTTP 429: this client sent too many requests. The hint carries the portal's Retry-After when it sent one.

## Why it happens

- a script calling goat in a tight loop
- an older goat whose `topic ls` with no prefix made one request per org

## What to do

One line per step or alternative:

```text
wait the Retry-After the hint names, then retry
space the calls out, or list one org at a time: goat topic ls <org>/
goat update (current goat lists the whole goatnet in one request)
```

## Read more

- [`docs/design/goat-cli-error-codes.md`](../../../docs/design/goat-cli-error-codes.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

## Exit code

`goat` exits with 8.
