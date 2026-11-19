# ADR 1257 — Every machine kind is a goatbuilder build target type, declared as data

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-11-19 |
| Deciders | operator |
| Repo scope | goat core: `crates/goatbuilder`, `ops/goatbuilder/targets/`, `ops/goatbuilder/families/`, `ops/bootstrap/` |
| Supersedes | — |
| Superseded by | — |
| Amends | [ADR 1141](1141-four-node-host-site-baseline.md), [ADR 1183](1183-rescue-nodes-are-part-of-the-site-baseline.md), [ADR 1114](1114-breakglass-node-class-and-two-path-reach.md), [ADR 1131](1131-network-jump-class-and-herd-gear-scope.md) (where each meets a target type; see Decision 8) |
| Related | [ADR 1200](1200-deployment-e2e-three-tiers.md) (deployment-e2e, which the CI tier extends), [ADR 1198](1198-goatbuilder-config-driven-deploy-release-cli.md) (goatbuilder), [ADR 1248](1248-goat-follows-stock-upstream-netbird-and-wireguard-and-retires-its-departures.md) (stock NetBird), [ADR 1253](1253-simple-boundary-node-pseudo-boundary-node-and-one-boundary-monitor.md) (simple boundary node); design [goatbuilder-build-targets.md](../design/goatbuilder-build-targets.md) |

## TL;DR (for a layperson)

A goatnet runs many kinds of machine, from the servers at a site to small boxes wired to sensors. Today the build tool knows a few of them, each in its own code, and none from a bare machine. This decision writes down, once each, two things:

- the families of hardware we use, and how each is prepared;
- the job a machine does in a goatnet.

It also lets each machine list the sensors wired to it, and the tool then turns on one small relay for each of those sensors. The tool checks any machine against these files, however it was set up. On hardware we know, the tool can also do the whole job from a blank disk. On other hardware, a capable person follows written directions, and the tool checks that they got there. Every kind is built and proven in a fixed order before 1.0.

## Context

The operator set this as 1.0 scope on 2026-11-19, in these words, quoted as written. `[kit]` replaces the name of a private sensor-kit product line, and `[service]` the name of a military service; every other word is the operator's.

1. "we need a deployment plan that is able to holistically deploy our services on a machine from the baremetal up just like we can do for other things like host site machines for core and obs and break glass and network jump box, and pods and [kit] kids and boundary nodes for federation and simple boundary nodes, we need the ability to build goat-processing nodes. all of these actually need to be first class build target types in goatbuilder and they should be sequenced to be built and proven capabilities before the 1.0 release I think."
2. "there might be a gap in that we manually install ubuntu and do luks encryption first, but we should really guide as much as possible without createing a negative abstraction constraint by forcing it to muuch to be on rails"
3. "yeah and I think that goatbuilder should incoproate the rpi and [kit] buildout stuff as much as that is possible so that admins can really use one tool for all infra mangement, unless it's just stoo special and sprawled to reign into the cli"
4. "let's be careful not to overdo it on the managment of os and such things though it's really hard to get that right with a cli, we can probably provide a golden path on known hardware and some generalization, but we need to maintain a supported generalized approach that relaies on a smart and driven human to understand intent and end state and work through gneral directions"
5. "I'm not so concerned about the nuances of before or after the cutoff for every detail as long as it's roughly corectr and everythign relevant is properly sequenced"
6. "these arent' two different things "Raspberry Pi and [kit] buildout" the pis need certain attention to prep them and in the future we will use the smaller computer that onLogic produces instead of pis by the way. and the [kit] kid is what runs on a small computer or a pod or a boundary node"
7. "or a break glass or a network jump box"

Later the same evening the operator corrected the model again:

8. ""small computer" is a family of platform type, not a target type. and why is [kit] the only payload? a [kit] kid is more like a target type that is run on a small computer as some other target types are as well. you also forgot octopod and firstbite"
9. "the [service] approved hardware that is the mobile edge node call the TSI V3 (the regular size and the mini)" ... "one that we probably wont meddle with because it's such a pain in the ass but worth mentioning and putting in the structure now"
10. "if we list aws ec2 we should probably list the other two counterparts for azure and gcp"
11. "the karbon 801/802 and thor and orin all belong to a familly of platforms that are suitable for running core services and obs services"
12. "I think you got the [kit] kid wrong unless it has changed a lot since I last looked at it you have it deploying like all the functionality -- is that actually true?"

