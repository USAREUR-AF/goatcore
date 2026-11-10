# Phoenix Architecture — Scheduled Full-System Teardown and Rebuild

**Status:** v1 design exploration, 2026-04-24. Not yet committed to implementation. This document frames the concept, surveys industry precedent, identifies the load-bearing design decisions, and poses the questions that must be answered before the block is scoped. Commit decision belongs to project lead after this design converges.

**Position.** We want to make full-system teardown + rebuild a routine engineering practice, not a crisis operation. The discipline this forces (bootstrap-from-zero in hours, not weeks; identity-free peer rediscovery; no hidden state) is itself the payoff. Periodic rebuilds also kick out any squatter or drift — anything that survived a rebuild cycle either reflects deliberate design or is a bug we haven't noticed.

**Framing refined 2026-04-24.** Infrastructure rebuild is the easy half — IaC already solves it. **The hard half is the accumulated cruft of peer join**: bespoke auth setup, hardcoded discovery, per-peer integration modules, config files that mediate between vendor assumptions and ours, operational procedures captured only in someone's head. Every one of these is a dependency the rebuild must re-establish. This document centers the hard half: making every peer, service, and integration **phoenix-native** so rebuild becomes trivial by construction.

**Statefulness is not the problem.** Rich in-memory state, hot caches, open sessions, per-session optimization — all fine during runtime. The recycle is what forces every piece of that state to be *regeneratable* from four inputs: persistent cryptographic identity (on TPM / YubiKey / offline CA), declarative role announcement (the peer tells us what it is), capability-based references (not hardcoded endpoints), and supply-chain-delivered code (from Chainguard, not hand-edited). Stateful-while-running is a feature we keep; the rebuild just wipes accumulated cruft and re-derives runtime state from first principles.

