# ADR 0312 — DNS two-tier (Cloudflare public + mesh CoreDNS)

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-04-25 |
| Repo scope | goat core |
| Related | [`0202-three-vps-relay-topology.md`](0202-three-vps-relay-topology.md), [`0206-island-mode-by-construction.md`](0206-island-mode-by-construction.md), `../design/dns-service-discovery.md` |

## TL;DR (for a layperson)

We use two layers of DNS. The first layer (Cloudflare) handles
public DNS — the entries needed for new devices to find the
management server while joining the mesh. The second layer
(mesh-internal CoreDNS on the network host plus dnsmasq on each
relay) handles all the names of services *inside* the mesh — those
are never exposed publicly. This split keeps mesh-internal service
names private, supports island-mode resolution, and prevents
public DNS lookups from leaking knowledge about mesh services.

## Diagram



Source: [`diagrams/0312-dns-two-tier.mmd`](diagrams/0312-dns-two-tier.mmd).

## Context

Two distinct DNS use cases:

1. **Enrollment.** A new peer needs to resolve
   `netbird.legacy.example` (the management plane on-ramp).
   This must be publicly resolvable.
2. **Mesh-internal service discovery.** A peer once enrolled
   needs to resolve `*.netbird.legacy.example` to the mesh IP
   of the network host. Public exposure of these names would
   leak the service inventory and force public DNS dependency
   for routine operation (violating Island Mode).

The two-tier split solves both.

## Decision

Two DNS tiers with clear boundaries:

| Tier | Scope | Implementation |
|---|---|---|
| **Public (Cloudflare)** | Enrollment names: `netbird.legacy.example`, `bridge.legacy.example`, etc. | Cloudflare DNS, Terraform-managed. DNSSEC active (Cloudflare signs; DS at Route 53). |
| **Mesh-internal** | All `*.netbird.legacy.example` service names. | CoreDNS container on the network host + native dnsmasq on each relay. Same `mesh-hosts` zone file on all four. |

Properties:

- **No public DNS records for mesh-internal names.** They exist
  only in the mesh-hosts zone.
- **Push to peers.** NetBird's DNS nameserver group pushes the
  mesh resolver to peers; peers resolve `*.netbird.legacy.example`
  through the mesh resolver, not through the public DNS.
- **Island Mode.** Mesh DNS works without WAN access (zone
  file is local).
- **Source of truth.** `ops/dns/mesh-hosts` is the single
  zone file; CoreDNS and dnsmasq read it. Service-onboarding
  helper script on the network host adds entries.

The full design is in
`../design/dns-service-discovery.md`.
Migration to CoreDNS from jpillora/dnsmasq on the network host
was a Block 13A supply-chain hardening
(`../design/os-supply-chain-security.md`).

## Alternatives considered

- **Public DNS for mesh-internal names.** Leaks service
  inventory; creates public-DNS dependency for routine
  operation. Rejected.
- **Single-tier mesh DNS only.** Loses public enrollment
  on-ramp resolution. Rejected.
- **Different mesh resolvers per site.** Adds drift surface;
  same zone file across all four resolvers is simpler.
  Rejected.

## Consequences

**Positive.** Mesh service names are private. Island Mode
DNS works. Public DNS surface is minimal (enrollment names
only). DNSSEC on the public side. Sovereign mesh-side
resolution.

**Negative.** Two resolvers to keep in sync; we mitigate via
the single zone file plus Ansible deployment.

## Assessor view (SCA-R)

The DNS split serves SC-7 (boundary protection — public DNS
exposes only what is necessary), SC-20 (secure DNS — DNSSEC
on the public side), and CP-2 (contingency — mesh DNS is
island-mode-correct). NIST SP 800-81 secure DNS deployment
guidance applies to the public tier.

## Trigger conditions for revisit

- Public DNS provider lock-in concern (Cloudflare disappears,
  pricing shifts). Have a documented exit path; not urgent.
- Mesh DNS scale exceeds the single-zone-file pattern (~10k
  entries). Re-architect; do not silently expand.

---

*See `GLOSSARY.md` for terms and acronyms.*
