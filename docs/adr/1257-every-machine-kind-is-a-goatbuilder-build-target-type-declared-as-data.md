# ADR 1257 — Every machine kind is a goatbuilder build target type, declared as data

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-11-19 |
| Deciders | operator |
| Repo scope | goat core: `crates/goatbuilder`, `ops/goatbuilder/targets/`, `ops/goatbuilder/payloads/`, `ops/bootstrap/` |
| Supersedes | — |
| Superseded by | — |
| Amends | [ADR 1141](1141-four-node-host-site-baseline.md), [ADR 1183](1183-rescue-nodes-are-part-of-the-site-baseline.md), [ADR 1114](1114-breakglass-node-class-and-two-path-reach.md), [ADR 1131](1131-network-jump-class-and-herd-gear-scope.md) (where each meets a target type; see Decision 8) |
| Related | [ADR 1200](1200-deployment-e2e-three-tiers.md) (deployment-e2e, which the CI tier extends), [ADR 1198](1198-goatbuilder-config-driven-deploy-release-cli.md) (goatbuilder), [ADR 1248](1248-goat-follows-stock-upstream-netbird-and-wireguard-and-retires-its-departures.md) (stock NetBird), [ADR 1253](1253-simple-boundary-node-pseudo-boundary-node-and-one-boundary-monitor.md) (simple boundary node); design [goatbuilder-build-targets.md](../design/goatbuilder-build-targets.md) |

## TL;DR (for a layperson)

A goatnet runs many kinds of machine, from the servers at a site to small sensor boxes. Today the build tool knows a few of them, each in its own code, and none from a bare machine. This decision keeps three things apart:

- the hardware and how it is prepared;
- the job a machine does in a goatnet;
- extra software that runs on some machines.

Each of the three is written down once as a file. The tool checks any machine against those files, however it was set up. On hardware we know, the tool can also do the whole job from a blank disk. On other hardware, a capable person follows written directions, and the tool checks that they got there. Every kind is built and proven in a fixed order before 1.0.

## Context

The operator set this as 1.0 scope on 2026-11-19, in these words, quoted as written. `[kit]` replaces the name of a private sensor-kit product line; every other word is the operator's.

1. "we need a deployment plan that is able to holistically deploy our services on a machine from the baremetal up just like we can do for other things like host site machines for core and obs and break glass and network jump box, and pods and [kit] kids and boundary nodes for federation and simple boundary nodes, we need the ability to build goat-processing nodes. all of these actually need to be first class build target types in goatbuilder and they should be sequenced to be built and proven capabilities before the 1.0 release I think."
2. "there might be a gap in that we manually install ubuntu and do luks encryption first, but we should really guide as much as possible without createing a negative abstraction constraint by forcing it to muuch to be on rails"
3. "yeah and I think that goatbuilder should incoproate the rpi and [kit] buildout stuff as much as that is possible so that admins can really use one tool for all infra mangement, unless it's just stoo special and sprawled to reign into the cli"
4. "let's be careful not to overdo it on the managment of os and such things though it's really hard to get that right with a cli, we can probably provide a golden path on known hardware and some generalization, but we need to maintain a supported generalized approach that relaies on a smart and driven human to understand intent and end state and work through gneral directions"
5. "I'm not so concerned about the nuances of before or after the cutoff for every detail as long as it's roughly corectr and everythign relevant is properly sequenced"
6. "these arent' two different things "Raspberry Pi and [kit] buildout" the pis need certain attention to prep them and in the future we will use the smaller computer that onLogic produces instead of pis by the way. and the [kit] kid is what runs on a small computer or a pod or a boundary node"
7. "or a break glass or a network jump box"

The operator agreed to the plan this ADR records, including two proof tiers. Statements 6 and 7 corrected the first model: the sensor kit is not a machine kind but software that runs on several of them, and the Pi is hardware that needs its own preparation.

What exists today is measured in the design document (§2). In short:

- Three taxonomies answer "what kind of machine is this", and none reads the others:
  - `join-node`'s compiled class list (`crates/goatbuilder/src/joinnode.rs:90`);
  - `standup`'s group_vars flags (`crates/goatbuilder/src/standup.rs:322`, `:691`, `:692`);
  - the image builder's roles (`ops/bootstrap/hardware-classes.yaml:111`).
- No verb installs or checks an operating system.
- Only some kinds are built, and few are proven: there is no hermetic founder standup, and there is no acceptance for break-glass or network-jump.

## Decision

