# E_TOPIC_CONFLICT: the topic already exists

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `E_TOPIC_CONFLICT` is printed by `goat`. The same text offline: `goat explain E_TOPIC_CONFLICT` or `goatbuilder explain E_TOPIC_CONFLICT`.

## What it means

`goat topic create` was asked for a name that is already registered.

## Why it happens

- someone created it first
- a re-run of a create that already succeeded

## What to do

One line per step or alternative:

```text
goat topic show <topic>
pick a new name, or version the topic (docs/design/pubsub-best-practices.md)
```

## Read more

- [`docs/design/goat-cli-error-codes.md`](../../../docs/design/goat-cli-error-codes.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

## Exit code

`goat` exits with 1.
