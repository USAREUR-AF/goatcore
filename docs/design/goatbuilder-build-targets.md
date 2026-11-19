# goatbuilder build targets: platform families, target types and sensors

| Field | Value |
|---|---|
| Status | Designed, not built ([ADR 1257](../adr/1257-every-machine-kind-is-a-goatbuilder-build-target-type-declared-as-data.md), Proposed) |
| Scope | What goatbuilder builds a machine into, for every kind of machine a goatnet runs; the hardware families it prepares; the per-sensor relays it turns on; how each is checked and proven |
| Related | [goatbuilder-v1-gate.md](goatbuilder-v1-gate.md) group J; [deployment-e2e.md](deployment-e2e.md) ([ADR 1200](../adr/1200-deployment-e2e-three-tiers.md)); [ADR 1141](../adr/1141-four-node-host-site-baseline.md), [ADR 1183](../adr/1183-rescue-nodes-are-part-of-the-site-baseline.md), [ADR 1114](../adr/1114-breakglass-node-class-and-two-path-reach.md), [ADR 1131](../adr/1131-network-jump-class-and-herd-gear-scope.md); [`ops/bootstrap/README.md`](../../ops/bootstrap/README.md) |

## 1. What this is for

A goatnet runs several kinds of machine: the core and observability nodes of a host site, a break-glass node, a network jump box, boundary nodes, a processing node, and edge machines that front sensors. goatbuilder builds some of them, each by its own code path, and none from the bare machine.

This design keeps two things apart, and treats sensors as a property of a machine:

| Concept | What it is | Examples |
|---|---|---|
| **Platform family** | Hardware that can play the same roles, and its preparation: firmware, boot media, the OS image, disk encryption and how it unlocks. The golden path lives here. A model joins a family; no target type names a model. | host-class (Karbon 801 and 802, Jetson Orin and Thor), cloud (AWS EC2; Azure VMs and Google Compute Engine), small computer (Raspberry Pi 5 today; a smaller OnLogic model later), ruggedized mobile edge (TSI V3 and TSI V3 mini) |
| **Target type** | The role the machine plays in a goatnet: its plays, services and acceptance checks | host site: core, obs, break-glass, network jump box. Edge: octo-pod (the edge gateway appliance), sensor node, first-bite (the tap decoder). Boundary: federation boundary, simple boundary. Processing: goat-processors. Also phone |
| **Wired sensors** | Declared per machine. For each one, the sensor kit's matching kid (a per-sensor relay process) is turned on, on any type whose file allows sensors | a radar wired to an octo-pod; a camera wired to a sensor node; no sensors, so no kids |

goatbuilder builds a target type on a machine of an accepted family, and turns on the kids for the sensors declared on that machine. Families and types are declared as data, and a check confirms the end state whatever path got the machine there.

## 2. What exists today (measured on `main`, 2026-11-19)

### 2.1 Three taxonomies for one question

Three places answer "what kind of machine is this", and none reads the others:

1. **`join-node`'s class list**, compiled in. `crates/goatbuilder/src/joinnode.rs:90` declares `CLASSES = ["core", "obs", "breakglass", "network-jump", "phone"]`. Each class has its own match arm, at `joinnode.rs:583` (core), `joinnode.rs:586` (obs), `joinnode.rs:602` (breakglass) and `joinnode.rs:608` (network-jump). Any other class is refused with GB-247 (`joinnode.rs:614`). A new kind of machine is new Rust.
2. **`standup`'s group_vars flags.** The founder standup does not read that list. It reads one flag per optional machine from the site's `group_vars`:
   - `has_obs_host` (`crates/goatbuilder/src/standup.rs:322`);
   - `has_breakglass_node` and `has_network_jump_node` with their absence reasons, ADR 1183's keys (`standup.rs:691` and `standup.rs:692`).
3. **The image builder's hardware classes.** `ops/bootstrap/hardware-classes.yaml` already keeps two axes apart: platforms (what an image is built for, from line 21, including `rpi5` at line 89) and roles (what a host becomes at first boot, from line 111: `core`, `obs`, `core-backup`, `obs-backup`, `breakglass` and `jumphost`). It already records that boot unlock is a platform property, because it depends on TPM presence (`hardware-classes.yaml:46`). goatbuilder does not read this file.

The three disagree in names (`network-jump` and `jumphost`), in coverage (the image roles have no `phone`, and `join-node` has no backup roles), and in what they carry: a play list in Rust, a flag in YAML, a purpose sentence in YAML.

### 2.2 Below the reachable host

