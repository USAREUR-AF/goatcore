# role: pubsub-state-checkout

Standby (recovery) side of the Block 86 pub/sub state-of-record substrate. Keeps
a **warm, current checkout** of the per-site `pubsub-state` Forgejo repo on the
passive mgmt node plus a **staged `pubsub-apply` binary**, so a failover rebuild
of the Zenoh fabric structure runs from local disk — no clone over a degraded
network at the worst moment.

**Design:** `docs/design/pubsub-state-as-code.md`.

**Companion role:** [`pubsub-state-repo`](../pubsub-state-repo/) — the mirror
half that runs on the ACTIVE node and pushes portal state into Forgejo.

**Apply playbook:** [`ops/ansible/pubsub-state-checkout-deploy.yml`](../../pubsub-state-checkout-deploy.yml).

## What it leaves on the host

- `{{ pubsub_state_checkout_dir }}/repo/` — warm clone of the Forgejo repo (default `/opt/pubsub-state/repo`)
- `{{ pubsub_state_checkout_dir }}/bin/pubsub-apply` — staged reconciler (static linux/arm64)
- `{{ pubsub_state_checkout_dir }}/RECOVERY.md` — the exact failover one-liner
- `/usr/local/bin/pubsub-state-pull.sh` — pull-cycle script
- `pubsub-state-pull.{service,timer}` — every `pubsub_state_checkout_interval_sec` (default 300s)

## Required env vars

| Var                  | Purpose                                            |
| -------------------- | -------------------------------------------------- |
| `FORGEJO_PUBSUB_TOKEN` | Read the Forgejo repo (the mirror bot PAT is reused) |

The Forgejo this reads from is one you run: goatcore ships no Forgejo role. See
[`pubsub-state-repo`](../pubsub-state-repo/README.md#it-needs-a-forgejo-which-goatcore-does-not-deploy).

## What it does NOT do

It never runs `pubsub-apply`. Reconciling mutates the portal (mints certs,
creates/retires topics); that is a deliberate failover action, documented in the
`RECOVERY.md` it drops, not a routine timer job. The role's job is only to make
sure that when failover comes, the state and the tool are already on disk.

## Binary staging

`pubsub_state_apply_binary_src` is a control-node path to a prebuilt static
`linux/arm64` binary. The apply playbook cross-compiles it
(`GOOS=linux GOARCH=arm64 CGO_ENABLED=0 go build ./cmd/pubsub-apply`) before the
role runs. Empty = checkout-only (no binary staged).
