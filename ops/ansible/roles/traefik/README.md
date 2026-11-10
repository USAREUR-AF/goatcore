# traefik (mgmt-host TLS terminator)

> Anchor: [ADR 0407](../../../../docs/adr/0407-internal-x509-ca-for-service-tls.md)
> · Design: [`docs/design/internal-tls-ca.md`](../../../../docs/design/internal-tls-ca.md) §6
> · First consumer: Block 22 wg-cp0

## What this role does

Configures the `traefik` service in the mgmt-host compose stack:

1. **Pre-flight** — fails loudly if `traefik_services` is empty or if any
   referenced cert/key file is missing on the operator laptop. Catches
   "forgot to run cert-sign" before it lands as a confusing Traefik
   container error.
2. **Host filesystem** — creates `traefik_tls_root` (mode 0700)
   and per-service sub-dirs.
3. **Cert delivery** — pushes leaf cert PEM (mode 0644) and leaf key
   PEM (mode 0400) from operator laptop into the per-service dir.
   Cert and key are produced by `ops/enrollment/cmd/cert-sign/` in an
   operator ceremony per ADR 0407.
4. **Dynamic config** — renders `/etc/traefik/dynamic/dynamic.yml`
   from [`templates/dynamic.yml.j2`](templates/dynamic.yml.j2): TLS 1.3
   options, certificates listing, Host-routed routers + backend
   services.
5. **Liveness** — `docker compose up -d traefik` to ensure the
   container is running. Idempotent.

## What this role explicitly does NOT do

- Does NOT install Traefik on the host directly — Traefik runs as a
  compose service alongside postgres-a / signal-a / management-a, per
  [`ops/compose/mgmt-host/docker-compose.yml`](../../../compose/mgmt-host/docker-compose.yml).
- Does NOT generate certs — that's a separate operator ceremony with
  `cert-sign` (CLI lives at `ops/enrollment/cmd/cert-sign/`).
- Does NOT manage Let's Encrypt / ACME — internal-only endpoints
  behind wg-cp0 don't have public DNS per ADR 0314.
- Does NOT restart the Traefik container on cert/config change —
  Traefik's `--providers.file.watch=true` picks up rotations
  automatically.

## Required vars

`traefik_services` (list of dicts) — required, per group_vars or per-host.
Each entry:

| Field | Example | Purpose |
|---|---|---|
| `name` | `mgmt-a` | Service identifier; becomes `<traefik_tls_root>/<name>/` and the router/service name in dynamic config. |
| `hosts` | `["mgmt.vega.netbird.legacy.example", "198.18.0.1", "127.0.0.1"]` | Host header values that route to this service. |
| `backend` | `"http://management-a:80"` | Upstream container URL inside the compose `mgmt` network. |
| `cert_src` | `"~/.goat/state/vega/mgmt-a-cert.pem"` | Path on operator laptop to the leaf cert PEM. |
| `key_src` | `"~/.goat/state/vega/mgmt-a-key.pem"` | Path on operator laptop to the leaf key PEM. |

## Operator workflow

```bash
# 1. Issue cert (one-time ceremony; once per service per year)
cert-sign \
  --root-cert ops/enrollment/ca/dev/root-cert.pem \
  --root-key  ops/enrollment/ca/dev/ca.key.age \
  --age-identity ~/.config/age/keys.txt \
  --site-id vega \
  --cn mgmt.vega.netbird.legacy.example \
  --san 198.18.0.1 --san 127.0.0.1 --san mgmt.vega.netbird.legacy.example \
  --out-cert ~/.goat/state/vega/mgmt-a-cert.pem \
  --out-key  ~/.goat/state/vega/mgmt-a-key.pem

# 2. Push to host (Ansible pulls cert+key from the paths above)
cd ops/ansible
OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES ansible-playbook \
  -i inventory/hosts.ini -i inventory/mgmt-hosts.ini \
  -l mgmt-vega-01-orin <playbook-that-imports-this-role>.yml
```

## Cert rotation (annual)

Re-run the `cert-sign` ceremony, then re-run the playbook. The role
overwrites the existing cert/key. Traefik's file-watcher picks up the
new cert on next handshake; existing TLS sessions continue with the
old cert until they re-handshake.
