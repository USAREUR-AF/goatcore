# Standing up a GOAT-Net — build-out overview

The one-glance picture before you start: what you are building, what the machine
does versus what you do, what to have ready, and how long it actually takes.
This is the **map and the expectation-setting**; the step-by-step is the
[standup handbook](goatnet-standup.md), and `goat-standup.sh <site> --plan` is
the always-current command reference for your profile. Read this first.

## What you are building

A **GOAT-Net** is one complete, self-contained environment: the mesh control
plane (NetBird management + signal + Postgres + Traefik), an identity stack (Ory
Kratos + Hydra), a Zenoh data-fabric router, and the services on top — running
on the host node's core (a single box for a dev GOAT-Net, a resilient core pair
in production; see below).

The GOAT-Net deployments the fleet actually runs are
**internet-enrollable and distributed**: peers join over the public internet
through cloud relays and a public DNS zone, and the host enrolls onto its own
mesh. That distributed path is the real system and the bulk of the work. A
LAN-only GOAT-Net (one host, or a laptop VM, peers over a LAN, no cloud) is a
genuinely smaller topology — useful when the deployment really is LAN-scoped, or
to exercise the management stack in isolation. It is **not** a warm-up for an
internet site: growing one means adding the relay tier, the public zone, ACME
and mesh-enrollment later. Build the environment the way you will run it.

## How it is driven — three engines, one command

You do not hand-execute fifteen phases from prose. The path is three fail-closed
engines chained by one driver, and the driver is your working surface.

| Engine | Answers | Command |
|---|---|---|
| **READY** | Is the config surface complete and consistent, before any host is touched? | `standup-config-preflight.py <group>` |
| **ORDER** | What runs, in what order, pausing at the genuinely manual steps? | `goat-standup.sh <site>` |
| **DONE** | Is every provisioned aspect actually live and *functional*, not just process-up? | `run-acceptance.py --env <site> --all-eligible` |

**The run, in one line:** **1** gates → **2** baseline+LUKS → **3** secrets →
**4** certs → **5** mgmt-stack + relay-connect → **6** identity + finish 5 →
**7** public TLS + relays → **⏸ 8 mesh-enroll** → **9** router + SPIRE → **10**
dashboard/OIDC → **⏸ 11 browser step** → **12** ACL, identities, services →
**13** acceptance gate → **14** rescue nodes + cross-goatnet surfaces.

Two **⏸ pauses** need a human (enroll the host; first browser login + API
token). A LAN GOAT-Net skips phases 7, 8 and 11 and the relay-connect step. Run `--plan` first
— it prints every command for your profile and executes nothing.

## What to have ready first

The driver starts at host baseline; everything here is prior to that and is the
real gap between "I have hardware" and "I can run phase 1."

### The host node — six computers plus network gear

A production host node is **six x86 computers** plus its own network gear. The
standup provisions the **core** (the management / control plane); the rest
complete a resilient site. At the small end — a dev or LAN GOAT-Net — the control
plane runs on a single x86 mini-PC or a VM; six computers is the production
shape, not the minimum.

| Role | Qty | Purpose |
|---|---|---|
| **Core pair** | 2 | Management / control plane — active + warm standby. Runs the GOAT-Net stack (postgres-spock, netbird management + signal, Ory, Traefik, the Zenoh router). **This is what the standup provisions.** |
| **Observability pair** | 2 | Logs, metrics, dashboards, object storage (OpenObserve, MinIO, Prometheus + Grafana). The second obs node holds the second copy of the audit record. |
| **Breakglass backups** | 2 | One core, one obs — dormant recovery nodes, held cutover-safe until an operator promotes them. |
| **Network gear** | 1 | A jumpbox for admin entry to the segment, plus the local switch and transport uplink. |

**Spec rule of thumb, per compute node** (source: `compute-spec-minimums.md`):
an x86 mini-PC — industrial fanless class in production, a NUC-class box
for dev or interim. **8-core, 16 GB RAM** (32 GB for a multi-site fleet), a small
system disk *plus* a **≥256 GB NVMe (512 GB recommended)** carrying the
LUKS-encrypted data tier — Postgres, audit logs, `.env` files and TLS keys live
there, and capacity on the wrong mount point is useless capacity. Node shape from
ADR 1141 (core pair + obs pair),
extended with the two breakglass backups.

### And in the repo and cloud

- **Cloud accounts** (internet topologies) — AWS for the relay tier and a
  Cloudflare zone. LAN-only needs neither; the sovereign case owns both.
- **The offline CA key** — an age-encrypted blob, handed to you out-of-band by
  the trunk owner. It is the one thing you cannot self-serve.
- **The config surface** — a group-vars *file inventory* (~30 files, not one
  `vars.yml`), an inventory group, a cert manifest, and a fleet-pins row.
  `standup-config-preflight.py <group> --scaffold-plan` lists exactly what you owe.

## What to expect — the three buckets

