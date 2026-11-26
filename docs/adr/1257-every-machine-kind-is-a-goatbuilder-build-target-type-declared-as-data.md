# ADR 1257 — Every machine kind is a goatbuilder build target type, declared as data

| Field | Value |
|---|---|
| Status | Accepted (2026-11-26, operator: "let's do this and aim for the lower range", said in answer to accepting this ADR and fanning group J out now; proposed 2026-11-19); built in part (gate group J) |
| Date | 2026-11-19 |
| Deciders | operator |
| Repo scope | goat core: `crates/goatbuilder`, `ops/goatbuilder/targets/`, `ops/goatbuilder/families/`, `ops/bootstrap/` |
| Supersedes | — |
| Superseded by | — |
| Amends | [ADR 1141](1141-four-node-host-site-baseline.md), [ADR 1183](1183-rescue-nodes-are-part-of-the-site-baseline.md), [ADR 1114](1114-breakglass-node-class-and-two-path-reach.md), [ADR 1131](1131-network-jump-class-and-herd-gear-scope.md) (where each meets a target type; see Decision 9) |
| Related | [ADR 1200](1200-deployment-e2e-three-tiers.md) (deployment-e2e, which the CI tier extends), [ADR 1198](1198-goatbuilder-config-driven-deploy-release-cli.md) (goatbuilder), [ADR 1248](1248-goat-follows-stock-upstream-netbird-and-wireguard-and-retires-its-departures.md) (stock NetBird), [ADR 1253](1253-simple-boundary-node-pseudo-boundary-node-and-one-boundary-monitor.md) (simple boundary node), [ADR 1255](1255-vendor-systems-connect-through-one-vendor-bridges-repository-and-open-standards-go-through-goat-transformers.md) (vendor bridges); design [goatbuilder-build-targets.md](../design/goatbuilder-build-targets.md) |

## TL;DR (for a layperson)

A goatnet runs many kinds of machine: the servers at a site, small boxes that let an operator in when the network is down, small boxes wired to sensors, gateways to partners. Today the build tool knows a few of them, each in its own code, and none from a bare machine. This decision writes down, once each, two things:

- the **families of hardware** we use, and how each is prepared;
- the **job** a machine does in a goatnet (its target type).

The tool builds any machine from those files and checks the result, however the machine was first set up. On hardware we know, the tool can also prepare the machine from a blank disk. On other hardware, a capable person follows written directions, and the tool checks that they got there. One shared foundation is built first; after it, the work runs in parallel lanes, and every type is built and proven before 1.0. A service that runs on a machine is not a type: the tool's deploy command places it. A team can add its own types and services in its private files without changing the tool.

## Context

The operator set this as 1.0 scope on 2026-11-19 and corrected the model through the evening, in these words, quoted as written. `[kit]` replaces the name of a private sensor-kit product line, `[service]` the name of a military service and `[a teammate]` a person's name; every other word is the operator's.

1. "we need a deployment plan that is able to holistically deploy our services on a machine from the baremetal up just like we can do for other things like host site machines for core and obs and break glass and network jump box, and pods and [kit] kids and boundary nodes for federation and simple boundary nodes, we need the ability to build goat-processing nodes. all of these actually need to be first class build target types in goatbuilder and they should be sequenced to be built and proven capabilities before the 1.0 release I think."
2. "there might be a gap in that we manually install ubuntu and do luks encryption first, but we should really guide as much as possible without createing a negative abstraction constraint by forcing it to muuch to be on rails"
3. "yeah and I think that goatbuilder should incoproate the rpi and [kit] buildout stuff as much as that is possible so that admins can really use one tool for all infra mangement, unless it's just stoo special and sprawled to reign into the cli"
4. "let's be careful not to overdo it on the managment of os and such things though it's really hard to get that right with a cli, we can probably provide a golden path on known hardware and some generalization, but we need to maintain a supported generalized approach that relaies on a smart and driven human to understand intent and end state and work through gneral directions"
5. "I'm not so concerned about the nuances of before or after the cutoff for every detail as long as it's roughly corectr and everythign relevant is properly sequenced"
6. "the pis need certain attention to prep them and in the future we will use the smaller computer that onLogic produces instead of pis by the way. and the [kit] kid is what runs on a small computer or a pod or a boundary node" ... "or a break glass or a network jump box"
7. ""small computer" is a family of platform type, not a target type. ... you also forgot octopod and firstbite"
8. "the [service] approved hardware that is the mobile edge node call the TSI V3 (the regular size and the mini)" ... "one that we probably wont meddle with because it's such a pain in the ass but worth mentioning and putting in the structure now"
9. "if we list aws ec2 we should probably list the other two counterparts for azure and gcp"
10. "the karbon 801/802 and thor and orin all belong to a familly of platforms that are suitable for running core services and obs services"
11. "I think [kit] kids just run integration componenets and nothing else"
12. "i think the gateway would b e another thing that runs on a small computer"
13. "for the pis pay attention to the pi script that [a teammate] has been honing even recently, I think we need that level of builder for any hardware that we use"
14. (about a strictly linear build order) "you think that each one needs to build on the last? where is that coming from?"
15. "not included but it should be acknowledged is the network gear the switches and routers that connecting things at local sites and connect them to transport" ... "jump box needs to be clarified that it's for network gear only"
16. "break glass and network jump box are both small computers they just facilitate entery into othe rmachines"
17. "boundary nodes and pods could be eithe rmsall omcputers or host class computers depednign on the load the need for resilience and ruggedization"
18. "processing should probably be host class due to the load the need for resilience"

