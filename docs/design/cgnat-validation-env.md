# CGNAT-aware signal federation validation env

Ephemeral validation environment for exercising the goat
signal-server federation path under production-realistic NAT conditions.
Addresses the hadev soak's major blind spot: hadev's public-EIP topology
let peers discover each other directly via P2P, bypassing signal almost
entirely (1 Forward RPC over ~5 days of continuous uptime). Production
satellite-uplink CGNAT forces every peer through signal for tunnel bootstrap —
our validation path didn't stress that path.

**Status (2026-04-24):** attempt-1 (nested-container peer swarm)
retired — infrastructure validated, peer simulator identified as the
blocker; findings at
`../reports/2026-04-24-cgnat-validation-attempt-1-findings.md`.
Attempt-2 pivots to **Path C: 100 x t2.nano native-netbird systemd
peers** (see §2, updated). Run scheduled per
`../project/roadmap.md` priority-risk §7.

---

## 1. Why this env exists

Per the 2026-04-24 hadev evidence (`../reports/2026-04-24-wave3-hadev-soak-evidence.md` §4):

- `signal_server_forward_sent_total{outcome="delivered"} = 1` over 5 days.
- `signal_server_connected_peers = 0` at capture.
- EC2 public EIPs + permissive SGs let peers discover each other
  directly — signal server was largely idle.

Production reality:

- The satellite uplink uses **CGNAT** (RFC 6598 100.64/10 customer subnets behind a
  small public-IP pool with port translation).
- CGNAT on satellite-uplink networks is typically **symmetric** —
  different source ports per destination, which breaks the STUN-discovered
  public-endpoint assumption. Hole-punching fails.
- Every peer must bootstrap via signal server. Every cross-site tunnel
  attempt requires Forward RPC traversal between signal-a and signal-b.
- When hole-punching fails, traffic must fall through to the Netbird
  relay (TCP 33073 on our VPS relays).

**The Broadcast per-peer deadline fix** (commit 50ffe81) exists in the
code but has no runtime soak behind it. This env provides the first real
test.

## 2. Topology

Two sites (<cloud-region-1-notional> + <cloud-region-alt-notional>) to match hadev's region pattern
and exercise cross-AWS-region signal federation. Each site:

- **1 mgmt-plane host** (public EIP, t3.small) running the same
  netbird-mgmt + signal-server + postgres-spock stack that hadev used.
  Public EIP because (a) peers need to reach the signal gRPC endpoint
  and (b) the VPS relay servers need to be reachable from the peers.
- **1 NAT Gateway** in the same VPC providing egress for private-subnet
  peers. NAT Gateway is AWS's implementation of symmetric NAT: source
  port translation per destination. Matches a satellite-uplink
  CGNAT profile for the purposes of hole-punching + relay fallback.
- **50 peer-host EC2** (t2.nano, private subnet, no public IP; egress
  via NAT Gateway only). Each peer-host is exactly **one netbird peer**
  running stock `netbird` as a native systemd service — not a nested
  container. 50 peers per site × 2 sites = **100 peers total**.

```
Site α (<cloud-region-1-notional>)                       Site β (<cloud-region-alt-notional>)
──────────────────────                      ──────────────────────
Public subnet:                              Public subnet:
  mgmt-plane-α (public EIP, t3.small)         mgmt-plane-β (public EIP, t3.small)
    postgres-a + signal-a                       postgres-a + signal-a
    netbird-management                          netbird-management
                                              
NAT Gateway α                              NAT Gateway β
  (symmetric NAT, egress for priv subnet)    (same)

Private subnet:                             Private subnet:
  peer α00..α49 (t2.nano each,                peer β00..β49 (t2.nano each,
   stock netbird as systemd)                   stock netbird as systemd)
   = 50 peers in site α                        = 50 peers in site β
```

Signal cross-site federation: `signal-a` on site α is configured with
`--federation-peers=<signal-β-public-endpoint>` and vice versa. Forward
RPCs flow between the two mgmt-plane public EIPs over the public
internet — exactly the production path.

### 2.1 Why 1 peer per EC2 (not 10 containers per host) — updated 2026-04-24

Attempt-1 used 5 peer-host EC2 per site with 10 `netbirdio/netbird:latest`
containers each (50 peers per site). That approach was abandoned after
container-init issues blocked stable 100-peer enrollment across netbird
releases — see findings §3 of
`../reports/2026-04-24-cgnat-validation-attempt-1-findings.md`.

Attempt-2 (Path C) uses 50 × t2.nano per site, one native netbird peer
each:

- **Cost.** 100 × t2.nano @ $0.0052/hr × 24h = ~$12.48. Full env cost
  is ~$16/run including NATGWs + mgmt-plane. The attempt-1 container
  topology was marginally cheaper on paper but the operational cost of
  fighting container init makes Path C the actual-cheapest route to a
  successful soak.
