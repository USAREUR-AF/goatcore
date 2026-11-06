# role: pubsub-state-repo

Provisions the per-site Forgejo repo that holds the pub/sub
state-of-record (Block 86), drops the mirror script + systemd timer,
and seeds the first commit from the live portal.

**Design:** `docs/design/pubsub-state-as-code.md`.

**Apply playbook:** [`ops/ansible/pubsub-state-mirror-deploy.yml`](../../pubsub-state-mirror-deploy.yml).

## It needs a Forgejo, which goatcore does not deploy

The mirror's primary tier is a Forgejo instance for the goatnet. goatcore ships
no Forgejo role or play: run Forgejo the way its own documentation describes (a
container on the obs host is the shape this role was written against), then
declare it in `group_vars/<site>_hosts/pubsub-state.yml` (`pubsub_state_forgejo_url`,
or `FORGEJO_URL` in the environment) and mint the mirror bot with
`ops/scripts/forgejo-mint-pubsub-bot.sh <forgejo-ssh-target>`, which writes
`FORGEJO_PUBSUB_TOKEN`.

The mirror is optional. A goatnet with no `group_vars/<site>_hosts/pubsub-state.yml`
has no mirror: `goatbuilder standup` and `ops/scripts/goat-standup.sh` print
the phase-12 bot mint and mirror deploy as skipped, with that reason, and go on.
Run directly, this role and `pubsub-state-mirror-deploy.yml` refuse at their
pre-flight when the token is unset or the Forgejo does not answer
`<url>/api/v1/version`.

## Required env vars

| Var                       | Purpose                                    |
| ------------------------- | ------------------------------------------ |
| `FORGEJO_PUBSUB_TOKEN`    | Forgejo PAT scoped to write:repository     |
| `PORTAL_OPERATOR_API_KEY` | Portal operator key (Bearer auth)          |

## Portal-by-container resolve mode (opt-in)

Default off: the mirror reaches the portal at `pubsub_state_portal_url`
(`127.0.0.1:8090`), which assumes the portal's 8090 is host-published (a single-host site).

On the three-Orin topology the portal's 8090 is reachable only by container name
on a docker network (no host publish, no Traefik front), and its container IP
moves on redeploy. Set `pubsub_state_portal_container` (+ `_network`, default
`mgmt`; `_port`, default 8090) and the mirror script re-resolves that
container's IP on the named network at the top of **every** cycle, so a portal
redeploy does not strand the mirror. Requires the mirror to run with docker
access (the systemd unit runs as root — it does).

## What it leaves on the host

- `/usr/local/bin/pubsub-state-mirror.sh` — mirror cycle script
- `/etc/systemd/system/pubsub-state-mirror.service` — oneshot unit
- `/etc/systemd/system/pubsub-state-mirror.timer` — every `pubsub_state_mirror_interval_sec` (default 300s)
- `/var/lib/pubsub-state-mirror/repo/` — local working clone of the Forgejo repo

## What it pushes to Forgejo

A repo under org `goat-pubsub-state` with the canonical layout
described in the design doc. Every mirror cycle replaces the tree in
full, so the repo is always "current state" — no history beyond
mirror-commit messages.

## What it pushes to MinIO

When `pubsub_state_minio_bucket` + `pubsub_state_minio_alias` are set
(both have defaults; the alias is `local-<site>`), every cycle also drops
`pubsub-state-snapshots/snapshot-<ts>.tar.gz` and overwrites
`pubsub-state-snapshots/latest.tar.gz`. The MinIO push is best-effort —
a failure logs to stderr but does not abort the mirror cycle.

## Restoring a site from this repo

See the operator runbook `docs/operations/pubsub-state-rebuild.md`.

## Operator one-shot

```
$ cd ops/ansible
$ FORGEJO_PUBSUB_TOKEN=... PORTAL_OPERATOR_API_KEY=... \
    OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES aws-vault exec <profile> --no-session -- \
    ../scripts/ansible-preflight.sh \
      -i inventory/mgmt-hosts.ini -i inventory/mgmt-hosts-mesh.ini \
      -e target=<site>_hosts pubsub-state-mirror-deploy.yml
```
