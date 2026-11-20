# goatbuilder build targets: platform families and target types

| Field | Value |
|---|---|
| Status | Designed, not built ([ADR 1257](../adr/1257-every-machine-kind-is-a-goatbuilder-build-target-type-declared-as-data.md), Proposed) |
| Scope | What goatbuilder builds a machine into, for every kind of machine a goatnet runs; the hardware families it prepares; how each machine is checked and proven; the order of the work |
| Related | [goatbuilder-v1-gate.md](goatbuilder-v1-gate.md) group J; [deployment-e2e.md](deployment-e2e.md) ([ADR 1200](../adr/1200-deployment-e2e-three-tiers.md)); [ADR 1141](../adr/1141-four-node-host-site-baseline.md), [ADR 1183](../adr/1183-rescue-nodes-are-part-of-the-site-baseline.md), [ADR 1114](../adr/1114-breakglass-node-class-and-two-path-reach.md), [ADR 1131](../adr/1131-network-jump-class-and-herd-gear-scope.md), [ADR 1255](../adr/1255-vendor-systems-connect-through-one-vendor-bridges-repository-and-open-standards-go-through-goat-transformers.md); [`ops/bootstrap/README.md`](../../ops/bootstrap/README.md) |

## 1. What this is for

One tool builds every machine in a goatnet. A machine is one **platform family** (hardware, and how it is prepared) carrying one **target type** (the role it plays). Each family and each type is a data file goatbuilder reads, so a new one is a new file, not new code. goatbuilder checks every machine's end state, however it was first set up.

| Concept | What it is | Examples |
|---|---|---|
| **Platform family** | Hardware that can play the same roles, and its preparation: firmware, boot media, the OS image, disk encryption and how it unlocks. The golden path lives here. A model joins a family; no target type names a model | host-class, cloud, small computer, ruggedized mobile edge |
| **Target type** | The role the machine plays in a goatnet: its plays, services and acceptance checks | core, obs, break-glass, network-gear jump box, octo-pod, sensor node, vendor gateway, first-bite, federation boundary, simple boundary, goat-processors, phone |

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
| break-glass | Built: `standup` phase 14; `join-node --class breakglass` | none |
| network-gear jump box | Built: `join-node --class network-jump` (needs `herd_connector_image`) | none |
| federation boundary | Not built as a kind. `federate` configures hosts that already exist | `ops/federation-e2e`; gate row B1 measured |
| simple boundary | Not built. `ops/simple-bn/` is placed by hand ([simple-boundary-node.md](simple-boundary-node.md), WIP; ADR 1253, Proposed) | unit tests |
| goat-processors | Not built | none in goatcore; the engines pass their conformance vectors in their own repository |
| octo-pod | Not built in goatcore; its own repository has its own first boot. A pull agent is refused (GB-307); the pull converger comes after 1.0 (gate group D) | none in goatcore |
| sensor node | Not built in goatcore; three lines of the sensor kit each install it their own way (section 3.4) | none in goatcore |
| vendor gateway | Not built; the vendor bridges are packages in their own repository (ADR 1255) | none in goatcore |
| first-bite | Not built in goatcore; its own repository | none in goatcore |
| phone | Built: `join-node --class phone` ([phone-node.md](phone-node.md)) | gate group H |

## 3. The design

### 3.1 Platform families

One data file per family, under `ops/goatbuilder/families/`. A model joins a family by an entry in it: its firmware, boot media, OS image, disk-encryption shape and unlock, and its golden-path builder. `ops/bootstrap/hardware-classes.yaml`'s platforms become the models of these families, and its roles axis is superseded by the type files.

| Family | Models | Builder (golden path) | Status |
|---|---|---|---|
| host-class | Karbon 801, Karbon 802, Jetson Orin, Jetson Thor | `build-image-karbon.sh`, `build-image-orin.sh` | supported; suited to core, obs and other heavy or resilient roles |
| cloud | AWS EC2 | `build-image-ec2.sh` | supported |
| cloud | Azure Virtual Machines, Google Compute Engine | none | listed, not built |
| small computer | Raspberry Pi 5 today; a smaller OnLogic model later | the team's Pi base-image builder | proposed; the OnLogic model joins when the hardware is in hand |
| ruggedized mobile edge | TSI V3, TSI V3 mini | none: their own baseline | listed, not planned; goatbuilder only checks their end state |

