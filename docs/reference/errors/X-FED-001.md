# X-FED-001: a federation verb cannot read a file it was given (the message names it)

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-FED-001` is an error of `goat-federation` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-FED-001` or `goatbuilder explain X-FED-001`.

## What it means

A federation verb (goatbuilder federate init|lint|compose|diff|verify|apply, or goat federate, which runs the same library) could not read a file it was named: a package, an agreement, a --keyring file or a --signing-key file. The message names the file and the system's reason. Nothing was checked or written.

## Why it happens

- the path is mistyped, or the file was never copied to this machine (a partner's package, the composed agreement)
- the file is not readable by this user
- the path names a directory

## What to do

One line per step or alternative:

```text
check the path the message names: ls -l <path>
copy the file to this machine, or pass the path where it is, and run the same command again
```

## Read more

- [`docs/design/goatbuilder-conformance.md`](../../../docs/design/goatbuilder-conformance.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
