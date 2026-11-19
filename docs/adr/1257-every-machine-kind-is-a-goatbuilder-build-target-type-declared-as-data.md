# ADR 1257 — Every machine kind is a goatbuilder build target type, declared as data

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-11-19 |
| Deciders | operator |
| Repo scope | goat core: `crates/goatbuilder`, `ops/goatbuilder/targets/`, `ops/bootstrap/` |
| Supersedes | — |
| Superseded by | — |
| Amends | [ADR 1141](1141-four-node-host-site-baseline.md), [ADR 1183](1183-rescue-nodes-are-part-of-the-site-baseline.md), [ADR 1114](1114-breakglass-node-class-and-two-path-reach.md), [ADR 1131](1131-network-jump-class-and-herd-gear-scope.md) (where each meets a target type; see Decision 6) |
| Related | [ADR 1200](1200-deployment-e2e-three-tiers.md) (deployment-e2e, which the CI tier extends), [ADR 1198](1198-goatbuilder-config-driven-deploy-release-cli.md) (goatbuilder), [ADR 1253](1253-simple-boundary-node-pseudo-boundary-node-and-one-boundary-monitor.md) (simple boundary node), [ADR 1251](1251-every-source-keeps-its-identity-through-a-pod-carriage-rules-provenance-at-entry-and-pod-local-authority.md) (pods); design [goatbuilder-build-targets.md](../design/goatbuilder-build-targets.md) |

## TL;DR (for a layperson)

A goatnet runs many kinds of machine, from the servers at a site to small sensor boxes. Today the build tool knows a few of them, each in its own code, and none from a bare machine. This decision makes every kind a "build target": one file that says what that machine must be when it is finished. The tool checks any machine against that file, however it was set up. On hardware we know, the tool can also do the whole job from blank disk. On other hardware, a capable person follows written directions and the tool checks that they got there. Each kind must be proven on a test machine before 1.0, and on real equipment where we have it.

## Context

The operator set this as 1.0 scope on 2026-11-19, in these words, quoted as written. The brackets replace the name of a private product line; every other word is the operator's.

1. "we need a deployment plan that is able to holistically deploy our services on a machine from the baremetal up just like we can do for other things like host site machines for core and obs and break glass and network jump box, and pods and [the sensor-node kits] and boundary nodes for federation and simple boundary nodes, we need the ability to build goat-processing nodes. all of these actually need to be first class build target types in goatbuilder and they should be sequenced to be built and proven capabilities before the 1.0 release I think."
2. "there might be a gap in that we manually install ubuntu and do luks encryption first, but we should really guide as much as possible without createing a negative abstraction constraint by forcing it to muuch to be on rails"
3. "yeah and I think that goatbuilder should incoproate the rpi and [sensor-node kit] buildout stuff as much as that is possible so that admins can really use one tool for all infra mangement, unless it's just stoo special and sprawled to reign into the cli"
4. "let's be careful not to overdo it on the managment of os and such things though it's really hard to get that right with a cli, we can probably provide a golden path on known hardware and some generalization, but we need to maintain a supported generalized approach that relaies on a smart and driven human to understand intent and end state and work through gneral directions"

The operator agreed to the plan this ADR records, including two proof tiers. Whether pods are push-only in 1.0 is still open.

What exists today is measured in the design document (§2). In short:

- Three taxonomies answer "what kind of machine is this", and none reads the others:
  - `join-node`'s compiled class list (`crates/goatbuilder/src/joinnode.rs:90`);
  - `standup`'s group_vars flags (`crates/goatbuilder/src/standup.rs:322`, `:691`, `:692`);
  - the image builder's roles (`ops/bootstrap/hardware-classes.yaml:111`).
- No verb installs or checks an operating system.
- Only some kinds are built at all. Few are proven: there is no hermetic founder standup, and no acceptance for break-glass or network-jump.

## Decision

1. **A target type is data.** Each machine kind is one file, `ops/goatbuilder/targets/<type>.yaml`, declaring:
   - the OS baselines it accepts;
   - its plays, in order;
   - the services it carries;
   - its acceptance checks;
   - its proof tiers.

   `standup`, `join-node` and the image tooling read it. A new kind is a new file. The compiled class list and the image roles are replaced by these files. `hardware-classes.yaml` keeps its platforms axis.
2. **The contract starts at a reachable Ubuntu host with an SSH key.** goatbuilder builds above that line, whatever got the machine there.
3. **The end state is checked, whichever path was taken.** `goatbuilder doctor --target <type>` reads a host and reports each property its type requires, for example LUKS, the OS version, chrony before docker, the docker data root and the inotify floor. It changes nothing, and every refusal says what to fix on the machine.
4. **Two install paths below the line, and goatbuilder stays thin there.**
   - **Golden path, optional:** `goatbuilder image <type>` wraps `ops/bootstrap` for known hardware. It does an unattended Ubuntu install with LUKS, first boot and enrolment.
   - **General path, supported:** written directions per type that state the intent and the end state, for a capable person on any other hardware.

   goatbuilder is not an operating-system installer or configuration manager beyond the golden path.
