# mesh-coredns

Deploy CoreDNS as the per-site mesh-internal DNS server for `*.netbird.legacy.example`. Generic across every mgmt-host site.

## Why this exists

Bare `netbird.legacy.example` is **public** Cloudflare DNS — round-robin to VPS relays, used for first-contact peer enrollment.

`*.netbird.legacy.example` is **mesh-internal only** — services + per-site VIPs resolved by mesh CoreDNS, pushed to enrolled peers via netbird's nameserver group. **Cloudflare must NOT hold A/CNAME records for `*.netbird.legacy.example`.** This is the silent-control-plane invariant per ADR 0313 + ADR 0314.

This role replaces the previous NETHOST-only inline play (`ops/ansible/nethost-dns.yml`) with a parameterized, multi-site-capable role.

## What it does

1. Render `Corefile` from `templates/Corefile.j2` with the host's mesh IP as the listen address (NOT LAN IP — explicit assertion enforces).
2. Render `docker-compose.yml` from template.
3. Copy the canonical mesh-hosts zone file (`ops/dns/mesh-hosts`) to the host.
4. `docker compose up -d` (idempotent — re-runs are no-ops on healthy hosts).
5. Smoke-test via `dig` against a known record.

## Required variables

| Var | Required? | Description |
|---|---|---|
| `mesh_coredns_bind_ip` | **Yes** | Host's **mesh IP** (`100.X.Y.Z`), the netbird overlay address. Role refuses to run if empty or set to the LAN IP. |

## Optional overrides

| Var | Default | Description |
|---|---|---|
| `mesh_coredns_dir` | `/opt/goat/dns` | Where Corefile + zone + compose live |
| `mesh_coredns_image` | `coredns/coredns:1.11.3` | Pin per Block 13C / Chainguard procurement |
| `mesh_coredns_port` | `5353` | UDP port the resolver listens on |
| `mesh_coredns_zone_dir` | `{{ playbook_dir }}/../dns` | Source location for mesh-hosts file |
| `mesh_coredns_smoke_test_name` | `mgmt.{{ site_id }}.{{ site_dns_zone }}` | Known mesh record for post-deploy verification (per-instance; derives from site identity) |

## Pre-requisites

- Docker + docker-compose plugin installed (handled by `mgmt-host-baseline.yml` → `docker-engine` role).
- Host has a netbird mesh IP — host must be **enrolled** into the mesh before this role runs.
- `ops/dns/mesh-hosts` populated locally (sourced from the NetBird API by the network host's peer-sync script).

## Wiring netbird's nameserver group

This role does NOT update netbird's nameserver group — that's a wrapper-level operation (per-site, may be additive across sites). The wrapper play should:

1. Mint or look up the per-site nameserver group via the netbird API.
2. Add this host's mesh IP + port to the group's `nameservers` list.
3. Set the group's domains to `["netbird.legacy.example"]`.
4. Set `groups` to the mgmt + observer ACL groups for the site.

See `ops/ansible/nethost-dns.yml` for the canonical wrapper pattern (single-site NETHOST) and `ops/ansible/mgmt-host-dns.yml` for the multi-site mgmt-host pattern.

## Standard usage in a wrapper play

```yaml
- name: Deploy mesh DNS on this site's mgmt-host
  hosts: "{{ target | default('mgmt_hosts') }}"
  become: true
  vars:
    mesh_coredns_bind_ip: "{{ mesh_ip }}"   # set per-host in inventory or group_vars
  roles:
    - role: mesh-coredns
```

## Handset nameserver group (ADR 1175, default-off)

NetBird documents match-domain nameserver groups as applied by desktop clients only. An iPhone resolved mesh names through `<site_id>-mesh-dns` anyway; Android is unmeasured. For the case where a phone does not, `goat-coredns.yml` can provision a second group, `<site_id>-handset-dns`: the same CoreDNS replicas, `primary: true`, no match domains, distributed only to `<SITE_PREFIX>-HANDSET` (declared in `netbird_acl_taxonomy.yml`, created by `netbird-acl-bootstrap.yml`, held by phones enrolled on a handset key beside OBSERVER). Every DNS query from a connected phone then reaches CoreDNS, which forwards non-mesh names upstream (`Corefile.j2`, `forward .`). Desktops keep the match-domain group. Switch it on per goatnet with `handset_dns_enabled: true` in group_vars and a `goat-coredns.yml` run; the play refuses if the HANDSET group is missing.
