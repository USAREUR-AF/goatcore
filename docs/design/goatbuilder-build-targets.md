# goatbuilder build targets: every machine kind a target type

| Field | Value |
|---|---|
| Status | Designed, not built ([ADR 1257](../adr/1257-every-machine-kind-is-a-goatbuilder-build-target-type-declared-as-data.md), Proposed) |
| Scope | What goatbuilder builds a machine into, for every kind of machine a goatnet runs; how the result is checked; how each kind is proven |
| Related | [goatbuilder-v1-gate.md](goatbuilder-v1-gate.md) group J; [deployment-e2e.md](deployment-e2e.md) ([ADR 1200](../adr/1200-deployment-e2e-three-tiers.md)); [ADR 1141](../adr/1141-four-node-host-site-baseline.md), [ADR 1183](../adr/1183-rescue-nodes-are-part-of-the-site-baseline.md), [ADR 1114](../adr/1114-breakglass-node-class-and-two-path-reach.md), [ADR 1131](../adr/1131-network-jump-class-and-herd-gear-scope.md); [`ops/bootstrap/README.md`](../../ops/bootstrap/README.md) |

## 1. What this is for

A goatnet runs several kinds of machine: the core and observability nodes of a host site, a break-glass node, a network-jump host, a federation boundary node, a simple boundary node, a processing node, a pod, and small sensor nodes and gateways. goatbuilder builds some of them, each by its own code path, and none from the bare machine. This design makes each kind a **target type**: one declaration of what the machine must be at the end, read by every verb that builds or checks one. It also states how far goatbuilder goes below that, into the operating system, and how far it does not.

## 2. What exists today (measured on `main`, 2026-11-19)

### 2.1 Three taxonomies for one question

Three places answer "what kind of machine is this", and none reads the others:

1. **`join-node`'s class list.** It is compiled in. `crates/goatbuilder/src/joinnode.rs:90` declares `CLASSES = ["core", "obs", "breakglass", "network-jump", "phone"]`. Each class has its own match arm, at `joinnode.rs:583` (core), `joinnode.rs:586` (obs), `joinnode.rs:602` (breakglass) and `joinnode.rs:608` (network-jump). A class outside the list is refused with GB-247 (`joinnode.rs:614`). A new kind of machine is new Rust.
2. **`standup`'s group_vars flags.** The founder standup does not read that list. It reads one flag per optional machine from the site's `group_vars`:
   - `has_obs_host` (`crates/goatbuilder/src/standup.rs:322`);
   - `has_breakglass_node` and `has_network_jump_node` with their absence reasons, ADR 1183's keys (`standup.rs:691` and `standup.rs:692`).
3. **The image builder's hardware classes.** `ops/bootstrap/hardware-classes.yaml` keeps platforms (what an image is built for, from line 21) apart from roles (what a host becomes at first boot, from line 111): `core`, `obs`, `core-backup`, `obs-backup`, `breakglass` and `jumphost`. goatbuilder does not read this file.

The three disagree in names (`network-jump` and `jumphost`), in coverage (the image roles have no `phone`, and `join-node` has no backup roles), and in what they carry: a play list in Rust, a flag in YAML, a purpose sentence in YAML.

### 2.2 Below the reachable host

No goatbuilder verb installs or checks an operating system. `ops/bootstrap/` holds the golden-path pieces, and goatbuilder calls none of them:

- an Ubuntu 24.04 autoinstall answer file with full-disk LUKS on LVM and key-only SSH (`ops/bootstrap/autoinstall/user-data.yaml:2`, `:28`);
- per-platform image builders (`build-image-orin.sh`, `build-image-karbon.sh`, `build-image-ec2.sh`);
- the first-boot unit and its container integration test (`first-boot/`).

Today a person installs Ubuntu and sets up LUKS by hand, or runs these scripts, before goatbuilder's first phase.

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
| pod | Not built. A pull agent is refused (GB-307), and gate group D puts the pull converger after 1.0 | none |
| sensor nodes and gateways (Raspberry Pi 5) | Not built. Built today from two repositories outside goatcore | none in goatcore |
| phone | Built: `join-node --class phone` ([phone-node.md](phone-node.md)) | gate group H |

## 3. The design

### 3.1 A target type is data

One file per kind: `ops/goatbuilder/targets/<type>.yaml`. It declares five things.

```yaml
# ops/goatbuilder/targets/breakglass.yaml (sketch; the schema is part of step 1)
type: breakglass
os:                      # baselines the type accepts; doctor --target checks them
  - ubuntu: "24.04"
plays:                   # in order; the same plays join-node and standup run today
  - play: mgmt-host-baseline.yml
  - play: breakglass-nodes.yml
    group: "{site}_breakglass_hosts"
services: [netbird-client, oob-overlay]
acceptance:              # rows in ops/validation/acceptance-tests.yaml, by token
  requires: [breakglass]
proof:
  ci: vm                 # a VM from a fresh Ubuntu cloud image (section 3.4)
  hardware: required     # a recorded run on real equipment
```

