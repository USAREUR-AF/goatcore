# X-ANSIBLE-006: an Alertmanager render would drop a receiver or its credential

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-ANSIBLE-006` is an error of `ansible` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-ANSIBLE-006` or `goatbuilder explain X-ANSIBLE-006`.

## What it means

obs-alertmanager-render.yml refused to write an obs host's Alertmanager config, because a heartbeat address has no token or because the new config lacks a receiver, or a receiver's credential, that the running config has. Nothing was written.

## Why it happens

- a receiver's secret (the heartbeat token, a paging key, a webhook password) is missing from the deploy environment, so the template rendered the receiver without it
- the site's settings no longer declare a receiver the host still runs

## What to do

One line per step or alternative:

```text
resolve the missing secret in the deploy environment (the message names the receiver and its kind), then run the play again
ansible-playbook obs-alertmanager-render.yml -e target=<site>_obs_hosts -e obs_alertmanager_allow_receiver_loss=true (only to remove a receiver on purpose)
a watcher that runs with no token: set deadman_webhook_token_optional: true in the obs group's settings
```

## Read more

- [`ops/ansible/roles/obs-host/README.md`](../../../ops/ansible/roles/obs-host/README.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)
