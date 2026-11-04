# GOAT-Net standup — the procedure. Start here.

If a site node cannot install, boot, connect, or converge, start with the
symptom router in [`site-node-troubleshooting.md`](site-node-troubleshooting.md)
before selecting a recovery procedure.

How to stand up a complete GOAT-Net from nothing to an acceptance-gate pass. A
**GOAT-Net** is one self-contained environment: a core management host running
the mesh control plane (NetBird management + signal + Postgres + Traefik), an
identity stack (Ory Kratos + Hydra), a Zenoh data-fabric router, and the
services on top — **plus, when the site declares one, a separate observability
host** (the ADR 0826 "Orin-C": Prometheus + Alertmanager + Grafana, and Forgejo,
the pub/sub state-of-record primary). The obs host is part of this path: the
driver builds it **in parallel** with the core host, automatically, whenever it
is identified and prepped (see [The obs leg](#the-obs-leg--built-in-parallel)).
What is still out of scope is a core A/B *pair* (vega / zagros-prod streaming
standby), which follows its own `<env>-bootstrap.yml` and runbook.

**New to this? Read [`goatnet-buildout-overview.md`](goatnet-buildout-overview.md)
first** — the big picture, the six-computer host node, and what to expect on time.
This doc is the step-by-step.

> ⚠️ **NOT YET CERTIFIED TURNKEY.** The driver's phase commands are transcribed
> from validated runs, but the whole path has **not** passed the cold-read drill
> — a non-author, no AI, this doc set only, standing a GOAT-Net up start to end
> with zero improvisation, which is the definition of done in
> `standup-remediation-plan.md`. The
> internet-profile chaining in particular (the phase 5 → 6 schema-wait hand-off
> and the post-login re-runs) has not been driven end to end. **Expect gaps, and
> treat any step that needs improvisation or outside knowledge as a defect in
> the procedure, not operator error** — file it (see [When you hit a
> gap](#troubleshooting--when-you-hit-a-gap)). Do not present this as certified turnkey until the
> drill has passed. Where a default here suits a VM but not your host, the host
> wins — see [The decisions that must be right](#the-decisions-that-must-be-right).

---

## How this works — three engines and one driver

This procedure is not a transcript you hand-execute. It is three fail-closed
engines, chained by one driver. Read this doc once for the map and the
decisions; then the driver is your working surface.

| Engine | Answers | Command |
|---|---|---|
| **READY** | is the config surface complete and internally consistent, before any host is touched | `standup-config-preflight.py <group>` |
| **ORDER** | what runs, in what order, pausing at the genuinely manual steps | `goatbuilder standup <env> --site <site>` (supported); `goat-standup.sh <site>` the documented alternative ([ADR 1201](../adr/1201-two-avenues-scripts-documented-cli-supported.md)) |
| **DONE** | is every provisioned aspect actually live and *functional* (not just process-up) | `goatbuilder assay <site>` (= `run-acceptance.py --env <site> --all-eligible --by-layer`); re-run it any time after standup |

The ORDER engine has two avenues, and ADR 1201 keeps both working. Each **is**
the phase list below, executable: it runs each phase's tool/playbook in the
load-bearing order, runs the config gate before touching a host and the
acceptance gate after, and stops at operator-fired steps with the exact
instruction (resume with `--from <next>`). The supported avenue is `goatbuilder
standup <env> --site <site>`; `goat-standup.sh <site>` is the documented
alternative, kept functional and exercised by the same gates, and a script is
never removed because its verb exists. Both read the same config surface and run
the same numbered phases. Everything else in the repo is
*why* — start with
[`goatnet-deployment-doctrine.md`](../design/goatnet-deployment-doctrine.md), and
read it before changing anything here or in the deployment surface.

**When you hit a gap** — and you will until the drill passes clean — the fourth
engine is the feedback loop: file the gap against the remediation plan, and if it
is a wrong default, a missing config file, a coupled fact, or a silent failure,
add a check to the preflight or a row to the acceptance suite so the *next*
standup cannot hit it. That loop, not prose, is how this becomes turnkey.

---

## Pick your topology

**Pick the topology that matches what you are building — not the smallest one.**
The GOAT-Net deployments this fleet actually runs are
**internet-enrollable**: peers join over the public internet through cloud
relays and a public DNS zone, and the management host enrolls onto its own mesh
(phase 8). That distributed path — relays, public DNS, ACME, mesh-enrollment —
is the real system and the bulk of the work. LAN-only is a genuinely different,
smaller topology, not a warm-up for it.

One procedure covers all three; they differ only in **who owns the cloud
accounts** and **whether peers enroll over a LAN or the public internet** — not
in the phase order. The driver takes `--profile`; the AWS account is
`GOAT_AWS_PROFILE`.

| Topology | Peers reach the mesh over | Cloud accounts | `--profile` | `GOAT_AWS_PROFILE` |
|---|---|---|---|---|
| **Internet, usareur-af-hosted** | the public internet, via usareur-af's relays + DNS zone | usareur-af AWS + Cloudflare | `internet` | `usareur-af` (default) |
| **Internet, sovereign-owned** | the public internet, via **your** relays + zone | your own AWS + Cloudflare + domain | `internet` | your own profile |
| **LAN-only** | one LAN (or one laptop, as a VM) | none | `lan` | any profile `aws-vault` can resolve for GHCR pulls |

**A learner's first goatnet is goatlite**: `preset: lite`, one box, the LAN
profile, from [`docs/tutorials/00-goatlite.md`](../tutorials/00-goatlite.md).
The learner writes four facts and the preset derives the config surface this
runbook otherwise has you author by hand; the same fourteen phases then run. It is
an instance of this procedure, not a second one
([goatlite.md](../design/goatlite.md), [ADR 1213](../adr/1213-goatlite-preset-one-box-one-file.md)).

Choose **LAN-only** when the deployment really is LAN-scoped, or to exercise the
management stack in isolation — **not** as an on-ramp to an internet site.
Growing a LAN GOAT-Net into an internet one is not free: it means adding the relay
tier, the public DNS zone, ACME, and host mesh-enrollment (the relay-connect step of phase 5, and phases 7 and 8), none
of which the LAN path stands up. If the thing you are ultimately building is
internet-enrollable, build it that way from the start. The driver auto-detects
the profile from the config (`dashboard_cert_delivery_mode: acme` means
internet); pass `--profile` to override.

The internet topologies additionally need a **relay tier** (Terraform +
`relay-server.yml`) and a **public DNS zone** stood up before phase 5 — see
`zagros-prod-standup-runbook.md` §"Relay tier" for
the worked instance; the sovereign case repoints its Terraform tree and
Cloudflare token to its own accounts.

---

## Before phase 1 — what the driver assumes already exists

> **From an unboxed machine?** The steps below assume Ubuntu 24.04 + full-disk
> LUKS + your key are already on the box. Getting there from bare metal —
> firmware settings, the installer USB, and the autoinstall answer file — is
> [bare-metal-install.md](bare-metal-install.md).

Both drivers (`goatbuilder standup` and `goat-standup.sh`) start at host
baseline. Everything here is **prior** to that, is not automated, and is the
single biggest gap between "I have hardware" and "I can run phase 1." Nothing
here touches a host; it is all repo, cloud and hardware.

| Prep | Before the run | What it is | Where |
|---|---|---|---|
| 1 | Hardware re-baseline | Boxes coming off another GOAT-Net: de-enroll from the old mesh, drop Spock/pgBackRest membership before wiping, revoke wg-cp0 bundles + leaf certs, `cryptsetup luksErase`, re-flash. Re-flashing an Orin needs an **x86 Ubuntu host cabled over USB-C** — a Mac cannot do it | `zagros-prod-standup-runbook.md` §"Orin decommission and re-baseline" |
| 2 | Credentials | Per-zone Cloudflare token (`Zone:Read` + `DNS:Edit`), GHCR `read:packages` PAT, AWS via `aws-vault add`, and the age-encrypted dev-CA blob (out-of-band from the trunk owner — the one thing that cannot be self-served) | [`.envrc.example`](../../.envrc.example) |
| 3 | Terraform tree (internet only) | `terraform-<env>/` with the cross-account fuse in the same commit; the zone, relays and DNS records live here. **The driver has no terraform phase** | runbook §"Terraform tree with the cross-account fuse" |
| 4 | Zone reputation (internet only) | The moment the public zone exists, verify it in Google Search Console (add the verification `TXT` record alongside the zone's other records in `terraform-<env>/`) and submit a review request for the enrollment apex + `dashboard`/`auth`. A fresh public login domain is flagged as social engineering by default; establishing the zone with Google ahead of first enrollment is what keeps the red interstitial from ever appearing. The finish gate's `public-auth-fqdns-not-flagged` row is the backstop, not the fix | Search Console; finish-gate row in [Verification](#verification--done-means-green-proven) |
| 5 | Inventory + config surface | An inventory group `[<site>_hosts]` **and** the group_vars *file inventory* — ~30 files, not one `vars.yml` | `standup-config-preflight.py <group> --scaffold-plan` |
| 6 | Obs host (optional) | If the site has an observability host: `has_obs_host: true` in `group_vars/<site>_hosts/vars.yml`, its own config surface `group_vars/<site>_obs_hosts/` (`luks_volumes_mgmt_host` + `docker_engine_data_root` at minimum; `obs_scrape_targets` once the core host's address exists; `forgejo.yml`), an inventory group `[<site>_obs_hosts]` with the host (LAN address in `mgmt-hosts.ini`, mesh address in `mgmt-hosts-mesh.ini` once enrolled; `ansible_user` in `[<site>_obs_hosts:vars]`), and a `doors.yaml` row with `gv_group: <site>_obs_hosts`. All four present = the driver builds it in parallel; any absent = skipped with the reason, never an error | `standup-config-preflight.py <site>_hosts --obs <site>_obs_hosts` |
| 7 | Cert manifest | `ops/enrollment/manifests/<site>-certs.yaml`, from [`TEMPLATE-certs.yaml`](../../ops/enrollment/manifests/TEMPLATE-certs.yaml). Copy the template, never another GOAT-Net's manifest — its URI SANs name that GOAT-Net | template header |
| 8 | Fleet-pins row + parity | The env must appear in `ops/pins/fleet-pins.yaml` and the parity surfaces from the first commit, or it deploys unobserved | runbook §"Fleet-pins row" and §"Parity observability" |
| 9 | Env-door registry entry | One entry in [`ops/ansible/inventory/doors.yaml`](../../ops/ansible/inventory/doors.yaml) declaring the goatnet's door transport (`ssm` peer + port, or `tailscale` IP) so it is deployable off-profile. Phase 1 then renders its `doors.ini` / `doors-groupvars.ini` rows from it (`generate-door-inventory.py <site>`); without the entry the standup stops and names the fields. [env-doors](../design/env-doors.md) | `doors.yaml` + [`env-doors`](../design/env-doors.md) |
| 10 | Compliance-dashboard decision | **A decision, opt-in:** a cloud goatnet ties into the compliance dashboard; a throwaway LAN sandbox with no cloud footprint does not. To opt in, the env's `terraform-<env>/main.tf` carries BOTH the `compliance` (tags) and `iac_registry` (the `/usareur-af/<env>/iac-registry/*` write the dashboard enumerates) modules — apply registers it. Half of that pair (tags without the registry write) is the trap: resources are tagged but the tree is invisible to the dashboard. `check-compliance-registration.py` fails a tree that has one module without the other, so the decision is whole or absent — never silently half-registered | `terraform-<env>/main.tf`; `ops/ci/check-compliance-registration.py` |

**A goatlite skips this table.** Under `preset: lite` ([`goatlite.md`](../design/goatlite.md)), the first phase-1 gate runs `ops/scripts/goatlite-render.py`, which writes rows 5, 7, 8 and 9 (the config surface, the cert manifest, the pins row in the sidecar table `ops/pins/lite-pins.yaml`, the `direct` door entry) from the four facts in `goatlite.yaml`, and mints the goatnet's CA (row 2's one non-self-served item) on the first run.

`--scaffold-plan` is the one to run first, because it works before anything
exists:

```
ops/scripts/standup-config-preflight.py <site>_hosts --scaffold-plan
```

It lists every config file the reference env has, marks each `have` / `MISSING`
/ `excluded`, and treats a missing file as a decision you owe — port it
re-threaded for your site, or declare the absence with a reason in
`.groupvars-parity.yaml`. The reference environment (`zagros_hosts`) is the file
inventory to diff against; the count grows as service surfaces are added, so read
the tool's number, not a number written here.

`--scaffold <site_id>` then writes those files from the reference env. Any value
naming another environment is replaced with a `TBD_` sentinel. That covers list
items and indented values as well as top-level keys, and the preflight refuses
to pass while a sentinel remains. The preflight also refuses any config line
naming another goatnet's domain, unless the line carries an inline
`# cross-goatnet: <reason>`. CI runs the same check over every goatnet
(`ops/ci/check-no-foreign-goatnet-config.py`). The scaffold checks list items
and indented values as well as top-level keys, because a nested list naming
another goatnet's relays passes a top-level-only check and every one of them answers.

---

## Prerequisites — everything you need, in one place

**Tools (control node, once):**

```
brew install aws-vault direnv ansible go jq yq age            # + coreutils, curl, openssl
brew install hudochenkov/sshpass/sshpass                      # not in core Homebrew
ansible-galaxy collection install -r ops/ansible/requirements.yml
```

Every one is load-bearing:

| Tool | Fails without it |
|---|---|
| `yq` | phase 4 — `mint-site-certs.sh` parses the cert manifest with it |
| `sshpass` | before phase 2 — `seed-ssh-key.sh` consumes the factory password once |
| `go` | phase 4 — the name-snapshot signer and the CA/cert tooling are Go programs |
| `age` | the dev-CA blob is age-encrypted; no `age`, no CA key, no leaf certs |
| `aws-vault` | every ansible invocation wraps in it (`--no-session`, always) |

Versions are NOT pinned anywhere, including `ansible-core` — that gap is tracked
as C1/row 49 in the remediation plan and is the one prerequisite this list cannot
yet make reproducible.

**Credentials (once per operator):** copy [`.envrc.example`](../../.envrc.example)
into `.envrc.local.d/*.envrc`, fill the placeholders (each carries a source
note), `direnv allow`. AWS via `aws-vault add <profile>` (OS keychain, never a
file; always `--no-session`). The dev-CA blob (`ca.key.age`) and roster seat come
out-of-band from the trunk owner — the one thing that cannot be self-served. Full
layout: `local-secrets-pattern.md`.

---

## The decisions that must be right

Three config choices are load-bearing for security or correctness, easy to get
wrong by pasting a default, and not caught by the preflight. Get these right in
your group vars *before* phase 1.

**1 — Encrypt the data disk (the default, not the exception).** A real host with
a data disk MUST encrypt it: `os-baseline-and-luks.md`
R4 requires data-at-rest encryption on every box carrying non-public data (STIG
V-260515, CSfC DAR CP), and the data disk is exactly where Postgres, audit logs,
`.env` files and TLS private keys live. Two settings move together and must
agree:

- `luks_volumes_mgmt_host` — copy the engaged shape from `zagros_prod_hosts/vars.yml`
  (a passphrase-backed `docker-data` volume). It lists `tang_client` for
  network-bound auto-unseal because it is multi-node; **drop `tang_client` on a
  single host** (tang needs two or more same-site nodes; alone, passphrase-only
  leaves the volume `noauto` — you type the passphrase at the console on each
  boot). Keep `passphrase` as the slot-0 break-glass.
- `docker_engine_data_root: /opt/docker-data`, `docker_engine_containerd_root:
  /opt/docker-data/containerd`, `require_docker_data_separate_mount: true` — so
  Docker's data lands on that encrypted mount and phase 1's host preflight
  *proves* it did on every later run.
- `<SITE>_A_LUKS_PASS` in `.envrc.local.d/20-host-access.envrc` must be the
  **real** passphrase — phase 2 encrypts the disk with it; a placeholder value
  encrypts your disk with a known throwaway string.

The **only** exception is a VM or a host with no second disk: set
`luks_volumes_mgmt_host: []` and the three docker keys to `/var/lib/docker` +
`false`. That runs unencrypted at rest — record the deviation in
`.groupvars-parity.yaml` so a later reader can tell it was a choice.

**2 — `site_admins` is you.** The dashboard admits only the emails listed there;
self-registration is off, and the reference copy names someone else. Rewrite it to
your `OWNER_EMAIL` before phase 5 — phase 6 creates that login identity.

**3 — The mesh IP couples to the router cert (internet only).** The Zenoh
router's server cert pins the mesh IP as a SAN, and enrollment (phase 8) assigns
that IP. If it changes, re-mint the router cert (phase 8) *before* phase 9, or
the router presents a cert for the wrong address.

---

## The run

**Phase 1 is two gates, and the split is deliberate.** The config preflight reads
`group_vars` and contacts no host; the host preflight reads hosts and no
`group_vars`. A configuration can be complete and internally consistent for a
machine that does not match what it assumes, and every invariant the host gate
checks is one this repository already documents and that otherwise fails LATE —
during a play, after deployment, or not at all:

| Read | Why it fails late otherwise |
|---|---|
| Docker **and** containerd data-roots both on the Tier-3 NVMe | one left on the root disk fills it, and the node wedges |
| `net.ipv4/ipv6.ip_nonlocal_bind = 1` | a VIP cannot be bound before it is held; failover then reads as a service crash |
| `chrony-presync.service` ordered before chrony and docker | containers start against an unsynced clock, and certificate validity fails for reasons that have nothing to do with certificates |

It changes nothing, and a probe it cannot read is reported UNKNOWN rather than
passed — an unreadable probe is an unknown, and reporting one as a pass is the
substitution the phase exists to prevent.


The supported avenue (ADR 1201):

```
goatbuilder standup <env> --site <site> --dry-run    # print every phase and command for your profile; runs nothing
goatbuilder standup <env> --site <site>              # execute; pauses at manual steps
goatbuilder standup <env> --site <site> --from <n>   # resume after a manual step or a fixed failure
```

The documented alternative — the same phases, same order, kept functional:

```
ops/scripts/goat-standup.sh <site> --plan            # inspect the full plan for your profile; runs nothing
ops/scripts/goat-standup.sh <site>                   # execute; pauses at manual steps
ops/scripts/goat-standup.sh <site> --from <n>        # resume after a manual step or a fixed failure
```

`goatbuilder standup --dry-run` (and the script's `--plan`) prints the exact
command per phase for your profile — that printout is the always-current command
reference; there is no separate crib sheet to drift.
`GATE` = fail-closed check · `T` = operator-machine tool · `A` = ansible · `O` =
genuinely manual. Phases are numbered 1 to 14 with no gaps; a LAN site skips the
internet-only phases 7 and 8, and phases 10 and 11 differ by profile.

| # | Kind | Phase | Profile | Worked when |
|---|---|---|---|---|
| 1 | GATE | Config preflight (TBD sentinels, file inventory, coupled-fact + secret-format checks) | both | prints `preflight OK`; any failure names the file + reason |
| 1 | GATE | **Declared ACL** — `netbird-policy-posture.py <site> --declared` (a joiner checks its founder's) | both | prints `OK: '<site>' declares its ACL (N rules) and default_policy_action: delete`; a goatnet with no declared ACL is refused here, because without one its peers reach nothing ([ADR 1185](../adr/1185-no-default-all-to-all-standup-applies-declared-acl.md)) |
| 1 | GATE | **Host preflight** — measures the target hosts and changes nothing (`standup-host-preflight.py`) | both | prints `READY`; a failure names the invariant, the host, and what it read. Skips with a note when the group has no hosts in inventory yet, which is normal before phase 1 |
| 2 | A | Host baseline + LUKS (all nodes; `seed-ssh-key.sh` first, once) | both | `DockerRootDir` is your encrypted mount; on hardware, `findmnt /opt/docker-data` shows the LUKS mapper |
| 3 | T | Generate the ~30 per-instance secrets | both | `env \| grep <SITE>_` lists the site's vars after `direnv allow` |
| 4 | T | Mint every leaf cert from the manifest (+ name snapshot) | both | `mint-site-certs.sh` reports every leaf present; no manifest-not-found |
| 5 | A | mgmt-stack (`site-bootstrap.yml`) — **first run stops at the schema wait** | both | on a first run it stops waiting for the netbird schema — **expected**; phase 6 finishes it |
| 5 | A | Relay-connect (`site-rathole-client.yml`, or the site's own `<site>-rathole-client.yml` when it keeps one) | internet | `ss -tnp \| grep 2333` shows one ESTABLISHED tunnel per relay; a `curl` of `netbird.<zone>` returns any HTTP code (`000` = the tunnel is not carrying traffic) |
| 6 | A | Identity stack — Ory (kratos/hydra), **before** account-seed | both | `docker ps` shows kratos + hydra `Up`; `hydra:4444/health/ready` returns `{"status":"ok"}` |
| 6 | A | Re-run `site-bootstrap.yml` — completes phase 5, prints the setup key | both | play ends `failed=0` and prints the single-use bootstrap setup key — **save it** |
| 7 | A | Public TLS via ACME (`traefik-acme.yml`, DNS-01): Ory sites must declare non-empty, equal `site_dns_zone` and `public_mgmt_fqdn` in the target group; preflight checks this before applying the overlay | internet | the issuer stops reading `TRAEFIK DEFAULT CERT`; if it does not, it is a DNS-01 failure — see [Cloudflare failure modes](#troubleshooting--when-you-hit-a-gap) |
| 8 | O | Mesh-enroll the host + re-mint the router cert | internet | `netbird status` shows `Management: Connected` and a `100.x` mesh IP — see [Phase 8 in full](#phase-8--mesh-enroll-in-full) |
| 9 | A | Per-uuid Zenoh router (binds the mesh IP; cert from phase 4/8) | both | container `Up (healthy)`, a `7447` listener on the intended IP, `0` "Cannot find volume" log lines — see [Phase 9](#phase-9--the-router-is-bound-not-just-up) |
| 9 | A | SPIRE server on the founder (`spire-deploy.yml -l 'mgmt-<site>-01*' -e spire_enabled=true`): workload identity for the portal bridge, id-console, onboard-runner and grant-keyd | both | `spire-server` container healthy; `docker exec spire-server /opt/spire/bin/spire-server entry show` lists the `grant-keyd` entry |
| 10 | A | LAN: combined LAN services play · internet: dashboard + OIDC clients (`site-oidc-clients.yml`, or the site's own `<site>-oidc-clients.yml`) | split | LAN: dashboard reachable on the LAN; internet: `dashboard-deploy` ends `failed=0`, OIDC client registered in Hydra |
| 11 | O | Browser step: first login, admin-promote, the bootstrap NetBird token. Internet: at the public dashboard. LAN: at the LAN dashboard, with the laptop resolving the zone (the box as its DNS server, or a hosts-file pin); the driver prints the steps and skips the phase when `<SITE>_NB_API_TOKEN` is already available under the declared secrets posture | both | you can log in, the UI stops `403`-ing after the promote re-run, `<SITE>_NB_API_TOKEN` is exported — see [Phase 11 in full](#phase-11--first-login-the-browser-step) |
| 12 | A + GATE | **The declared ACL, first**: `netbird-acl-bootstrap.yml`, cutover `mode=apply`, cutover `mode=enforce` (founder only), then `netbird-policy-posture.py <site> --url https://<public_mgmt_fqdn>` (both) | both | the gate prints `OK: no All-group rule; every enabled policy is declared.` No goatnet carries a Default All→All, so until this runs enrolled peers reach nothing — see [ADR 1185](../adr/1185-no-default-all-to-all-standup-applies-declared-acl.md) |
| 12 | T + GATE | NetBird service identities: `netbird-service-identities.py <site> --mint`, then `--live` | both | the gate prints `OK: every required identity is present and verified.` — see [Phase 12](#phase-12--netbird-service-identities) |
| ∥ host | A ∥ | **Observability leg `host`** — baseline + LUKS on the obs host, then `utility-host-bootstrap.yml` (docker data-root on the NVMe, then Forgejo). Starts with phase 2, runs alongside 2–6, **joins before the phase-12 mirror**. A tree without `utility-host-bootstrap.yml` refuses before phase 1; `--skip-obs` stands the site up without its obs host | both, when an obs host is declared + inventoried | log `~/.goat/state/standup-<site>-obs-host.log` ends with `Forgejo GREEN`; at the join the driver prints its tail |
| 12 | T | Mint the pubsub-state mirror bot on the obs host's Forgejo (once; skipped when `FORGEJO_PUBSUB_TOKEN` is already set) | obs host only | the token is written to the operator envrc; the run stops so you can `direnv allow`, then `--from 12` |
| 12 | A | netbird account baseline (mesh-DNS nameserver group, no policy; **founder only** — a joiner's account is the founder's), mesh CoreDNS (internet), **portal** (`portal-deploy.yml`, before everything below that reads it), pub/sub Repl, onboarding intake, onboard-runner, **grant-keyd** (`grant-keyd-deploy.yml`, encryption key distribution) | both | account-init `failed=0`; CoreDNS resolves `*.<site>.<zone>` (then drop the laptop's `/etc/hosts` pin); the grant-keyd play ends by proving the portal answers `200` on `/api/access/release-sets` with keyd's token; the add-ons skip cleanly when their group_var is absent |
| ∥ stack | A ∥ | **Observability leg `stack`** — `obs-host-bootstrap.yml` (Prometheus + Alertmanager + Grafana). Starts after 9 (needs `obs_scrape_targets` = the core host's addresses, which an internet site only has after 8), runs alongside 10–12, **joins before 13** | both, when an obs host is declared + inventoried | the play's opening gate passes (targets declared); Prometheus on the obs host shows every target `up` — the gate's `prometheus-targets-up` row |
| 13 | GATE | Acceptance (`goatbuilder assay`) — every provisioned aspect proven live + functional | both | `run-acceptance.py` reports every eligible row PASS; read the report, not `$?` |
| 14 | A | **Rescue nodes** — the site's break-glass node (ADR 1114) and network-jump host ([ADR 1131](../adr/1131-network-jump-class-and-herd-gear-scope.md)), each converged when the site declares it | both, when declared | the declared node's play ends `failed=0`; a class the site declares it does not have prints the recorded reason and skips. A class declared `true` with no host in inventory skips as not-yet-built |
| 14 | A | Relay-health box + cross-goatnet surfaces — `ops/goatnets/cross-env-apply.sh --require <site>` deploys the goatnet's own relay-health box and redeploys every other box from `ops/goatnets/cross-env.yaml` | both | `https://relay-health.<env>.goat.example/` serves the goatnet's relays, host-net and zenoh bands; every box's `/goatnets.html` lists the goatnet, green, footer naming this run's commit |

**Every goatnet with a relay tier gets its own relay-health box, and phase 14
refuses to run without one.** The box is the cloud EC2 that probes the goatnet's
relays from outside, serves its dashboard at `https://relay-health.<env>.goat.example/`,
and receives the host-net, zenoh-health and peer-name reports the goatnet's hosts
publish. Before phase 14:

1. **Terraform:** `module "relay_health_host"` (source
   `../terraform-zagros/modules/relay-health-host`) and the `relay-health.<env>`
   record on the goat.example zone in `terraform-<site>/`, copied from
   `terraform-zagros-dev/relay-health-host.tf`. Mint a one-off NetBird setup key on
   the new goatnet with auto-group `<SITE>-INFRA` for
   `TF_VAR_<site>_netbird_setup_key`, then apply. `check-cloud-parity.sh` fails
   the tree while `relay-health-cloud-host` is neither present nor declared absent.
2. **Dex:** add `https://relay-health.<env>.${var.domain_name}/oauth2/callback` to
   `oauth2_proxy_redirect_uris` in the cloud identity tree; apply `-target=module.dex[0]`.
3. **Inventory + group_vars:** `ops/ansible/inventory/<site>-relay-health.ini`
   (instance id and mesh IP from `terraform output relay_health_host`) and
   `group_vars/<site>_relay_health/vars.yml`, copied from zagros-dev's. Secrets:
   `<SITE>_RELAY_HEALTH_OAUTH2_COOKIE_SECRET` (`openssl rand -hex 16`) and
   `<SITE>_RELAY_HEALTH_ALLOWED_EMAILS`.
4. **Roster:** the goatnet's entry in `ops/goatnets/cross-env.yaml` (box, url,
   inventory, terraform), then `python3 ops/goatnets/render-cross-env.py`. That
   writes the box's overview wiring and, into every fleet-pins member group of the
   goatnet, the URLs its hosts publish to. CI (`render-cross-env.py --check`,
   `env-coverage` workflow) fails when the roster, a box's terraform, inventory or
   group_vars, or any generated file is out of step, when a hand-written group_vars
   file sets a generated URL, and when any group_vars value names a relay-health
   host that no box serves.

A goatnet with no relay tier declares `relay_health: {absent: <reason>}` instead,
and gets no box, no card and no publisher URLs.

**Retiring a goatnet runs phase 14 in reverse.** Delete its entry from
`ops/pins/fleet-pins.yaml`, `ops/validation/goatnet-runners.yaml` and
`ops/goatnets/cross-env.yaml` in one change, re-render, merge, then run
`ops/goatnets/cross-env-apply.sh`. The redeploy removes the goatnet from every
overview and deletes its stored summary; destroy its box with its terraform tree.
Merging the change does none of that on its own: a retired goatnet stays on
every overview until the boxes are redeployed.

**Ordering is load-bearing:** Ory before account-seed; the schema wait before the
router; mesh-enroll before the router (it binds the mesh IP); cert + client before
the service. The driver encodes it so you do not have to. Account-seed is not a
phase of its own: it runs inside site-bootstrap (phases 5 and 6). `--from`/`--to`
over a phase the profile skips runs nothing.

### Phase numbers

Phases are numbered 1 to 14, whole numbers, with no gaps and no lettered
sub-phases, in both avenues (`goat-standup.sh` and `goatbuilder standup`).
Before 2026-10-29 the numbering skipped 1–3 and 8 and lettered some steps; a
run, a note or a log from before then uses the old numbers:

| Old | Now | What |
|---|---|---|
| 0 (gates, incl. the host preflight once called 0b) | 1 | gates: config, inputs, ACL, door, hosts |
| 0 (host baseline) | 2 | host baseline + LUKS |
| 4 | 3 | per-instance secrets |
| 5, 5b | 4 | leaf certs + name snapshot |
| 6, 6b | 5 | mgmt stack, relay-connect |
| 7, 7a, 7b, 7c | 6 | identity stack, re-run site-bootstrap, the joiner's seed and mirror |
| 9, 9b, 9c | 7 | public TLS + relay tier |
| 10 | 8 | mesh enrolment (manual) |
| 11, 11b | 9 | router + SPIRE |
| 12 | 10 | dashboard + OIDC clients / LAN services |
| 12m | 11 | first login + the bootstrap NetBird token (manual) |
| 13 | 12 | declared ACL, service identities, services (the `--parallel` waves) |
| 14 | 13 | acceptance gate |
| 15 | 14 | rescue nodes + cross-goatnet surfaces |

`--from`/`--to` take the new numbers; a lettered id and `0` are refused with the
phase they became. The observability legs O1+O2 and O3 are now the `host` and
`stack` legs. `goatbuilder federate` and `peering-standup.sh` number their
phases 1–11 (were 0–10).

### The obs leg — built in parallel

Nothing on the obs host's *host half* (baseline, LUKS, docker data-root,
Forgejo) depends on the core host, so the driver starts it with phase 2 and lets
it run while the core goes through 2–6. The *stack half* scrapes the core
host, so it starts once the router is up. Each half is a background job with
its own log; the driver joins it at the first phase that needs it — Forgejo
before the phase-12 mirror deploy, the obs stack before the acceptance gate —
prints the log tail, and fails the run there if the leg failed. The legs are
idempotent: `goat-standup.sh <site> --obs-only` re-runs them in the foreground
(after filling `obs_scrape_targets`, after a fix, or after a **Docker
data-root rebuild on the obs host** — Forgejo's containers live in that root
and nothing else re-creates them, F-504);
`--skip-obs` defers the whole leg. `--plan` prints both halves labelled `∥`.

The driver decides for itself whether there is an obs host to build — the four
conditions in prep step 6 — and says which one is missing when there is
not. It never errors on a declared-but-not-yet-racked obs host: that is the
normal state of a site whose third box is on order, and the gate's obs rows
read `N/A` until it arrives.

---

### Second host site — the joiner path

A site whose `group_vars` declares `goatnet_founder: false` (ADR 1084) is a
*joiner*: it attaches to an existing goatnet instead of originating one, and the
driver runs the three-pass Spock join in place of the account seed. Set
`goatnet_founder: false` and `join_founder_site: <founder-site-id>` in the new
site's `vars.yml`; the driver reads both from config, never the command line —
setting it wrong seeds a second account into a replicated schema.

What the driver does differently, inside the same phases:

- **Pass 1** (phase 5) runs `site-bootstrap` with `participates_in_spock=false` —
  the account and schema are not seeded locally, they arrive by replication.
- **Pass 2** (phase 6, joiner-only) runs `<site>-seed-from-peer.yml`, the
  guarded data-only copy of identity + mesh state from the founder.
- **Pass 3** (phase 6) re-runs `site-bootstrap` with Spock enabled, creating the
  `netbird_mgmt` subscriptions.
- **Founder-side mirror** (phase 6, joiner-only) runs `spock-mesh-wire.yml` on
  the founder so the subscriptions exist in both directions.

One prerequisite the driver cannot supply and the config must: the founder's
`group_vars` must name this joiner as a peer (`inter_site_spock_peers` +
`spock_mesh_publisher_host` set to the joiner's mesh IP) before the mirror runs.
The full joiner mechanics, and why the passes cannot be reordered, are in
node-addition.md §4.

## Phase 8 — mesh-enroll, in full

*(internet profile only — a LAN host's router cert is already final from phase 4,
bound to the LAN IP, so there is no phase 8 on LAN.)*

This is the one genuinely operator-fired step and the one with the most ways to
half-succeed, so run the preflight first — it **prints** the exact `netbird up`
line for this GOAT-Net rather than describing it:

```
ops/preflight-enrollment.sh          # note: ops/, not ops/scripts/
```

It checks the netbird CLI is installed, no stale `wt0` is left over, the setup key
is format-sane, the management URL answers, and the clock is within 60 s of the
server (gross skew kills the TLS handshake with an error that names neither the
clock nor the cert).

Then enroll on the management host:

```
sudo netbird up --management-url https://netbird.<zone>:443 --setup-key <the key from phase 6>
```

**Confirm enrollment before you do anything else** — both failure modes below are
silent, and the next step mints a certificate you do not want to mint onto a host
that never joined:

```
sudo netbird status          # want: Management: Connected, and a 100.x IP on wt0
```

Three things that have each cost a standup:

1. **The apex and `--management-url` port must agree**. If the name
   resolves to the relay path, use `:443`; if it resolves to the host itself, use
   `:8443`. A mismatch enrolls the peer against the wrong entrypoint, gets a 404
   from the management API, and the host silently drops off its own mesh. Never
   point the name at loopback via `/etc/hosts`.
2. **If the GOAT-Net sets `rosenpass_enabled: true`, append `--enable-rosenpass
   --rosenpass-permissive`.** Omit them and the peer enrolls PQ-less and silently
   stays that way; the `rosenpass-pq-negotiated` acceptance row is what catches it.
3. **Re-mint the router cert against the real mesh IP.** Fill the mesh IP into
   `inventory/mgmt-hosts-mesh.ini` and the cert manifest, then (the delete is
   required — the mint skips a leaf whose file already exists):
   ```
   rm -f ~/.goat/state/<site>-leaves/router-{cert,key}.pem
   ops/scripts/mint-site-certs.sh <site> --only router
   cp ~/.goat/state/<site>-leaves/router-{cert,key}.pem ops/enrollment/<site>/
   ```

Resume the driver with `--from 9`.

## Phase 9 — the router is bound, not just up

`failed=0` and a healthcheck are process-up signals; confirm the router is
actually bound and not crash-looping on a missing volume:

```
ssh $SSH_USER@$HOST_IP "sudo docker ps --filter name=<site>-zenoh-router --format '{{.Status}}'; \
  sudo ss -tlnp | grep 7447; \
  sudo docker logs <site>-zenoh-router --since 10m 2>&1 | grep -c 'Cannot find volume'"
```

Expect `Up (healthy)`, a `7447` listener on the intended IP (mesh `100.x` on
internet, the LAN IP on LAN — not `0.0.0.0`), and `0` volume errors. That the
fabric actually carries messages is proven by the acceptance gate and `goat top`,
not here.

## Phase 11 — first login, the browser step

The one NetBird credential a person has to create by hand is the goatnet's
bootstrap service identity, because it can only be made after someone signs in
to the dashboard. Phase 12 mints every other identity from it.

1. Enroll **your laptop** onto the mesh (`netbird up …`), pin the three dashboard
   hosts in `/etc/hosts`, and read the one-time password from
   `/opt/mgmt-host/.bootstrap-passwords/<OWNER_EMAIL>.txt` on the host.
2. Log in at `https://dashboard.<site>.<zone>`.
3. Re-run the phase-10 `dashboard-deploy` line **once** — it promotes your fresh
   account to admin (the UI `403`s until then).
4. **Team → Service Users → Create Service User**: name `svc-<site>-netbird-infra`,
   role admin. **Create Token** on it. Add
   `export <SITE>_NB_API_TOKEN=<token>` to `.envrc.local.d/30-site-services.envrc`
   and store the token where the declared secrets posture says.

Resume with `--from 12`.

---

## Phase 12 — NetBird service identities

A goatnet's services read more than one NetBird token, and each belongs to a
named service user with its own revocation. `ops/netbird/service-identities.yaml`
lists them; this goatnet's `group_vars` decide which it needs. Phase 12 creates
the missing ones and refuses to continue until each required token exists and
authenticates as its service user.

```
ops/scripts/netbird-service-identities.py <site> --mint   # creates only what is missing
ops/scripts/netbird-service-identities.py <site> --live   # the gate
```

`--mint` creates each missing service user (role admin), mints a 365-day token,
appends the export to `.envrc.local.d/30-site-services.envrc` and stores it in
the secret-store item the registry names. It never prints a token and never
replaces one already set. The gate reports, per identity, whether it is
required and why, the service user, the variable, and whether the token
authenticated as that user:

| Identity | Service user | Variable | Required when |
|---|---|---|---|
| `netbird-infra` | `svc-<site>-netbird-infra` | `<SITE>_NB_API_TOKEN` | always (minted in phase 11) |
| `portal-mint` | `goat-portal-mint` | `<SITE>_PORTAL_NB_SVC_TOKEN` | `portal_mesh_key_mint_enabled` |
| `console-enrollwatch` | `svc-console-enrollwatch` | `<SITE>_ENROLLWATCH_NB_TOKEN` | `id_admin_console_enabled` |
| `provenance-guard` | `svc-<site>-provenance-guard` | `<SITE>_PROVENANCE_GUARD_NB_TOKEN` | `machine_provenance_guard_enabled` (default true) |
| `membership-reconcile` | `svc-<site>-membership-reconcile` | `<SITE>_NB_RECONCILE_TOKEN` | `mesh_membership_reconcile_enabled` (default true) |

The registry is the source of truth; this table is a reading of it on
2026-09-16. Every role that reads one of these tokens also refuses to deploy
while its token is empty, so skipping phase 12 stops the next deploy instead of
shipping a service without its capability. Design:
[`netbird-service-identities.md`](../design/netbird-service-identities.md).

**LAN sites** take the same browser step at the LAN dashboard: the laptop
resolves the zone through the box (set it as the laptop's DNS server, or pin the
dashboard, login and enrolment names to the box's address in the hosts file),
and the token goes where the declared secrets posture says
(`ops/secrets/posture.py hint <SITE>_NB_API_TOKEN`). The driver prints the steps
and pauses; when the token is already available under that posture it skips
phase 11 and says so.

## Verification — done means green, proven

The gate resolves every probe against a value set **derived from this goatnet's
own config** — phase 13 runs `generate-values.py <site>` first (reading
`group_vars` + the inventory: the zone, `mesh_ip_a`, the host address, the
`has_obs_host` / `intra_site_pair` / `name_snapshot_enabled` flags) and writes
`ops/validation/values-<site>.yaml`, then runs the suite against it. Without that
step every row resolves to `<UNRESOLVED>` and the gate renders no verdict at all.
To run it by hand:

```bash
python3 ops/validation/generate-values.py <site>
python3 ops/validation/run-acceptance.py --env <site> \
  --values ops/validation/values-<site>.yaml --all-eligible
```

The standup is complete when every applicable row is `PASS`. A probe that cannot
run is `UNKNOWN`, never a pass — read the report, not the exit code (`$?` is `0`
unless a row FAILs). Each row cites the defect it catches; the suite is both the
phase-13 gate and the standing regression surface.

**The obs host is graded too.** With `has_obs_host: true` the observability rows
(targets up, rules loaded, Alertmanager reached, deadman firing) apply, and with
a `pubsub-state.yml` group_var the `pubsub-state-forgejo-primary-up` row requires
the state-of-record primary to *answer* on the obs host — the mirror timer being
`active` proves nothing about it.

**Service coverage is exact, so a goatnet cannot finish missing services.**
`declared-service-actually-deployed` checks that the container behind *every*
service you declared in `isitup_services` is running on the host — one hard FAIL
per missing container, naming it — and `every-declared-service-resolves` checks
each one answers in the mesh DNS. Declaring a service you never deployed (or that
lost its route) fails the gate, by name; "stood up" therefore means the gate is
green, not that the ansible finished. An expected-`UNKNOWN`/`N/A`
list is normal — rows for hosts you do not have (a single-host site is `N/A` on
the observability and intra-site-pair rows), probes that need a live enrolled peer
or SSH to the host, or checks whose probe is not yet self-contained — and the
report names each with its reason. Design:
[`goatnet-acceptance-suite.md`](../design/goatnet-acceptance-suite.md).

**The gate also checks the public login surface is not flagged as dangerous.**
`public-auth-fqdns-not-flagged` runs the enrollment apex and every `access: public`
surface (`generate-values.py` derives the list into `public_auth_fqdns`) through
Google Safe Browsing. A fresh public login/consent domain gets flagged as social
engineering — Google shows enrollers a full-page red interstitial before they ever
reach the portal, and it reads as a phishing verdict, not the TLS error operators
go looking for. Grading it needs `SAFE_BROWSING_API_KEY` (a Google Cloud key with
the Safe Browsing API enabled) in the runner's environment; without it the row is
`UNKNOWN`, never a false pass. A `FAIL` names the flagged host — file a review
request in Google Search Console for the zone. Two things keep the flag from
happening in the first place: the zone pre-step below, and not exposing more auth
surface publicly than enrollment needs — a `lan`-profile goatnet has no public
login surface for Google to flag at all, and even on `internet` the only surfaces
that must be public are the enrollment apex and `dashboard`/`auth`.

---

## Troubleshooting / when you hit a gap

Until the cold-read drill passes clean, expect gaps. The contract: **any step
that needs improvisation or outside knowledge is a defect in the procedure, not
operator error.** For each gap — write down the phase, the exact command, the
exact error, and what you did instead; file it against
`standup-remediation-plan.md` with its
class; and if the cause was a wrong default, a missing config file, two settings
that must agree, or a silent failure, add the check to `standup-config-preflight.py`
or a row to the acceptance suite so the next standup cannot hit it. Known upstream
defects are tracked as issues (#382 and the ones it links), not in this document.

Quick triage for the most likely failures:

| Symptom | Likely cause | Fix |
|---|---|---|
| `ansible-preflight` refuses to run | checkout behind or diverged from the remote main, a locally modified safety-critical playbook, or an unreachable registry namespace | read its message — the checkout case prints the exact `git pull` |
| preflight: `no group_vars dir` | group name mismatch | directory must be exactly `<group>` (underscores) |
| `manifest not found` in phase 4 | cert manifest not authored | prep step 7 above |
| `ansible-preflight`: "at least one live pin does not exist under that namespace" | the primary registry namespace is missing images that exist on the mirror | `export GHCR_ORG=goat-maintainer` in `00-site-identity.envrc`, `direnv allow`, re-run |
| a command fails on a variable you just added, and `direnv: loading` prints *after* the failure | the shell reloads direnv only at the next prompt, so the command ran with the old env | press Enter once, confirm with `echo $<var>`, re-run |
| phase 5 stops at `wait for the netbird schema` and the play says "management-a cannot reach Hydra (hydra:4444)"; `management-a` restart-loops on `hydra:4444: connection refused` | the management service needs the identity stack, which phase 6 installs — expected on every first standup | run phase 6, which also re-runs the site bootstrap (the re-run of 6) — its last step prints the setup-key banner |
| phase 9 fails: `range of CPUs is from 0.01 to N.00, as there are only N CPUs available` | the router's compose CPU cap exceeds the host's core count | set `goat_zenoh_cpus` below the core count in the group vars, re-run phase 9 |
| phase 9 fails pulling the zenoh image: `permission_denied: read_package` | the token cannot read that package under the org your checkout resolved | pull the other org from the host (swap `usareur-af`/`goat-maintainer`); if it works, pin `…_zenoh_image` to that org's digest and re-run |
| phase 9 refuses with GB-071, naming `~/.goat/state/<site>-leaves/router-cert.pem` (phase 1's input scan and `goatbuilder doctor --site` report the same line before any host is touched) | the router pair is not in the site manifest's `out_dir`, the one place plays read leaves from (the `goat_leaf` filter, #2165; a copy staged at `ops/enrollment/<site>/` by an older mint still counts) | `ops/scripts/mint-site-certs.sh <site> --only router` (a leaf already there is skipped), re-run |
| `direnv allow` prints `SITE: No such file or directory` (or `SITE_UPPER`, …) | a `<placeholder>` was pasted literally — the shell reads unquoted `<…>` as a redirection | replace every `<…>` with your real value; none may remain in an envrc file |
| enrollment hangs / host off mesh (internet) | the URL and the DNS answer do not agree as a pair | Phase 8, item 1 |
| router serves wrong cert / crash-loops (internet) | router minted before enrollment, or a stale key | delete both router pem files, re-mint `--only router` (Phase 8, item 3) |
| **Cloudflare DNS-01 (phase 7):** issuer stays `TRAEFIK DEFAULT CERT` | the token is missing `Zone:DNS:Edit`, or is scoped to the wrong zone | fix the token scope, re-run phase 7 |
| dashboard loads but every panel is empty and `/api/policies` returns `403` | your account exists as a plain user — admin comes from the promote re-run | re-run the phase-10 dashboard-deploy once (Phase 13, step 3), refresh |
| login submits, then a 500: "Unable to accept OAuth 2.0 Login Challenge" | a login flow whose challenge expired — usually a stale tab | close the tab, start fresh from `https://dashboard.<site>.<zone>` |
| browser `403 Forbidden` on `dashboard.`/`auth.<site>.<zone>` | the login-surface IP allowlist (`lan_allowlist_cidr`) still carries the reference site's subnet | set it to your subnet in the group vars, re-run the phase-12 play |
| phase 13 prints many `UNKNOWN … unresolved vars` and FAILs against hosts you don't have | the gate ran without `--values`/`--deployed` | author the values file and pass the deployed list, re-run |
| any command errors `No such file or directory` on an `ops/…` path that exists | the terminal is not at the repo root (a previous block `cd`'d into a subdir) | `cd "$(git rev-parse --show-toplevel)"`, re-run |

---

## Related docs — none of these is this procedure

| Doc | Role |
|---|---|
| [`goatnet-buildout-overview.md`](goatnet-buildout-overview.md) | the build-out overview — big picture, host-node hardware, and time expectations; read before this |
| [`goatnet-standup-walkthrough.md`](goatnet-standup-walkthrough.md) | a phase-by-phase companion — plain + engineer read of each phase, the founder/joiner fork, and the per-phase acceptance criterion |
| [`goatnet-deployment-doctrine.md`](../design/goatnet-deployment-doctrine.md) | the principles that keep this one path from splitting — read before changing anything |
| `goatnet-turnkey-standup.md` | the *why* of the ordering + the automation design |
| `standup-remediation-plan.md` | the gap map (defect classes → mechanisms) governing what is left to build |
| `<env>-standup-runbook.md` (e.g. zagros-prod) | one filled-in execution instance with per-env values, relay-tier setup + hazards |
| `*-bringup.md` (nova, hobby, …) | historical per-env records — read for values, not sequence |
| `node-addition.md` | the adjacent job — the GOAT-Net already exists and one more node joins it |
