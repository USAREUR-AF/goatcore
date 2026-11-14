# goatbuilder speed — a goatnet in an hour, a node in fifteen minutes, a burn-down in fifteen

| Field | Value |
|---|---|
| Status | **Built; the hour not yet measured on hardware.** Built: SSH multiplexing for direct hosts, per-phase timing, a static required-input scan as a phase-1 gate, phase 12 in dependency waves (`--parallel`), the bot-token pause removed, phase 8 enrolment as a scripted step, a parallel pre-pull of every pinned image, the site cache on the obs host, and a teardown with a built way down for every step that is not refused, in both avenues. The autoinstall seed per node class is built and **Status: WIP, not for production use — gate: a timed host baseline on a seeded Karbon 801 against a stock one, recorded as a measurement** |
| Design | [goatbuilder.md](goatbuilder.md) §"Standing up a goatnet in an hour" · [phoenix-architecture.md](phoenix-architecture.md) · [ADR 1201](../adr/1201-two-avenues-scripts-documented-cli-supported.md) (both avenues) |
| Targets | founder site declared → verified ≤ 60 min · one new node ≤ 15 min · burn-down to nothing, records kept ≤ 15 min |

## Where the day goes

Past buildouts took a day or more. Most of that time was not Ansible running.
It was a run stopping on a defect, the defect being fixed on main, and the run
resuming. Each defect cost a stop, a diagnosis, a pull
request and a resume. The runtime itself is the second cost.

Ranked by the minutes each costs a founder standup:

| # | Time sink | Cost before | Lever and status | Saving |
|---|---|---|---|---|
| 1 | Defects found during the run, one stop each | hours to days; the inchworm skill's case: 16 runs of a 5-min play to find 12 missing inputs, 80 min | **built**: `goatbuilder doctor --site <s>` and the phase-1 gate in both avenues scan every play the plan reaches for inputs it requires and secrets it reads, in one pass | **measured**: one scan takes 6–13 s per site. **Modelled**: each input it names is one stopped run avoided |
| 2 | A fresh SSH connection per Ansible task | 62–133 min | **built**: multiplexing for direct hosts | **modelled** from M-120's measured per-call cost: 14–30 min remain |
| 3 | Human pauses | each one waits on a person; any absence adds hours | **built**: the bot step reads its token and continues; phase 8 enrols the hosts and re-mints the router certificate with no one at the keyboard. Phase 11 stays a person: the first NetBird token needs a browser login | two stops removed; phase 11 is the one left |
| 4 | Serial phase-12 steps | 14 steps one after another | **built**: every step declares `needs:`; `--parallel` runs them in 6 waves | **modelled**: 14 step-durations become 6, about 8 min at a minute a step |
| 5 | Host baseline + LUKS per host (215 tasks) | 10–20 min per host (**modelled**) | **built, WIP**: an autoinstall seed per node class (`node-seed.py`); unproven until a seeded host is timed | **modelled**: 3–5 min per host |
| 6 | Image pulls per role | unmeasured | **built**: every pinned image pulled on the box in parallel between phases 2 and 5; the obs host runs apt-cacher-ng and a registry mirror, and a core host that declares `site_cache_host` uses both, with fallback to upstream | pre-pull: phases 3–4 overlap the pulls. Cache: apt and Docker Hub pulls on rebuilds and joiners, not the first standup; ghcr.io images do not go through it (below) |
| 7 | Fact gathering per play | small | `gathering = smart` with a one-hour cache | none needed |

Hosts in one play already run together: `ansible.cfg` sets `forks = 10`, and the
obs leg runs beside the core host. Sink 5's saving is per host, not per site.

## The budget

| Phase | Minutes | Basis |
|---|---|---|
| 1 gates, preflight, required-input scan, door | 3 | **measured**: the scan takes 6–13 s |
| 2 host baseline + LUKS (hosts together; installer seed per class) | 10 | **modelled**, sink 5 |
| 3–4 secrets, leaf certs, name snapshot | 2 | **modelled** |
| 5–6 mgmt stack, relay connect, identity | 15 | **modelled** |
| 7 public TLS and relay | 5 | **modelled** |
| 8 enrolment (scripted) | 3 | **modelled** |
| 9 router and workload identity | 5 | **modelled** |
| 10–11 dashboard, OIDC clients, first admin and token | 4 | a person, phase 11 |
| 12 services in 6 waves | 8 | **modelled**, sink 4 |
| 13 acceptance | 3 | **modelled** |
| 14 rescue classes and cross-goatnet surfaces | 2 | **modelled** |
| **Total** | **60** | |