The operator agreed to the plan this ADR records, including two proof tiers. Statements 6 to 12 corrected the first model twice. The Pi is hardware that needs its own preparation, and a small computer is a family of hardware, not a role. The kit's kids are per-sensor relay processes, not a whole stack, so a machine's wired sensors decide which of them run. The edge has two more roles, octo-pod and first-bite. Hardware comes in families, and the cloud family lists all three providers.

What exists today is measured in the design document (§2). In short:

- Three taxonomies answer "what kind of machine is this", and none reads the others:
  - `join-node`'s compiled class list (`crates/goatbuilder/src/joinnode.rs:90`);
  - `standup`'s group_vars flags (`crates/goatbuilder/src/standup.rs:322`, `:691`, `:692`);
  - the image builder's roles (`ops/bootstrap/hardware-classes.yaml:111`).
- No verb installs or checks an operating system.
- Only some kinds are built, and few are proven: there is no hermetic founder standup, and there is no acceptance for break-glass or network-jump.

## Decision

1. **Platform families and target types, kept apart, each declared as data; sensors declared per machine.**
   - **Platform family:** hardware that can play the same roles, and its preparation: firmware, boot media, the OS image, the disk-encryption shape and how it unlocks. One data file per family, under `ops/goatbuilder/families/`. A model joins a family by an entry in it, and no target type names a model. `ops/bootstrap/hardware-classes.yaml`'s platforms become the models of these families. The families:
     - **host-class:** Karbon 801 and 802, Jetson Orin and Thor; supported; suited to core and obs.
     - **cloud:** AWS EC2, supported; Azure VMs and Google Compute Engine, listed, not built.
     - **small computer:** Raspberry Pi 5 today, a smaller OnLogic model later; proposed.
     - **ruggedized mobile edge:** TSI V3 and TSI V3 mini; listed, not planned. They keep their own baseline, and goatbuilder only checks their end state.
   - **Target type:** the role a machine plays in a goatnet. It is one file, `ops/goatbuilder/targets/<type>.yaml`, declaring:
     - the families it accepts;
     - the minimum encryption scope it requires;
     - the OS baselines it accepts;
     - its plays, in order;
     - its services;
     - whether it allows sensors;
     - its acceptance checks;
     - its proof tiers.

     `standup`, `join-node` and the image tooling read it, and the compiled class list and the image roles are replaced by these files. The types are:
     - **host site:** core, obs, break-glass, network jump box;
     - **edge:** octo-pod (the edge gateway appliance), sensor node (a small computer whose only job is sensors), first-bite (the tap decoder);
     - **boundary:** federation boundary, simple boundary;
     - **processing:** goat-processors;
     - and phone, which `join-node` builds today.

     **Nothing in a type may assume a model**, and in particular none may assume a Raspberry Pi. A small computer is a family, not a type.
   - **Wired sensors, declared per machine.** Sensor support is a capability, not a machine: a machine's declaration lists its wired sensors, and goatbuilder installs the sensor platform and turns on the per-sensor relay processes for them, on any target type that allows sensors; the small-computer "sensor node" type is the one whose only job is sensors.
     - The kit's relays (its kids) convert a sensor's native output to ASTERIX, CoT, ISA or SAPIENT, and forward it to the fabric.
     - Declaring one sensor turns on that sensor's set of relay processes (tracks, status, events and so on), not a single process.
     - A machine with no sensors runs none.
     - The types that allow sensors today are sensor node, octo-pod, break-glass, network jump box and both boundary types.
     - There is no separate payload tier.
     - Which install model goatbuilder targets is open (question 5 below): one package with an activation list, or per-sensor bundles installed per machine.
2. **Encryption scope is split between family and type.**
   - The family declares the scope each model can deliver and how it unlocks. For example, a Karbon delivers `root` with TPM2 unlock; a Pi, with no TPM, delivers `data` with a hardware token, or `root` with a network unlock.
   - The type declares the minimum scope its role requires.
   - A type accepts only families whose models deliver its scope, and `doctor --target` checks the scope delivered.

   Why: unlock depends on the hardware (the existing file already says so, `hardware-classes.yaml:46`). How much must be encrypted is a security property of the role, checked on the end state. Joining the two would either make every type know every model, or let weaker hardware lower a role's requirement.
