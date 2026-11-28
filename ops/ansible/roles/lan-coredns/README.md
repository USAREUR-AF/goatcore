# lan-coredns — closed-LAN authoritative DNS for a single mgmt-host instance

Per-site CoreDNS role purpose-built for the **closed-LAN posture**: a goatnet instance on a LAN with no uplink, whose instance design is kept with that goatnet's own configuration. Sibling-shape to [`mesh-coredns`](../mesh-coredns/README.md) but with the inverse load-bearing invariants:

| Invariant | `mesh-coredns` | `lan-coredns` |
|---|---|---|
| Bind interface | Netbird mesh IP + optional wg-cp0 IP (NEVER LAN) | **LAN IP only** (no mesh, no wg-cp0) |
| Forwarder | `forward . 1.1.1.1 8.8.8.8` | **none** — a name it does not hold answers NXDOMAIN, inside the zone or outside it, and no query leaves the server |
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

## Answers

The Corefile has two server blocks. The zone's block reads the hosts file and falls through to a zone file holding only the zone's SOA and NS (`templates/zone.soa.j2`, rendered to `lan_coredns_soa_path`). The other block takes every other name and answers NXDOMAIN from a `template`.

| Query | Answer |
|---|---|
| a name in the hosts file | NOERROR, its address |
| that name, for a type the hosts file does not hold (AAAA) | NOERROR, no answer |
| a missing name in the zone | NXDOMAIN, with the zone's SOA (cached for 60 seconds, the SOA minimum) |
| a name outside the zone, reverse names included | NXDOMAIN |

NXDOMAIN, not SERVFAIL: a resolver that gets SERVFAIL treats the server as broken, retries, and may try another server. The hosts plugin alone answers SERVFAIL for a name it does not hold, because a hosts file has no SOA, and so does CoreDNS for a name no server block claims. The catch-all is a block of its own because CoreDNS runs plugins in its compiled order, where `template` comes before `hosts`: in the zone's block it would answer every name before the hosts file was read. `tests/test_lan_coredns_nxdomain.py` checks each row on the role's pinned image.

## Smoke test

`dig @<bind-ip> -p <port> <smoke-name> +short` must return a non-empty answer. `dig ... +noall +comments` must show `status: NXDOMAIN` for a missing name in the zone and for a name under `.invalid`. The rcode is checked, not an empty answer, because SERVFAIL prints nothing under `+short` either.

## Composes with

- Phase B.3 bootstrap-bundle wire-up — distributes the LAN CA's root cert and seeds `/etc/resolv.conf` on Tier-T hosts to point at this resolver.
- Phase C Ansible role tree — Traefik on the first mgmt host consumes service-cert files keyed by hostname; those hostnames must resolve via this role.
- Phase D translator framework — translators on Tier-L hosts do not use this resolver (Tier-L has no DNS by design); the resolver serves Tier-T only.

## The host's own names for its zone

`lan_coredns_host_zone_route_enabled` (default `false`) writes the zone file's
lines (comments dropped) into a marked block of `lan_coredns_hosts_file`
(`/etc/hosts`), and checks that the host then resolves
`lan_coredns_smoke_test_name` to `lan_coredns_smoke_test_expect`. A goatlite turns
it on. Setting it back to `false` removes the block; the rest of the file is kept.

It is not a systemd-resolved route. resolved routes by scope, not by server: a
route to this server (which answers NXDOMAIN for every name outside the zone)
shares a scope with the uplink's server and can take the host's outside names
with it. An earlier form of the role wrote such a route
(`/etc/systemd/resolved.conf.d/50-goat-lan-zone.conf`); every run removes it.
