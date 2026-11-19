# goatbuilder build targets: platforms, target types and payloads

| Field | Value |
|---|---|
| Status | Designed, not built ([ADR 1257](../adr/1257-every-machine-kind-is-a-goatbuilder-build-target-type-declared-as-data.md), Proposed) |
| Scope | What goatbuilder builds a machine into, for every kind of machine a goatnet runs; the hardware it prepares; the software it deploys onto a machine; how each is checked and proven |
| Related | [goatbuilder-v1-gate.md](goatbuilder-v1-gate.md) group J; [deployment-e2e.md](deployment-e2e.md) ([ADR 1200](../adr/1200-deployment-e2e-three-tiers.md)); [ADR 1141](../adr/1141-four-node-host-site-baseline.md), [ADR 1183](../adr/1183-rescue-nodes-are-part-of-the-site-baseline.md), [ADR 1114](../adr/1114-breakglass-node-class-and-two-path-reach.md), [ADR 1131](../adr/1131-network-jump-class-and-herd-gear-scope.md); [`ops/bootstrap/README.md`](../../ops/bootstrap/README.md) |

## 1. What this is for

A goatnet runs several kinds of machine: the core and observability nodes of a host site, a break-glass node, a network-jump host, a federation boundary node, a simple boundary node, a processing node, a pod, and small edge computers that front sensors. goatbuilder builds some of them, each by its own code path, and none from the bare machine.

This design separates three things that today are mixed together:

| Concept | What it is | Examples |
|---|---|---|
| **Platform** | The machine and its preparation: firmware, boot media, the OS image, disk encryption and how it unlocks. The golden path lives here. | x86 mini PC (Karbon), Jetson Orin and Thor, Raspberry Pi 5 (today; it needs its own preparation), a small OnLogic computer (future; it replaces the Pi), a cloud VM |
| **Target type** | The role the machine plays in a goatnet: its plays, services and acceptance checks | core, obs, breakglass, network-jump, federation boundary node, simple boundary node, processing node, pod, small computer (an edge device), phone |
| **Payload** | Software deployed onto a target type, with its own acceptance, onto any type that accepts it | the sensor kit (the KID gear class), deployable onto a small computer, a pod, a federation boundary node, a break-glass node or a network-jump host |

goatbuilder builds a target type on a platform, and deploys payloads onto it. Each of the three is declared as data. A check confirms the end state of each, whatever path got the machine there.

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
| pod | Not built. A pull agent is refused (GB-307); the pull converger comes after 1.0 (gate group D) | none |
| small computer (edge device) | Not built | none |
| phone | Built: `join-node --class phone` ([phone-node.md](phone-node.md)) | gate group H |
| sensor kit (payload) | Not built in goatcore. Built today from two repositories outside goatcore | none in goatcore |

## 3. The design

### 3.1 A target type is data

One file per role: `ops/goatbuilder/targets/<type>.yaml`. `standup`, `join-node` and the image tooling all read it.

```yaml
# ops/goatbuilder/targets/breakglass.yaml (sketch; the schema is part of build step 1)
type: breakglass
platforms: [karbon-801, karbon-802]   # the platforms this role accepts
encryption: root                      # the minimum scope this role requires (section 3.2)
os:
  - ubuntu: "24.04"
plays:                                # in order; the same plays join-node and standup run today
  - play: mgmt-host-baseline.yml
  - play: breakglass-nodes.yml
    group: "{site}_breakglass_hosts"
services: [netbird-client, oob-overlay]
acceptance:                           # rows in the acceptance catalog, by token
  requires: [breakglass]
proof:
  ci: vm                              # a VM from a fresh Ubuntu cloud image (section 3.6)
  hardware: required                  # a recorded run on real equipment
```

- `join-node --class <type>` accepts any type that has a file. `standup` builds a site's declared machines from the files of their types.
- The site still declares which machines it has, with ADR 1183's keys. A site's declaration says *whether*; the type file says *what*.
- A new role is a new file, not new code. A role whose plays need logic of their own still gets a file, and the file names the verb that converges it.
- **Nothing in a type may assume a platform.** A type names the platforms it accepts, and its plays work on each of them. In particular, no type assumes a Raspberry Pi.

### 3.2 A platform is data, and the golden path lives there

`ops/bootstrap/hardware-classes.yaml` keeps its platforms axis and gains each platform's preparation: firmware, boot media, the OS image, the disk-encryption shape and how it unlocks. Its roles axis is replaced by the type files. Its aliases stay resolvable.

