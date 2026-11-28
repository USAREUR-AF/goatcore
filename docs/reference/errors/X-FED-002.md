# X-FED-002: a federation verb cannot write a file it was asked to write (the message names it)

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-FED-002` is an error of `goat-federation` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-FED-002` or `goatbuilder explain X-FED-002`.

## What it means

A federation verb could not write its output: compose or init writing --out, or apply --confirm writing the rendered files under --out &lt;dir>. The message names the file or directory and the system's reason. apply stops at the first file it cannot write; the files before it in its list are written.

## Why it happens

- --out names a place that does not exist or is not a directory (a path under a file)
- the directory is not writable by this user, or the disk is full

## What to do

One line per step or alternative:

```text
check the place the message names, and pass an --out this user can write
for apply, run the same command again with that --out: it writes every file again
```

## Read more

- [`docs/design/goatbuilder-conformance.md`](../../../docs/design/goatbuilder-conformance.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
