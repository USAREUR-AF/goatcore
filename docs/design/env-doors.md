# Env doors — cross-env management without switching the laptop's mesh

**ADR:** [1117](../adr/1117-per-env-doors-cross-env-management.md) · **Status:** Tier 1 live (proven 2026-08-08); Tier 2 first lane **LIVE** (zenoh-door, ZAGROS, deployed + verified 2026-08-09 — M-105, track `aggregator-golive`); SSH lanes still designed-not-built · **Track:** `env-doors`

## Problem

The operator laptop's NetBird profile is a machine-global singleton: one
profile active at a time, flipping it cuts every other session off their
mesh (`.envrc.local.d/50-mesh-node-access.envrc` documents the
announce-on-bus discipline; F-269 documents the drift failure mode). All
goatnets allocate peers from the same `100.64.0.0/16`, so concurrent
membership in two meshes is a routing collision, not a client limitation.
Result: deploys to different goatnets serialize on the
profile, and concurrent Claude sessions stall.

## Principle

**Overlay membership stays on hosts that are permanently in exactly one
mesh. The laptop selects the env per-connection, not per-machine.** Every
door leads into exactly one goatnet and touches only that goatnet's
infrastructure — there is no cross-env path anywhere in this design, and
env isolation is unchanged. The laptop's mesh profile becomes a tool for
interactive mesh work, not the deploy transport.

Fleet reach inside an env is one `ProxyJump` deeper: the door lands on a
mgmt host, which is a mesh peer of its own goatnet and reaches the rest of
that fleet by the same mesh IPs the inventories already use.

## The door registry — established at standup, not hand-rolled

A goatnet is deployable off-profile only once its rows are in the door
inventories, and those rows were hand-maintained — so a fresh goatnet had none.
The door is now part of the golden path.

`ops/ansible/inventory/doors.yaml` is the source of truth — one entry per
goatnet declaring its door transport. `ops/scripts/generate-door-inventory.py`
renders each goatnet's rows into the two door inventories (`doors.ini`,
`doors-groupvars.ini`), and `goat-standup.sh` runs it as a phase-1
gate, so a goatnet is door-reachable by construction. `env-door.sh` holds no
goatnet: each time it runs it reads the door's instance, region, port, user and
landed host from the registry (`generate-door-inventory.py --arm <goatnet>`),
and opens it under the AWS profile the checkout declares (`GOAT_AWS_PROFILE`,
else `deploy.aws_profile` in `ops/goatbuilder/goatbuilder.yaml`). The generator **adds, never
reformats**: a goatnet already present (the legacy hand-written zagros / zagros-prod
blocks, with their own comments) is left untouched; only a goatnet missing from a
file gets its block appended. The golden-path linter runs `--all --check` as a
drift-gate.