- **Adding hardware adds a platform, not a type.** The small OnLogic computer is a platform. It stays Designed until the hardware is in hand.
- **Raspberry Pi preparation is platform data.** The Pi needs its own preparation (exposing and writing its drive, and its unlock). That preparation belongs to the `rpi5` platform, not to any type.
- **Encryption scope.** This is decided as follows:
  - **The platform declares the scope it can deliver** and how it unlocks. For example, a Karbon delivers `root` with TPM2 unlock in the initramfs (`hardware-classes.yaml:51`). A Pi, with no TPM, delivers either `data` (the data volume, with a hardware unlock token) or `root` with a network unlock.
  - **The type declares the minimum scope its role requires.** For example, core requires `root`, and the small computer accepts `data`.
  - **A type accepts only platforms that deliver its scope.** `doctor --target` checks the scope the platform actually delivered.

  Why it is split: how a disk unlocks is a property of the hardware (TPM or not), which the file already records at `hardware-classes.yaml:46`. How much must be encrypted is a security property of the role, and it is checked on the end state whatever installed it. Putting both on the type would make every type know every platform. Putting both on the platform would let a weaker platform lower a role's requirement.
- **One Pi image builder first.** Three image builders for the Pi exist today and differ. One is picked as the `rpi5` platform's golden path before goatbuilder wraps any of them. That choice is an open question (section 6). The `rpi5` entry today proposes installer full-disk encryption with a network unlock (`hardware-classes.yaml:92`, `:96`), while the existing Pi buildout encrypts the data volume. The choice of builder settles which one the platform delivers.

### 3.3 A payload is data, with its host list as data

A payload is software deployed onto a target type, declared once with its own acceptance, in `ops/goatbuilder/payloads/<payload>.yaml`. It names the target types that may host it. That list is data in the declaration, never a list in code.

**The sensor kit** is the first payload. Its host types are small computer, pod, federation boundary node, breakglass and network-jump. What it needs, from the survey of the existing buildout:

- **Its host:**
  - the type's Ubuntu Server 24.04 baseline, on a small arm64 or amd64 machine;
  - mesh enrolment under the KID gear class;
  - egress to fetch its release.
- **What it installs:** the sensor platform's pinned, signed release, and that platform's local services:
  - a local island router that never federates;
  - a client-mode fabric session;
  - the shared translator;
  - a one-way bridge;
  - a health publisher.
- **Its acceptance:**
  - the sensor platform's own verification;
  - a round trip of its health sample through the site fabric;
  - the node's mesh peer in the right group;
  - the mesh management name resolving on the node.
- **Its updates:** a pin bump of the sensor platform's release, followed by the payload's acceptance. A release with a breaking contract change is refused until its upgrade notes are done.
- **What stays outside the command line**, as written directions with the end state checked:
  - Authoring and deploying the per-sensor packages, including their sensor-interface and identity settings. Those are owned by each sensor's owner and use the sensor platform's own tool.
  - The physical work on a Pi: exposing and writing its drive, and enrolling its hardware unlock token. That is `rpi5` platform preparation (section 3.2), not part of the payload.

### 3.4 The small computer is a target type

The **small computer** is an edge device: what a Pi or a small OnLogic box is when it is not a pod. Its type file declares the Ubuntu Server 24.04 baseline, the platforms it accepts (`rpi5` today; the OnLogic platform when it exists), the `data` encryption scope, mesh enrolment, and the acceptance of an empty edge device: mesh peer, name resolution and egress. A payload, such as the sensor kit, gives it its purpose.

### 3.5 Check the end state; guide the method; never put it on rails

The contract starts at **a reachable Ubuntu host with an SSH key**, however it was installed. Above that line, goatbuilder builds. Below it, goatbuilder guides, and it checks the result.

- **`goatbuilder doctor --target <type>`** checks a host's end state against its type and platform, whatever path built it, and changes nothing. For example:
  - the encryption scope the type requires is in place;
  - the OS version is one the type accepts;
  - chrony starts before docker;
  - the docker data root is on the tier the type declares;
  - the inotify floor is met;
  - for a small computer, first boot completed, the root filesystem grown and key-only SSH;
  - for a break-glass node, ADR 1114 §5's dual-homing contract.

  Every refusal names what to fix on the machine, not which installer to use. A payload's acceptance runs after the host passes.
- **The golden path, optional:** `goatbuilder image <platform>` wraps that platform's preparation in `ops/bootstrap`. It produces an install ready for the type's plays: unattended Ubuntu, the platform's encryption shape, first boot and enrolment.
- **The supported general path:** written directions for each type that state the intent and the end state, for a capable person to work through on hardware the golden path does not know. `doctor --target` is how that person knows they arrived.
- **What goatbuilder does not do:** it is not an operating-system installer or configuration manager beyond the golden path. It does not partition disks, choose kernels or manage packages outside the type's plays.

### 3.6 Proof tiers