Added on 22 and 23 November (`[...]` marks words left out):

19. "we need to be able to build a passive fail open tab on a small computer (like a pi) with two nics."
20. "we need to get goatbuilder to 1.0 and we need to enable it's ability to build all possible goat services including th enew ones that we built out this week [...]"

What exists today is measured in the design document (§2). In short:

- Three taxonomies answer "what kind of machine is this", and none reads the others:
  - `join-node`'s compiled class list (`crates/goatbuilder/src/joinnode.rs:90`);
  - `standup`'s group_vars flags (`crates/goatbuilder/src/standup.rs:322`, `:710`, `:711`);
  - the image builder's roles (`ops/bootstrap/hardware-classes.yaml:111`).
- No verb installs or checks an operating system.
- Only some kinds are built, and few are proven: there is no hermetic founder standup, and there is no acceptance for break-glass or network-jump.
- The edge kinds (octo-pod, the sensor kit, first-bite, the vendor bridges) each have their own repository and their own installer, and goatbuilder calls none of them.

## Decision

1. **Platform families, declared as data.** A family is hardware that can play the same roles, with its preparation: firmware, boot media, the OS image, the disk-encryption shape and how it unlocks. One file per family, under `ops/goatbuilder/families/`. A model joins a family by an entry in it, and no target type names a model. `ops/bootstrap/hardware-classes.yaml`'s platforms become the models of these families.

   | Family | Models | Status |
   |---|---|---|
   | host-class | Karbon 801 and 802, Jetson Orin and Thor | supported; suited to core, obs and other heavy or resilient roles |
   | cloud | AWS EC2; Azure Virtual Machines and Google Compute Engine | EC2 supported; Azure and Google listed, not built |
   | small computer | Raspberry Pi 5 today; a smaller OnLogic model later | proposed |
   | ruggedized mobile edge | TSI V3 and TSI V3 mini | listed, not planned: they keep their own baseline, and goatbuilder only checks their end state |

   **Every family's builder works at the level of the team's Raspberry Pi base-image builder** (statement 13): numbered, re-runnable steps from blank media to a verified machine, each proving its result before the next, ending in a check of the built box. That builder is the small-computer family's golden path. The other Pi preparations (an SD-seed self-install, and the Clevis-and-Tang proposal in `hardware-classes.yaml`) fold into it. The host-class and cloud builders (`ops/bootstrap/build-image-*.sh`) are measured against the same level.

2. **Target types, declared as data.** A target type is the role a machine plays in a goatnet. One file, `ops/goatbuilder/targets/<type>.yaml`, declares the families it accepts, the minimum encryption scope it requires, the OS baselines it accepts, its plays in order, its services, its acceptance checks and its proof tiers. `standup`, `join-node` and the image tooling read it, and the compiled class list and the image roles are replaced by these files.

   | Group | Type | What it does | Families |
   |---|---|---|---|
   | host site | core | management plane: mesh control, identity, fabric router | host-class, cloud |
   | host site | obs | observability and storage | host-class, cloud |
   | access | break-glass | entry into the site's goat machines when the mesh and control plane are both down | small computer |
   | access | network-gear jump box | entry into the site's network gear only: switches, routers, out-of-band management. Not a general jump host | small computer (a Thor serves today) |
   | edge | octo-pod | edge gateway: takes in feeds, records and audits them, releases chosen feeds to the fabric | small computer or host-class |
   | edge | Billie node | runs the sensor kit's relays for the sensors wired to it | small computer |
   | edge | vendor gateway | runs the vendor bridges (ADR 1255) | small computer |
   | edge | first-bite | decodes binary messages from a network tap | small computer or host-class |
   | edge | network tap | records one Ethernet link (Decision 12) | small computer or host-class |
   | boundary | federation boundary | the edge router that peers with another goatnet | small computer or host-class |
   | boundary | simple boundary | sends selected data out to a partner receiver | small computer or host-class |
   | processing | goat-processors | correlation, fusion, filtering and quality | host-class |

   `join-node` also builds `phone` today, which becomes a type file like the rest. Where a type accepts two families, the machine's declaration picks one, by load, resilience and ruggedization (statement 17). **Nothing in a type may assume a model**, and in particular none may assume a Raspberry Pi.

