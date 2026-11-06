# Egress Proxy — design + decision

Per-site, cascading, audited egress control for mesh peers. Closes the
"enrolled mesh peer can reach arbitrary internet" back-door before
Phase 3 production.

**Status:** design locked 2026-04-24. Implementation tracked at
`../project/egress-proxy-implementation.md`
and `../project/implementation-plan.md`
Block 17.

**Framing.** This design is grounded in published standards, not
invented for this project. References used throughout:

- NIST SP 800-207 (Zero Trust Architecture) §3.4 — PEP at egress
- CISA Zero Trust Maturity Model v2.0 — Network pillar, Advanced tier
- DISA SRG-NET-000132 / Forward Proxy SRG — compliance target
- CIS Controls v8 — controls 13.5 + 13.10
- Google BeyondCorp egress papers — reference implementation

---

## Table of contents

- [1. Problem + goals](#1-problem--goals)
- [2. Threat model](#2-threat-model)
- [3. The 5-tier pattern](#3-the-5-tier-pattern)
- [4. Tier-by-tier design](#4-tier-by-tier-design)
- [5. Cross-cutting: auth, audit, HA, bypass, integrity](#5-cross-cutting-auth-audit-ha-bypass-integrity)
- [6. Approval + exception workflow](#6-approval--exception-workflow)
- [7. What we reject and why](#7-what-we-reject-and-why)
- [8. Complexity mitigation self-audit](#8-complexity-mitigation-self-audit)
- [9. Trigger conditions for revisit](#9-trigger-conditions-for-revisit)
- [10. Related artifacts](#10-related-artifacts)

---

## 1. Problem + goals

### 1.1 The back-door

Without egress control, any host enrolled in the Netbird mesh can also freely navigate
the public internet through its site's WAN gateway. Enrollment
gives the host mesh membership **and** unrestricted egress at the same
time. There is no audit of what a peer reaches, no policy over what it
is allowed to reach, no way to revoke internet access without
de-enrolling the peer.

### 1.2 Goals

1. **Default-deny egress.** A site's WAN gateway blocks all outbound
   traffic except what Tier 1 explicitly permits.
2. **Identity-bound forward proxy** as the normal egress path, with
   per-request audit (who, where, when, bytes, decision).
3. **No TLS MITM** — SNI-level visibility only. Full-URL / per-path
   policy is out of scope for v1; documented as a compliance trade-off
   with explicit revisit triggers.
4. **Per-site deployment.** Every site is autonomous during WAN
   partition (island-mode correctness). A site does not depend on
   another site's proxy to function.
5. **Dual-control exceptions.** No individual operator can open an
   egress path unilaterally. Two independent peer approvals required
   for any allowlist change or break-glass exception.
6. **Auto-expiry for all exceptions.** Nothing gets a permanent
   allowlist entry without explicit annual re-review.
7. **Reuse existing primitives.** No new services invented. Every
   component either already runs in the fleet or has a published DISA
   STIG benchmark.

### 1.3 Non-goals (v1)

- Full-URL or per-method HTTPS policy (requires MITM)
- User-level (human) authentication — source-host auth first, migrate
  to Kanidm SPNEGO when `implementation-plan.md` Block 3 lands
- Cross-site log aggregation — assumed but noted as a follow-on
  (Prometheus/Loki federation is not yet deployed cross-site)
- Egress for bridge VPSes (they already have partner-specific socat
  forwards with CIDR allowlists; see
  `../operations/bridge-peering.md`)
  — this design covers *mesh peer → internet*, not *mesh → partner*

---

## 2. Threat model

### 2.1 Who / what are we defending against

| Threat | Mitigation tier(s) |
|---|---|
| Compromised mesh peer exfiltrates data over HTTPS POST to attacker-controlled host | Tier 2 SNI allowlist + Tier 2 audit log (attacker host is not on allowlist → denied + logged) |
| Compromised peer exfiltrates via DoH (DNS-over-HTTPS to 1.1.1.1) | Tier 0 — DoH bootstrap IPs blocked; DNS forced through mesh resolver |
| Compromised peer exfiltrates via QUIC / HTTP/3 (UDP/443) | Tier 1 — default-deny outbound UDP/443 |
| Operator error — accidentally reaches wrong partner endpoint | Tier 2 allowlist denies; Tier 2 log shows the attempt |
| Ops pressure — 3am "just unblock github.com" shortcut | Tier 6 approval workflow prevents unilateral change; break-glass path is the honest escape |
| Insider — one operator adds a malicious destination to allowlist | Dual-path approval via §6 policy engine: normal path = role-holder review; fast path = two distinct OIDC subjects (≠ requestor) required |
| Supply chain — compromised upstream host serves malware | Out of scope at Tier 2 (no content inspection); handled at OS/image layer by Block 13 supply chain controls |
| Long-lived exceptions forgotten and never reviewed | Auto-expiry — systemd timer scans for expired entries, opens removal PR |

### 2.2 What we accept

- **SNI spoofing / domain fronting.** SNI-based allowlist can be
  defeated by an attacker who controls a domain behind a large CDN
  whose SNI we allow. Accepting this trade-off in v1; MITM would close
  it but costs sovereignty (CA on every peer) and compliance overhead
  that doesn't pay for itself yet.
- **Side-channel exfil.** Timing channels, DNS-query-length channels,
  etc. are not mitigated at this layer. Network-layer side channels
  are a Phase 4 / TEMPEST concern, not a Phase 2 proxy concern.
- **Logging gap on HTTPS bodies.** No-MITM means we log SNI + bytes
  but not method, path, or body. Incident responders need to know this
  going in — it's documented in
  [`../operations/egress-runbook.md`](../operations/egress-runbook.md).

---

## 3. The 5-tier pattern

A single-layer proxy is not enough. The design is five independent
tiers, each catching a distinct class of evasion.

```text
  mesh peer
     │
     ▼
  ┌─────────────────────────────────────────────────────┐
  │ Tier 0: DNS egress control                          │
  │  • block outbound UDP/53, TCP/53, TCP/853           │
  │  • block known DoH endpoint IPs                     │
  │  • force all DNS through mesh CoreDNS/dnsmasq       │
  └─────────────────────────────────────────────────────┘
     │
     ▼
  ┌─────────────────────────────────────────────────────┐
  │ Tier 1: Default-deny gateway firewall               │
  │  • UFW/nftables on Router-A/B                       │
  │  • allows: loopback, mesh CIDR, observability,      │
  │    proxy egress, a short bypass list (NTP, etc.)    │
  │  • denies: everything else                          │
  └─────────────────────────────────────────────────────┘
     │ (only proxy traffic reaches here)
     ▼
  ┌─────────────────────────────────────────────────────┐
  │ Tier 2: HTTP/HTTPS forward proxy (Squid)            │
  │  • explicit proxy (HTTP_PROXY env)                  │
  │  • SNI-based allowlist                              │
  │  • identity = source mesh IP (→ Kanidm SPNEGO later)│
  │  • access log → Loki                                │
  └─────────────────────────────────────────────────────┘
     │
     ▼
  ┌─────────────────────────────────────────────────────┐
  │ Tier 3: SOCKS5 proxy (dante) — non-HTTP             │
  │  • git-ssh, rsync, narrow protocol exceptions       │
  │  • same identity model + audit log                  │
  │  • smaller allowlist than Tier 2                    │
  └─────────────────────────────────────────────────────┘
     │
     ▼
  ┌─────────────────────────────────────────────────────┐
  │ Tier 4: Direct CIDR allowlist — last resort         │
  │  • firewall rules with expiry date + ticket ref     │
  │  • systemd timer auto-expires stale rules           │
  │  • requires two-of-N approval                       │
  └─────────────────────────────────────────────────────┘
```

Each tier is independently testable, replaceable, and logged.
Failure of one tier does not open a hole in the others.

---

## 4. Tier-by-tier design

### 4.1 Tier 0: DNS egress control

**Why this is Tier 0, not part of Tier 1.** DNS is the first egress
decision. DoH/DoT (DNS-over-HTTPS/TLS) to public resolvers is the
single most effective modern proxy-evasion technique — a compromised
peer can bypass Tier 2 entirely if it can reach `1.1.1.1/dns-query`
directly. DNS must be constrained *before* we even talk about HTTP.

**Mechanism.**

| Destination | Policy | Enforced by |
|---|---|---|
| UDP/53 outbound | DROP (except to mesh resolver IP) | Tier 1 firewall |
| TCP/53 outbound | DROP (except to mesh resolver IP) | Tier 1 firewall |
| TCP/853 outbound (DoT) | DROP | Tier 1 firewall |
| Known DoH endpoint IPs (Cloudflare, Google, Quad9, NextDNS, Mullvad) | DROP | Tier 1 firewall |
| All other DNS traffic | Must resolve via mesh CoreDNS (network host) or dnsmasq (VPS) | Already deployed |

**Maintained blocklist.** `ops/egress/doh-blocklist.txt` — updated
quarterly via a CI job that pulls from the Curl Trust Anchor DoH list
combined with a curated known-public-DoH list. Changes go through the
same two-of-N PR workflow as allowlist changes.

**Gap acknowledged.** Well-funded adversaries can run DoH on arbitrary
IPs behind commercial CDNs. Blocklist is a speed-bump, not an
impenetrable wall. The Tier 2 SNI allowlist catches the follow-on
HTTPS traffic if DoH somehow gets through Tier 0.

### 4.2 Tier 1: Default-deny gateway firewall

**Mechanism.** UFW on each mgmt-host (Router-A + Router-B), default
policy `deny (outgoing)`. Explicit allow rules for exactly the
following traffic classes:

| Allowed | CIDR / port | Reason |
|---|---|---|
| Loopback | 127.0.0.0/8 | Host-local services |
| Mesh CIDR | 100.64.0.0/10 | Peer-to-peer + service mesh |
| Mesh DNS resolver | Tier 0 exempt | DNS path |
| Tier 2 proxy (Squid) egress | `squid` user, outbound :80, :443 | The sanctioned HTTP path |
| Tier 3 proxy (dante) egress | `sockd` user, outbound per-protocol | Sanctioned non-HTTP path |
| Observability (Prometheus remote_write, Loki push) | specific CIDRs | See §5.2 proxy-bypass |
| NTP upstream | specific NTS servers, UDP/123, TCP/4460 | See [`ntp-strategy.md`](ntp-strategy.md) |
| pgBackRest S3 endpoints | AWS S3 CIDRs for this region | Backup path; see [`pgbackrest.md`](pgbackrest.md) |
| Tier 4 direct allowlist rules | See §4.5 | Last-resort exceptions |

**Default-deny UDP/443** (QUIC / HTTP/3) is explicit. Without this,
Go clients and modern browsers will use QUIC and bypass Squid. Apps
fall back to TCP/443 → Tier 2.

**Ansible role.** `ops/ansible/roles/egress-firewall/` — renders
nftables rules from a single source of truth
(`ops/egress/firewall-policy.yaml`). Changes are PR-gated.

### 4.3 Tier 2: HTTP/HTTPS forward proxy (Squid)

**Why Squid.**

- 30+ years of operational track record
- Published DISA STIG benchmark (Squid 4.x / 5.x)
- FIPS-compatible build path using OpenSSL FIPS module
- Native SNI parsing without MITM (`ssl_bump peek + splice`)
- Access-log format is SIEM-standard
- Chainguard publishes a minimal Squid image (`cgr.dev/chainguard/squid`)

**Mode.** **Explicit proxy**, not transparent. Peers are configured
with `HTTP_PROXY` / `HTTPS_PROXY` env vars via
`/etc/environment` + systemd drop-ins + `apt.conf.d/proxy` +
`~/.docker/config.json` + `~/.cargo/config.toml`, all rendered by
the existing Ansible role chain. Apps that don't honor env vars
simply fail — that's the correct default-deny behavior.

**TLS handling.** `ssl_bump peek + splice` — Squid peeks at the
ClientHello to extract SNI, consults the allowlist, then either
splices the connection through (allowed) or resets (denied). No
MITM; no custom CA on peers; client cert pinning still works.

**Allowlist format.** Authoritative storage is the Spock-replicated
`egress_policy` table managed by the §6 policy engine. The on-disk
serialization for Squid + dante config rendering is shown below for
reference; operators submit/edit allowlist entries via the policy
engine Web UI / CLI, **not** by editing this file directly.

```yaml
# Rendered from egress_policy table by db-policy-engine.
# Manual edits will be overwritten on next coordinated-upgrade run.
# Submit changes via: db egress-policy submit ...
# Every entry MUST have: host-pattern, reason, owner, expires (or "review-annual").
allowlist:
  - sni: "*.chainguard.dev"
    reason: "Container image pulls (Block 13C migration)"
    owner: "@owner-team"
    expires: "review-annual"
  - sni: "esm.ubuntu.com"
    reason: "Ubuntu Pro FIPS updates (Block 6A)"
    owner: "@owner-team"
    expires: "review-annual"
  - sni: "contracts.canonical.com"
    reason: "Ubuntu Pro attach"
    owner: "@owner-team"
    expires: "review-annual"
  - sni: "ghcr.io"
    reason: "Our own images"
    owner: "@owner-team"
    expires: "review-annual"
  - sni: "index.crates.io"
    reason: "Rust crate registry (zenoh-plugins build)"
    owner: "@owner-team"
    expires: "review-annual"
  - sni: "crates.io"
    reason: "Rust crate downloads"
    owner: "@owner-team"
    expires: "review-annual"
  - sni: "acme-v02.api.letsencrypt.org"
    reason: "ACME cert issuance"
    owner: "@owner-team"
    expires: "review-annual"
  - sni: "api.cloudflare.com"
    reason: "DNS-01 challenge + Terraform Cloudflare provider"
    owner: "@owner-team"
    expires: "review-annual"
  - sni: "*.amazonaws.com"
    reason: "AWS APIs (Terraform, SSM, Secrets Manager, S3)"
    owner: "@owner-team"
    expires: "review-annual"
  - sni: "api.github.com"
    reason: "CI + release automation"
    owner: "@owner-team"
    expires: "review-annual"
  # ... more (enumerated during Phase 1 rollout) ...

break_glass:
  # Short-term exceptions. Auto-expire via systemd timer.
  # Example:
  # - sni: "downloads.example.com"
  #   reason: "Incident 2026-05-XX — partner delivery"
  #   owner: "@opsperson"
  #   approvers: ["@reviewer1", "@reviewer2"]
  #   ticket: "https://github.com/<org>/<repo>/issues/NNN"
  #   expires: "2026-05-XX"
```

**Initial allowlist scope.** Enumeration is real work, not trivial.
The implementation plan calls for a "monitor mode" first phase where
Squid logs but does not block — the real allowlist is derived from
observed traffic plus the known-needed destinations above.

**Audit log.**

- Format: Squid `logformat` customized to W3C extended + JSON (for Loki)
- Fields: timestamp, source (mesh IP → peer name lookup), destination SNI, destination IP, bytes_up, bytes_down, decision (`TCP_TUNNEL` / `TCP_DENIED`), latency_ms, rule_id
- Shipped to existing Loki via promtail sidecar
- Retention: 90 days hot (Loki), 1 year cold (S3 via pgBackRest-adjacent pipeline) — matches CIS 8.10 guidance
- Prometheus counters for decision-type, top-10 destinations, denied-request rate — alerts wired to existing Alertmanager

**Container hardening** (mirrors Block 13 posture):

- Chainguard `cgr.dev/chainguard/squid` image, pinned by digest
- `user: squid` (rootless inside container)
- `read_only: true` root filesystem, tmpfs for `/var/spool/squid`
- `cap_drop: [ALL]` + `cap_add: [NET_BIND_SERVICE]` only
- seccomp profile: default Docker + explicit deny of `ptrace`, `mount`, `clone` (namespaces)
- `no-new-privileges: true`
- Resource limits: 512 MB RAM, 1 vCPU, 10k file descriptors
- Runtime scanning via existing secwatch cadence

### 4.4 Tier 3: SOCKS5 proxy (dante) — non-HTTP

**Scope.** Protocols that cannot reasonably go through an HTTP proxy:

| Protocol | Typical source | Justification |
|---|---|---|
| git over SSH | Ansible `git` role, developer workflows | Some repos are SSH-only |
| rsync | Backup verification, ad-hoc data movement | Not HTTP-capable |
| Non-HTTP Ubuntu mirrors (rare) | Fallback only | Edge case, may not be needed |

**Why dante over gost.**

- Dante has a longer operational track record and aligned STIG history
- Clearer audit-log format
- Simpler config surface — less chance of misconfiguration
- gost is more modern but the multi-protocol flexibility creates
  audit complexity we don't need; tracked as a successor candidate
  (same pattern as Compose → Podman+Quadlet)

**Auth + allowlist.** Same source-IP auth as Tier 2. Separate
allowlist file: `ops/egress/socks-allowlist.yaml`. Smaller set of
allowed destinations than Tier 2 because most things go over HTTPS.

**Audit.** Dante logs per-connection: timestamp, source, destination,
bytes, decision. Shipped to Loki via same pipeline as Squid.

### 4.5 Tier 4: Direct CIDR allowlist — last resort

**When to use this tier.** Only when Tier 2 and Tier 3 cannot handle a
required flow — e.g., a legacy protocol, a partner integration that
isn't HTTP and isn't SOCKS-friendly, or a diagnostic window.

**Format.** nftables rules in `ops/egress/direct-allowlist.yaml`:

```yaml
allowlist:
  - cidr: "198.51.100.0/28"
    ports: [443]
    protocol: tcp
    reason: "Partner X cert-pinned API that rejects SOCKS"
    owner: "@owner-team"
    ticket: "https://github.com/<org>/<repo>/issues/NNN"
    approvers: ["@reviewer1", "@reviewer2"]
    expires: "2026-10-31"
```

**Auto-expiry.** `ops/egress/expire-scanner.sh` runs hourly under a
systemd timer:

1. Parses all three allowlist files
2. Compares `expires:` to today
3. Opens a removal PR 7 days before expiry (grace period)
4. If expiry passes, opens an urgent removal PR and fires Alertmanager
5. Merging the removal PR regenerates the nftables/Squid/dante config

### 4.6 Tier summary

| Tier | Component | Scope | Audit surface |
|---|---|---|---|
| 0 | nftables + mesh DNS | DNS egress control | Per-packet at firewall |
| 1 | nftables | Default-deny everything else | Per-packet at firewall |
| 2 | Squid | HTTP/HTTPS (normal path) | Per-request SNI + bytes |
| 3 | dante | SOCKS5 for non-HTTP | Per-connection metadata |
| 4 | nftables | Direct CIDR exceptions | Per-packet at firewall |

---

## 5. Cross-cutting: auth, audit, HA, bypass, integrity

### 5.1 Auth model — proxy-side (peers reaching the proxy)

Distinct from §6.6 (operator identity for the *policy engine*); this
section covers how Squid + dante identify the *mesh peer* making an
egress request.

**v1: source-IP → peer identity.** Every mesh peer has a stable mesh
IP assigned by Netbird. The Squid + dante config look up the peer name
via the Netbird API (cached + refreshed every 5 min) and log it
alongside the source IP. Audit entries read as "peer `host-site-a`
requested `ghcr.io`" rather than "100.127.1.42 requested ghcr.io" —
more useful for incident response and matches BeyondCorp's
device-identity pattern.

**v2: OIDC SPNEGO (IdP-abstract).** Once an OIDC IdP is in production
(Dex per Block 3C is the v1 IdP; Kanidm is one possible upstream
behind Dex but not the only one), Squid gains SPNEGO auth via the
existing OIDC token. Human operators authenticate via the standard
OIDC flow; service accounts continue with source-IP. Migration is
config-only — no protocol change, no Kanidm-specific anything.
**Strict adherence to standard OIDC claims (`sub`, `email`, `groups`)
means we are never coupled to any one IdP.**

**Explicit dependency.** Block 17 (egress proxy) does NOT block on
Block 3. v1 ships with source-IP auth; SPNEGO is Phase 7 within Block
17, gated only on whatever OIDC IdP is in production at the time.

### 5.2 Proxy-bypass allowlist (critical — prevents bootstrap deadlock)

The Tier 1 firewall has a hardcoded list of destinations that bypass
Tier 2/3/4 and go direct. Without this, the proxy itself cannot fetch
its own updates, Prometheus can't scrape, Netbird management can't
phone home, and we get a chicken-and-egg failure at every boot.

**Bypass destinations** (examples, full list in Ansible role):

| Destination | Reason |
|---|---|
| Mesh CIDR (100.64.0.0/10) | Internal — never goes through proxy |
| Loki / Prometheus remote_write peers | Observability shipping |
| Netbird signal + management | Mesh control plane |
| NTP upstream servers (post-Block 12) | Time hygiene — required before proxy can trust certs |
| OCSP / CRL distribution points (hardcoded IPs) | Cert validation — needed before Squid can serve TLS |

**Bootstrap order** (enforced by systemd unit dependencies):

1. Tier 0 DNS rules
2. Tier 1 firewall with bypass list
3. NTP sync (must succeed — otherwise TLS validation fails)
4. Squid / dante start
5. All other mgmt-plane services

**Audit.** The bypass list is itself a two-of-N-approved file. Adding
a bypass is treated with the same scrutiny as a Tier 4 allowlist
entry.

### 5.3 HA model

**Active-passive on Router-A / Router-B.** Squid + dante run on both
mgmt-hosts. Keepalived manages a VIP (virtual IP) that clients use as
their proxy endpoint. Failover is automatic on node or service
failure. Same pattern as PostgreSQL VIP failover; see
`ha-failover.md`.

**Fail-closed when both routers die.** No emergency direct-internet
fallback. Island-Mode philosophy: a partitioned site already survives
WAN loss; egress loss is in-scope and survivable. Emergency human
override requires physical access to a mgmt-host and `systemctl stop
egress-firewall.service` — logged, not hidden.

### 5.4 Audit pipeline

**Already-deployed infrastructure used:**

- Loki (log shipping) — add promtail tailing Squid + dante + nftables logs
- Prometheus — scrape Squid's `/metrics` (via squid-exporter) and dante's metrics sidecar
- Alertmanager — new rules:
  - `EgressProxyDown` — either Squid or dante on both mgmt-hosts
  - `EgressDenyRateHigh` — >10/min denied requests from a single peer (possible compromise)
  - `EgressAllowlistExpiringSoon` — any entry within 7 days of expiry
  - `EgressAllowlistExpired` — any entry past expiry and still in effect
  - `EgressUnexpectedDestination` — first-seen SNI not in allowlist history (human review trigger)

**Per-site retention, remote audit access — not federation.** Each
site retains its own logs locally. There is **no** cross-site log
replication, no federated query layer, no active-active audit store.
This is a deliberate simplification: the HA/consistency complexity we
accept for PostgreSQL (Spock multi-master) is not warranted for audit
logs, and cross-site log federation was causing us to re-invent
coordination machinery that this design explicitly avoids.

How remote audit works instead:

- **Auditor is a mesh peer.** Any authorized workstation that is
  enrolled in the mesh can reach any site's Loki query API and
  Prometheus UI directly over WireGuard.
- **Read-only audit account** at each site's Loki / Prometheus —
  separate from operator access, logged, and itself auditable.
- **Retention per site:** 90 days hot in local Loki; optional cold
  archive to site-local disk (extending to ~1 year) matching CIS 8.10
  guidance. No cloud storage in the audit path unless the site
  operator explicitly opts in.
- **Incident response across sites** is a human process: the
  investigator queries each site's Loki in turn. Slower than a
  federated query, but correctness-preserving and does not require
  running an additional distributed system.

If a requirement later emerges for real-time cross-site correlation
(e.g., "detect coordinated exfil across 5 sites within 60s"), that
triggers §9 revisit — not a Block 17 blocker.

### 5.6 Audit log integrity (dependency — closes gap flagged 2026-04-24)

The audit guarantees in §5.4 are circular if the logs can be tampered
with by the same host that generated them. Squid + Loki + nftables
all run on the mgmt-host; a compromise of that host lets the attacker
edit their own audit trail. This sub-section is foundational work for
the whole mgmt-plane, not just egress — but egress is the forcing
function, so the first concrete implementation lands here.

**Principle.** A minimal set of high-integrity events is duplicated to
an append-only store, off the originating host, at or near the moment
of occurrence. Loss of the originating host destroys operational
context; it does **not** destroy the audit record.

**In-scope event classes.**

| Event class | Source | Why in-scope |
|---|---|---|
| Egress decisions (Tier 0/1/2/3/4) | nftables, Squid, dante | This design's primary audit surface |
| Auth + sudo on mgmt-host | auditd, journald | PAM gap (§gap #3) starter |
| Config changes to egress policy | git commit + Ansible deploy | Approval trail |
| Break-glass exceptions | allowlist `break_glass:` blocks | High-risk path |
| Privilege escalation attempts | auditd | Compromise indicator |
| Enrollment / de-enrollment of peers | Netbird management API | Perimeter change events |

**Out of scope (v1).** Application-level logs (efforts, gatewatch,
airdefenders, Zenoh samples), Traefik access logs, routine Prometheus
scrapes. These stay in local Loki with normal retention.

**Destination.** Per-site append-only store, **not** cross-site
federation (matching §5.4 — per-site retention is the sovereignty
posture). Options ranked by preference:

1. **S3-compatible object storage with Object Lock in Compliance
   mode** — local MinIO on a dedicated boundary node per site, or
   site-approved cloud S3 where sovereignty allows. Object Lock
   prevents deletion or modification for the retention period, even
   by the root account of the storage system.
2. **Hardware-backed WORM** (write-once NVMe or append-only partition
   with fsverity) — considered for a pilot; tracked as a
   future successor once the hardware attestation story matures.
3. **Remote journald forwarding** to a log-only host with no
   operator shell access — acceptable interim if neither (1) nor (2)
   is available at deploy time.

v1 picks option 1 with site-local MinIO on the mgmt-host's second NVMe
(already part of the storage plan — see [`os-baseline-and-luks.md`](os-baseline-and-luks.md)
three-tier storage). Object Lock retention: 1 year. Keys to MinIO are
held by a separate service account not shared with the Squid /
Ansible / operator accounts.

**Forwarding mechanism.**

- Promtail on the mgmt-host reads local log sources, applies an
  event-class filter, and ships to (a) local Loki (normal retention)
  and (b) the high-integrity bucket with its Object-Lock write policy.
- The high-integrity bucket lives on the **other** mgmt-host
  (Router-B's disk for Router-A's logs, and vice versa) — this gives
  us off-originating-host durability without any new site.
- Integrity hash: each log chunk is hashed + the hash itself is
  written to a separate immutable ledger (a simple append-only text
  file on a third disk is acceptable at v1 — the point is out-of-band
  verification that the chunk hasn't been replaced).

**Audit access.** Read-only audit account (§5.4) also grants read
access to the high-integrity bucket. Tamper detection runs as a
daily CI job that re-hashes recent chunks and compares to the ledger;
mismatch fires `EgressAuditIntegrityAlert`.

**What this does not protect against.** Root-on-both-mgmt-hosts
simultaneously + MinIO service account compromise + ledger tampering
is a total compromise of the site. We accept this — at that point the
site is owned, and the off-site mesh peers + git history + the other
sites' reports of having received traffic from this site provide the
reconstruction surface. No design defeats a fully compromised site;
the goal is to make the attacker's job noisy, traceable, and slow.

**Complexity note.** This is new infrastructure (MinIO + Object Lock
config + integrity ledger + tamper-detection CI). Listed explicitly
here because §8 promised "zero net-new services" and this adds one.
The trade-off: an egress-proxy design without audit integrity has
circular audit guarantees, which is worse than the complexity cost.
MinIO is itself a proven, STIG-benchmarked option (FIPS mode
available) — so we stay within the "proven approaches" principle even
while adding a service.

### 5.5 Enforcement validation (CI)

**Weekly smoketest job** (`ops/smoketest/egress.yaml`):

1. From each site's `ops-test` peer, attempt to reach a known-blocked
   SNI. Expect `TCP_DENIED` + log entry.
2. From the same peer, attempt to reach a known-allowed SNI
   (`example.com` — always-allowed canary). Expect `TCP_TUNNEL` + log entry.
3. Attempt to resolve via DoH (`1.1.1.1/dns-query`). Expect
   connection reset at Tier 1.
4. Attempt UDP/443 to `www.google.com`. Expect drop at Tier 1.
5. Verify all four log entries are present in Loki.

Failure of any step opens an incident + alerts the on-call rotation.

---

## 6. Approval + exception workflow

**Dual-path approval via the per-site policy engine — sovereign,
Spock-replicated, Island-Mode-correct.** A normal path with an
assigned role-holder + 24h SLA; a fast path of two-of-N peer approval
as the urgent / SLA-breach release valve. Same engine handles both
paths and batch requests. Replaces an earlier draft that put the
approval workflow on GitHub PRs — that approach was rejected because
it violated Island Mode (a partitioned site cannot reach GitHub) and
introduced an external dependency for the most security-critical
config-change operation in the system. Rejection rationale recorded
in §7.

### 6.1 Architecture overview

```text
  Operator (Web UI or CLI)
         │
         ▼ submit
  ┌────────────────────────────────────────────────┐
  │ Policy Engine Service (db-policy-engine, Go)   │
  │  • OIDC auth (Dex v1 — IdP-abstract)           │
  │  • REST API + small SPA for human review       │
  │  • Validates two-of-N or role+SLA semantics    │
  │  • Pluggable notification adapters             │
  └────────────────────────────────────────────────┘
         │
         ▼ writes to Postgres
  ┌────────────────────────────────────────────────┐
  │ egress_policy + egress_approval (Spock-replic.)│
  │  • CRDT-style conflict resolution              │
  │  • State machine: pending → active → expired   │
  └────────────────────────────────────────────────┘
         │
         ▼ on activation
  Ansible coordinated-upgrade.yml → Squid / dante / nftables config
         │
         ▼
  Integrity tier (Block 17D MinIO) gets every event
```

The policy engine runs on every mgmt-host (active-passive behind the
same keepalived VIP as Squid). Each site has its own approval queue
visible to its own operators via the local web UI; cross-site
visibility happens via Spock replication of the underlying tables —
no operator ever needs to reach a different site to approve a request,
but they may.

### 6.2 The two paths

**Normal path (role-holder + 24h SLA).** The default. Most allowlist
changes are not urgent and benefit from the deliberation of a single
accountable role-holder.

1. Operator submits via Web UI or CLI: tier, payload (sni / cidr /
   port / protocol), reason, owner, expires, priority=normal.
2. Engine writes the request to `egress_policy` (state=pending) +
   notifies the role-holder roster via every active channel
   (email + Matrix room + isitup widget).
3. Role-holder reviews via the Web UI, approves or rejects. Single
   approval is sufficient on the normal path; the role carries the
   formal responsibility.
4. SLA timer = submission timestamp + 24h. Warning notification at
   T+22h. Breach alert (`PolicyRequestSLABreached`) at T+24h, AND the
   request automatically becomes eligible for the fast path (§6.3).
5. On approval, engine flips state to active; Ansible
   coordinated-upgrade picks up the change within the next run.
6. All events written to integrity tier.

**Fast path (two-of-N peer approval).** Release valve for urgent
needs OR auto-engaged on SLA breach.

1. Operator submits with priority=urgent (or the request escalates
   automatically after SLA breach), reason explicitly states why
   urgency or what blocked the role-holder.
2. Any two distinct operators (≠ requestor) approve via Web UI or
   CLI. Approver identity is OIDC `sub` claim from their session.
3. On the second approval, engine activates the policy. No SLA
   timer — fast path is, by definition, fast.
4. Post-action review required within 7 days: either (a) the
   role-holder retroactively confirms the change in the audit
   record, or (b) an explicit "this should not have used the fast
   path" note is logged and a process-improvement issue opened.

**Batch.** Submission carries multiple entries (YAML payload).
Reviewer sees them as a per-row checklist on the same screen; can
approve / reject each individually or batch-approve all. Each entry
generates its own `egress_approval` row. Mixed normal/urgent in a
single batch is supported — each row carries its own priority.

### 6.3 SLA semantics + escalation

| Event | When |
|---|---|
| T0 | `egress_policy.created_at` (server-side timestamp at submission) |
| Warning ping | T0 + 22h, sent to role-holder roster on all channels |
| Breach + fast-path eligibility | T0 + 24h, fires `PolicyRequestSLABreached`, sets `fast_path_eligible=true` |
| Force-close as stale | T0 + 7 days with no decision, requestor notified, request archived |

SLA timer measured from submission timestamp (not from notification
delivery) — keeps the rule simple, avoids "the email bounced so the
SLA didn't start" excuses. If a notification fails to deliver, that's
a separate alert (`PolicyEngineNotificationFailure`); the SLA still
ticks.

### 6.4 Role roster

**v1 baseline.** Single named operator per site as the minimum
viable. Strongly preferred: 2-3 named per site so vacation / illness
doesn't block the normal path. Identifies are OIDC `sub` claims with
the `egress-policy-approver` group.

**Cross-site approval is allowed.** A role-holder at one site can
approve another site's request — useful when sites are on opposite TZs and
the local role-holder is asleep. The Web UI shows `requestor_site` so
the approver has context. Cross-site approvals carry a `cross_site=
true` flag in the audit record so the pattern can be reviewed
separately from same-site approvals.

**Roster source of truth.** `ops/policy-engine/approvers.yaml`,
two-of-N protected via the same engine (bootstrap chicken-and-egg
handled by initial-deployment override + audit log).

### 6.5 Conflict resolution under partition

Spock multi-master replication means two sites can independently
commit policy changes during partition. When they reconnect:

- **Same `(tier, key)` tuple** (e.g., both sites approve adding
  `*.partner-x.com` to Tier 2): latest `expires_at` wins, vector
  clock records both; alert `PolicyConflictResolved` for human
  review.
- **Conflicting deny vs allow for same tuple**: deny wins (fail-
  closed); alert `PolicyConflictDenied` for human review.
- **Different tuples**: union, no conflict.
- **Approval count under partition**: each site enforces its own
  count locally. A single role-holder approval at a partitioned site
  applies locally; on reconvergence, the approval record propagates
  but the policy doesn't re-trigger Ansible at peer sites (it's
  already there).

Vector clock per row plus a small daemon (`policy-merger`) handles
post-reconnection reconciliation. Implemented as a Postgres trigger
combined with a Go service that reacts to `egress_policy_conflict` rows.

### 6.6 Identity + signing (OIDC-abstract)

Operator identity is a standard OIDC subject (`sub` claim) from any
configured IdP. The engine uses `coreos/go-oidc` against a discovery
URL — no IdP-specific code anywhere.

| Component | v1 (today) | v2 (after Block 3A) |
|---|---|---|
| OIDC IdP | **Dex** per-site (already deployed for Block 3C) | Kanidm via Dex broker — config change only |
| Web UI auth | OIDC implicit flow, session cookie | Same |
| CLI auth | `db login` triggers device-code flow → token cached locally | Same |
| Approval signature | Server-side HMAC over (policy_id, approver_sub, ts) using the engine's per-site key, written to integrity tier | Same — or upgraded to per-operator Ed25519 if compliance demands non-repudiation |

**Why Dex specifically.** It's already in the architecture (Block
3C), it's a federating broker (we can swap the upstream IdP without
touching the policy engine), and it has a Chainguard image. Dex is
the configuration choice, not a coupling — the policy engine's only
contract is "give me a valid OIDC discovery URL." If we ever
abandon Dex for Authentik / Keycloak / Zitadel / Authelia / external
provider, the change is one config field.

**No Kanidm SDK or vendor lock-in.** All claims used are standard:
`sub`, `email`, `groups`, `aud`, `iss`. JWKS rotation handled by the
generic library.

### 6.7 Notifications

Three surfaces fire per request, three more on state changes (warn,
breach, activate). All channels are pluggable adapters in the engine:

| Channel | Mechanism | Sovereignty |
|---|---|---|
| **Email** | SMTP, per-site relay config | Per-site outbound (no shared SaaS) |
| **Matrix** | Per-site tuwunel homeserver per [ADR 0835](../adr/0835-secure-messaging-substrate-tuwunel.md), room `#policy-alerts:<site>.netbird.legacy.example`, posts via standard `_matrix/client/v3` API | Fully sovereign (Matrix is federation-aware but each site runs its own homeserver; federation is opt-in and controlled over wg-cp0) |
| **Dashboard widget** | Tile on `isitup.netbird.legacy.example` showing pending count per site, click-through to per-site queue | Already mesh-internal, no new infra |

All three fire — never any single channel alone — so a missed email
or a Matrix outage doesn't silence the request. Breach alerts also
hit Alertmanager (the same channel as `EgressProxyDown` etc.) so
on-call sees them.

**Adapter selection per site is config-only.** A site without
outbound email simply disables the email adapter; a site without a
Matrix homeserver disables the Matrix adapter. Dashboard widget is
always on.

### 6.8 What this workflow does NOT need

- No GitHub dependency (the previous design's flaw)
- No Vault Control Groups
- No external ticketing integration (the engine is itself the
  ticketing system; integration with external trackers is optional
  via ticket URL field)
- No bespoke audit log — events ride the integrity tier (Block 17D)

Cost vs the rejected GitHub design: ~1 week of focused engineering
to land the policy engine + adapters. This cost is paid in service
of Island-Mode correctness, sovereignty, and architectural coherence
with Spock-everywhere. Worth it.

### 6.9 Operator artifacts shipped with the engine

- `db-policy-engine` Go service binary + Chainguard container image
- `db` CLI subcommand (`db egress-policy submit / approve / list / show`)
- React/TypeScript SPA mounted at `policy.<site>.netbird.legacy.example`
- Postgres migrations for `egress_policy` + `egress_approval` +
  `egress_request_event` tables
- Ansible role `policy-engine` (deploys service + SPA + Postgres
  migrations + Spock subscription on first run)
- Documentation: this section + the operator runbook
  ([`../operations/egress-runbook.md`](../operations/egress-runbook.md))

---

## 7. What we reject and why

| Option | Why rejected |
|---|---|
| **No egress control** | Unacceptable for Phase 3 production — the back-door this design closes. |
| **Single-layer firewall only** | Doesn't give per-request audit; rule count grows unbounded; IP-based rules break when CDNs rotate. |
| **Single-layer proxy only** | No default-deny at the gateway means peers can bypass the proxy entirely (trivially via direct TCP). |
| **TLS MITM** | Requires custom CA on every peer; breaks cert pinning; doesn't pay for itself at v1 scale. Revisit if per-URL policy becomes a compliance requirement. |
| **Transparent proxy** | Requires MITM to handle TLS. Fails the no-MITM goal. Also brittle for non-HTTP traffic. |
| **Commercial SWG** (Zscaler / Prisma / Cloudflare Gateway) | Violates sovereignty posture; creates external dependency on SaaS for mesh-internal policy decisions. |
| **Envoy as primary proxy** | Viable long-term — modern, better metrics, gRPC-native — but no published STIG benchmark and larger config surface. Tracked as successor candidate, same pattern as Docker Compose → Podman+Quadlet. |
| **gost for SOCKS5 as v1** | More modern and multi-protocol than dante, but less STIG precedent. Tracked as successor candidate. |
| **GitHub PR + CODEOWNERS for approval** *(initial design, rejected 2026-04-24 on review)* | Violates Island Mode — a partitioned site cannot reach GitHub to open or approve PRs. Introduces external SaaS dependency on the most security-critical config-change operation in the system. Inconsistent with Spock-everywhere posture for replicated state. Replaced by §6 per-site policy engine on Spock-replicated tables. |
| **Vault Control Groups for approval** | Adds HashiCorp Vault to the stack solely for approval semantics; we have no other Vault need. Postgres + Spock already in the stack — reuse beats re-architect. |
| **Custom approval UI without OIDC** | Would require a separate user database; per-site role rosters drift; Block 3 OIDC roadmap argues against. OIDC-abstract policy engine reuses the IdP we're already deploying for everything else. |
| **Centralized (not per-site) proxy** | Breaks island-mode — a partitioned site would lose all egress. Non-negotiable. |
| **User-level auth (Kanidm) in v1** | Chicken-and-egg: Kanidm isn't deployed until Block 3. Source-IP auth is sufficient for v1 and trivially upgraded later. |
| **Self-written egress daemon** | No. Every authoritative source (NIST, DISA, CIS, BeyondCorp) uses published forward-proxy software. Inventing our own would be the worst possible compliance story. |

---

## 8. Complexity mitigation self-audit

The design intentionally applies three engineering principles to keep
complexity bounded. This section exists so a future reviewer — or a
future us — can check that the principles were actually followed and
not just invoked.

### 8.1 Proven approaches — no invented systems

| Component | Why it's proven |
|---|---|
| Squid | 30+ yr track record, DISA STIG benchmark published, FIPS-compatible build, Chainguard image exists |
| dante | Long ops history in regulated-enterprise egress, stable config surface |
| nftables / UFW | Already deployed on every mgmt-host (Block 1A) |
| CoreDNS / dnsmasq | Already deployed (Block 13A migration) |
| keepalived | Already used for PostgreSQL VIP (HA block) |
| systemd-timer for expiry | Already used for setup-key rotation |
| OIDC (Dex per-site, IdP-abstract) | Standard claims `sub` / `email` / `groups`; switching IdP is one config field — no vendor lock |
| Spock multi-master replication | Already in use for postgres; reusing it for the policy engine is the canonical sovereign-replication choice |
| Postgres triggers + CRDT-style merge | Standard pattern; well-understood failure modes |
| Prometheus + Alertmanager + Loki | Already deployed — this design adds scrape targets + alert rules, not new infra |
| Promtail sidecar pattern | Already used for Traefik + postgres logs |

**Net:** zero net-new services. Every component in this design either
already runs in the fleet or has an established STIG/compliance track.

### 8.2 Isolated concerns

| Concern | Component | Replaceable without touching |
|---|---|---|
| DNS egress | Tier 0 nftables + mesh resolver | Tier 2/3/4 |
| Default-deny | Tier 1 nftables | Tier 2/3/4 |
| HTTP audit | Tier 2 Squid | Tier 0/1/3/4 — swap for Envoy |
| Non-HTTP audit | Tier 3 dante | Tier 0/1/2/4 — swap for gost |
| CIDR exception | Tier 4 nftables | Tier 0/1/2/3 |
| Approval | GitHub (out-of-band) | All runtime tiers |
| Audit shipping | Loki/Promtail | All runtime tiers |

Each tier is a horizontal slice with well-defined inputs/outputs. A
compromise or outage in one tier does not cascade.

### 8.3 Graceful failures + escape hatches

| Failure mode | Behavior |
|---|---|
| Squid crashes on Router-A | Keepalived fails VIP to Router-B; Squid on B serves traffic |
| Squid crashes on both routers | Tier 1 default-deny blocks HTTP egress — site loses internet but mesh, observability, backups, NTP continue to work (bypass list). Matches island-mode. |
| Allowlist PR is wrong | CI rejects before merge (syntax + duplicate + bypass cross-check). |
| Deployed allowlist is wrong | `coordinated-upgrade.yml` canaries one site first; smoke test catches regressions before fleet rollout. |
| Operator needs urgent exception at 3am | Break-glass PR path, 24h max, two-of-N approval, auto-expiry. Honest escape hatch — no one is forced to edit iptables on a live host. |
| Ops team loses GitHub access | Local admin on mgmt-host can `systemctl stop egress-firewall.service` — logged, audited after the fact. We never lock ourselves out of our own system. |
| Role-holder + two-of-N approvers all unreachable | Roster of 1 minimum / 2-3 preferred per site, plus cross-site approval allowed (one site approves another's). If still blocked: local-admin escape hatch with post-hoc audit. |

### 8.4 Acknowledged risks (not hidden)

1. **Initial allowlist enumeration** — ~20-40 entries, must be
   CI-validated. Real work, bounded, one-time. Phase 1 "monitor mode"
   derives the list from observed traffic.
2. **Auth migration path** — source-IP for v1, SPNEGO in Phase 5
   (depends on Block 3). Explicit dependency, not assumed-free.
3. **SNI-only visibility** — HTTPS audit has no method/path/body.
   Documented in the runbook so incident responders aren't surprised.
4. **DoH evasion via CDN fronting** — SNI allowlist doesn't catch
   this perfectly. Accepted at v1; revisit triggers in §9.
5. **Chainguard coverage for Tier 3.** Squid has a known Chainguard
   path; dante and squid-exporter likely do not (to be confirmed
   during procurement). If not in the catalog at v1 ship, we either
   (a) build from `cgr.dev/chainguard/wolfi-base`, or (b) pivot Tier
   3 to gost if gost is available. Tracked in
   `../project/chainguard-procurement.md`
   as a Block 17 procurement dependency.

---

## 9. Trigger conditions for revisit

Revisit this design if any of the following fire.

| Trigger | Revisit outcome |
|---|---|
| Per-URL HTTPS policy becomes a compliance requirement (e.g., a partner demands "allow github.com/org but not github.com/other-org") | Evaluate MITM with custom CA — separate ATO decision |
| Site count grows past 10 and Squid config drift becomes ops burden | Evaluate centralized policy engine (Open Policy Agent) fronting per-site Squid instances |
| Host count per site grows past ~20 and source-IP auth becomes unwieldy | Accelerate SPNEGO migration (or evaluate mTLS client-cert auth) |
| First real DoH-fronted exfiltration incident | Harder Tier 0: switch to DNS-sinkhole model with RPZ (response policy zones) |
| Squid STIG benchmark is retired or falls behind current Squid release | Evaluate Envoy as replacement — successor already tracked |
| Team grows past 5 ops people | Evaluate whether two-of-N should become three-of-N for some change classes |
| Kanidm (Block 3) lands in production | Execute Phase 5 SPNEGO migration (within Block 17, not a new block) |

---

## 10. Related artifacts

- `../project/egress-proxy-implementation.md` — phased rollout tracker
- `../project/implementation-plan.md` — Block 17
- [`../operations/egress-runbook.md`](../operations/egress-runbook.md) — ops procedures (exception request, break-glass, investigation)
- `os-supply-chain-security.md` — Block 13, complementary (image supply chain)
- `ha-multi-site.md` — island-mode + HA patterns this design inherits from
- [`ntp-strategy.md`](ntp-strategy.md) — NTP is a proxy-bypass dependency
- `../operations/bridge-peering.md` — partner egress (separate concern)
- NIST SP 800-207 — Zero Trust Architecture (§3.4 PEP at egress)
- CISA Zero Trust Maturity Model v2.0 — Network pillar
- DISA SRG-NET-000132 — Forward Proxy SRG
- CIS Controls v8 — 13.5 and 13.10
- Google BeyondCorp egress papers