The first timed run on hardware replaces the modelled rows. A new node is the
phase-1 and class-converge subset: ≤ 15 min. A burn-down of the services is
phase 12 in reverse: ≤ 5 min (**modelled**). `--services` from phase 1 also stops
the router, SPIRE, the dashboard, the relay tunnel and the obs stacks; no timed
run backs a number for that yet. The NetBird account, the identity store and the
database come down only by a person's hand.

## What is built

### The required-input scan (sink 1)

`ops/scripts/standup-required-inputs.py <site>` reads the plan that
`goat-standup.sh <site> --plan` prints. It follows every play, role, included
task file and meta dependency the plan reaches. It collects the variables each
one requires (`assert`, `| mandatory`) and the environment each one reads. Each is
checked against the site's group_vars, host_vars, role defaults and role
parameters. Secrets are checked through the secrets posture (ADR 1204), the same
way `goatbuilder doctor` checks them. A requirement inside a `when:` it cannot
evaluate is reported as possible, not as missing. It runs as the phase-1 gate in
both avenues and as `goatbuilder doctor --site <s>`.

| Code | Meaning |
|---|---|
| GB-060 | an input the run will reach is required and not set |
| GB-061 | a secret the run reads is one the secrets posture cannot supply |
| GB-062 | an input may be required: its condition cannot be decided before the run |
| GB-063 | a play's `--limit` selects no host, so the play does nothing |

Measured on 2026-10-29, with the secrets check off:

| Site | Requirements checked | Missing | Time |
|---|---|---|---|
| zagros-dev | 91 | 0 | 5.9 s |
| zagros | 101 | 0 | 8.4 s |
| zagros-prod | 132 | 0 | 13.1 s |
| fixture `ops/goatbuilder/fixtures/required-inputs` | 6 | 3, plus 1 secret | 0.1 s |

The scan found a real defect before it found zero. The relay-connect step of phase 5 on zagros runs
`zagros-rathole-client.yml`, whose hosts line requires `rathole_client_target`.
Neither avenue passed it, so every zagros standup stopped there. Both avenues now
pass the site's group.

### Phase 12 in waves (sink 4)

`ops/goatbuilder/standup-dag.yaml` names each phase-12 step, the steps it needs
and why. A step's wave is one more than the latest wave it needs. A need on a
step the run skips counts as met.

| Wave | Steps |
|---|---|
| 1 | ACL groups |
| 2 | ACL policies, service-identity mint, account baseline |
| 3 | ACL enforce, service-identity gate, mesh DNS |
| 4 | ACL gate |
| 5 | portal |
| 6 | pub/sub mirror, intake, runner, grant-keyd, synthetic publishers |

The dry run prints each step's `needs:` in both avenues, and the standup parity
gate compares them. Without `--parallel` the run is serial as before. With it,
each wave's steps run together with one log each. A failed step stops the run
after its wave, and the message names every failed step and its log.
`standup-dag.py --check` refuses a cycle or an unknown need.

### Pauses that became inputs (sink 3)

The bot step in phase 12 used to stop the run so a person could export
`FORGEJO_PUBSUB_TOKEN` and resume. It now reads the token from the operator's
envrc with `ops/scripts/envrc-var.sh` and continues. `goatbuilder` does the same,
and keeps the token as the run's own state (`RunState` in `crates/goatbuilder/src/standup.rs`):
only the phases that read it, the mint step and the mirror play, receive it, and
every other phase child is started without it (built).

### Phase 8 enrolment as a scripted step (sink 3)

Phase 8 was a person: run the `netbird up` line `ops/preflight-enrollment.sh`
printed, check the peer, fill the mesh IP into inventory and re-mint the router
certificate. It is two steps now, in both avenues, on an internet site:

| Step | What it does |
|---|---|
| 8 A `mesh-enroll-host.yml -e target=<site>_hosts` | per host, one at a time: nothing if `netbird status --json` already shows the management connection and a mesh IP. Otherwise it checks the site's declared `mesh_enrollment_url` against where the host resolves the name and refuses a pair that disagrees; mints a single-use key (one use, one hour, role bootstrap) with `mgmt-host-bootstrap create-setup-key` on the site's first host; hands it to `netbird up --setup-key-file` through a 0600 file under `/run`, removed afterwards; adds the rosenpass flags when `rosenpass_enabled`; waits up to `mesh_enroll_wait_seconds` (180 s) for the connection and a mesh IP. It writes each host's IP to `~/.goat/state/standup-<site>-mesh-ips.json` |
| 8 T `standup-mesh-record.py <site> --ips <file>` | `mesh_ip_a` / `mesh_ip_b` in the site's group_vars (the manifest's mesh-IP placeholders resolve from them), each host's `ansible_host` in its `mgmt-hosts-mesh.ini` line (a host with no line keeps its transport), a literal old IP in the manifest's router leaf, then the router pair re-minted. A second run with the same IPs mints nothing |