3. **The sensor kit's relays are integration components, and the Billie node runs them.** A kid decodes one sensor's native output and converts it to ASTERIX, CoT, ISA or SAPIENT, and nothing else (statement 11). A Billie node's declaration lists its wired sensors; goatbuilder installs the sensor platform and turns on each declared sensor's relays. Whether types other than the Billie node host relays (statement 6) is open (question 3), because the kit's platform brings its own local router and container runtime onto the host. How the relays are packaged and installed is open (question 1).

4. **Each machine is built in six steps.**
   1. **Declare it** in the site config: its family and model, its target type, and for a Billie node its sensors.
   2. **Install the OS and disk encryption:** scripted on a known model (the family's golden path), or by hand from written directions.
   3. **Host baseline:** hardening, packages and WireGuard modules, then checked.
   4. **Join the goatnet:** enrolment and mesh address.
   5. **Install the type:** its services, over the mesh.
   6. **Prove it:** the type's acceptance checks, written to the deploy ledger.

   Steps 3 to 6 are what `join-node` does today (`joinnode.rs`, phases 4 to 6, 10 to 12, 13 and 15). A founder site differs at step 4: there is no goatnet to join yet, so its core install creates the mesh control plane before anything enrols. An edge kind whose own installer enrols the machine keeps that order, and goatbuilder checks the end state.

5. **The contract starts at a reachable Ubuntu host with an SSH key**, and **the end state is checked, whichever path was taken.** `goatbuilder doctor --target <type>` reads a host and reports each property its type and family require: the encryption scope, the OS version, chrony before docker, the docker data root, the inotify floor, and the type's own properties. It changes nothing, and every refusal says what to fix on the machine, not which installer to use. For a family goatbuilder does not prepare, such as the ruggedized mobile edge, this check is goatbuilder's whole part below the line.

6. **goatbuilder stays thin below the line.** The golden path (`goatbuilder image <model>`) wraps a family's builder; the general path is written directions that state the intent and the end state. Steps that need a person stay as directions with the end state checked: the physical work on a Pi's drive and its unlock token, and authoring a sensor's relays. goatbuilder is not an operating-system installer or configuration manager beyond the golden path. **Edge kinds keep their own installers:** a type whose software has its own repository and installer (octo-pod, the sensor kit, first-bite, the vendor bridges) calls and checks that installer at a pinned release; it does not reimplement it.

7. **Encryption scope is split between family and type.** The family declares the scope each model can deliver and how it unlocks (a Karbon delivers `root` with TPM2; a Pi, with no TPM, delivers `root` with a hardware-token unlock). The type declares the minimum scope its role requires. A type accepts only families whose models deliver its scope, and `doctor --target` checks the scope delivered. Unlock depends on the hardware (`hardware-classes.yaml:46`); how much must be encrypted is a property of the role.

8. **Two proof tiers.**
   - **CI:** a VM from a fresh cloud image, through steps 3 to 6 and `doctor --target`. This extends ADR 1200's deployment-e2e harness from container hosts to VMs. For the small-computer family, CI proves the software on an arm64 VM; the physical steps of the Pi builder are proven on the bench.
   - **Hardware:** a recorded run on real equipment, from the family's golden path.

   The 1.0 gate (`goatbuilder-v1-gate.md` group J) requires CI for every type, and hardware where the equipment exists.

9. **Where this amends earlier decisions:**
   - **ADR 1141** (point 1, "Roles, not hardware") stands, and is now explicit in the data: a role's machine contract is its target-type file, and hardware is a model in a family (host-class is the family ADR 1141's core and obs run on). `hardware-classes.yaml`'s roles axis is replaced by the type files.
   - **ADR 1183** stands for *whether* a site has each rescue node. *What* `standup` phase 14 converges is now the `breakglass` and `network-jump` type files. Consequence 3, "a declared break-glass node is a graded peer", gains the type's acceptance rows, which do not exist today.
   - **ADR 1114** §5 (the dual-homing contract) becomes checks of the `breakglass` type in `doctor --target`. The break-glass node moves to the small-computer family (statement 16). §1 to §4 and the manual enrolment stand.
   - **ADR 1131** §2 becomes the `network-jump` type, renamed in prose "network-gear jump box" to state its scope (statement 15): its plays (`herd-connector-deploy.yml`), its required image input (`herd_connector_image`) and its acceptance. The class name, its ACL shape and operator enrolment stand.