- `standup`, `join-node` and the image tooling all read the file. `join-node --class <type>` accepts any type that has a file. `standup` builds a site's declared machines from the files of their types.
- The site still declares which machines it has, with ADR 1183's keys. A site's declaration says *whether*; the type file says *what*.
- A new kind of machine is a new file, not new code. A kind whose plays need logic of their own still gets a file, and the file names the verb that converges it.
- `ops/bootstrap/hardware-classes.yaml` keeps its platforms axis: what an image is built for. Its roles axis is replaced by the type files, and its aliases remain resolvable.

### 3.2 Check the end state; guide the method; never put it on rails

The contract starts at **a reachable Ubuntu host with an SSH key**, however it was installed. Above that line goatbuilder builds. Below it, goatbuilder guides, and it checks the result.

- **`goatbuilder doctor --target <type>`** checks a host's end state against its type, whatever path built it, and changes nothing. For example:
  - LUKS is on the root volume;
  - the OS version is one the type accepts;
  - chrony starts before docker;
  - the docker data root is on the tier the type declares;
  - the inotify floor is met;
  - for a break-glass node, ADR 1114 §5's dual-homing contract holds.

  Every refusal names what to fix on the machine, not which installer to use.
- **The golden path, optional:** `goatbuilder image <type>` wraps `ops/bootstrap`. On known hardware (the platforms of `hardware-classes.yaml`) it produces an unattended Ubuntu install with LUKS, first boot and enrolment, ready for the type's plays.
- **The supported general path:** written directions for each type that state the intent and the end state, for a capable person to work through on hardware the golden path does not know. `doctor --target` is how that person knows they arrived.
- **What goatbuilder does not do:** it is not an operating-system installer or configuration manager beyond the golden path. It does not partition disks, choose kernels or manage packages outside the type's plays.

### 3.3 Proof tiers, per type

| Tier | What runs | Where |
|---|---|---|
| CI | A VM from a fresh Ubuntu cloud image, then the image step where the type has one, then `join-node` or `standup`, then the type's acceptance rows and `doctor --target` | CI, extending ADR 1200's deployment-e2e harness from container hosts to VMs |
| Hardware | The same steps on real equipment, recorded as a run record | an existing site, for the types that site has |

Every type passes the CI tier before 1.0. The hardware tier is required where equipment exists: core, obs, breakglass, network-jump and the federation boundary node (gate group J).

### 3.4 Status per type

| Type | As a target type | Today's converge path |
|---|---|---|
| core | Designed | Built (`standup`, `join-node`) |
| obs | Designed | Built (`standup`, `join-node`) |
| breakglass | Designed | Built (`standup` phase 14, `join-node`) |
| network-jump | Designed | Built (`join-node`) |
| phone | Designed | Built (`join-node --class phone`) |
| federation boundary node | Designed | Not built as a machine kind (`federate` configures an existing host) |
| simple boundary node | Designed | Not built |
| processing node | Designed | Not built |
| pod | Designed; push or pull is an open question (section 5) | Not built |
| sensor nodes and gateways | Not built; the fit is being surveyed (section 6) | Not built in goatcore |

## 4. Build order

One revertible pull request per step, on the program's normal path. No dates.

1. **The type definition.** The schema, `ops/goatbuilder/targets/`, and core, obs, breakglass and network-jump moved onto it: `join-node` and `standup` read the files, and the compiled list at `joinnode.rs:90` goes. The same step adds acceptance rows for breakglass and network-jump, which have none today. Also `doctor --target` for these four types.
2. **Standup phase 11 automated.** The two manual steps at `standup.rs:611` and `standup.rs:617`: the first login, the admin promotion and the bootstrap NetBird token.
3. **The federation boundary node as a type.** The host `goatbuilder federate` configures becomes a machine `join-node` can build from a reachable Ubuntu host.
4. **The simple boundary node and the processing node as new types.** The processing node's acceptance runs the engines of the public `goat-processors` repository on recorded data, and checks the output against its conformance vectors.
5. **Sensor nodes and gateways.** Brought into goatbuilder as far as they fit, after the survey in section 6. What is too special is named, with the reason.
6. **Pods.** On the operator's answer to section 5.

`goatbuilder image <type>` (the golden path) and the CI tier's VM harness are built alongside step 1, for the first type that uses them, and each later step adds its type to both.

## 5. Open question for the operator: pods

On 15 November the operator decided that the pull converger comes after 1.0 (gate group D). A pod that is built and updated by push is possible under that decision: `join-node` reaches it like any other machine. So the proposal is that **pods enter 1.0 as push-deployed only**, and pull arrives with the converger after 1.0.

This is not decided here. The operator decides whether pods are push-only in 1.0, or wait for pull and leave 1.0's scope.

## 6. Sensor nodes and gateways: placeholder for the survey

The Raspberry Pi 5 sensor nodes and gateways are built today from two repositories outside goatcore. A survey of both is in progress. This section will record:

- which parts of that buildout fit a target type (OS baseline, plays, services, acceptance), and how;
- which parts are too special or too sprawling to bring into the command line, each with the reason.

Until the survey lands, this kind's status is Not built.

## 7. What does not change

- The node classes, their ACL positions and their enrolment rules stay as ADRs 1114 and 1131 set them. Enrolment of a break-glass node stays manual by ADR 1114's reasoning.
- A site still declares which rescue machines it has, or why it has none (ADR 1183).
- Roles, not hardware (ADR 1141): a target type names a role; the hardware is a procurement choice, and the golden path covers only the hardware it knows.