1. **Three concepts, kept apart, each declared as data.**
   - **Platform:** the machine and its preparation: firmware, boot media, the OS image, the disk-encryption shape and how it unlocks. It is declared in `ops/bootstrap/hardware-classes.yaml`'s platforms axis, extended with each platform's preparation. Adding hardware adds a platform: the small OnLogic computer is a platform, Designed until the hardware is in hand.
   - **Target type:** the role a machine plays in a goatnet. It is one file, `ops/goatbuilder/targets/<type>.yaml`, declaring:
     - the platforms it accepts;
     - the minimum encryption scope it requires;
     - the OS baselines it accepts;
     - its plays, in order;
     - its services;
     - its acceptance checks;
     - its proof tiers.

     `standup`, `join-node` and the image tooling read it, and the compiled class list and the image roles are replaced by these files. The types are core, obs, breakglass, network-jump, federation boundary node, simple boundary node, processing node, pod, small computer (an edge device) and phone. **Nothing in a type may assume a platform**, and in particular none may assume a Raspberry Pi.
   - **Payload:** software deployed onto a target type. It is declared once, in `ops/goatbuilder/payloads/<payload>.yaml`, with its own acceptance and the list of types that may host it, as data. The sensor kit (the KID gear class) is the first payload. Its host types are small computer, pod, federation boundary node, breakglass and network-jump.
2. **Encryption scope is split between platform and type.**
   - The platform declares the scope it can deliver and how it unlocks. For example, a Karbon delivers `root` with TPM2 unlock; a Pi, with no TPM, delivers `data` with a hardware token, or `root` with a network unlock.
   - The type declares the minimum scope its role requires.
   - A type accepts only platforms that deliver its scope, and `doctor --target` checks the scope delivered.

   Why: unlock depends on the hardware (the existing file already says so, `hardware-classes.yaml:46`). How much must be encrypted is a security property of the role, checked on the end state. Joining the two would either make every type know every platform, or let a weaker platform lower a role's requirement.
3. **The contract starts at a reachable Ubuntu host with an SSH key.** goatbuilder builds above that line, whatever got the machine there.
4. **The end state is checked, whichever path was taken.** `goatbuilder doctor --target <type>` reads a host and reports each property its type and platform require, for example the encryption scope, the OS version, chrony before docker, the docker data root and the inotify floor. It changes nothing, and every refusal says what to fix on the machine. A payload's acceptance runs after its host passes.
5. **Two install paths below the line, and goatbuilder stays thin there.**
   - **Golden path, optional:** `goatbuilder image <platform>` wraps that platform's preparation in `ops/bootstrap`. Raspberry Pi preparation is golden-path data of the `rpi5` platform. Three Pi image builders exist today and differ, and one is picked before goatbuilder wraps any.
   - **General path, supported:** written directions per type that state the intent and the end state, for a capable person on any other hardware.

   Steps that stay outside the command line are given as written directions with the end state checked. One is the physical work on a Pi's drive and its unlock token. Another is the per-sensor packages of the sensor kit, which each sensor's owner authors with the sensor platform's own tool. goatbuilder is not an operating-system installer or configuration manager beyond the golden path.
6. **Two proof tiers.**
   - **CI:** a VM from a fresh Ubuntu cloud image, through the build, to the type's acceptance, `doctor --target` and each hosted payload's acceptance. This extends ADR 1200's deployment-e2e harness from container hosts to VMs.
   - **Hardware:** a recorded run on real equipment, from the platform's golden path.

   The 1.0 gate (`goatbuilder-v1-gate.md` group J) requires CI for every type and payload, and hardware for core, obs, breakglass, network-jump and the federation boundary node.
7. **The sequence.** Each step builds on the one before it; exact 1.0 cut lines are not decided item by item (statement 5).
   1. The type and platform definitions, with core, obs, breakglass and network-jump moved onto them.
   2. Standup phase 11 automated, by extending `ops/mgmt-host-bootstrap` with a verb that creates the admin service user and a hashed token. The plaintext is written once to a mode-0600 file under `/run`. There is no fork of NetBird's management service, under ADR 1248; this replaces the forked plan in `goatlite.md`'s table row 6. Designed. It mints an admin credential for the mesh control plane, so its build is reviewed as a trust change before it starts.
   3. The federation boundary node.
   4. The simple boundary node and the processing node.
   5. Raspberry Pi platform preparation, as golden-path data; then the small-computer type; then the sensor-kit payload. The payload goes first on a small computer, a pod and a federation boundary node, then on a break-glass node and a network-jump host.
   6. Pods, push-deployed.