- **The builder bar.** The team's Raspberry Pi base-image builder is the model for every family's builder: numbered, re-runnable steps from blank media to a verified machine (prerequisites, flash and verify the card, clone to NVMe, encrypt the root filesystem in place with LUKS2 using FIPS-approved primitives, enrol a YubiKey or removable-key unlock, enable FIPS mode, provision the substrate, verify the image, verify the box), each step proving its result before the next. The host-class and cloud builders are measured against it, and gaps are filed against each.
- **One Pi preparation.** The Pi base-image builder is the small-computer family's golden path. The two other Pi preparations fold into it: an SD-card seed that installs itself to NVMe with a YubiKey FIDO2 unlock, and the Clevis-and-Tang network unlock proposed in `hardware-classes.yaml:92`.
- **Encryption scope** is split between family and type. The family declares the scope each model delivers and how it unlocks: a Karbon delivers `root` with TPM2 unlock in the initramfs (`hardware-classes.yaml:51`); a Pi, with no TPM, delivers `root` with a YubiKey or removable-key unlock, and a recovery passphrase. The type declares the minimum scope its role requires. A type accepts only families whose models deliver it, and `doctor --target` checks the scope a machine delivered. Unlock is a property of the hardware (`hardware-classes.yaml:46`); how much must be encrypted is a property of the role.

### 3.2 Target types

One file per role: `ops/goatbuilder/targets/<type>.yaml`. `standup`, `join-node` and the image tooling all read it.

```yaml
# ops/goatbuilder/targets/breakglass.yaml (sketch; the schema is part of the foundation)
type: breakglass
families: [small-computer]             # the families this role accepts; never a model
encryption: root                       # the minimum scope this role requires (section 3.1)
os:
  - ubuntu: "24.04"
plays:                                 # in order; the same plays join-node and standup run today
  - play: mgmt-host-baseline.yml
  - play: breakglass-nodes.yml
    group: "{site}_breakglass_hosts"
services: [netbird-client, oob-overlay]
acceptance:                            # rows in the acceptance catalog, by token
  requires: [breakglass]
proof:
  ci: vm                               # a VM from a fresh cloud image (section 3.6)
  hardware: required                   # a recorded run on real equipment
```

| Group | Type | What it does | Families |
|---|---|---|---|
| host site | core | management plane: mesh control, identity, fabric router; A and B pair | host-class, cloud |
| host site | obs | observability and storage; A and B pair | host-class, cloud |
| access | break-glass | entry into the site's goat machines when the mesh and control plane are both down | small computer |
| access | network-gear jump box | entry into the site's network gear only (switches, routers, out-of-band management); not a general jump host | small computer (a Thor serves today) |
| edge | octo-pod | edge gateway: takes in feeds, records and audits them, releases chosen feeds to the fabric | small computer or host-class |
| edge | sensor node | runs the sensor kit's relays for the sensors wired to it (section 3.4) | small computer |
| edge | vendor gateway | runs the vendor bridges, one package per vendor (ADR 1255) | small computer |
| edge | first-bite | decodes binary messages from a network tap and hands them to authorized software | small computer or host-class |
| boundary | federation boundary | the edge router that peers with another goatnet | small computer or host-class |
| boundary | simple boundary | sends selected data out to a partner receiver | small computer or host-class |
| processing | goat-processors | correlation, fusion, filtering and quality on translated data | host-class |
| (existing) | phone | a phone as a full core joiner ([phone-node.md](phone-node.md)) | phone |

- `join-node --class <type>` accepts any type that has a file. `standup` builds a site's declared machines from the files of their types.
- The site still declares which machines it has, with ADR 1183's keys. A site's declaration says *whether*; the type file says *what*.
- Where a type accepts two families, the machine's declaration picks one, by load, resilience and ruggedization.
- **A type names families, never a model.** No type assumes a Raspberry Pi.
- **Edge kinds keep their own installers.** A type whose software has its own repository and installer (octo-pod, the sensor kit, first-bite, the vendor bridges) has plays that call that installer at a pinned, signed release and check its result. goatbuilder does not reimplement it. The type file names the repository and release as data.

### 3.3 How a machine is built

