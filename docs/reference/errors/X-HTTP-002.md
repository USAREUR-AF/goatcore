# X-HTTP-002: the service refused the identity (403)

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-HTTP-002` is an error of `http` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-HTTP-002` or `goatbuilder explain X-HTTP-002`.

## What it means

the credential was accepted and the identity is not allowed to do this.

## Why it happens

- the wrong one of two credentials (participant vs operator)
- no grant for this surface

## What to do

One line per step or alternative:

```text
for goat admin verbs: GOAT_PORTAL_TOKEN=op_… goat …
```

## Read more

- [`docs/design/identity-access-and-onboarding-reference.md`](../../../docs/design/identity-access-and-onboarding-reference.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
