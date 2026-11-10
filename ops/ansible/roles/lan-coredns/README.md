# lan-coredns — closed-LAN authoritative DNS for a single mgmt-host instance

Per-site CoreDNS role purpose-built for the **closed-LAN posture**: a goatnet instance on a LAN with no uplink, whose instance design is kept with that goatnet's own configuration. Sibling-shape to [`mesh-coredns`](../mesh-coredns/README.md) but with the inverse load-bearing invariants:

| Invariant | `mesh-coredns` | `lan-coredns` |
|---|---|---|
| Bind interface | Netbird mesh IP + optional wg-cp0 IP (NEVER LAN) | **LAN IP only** (no mesh, no wg-cp0) |
| Forwarder | `forward . 1.1.1.1 8.8.8.8` | **none** — return NXDOMAIN for unknown queries |
| Posture | Per-site overlay for hostnames on top of public DNS | **Authoritative** for the entire site zone (`lan_coredns_zone_name`) |
| Trust boundary | Silent control plane (ADR 0313/0314) | The closed LAN itself (the instance design, § Internal CA + DNS) |

The role is deliberately separate from `mesh-coredns` rather than parameterizing the existing one — `mesh-coredns` aggressively rejects LAN binding as a silent-control-plane invariant, and weakening that assertion would create a footgun for every other site that uses the existing role. Keep the postures structurally distinct.

## When to use this role

- Stand up the canonical name server on the first mgmt host of a closed-network deployment.
- Any further closed-LAN site of the same shape (its own zone name, same posture).

Do **not** use this role for:
- A site whose design serves names on the mesh interface — that is `mesh-coredns`.
- Any deployment where the host has a public uplink and needs to forward queries it doesn't own.

## Required variables

| Variable | Description |
|---|---|
| `lan_coredns_bind_ip` | The host's LAN IP (NOT mesh, NOT wg-cp0). Bound on UDP port `lan_coredns_port` (default 53; the closed network has no other consumer of port 53 on the host). |
| `lan_coredns_zone_name` | The authoritative zone (e.g. `site.lan`). Used in synthetic SOA records and zone-file path resolution. |
| `lan_coredns_zone_dir` | Path to the directory containing the zone-data file (default `{{ playbook_dir }}/../dns/{{ site_id }}`). The role reads `<zone_dir>/<zone_name>.hosts`. |
| `lan_coredns_smoke_test_name` | A hostname expected to resolve under the zone (e.g. `portal.site.lan`). |
| `lan_coredns_smoke_test_expect` | The IP that `lan_coredns_smoke_test_name` should resolve to. |

## Optional variables

See `defaults/main.yml`.

## Apply

Via a per-site wrapper playbook, or `ops/ansible/site-services.yml`. The caller sets the bind IP from the per-site inventory and points `lan_coredns_zone_dir` at the site's zone data under `ops/dns/`.

## Smoke test

Same shape as `mesh-coredns` — `dig @<bind-ip> -p <port> <smoke-name> +short` must return a non-empty answer.

## Composes with

- Phase B.3 bootstrap-bundle wire-up — distributes the LAN CA's root cert and seeds `/etc/resolv.conf` on Tier-T hosts to point at this resolver.
- Phase C Ansible role tree — Traefik on the first mgmt host consumes service-cert files keyed by hostname; those hostnames must resolve via this role.
- Phase D translator framework — translators on Tier-L hosts do not use this resolver (Tier-L has no DNS by design); the resolver serves Tier-T only.