10. **The order of work: one foundation, then five lanes in parallel** (statements 5 and 14). The only dependencies are real ones:
    - **Foundation, first:** the type and family file formats and their reader, and the `doctor` base check. Nothing moves onto files before they exist. It is built incrementally: core first, proven unchanged against today's plans, then each other type.
    - **Host-site lane:** core and obs onto type files; `standup` phase 11 automated (by extending `ops/mgmt-host-bootstrap` with a verb that creates the admin service user and a hashed token, no fork of NetBird under ADR 1248; reviewed as a trust change before it starts).
    - **Boundary lane:** federation boundary and simple boundary. Their small-computer variants wait for the small-computer builder.
    - **Processing lane:** goat-processors, whose acceptance runs the engines on recorded data and checks the output against the conformance vectors.
    - **Small-computer lane:** the small-computer builder first; then break-glass, network-gear jump box, Billie node, octo-pod, vendor gateway, first-bite and network tap on it.
    - **Services lane:** the `release` artifact type and the services that need it, and the check that every deploy play is reached by a verb or listed; these wait for nothing. Overlay declarations and a type's services list wait for the foundation (Decisions 13 and 14).

    Exact 1.0 cut lines are not decided item by item (statement 5). octo-pod is push-deployed like every other type; the pull converger comes after 1.0, as the operator decided on 15 November (gate group D).

11. **Network gear is outside goatbuilder, and acknowledged** (statement 15). Each site's switches and routers connect its machines to each other and the site to its transport. goatbuilder does not configure them, reaches them only through the network-gear jump box, and every machine it builds depends on them.

12. **A network tap is a machine type with a declared mode** (statement 19). It has two wired interfaces for the link, and its management, mesh and default route on a third. In **listen** mode it only receives, from a passive copper tap, a bypass relay or a switch mirror port, and the link is unaffected by the machine: this is the fail-open tap the operator asked for. In **inline** mode the machine bridges the link, and the link is down whenever the machine is. A small computer that bridges two interfaces is always inline. A declaration states its mode, with no default. Inline is supported: the operator decided to fully support the Raspberry Pi setup now and to improve the tap's hardware later. In either mode a tap port and the bridge carry no address, and the bridge runs no STP.

13. **A service is not a type** (statement 20). A service is software on a machine that already exists, declared as a component of `ops/pins/fleet-pins.yaml` and placed by `goatbuilder deploy`. One that `deploy` covers today needs nothing. A service that ships as a file takes a new artifact type, `release`: repository, tag, file per architecture, digest and signer, verified before any host is touched. A type lists its services, and its build deploys them by the same code. Every deploy play in the public repositories is reached by a verb or is listed with its reason, and CI checks it.

14. **Types and services can be declared in a private overlay.** The loader reads type files from the assembled tree, so an overlay adds a type by adding a file, and a service by adding a pins row. Either names its source repository, a full commit or signed tag, a digest and a signer, and its play and installer stay in that repository. The operator's own access fetches it; goatbuilder holds no credential for it. An overlay adds types and never replaces a public one. The public code, documents, tests and releases name none of them, and public CI proves the mechanism with a synthetic type.

### Open questions for the operator

1. **How the sensor kit is installed.** The kit's original design installs one package on every edge machine and turns on only the relays for its sensors. Its current prototype composes a chosen set of relays per machine and starts only those whose sensor is discovered live on the local network. Its public sensor-neutral platform (release 0.3.0 in preparation, unsigned so far) installs a platform package and then one bundle per sensor and architecture, with `install`, `configure` and `activate` per machine. The recommendation is the public platform's model: it is the one with a per-machine command set goatbuilder can call and check, and it matches what both newer lines already do (per machine, not uniform). It needs a signed release and signed bundles before goatbuilder relies on it.
2. **Fabric client certificates.** Decided by the operator (23 Nov): the site's own certificate authority issues them for Billie nodes.
3. **Relays on other types.** Do any types besides the Billie node run sensor relays, given that the kit's platform brings its own local router and container runtime?
4. **The Pi's OS baseline.** Ubuntu Server 24.04 (the public sensor platform supports only 24.04) or 26.04?
5. **The tap's repository.** Decided by the operator (23 Nov): a public repository of its own, usable without goat, with its own releases and owner. first-bite is not its home: it never transmits on its capture interface, and an inline tap does. Open: the name and the owner.
6. **The fail-open part.** Decided by the operator (23 Nov): the Raspberry Pi setup is fully supported now, inline included, and the hardware is improved later. No fail-open source is standardized yet.