8. **Where this amends earlier decisions:**
   - **ADR 1141** (Decision, point 1, "Roles, not hardware") stands, and is now explicit in the data: a role's machine contract is its target-type file, and hardware is a platform. `hardware-classes.yaml`'s roles axis is replaced by the type files.
   - **ADR 1183** (Decision: the two keys per class, and consequence 2, "`goat-standup.sh` phase 14 converges each declared node") stands for *whether* a site has each rescue node. *What* phase 14 converges is now the `breakglass` and `network-jump` type files, not plays named in the driver. Consequence 3, "a declared break-glass node is a graded peer", gains the type's acceptance rows, which do not exist today.
   - **ADR 1114** §5 (the dual-homing contract) becomes checks of the `breakglass` type in `doctor --target`: the route-metric asymmetry, out-of-band inbound closed, and TCP-level health on the out-of-band path. Today nothing checks them. §1 to §4 and the manual enrolment stand.
   - **ADR 1131** §2 (the `NETWORK-JUMP` peer, whose connector holds the key) becomes the `network-jump` type: its plays (`herd-connector-deploy.yml`), its required image input (`herd_connector_image`) and its acceptance. The class, its ACL shape and operator enrolment stand.
9. **Pods are push-deployed.** A pod is built and updated like any other type. The pull converger comes after 1.0, as the operator decided on 15 November (`goatbuilder-v1-gate.md` group D), and arrives as a value of `converger:`.

### Open questions for the operator

1. **Multi-sensor gateways.** Is a gateway that fronts several sensors a variant of the sensor-kit payload, or a separate payload?
2. **Fabric client certificates.** Does the site's own certificate authority issue the fabric client certificates of the payload's nodes?
3. **The Pi's OS baseline.** Ubuntu Server 24.04 or 26.04?
4. **The Pi's golden path.** Which of the existing Pi image builders becomes the `rpi5` platform's golden path? That choice also settles whether the platform delivers root or data-volume encryption.

## Alternatives considered

- **Keep the classes in code and add arms for the new kinds.** Every new kind would be new Rust, and the image builder and `standup` would keep their own lists. Rejected.
- **One concept, "machine kind", covering hardware, role and software.** This was the first draft of this plan. It made the sensor kit a machine kind and tied it to the Pi, and the operator corrected it (statement 6): the same kit runs on several roles, and the Pi is one of several platforms. Rejected.
- **goatbuilder installs and manages the operating system everywhere.** This is the rails the operator warned against (statements 2 and 4). Hardware goatbuilder does not know would have no supported path. Rejected in favour of a per-platform golden path plus the general path.
- **Leave the OS to the operator entirely, with no checks.** This is today's state. A hand install that misses its encryption or the docker data root fails later, far from its cause. `doctor --target` closes that without owning the install. Rejected.
- **Hardware proof only.** Most kinds have no equipment in the test laboratory, so most would never be proven. Rejected as the only tier.
- **Bring every step of the sensor-kit buildout into the command line.** Statement 3 bounds this to "as much as that is possible". The per-sensor packages belong to each sensor's owner and their tool, and the physical preparation of a device is a person's work. Both stay as written directions with the end state checked.

## Consequences

**Positive.**

- Every role, platform and payload has one written definition, read by every verb.
- A new role costs a file; new hardware costs a platform entry; new software on existing roles costs a payload file.
- Any install path is checked by the same end-state test.
- Rescue nodes and boundary nodes gain acceptance checks and proof.
- Standup's last manual step goes, without a fork of NetBird.

**Negative.**

- The 1.0 gate grows by one group.
- The CI tier needs VMs in CI, which costs runner time and needs a new harness beside ADR 1200's container hosts.
- The type, platform and payload schemas become compatibility surfaces under ADR 1228 once 1.0 ships.
- Moving four existing kinds onto the files touches `join-node` and `standup` together.
- Automating phase 11 puts an admin credential's creation in code, which must be reviewed as such.

## Assessor view (SCA-R)

This strengthens configuration management (NIST SP 800-53 CM-2, CM-6, CM-8) and system integrity (SI-7):

- every role, platform and payload has a declared baseline in the repository;
- every built machine is checked against it by a read-only test, whatever path installed it;
- full-disk or data-volume encryption (SC-28) and time synchronisation (AU-8) become checked properties per role, not installer conventions.

The automated phase 11 creates an administrative credential (AC-2, IA-5). It writes the plaintext once, to a root-only file, stores only a hash in the management database, and is tested for never appearing in logs.

Residual risk: on hardware outside the golden path, the install is a person's work. The compensating control is that `doctor --target` refuses the end state until it matches, and the run record keeps that result.

## Trigger conditions for revisit

- A role whose build cannot be expressed as plays in order on a reachable host.
- The pull converger ships (pods, and how every type updates in place).
- A second operating-system family enters a type's accepted baselines.
- The CI VM tier costs more runner time than the gate can carry on every pull request.
- The OnLogic hardware arrives (its platform moves from Designed to built).