5. **Two proof tiers per type.**
   - **CI:** a VM from a fresh Ubuntu cloud image, through the build, to the type's acceptance and `doctor --target`. This extends ADR 1200's deployment-e2e harness from container hosts to VMs.
   - **Hardware:** a recorded run on real equipment.

   The 1.0 gate (`goatbuilder-v1-gate.md` group J) requires CI for every type, and hardware for core, obs, breakglass, network-jump and the federation boundary node.
6. **Where this amends earlier decisions:**
   - **ADR 1141** (Decision, point 1, "Roles, not hardware") stands. This adds that each role's machine contract (baselines, plays, services, acceptance) is its target-type file, and that the hardware named in `hardware-classes.yaml` is an input to the golden path only, never a role.
   - **ADR 1183** (Decision: the two keys per class, and consequence 2, "`goat-standup.sh` phase 14 converges each declared node") stands for *whether* a site has each rescue node. *What* phase 14 converges is now the `breakglass` and `network-jump` type files, not plays named in the driver. Consequence 3, "a declared break-glass node is a graded peer", gains the type's acceptance rows, which do not exist today.
   - **ADR 1114** §5 (the dual-homing contract) becomes checks of the `breakglass` type in `doctor --target`: the route-metric asymmetry, out-of-band inbound closed, and TCP-level health on the out-of-band path. Today nothing checks them. §1 to §4 and the manual enrolment stand.
   - **ADR 1131** §2 (the `NETWORK-JUMP` peer, whose connector holds the key) becomes the `network-jump` type: its plays (`herd-connector-deploy.yml`), its required image input (`herd_connector_image`) and its acceptance. The class, its ACL shape and operator enrolment stand.
7. **Order.** The design document's build order (§4) is the order of work:
   1. the type definition, with core, obs, breakglass and network-jump moved onto it;
   2. standup phase 11 automated;
   3. the federation boundary node;
   4. the simple boundary node and the processing node;
   5. the sensor nodes and gateways, as far as they fit;
   6. pods.
8. **Pods are an open question for the operator.** The proposal is that pods enter 1.0 push-deployed only, following the 15 November decision that the pull converger comes after 1.0. It is not decided here.

## Alternatives considered

- **Keep the classes in code and add arms for the new kinds.** Every new kind would be new Rust, and the image builder and `standup` would keep their own lists. Three taxonomies would become three longer ones. Rejected.
- **goatbuilder installs and manages the operating system everywhere.** This is the rails the operator warned against (statements 2 and 4). Hardware goatbuilder does not know would have no supported path, and the tool would own a configuration-management problem it cannot do well. Rejected in favour of the golden path plus the general path.
- **Leave the OS to the operator entirely, with no checks.** This is today's state. A hand install that misses LUKS or the docker data root fails later, far from its cause. `doctor --target` closes that without owning the install. Rejected.
- **One proof tier: hardware only.** Most kinds have no equipment in the test laboratory, so most would never be proven. A CI VM proves the build on every pull request. Rejected as the only tier.
- **Bring every sensor-node and gateway buildout step into the command line.** Statement 3 bounds this to "as much as that is possible". The survey decides what fits, and names what does not, with reasons.

## Consequences

**Positive.** Every kind of machine has one written definition, read by every verb. A new kind costs a file. Any install path is checked by the same end-state test. Rescue nodes and boundary nodes gain acceptance checks and proof.

**Negative.**

- The 1.0 gate grows by one group, with a CI row for every type and a hardware row for five.
- The CI tier needs VMs in CI, which costs runner time and a new harness beside the container hosts of ADR 1200.
- The type file's schema becomes a compatibility surface under ADR 1228 once 1.0 ships.
- Moving four existing kinds onto the files touches `join-node` and `standup` together, which needs a careful revertible step.

## Assessor view (SCA-R)

This strengthens configuration management (NIST SP 800-53 CM-2, CM-6, CM-8) and system integrity (SI-7):

- every machine kind has a declared baseline in the repository;
- every built machine is checked against it by a read-only test, whatever path installed it;
- full-disk encryption (SC-28) and time synchronisation (AU-8) become checked properties instead of installer conventions.

Residual risk: on hardware outside the golden path, the install is a person's work. The compensating control is that `doctor --target` refuses the end state until it matches, and the run record keeps that result.

## Trigger conditions for revisit

- A machine kind whose build cannot be expressed as plays in order on a reachable host.
- The pull converger ships (pods, and how every type updates in place).
- A second operating system family enters a type's accepted baselines.
- The CI VM tier costs more runner time than the gate can carry on every pull request.