`join-node` runs the same two steps for one node at its phase 10. For a joiner it mints on the founder's first host (`mesh_enroll_mint_group`), with the groups that host's peer holds (`mesh_enroll_groups_from_peer`; the query is designed and not measured on a live database). `standup-mesh-record.py` sets `mesh_ip_a`/`mesh_ip_b` by each host's place in the inventory, so recording one node leaves the other's address alone. See [goatbuilder.md](goatbuilder.md), join-node part "(f)".

`mesh_enrollment_url` is declared per site in group_vars. The key
is minted on the first host because that is where the NetBird database is; for
the second host of a pair it crosses the Ansible connection under `no_log`.
Phase 11 stays a person: the first NetBird token needs a browser login by the
first admin.

### Image pre-pull and the site cache (sink 6)

| Part | What it does |
|---|---|
| pre-pull | `standup-image-pull.yml` starts after phase 2 and is joined before phase 5: every image the site's pins name, pulled on the site's hosts four at a time, every failure named in one run |
| site cache | the obs leg's host play (from phase 2) applies the `apt-cacher-ng` (:3142) and `docker-registry-mirror` (docker.io :5000, ghcr.io :5001, quay.io :5002) roles on the obs host's NVMe |
| cache client | a core host whose group_vars set `site_cache_host` (the obs host's LAN address) gets it as Docker's registry mirror in `daemon.json` (docker-engine role) and as apt's proxy through a probe (`site-cache-client` role, first in `mgmt-host-baseline.yml`) that answers the cache's URL when it accepts a connection and `DIRECT` when it does not. Unset, neither file changes, so no daemon restarts |

Docker has a mirror setting for Docker Hub only. The goat images on ghcr.io
therefore pull from ghcr.io on every host; the pre-pull makes those pulls
parallel, and the cache serves apt packages and Docker Hub images. A rebuild
after a burn-down, and every node that joins, finds the cache warm; the first
standup of a site fills it.

### The autoinstall seed per node class (sink 5)

**Status: WIP, not for production use — gate: a timed host baseline on a seeded
Karbon 801 against a stock one, recorded as a measurement.**

A cloned disk image is ruled out: a cloned LUKS header gives every host the same
volume key, and host-site-node-disk-encryption.md
starts from LUKS created by each host's own installer. Ubuntu Pro attachment and
FIPS enablement are per machine with a token and a reboot.

`ops/goatbuilder/node-classes.yaml` declares the class `x86-tpm2` (the Karbon 801
core and obs nodes). `ops/scripts/node-seed.py <site> --host <h> --class x86-tpm2`
writes a NoCloud seed (`user-data` + `meta-data`, 0600) to
`~/.goat/state/seeds/<h>/`:

| Part | Where it runs |
|---|---|
| LUKS-encrypted LVM on the class's disk (the largest NVMe, or `--disk-serial`): the `crypto_LUKS` → `ubuntu-vg` layout the LUKS role asserts, with the host's `<HOST>_LUKS_PASS` from the secrets posture | the installer |
| the class's packages plus the `luks-unlock` and `tang-server` default lists, read at generation time; through the site cache when `site_cache_host` is declared | the installer |
| `goat`, key-only SSH from `PLATFORM_ADMIN_SSH_PUBKEYS`, passwordless sudo, a locked password | the installer |
| `/etc/goat/node-seed.json`: the class and the seed's hash | a late command |
| Pro attach, FIPS enable, the TPM2 or Tang unlock slot | the existing plays, unchanged |

The seed holds the LUKS passphrase, so it belongs on the install media only. The
baseline play then converges a host that is already most of the way there. The
Raspberry Pi 5 (`tang_initramfs`) has no class: it installs from the Raspberry Pi
image, which has no autoinstall.

### Teardown

`ops/goatbuilder/teardown.yaml` declares how each standup step comes down. Both
`goatbuilder teardown` and `ops/scripts/goat-teardown.sh` read it, and the
teardown parity gate compares their plans.

| Action | Meaning | Steps |
|---|---|---|
| compose-down | stop the compose projects, volumes kept, proved by `docker compose ps -q` | synthetic publishers, grant-keyd, runner, intake, portal, mesh CoreDNS, the zenoh router (`/opt/<site>-zenoh`), the obs stack and Forgejo (obs host) |
| service-stop | stop named services inside a shared compose project, containers and volumes kept, proved by no running container of those services | the dashboard and SPIRE in `/opt/mgmt-host` |
| systemd-stop | stop and disable units, unit files kept, proved by no active unit | the relay tunnel (`rathole-client@*.service`) |
| command | run a line on the operator machine; `--services` leaves it in place | the cross-goatnet surfaces: `cross-env-apply.sh --absent <site>` refuses while `cross-env.yaml` lists the site, then redeploys every relay-health box without it |
| keep | nothing to reverse | gates, phase-8 enrolment, the pub/sub mirror, the TLS certificate, the host baseline, the rescue nodes |
| refused | reversing it destroys data | the NetBird account rows (the mesh-DNS nameserver group among them), service identities, ACL, OIDC clients, Ory, the mgmt stack, the relay tier |

A full teardown that reaches a refused step refuses with exit 3 before touching
anything; one whose range reaches none (`--from 13` takes phase 14 only) runs.
`--services` runs every compose-down, service-stop and systemd-stop step in
reverse order and lists the rest as left in place. Each stop appends a row to
`/var/lib/goat/<component>/teardown.jsonl`. `deploy-history.jsonl` is never
touched, so rollback's history is unchanged.

Planned on zagros-dev, dry run only:

| Command | Result |
|---|---|
| `goatbuilder teardown zagros_dev --site zagros-dev --dry-run` | refused: 7 steps hold data |
| `… --services --dry-run` | 10 steps come down, 8 left in place |
| `… --services --from 12 --dry-run` | 6 steps come down, 3 left in place |

The hermetic e2e (`ops/e2e/hermetic.sh`) runs `teardown-service-stop.yml` and
`teardown-compose.yml` against the dev canary on a container host: nothing of it
runs afterwards, two `teardown.jsonl` rows are written, and the deploy ledger's
row count is unchanged. The fixture rehearsal (`goatbuilder --mock`) verifies 5
components on 4 machines, appends 8 teardown records, and finds all 8 already
torn down on a second run.

The refused steps stay refused. Destroying a NetBird account, an identity store
or the database is a decision a person makes by hand, not a teardown flag.

## What remains

| Item | What proves it |
|---|---|
| phase 8 on hardware | a standup of an internet site whose hosts are not yet enrolled: both hosts connected, the router certificate carrying their IPs, no stop at phase 8. The play has run in no live run; CI checks its syntax, the plan lines in both avenues and the recorder's unit tests |
| the seed | a Karbon 801 installed from `node-seed.py`'s seed, its baseline timed against a stock one, recorded as a measurement |
| the site cache on hardware | the obs leg's host play on an obs host, then a core host with `site_cache_host` set pulling a Docker Hub image and an apt package through it. The client's fallback is proved in the hermetic e2e; the obs-host roles have not run in this shape |
| `teardown-systemd.yml` | a run against a host with the relay tunnel up; the hermetic container hosts have no systemd |
| ghcr.io through the cache | Docker reads no mirror for ghcr.io; serving goat images from the obs host needs the containerd image store's per-registry hosts, not built |
| a goatbuilder avenue for the seed | `node-seed.py` is a script only; the seed is not a standup phase |
| the hour | the first timed standup on hardware (below) |

## Both avenues

Every lever lives in both drivers: the scan gate, the `needs:` lines and waves,
the bot-token read, the rathole target, the image pre-pull, the two phase-8
steps and the teardown. The site cache is in the plays both drivers run, and the
seed generator is a script both can call. The shared facts live in
data files both read: `standup-dag.yaml` and `teardown.yaml`. The parity gates in
`goatbuilder-pr.yml` compare the standup plans, the
teardown plans, and the federate and deploy plans.

## The laptop stays awake

A run from a Mac holds an idle-sleep assertion for its own lifetime: the long
goatbuilder verbs spawn `caffeinate -i -w <pid>` (`crates/goatbuilder/src/awake.rs`)
and the drivers source `ops/scripts/lib/keep-awake.sh`, which does the same.
Without it a Mac that idles into sleep drops the mesh and stalls the play; a herd
deploy on zagros-dev took 1095 s instead of 145 s that way (#2122). One assertion
per run: `GOAT_KEEP_AWAKE` passes the holder's pid down, so a script that calls
the binary, or the reverse, does not start a second. `GOAT_NO_CAFFEINATE=1` opts
out; off macOS nothing changes.

## Measuring the hour

The first timed run on hardware is the number that matters:

```bash
goatbuilder standup <env> --site <site> --parallel --yes
cat ~/.goat/state/standup-<site>-timings.tsv
```

Record the total and the per-phase rows as a measurement. The slowest three
phases are the next three levers.

What is inside the clock and before it, how phase 11 is timed on its own, and
what a measurement records are in
[goatbuilder-hardware-measurement.md](goatbuilder-hardware-measurement.md).
