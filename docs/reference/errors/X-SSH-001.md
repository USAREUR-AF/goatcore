# X-SSH-001: the host refused the key

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-SSH-001` is an error of `ssh` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-SSH-001` or `goatbuilder explain X-SSH-001`.

## What it means

the connection reached the host and SSH rejected every key offered.

## Why it happens

- your public key is not on the host's operator user (ssh-key-self-service)
- the wrong user (hosts take the deploy.ssh_user account)
- the key is not loaded in the agent

## What to do

One line per step or alternative:

```text
ssh -v <ssh user>@<host> true 2>&1 | grep -i 'offering\|denied'
goat admin ssh-key … (register the key through the goatnet)
```

## Read more

- [`docs/design/ssh-key-self-service.md`](../../../docs/design/ssh-key-self-service.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