| Bucket | What it is | Time (repeat operator, hardware + accounts in hand) |
|---|---|---|
| **Before the run** (before phase 1) | Author the ~30-file config surface, the cert manifest, the inventory, the fleet-pins row. Desk work in the repo — no host touched. | **½–1 day** — the long pole |
| **Driver run** (phases 1–14) | Mostly automated ansible, plus the schema-wait cycle (5 → 6) and the two human pauses. Image pulls and DNS propagation dominate the waiting. | **~2–4 h** internet, ~1–2 h LAN |
| **Verify** | Read the acceptance gate; fix what it flags; re-run. | minutes–1 h |

**The honest number:** for a repeat internet build-out with hardware and accounts
in hand, expect **about a day of operator time** — and most of that day is config
authoring, not the run. A first-timer, or new hardware / new accounts, is **2–4
days**. This is a design *target*, not a measurement: the path has not yet passed
a cold-read drill (a non-author, no-AI operator, this doc set only), and that
drill is what converts the target into a fact. The biggest remaining speedup is
deriving the config surface from the reference environment automatically instead
of hand-porting ~30 files — that turns the day into an afternoon.

## Appendix — what the pieces look like

Illustrative and abbreviated; real values come from the scaffold plan, the cert
template, and your own accounts.

**1 — Site identity**, set once in `.envrc.local.d/00-site-identity.envrc`; direnv
loads it in every repo terminal, and the driver and playbooks read it:

```bash
export SITE=demo
export GROUP=demo_hosts
export SITE_UPPER=DEMO
export ZONE=netbird.demo.example.net
export OWNER_EMAIL=person-139@example.org
export HOST_IP=192.168.1.40
```

**2 — The decisions that must be right**, in `group_vars/<site>_hosts/vars.yml`
(the secure default — encrypt the data disk, and these three settings move
together with the LUKS volume):

```yaml
site_id: demo
site_dns_zone: netbird.demo.example.net
dashboard_cert_delivery_mode: acme        # acme = internet profile; file = LAN

docker_engine_data_root: "/opt/docker-data/docker"
docker_engine_containerd_root: "/opt/docker-data/containerd"
require_docker_data_separate_mount: true
luks_volumes_mgmt_host:
  - {name: docker-data, device: /dev/nvme0n1, mount_point: /opt/docker-data,
     filesystem: ext4, unlock_modes: [passphrase], format_if_unformatted: true,
     passphrase: "{{ lookup('env', luks_passphrase_envvar) }}"}

site_admins:                              # only these emails may log in
  - {email: "person-139@example.org", name: "You"}
```

**3 — The cert manifest** — copy the template, never another GOAT-Net's (its SANs
name that GOAT-Net):

```bash
# cp ops/enrollment/manifests/TEMPLATE-certs.yaml \
#    ops/enrollment/manifests/demo-certs.yaml   ; then replace every <SITE> and TBD_
ca_id: dev-demo-ca-ecdsa-2026-09-07
site_id: demo
leaves:
  - {name: router,      cn: "router.demo",  uri_san: "spiffe://demo/router", mesh_ip_san: TBD_MESH_IP}
  - {name: dashboard-a, cn: "dashboard.demo.netbird.demo.example.net", eku: serverAuth}
```

**4 — Inspect the plan before you run it** — `goat-standup.sh demo --plan --profile internet`:

```
goat-standup: site=demo  group=demo_hosts  profile=internet  (PLAN — nothing executes)
=== Phase 1 [GATE] config preflight (fail-closed) ===
=== Phase 2 [A] host baseline + LUKS (this site's nodes only) ===
=== Phase 3 [T] generate per-instance secrets ===
=== Phase 4 [T] mint leaf certs ===
=== Phase 5 [A] mgmt-stack (site-bootstrap) — first run STOPS at the schema wait ===
=== Phase 5 [A] [internet] relay-connect (rathole-client, site-rathole-client.yml) ===
=== Phase 6 [A] ory (kratos/hydra/ui) ===
=== Phase 6 [A] re-run site-bootstrap (completes phase 5; prints + saves the setup key) ===
=== Phase 7 [A] [internet] public TLS certificate (traefik-acme) ===
=== Phase 8 [MANUAL] [internet] mesh-enroll the host + re-mint the router cert ===
=== Phase 9 [A] zenoh router ===
=== Phase 13 [GATE] acceptance suite (values derived from config; all applicable rows green) ===
```

**5 — Done means green, proven** — the acceptance gate (phase 13). Read the
report, not `$?` (the run exits 0 unless a row FAILs). An expected-`UNKNOWN` list
is normal on a single-host site:

```
$ python3 ops/validation/run-acceptance.py --env demo --all-eligible
  PASS  netbird-mgmt        management API answers on the mesh IP
  PASS  kratos / hydra      login + OAuth2 endpoints healthy
  PASS  zenoh-router        bound to the mesh IP, 0 volume errors
  PASS  rosenpass-pq        post-quantum negotiated on the enrolled peer
  UNKNOWN mgmt-host-baseline  probe not eligible on a single-host site   # expected
```