## Alternatives considered

- **Keep the classes in code and add arms for the new kinds.** Every new kind would be new Rust, and the image builder and `standup` would keep their own lists. Rejected.
- **One concept, "machine kind", covering hardware, role and software.** The first draft. It made the sensor kit a machine kind and tied it to the Pi; the operator corrected it. Rejected.
- **The sensor kit as a payload tier deployed onto many types.** The second draft. A kid is an integration component, not a stack to place on any machine (statement 11). Rejected.
- **The small computer as a target type.** It is hardware that several roles run on (statement 7). Rejected.
- **A strictly linear build order.** Nothing but the foundation and the small-computer builder forces an order (statement 14). Rejected in favour of parallel lanes.
- **goatbuilder reimplements the edge kinds' installers.** Duplicates working tools and takes them from their owners. Rejected in favour of calling and checking them at a pinned release.
- **goatbuilder installs and manages the operating system everywhere.** The rails the operator warned against (statements 2 and 4). Rejected.
- **Leave the OS to the operator entirely, with no checks.** Today's state: a hand install that misses its encryption fails later, far from its cause. Rejected.
- **A target type for every service.** A service has no OS, encryption or enrolment of its own, and many run on one machine. Rejected in favor of components and a type's services list.
- **A network tap that is inline by default.** The link would depend on the machine without anyone having said so. Rejected; inline is supported and declared.
- **Private types compiled into goatbuilder, or a private build of it.** Every team would no longer run the same tool. Rejected in favor of overlay data.
- **Hardware proof only.** Most kinds have no equipment in the test laboratory. Rejected as the only tier.

## Consequences

**Positive.**

- Every role and hardware family has one written definition, read by every verb.
- A new role costs a file; new hardware costs a model entry in a family.
- Any install path is checked by the same end-state test.
- Rescue nodes and boundary nodes gain acceptance checks and proof.
- Standup's last manual step goes, without a fork of NetBird.
- Five lanes can be built by separate people at once.

**Negative.**

- The 1.0 gate grows by one group.
- The CI tier needs VMs in CI, which costs runner time and needs a new harness beside ADR 1200's container hosts.
- The type and family schemas become compatibility surfaces under ADR 1228 once 1.0 ships.
- Moving the existing kinds onto the files touches `join-node` and `standup` together.
- Automating phase 11 puts an admin credential's creation in code, which must be reviewed as such.
- The edge kinds depend on other repositories' releases, which must be pinned and signed.
- The pins schema gains an artifact type that both deploy drivers must carry.
- A type in an overlay is proven only by its owner's CI.

## Assessor view (SCA-R)

This strengthens configuration management (NIST SP 800-53 CM-2, CM-6, CM-8) and system integrity (SI-7):

- every role and hardware family has a declared baseline in the repository;
- every built machine is checked against it by a read-only test, whatever path installed it;
- disk encryption (SC-28) and time synchronization (AU-8) become checked properties per role, not installer conventions;
- software from other repositories is installed only at a pinned, signed release.

The automated phase 11 creates an administrative credential (AC-2, IA-5). It writes the plaintext once, to a root-only file, stores only a hash in the management database, and is tested for never appearing in logs.

A network tap records traffic (AU-9, SC-28): its recording is bounded and on an encrypted root. An inline tap is a point of failure on the link it records (CP-2), so its mode is declared and shown wherever the machine is reported. Code from a private source runs only at a pinned commit, digest and signer (SI-7, SR-4).

Residual risk: on hardware outside the golden path, the install is a person's work. The compensating control is that `doctor --target` refuses the end state until it matches, and the run record keeps that result.

## Trigger conditions for revisit

- A role whose build cannot be expressed as plays in order on a reachable host.
- The pull converger ships (how every type updates in place).
- A second operating-system family enters a type's accepted baselines.
- The CI VM tier costs more runner time than the gate can carry on every pull request.
- The OnLogic hardware arrives (its model joins the small-computer family).
- The sensor kit's install model is decided (question 1).