3. **The contract starts at a reachable Ubuntu host with an SSH key.** goatbuilder builds above that line, whatever got the machine there.
4. **The end state is checked, whichever path was taken.** `goatbuilder doctor --target <type>` reads a host and reports each property its type and family require, for example the encryption scope, the OS version, chrony before docker, the docker data root, the inotify floor and each declared sensor's relay processes running. It changes nothing, and every refusal says what to fix on the machine. For a family goatbuilder does not prepare, such as the ruggedized mobile edge, this check is goatbuilder's whole part below the line.
5. **Two install paths below the line, and goatbuilder stays thin there.**
   - **Golden path, optional:** `goatbuilder image <model>` wraps that model's preparation in `ops/bootstrap`. Raspberry Pi preparation is golden-path data of the Pi model in the small-computer family. Three Pi image builders exist today and differ, and one is picked before goatbuilder wraps any.
   - **General path, supported:** written directions per type that state the intent and the end state, for a capable person on any other hardware.

   Steps that stay outside the command line are given as written directions with the end state checked. One is the physical work on a Pi's drive and its unlock token. Another is the per-sensor packages, which each sensor's owner authors with the sensor platform's own tool. goatbuilder is not an operating-system installer or configuration manager beyond the golden path.
6. **Two proof tiers.**
   - **CI:** a VM from a fresh Ubuntu cloud image, through the build, to the type's acceptance, `doctor --target` and, where the type allows sensors, the relay processes of a declared test sensor. This extends ADR 1200's deployment-e2e harness from container hosts to VMs.
   - **Hardware:** a recorded run on real equipment, from the model's golden path.

   The 1.0 gate (`goatbuilder-v1-gate.md` group J) requires CI for every type, and hardware for core, obs, break-glass, network jump box and federation boundary.
7. **The sequence.** Each step builds on the one before it; exact 1.0 cut lines are not decided item by item (statement 5).
   1. The type and family definitions, with core, obs, break-glass and network jump box moved onto them.
   2. Standup phase 11 automated, by extending `ops/mgmt-host-bootstrap` with a verb that creates the admin service user and a hashed token. The plaintext is written once to a mode-0600 file under `/run`. There is no fork of NetBird's management service, under ADR 1248; this replaces the forked plan in `goatlite.md`'s table row 6. Designed. It mints an admin credential for the mesh control plane, so its build is reviewed as a trust change before it starts.
   3. Federation boundary.
   4. Simple boundary and goat-processors.
   5. The small-computer family, Raspberry Pi preparation first.
   6. octo-pod, sensor node and first-bite, with octo-pod push-deployed.
   7. Sensors declared per machine, with each declared sensor's relay processes turned on, on any type that allows them.
8. **Where this amends earlier decisions:**
   - **ADR 1141** (Decision, point 1, "Roles, not hardware") stands, and is now explicit in the data: a role's machine contract is its target-type file, and hardware is a model in a platform family (host-class is the family ADR 1141's core and obs run on). `hardware-classes.yaml`'s roles axis is replaced by the type files.
   - **ADR 1183** (Decision: the two keys per class, and consequence 2, "`goat-standup.sh` phase 14 converges each declared node") stands for *whether* a site has each rescue node. *What* phase 14 converges is now the `breakglass` and `network-jump` type files, not plays named in the driver. Consequence 3, "a declared break-glass node is a graded peer", gains the type's acceptance rows, which do not exist today.
   - **ADR 1114** §5 (the dual-homing contract) becomes checks of the `breakglass` type in `doctor --target`: the route-metric asymmetry, out-of-band inbound closed, and TCP-level health on the out-of-band path. Today nothing checks them. §1 to §4 and the manual enrolment stand.
   - **ADR 1131** §2 (the `NETWORK-JUMP` peer, whose connector holds the key) becomes the `network-jump` type: its plays (`herd-connector-deploy.yml`), its required image input (`herd_connector_image`) and its acceptance. The class, its ACL shape and operator enrolment stand.
9. **octo-pod is push-deployed.** It is built and updated like any other type. The pull converger comes after 1.0, as the operator decided on 15 November (`goatbuilder-v1-gate.md` group D), and arrives as a value of `converger:`.

### Open questions for the operator