**How the driver decides, per goatnet** (`deploy-component.sh`, in this order):
the mesh when the laptop's NetBird profile *is* that goatnet with `Management:
Connected`; otherwise the mesh when the goatnet's first mesh-inventory host
answers on tcp/22 within three seconds — a laptop on the goatnet whose profile
has another name deploys over the mesh like any other; otherwise the door when
the registry has one; otherwise the base inventory row. Since 2026-10-27 every
management host's base row carries its mesh IP, and a CI gate
(`ops/ci/check-goatnet-addresses.py`) refuses any inventory row that addresses
a management host elsewhere, and any use of a bad-day address (the
`breakglass-path` entries in `group_vars`) outside its declaration.

Two transport types:

| type | when | door lands on | env-door.sh opens it |
|---|---|---|---|
| `ssm` | mgmt host reached through a cloud mesh peer over SSM | `127.0.0.1:<port>` (port-forward); other mgmt hosts jump through it | yes |
| `tailscale` | no goatnet since 2026-10-27; the generator still renders it for history | the Tailscale IP, no port-forward | no |

Tailscale is not a door. It is the bad-day path onto a break-glass node or
management host when the mesh is down (ADR 1114 §4,
interim), and an address on it is reachable from one operator's machine, not
from every deployer's. Every goatnet's door is now `ssm`; who may use the tailnet, and for what, is
tailscale-rescue-access.md.
`goatbuilder doctor` refuses an environment whose only door is not `ssm`.

A goatnet that is not also in the base `mgmt-hosts.ini` is used standalone, so
its `doors-groupvars.ini` block carries `router_role` itself (the ADR-1084 §D3
admin-write guard reads it); an `ssm` goatnet in the base inventory inherits
that from the base inventory loaded first. A missing registry entry stops the
standup with the transport fields to declare.

## Tier 1 — SSM port-forward via the env's cloud mesh peer (LIVE)

Each goatnet with a cloud instance that is *also* a mesh peer gets a door
for free: SSM `AWS-StartPortForwardingSessionToRemoteHost` on that
instance forwards a laptop-local port to the mgmt host's mesh `:22`. SSH
authenticates end-to-end with the laptop's own key — the cloud peer only
moves ciphertext. No server-side changes; IAM-audited per session.

```bash
ops/scripts/env-door.sh <env>          # opens the tunnel (foreground)
ssh -p <port> <user>@127.0.0.1         # you are on that env's mgmt host
ssh -o ProxyJump=<user>@127.0.0.1:<port> <user>@<peer-mesh-ip>   # fleet
```

### Door registry

Local port is stable per env so ssh_config / inventories can be static.

| env | local port | door peer (SSM target) | region | lands on | user |
|---|---|---|---|---|---|
| `<goatnet>` | one stable port per goatnet | the goatnet's cloud mesh peer (its instance id) | the peer's region | the goatnet's management host, by mesh IP | that host's operator user |

The live rows are `ops/ansible/inventory/doors.yaml` and the inventories rendered
from it. A goatnet with no cloud mesh peer has no Tier 1 door.

### Verified (dry run)

With the laptop's NetBird parked on one goatnet the whole time and unaffected
afterward: both public relay doors answered concurrently (TLS through rathole to
each mgmt host's Traefik); concurrent SSH through both SSM doors landed on both
management hosts in the same instant, no profile switch; one `ansible-playbook`
run (ping + facts) against both goatnets in a single inventory passed.

This also confirms both meshes' ACLs admit cloud-peer → mgmt-host `:22`.

### Deploying a service through a door (the group_vars problem)

A door reaching the host is not the same as a door that can *deploy* to it.
Until 2026-08-12 there was no supported path for a service that templates from
`group_vars`, and the two obvious ones each fail for a different reason:

| inventory | reaches the host | resolves `group_vars` | why not |
|---|---|---|---|
| `doors.ini` | yes | **no** | Uses deliberately different group names (`zagros_prod_door_hosts`) so host-level plays cannot inherit site vars. A service deploy through it templates from role defaults and ships the wrong config, quietly. |
| `mgmt-hosts-mesh.ini` | yes | yes | Rides the laptop's NetBird tunnel — and **the netbird profile is one shared resource across every concurrent session**. Flipping it to deploy zagros-prod cuts every other session off ZAGROS. That is the whole reason doors exist. |

So a service deploy off-profile meant hand-rolling a local override inventory
each time, uncommitted and unreviewed (done exactly that way during the
`machine-mesh-alignment` portal deploys).

**`inventory/doors-groupvars.ini`** is that override, made supported. It is the
same hosts in their **real** groups carrying only door connection vars — the
identical shape `mgmt-hosts-mesh.ini` already uses for the mesh transport.
Three siblings, one pattern: `mgmt-hosts.ini` owns group membership and site
facts; a transport file layered after it overrides *only* `ansible_host` /
`ansible_port` / `ansible_user` / `ansible_ssh_common_args`.

```bash
ops/scripts/env-door.sh zagros-prod &          # open the door first
cd ops/ansible
OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES aws-vault exec usareur-af --no-session -- \
  ../scripts/ansible-preflight.sh \
    -i inventory/mgmt-hosts.ini -i inventory/doors-groupvars.ini \
    portal-deploy.yml -e target=zagros_prod_hosts
