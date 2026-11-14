# X-HTTP-004: an onboarding call went to the portal

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-HTTP-004` is an error of `http` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-HTTP-004` or `goatbuilder explain X-HTTP-004`.

## What it means

goat sent an onboarding call (invite, approve) to the portal instead of the onboarding endpoint: the portal answers 405 Method Not Allowed, or the call names a host that is only `portal`.

## Why it happens

- the context's profile has no onboard_endpoint, so onboarding calls went to the portal

## What to do

One line per step or alternative:

```text
GOAT_ONBOARD_ENDPOINT=<onboard url> goat … (for this run)
goat --context <name> profile init <bundle.cbor> from a current bundle (the profile then carries onboard_endpoint); goatbuilder doctor's tool:goat-contexts row names the contexts
```

## Read more

- [`docs/design/three-stage-candidate-onboarding.md`](../../../docs/design/three-stage-candidate-onboarding.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