- **Production-realistic.** Native netbird on systemd is exactly how
  it runs in production. No container-vs-host cgroup
  surprises, no capability combos that drift across netbird releases.
- **CGNAT shape preserved.** 50 peers behind one NATGW per site still
  exercises the symmetric-NAT pattern (multiple source ports sharing
  one pool IP). From the NATGW's perspective there's no material
  difference between 50 containers and 50 distinct instances behind
  the same route-table association.
- **Supply-chain cleaner.** No bespoke peer-swarm image to maintain.
  Stock netbird from `pkgs.netbird.io` — same as every production host.

### 2.2 Why mgmt-plane has a public EIP (not behind NATGW)

The signal server must be reachable by every peer. Putting it behind a
NAT Gateway would require additional inbound NAT rules + port-forwards
that AWS NATGW does not support (it's egress-only by design). A public
EIP on the mgmt-plane matches how the production mgmt-plane is reached
(via the VPS relay's rathole → public mgmt endpoint). The test subject
is signal federation, not mgmt-endpoint ingress.

## 3. Test scenarios + pass criteria

### 3.1 Scenario 1 — steady-state signaling load (hours 0–20)

100 peers enrolled, making periodic peer-list requests + holding
long-lived signal sessions. soak-gen drives enrollments across the
first 3h; after that, established peers exchange signal messages at a
background rate.

**Pass:**

- `signal_server_connected_peers` ≥ 45 on each side continuously (some
  slack for peer churn; 50 per site is theoretical max).
- `signal_server_forward_sent_total{outcome="delivered"}` growing at
  > 100/hr per site (realistic cross-site signaling).
- `signal_server_forward_sent_total{outcome="deadline_exceeded"}` = 0
  or < 0.1% of delivered.
- At least one successful peer↔peer tunnel between a site-α peer and a
  site-β peer — established via signal-mediated bootstrap, falling
  through to Netbird relay if hole-punching fails (expected under
  symmetric NAT).

### 3.2 Scenario 2 — federation-load harness (hour 20–21)

Apply the existing `ops/wave3/federation-load/` scenarios against this
env:

- Scenario 1: `docker kill signal-a` — measure impact on peers
  connected to signal-a and on cross-site Forward from signal-b.
- Scenario 2: `tc netem` 2000ms delay on signal-a's 10000/tcp. Measure
  p99 Forward latency on signal-b's outbound.
- Scenario 3: restart signal-a; verify re-registration + forward
  resumption.

**Pass:**

- p99 Send latency stays bounded by (Forward deadline + local lookup
  time). The Broadcast per-peer deadline fix (commit 50ffe81) should
  prevent single-peer stall from saturating the caller.
- No sustained blocking during either fault.
- Clean recovery — err_rate + p99 return to baseline after restart.

### 3.3 Scenario 3 — hole-punching vs relay fallback instrumentation (hour 21–23)

Query each peer node (via SSM) for its data-plane type (direct P2P
via hole-punching vs relay fallback). `netbird status --detail`
surfaces the peer's current tunnel kind. With symmetric NAT both sides,
direct P2P hole-punching should fail; relay fallback should succeed.

**Pass:**

- Relay fallback rate ≥ 80% of cross-site peer-pair establishments
  (the remaining ≤ 20% may succeed via direct P2P depending on NAT
  Gateway port allocation behavior — AWS NATGW is symmetric but not
  perfectly deterministic; some session windows may happen to align).
- No failed establishments (both paths exhausted) at steady state.

### 3.4 Scenario 4 — soak + observability (hours 0–24 background)

Throughout: Prometheus scrapes mgmt-plane + signal-server + netbird
metrics. Alertmanager configured with webhook-noop for this env (we
want to see alerts fire in the UI, not page anyone).

**Pass:**

- `Broadcast` handling continues normally through the entire run.
- No sustained `deadline_exceeded` growth beyond scenario-2 windows.
- Replication slots (if using Spock between sites) active + wal_status
  reserved throughout.

## 4. Out of scope

- **Ubuntu Pro FIPS / STIG.** This env uses stock Ubuntu 22.04 — the
  test subject is signal federation correctness, not compliance.
- **pgBackRest.** Not deployed on this env; there is no production data
  to back up.
- **Zenoh.** Zenoh federation lives on integdev; not needed for signal
  validation.

## 5. Environment lifecycle

Lifecycle tag: `ephemeral`.
TeardownBy tag: `cgnat-validation-complete`.
Cost cap: **stand up → 24h soak → destroy within 30h of terraform apply**.
Estimated cost (Path C): **~$16 per run** — 100 × t2.nano (~$12.48) +
2 × t3.small mgmt-plane (~$1.01) + 2 × NATGW (~$2.16) + data transfer
(≪ $1 at signal-federation traffic volumes).

**The env MUST NOT stay up after the validation completes.** Re-running
is cheap (apply + orchestrator = ~15 min to steady state). Persistent
infra is a cost leak and an attack-surface leak.

See `../operations/cloud-env-lifecycle.md`
for the full tracker row.

## 6. What passing this validates

- Netbird signal federation under symmetric-NAT conditions matching
  a satellite uplink.
- Broadcast per-peer deadline (commit 50ffe81) under live fault injection.
- Relay fallback path from peer → VPS relay when hole-punching fails.
- Cross-site Forward RPC rate + p99 latency baselines under realistic
  peer count (100 peers, 50 per site).

## 7. What this does NOT validate

- **Actual satellite-uplink behavior.** AWS NAT Gateway is
  symmetric NAT (the right class) but not identical to any specific
  provider's implementation. Operational-level uplink
  validation needs real hardware (deferred to on-site
  bring-up).
- **Peers in genuinely DDIL conditions** (high latency, jitter, loss)
  — this env provides low-latency AWS inter-region connectivity.
  Add `tc netem` on peer-hosts in a future scenario if operationally
  relevant.
- **100+ peer scales.** 100 peers is where we start; future runs can
  scale up as needed.

## 8. Validation-env bootstrap pattern (project-wide, not just CGNAT)

This env established a pattern that applies to **every multi-host
validation env** going forward. The naive Ansible-fan-out-via-SSM
approach was the bottleneck on attempt-2: SSM session start dominates
wall-clock, so 100 peers × per-task SSM round-trip added ~25 min of
unproductive time and surfaced a separate IAM/agent race condition
that broke initial enrollment outright.

The new default pattern:

1. **Bake the bootstrap script into peer `user_data`** at `terraform
   apply` time. The script is `base64encode(file(...))`-inlined, so
   the on-disk file remains a single source of truth invocable for
   debug + recovery via SSM/Ansible.
2. **Pass identity/secret material as Terraform variables.** For peer
   enrollment that means setup keys (one per site). The same values
   are then passed to Ansible as `-e cgnat_site_*_setup_key=…` so the
   DB seed in the mgmt-plane play matches what peers present.
3. **Retry indefinitely in the bootstrap script** with bounded
   backoff. Mgmt-plane comes up *after* peers (Ansible runs after
   Terraform), so peers start by failing — the retry loop catches up
   automatically once mgmt-plane is reachable.
4. **Async + log to `/var/log`.** `nohup … &` in cloud-init prevents
   the script from blocking the cloud-init phase. Logs land at
   `/var/log/<thing>-bootstrap.log` for `ec2 get-console-output` /
   SSM-RunShellScript debug.
5. **Ansible orchestrates only the small set of mgmt hosts.** With
   peer enrollment moved to first-boot, the playbook shrinks to
   "deploy mgmt-plane stack + sanity-check it." For this env that's
   2 hosts instead of 102. Per-task overhead becomes irrelevant.

**Wall-clock impact (attempt-2 → next run):** mgmt-plane deploy
~20 min unchanged; peer enrollment 25 min → 0 min (overlaps with
mgmt-plane deploy via cloud-init retry); smoke wait 3 min → 5 min.
Net saving ~22 min per run.

**Reference implementations in this repo:**
- `../../terraform/cgnat-peer-cloud-init.yaml.tpl` — the cloud-init template.
- `../../terraform/cgnat-validation.tf` — `templatefile()` invocation + `var.cgnat_site_{a,b}_setup_key`.
- `../../ops/cgnat-validation/peer-node/install-netbird.sh` — retry-looped bootstrap script.

**When NOT to use this pattern:**
- One-off ops fixes against a single host (just SSM-send-command).
- Bootstraps that need state from already-running peers (e.g.
  Spock subscriber bring-up that needs the publisher's slot info).
  Those still need Ansible's ordered, host-aware orchestration.

## 9. Related artifacts

- Orchestrator: [`../../ops/ansible/cgnat-validation.yml`](../../ops/ansible/cgnat-validation.yml)
- Operator runbook: [`../operations/cgnat-validation-runbook.md`](../operations/cgnat-validation-runbook.md)
- Load generator: [`../../signal-server/cmd/federation-load/main.go`](../../signal-server/cmd/federation-load/main.go)
- The Terraform root, the peers' cloud-init user_data, the inventory and any
  script that writes it belong to the deployment and live in its overlay
  repository; none of them is in this repository.
- The attempt-1 peer swarm is retired and not in this repository; why is §2.1.