| Tier | What runs | Where |
|---|---|---|
| CI | A VM from a fresh Ubuntu cloud image, then `join-node` or `standup` for the type, then the type's acceptance and `doctor --target`, then each payload the type hosts and its acceptance | CI, extending ADR 1200's deployment-e2e harness from container hosts to VMs |
| Hardware | The same steps on real equipment, from the platform's golden path, recorded as a run record | an existing site, for the types that site has |

Every type passes the CI tier. The hardware tier is required where equipment exists: core, obs, breakglass, network-jump and the federation boundary node (gate group J).

### 3.7 Status

| Item | Kind | Status | Today's converge path |
|---|---|---|---|
| core | type | Designed | Built (`standup`, `join-node`) |
| obs | type | Designed | Built (`standup`, `join-node`) |
| breakglass | type | Designed | Built (`standup` phase 14, `join-node`) |
| network-jump | type | Designed | Built (`join-node`) |
| phone | type | Designed | Built (`join-node --class phone`) |
| federation boundary node | type | Designed | Not built as a machine kind (`federate` configures an existing host) |
| simple boundary node | type | Designed | Not built |
| processing node | type | Designed | Not built |
| pod | type | Designed (push-deployed) | Not built |
| small computer | type | Designed | Not built |
| sensor kit | payload | Designed | Not built in goatcore |
| Karbon, Orin and Thor, cloud VM | platforms | Designed as golden-path data; builders built | `build-image-*.sh` (not called by goatbuilder) |
| Raspberry Pi 5 | platform | Designed; builder not picked (section 6) | Not built in goatcore |
| small OnLogic computer | platform | Designed until the hardware is in hand | Not built |

## 4. Build order

One revertible pull request per step, on the program's normal path. Each step builds on the one before it. No dates.

1. **The type definition.**
   - The type and platform schemas and `ops/goatbuilder/targets/`, with core, obs, breakglass and network-jump moved onto them. `join-node` and `standup` read the files, and the compiled list at `joinnode.rs:90` goes.
   - Acceptance rows for breakglass and network-jump, which have none today.
   - `doctor --target` for these four types.
   - The CI tier's VM harness, for the first type that uses it.
   - **A pure refactor.** `join-node` and `standup` print the same plans before and after: the golden plans (`crates/goatbuilder/tests/full_plans_unchanged.rs`) stay byte-identical, and a per-type `join-node --dry-run` parity test is written first.
   - **Phase entries are of three kinds:** a play, run as today; a built-in step whose logic stays in Rust behind a name (a core joiner's seeding, replication wiring and certificates); or a declared skip with its reason.
   - **A play may live outside goatcore.** The type file names its source repository as data, and the loader does not require the play, so a public clone loads every type. At run time a missing play is refused with its own error code, naming what to assemble. The network-jump play is the first such case.
   - **Platforms stay in `ops/bootstrap/hardware-classes.yaml`**, which gains what each platform delivers. Its roles axis is marked superseded by the type files; it is deleted in a later step, once first-boot role selection stops reading it.

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
3. **The federation boundary node as a type.** The host `goatbuilder federate` configures becomes a machine `join-node` builds from a reachable Ubuntu host.
4. **The simple boundary node and the processing node as new types.** The processing node's acceptance runs the engines of the public `goat-processors` repository on recorded data, and checks the output against its conformance vectors.
5. **The edge device and the sensor kit**, in three parts:
   1. Raspberry Pi platform preparation, as golden-path data, once one Pi image builder is picked (section 6);
   2. the small-computer type;
   3. the sensor-kit payload: first on a small computer, a pod and a federation boundary node, then on a break-glass node and a network-jump host, per its host list.

   The OnLogic platform stays Designed until the hardware is in hand.
6. **Pods, push-deployed.** A pod is built and updated like any other type: `join-node` reaches it and pushes. The pull converger comes after 1.0, as the operator decided on 15 November (gate group D). It arrives as a value of `converger:` and does not change the pod's type file.

## 5. What does not change

- The node classes, their ACL positions and their enrolment rules stay as ADRs 1114 and 1131 set them. A break-glass node's enrolment stays manual, by ADR 1114's reasoning.
- A site still declares which rescue machines it has, or why it has none (ADR 1183).
- Roles, not hardware (ADR 1141): a target type names a role. A platform names hardware, and covers only the hardware it knows.

## 6. Open questions for the operator

1. **Multi-sensor gateways.** Is a gateway that fronts several sensors a variant of the sensor-kit payload, or a separate payload?
2. **Fabric client certificates.** Does the site's own certificate authority issue the fabric client certificates of the payload's nodes?
3. **The Pi's OS baseline.** Ubuntu Server 24.04, as the existing buildout uses, or 26.04?
4. **The Pi's golden path.** Which of the three existing Pi image builders becomes the `rpi5` platform's golden path? That choice also settles whether the platform delivers root or data-volume encryption (section 3.2).