| Step | What happens | Today |
|---|---|---|
| 1. Declare it | the machine in the site config: its family and model, its type, and for a sensor node its sensors | group_vars and inventory |
| 2. Install the OS and disk encryption | scripted on a known model (the family's golden path, `goatbuilder image <model>`), or by hand from written directions | by hand, or `ops/bootstrap` scripts goatbuilder does not call |
| 3. Host baseline | hardening, packages, WireGuard modules; then checked by `doctor --target` | `join-node` phases 4 to 6 |
| 4. Join the goatnet | enrolment with a single-use setup key; mesh address recorded | `join-node` phases 10 to 12 |
| 5. Install the type | the type's services, deployed over the mesh | `join-node` phase 13 |
| 6. Prove it | the type's acceptance checks, written to the deploy ledger | `join-node` phase 15 |

A founder site differs at step 4: there is no goatnet to join yet, so `standup`'s core install creates the mesh control plane before anything enrols. An edge kind whose own installer enrols the machine keeps that order, and goatbuilder checks the end state.

### 3.4 The sensor node and the sensor kit

A **kid** is an integration component and nothing else: one small process that decodes one sensor's native output and converts it to ASTERIX, CoT, ISA or SAPIENT. A sensor may have several (tracks, status). Fusion and correlation are not the kit's job. A **sensor node** is the small computer that runs them, with the kit's local services: its own local router that never federates, a client session to the goat fabric, a one-way bridge from one to the other, a translator and a health publisher.

Three lines of the kit exist today, and none installs the way the kit's original design says (measured 2026-11-19 from each repository's code):

| Line | Status | How relays reach a machine | Joins the goat fabric | Hardware |
|---|---|---|---|---|
| Original design | not changed since 2026-09-01; its Pi deployment has moved to the prototype | one package with every relay on every node; an activation list turns on the node's relays (its ADR 0104) | — | Pi |
| Prototype | a feasibility spike (charter Proposed); the working Pi deployment today | the operator composes the relays each node gets; images built on the Pi or a laptop; a LAN scan starts only relays whose sensor is found live, unless marked external | NetBird enrolment; Zenoh mTLS from the fabric operator's bundle | Pi 5, arm64 |
| Public platform (`open-billie`) | a sensor-neutral rewrite; release 0.3.0 in preparation, not yet tagged; nothing signed yet | a platform `.deb`, then one bundle per sensor and architecture: `billie kid install`, `configure` (transport, bind address, port, source range), `activate`, with `upgrade`, `rollback`, `deactivate`, `remove` | NetBird optional; Zenoh mTLS from operator-supplied certificates | Ubuntu 24.04, arm64 and amd64 |

**Recommendation (open question 1):** goatbuilder targets the public platform's model. It is the only line with a per-machine command set goatbuilder can call and check, and per-machine selection is what both newer lines already do. Before goatbuilder relies on it, the platform needs a tagged, signed release and signed relay bundles. Under it, a sensor node's declaration lists each sensor with its parameters (transport, port, bind address on the sensor interface, source range, identifiers), and the type's plays install the platform at its pinned release, issue its fabric certificates, then install, configure and activate each declared sensor's bundle. Acceptance: the platform's own verification, a health sample round trip through the site fabric, and each declared sensor's relays running. Authoring a relay stays with the sensor's owner and the platform's own tool.

Running relays on types other than the sensor node is not decided (open question 3): the kit's platform brings its own local router and container runtime, which would meet goat's own router on a host-site, access or boundary machine, and it supports Ubuntu 24.04 only, not the Jetson OS.

### 3.5 Check the end state; guide the method; never put it on rails

The contract starts at **a reachable Ubuntu host with an SSH key**, however it was installed. Above that line, goatbuilder builds. Below it, goatbuilder guides, and it checks the result.

- **`goatbuilder doctor --target <type>`** checks a host's end state against its type and family, whatever path built it, and changes nothing. For example:
  - the encryption scope the type requires is in place;
  - the OS version is one the type accepts;
  - chrony starts before docker;
  - the docker data root is on the tier the type declares;
  - the inotify floor is met;
  - for a small computer, first boot completed and key-only SSH;
  - for a break-glass node, ADR 1114 §5's dual-homing contract.

  Every refusal names what to fix on the machine, not which installer to use. For a family goatbuilder does not prepare, such as the ruggedized mobile edge, this check is the whole of goatbuilder's part below the line.