1. **Multi-sensor machines.** A machine with several wired sensors runs each sensor's set of relay processes. Is a gateway that fronts several sensors anything more than a machine with a longer sensor list?
2. **Fabric client certificates.** Does the site's own certificate authority issue the fabric client certificates of the machines that run sensor relay processes?
3. **The Pi's OS baseline.** Ubuntu Server 24.04 or 26.04?
4. **The Pi's golden path.** Which of the existing Pi image builders becomes the Pi's golden path? That choice also settles whether the Pi delivers root or data-volume encryption.
5. **The sensor install model.** The sensor platform's design installs one package on every edge machine and activates only the relevant relay processes, from an activation list whose format is not fixed yet. Its current public release ships a sensor-neutral package, with each sensor's relay processes in a separate bundle installed and activated per machine. And one prototype starts a sensor's processes only when it is both declared and discovered live on the local network. Which of these does goatbuilder target?

## Alternatives considered

- **Keep the classes in code and add arms for the new kinds.** Every new kind would be new Rust, and the image builder and `standup` would keep their own lists. Rejected.
- **One concept, "machine kind", covering hardware, role and software.** This was the first draft of this plan. It made the sensor kit a machine kind and tied it to the Pi, and the operator corrected it (statement 6). Rejected.
- **The sensor kit as a payload that deploys the kit's whole stack onto a host.** This was the second draft. The operator corrected it (statements 8 and 12): a kid is one relay per sensor, the same package goes on every edge machine, and only the kids for the wired sensors run. A payload tier would also have added a third concept for what is a property of one machine. Rejected in favour of sensors declared per machine.
- **The small computer as a target type.** It is hardware that several roles run on (statement 8), so it is a family. Rejected.
- **goatbuilder installs and manages the operating system everywhere.** This is the rails the operator warned against (statements 2 and 4). Hardware goatbuilder does not know would have no supported path. Rejected in favour of a per-platform golden path plus the general path.
- **Leave the OS to the operator entirely, with no checks.** This is today's state. A hand install that misses its encryption or the docker data root fails later, far from its cause. `doctor --target` closes that without owning the install. Rejected.
- **Hardware proof only.** Most kinds have no equipment in the test laboratory, so most would never be proven. Rejected as the only tier.
- **Bring every step of the sensor-kit buildout into the command line.** Statement 3 bounds this to "as much as that is possible". The per-sensor packages belong to each sensor's owner and their tool, and the physical preparation of a device is a person's work. Both stay as written directions with the end state checked.

## Consequences

**Positive.**

- Every role and hardware family has one written definition, read by every verb, and every machine's sensors are declared next to it.
- A new role costs a file; new hardware costs a model entry in a family; a new sensor on a machine costs a line in its declaration.
- Any install path is checked by the same end-state test.
- Rescue nodes and boundary nodes gain acceptance checks and proof.
- Standup's last manual step goes, without a fork of NetBird.

**Negative.**

- The 1.0 gate grows by one group.
- The CI tier needs VMs in CI, which costs runner time and needs a new harness beside ADR 1200's container hosts.
- The type and family schemas, and the per-machine sensor declaration, become compatibility surfaces under ADR 1228 once 1.0 ships.
- Moving four existing kinds onto the files touches `join-node` and `standup` together.
- Automating phase 11 puts an admin credential's creation in code, which must be reviewed as such.

## Assessor view (SCA-R)

This strengthens configuration management (NIST SP 800-53 CM-2, CM-6, CM-8) and system integrity (SI-7):

- every role and hardware family has a declared baseline in the repository, and every machine's sensors are declared;
- every built machine is checked against it by a read-only test, whatever path installed it;
- full-disk or data-volume encryption (SC-28) and time synchronisation (AU-8) become checked properties per role, not installer conventions.

The automated phase 11 creates an administrative credential (AC-2, IA-5). It writes the plaintext once, to a root-only file, stores only a hash in the management database, and is tested for never appearing in logs.

Residual risk: on hardware outside the golden path, the install is a person's work. The compensating control is that `doctor --target` refuses the end state until it matches, and the run record keeps that result.

## Trigger conditions for revisit

- A role whose build cannot be expressed as plays in order on a reachable host.
- The pull converger ships (pods, and how every type updates in place).
- A second operating-system family enters a type's accepted baselines.
- The CI VM tier costs more runner time than the gate can carry on every pull request.
- The OnLogic hardware arrives (its model joins the small-computer family).
