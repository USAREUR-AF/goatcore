# Multi-cloud / multi-substrate — design

**Status:** Active (added 2026-05-04, operationalizes [ADR 0220](../adr/0220-multi-cloud-multi-substrate-posture.md)).

This document is the operational design for the multi-cloud / multi-substrate posture committed in [ADR 0220](../adr/0220-multi-cloud-multi-substrate-posture.md). Where the ADR commits to the *what* and *why*, this document defines the *how* — substrate classes, per-substrate module shapes, deployment + bringup mechanics, dev/test parity, the Partner Transit Substrate Contract, and the engineering-health metrics that signal when the posture is paying off (or isn't).

The companion (separate) effort is Anycast — see [`docs/design/anycast-relay-tier.md`](anycast-relay-tier.md) and [ADR 0221](../adr/0221-anycast-relay-tier.md). Anycast benefits from multi-substrate but is not gated on it.

---

## 1. Substrate classes — what they are, where they fit

Four substrate classes for the in-flight transit tier. Site-plane workloads stay sovereign-edge regardless ([`docs/design/os-baseline-and-luks.md`](os-baseline-and-luks.md), [ADR 0604](../adr/0604-luks-data-at-rest.md), [ADR 0900](../adr/0900-ubuntu-lts-luks-x86.md)).

### 1.1 Sovereign edge

**Examples:** Jetson AGX Orin, rugged x86 edge PCs.

**Why:** Maximum sovereignty. Bytes never leave the site. Used for site-plane workloads (mgmt-plane, observability, storage). Not in scope for transit-tier deployments — transit needs reachability from outside the site.

**Used by:** Block 22 wg-cp0 local-relay augmentation (ADR 0316). Allows in-site peers to terminate at sovereign hardware when reachable, fall through to cloud relays otherwise.

### 1.2 Partner-procured

**Examples:** Partner rack / VM / bare-metal hosted under partner authority. No real instances yet.

**Why:** Partner sovereignty. Strategic capability. The deploy shape that no commercial multi-cloud arrangement can match.

**Trust model:** Partner-procured node is a **transit endpoint**, not a data-at-rest store. Encrypts everything (WireGuard + TLS). Never holds plaintext mission data. Partner provides hardware + power + transit; we provide configuration + supervision via the same mechanisms used for self-operated nodes (Ansible-over-mesh, offline-CA-signed certs, audit-by-construction).

**Status:** Block 58.4 (Partner Transit Substrate Contract — paper artifact) + Block 58.5 (one named partner pilot). Not in production.

### 1.3 Bare-metal

**Examples (CSP-agnostic):**

| Provider | Per-host class | Rough $/mo (8-core) |
|---|---|---|
| Equinix Metal | m3.small.x86 (8-core Xeon, 25 Gbps) | ~$250 |
| Vultr Bare Metal | EPYC / Xeon 8-16 core, 25-40 Gbps | ~$120-200 |
| OVH | HG-15 / HG-30 series | ~$80-150 |
| Hetzner | EX-class dedicated | ~$50-100 |

**Why:** Closer to sovereign than hyperscaler-VM (single-tenant, jurisdictionally diverse, often non-US-incorporated). Performance lever — unlocks the full Block 33 stack ([ADR 0218](../adr/0218-relay-substrate-bare-metal-strategy.md): kTLS + io_uring SQPOLL + AF_XDP + NUMA-aware shard pinning + `mitigations=off` all benefit substantially more on bare-metal than hypervisor VMs).

**Used by:** Phase 2 of [ADR 0218](../adr/0218-relay-substrate-bare-metal-strategy.md). Block 58.2 (build pre-built escape-valve modules for at least 2 bare-metal vendors).

### 1.4 Hyperscaler CSP

**Examples:** AWS (current), GCP (Block 45B Pilot A), Azure (future federation participant).

**Why:** Operational maturity, rapid spin-up, CSP-managed-service capabilities. Today's posture for the entire transit tier.

**Limits:** Multi-tenant. Hypervisor scheduler limits Block 33 lever effectiveness. Single-CSP-coercion failure mode. Hypothetically subject to sanction / withdrawal at the political level.

**Used by:** Today's `terraform/` (dev/test) and `terraform-prod/` (prod relays). Continues, augmented by the other substrate classes per Block 58.

---

## 2. Substrate-portable build artifacts

The non-negotiable: every load-bearing transit binary runs on every substrate class without source-fork divergence.

### 2.1 Layered shape

```
┌─────────────────────────────────────────────────────────┐
│  goat-relay binary (substrate-agnostic Rust)             │
│  goat-bridge binary (substrate-agnostic Go)              │
│  Container images (Chainguard, multi-arch)               │
│  Ansible role (per-component, no per-substrate forks)    │
└─────────────────────────────────────────────────────────┘
                         ▲
        ┌────────────────┼────────────────┬─────────────┐
        │                │                │             │
┌───────────────┐ ┌─────────────┐ ┌─────────────┐ ┌─────────────┐
│ Terraform     │ │ Terraform   │ │ Terraform   │ │ Terraform   │
│ AWS module    │ │ GCP module  │ │ Hetzner     │ │ Equinix     │
│ + inventory   │ │ + inventory │ │ + inventory │ │ + inventory │
└───────────────┘ └─────────────┘ └─────────────┘ └─────────────┘
```

Per-substrate divergence allowed only in:

- `terraform/<env>/providers/<provider>/` — provider Terraform.
- `ops/ansible/inventory/<provider>-<env>.ini` — inventory.
- `ops/ansible/group_vars/<provider>_<env>_hosts/` — variables (image AMI / OS image / network setup specifics).

Disallowed: per-substrate source code, per-substrate container image, per-substrate Ansible role.

### 2.2 Disqualified substrates

A substrate is disqualified if it cannot run the canonical binary. Named example: Cloudflare Workers (no raw UDP / TCP listener; no kernel-level socket access; HTTP-shaped runtime model). Workers can be evaluated for *other* roles (DNS, edge-WAF, public-edge ingress to bridges) without becoming a relay substrate.

### 2.3 Specialized variants are NOT forks

If a substrate enables a specialized capability (e.g., bare-metal `mitigations=off`), the canonical binary supports it via runtime config, not a separate build. The Block 33 lever stack already follows this convention; new levers extend it.

---

## 3. Per-substrate module shape

A pre-built escape-valve module for substrate `<S>` consists of:

### 3.1 Terraform module

`terraform/<env>/providers/<S>/main.tf`. Provisions:

- Compute (VM / bare-metal host).
- Network (VPC / subnet / public IP).
- Security groups / firewall rules (TCP 443, UDP 51820, UDP 33073, etc., per service).
- Storage backend for SSM-equivalent if applicable.
- Per-substrate compliance tag map (consumes [ADR 0219](../adr/0219-multi-repo-iac-state-separation-coordinated-compliance.md) modules where the substrate is AWS; per-substrate equivalent otherwise).

State path: `<service>/<environment>/<substrate>/terraform.tfstate`.

### 3.2 Ansible inventory + group_vars

`ops/ansible/inventory/<S>-<env>.ini`:

```ini
[<S>_<env>_hosts]
relay-1 ansible_host=<provisioned-IP>
relay-2 ansible_host=<provisioned-IP>

[<S>_<env>_hosts:vars]
ansible_connection=ssh    # or aws_ssm / etc per substrate
ansible_user=<provider-default-user>
```

`group_vars/<S>_<env>_hosts/vars.yml`: per-substrate host/network specifics.

### 3.3 Bringup orchestrator script

`ops/relay/deploy-relay-<S>.sh` orchestrates terraform-apply → inventory-generate → ansible-bootstrap → smoke-test. Mirrors the existing `ops/relay/deploy-relay.sh` pattern but per-substrate.

A common parent script `ops/relay/deploy-relay.sh` accepts `--provider=<S>` and dispatches; the per-provider script is the inner loop.

### 3.4 Health probe

Each substrate's relays MUST expose the same Prometheus metrics + same `/healthz` endpoint. Cross-substrate dashboards aggregate without per-substrate special-casing.

### 3.5 "Dormant but executable" rule

Block 58.2 + 58.3 modules ship as **dormant** — fully executable but not currently deployed. Engineering-health scorecard ([Block 32](https://github.com/usareur-af/goatdocs/blob/main/docs/design/engineering-health-assessment.md)) reviews monthly:

- Does the module still apply cleanly against current provider APIs?
- Does the bringup script complete successfully?
- Does the smoke test pass?

Module rot is a real risk for dormant infrastructure; the monthly review is the forcing function against it.

---

## 4. Dev/test parity

**Rule:** Every substrate class supported in production MUST be exercisable in dev/test.

### 4.1 Why

Multi-substrate behavior cannot be validated on single-substrate substrates. The CGNAT validation lesson ([F-018ish via `docs/design/cgnat-validation-env.md`](cgnat-validation-env.md)) was: validation-env-shape-divergence-from-prod produces misleading evidence. Same logic applies here.

### 4.2 How

- Each per-substrate module under `terraform/` (dev/test tree) mirrors the `terraform-prod/` structure.
- The CGNAT-aware validation harness extends to spin peers across multiple substrates within a single env so multi-substrate behavior is observable end-to-end.
- New ephemeral validation envs (Wave-3-style) explicitly choose substrate per their validation goal:

| Validation goal | Substrate choice |
|---|---|
| Single-region failure recovery | Multi-region single-substrate (today's posture) |
| CSP-coercion failover | At least 2 hyperscalers |
| Bare-metal Block 33 lever stack measurement ([ADR 0218](../adr/0218-relay-substrate-bare-metal-strategy.md) Phase 2) | One bare-metal + one hyperscaler control |
| Partner-procured contract validation | One partner-procured + one hyperscaler control |
| Anycast convergence ([ADR 0221](../adr/0221-anycast-relay-tier.md)) | Multi-substrate with BGP-capable PoPs |

### 4.3 Cost discipline

Multi-substrate dev/test costs more than single-substrate dev/test. Discipline:

- Each ephemeral env explicitly states its substrate choice + validation goal in its `cloud-env-lifecycle.md` row.
- Dormant escape-valve modules are NOT spun up "to keep them warm" — they get exercised through the monthly engineering-health review (apply-then-destroy cycle), not standing instances.
- Standing dev/test substrate diversity is bounded: **one** representative environment from each class, not every class for every env.

---

## 5. Partner Transit Substrate Contract (Block 58.4)

The contract a partner signs to provide transit substrate.

### 5.1 What the partner provides

- Hardware (rack / VM / bare-metal) meeting a minimum spec (4-core x86_64 / 16 GB RAM / 100 Mbps sustained / public IPv4 + IPv6).
- Power + transit + reachability (24×7 with stated uptime expectation).
- Operational access (SSH key drop or out-of-band management interface).
- Legal authority for the substrate (the partner's own jurisdiction).

### 5.2 What we provide

- The `goat-relay` binary + container image, signed by our build pipeline.
- Ansible role for bringup + maintenance.
- Offline-CA-signed cert + bundle for first-boot enrollment.
- Audit-by-construction telemetry stream so the partner can see what their substrate is carrying.
- Operational supervision (Ansible-over-mesh after first-boot enrollment, identical pattern to self-operated nodes).

### 5.3 Trust model

- Partner-procured node is a **transit endpoint**, never a data store.
- All bytes encrypted at the WireGuard layer + TLS layer; partner sees nothing plaintext.
- Audit-by-construction: every byte the partner's substrate handles is observable to the partner via wide events (Block 27). No back doors.
- Either side can withdraw with N-day notice.

### 5.4 Operational shape

- Same bringup runbook as a cloud relay — provisioning differs (partner-side), bringup-after-provisioning is identical.
- Same observability surface — Prometheus + wide events + alerts.
- Same retire-and-replace flow when hardware ages out or partnership ends.

### 5.5 Composes with existing partner work

Partner Transit Substrate Contract is parallel to:

- [Partner Data Sharing Charter](https://github.com/usareur-af/goatdocs/blob/main/docs/partner-templates/data-sharing-charter.md) — bilateral data sharing.
- [Partner Reference Architecture](https://github.com/usareur-af/goatdocs/blob/main/docs/design/partner-reference-architecture.md) — partner-side mesh stack.

Block 58.4 builds the contract template; Block 58.5 stands up the first instance with one named partner.

---

## 6. Substrate selection criteria

When a new transit node is needed, pick substrate using this prioritized criteria:

| Criterion | Question | Bias |
|---|---|---|
| Sovereignty fit | Is this for partner federation? | Partner-procured > bare-metal-EU > hyperscaler-EU > hyperscaler-US |
| Resilience class | Is this part of an Anycast prefix or a per-region fallback? | Anycast: ≥3 substrate-diverse PoPs. Fallback: substrate-diverse from primary. |
| Performance class | Is this on the Block 33 hot path? | Bare-metal > hyperscaler-with-SR-IOV > hyperscaler-VM |
| Operational class | Is rapid spin-up / tear-down a priority? | Hyperscaler > bare-metal > partner-procured |
| Cost class | All else equal, prefer cheaper. | Bare-metal cheaper than hyperscaler-VM at sustained throughput; partner-procured essentially free for transit |

Every new transit node has a stated rationale that names which criterion(ia) drove the choice. The rationale lives in the env's `cloud-env-lifecycle.md` row.

---

## 7. Engineering-health signals

[Block 32](https://github.com/usareur-af/goatdocs/blob/main/docs/design/engineering-health-assessment.md) carrying-cost metric is the explicit signal for "is this multi-substrate posture paying off?" Specifically:

- **Carrying cost per service** by substrate class. If hyperscaler-relay is 0.1 FTE-month and bare-metal-relay is 1.0 FTE-month, the diversity premium is 10×; that's a number to watch.
- **Time-to-answer** for a multi-substrate diagnostic question. Goal: same as single-substrate via wide events (Block 27) `correlation_id`.
- **Uptime per substrate**. Independent measurement, not provider-claimed. Track per-PoP, per-substrate, per-region.
- **Dormant-module health**. Monthly: how many escape-valve modules still build? Decay = posture rot.

Quarterly review per [Block 32](https://github.com/usareur-af/goatdocs/blob/main/docs/design/engineering-health-assessment.md) cadence. Posture decisions (consolidate, diversify further, retire a substrate) follow scorecard trends.

---

## 8. Migration sequencing

Block 58 sub-tasks roughly:

1. **58.1 — substrate-portable build artifacts.** Today's `goat-relay` Rust binary + Ansible role. Validate across at least one bare-metal substrate end-to-end.
2. **58.2 — pre-built escape-valve modules.** Build dormant Terraform modules + Ansible inventory templates for ≥3 of {Hetzner, OVH, Equinix Metal, GCP, Azure}.
3. **58.3 — first non-AWS production deployment.** Pick one substrate from 58.2, bring up production-class relay, run for 30 days alongside AWS, retire if measurements warrant or keep if good. Net add to fleet either way.
4. **58.4 — Partner Transit Substrate Contract template.** Paper artifact ready to fill at first partnership.
5. **58.5 — first partner-procured pilot.** One named partner, one node, validation soak.
6. **58.6 — dev/test parity.** Each substrate from 58.2 has a dev/test env spinnable + tested.
7. **58.7 — engineering-health integration.** Block 32 scorecard adopts the multi-substrate metrics from §7.

Sequencing rationale: 58.1 is foundational. 58.2 is parallel work once 58.1 is done. 58.3 + 58.6 can pipeline. 58.4 is short — paper artifact. 58.5 is gated on having an interested partner. 58.7 is a Block 32 integration task.

---

## 9. Hard prohibitions

Re-stated from the ADR for operational visibility:

- No CSP-managed-mystery on data-at-rest.
- No multi-CSP per single Terraform stack.
- No spanning state lock across substrates.
- No CSP-locked-only build paths.
- No claiming "multi-cloud" when you mean "multi-region within one CSP."

---

## 10. Open questions

- **Anycast scope overlap.** Multi-substrate Anycast is structurally stronger than either alone. The two efforts run separately ([ADR 0220](../adr/0220-multi-cloud-multi-substrate-posture.md) + [ADR 0221](../adr/0221-anycast-relay-tier.md)) but the bringup-sequence ordering is open. Tentative: 58.1+58.2 first (substrate diversity ready), then 59A Anycast spike, then 58.3 first non-AWS production deploys onto an Anycast-ready substrate, compounding the resilience win.
- **Federation-plane substrate strategy.** [ADR 0216](../adr/0216-goat-net-federation-and-site-planes.md) covers federation participants but not federation-plane infrastructure substrate. Open whether federation-plane gets its own multi-substrate posture or rides on this one.
- **DR / backup substrate.** pgBackRest stanzas live per-site today; cross-region DR for the federation plane (when it stands up) might use a different substrate class than the live federation-plane participants. TBD.
- **Cost model.** ADR is silent on aggregate cost discipline across substrates. Engineering-health scorecard tracks; explicit budget envelope might be needed at Phase-3 cutover.

---

## 11. References

- [ADR 0220](../adr/0220-multi-cloud-multi-substrate-posture.md) — the ADR this document operationalizes
- `docs/design/cloud-environments.md` — env taxonomy this design extends
- [ADR 0218](../adr/0218-relay-substrate-bare-metal-strategy.md) — bare-metal-specific strategy
- ADR 0316 — the mixed-fleet pattern this design generalizes
- [`docs/design/anycast-relay-tier.md`](anycast-relay-tier.md) — the companion (separate) Anycast effort
- `docs/project/implementation-plan.md` Block 58 — execution plan
