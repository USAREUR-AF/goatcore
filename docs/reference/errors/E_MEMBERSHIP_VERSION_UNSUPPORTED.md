# E_MEMBERSHIP_VERSION_UNSUPPORTED: the membership file uses an unsupported version

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `E_MEMBERSHIP_VERSION_UNSUPPORTED` is printed by `goat`. The same text offline: `goat explain E_MEMBERSHIP_VERSION_UNSUPPORTED` or `goatbuilder explain E_MEMBERSHIP_VERSION_UNSUPPORTED`.

## What it means

goat parsed a membership fragment or scope file under its strict schema, or a generated baseline under its compatibility schema, then refused its numeric version because this build does not support it. Fragment and scope files reject unknown fields as parse errors; generated baselines deliberately permit unknown top-level fields. No membership change was applied.

## Why it happens

- the file's recognized fields were written by a newer tool whose numeric membership version this goat does not support
- the numeric version field was edited by hand while the file's required fields still parse
- the command was pointed at a different membership declaration file whose recognized fields parse but whose numeric version is unsupported

## What to do

One line per step or alternative:

```text
goat update (use a goat version that supports the declaration)
check version: in the file named by the error and confirm the membership declaration directory
for _baseline.generated.yaml, regenerate with python3 ops/warrant/membership.py --generate-baseline using the deployment's operator manifests; do not change only the version number to bypass the refusal
```

## Read more

- [`docs/operations/warrant-runbook.md`](../../../docs/operations/warrant-runbook.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

## Exit code

`goat` exits with 1.
