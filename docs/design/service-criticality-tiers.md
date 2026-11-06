# Service criticality tiers — what a goatnet node must run, should run, and can skip

**Status:** Active (added 2026-07-15, track `lab-node-minimal-node`).

Every service the mgmt-host stack can run is assigned exactly one **criticality tier**. The tier answers one question: *what breaks for users of this goatnet if the service is absent?* The assignment is machine-readable (`service_tiers` map, §4) so that a resource-constrained node — the first consumer is the `hobby` ZimaBoard instance (hobby-goatnet.md) — deploys a tier-filtered subset by setting one group_var, instead of hand-curating a service list that drifts.

Companion: instance-variability-model.md (the tier ceiling is a Class-C knob), goatnet-turnkey-standup.md (deploy ordering).

## 1. The four tiers

| Tier | Value | Definition | Absent ⇒ |
|---|---|---|---|
| **core** | 0 | Load-bearing foundational substrate. The mesh, identity, data fabric, ingress, and state under all of them. | The goatnet does not function. |
| **essential** | 1 | Critical for full platform functionality: onboarding, cert minting, audit capture, client distribution. | Existing users keep working; nobody new gets on, nothing is auditable. |
| **utility** | 2 | Inspection + observability surfaces. Absence degrades operator insight, not user capability. | Operators are blinder; users unaffected. |
| **nicety** | 3 | Demos, sandboxes, synthetic traffic. No effect on operational value. | Nothing operational. |

Assignment rules:

- **A service's tier is set by its worst consumer.** If an essential flow depends on it, it is at least essential.
- **Sidecars inherit their primary.** An `oauth2-proxy-<svc>` or `<svc>-spiffe-helper` carries `<svc>`'s tier.
- **Tiers are about *presence on a node*, not data classification or runtime priority.** QoS classes ([zenoh-qos-and-prioritization.md](zenoh-qos-and-prioritization.md)) and data-access roles are orthogonal.
- **A tier is fleet-wide vocabulary; the ceiling is per-site.** ZAGROS runs everything (`site_deploy_tier_max: 3`); a minimal node runs `≤ 1`.

## 2. Tier assignments

Judgment calls, made explicit:

- **selfservice-ui is core** — without the login UI there is no OIDC flow at all.
- **SPIRE (server + agent) is core** — F-200/F-202 self-heal and Phase-A SVID issuance sit under running services; external orgs stay portal-minted regardless (ADR 0412).
- **MinIO is essential, not utility** — it is the audit tier-2 store (ADR 0811 three-tier shape); losing it silently breaks audit retention, which is an essential property.
- **The env-health publishers (fabric/mesh/net/time/ntp/presence) are utility** — they are the honesty surface (ADR 1036), cheap (~42 MiB each), and worth running everywhere, but their absence does not remove user capability.
- **mesh-dns is core** — service discovery for every mesh consumer.

| Tier | Services |
|---|---|
| **core (0)** | traefik · netbird management + signal · netbird-dashboard (+admin-fence proxy — the first OIDC login mints the account Owner, F-188) · postgres · kratos · hydra · selfservice-ui · zenoh router · mesh-dns/CoreDNS · spire-server · spire-agent · admit-audit |
| **essential (1)** | goat-zenoh-portal · goat-id-console (+proxy, +spiffe-helper) · onboard-runner (+spiffe-helper) · onboard-intake-collector · minio · audit-subscriber · goat-get · goat-docs · grant-keyd (+spiffe-helper) |
| **utility (2)** | panoscope (+proxy) · stethoscope · flashlight · isitup (+proxy) · obs stack (prometheus, grafana, alertmanager, blackbox, cadvisor, node-exporter, postgres-exporter, process-exporter) · goat-logship · env-health publishers (fabric-health, mesh-health, node-net-health, node-net-health-rollup, time-health, ntp-health-pub, presence-health) · relay-health + relay-metrics-ingest (+proxy) · compliance proxies (aws-sigv4-proxy, oauth2-proxy-compliance) |
| **nicety (3)** | airdefenders (+zenoh-sidecar) · zenoh-sandbox (router + web) · synth-* vendor fleet · brief · dataflows |

## 3. Measured footprint (evidence)

Measured on a 14-core / 122 GiB node, 2026-07-15, full stack, `docker stats` + `ps`:

| Slice | RAM (RSS) | Notes |
|---|---|---|
| Whole stack | 12.7 GiB | matches host `free` |
| core tier | ~1.6 GiB | zenoh router 885 MiB is the largest member |
| essential tier | ~0.4 GiB + MinIO | MinIO sat at its 4 GiB cap (3.5 GiB used) |
| utility tier | ~6.3 GiB | **panoscope alone 5.1 GiB, uncapped**; everything else ~1.2 GiB |
| nicety tier | ~1.2 GiB | 20 synth vendors ≈ 870 MiB of it |
| Docker runtime overhead | ~1.36 GiB | dockerd 165 + containerd 118 + 81 shims × ~13 MiB |
| Stack CPU (steady state) | < 2 of 14 cores | 86 % idle; top consumers traefik/dockerd/minio/zenohd/netbird |

Two configuration levers dominate every platform-level frugality option (including a Podman/Quadlet swap, which recovers ~1 GiB — see ADR 0207's still-open Block 16 spike):

- **Cap panoscope.** 5.1 GiB uncapped on a utility-tier service; set a compose `mem_limit` wherever it deploys. The uncapped growth itself warrants investigation on ZAGROS.
- **Cap MinIO to the site's audit volume.** 4 GiB is an ZAGROS-scale cap; a minimal node runs correctly at 1 GiB.

**Inference (not yet load-tested):** a core+essential node needs ~3 GiB RAM + ~1 core steady-state at ZAGROS-scale traffic; a 4-core/16 GB Intel N150 class box fits with ample headroom. Confirm per ADR 1012 once the first minimal node is live.

## 4. Machine-readable mechanism

The tier map is fleet-shared vocabulary (Class-C, one definition), declared once in `ops/ansible/group_vars/all.yml`:

```yaml
# Fleet-wide service criticality tiers (docs/design/service-criticality-tiers.md).
# core=0 essential=1 utility=2 nicety=3. Sidecars inherit their primary.
service_tiers:
  traefik: 0
  netbird_mgmt: 0
  # … one entry per service …
  panoscope: 2
  airdefenders: 3

# Per-site ceiling — deploy services with tier <= this. Class-C knob;
# per-site group_vars override. Default: everything (full parity).
site_deploy_tier_max: 3
```

A site's group_vars derive per-service enables from the map instead of hand-setting them:

```yaml
# group_vars/hobby_hosts/vars.yml
site_deploy_tier_max: 1   # core + essential only
panoscope_enabled: "{{ service_tiers.panoscope | int <= site_deploy_tier_max | int }}"
```

Adoption is progressive: roles keep their existing `*_enabled` flags; the tier map becomes the value those flags are derived *from* in per-site group_vars. A new service must be added to `service_tiers` when its role lands (review-enforced via the table in §2 staying in sync — drift between §2 and the map is a doc bug).

**Boundary:** the legacy legacy `services.yaml`/`generate.py`/Caddy registry is out of scope — it retires with legacy (instance-variability-model.md §5). The tier model covers the role-based fleet method.

## 5. What this is not

- Not a runtime SLA or restart-priority scheme — Docker restart policies are unchanged.
- Not data classification — a nicety service's *data* may still be access-controlled.
- Not a deployment *order* — ordering stays with the standup sequence (goatnet-turnkey-standup.md); tiers only decide membership.