- **The golden path, optional:** `goatbuilder image <model>` wraps that model's builder. It produces an install ready for step 3: the OS, the model's encryption, first boot.
- **The supported general path:** written directions for each type that state the intent and the end state, for a capable person on hardware the golden path does not know. `doctor --target` is how that person knows they arrived.
- **What goatbuilder does not do:** it is not an operating-system installer or configuration manager beyond the golden path. It does not partition disks, choose kernels or manage packages outside a type's plays.

### 3.6 Proof tiers

| Tier | What runs | Where |
|---|---|---|
| CI | A VM from a fresh cloud image, through steps 3 to 6 and `doctor --target`. For the small-computer family, an arm64 VM proves the software; the Pi builder's physical steps (the card, the console during encryption, the YubiKey) are proven on the bench | CI, extending ADR 1200's deployment-e2e harness from container hosts to VMs |
| Hardware | The same steps on real equipment, from the model's golden path, recorded as a run record | an existing site or the bench, for the types there is equipment for |

Every type passes the CI tier. The hardware tier is required where equipment exists (gate group J).

### 3.7 Network gear

Each site's switches and routers connect its machines to each other and the site to its transport. goatbuilder does not configure them. It reaches them only through the network-gear jump box, and every machine it builds depends on them.

### 3.8 Status

| Item | Kind | Status | Today's converge path |
|---|---|---|---|
| core, obs | types | Designed | Built (`standup`, `join-node`) |
| break-glass | type | Designed | Built (`standup` phase 14, `join-node`) |
| network-gear jump box | type | Designed | Built (`join-node`) |
| phone | type | Designed | Built (`join-node --class phone`) |
| federation boundary | type | Designed | Not built as a machine kind (`federate` configures an existing host) |
| simple boundary | type | Designed | Not built |
| goat-processors | type | Designed | Not built |
| octo-pod, first-bite, vendor gateway | types | Designed | Not built in goatcore; their own repositories install them |
| sensor node | type | Designed; install model open (question 1) | Not built in goatcore |
| host-class | family | Designed as data; builders built | `build-image-*.sh` (not called by goatbuilder) |
| cloud | family | Designed as data; EC2 builder built; Azure and Google listed, not built | `build-image-ec2.sh` |
| small computer | family | Proposed; builder chosen (the Pi base-image builder), outside goatcore | not called by goatbuilder |
| ruggedized mobile edge | family | Listed, not planned: end-state check only | Not built |

## 4. The order of work

One revertible pull request per step, on the program's normal path. No dates. **One foundation first, then four lanes that run in parallel.** The only dependencies are real ones: nothing moves onto type files before the files and their reader exist, and nothing on a small computer is built or proven before the small-computer builder exists.

| Part | Work | Waits for |
|---|---|---|
| Foundation | the type and family file formats and their reader; `doctor --target`'s base check; the CI VM harness | nothing |
| Host-site lane | core and obs onto type files; standup phase 11 automated | foundation |
| Boundary lane | federation boundary and simple boundary as types | foundation; their small-computer variants also wait for the small-computer builder |
| Processing lane | goat-processors as a type | foundation |
| Small-computer lane | the small-computer builder brought in; then break-glass, network-gear jump box, sensor node, octo-pod, vendor gateway, first-bite | foundation |

### 4.1 Foundation

Built one type at a time: the formats and core first, proven unchanged against today's plans, then each other type.

