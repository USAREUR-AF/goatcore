# X-DNS-001: the name did not resolve

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-DNS-001` is an error of `dns` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-DNS-001` or `goatbuilder explain X-DNS-001`.

## What it means

the resolver has no answer for the name; mesh names resolve only while the mesh is up.

## Why it happens

- off the mesh (mesh names are served by mesh CoreDNS)
- a typo in the name

## What to do

One line per step or alternative:

```text
netbird status | head -5
dig +short <name> (then the same against the mesh resolver the profile names: goat names)
```

## Read more

- [`docs/design/mesh-dns-redundancy.md`](../../../docs/design/mesh-dns-redundancy.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