No goatbuilder verb installs or checks an operating system. `ops/bootstrap/` holds the golden-path pieces, and goatbuilder calls none of them:

- an Ubuntu 24.04 autoinstall answer file with full-disk LUKS on LVM and key-only SSH (`ops/bootstrap/autoinstall/user-data.yaml:2`, `:28`);
- per-platform image builders (`build-image-orin.sh`, `build-image-karbon.sh`, `build-image-ec2.sh`; the `rpi5` platform's builder is `none-yet`, `hardware-classes.yaml:93`);
- the first-boot unit and its container integration test (`first-boot/`).

Today a person installs Ubuntu and sets up LUKS by hand, or runs these scripts, before goatbuilder's first phase. For the Raspberry Pi, three image builders that differ from one another exist outside goatcore.

### 2.3 How each kind is built and proven today

| Kind | Built by goatbuilder today | Proven today |
|---|---|---|
| core | Built: `standup`; `join-node --class core` | assay phases 13 and 15. No hermetic e2e: "founder standup … **not built**" (`deployment-e2e.md:182`). Standup phase 11 is a manual step (`standup.rs:611`, `standup.rs:617`). Not run on hardware |
| obs | Built: `standup`'s obs leg; `join-node --class obs` | one acceptance row (`prom-up`) |
| breakglass | Built: `standup` phase 14; `join-node --class breakglass` | none |
| network-jump | Built: `join-node --class network-jump` (needs `herd_connector_image`) | none |
| federation boundary node | Not built as a kind. `federate` configures hosts that already exist | `ops/federation-e2e`; gate row B1 measured |
| simple boundary node | Not built. `ops/simple-bn/` is placed by hand ([simple-boundary-node.md](simple-boundary-node.md), WIP; ADR 1253, Proposed) | unit tests |
| processing node (`goat-processors`) | Not built | none |
| octo-pod (the edge gateway appliance) | Not built. A pull agent is refused (GB-307); the pull converger comes after 1.0 (gate group D) | none in goatcore |
| sensor node (a small computer whose only job is sensors) | Not built | none |
| first-bite (the tap decoder) | Not built | none in goatcore |
| phone | Built: `join-node --class phone` ([phone-node.md](phone-node.md)) | gate group H |
| sensor kit's kids (per-sensor relay processes) | Not built in goatcore. Built today from two repositories outside goatcore | none in goatcore |

## 3. The design

### 3.1 A target type is data

One file per role: `ops/goatbuilder/targets/<type>.yaml`. `standup`, `join-node` and the image tooling all read it.

```yaml
# ops/goatbuilder/targets/breakglass.yaml (sketch; the schema is part of build step 1)
type: breakglass
families: [host-class, cloud]          # the platform families this role accepts; never a model
encryption: root                       # the minimum scope this role requires (section 3.2)
os:
  - ubuntu: "24.04"
plays:                                 # in order; the same plays join-node and standup run today
  - play: mgmt-host-baseline.yml
  - play: breakglass-nodes.yml
    group: "{site}_breakglass_hosts"
services: [netbird-client, oob-overlay]
sensors: allowed                       # the machine's declaration may list wired sensors (section 3.3)
acceptance:                            # rows in the acceptance catalog, by token
  requires: [breakglass]
proof:
  ci: vm                               # a VM from a fresh Ubuntu cloud image (section 3.5)
  hardware: required                   # a recorded run on real equipment
```

- `join-node --class <type>` accepts any type that has a file. `standup` builds a site's declared machines from the files of their types.
- The site still declares which machines it has, with ADR 1183's keys. A site's declaration says *whether*; the type file says *what*.
- A new role is a new file, not new code. A role whose plays need logic of their own still gets a file, and the file names the verb or built-in step that converges it.
- **A type names families, never a model.** Its plays work on every model of each family it accepts. In particular, no type assumes a Raspberry Pi.

The types, by group:

| Group | Types |
|---|---|
| host site | core, obs, break-glass, network jump box |
| edge | octo-pod (the edge gateway appliance), sensor node (a small computer whose only job is sensors), first-bite (the tap decoder) |
| boundary | federation boundary, simple boundary |
| processing | goat-processors |
| (existing) | phone ([phone-node.md](phone-node.md)) |

### 3.2 A platform family is data, and the golden path lives there

One data file per family. A model joins a family by an entry in that file: its firmware, boot media, OS image, its disk-encryption shape and how it unlocks, and its golden-path builder if it has one. `ops/bootstrap/hardware-classes.yaml`'s platforms become the models of these families. Its roles axis is superseded by the type files.

| Family | Models | Status | Roles it suits |
|---|---|---|---|
| host-class | Karbon 801, Karbon 802, Jetson Orin, Jetson Thor | supported | core and obs, and the other host-site roles |
| cloud | AWS EC2 | supported | as declared per type |
| cloud | Azure VMs, Google Compute Engine | listed, not built | as declared per type |
| small computer | Raspberry Pi 5 today; a smaller OnLogic model later | proposed | sensor node, octo-pod |
| ruggedized mobile edge | TSI V3, TSI V3 mini | listed, not planned: they keep their own baseline, and goatbuilder only checks their end state | as declared per type |

- **Adding hardware adds a model to a family**, or a family. The smaller OnLogic model joins the small-computer family when the hardware is in hand.
- **Raspberry Pi preparation is golden-path data** of the Pi model in the small-computer family: exposing and writing its drive, and its unlock. It belongs to no type.
- **Encryption scope.** This is decided as follows:
  - **The family declares the scope each model can deliver**, and how it unlocks. For example, a Karbon delivers `root` with TPM2 unlock in the initramfs (`hardware-classes.yaml:51`). A Pi, with no TPM, delivers either `data` (the data volume, with a hardware unlock token) or `root` with a network unlock.
  - **The type declares the minimum scope its role requires.** For example, core requires `root`, and a sensor node accepts `data`.
  - **A type accepts only families whose models deliver its scope.** `doctor --target` checks the scope a machine actually delivered.

  Why it is split: how a disk unlocks is a property of the hardware (TPM or not), which the file already records at `hardware-classes.yaml:46`. How much must be encrypted is a security property of the role, and it is checked on the end state whatever installed it. Putting both on the type would make every type know every model. Putting both on the family would let weaker hardware lower a role's requirement.
- **One Pi image builder first.** Three image builders for the Pi exist today and differ. One is picked as the Pi's golden path before goatbuilder wraps any of them; that is an open question (section 6). The `rpi5` entry today proposes installer full-disk encryption with a network unlock (`hardware-classes.yaml:92`, `:96`), while the existing Pi buildout encrypts the data volume. The choice of builder settles which one the Pi delivers.
- **The ruggedized mobile edge family is in the structure now** so that a machine of it can be declared and checked, but goatbuilder does not prepare it.

### 3.3 Sensors are declared per machine; the kit's kids follow them

The sensor kit is one package, installed the same way on every edge machine. A **kid** is one small relay process for one sensor: it converts that sensor's native output to ASTERIX, CoT, ISA or SAPIENT, and forwards it to the fabric. A kid is not a machine and not the whole kit.

- **A machine's declaration lists its wired sensors.** That is a fact about the machine, next to its type and its family.
- **A type file says whether it allows sensors.** Today that is the sensor node, octo-pod, break-glass, network jump box, and both boundary types.
- **goatbuilder installs the kit's package and turns on the kid for each declared sensor**, on any machine whose type allows sensors. A machine with no sensors runs no kids. Turning a sensor off is removing it from the declaration.
- **The sensor node** is the target type for a small computer whose only job is sensors: the type's own plays and acceptance are those of an empty edge machine (mesh peer, name resolution, egress), and its purpose comes from its sensors.
- **What the kit needs from its host**, from the survey of its existing buildout:
  - the type's Ubuntu Server 24.04 baseline;
  - mesh enrolment under the KID gear class;
  - egress to fetch its release.
- **What it installs:** the sensor platform's pinned, signed release, and its local services:
  - a local island router that never federates;
  - a client-mode fabric session;
  - the shared translator;
  - a one-way bridge;
  - a health publisher.
- **Its acceptance** runs after the host passes:
  - the sensor platform's own verification;
  - a round trip of its health sample through the site fabric;
  - the node's mesh peer in the right group;
  - the mesh management name resolving on the node;
  - for each declared sensor, its kid running.
- **Its updates:** a pin bump of the sensor platform's release, followed by its acceptance. A release with a breaking contract change is refused until its upgrade notes are done.
- **What stays outside the command line**, as written directions with the end state checked:
  - The per-sensor packages, including their sensor-interface and identity settings. They are authored and deployed by each sensor's owner with the sensor platform's own tool.
  - The physical work on a Pi (its drive and its unlock token). That is family preparation (section 3.2).

### 3.4 Check the end state; guide the method; never put it on rails

The contract starts at **a reachable Ubuntu host with an SSH key**, however it was installed. Above that line, goatbuilder builds. Below it, goatbuilder guides, and it checks the result.

- **`goatbuilder doctor --target <type>`** checks a host's end state against its type and family, whatever path built it, and changes nothing. For example:
  - the encryption scope the type requires is in place;
  - the OS version is one the type accepts;
  - chrony starts before docker;
  - the docker data root is on the tier the type declares;
  - the inotify floor is met;
  - for a small computer, first boot completed, the root filesystem grown and key-only SSH;
  - for a break-glass node, ADR 1114 §5's dual-homing contract;
  - for each declared sensor, its kid enabled.

  Every refusal names what to fix on the machine, not which installer to use. For a family goatbuilder does not prepare, such as the ruggedized mobile edge, this check is the whole of goatbuilder's part below the line.
- **The golden path, optional:** `goatbuilder image <model>` wraps that model's preparation in `ops/bootstrap`. It produces an install ready for a type's plays: unattended Ubuntu, the model's encryption shape, first boot and enrolment.
- **The supported general path:** written directions for each type that state the intent and the end state, for a capable person to work through on hardware the golden path does not know. `doctor --target` is how that person knows they arrived.
- **What goatbuilder does not do:** it is not an operating-system installer or configuration manager beyond the golden path. It does not partition disks, choose kernels or manage packages outside a type's plays.

### 3.5 Proof tiers

| Tier | What runs | Where |
|---|---|---|
| CI | A VM from a fresh Ubuntu cloud image, then `join-node` or `standup` for the type, then the type's acceptance and `doctor --target`, then the kids for a declared test sensor where the type allows sensors | CI, extending ADR 1200's deployment-e2e harness from container hosts to VMs |
| Hardware | The same steps on real equipment, from the model's golden path, recorded as a run record | an existing site, for the types that site has |

Every type passes the CI tier. The hardware tier is required where equipment exists: core, obs, break-glass, network jump box and federation boundary (gate group J).

### 3.6 Status

| Item | Kind | Status | Today's converge path |
|---|---|---|---|
| core | type | Designed | Built (`standup`, `join-node`) |
| obs | type | Designed | Built (`standup`, `join-node`) |
| break-glass | type | Designed | Built (`standup` phase 14, `join-node`) |
| network jump box | type | Designed | Built (`join-node`) |
| phone | type | Designed | Built (`join-node --class phone`) |
| federation boundary | type | Designed | Not built as a machine kind (`federate` configures an existing host) |
| simple boundary | type | Designed | Not built |
| goat-processors | type | Designed | Not built |
| octo-pod | type | Designed (push-deployed) | Not built in goatcore |
| sensor node | type | Designed | Not built |
| first-bite | type | Designed | Not built in goatcore |
| wired sensors and the kit's kids | declaration | Designed | Not built in goatcore |
| host-class | family | Designed as data; its models' builders are built | `build-image-*.sh` (not called by goatbuilder) |
| cloud | family | Designed as data; AWS EC2's builder is built; Azure VMs and Google Compute Engine listed, not built | `build-image-ec2.sh` |
| small computer | family | Proposed; the Pi's builder is not picked (section 6); the OnLogic model waits for the hardware | Not built in goatcore |
| ruggedized mobile edge | family | Listed, not planned: end-state check only | Not built |

## 4. Build order

One revertible pull request per step, on the program's normal path. Each step builds on the one before it. No dates.

1. **The type definition.**
   - The type and family schemas and `ops/goatbuilder/targets/`, with core, obs, break-glass and network jump box moved onto them. `join-node` and `standup` read the files, and the compiled list at `joinnode.rs:90` goes.
   - Acceptance rows for breakglass and network-jump, which have none today.
   - `doctor --target` for these four types.
   - The CI tier's VM harness, for the first type that uses it.
   - **A pure refactor.** `join-node` and `standup` print the same plans before and after: the golden plans (`crates/goatbuilder/tests/full_plans_unchanged.rs`) stay byte-identical, and a per-type `join-node --dry-run` parity test is written first.
   - **Phase entries are of three kinds:** a play, run as today; a built-in step whose logic stays in Rust behind a name (a core joiner's seeding, replication wiring and certificates); or a declared skip with its reason.
   - **A play may live outside goatcore.** The type file names its source repository as data, and the loader does not require the play, so a public clone loads every type. At run time a missing play is refused with its own error code, naming what to assemble. The network-jump play is the first such case.
   - **Models stay in `ops/bootstrap/hardware-classes.yaml`** for step 1, grouped into families, and gain what each delivers. Its roles axis is marked superseded by the type files; it is deleted in a later step, once first-boot role selection stops reading it.

   Follow-ups to step 1, each its own pull request (Designed):
   - The `breakglass-node` role gains ADR 1114 §5's out-of-band route metric. The role does not set it today, so `doctor --target`'s route-metric check for a break-glass node fails until it does.
   - A type's required inputs refused before the node is changed: for network-jump, `herd_connector_image`, which today is only named in a label. It changes `join-node`'s plan, so it carries its own golden update.
   - The roles axis of `hardware-classes.yaml` deleted, once first-boot role selection reads the type files.
2. **Standup phase 11 automated.** This removes the two manual steps at `standup.rs:611` and `standup.rs:617`.
   - **Today**, the operator does four things by hand. First, the first dashboard login with the bootstrap password (`goatnet-standup.md:529`). Second, a re-run of `dashboard-deploy.yml`, whose post-task promotes the first user (`dashboard-deploy.yml:212`). Third, the admin service user `svc-<site>-netbird-infra` and its token, made in the UI. Fourth, storing the token as `<SITE>_NB_API_TOKEN` and resuming at phase 12, which mints every other identity from it (`standup.rs:628`). It is manual because the first NetBird admin needs an interactive sign-in (`goatbuilder-speed.md:152`).
   - **The route:** extend `ops/mgmt-host-bootstrap`. It already writes NetBird's database directly (its `create-account` and `create-setup-key` verbs, raw SQL under ADR 0413), and its tests are held to the NetBird management version it is pinned to. It gains a verb that creates the admin service user and a hashed token. The plaintext token is written once, to a mode-0600 file under `/run`, and then into the declared secrets store. Phase 11 becomes an applied step plus the existing `--live` gate, and the first human login moves after the standup.
   - **No fork.** The plan in [goatlite.md](goatlite.md)'s table row 6 (line 94), for the management service to seed the token, would need a modified NetBird management service. That conflicts with [ADR 1248](../adr/1248-goat-follows-stock-upstream-netbird-and-wireguard-and-retires-its-departures.md) (stock upstream NetBird), so this route replaces it.
   - **Its checks:**
     - the account and its owner exist;
     - it is idempotent, and never replaces a token already stored;
     - the token authenticates;
     - the store is writable;
     - the management version matches the pinned image.
   - **Its refusals:** a version mismatch; a service user with no stored token; management unreachable; the store not writable; the `--live` authentication failing.
   - **Its CI-tier proof:** a founder standup through phase 13 with no MANUAL row. Then the service user and token are checked through the API, the token appears in no log, a re-run mints nothing, and each refusal case is exercised.
   - **Status:** Designed. It mints an admin credential for the mesh control plane, so its build is reviewed as a trust change before it starts.
3. **The federation boundary as a type.** The host `goatbuilder federate` configures becomes a machine `join-node` builds from a reachable Ubuntu host.
4. **The simple boundary and goat-processors as new types.** The processing node's acceptance runs the engines of the public `goat-processors` repository on recorded data, and checks the output against its conformance vectors.
5. **The small-computer family**, with Raspberry Pi preparation first, as golden-path data, once one Pi image builder is picked (section 6). The OnLogic model joins when the hardware is in hand.
6. **The edge types: octo-pod, sensor node and first-bite.** octo-pod is push-deployed: `join-node` reaches it and pushes, like any other type. The pull converger comes after 1.0, as the operator decided on 15 November (gate group D), and arrives as a value of `converger:` without changing any type file.
7. **Sensors declared per machine**, with the kit's package installed and the matching kids turned on, on any type that allows sensors.

## 5. What does not change

- The node classes, their ACL positions and their enrolment rules stay as ADRs 1114 and 1131 set them. A break-glass node's enrolment stays manual, by ADR 1114's reasoning.
- A site still declares which rescue machines it has, or why it has none (ADR 1183).
- Roles, not hardware (ADR 1141): a target type names a role. A family names hardware, and a model joins it.

## 6. Open questions for the operator

1. **Multi-sensor machines.** A machine with several wired sensors runs one kid per sensor. Is a gateway that fronts several sensors anything more than a machine with a longer sensor list?
2. **Fabric client certificates.** Does the site's own certificate authority issue the fabric client certificates of the machines that run kids?
3. **The Pi's OS baseline.** Ubuntu Server 24.04, as the existing buildout uses, or 26.04?
4. **The Pi's golden path.** Which of the three existing Pi image builders becomes the Pi's golden path? That choice also settles whether the Pi delivers root or data-volume encryption (section 3.2).