**What the foundation holds.**
   - The type and family schemas and `ops/goatbuilder/targets/`, with core, obs, break-glass and network jump box moved onto them. `join-node` and `standup` read the files, and the compiled list at `joinnode.rs:90` goes.
   - Acceptance rows for breakglass and network-jump, which have none today.
   - `doctor --target` for these four types.
   - The CI tier's VM harness, for the first type that uses it.
   - **A pure refactor.** `join-node` and `standup` print the same plans before and after: the golden plans (`crates/goatbuilder/tests/full_plans_unchanged.rs`) stay byte-identical, and a per-type `join-node --dry-run` parity test is written first.
   - **Phase entries are of three kinds:** a play, run as today; a built-in step whose logic stays in Rust behind a name (a core joiner's seeding, replication wiring and certificates); or a declared skip with its reason.
   - **A play may live outside goatcore.** The type file names its source repository as data, and the loader does not require the play, so a public clone loads every type. At run time a missing play is refused with its own error code, naming what to assemble. The network-jump play is the first such case, and the edge kinds' installers follow the same rule.
   - **Models stay in `ops/bootstrap/hardware-classes.yaml`** for step 1, grouped into families, and gain what each delivers. Its roles axis is marked superseded by the type files; it is deleted in a later step, once first-boot role selection stops reading it.

   Follow-ups, each its own pull request (Designed):
   - The `breakglass-node` role gains ADR 1114 §5's out-of-band route metric. The role does not set it today, so `doctor --target`'s route-metric check for a break-glass node fails until it does.
   - A type's required inputs refused before the node is changed: for network-jump, `herd_connector_image`, which today is only named in a label. It changes `join-node`'s plan, so it carries its own golden update.
   - The roles axis of `hardware-classes.yaml` deleted, once first-boot role selection reads the type files.
**Standup phase 11, automated (host-site lane).** This removes the two manual steps at `standup.rs:611` and `standup.rs:617`.
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

### 4.2 Host-site lane

- **core and obs onto type files**, each proven unchanged against today's plans.
- **Standup phase 11 automated**, as designed in section 4.1: an `ops/mgmt-host-bootstrap` verb, no NetBird fork, reviewed as a trust change before its build starts.

### 4.3 Boundary lane

- **Federation boundary as a type.** The host `goatbuilder federate` configures becomes a machine `join-node` builds from a reachable Ubuntu host, on a small computer or a host-class machine.
- **Simple boundary as a type**, from `ops/simple-bn/` (ADR 1253).
- Their small-computer variants are proven once the small-computer builder is in.

### 4.4 Processing lane

- **goat-processors as a type**, host-class only. Its plays install the engines from the public `goat-processors` repository at a pinned release. Its acceptance runs them on recorded data and checks the output against the repository's conformance vectors.

### 4.5 Small-computer lane

1. **The small-computer builder brought in.** `goatbuilder image <model>` wraps the Pi base-image builder for the Pi model; its steps that need a person (the card, the console during encryption, the YubiKey) stay as directions with the end state checked. The other two Pi preparations fold into it. The OnLogic model joins when the hardware is in hand.
2. **Then, in any order:**
   - break-glass and network-gear jump box moved onto the small-computer family (ADR 1114 §5's checks with them);
   - sensor node, per section 3.4, once open question 1 is answered;
   - octo-pod, push-deployed: `join-node` reaches it and pushes, like any other type; the pull converger comes after 1.0 (gate group D) and arrives as a value of `converger:` without changing any type file;
   - vendor gateway, installing the vendor bridges at a pinned release (ADR 1255);
   - first-bite, installing it at a pinned release.

### 4.6 Fieldkit tools a type serves

A Fieldkit tool is a single-file page a host serves over its own API ([fieldkit-and-bleating-edge.md](fieldkit-and-bleating-edge.md)). A type file lists the Fieldkit tools its machine serves, and the type's acceptance checks each one is served and passes its scope guard. The first is Fieldkit Gatepost, served by the pod API (`ops/goat-pod-api`, ADR 1251) on a machine that runs it, such as an octo-pod. Operator-facing pages goatbuilder itself adds use the Bleating Edge design system.

## 5. What does not change

- The node classes, their ACL positions and their enrolment rules stay as ADRs 1114 and 1131 set them. A break-glass node's enrolment stays manual, by ADR 1114's reasoning.
- A site still declares which rescue machines it has, or why it has none (ADR 1183).
- Roles, not hardware (ADR 1141): a target type names a role. A family names hardware, and a model joins it.
- Each edge kind's repository keeps its own installer and owners; goatbuilder calls it at a pinned release.

## 6. Open questions for the operator

1. **How the sensor kit is installed.** Recommendation: the public platform's model (section 3.4), once it has a signed release and signed relay bundles.
2. **Fabric client certificates.** Does the site's own certificate authority issue the fabric client certificates of sensor nodes?
3. **Relays on other types.** Do any types besides the sensor node run sensor relays, given that the kit's platform brings its own local router and container runtime?
4. **The Pi's OS baseline.** Ubuntu Server 24.04 (the public sensor platform supports only 24.04) or 26.04?
