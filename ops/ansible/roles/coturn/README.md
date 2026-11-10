# `coturn` role — STUN/TURN for the all-Rust goat-relay tier

Re-adds coturn (STUN, UDP 3478) to relays deployed by `goat-relay-deploy.yml`.
The legacy Go `relay-server.yml` carried coturn inline; the all-Rust tier never
did, so goatnets on it ran zero STUN — every peer relay-forced, no direct P2P.

## What it does

apt-installs coturn, writes `/etc/turnserver.conf` from the template, enables
`TURNSERVER_ENABLED=1`, and starts the daemon. `external-ip` comes from the
inventory's per-host `goat_relay_public_ip` (set by `inventory/generate.sh`).

## STUN-first / TURN-deferred (default)

`coturn_turn_enabled: false` (default). coturn answers STUN Binding (no auth,
that's the point); TURN allocations can't relay media because the goat-relay
Security Group does not open the relay port range and no TURN creds are
advertised. The netbird `rels://` relay already covers relay-fallback, so TURN's
marginal value is low. To enable TURN later: flip `coturn_turn_enabled`,
set the terraform `coturn_turn_enabled` var (opens UDP 49152-65535), and set
`turn_hosts` in group_vars so `management.json` advertises the TURN servers.

## Usage

**Incremental (preferred for adding coturn to already-running relays):** the
standalone `goat-relay-coturn.yml` applies *only* this role (no goat-relay image
re-push), detecting the public IP via IMDS for `external-ip`:

```
cd ops/ansible
OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES aws-vault exec usareur-af --no-session -- \
  ../scripts/ansible-preflight.sh \
  -i inventory/hosts-zagros.ini -i inventory/zagros-goat-relay-alias.ini \
  goat-relay-coturn.yml
```

**Full relay deploy:** `goat-relay-deploy.yml` also includes this role gated on
`deploy_coturn`:

```yaml
roles:
  - role: goat-relay
  - role: coturn
    when: deploy_coturn | default(false) | bool
```

`deploy_coturn` is a **relay-side** var — set it on the relay group, not the
mgmt group_vars (ZAGROS: `inventory/zagros-goat-relay-alias.ini
[goat_relay_hosts:vars]`). Requires UDP+TCP 3478 open in the goat-relay SG — the
terraform `coturn_enabled` module var. Verify after:

```
nc -uvz relay-<name>.<zone> 3478          # STUN port answers
netbird status -d | grep -E "Connection type:|ICE"   # peers flip to P2P
```

## Relationship to the legacy block

This role is **additive** — it does not touch the inline coturn tasks in
`relay-server.yml` (still used by the legacy Go relay tier). Its template is a
separate copy (`roles/coturn/templates/turnserver.conf.j2`) so the two evolve
independently.