**Decoupling added 2026-04-24 (project lead's framing).** Two distinct workstreams, separable in commitment:

1. **Engineering discipline: phoenix-native peers.** Required. Measurable via the cruft test. A peer meets the four properties (2A.3) or it doesn't. This is engineering work with definite acceptance criteria, incremental adoption, and value independent of whether rebuilds are ever scheduled. A phoenix-native peer is more operationally resilient on every axis — easier to deploy, easier to troubleshoot, easier to replace, easier to scale — not just more rebuild-friendly.

2. **Operational practice: scheduled rebuild cadence.** Optional, operator-choice, site-by-site. A mature production site may run stably for quarters. A new site in bring-up may rebuild monthly. A post-incident site may rebuild on demand. This is policy, not architecture. Operators decide; engineers enable.

The discipline is strictly stronger than the practice. If peers are phoenix-native, rebuild cadence becomes a choice. If peers are not phoenix-native, rebuild is impossible regardless of policy. Engineering work therefore targets the discipline; the practice follows from it if and when operators want it.

## 1. Motivation — two distinct payoffs

### 1.1. Security payoff

A system that is periodically torn down and rebuilt:

- **Kicks out squatters.** Any adversary that has established persistence in a long-lived host, mesh peer, or service is removed at teardown. The assumed-clean post-rebuild state is actually clean, not just hoped-clean. This is the strongest possible defense against low-and-slow persistent threats that evade detection.
- **Forces re-attestation.** Every device re-enrolls; every capability token is freshly issued. Stale credentials, orphaned service accounts, and over-privileged identities that accumulated through ad-hoc operations are all flushed.
- **Limits credential-theft blast radius.** An attacker who stole a credential between rebuild N and rebuild N+1 loses it at rebuild N+1. The blast window is bounded by rebuild cadence, not by the credential's nominal TTL.
- **Exercises incident-response muscle.** A planned teardown is operationally similar to a forced rebuild after compromise. Routine practice makes the actual-incident case cheap.

### 1.2. Operational payoff

The discipline required to survive routine teardowns is exactly the discipline required for:

- **Rapid deployment to new sites.** If you can rebuild a site in six hours every quarter, you can stand up any future site in six hours first time. The deployment runbook is exercised, not theoretical.
- **Migration safety.** "We can't upgrade Postgres from 16 to 17 because the data is load-bearing and nobody remembers how it was set up" is a pattern that cannot exist in a phoenix system. Every dependency, every config, every credential is documented-by-necessity.
- **Onboarding new engineers.** A system that rebuilds itself is one a new engineer can understand. No "ask Dave, he's the only one who knows why that host has that config."
- **Supply chain validation.** Every rebuild re-pulls images, re-verifies signatures, re-installs packages. Tampering or drift at the supply layer surfaces at the next rebuild.
- **Cost of novel tech is paid upfront, not at crisis time.** If we commit to phoenix architecture before Phase 3 production, we do the work when there's time. If we put it off, we face migration risk at every upgrade.

These are not two separate benefits of one design — they are the same benefit, one discipline, expressed at two layers.

## 2. Industry precedent — what exists publicly

### 2.1. Mature pieces

The concept decomposes into well-known patterns:

| Pattern | Origin | What it does | Fit for us |
|---|---|---|---|
| Phoenix server | Martin Fowler, 2013 — [martinfowler.com/bliki/PhoenixServer.html](https://martinfowler.com/bliki/PhoenixServer.html) | Per-host periodic rebuild | Foundational concept |
| Cattle not pets | Gavin McCance, CERN, 2012 | Treat servers as replaceable livestock, not pets | Philosophical foundation |
| Immutable infrastructure | HashiCorp + Kief Morris, *Infrastructure as Code* (O'Reilly 2016, 2020) | Every change = new provision, never mutate | Our IaC already follows this |
| Chaos engineering (instance-level) | Netflix Chaos Monkey, 2011 | Random instance kills in production | Partial fit — kills hosts, not identity/network |
| Chaos engineering (region-level) | Netflix Chaos Kong, Chaos Gorilla | Kills AZs and regions, forces failover | Partial fit — higher-order failure than instance |
| Red-black / blue-green deploys | Spinnaker (Netflix) | Full new application stack, cut over, tear down old | Application layer only |
| Game days / DR exercises | AWS Well-Architected Reliability Pillar | Scheduled simulated disasters | Cultural parallel; not continuous |
| Google DiRT | Google SRE (Disaster Recovery Testing) | Annual region-loss simulation | Highest-discipline public example at scale |
| Workload identity re-issuance | SPIFFE / SPIRE | Cryptographic identity that re-issues on restart | Direct enabler |
| Content-addressable peer discovery | IPFS, libp2p, BitTorrent DHT | Peers find each other by public key, not by IP | Direct enabler for identity-free rediscovery |
| Gossip mesh | HashiCorp Serf, Consul gossip | Peers discover each other via gossip from seed nodes | Direct enabler |
| Ephemeral enrollment keys | Tailscale auth keys, Netbird setup keys | Short-TTL, auto-expiring enrollment tokens | We already use this |

### 2.2. Where public precedent runs out

**Netflix, Google, Stripe, Shopify** — none of them tear down their network overlay, identity plane, or trust root as regular practice. They chaos-test *within* a stable foundation. The foundation itself is considered load-bearing and is not phoenixed.

Specifically:

- **Netflix Chaos Monkey / Kong / Gorilla** kills *instances* and *regions*. Does NOT tear down the AWS account, the VPC overlay, the IAM trust root, or Spinnaker itself.
- **Google DiRT** simulates data center loss. Does NOT rebuild Borg, Spanner trust, or internal PKI.
- **AWS game days** are application-scoped. AWS itself is stable.

### 2.3. Closest public analogs

- **CERN OpenStack practice** — treats compute nodes as replaceable livestock and has published on the cultural / operational model. Does not extend to full identity-plane teardown. Closest cultural match.
- **Platform One / Iron Bank** — "build from scratch every time" as an image-level philosophy. Partial match at the supply-chain layer; does not extend to network overlay.
- **NSA / Red Team University Cyber Range exercises** — DO tear down and rebuild full networks. But these are *training environments*, not production.
- **Unikernel philosophies** (MirageOS) — burn-and-rebuild at VM level. Never reached mainstream.

### 2.4. Honest positioning

Phoenix engineering the whole system — including identity and network overlay — is somewhere between **novel in combination** and **quietly done by a few high-discipline shops**. There is no public Netflix-scale example to point to.

**This is not a gap. This is a publishable story.** If we do this and publish it, we become the reference example for sovereign phoenix architecture. This aligns with the broader goat posture of doing things right and telling the industry how we did them.

## 2A. The phoenix-native peer pattern — the actual hard problem (centered 2026-04-24)

This section was added after v1 drafting when project lead identified that the existing §3-§5 framing buried the central insight. This section names the insight, defines the peer-level design pattern that falls out of it, and positions the rest of the document as implementation around this core.

### 2A.1. What cruft is, and why it accumulates

In real systems, every peer that joins a network typically brings with it:

- **Bespoke auth setup.** API keys placed in specific filesystem paths, shared secrets in specific env vars, certs loaded into vendor-specific stores. Each peer has its own convention.
- **Bespoke discovery.** Hardcoded IPs in config, hand-crafted DNS entries, per-peer registry lines. When the peer rebuilds, each of these must be re-established by hand or by bespoke automation.
- **Bespoke integration modules.** Shim services, adapter containers, vendor-specific sidecars that mediate between the vendor's assumptions and ours. One per integration.
- **Bespoke config files** mediating between vendor protocol peculiarities and our architecture. Different file, different format, different owner per integration.
- **Bespoke operational procedures.** Restart order, health check endpoints, upgrade dance. Usually captured only in someone's head or a runbook that rots.

Each of these is a dependency. Each is cruft. Each is a rebuild-time failure point. Crucially, **each is invisible when the system is running**. Cruft only surfaces when you try to rebuild.

Infrastructure rebuild is easy — IaC handles it. **Peer-join cruft is where scheduled rebuilds historically fail**, and is the reason most organizations stop scheduling them.

### 2A.2. Stateful-while-running is a feature, not a problem

The phoenix discipline does NOT require services to be stateless. Rich in-memory state, hot caches, open sessions, per-session optimization, cached routing decisions, learned peer preferences — **all fine during runtime**. We keep the performance and operational benefits of statefulness.

The rebuild is what forces every piece of that state to be *regeneratable from four inputs only*:

1. **Persistent cryptographic identity** — on TPM, YubiKey, offline CA. Survives every rebuild short of hardware replacement or tier-4 ceremonial rekey.
2. **Declarative role announcement** — the peer announces what it is, what group it belongs to, what it publishes, what it consumes. Not assigned externally.
3. **Capability-based references** — the peer discovers its collaborators through a rendezvous substrate that speaks in *semantic* addresses (topics, capabilities), not hardcoded endpoints.
4. **Supply-chain-delivered code** — from Chainguard, verified at pull, never hand-edited on the host.

Anything derivable from those four is phoenix-compatible. Anything else is cruft.

### 2A.3. A peer is "phoenix-native" if

Given ONLY its persistent identity + a pointer to the network's rendezvous layer, a phoenix-native peer can:

1. **Attest who it is.** Cryptographic proof of identity — signed by a long-term key bound to secure hardware.
2. **Declare its role and capabilities.** "I am a thermal sensor in group A. I publish to topic `telemetry/site/<site>/sensor/thermal/<my-id>`. I consume `config/sensor/thermal/*`." The declaration is the config.
3. **Discover its collaborators via the rendezvous layer**, not via bespoke config. If the peer needs a database, it asks the layer for one by capability, not by IP.
4. **Begin operating** without any per-peer state on the infrastructure side. The management plane has zero per-peer cruft to recreate at rebuild.
5. **Verify the identity of other peers** before integrating with them — not just self-attest, but cryptographically check that an incoming peer's identity is signed by a trust root the receiving peer holds locally (the offline CA per ADR 0406). Self-attestation alone is insufficient against compromise of the management authority; mutual attestation closes the gap that mgmt-host compromise would otherwise open. Phase 3 deferred per ADR 0315 (Block 22H); the principle belongs in §2A.3 even though the implementation lands later.

Zero bespoke glue. Zero per-peer config on the management plane. Zero integration modules. The peer's declaration IS the config — *and the peer's verification of every other peer's declaration is its own discipline, not a property of trusting the central authority.*

### 2A.4. Zenoh is the natural substrate

We already have Zenoh in the architecture as a data fabric (Block 11, Phase 2D, `zenoh-data-fabric.md`). Zenoh's actual pattern is exactly the phoenix-native peer interaction pattern at the data layer:

- **Key expressions as declarative capability addresses.** A sensor publishes to `telemetry/site/<site>/sensor/thermal/*`; consumers subscribe by pattern.
- **No endpoint configuration.** The Zenoh router handles routing between publishers and subscribers.
- **Late binding.** A publisher doesn't need to know who's listening; a subscriber doesn't need to know who's publishing. They meet in the namespace.
- **Rebuild semantics are trivial.** A rebuilt peer announces the same key expressions; routing reconverges; subscribers resume without intervention.

**Generalization.** Zenoh was introduced as "the data fabric." This insight reframes it as "the peer-interaction substrate for the entire system":

| Function | Mechanism |
|---|---|
| Data publish/subscribe | Zenoh pub/sub (native) |
| Service discovery | Zenoh key-expression registry — services register under `services/<class>/<instance>/`; consumers query by pattern |
| Control / command | Zenoh request-response (native) |
| Identity attestation | Upstream Zenoh authn/access control, anchored to the offline-CA-issued peer identity. No goat-specific cap-token format. See `zenoh-data-fabric.md` §"Scope" Identity attestation row |
| Configuration distribution | Config under `config/<scope>/*`; peers subscribe to their scope |
| Partner federation | Zenoh bridge translating to partner protocol (for partners who don't speak Zenoh) |
| Health / observability | Peers publish health to `health/<class>/<instance>`; aggregator subscribes by pattern |

The Phase 2D Zenoh pilot is already the first step down this road. This reframing elevates Zenoh from "data fabric" to "universal peer-interaction substrate" and should be reflected in `zenoh-data-fabric.md` scope discussion at the Phase 2 → Phase 3 gate.

### 2A.5. The shim pattern — for legacy and vendor peers

Not every peer will be phoenix-native out of the box. Real inventory includes:

- **TAK Server and TAK clients**
- **Radios** (Meshtastic, HaLow, future coalition radios)
- **ATAK plugins** (end-user client layer)
- **Partner federation gateways** (each with its own protocol)

For peers that can't natively speak Zenoh / capability-based discovery / self-declaration, the phoenix-native pattern is:

- **A thin, stateless, generic shim** translates between the vendor's protocol and the rendezvous substrate.
- **One shim per vendor class**, not per deployment. A single "TAK-to-Zenoh bridge" handles every TAK integration.
- **Shim is itself phoenix-native.** It rebuilds from IaC + Chainguard image + a declarative config that says "I bridge TAK CoT to Zenoh topic `telemetry/tak/cot/*`."
- **Shim holds the cruft.** All vendor-specific knowledge — how to authenticate to TAK, how to parse CoT, how to handle TAK connection recovery — lives inside the shim. None of it leaks into the mesh.
- **Shim versioning is independent of the peer.** TAK Server upgrades don't touch the shim; the shim upgrades independently when TAK's protocol changes.

This is the same pattern Kubernetes uses for operators, Kafka uses for connectors, Zenoh itself uses for plugins. We're adopting it as an architectural first principle, not a one-off pattern.

**Conventions are codified separately.** The five conventions (identity, declarative role announcement, key-expression namespace, restart semantics, supply chain + repo layout), the anti-patterns, the first-shim (TAK-to-Zenoh) reference implementation, and the per-shim review checklist live at `shim-pattern.md`. Every subsequent shim is reviewed against that checklist. The first shim's extra-review investment compounds via the conventions document — future shim authors don't rediscover them.

### 2A.6. The cruft test

A concrete heuristic for every design decision: every time the team catches itself writing "we'll add this IP to hosts," "we'll set this env var on the mgmt-host," "we'll drop this cert in /etc/X," or "we'll configure this in the dashboard," — that's cruft accumulation. Convert the thought into a question:

> "How would this peer do this for itself after a rebuild?"

If the answer is "the peer already can," the design is clean. If the answer is "we'd have to do it manually again," the design is incomplete. Three ways to fix:

1. **Make the peer phoenix-native** (the ideal — add self-declaration to the peer).
2. **Write a shim** (for vendor peers that can't be modified).
3. **Accept non-phoenix-native, document the cost** (sometimes correct — flagged in the peer's integration memo; rebuild cost acknowledged).

Option 3 is allowed but must be explicit. A peer marked "non-phoenix-native — rebuild requires manual re-onboarding" is a known debt, not invisible cruft.

### 2A.7. Precedent for this pattern

The individual pattern pieces exist publicly. The combination is where we innovate:

- **SPIFFE / SPIRE** — workload identity that re-issues on restart. Widely adopted in cloud-native. Closest direct analog for the identity + attestation piece.
- **Consul service discovery** — self-announcing services. Fits 2A.3 item 3.
- **Kafka event-driven architecture** — services communicate via topics, not endpoints. Fits 2A.4.
- **mDNS / Bonjour** — self-announcing on a LAN. Fits 2A.3 at small scale.
- **Kubernetes declarative resources** — services define themselves; controllers reconcile. Fits 2A.3 conceptually.
- **Zenoh** — pub/sub by key expression. Fits 2A.4 directly.
- **Service meshes (Istio, Cilium, Linkerd)** — identity-aware networking with auto-mTLS. Fits 2A.3 item 1 at service-to-service scope.

None of these individually cover the full phoenix-native peer pattern for a tactical sovereign mesh. The combination — **SPIFFE-style identity + Consul-style self-registration + Zenoh-style capability addressing + Kubernetes-style declarative resources + shim-pattern for vendor inclusion, all survivable across full infrastructure rebuild** — is new territory.

### 2A.8. What §3 onwards now describes

The remainder of this document (§3 scope, §4 state survival, §5 rediscovery, §6 existing architecture fit, §7-§9 governance and open questions) is **implementation around the pattern defined here**. Read 2A as the center; read the rest as the how.

---

## 3. Scope — what burns, what survives

The design question is not "do we rebuild" but "what is in each teardown scope." Three scope tiers that compose:

### 3.1. Tier 1 — service rebuild (already a solved problem)

- **Scope:** all containers and their config on a mgmt-host, torn down and re-pulled.
- **Frequency:** per-deploy (already happens via `docker compose down / up`).
- **Survives:** host OS, kernel, disk, peer enrollments, PostgreSQL data, audit logs, netbird IP assignments.
- **Payoff:** supply chain validation at deploy cadence.
- **Status:** this already works; no design effort needed.

### 3.2. Tier 2 — host rebuild (core of this proposal)

- **Scope:** complete re-provisioning of a mgmt-host — OS reinstalled from cloud-init + Ansible, disks reformatted (LUKS re-generated), peer re-enrolled, PostgreSQL restored from pgBackRest, mesh identity re-issued.
- **Frequency:** open question — monthly? quarterly? on-demand? See §8.
- **Survives:** the offline CA (YubiKey), the git repo, the cloud DNS zone, the VPS relay trust, the partner federation trust chain, the data in pgBackRest S3, the data in the MinIO integrity tier.
- **Payoff:** forces the full bootstrap path to be exercised at cadence; all per-host state is re-provisioned from IaC.
- **Status:** not yet routine. Substance exists — Ansible bootstrap playbooks are complete — but scheduled exercise is new.

### 3.3. Tier 3 — site rebuild (the architectural goal)

- **Scope:** complete rebuild of a site including the mgmt-host pair, mesh routing, peer enrollment, Zenoh routers, observability stack, egress proxy. Hardware stays; everything above the bare metal is re-provisioned.
- **Frequency:** open question — semi-annually? annually? See §8.
- **Survives:** bare metal, data in pgBackRest (S3 + local), data in MinIO integrity tier, the offline CA, DNS zone, partner trust chains.
- **Payoff:** forces identity-plane rediscovery, mesh re-federation, partner re-onboarding. Exercises the "can we stand up a new site" path on an existing site.
- **Status:** the hard case. Requires the identity-free rediscovery design (§5).

### 3.4. Tier 4 — universe rebuild (the extreme)

- **Scope:** every site, every VPS relay, every bridge VPS, the offline CA (with ceremonial rekey), the mgmt-plane DNS zone.
- **Frequency:** never routinely — only at major architectural transitions or after suspected compromise of the trust root.
- **Survives:** only the partner federation trust chains (which are in the partners' custody anyway), and the data in pgBackRest.
- **Status:** this is the nuclear option. Design should make it *possible* (Block 18E tamper response already anticipates this); routine practice is not the goal.

**Primary design target:** Tier 2 monthly, Tier 3 semi-annually or at major upgrade boundaries. Tier 4 is capability, not cadence.

## 4. The load-bearing design question — what survives the rebuild?

Everything in a rebuilt system is either:

- **Re-provisioned from code** (Ansible, Terraform, Chainguard images). Lives in git; re-pulled at rebuild.
- **Re-provisioned from data** (pgBackRest backup, MinIO integrity tier). Lives in persistent storage; re-mounted at rebuild.
- **Re-provisioned from external root** (offline CA, DNS zone, partner trust). Lives outside the rebuild scope; re-referenced at rebuild.
- **Explicitly lost** (running process state, in-memory caches, peer sessions, mesh dynamic IPs). Re-established by reconnection.

The design principle: **there is no such thing as "state that must persist on the host being rebuilt."** Every piece of state lives in exactly one of the three "re-provisioned from" categories or is explicitly ephemeral. Anything else is a bug.

This principle is uncomfortable because it rejects:

- Host-local SSH keys (re-generated from config management every rebuild)
- Host-local systemd state
- Host-local package caches (acceptable to lose; re-pulled from Chainguard)
- Host-local configuration that is not represented in the repo
- Any "we'll remember where that file is" knowledge

We enforce the principle by making the rebuild cadence routine enough that accidental host-local state fails fast.

## 5. Identity-free peer rediscovery — the hard sub-problem

The user's key insight: **peer rediscovery after rebuild must not depend on MAC address, static LAN IP, Netbird peer IP, or any host-local identifier.** Those all change on rebuild; if the mesh depends on them, the rebuild breaks the mesh.

### 5.1. What's stable across rebuilds

- **Public key of offline CA.** Lives on YubiKey. Survives everything except tier-4 rebuild.
- **Public DNS zone (`legacy.example`).** Lives on Cloudflare + Route 53. Survives tier 1-3; tier 4 requires DNS re-registration.
- **Per-device long-term identity.** If we issue devices an Ed25519 long-term keypair (separate from WireGuard ephemeral keys) at factory-provisioning time, and the private key lives in a TPM or equivalent, the device's cryptographic identity survives every rebuild short of hardware replacement.
- **Per-human long-term identity.** FIDO2 keypairs on YubiKey. Survives everything.
- **Per-site rendezvous endpoint.** If we run a well-known cloud-hosted rendezvous (e.g., Cloudflare Worker at `rendezvous.legacy.example`), peers can check in and discover the current mesh without knowing IP assignments.

### 5.2. Rediscovery patterns (candidates)

**Pattern A: DNS-based bootstrap (we already have this)**

- Peer knows `netbird.legacy.example` as the management URL.
- DNS resolves to current VPS relay fleet; rathole forwards to current mgmt-host pair.
- Peer enrolls with a capability token from offline CA; management assigns mesh IP.
- **Survives:** tier 1, 2, 3. Tier 4 requires DNS re-propagation (minutes, not a design problem).
- **Status:** already in use. Sufficient for management-plane-mediated rediscovery.

**Pattern B: Content-addressable peer discovery (IPFS-style)**

- Peer identity is its Ed25519 public key. Peers advertise their keys to a DHT (Kademlia-style).
- Other peers look up peers by key, retrieve current endpoint info, establish WG tunnel.
- **Survives:** any rebuild that preserves the long-term keypair.
- **Status:** would be a new component; **libp2p** is the canonical implementation we would reach for if this pattern is ever activated (the Kademlia DHT + identify/ping protocols cover the discovery shape directly; we would not roll our own DHT). libp2p is **not** evaluated as a replacement for any other substrate — for the mesh control plane it is the wrong shape (per ADR [`0300`](../adr/0300-netbird-as-mesh-control-plane.md) alternatives), and for Zenoh it lacks key-expression routing + queryables + storage (per ADR [`0210`](../adr/0210-zenoh-universal-substrate.md) alternatives). It is specifically a candidate for the *DHT-discovery sub-pattern* and nothing else. Moderate complexity. Today this pattern is deferred — see §5.3 — because mgmt-server-mediated enrollment (Pattern A) is sufficient for tier 1-3 rebuilds and adding a DHT layer would broaden the substrate budget without closing a real gap.

**Pattern C: Gossip-based mesh rediscovery**

- Post-rebuild, each peer contacts a small list of seed peers (loaded from config).
- Seeds exchange peer lists via Serf/gossip. Mesh reconverges.
- **Survives:** any rebuild where seeds come back up first.
- **Status:** would be a new component. Familiar territory (Consul-style).

**Pattern D: Cloud-hosted rendezvous service**

- A Cloudflare Worker at a well-known URL holds the current "mesh directory" — signed per-peer current endpoint info.
- Post-rebuild peers update the directory with their new endpoint + a signature from their long-term key.
- Other peers pull the directory, find new endpoints, establish tunnels.
- **Survives:** any rebuild; Cloudflare Workers are not in our rebuild scope.
- **Status:** moderate complexity. Fits the broader Block 6 (Cloudflare Workers) architecture.

**Pattern E: Physical QR + human re-onboarding**

- Engineer carries a QR code with a short-lived re-onboarding claim.
- After rebuild, operator scans QR; device re-enrolls.
- **Survives:** any rebuild.
- **Status:** manual; not scalable; useful as break-glass.

### 5.3. Recommendation

**Pattern A (DNS + management-plane mediation) is our current mechanism and is sufficient for tier 1-3 rebuilds.** The DNS zone is outside the rebuild scope; `netbird.legacy.example` resolves to the current relay fleet; management-plane mediation handles the rest.

**Pattern D (cloud-hosted rendezvous) should be added as resilience for the case where management plane itself is being rebuilt.** Use case: tier 3 rebuild of the primary site while tier-2-rebuilt peers at other sites are trying to reconnect. A rendezvous service outside the primary site's blast radius lets peers bootstrap without the primary site being up.

**Pattern B (DHT) is a longer-term research direction.** It is strictly stronger than A + D (no external dependency), but complex. Parked as a research item for Phase 4+.

**Patterns C and E** are supplements — gossip for intra-mesh resilience, QR for break-glass.

## 6. How phoenix exercises the existing architecture

Several goat design decisions already assume phoenix-compatibility, and phoenix architecture formalizes them:

| Existing design | Phoenix-compatible aspect |
|---|---|
| Offline enrollment bundles ([`offline-enrollment.md`](offline-enrollment.md)) | Already designed for re-enrollment — CBOR bundle + capability token + 90-day expiry |
| Setup-key rotation ([`../operations/setup-key-rotation.md`](../operations/setup-key-rotation.md)) | Short TTLs already force re-enrollment at cadence |
| IaC (Terraform + Ansible) | Already the authoritative source of truth for every host |
| Chainguard-first image supply chain | Re-pull + re-verify at every rebuild |
| Block 17 egress proxy | Re-provisioned at rebuild; audit log preserved in integrity tier |
| Block 17D MinIO integrity tier | Object Lock semantics — state survives in a tier that is *not* being rebuilt |
| pgBackRest per-site paired stanzas | PG data restored from backup at rebuild |
| Spock multi-master replication | Replication re-established after rebuild; peer site is the read-through source |

Phoenix architecture does NOT require a new substrate. It requires:

1. **Routine exercise** of the existing bootstrap paths.
2. **A few new seams** for identity-free rediscovery (§5).
3. **Cadence automation** — scheduled rebuilds, not manual ones.
4. **Observability of the rebuild itself** — how long, what failed, what had to be fixed.

## 7. Governance — what could go wrong

Phoenix architecture done badly is worse than no phoenix:

- **Data loss at rebuild.** Pre-requisite: pgBackRest restore-drill (roadmap risk callout #1) must pass reliably before scheduled rebuilds begin.
- **Rebuild takes too long.** If a rebuild takes 48 hours, we can't do it monthly. Target: tier 2 rebuild in 2 hours; tier 3 in 1 business day.
- **Partner federation breaks.** TAK cert chains and partner trust — must be preserved across rebuild. Design assumption: these live in partner-controlled scope, but our side of the cert chain must survive or be re-issued. Needs per-federation memo.
- **Operator fatigue.** A monthly full-fleet rebuild imposes cost on humans. Target: rebuild is 95% automated, 5% human verification.
- **Phoenix becomes a bad habit.** If engineers stop debugging production issues ("we'll fix it at the next rebuild"), we lose the signal. Phoenix is a *scheduled* practice, not a *debugging substitute*.
- **Cascading rebuild failure.** If rebuild N fails on a specific site and the operator doesn't notice, rebuild N+1 might re-break the same site. Needs observability.

## 8. Open design questions (for project lead)

Questions the design needs answered before Block 20 is scoped:

1. **Cadence.** Tier 2 monthly? Quarterly? Tier 3 semi-annually? Annually? Recommendation: start with tier 2 quarterly on a single canary site (hadev or integdev); expand once the pattern is proven.
2. **Canary site.** hadev, integdev, or a purpose-built "phoenix-canary" site? Recommendation: integdev — already has the full stack.
3. **Rendezvous service (Pattern D).** Cloudflare Worker, or a small EC2 service? Recommendation: Cloudflare Worker — already on the roadmap as Block 6.
4. **Long-term device identity.** Add an Ed25519 long-term keypair per device, TPM-bound, separate from WireGuard? Or rely on offline-CA-issued certificates with longer TTLs? Recommendation: TPM-bound long-term key on production mgmt-hosts (x86 hosts carry TPM 2.0); offline-CA cert on Tegra hosts (Tegra-SE-backed).
5. **What survives rebuild?** Exhaustive inventory per tier — the §4 list above is a sketch; needs to be enumerated fully before first rebuild.
6. **Rebuild observability.** What metrics do we collect about the rebuild itself? Recommendation: per-rebuild report with start/end times, per-phase timing, diffs from prior rebuild, any manual interventions.
7. **Partner federation preservation.** Per-federation memo documenting what trust artifacts exist on our side, how they're preserved across rebuild, and how re-issuance works if lost.
8. **Break-glass.** Pattern E (QR re-onboarding) scoped — when, who, how recorded?
9. **What about the dev environments?** Do ephemeral cloud envs get phoenixed separately, or is their existence already phoenix-like? Probably the latter — they're already torn down and rebuilt routinely. Document that they're the progenitor of this pattern.
10. **Publication.** If we do this, do we publish a talk / paper / blog post? That's a separate decision but worth flagging now — the publishability is part of the value.

## 9. Block layout (committed 2026-04-24, restructured after project-lead review)

**Block 20: Phoenix-native peer discipline + optional scheduled rebuild.** See `../project/implementation-plan.md` Block 20 for the authoritative sub-block breakdown. Summary reproduced here for design-side context:

- **20A. Peer-cruft audit.** Inventory every peer class; score against the four phoenix-native properties. Produces `docs/design/peer-cruft-inventory.md`.
- **20B. Zenoh as peer-interaction substrate.** Extend Zenoh's scope from "data fabric" to "universal peer-interaction substrate." Design update in `zenoh-data-fabric.md` Phase 3+ scope expansion section. Contingent on Phase 2 Zenoh gate passing.
- **20C. First phoenix-native peer (gatewatch).** Concrete conversion. Canary for the pattern.
- **20D. First shim (TAK-to-Zenoh).** Establishes the conventions codified in `shim-pattern.md`. Extra-review target — first shim's investment compounds via the conventions doc.
- **20E. Cruft-test CI gate.** Every peer-integration PR requires a cruft-score entry. Prevents regression.
- **20F. Rebuild instrumentation.** Measure rebuild time. No cadence commitment.
- **20G. Scheduled rebuild cadence (optional, per-site operator choice).** Policy artifact per site, not engineering work.

Effort estimate: ~6-8 wks engineering for 20A-20F. 20G is ongoing policy maintenance per site.

### 9.1. Publication — deferred, positioned

Project lead's direction (2026-04-24): **defer publication, position to publish when we have more experience and validation.** This is the right call — we don't have enough operational data yet to credibly write up what works and what doesn't. But the *option* should be kept alive and the work structured to make publication tractable when the time comes.

**Publication readiness criteria** (when these are met, publication is viable):

1. **At least three phoenix-native peers in production** exercising the full pattern under real operational load (not just canary).
2. **At least two vendor shims shipped** — TAK and one other — demonstrating the shim pattern compounds across vendor classes.
3. **At least one full-site rebuild (tier 3)** completed successfully on a non-trivial production-shaped site, with timing data and any manual-intervention points documented.
4. **At least one rebuild that surfaced a bug** we would not have caught through normal operations. The narrative "phoenix caught this" is what distinguishes a publishable pattern from a theoretical one.
5. **Six months of operational data** on rebuild cadence (if any site opts in) or on phoenix-native peer resilience during normal operation.
6. **Partner federation preservation** demonstrated across at least one rebuild cycle — evidence that the pattern works with external trust chains.

**What we do now to preserve the publication option:**

- Keep rebuild observability data (20F) in a form that could be summarized publicly without revealing operational detail.
- Document each rebuild event's surprises in a running log — not for internal use only, but such that a future publication can reference specific incidents.
- Structure `peer-cruft-inventory.md` so it could be sanitized and published as an appendix without exposing deployment detail.
- Keep the shim-pattern review checklist in `shim-pattern.md` up to date — this is the most publishable artifact, likely the first thing an outside reader would find useful.

**When publication is considered:** Lead's decision, informed by the six readiness criteria. Venues worth considering when we get there: USENIX SRECon (cultural/operational audience); CNCF KubeCon chaos engineering track (technical audience with relevant vocabulary); blog post for broad reach.

**Why position now rather than decide now:** publications rot fast if written before the pattern is proven. But not positioning early means the publishable artifacts (logs, criteria, a coherent narrative) don't get preserved in a form that a future write-up can draw on. Positioning is cheap insurance.

## 10. Decision — commit status

**Committed 2026-04-24** to engineering discipline (20A-20F). Operational practice (20G) remains operator-choice per site.

Soft prerequisites:

- Restore drill (roadmap callout #1) retires clean before any site opts into scheduled rebuild (20G).
- §8 open questions converge during 20A / early 20B.
- Resourcing scoped as parallel with other Phase 2 / Phase 3 work; not a crash project.

## 11. Related documents

- `architecture.md` — overall system architecture
- `shim-pattern.md` — **conventions for vendor-peer shims, codified from the first shim's design work**
- `zenoh-data-fabric.md` — data fabric doc, Phase 3+ scope expansion section extends Zenoh to universal peer-interaction substrate
- [`offline-enrollment.md`](offline-enrollment.md) — capability tokens + re-enrollment flow
- [`bootstrap-bundle.md`](bootstrap-bundle.md) — cloud-held stateless bundle for rapid site provisioning
- [`os-baseline-and-luks.md`](os-baseline-and-luks.md) — per-host disk setup
- [`../operations/setup-key-rotation.md`](../operations/setup-key-rotation.md) — short-TTL enrollment keys
- `../project/implementation-plan.md` Block 20 — authoritative implementation tracking
- `../compliance/comparative-posture.md` — posture framing that this block strengthens

## 12. Change history

| Date | Change | By |
|---|---|---|
| 2026-04-24 | Initial v1 design exploration — motivation, industry precedent, scope tiers, rediscovery patterns, open design questions | engineering lead |
