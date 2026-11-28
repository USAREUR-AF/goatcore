# E_CARRIAGE_NOT_AUTHORIZED: this identity may not make that carriage change

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `E_CARRIAGE_NOT_AUTHORIZED` is printed by `goat`. The same text offline: `goat explain E_CARRIAGE_NOT_AUTHORIZED` or `goatbuilder explain E_CARRIAGE_NOT_AUTHORIZED`.

## What it means

The portal or pod refused a carriage change from this identity (HTTP 403). A rule is granted by the source's custodian and accepted by the pod's custodian; extend, revoke and reactivate belong to the rule's owner; ratify and lapse to the org lead of the pod's org; registering a pod to an operator; on a pod, only identities its signed rules name as local authority may change rules (ADR 1251). Nothing was changed.

## Why it happens

- the active context holds another person's or machine's credential
- the identity is not named as the pod's local authority in its current rule generation
- the identity is not the custodian, rule owner or org lead the change needs

## What to do

One line per step or alternative:

```text
goat --context <name> carriage … (the context that holds the right credential)
goat pod status (shows the pod's rule generation; its local authority comes from that generation)
ask the source's custodian, the pod's custodian or the org lead to make the change
```

## Read more

- [`docs/design/pod-carriage-and-provenance.md`](../../../docs/design/pod-carriage-and-provenance.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

## Exit code

`goat` exits with 1.