```

Base inventory **first** — it supplies group membership (hence `group_vars`)
and `router_role`, which the ADR 1084 §D3 admin-write guard reads. Never pass
`mgmt-hosts-mesh.ini` in the same run: two transport overrides for one host
means whichever loads last silently wins, and the symptom is an unreachable
host rather than a mis-ordered command.

Two footguns worth knowing, both cost time when learned the hard way:

- **`group_vars` resolve relative to the PLAYBOOK directory**, not the
  inventory's. Running a playbook from outside `ops/ansible/` silently yields
  `site_id` undefined even though `ansible-inventory` (run from `ops/ansible/`)
  shows it defined. Run from `ops/ansible/`.
- **`doors.ini` alone now fails the §D3 guard rather than mis-deploying** — its
  groups carry no `router_role`, and an admin-mutation play targeting several
  such hosts refuses. That is the guard working: the run that would have
  templated from role defaults stops instead of shipping.

Verified live against a production management host, with the laptop's
NetBird profile on another goatnet throughout and untouched: the run resolved
the target's `site_id`, `site_dns_zone` and `router_role` from the base inventory
and its transport from the door file.

`env-door.sh` takes each door's port and user from the registry; both door
inventories restate them, so the script asserts agreement on every run and **refuses to open** a
door whose inventories disagree — a mismatch otherwise surfaces as
"unreachable", or connects somewhere unintended.

### Limits (Tier 1)

- Needs `aws-vault exec usareur-af --no-session` + `session-manager-plugin`
  on the laptop; keychain may prompt once per boot.
- The door peer instance must be up; it is a per-env single point for this
  path only (mesh and LAN paths are untouched fallbacks).
- One tunnel per (env, target); fleet fan-out rides `ProxyJump` through
  the landed mgmt host, so a tunnel per env is enough.
- **SSM sessions idle-timeout (~20 min default)** — a door with no traffic
  closes itself. A closed
  local port means re-run `env-door.sh`, not that the path broke.
- AWS API is in the connect path. Island-mode or AWS-down means Tier 1 is
  down — same failure domain as enrollment, not worse.

### Reading a goatnet's fabric through a door (proposed, ADR 1227)

**Status: proposed, not accepted.** A platform-admin workaround only, never a
person-facing path; the supported ways to read a goatnet's fabric are a `goat`
profile on its mesh, or its panoscope. The complexity, fragility and security
costs are in [ADR 1227](../adr/1227-platform-admin-may-read-a-goatnets-fabric-through-its-door.md).

With it, a platform admin's laptop that is not a peer of a goatnet can run `goat health`, `goat sub`
or any other fabric command against it, with the operator's own certificate,
through that goatnet's door. Nothing is placed on the host and the laptop's mesh
profile is not touched.

```bash
ops/scripts/env-door.sh zagros                              # the door, as above
ops/scripts/env-door.sh zagros --forward 192.0.2.10:7447    # 127.0.0.1:17447 -> the router
goat --context zagros-op --router tls/127.0.0.1:17447 health --domain mesh
```

- `env-door.sh <goatnet> --forward <ip>:<port>` needs the door open. It forwards
  local port `<port> + 10000` (or `GOAT_FORWARD_PORT`) to `<ip>:<port>` on that
  goatnet's mesh, through the management host, in the foreground. The hop to the
  door port skips its host key check because that key churns; the outer ssh
  checks the management host's own key, the `--host-target` shape.
- `goat --router <endpoint>` (or `GOAT_ROUTER_ENDPOINT`) replaces the context's
  `router_endpoint` for one command and prints that it did. The certificate,
  key and CA roots still come from the context, so the router authenticates the
  operator by mTLS exactly as it would over the mesh. Every reader of the router
  endpoint (`open_zenoh_session_opts`, `goat status`, `goat heal`, `goat doctor`)
  resolves it through `profile::resolve_router_endpoint`, so the override
  reaches all of them.
- The endpoint is an IP address, so `goat` skips router hostname verification,
  as it does for any context whose `router_endpoint` is an IP. Both ends are
  still authenticated against the goatnet's CA.
- The forward dies with the door: an idle SSM session closes after about 20
  minutes, and a closed local port means re-running both commands.

### A LAN jump as stopgap

A goatnet with no door in either tier can be reached by one `ProxyJump` through
a management host of another goatnet that shares its LAN, without touching the
laptop's profile. For a service deploy this obeys the group_vars rule above —
base inventory first for membership, a transport-only override second. It is a
stopgap, not a door: it inherits the jump host's availability, so it dies with
the site the way a real door would not. That is the argument for the Tier 2 SSH
lane or a cloud peer, not against it.

**A half-alive host on its own mesh — F-263.** A host answered `:443` from a
mesh peer but timed out on `:22`, and could not reach its own CoreDNS on its
mesh IP while loopback worked. Cause: tailscale at `netfilter-mode=2` installs
`ip saddr 100.64.0.0/10 iifname != "tailscale0" drop`, and netbird's mesh sits
inside that range while arriving on `wt0`. Bridge-network containers survived
because their traffic is DNAT'd through FORWARD and never reaches INPUT, which is
why the host looked half-alive. `tailscale set --netfilter-mode=off` restored
it; the codified fix is `tailscale-netbird-coexist.yml` over the
`tailscale_netbird_hosts` group.

## Tier 2 — mTLS lane through the relay ingress (first lane as-built)

Every goatnet's relay `:443` already carries TLS through rathole tunnels
to the mgmt host's Traefik (relay-ingress-dispatch.md,
ADR 1085) — verified live for both zagros and zagros-prod in the dry run. The
lane shape: a Traefik **TCP router** on the enrollment entrypoint,
`HostSNI('<lane>.<zone>')`, TLS-terminated with
`RequireAndVerifyClientCert` against the offline CA
(`ops/enrollment/ca/dev/`), forwarding to a host-local TCP port. Client
side is a TLS wrap with the client cert. This is the
public-mtls-crutch-tier.md gate (ADR 0843)
applied to one TCP route — same hard-gate posture, no new subsystem.

Properties: no AWS API at connect time; works for any goatnet with relay
ingress (including one with no Tier 1 door); fail-closed to
non-holders of a CA-signed client cert, so the public surface does not
widen in practice.

### As-built: the ZAGROS zenoh-door lane (2026-08-09, track `aggregator-golive`)

The first built Tier 2 lane carries **zenoh, not SSH** — the CF-7 hosted
aggregator's cross-env leg
(consistency-foundation.md §7.8 as-built),
operator-decided 2026-08-09. **Deployed + live-verified same day
(M-105):
certless refused (`TLSV13_ALERT_CERTIFICATE_REQUIRED`), all five relay legs
land on the lane terminus, the inner zenoh-router TLS handshake completes
through it, and the aggregator's ZAGROS session runs on it in steady state.**
Four layers:

1. **DNS:** `zenoh-door.zagros.example` A round-robin directly at the 5
   relay EIPs (`terraform-zagros/main.tf`, the `intake` shape).
2. **Relay:** explicit haproxy SNI pin to zagros-a's rathole tunnel
   (`ops/ansible/inventory/zagros-goat-relay-alias.ini`) — fail-closed, never
   the `balanced` pool, per the build note below.
3. **zagros-a:** Traefik TCP router on the `enrollment` entrypoint,
   `HostSNI` + `zenoh-door-mtls` TLS option (`RequireAndVerifyClientCert`
   against the offline dev CA — the same root that signs ZAGROS's zenoh
   leaves, installed from the on-host zenoh stack copy), forwarding raw to
   the router's `tls/:7447` at the mesh IP
   (`traefik-acme/mgmt-dynamic.yml.j2`, gated
   `zenoh_door_ingress_enabled`, group_vars/zagros_hosts).
4. **Client:** the `zagros-door` haproxy sidecar in the aggregator compose
   (consistency-console role, aggregate mode; the federation-proxy
   pattern) — accepts the console's zenoh TLS stream on `:17447` and
   carries it inside the outer client-cert TLS to all five ZAGROS relays,
   first-healthy leg carries.

The zenoh mTLS session (subscribe-only leaf, verify-at-read against ZAGROS's
CA) rides **inside** the lane unchanged end-to-end — the outer hop decides
who may reach the port, never who may read the fabric. Failure mode: lane
down ⇒ the aggregator renders ZAGROS **indeterminate-with-age**, never
green, never absent.

The SSH lane variant (`ProxyCommand` wrap to a host's sshd) remains
designed-not-built; it reuses layers 1-3 verbatim with sshd
as the backend.

Build notes recorded so they aren't relearned (all three honored by the
zenoh-door lane):

- An unknown SNI lands on whichever backend the relay pool currently favours
  A built lane must use an **explicit SNI pin** at the
  relay haproxy (like `intake` / `federation-*`), never the `balanced`
  pool.
- Rathole lands on Traefik's **enrollment** entrypoint `:8443`, not
  websecure — the TCP router belongs on `enrollment`.
- Server cert: the apex LE cert does not cover lane names; the lane router
  declares its own `tls.domains` anchor so the `le` resolver mints a
  dedicated DNS-01 cert (the declare-once rule in the dynamic config
  header).

## Break-glass

Profile switching remains valid and unchanged: announce on the session bus
(`ops/work/work bus warn ...`), minimize hold, flip back — per
`50-mesh-node-access.envrc`. Doors are additive; no path was removed.

## Relation to the endgame

Pull-based convergence (ADR 1109, [fleet-management.md](fleet-management.md))
removes the laptop from routine deploys entirely; doors then remain as the
interactive/break-glass reach. Doors are the bridge, not a rival design.
