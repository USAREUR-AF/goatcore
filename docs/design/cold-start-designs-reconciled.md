# Cold-start designs reconciled: what the stand-up uses, what it leaves unused, and what to combine

| Field | Value |
|---|---|
| Status | **Written.** A reconciliation of designs and code that already exist. It builds nothing. The decision it led to is [ADR 1277](../adr/1277-one-signed-site-bundle-is-the-cold-start-artifact.md), Accepted on 2026-11-28 and not built. The build order is tracked in [issue 1489](https://github.com/USAREUR-AF/goatcore/issues/1489) |
| Reads with | [goatbuilder-speed.md](goatbuilder-speed.md) (the budget and the levers) · [goatbuilder-v1-gate.md](goatbuilder-v1-gate.md) (row C1 and group D) · [bootstrap-bundle.md](bootstrap-bundle.md) · [self-healing-substrate-sync.md](self-healing-substrate-sync.md) · [goatbuilder-pull-converger-options.md](goatbuilder-pull-converger-options.md) · [ADR 1237](../adr/1237-a-site-is-rebuilt-from-nothing-and-a-combed-extract-of-its-identities.md) |

## What this is

The tree holds two generations of work on the same problem: getting a site from
blank machines to running, fast, and getting it back after a loss. The first
generation (April to September 2026) designed a cloud-held bootstrap bundle, an
image mirror in three tiers, pre-baked operating-system images, a per-site image
registry kept in sync from the cloud, and a boot-time self-update for hosts. The
second generation is `goatbuilder standup`, `join-node` and `teardown`, with the
levers in [goatbuilder-speed.md](goatbuilder-speed.md).

The timed stand-up uses the second generation only. This document lists every
mechanism of both, states what the code does today, and gives a verdict for
each: **superseded**, **adopt**, or **combine**. Statuses come from reading the
code, not from each document's own status line.

Status words: **measured** (a run produced the number), **built** (the code is
on `main` and something calls it), **written** (the code or document is on
`main` and nothing in the stand-up calls it), **designed** (decided, no code),
**not built**.

## The result in four sentences

1. The stand-up's own levers are the right base, and none of the earlier
   designs replaces one of them.
2. One earlier idea is still the best design and is not used: a signed archive
   of everything a site needs, fetched or carried in one piece. Two producers
   of such an archive exist on `main` and no stand-up step consumes either.
3. The site cache and that archive are two halves of one mechanism. The cache
   serves the second machine onward and starts empty; the archive is what
   fills it, and what a site with no internet has in place of it.
4. The per-site registry sync and the boot-time self-update describe jobs that
   the pull converger takes over. They are not revived as separate work.

## Every mechanism, with its status and verdict

### Used by the timed stand-up

| Mechanism | What it is for | Status | Verdict |
|---|---|---|---|
| Required-input scan in phase 1 (`goatbuilder doctor --site`, the phase-1 gate) | Finds every missing input and secret in one pass, before a host is touched | built; measured in the bench runs: phases 1 to 10 completed unattended with no stop | adopt (in use) |
| One SSH connection reused for every Ansible task | Removes a connection set-up per task | built | adopt (in use) |
| Phase 12 in waves (`needs:` on every step, `--parallel`) | Runs independent steps together | built; the saving is modelled, not measured | adopt; record on the next timed run whether `--parallel` was set |
| Image pre-pull (`ops/ansible/standup-image-pull.yml`, `ops/scripts/standup-image-pull.sh`, `standup-images.py`) | Pulls every pinned image on the hosts, four at a time, between phases 2 and 5 | built | adopt (in use); it is also the natural consumer of a site bundle |
| Site cache (`apt-cacher-ng`, `docker-registry-mirror` on the obs host; `site-cache-client` and `site_cache_host` on a core host) | A site-local source for packages and Docker Hub images | built; not measured on hardware. It serves Docker Hub and apt only: Docker has no mirror setting for `ghcr.io`, so the goat images still come from the internet on every host | combine with the site bundle (below) |
| Node seed (`ops/scripts/node-seed.py`, `ops/goatbuilder/node-classes.yaml`) | An unattended installer seed per machine class, so the baseline play starts from a host that is most of the way there | built as a script; a seeded reinstall of one bench board took 10 min 10 s (measured). It is not a stand-up phase | adopt; the operating-system install is outside row C1's clock, so this shortens preparation, not the timed hour |
| Release files verified before install (`release-file` role, keyless attestation) | A program is installed only from a signed release whose signature the tool verifies | built; measured for the first components | adopt; its verifier is the one a site bundle should use |
| Signed intent and digest pins (`goatbuilder pins`, the phase-1 checks) | States exactly which image every service runs | built | adopt; the pin table is the bundle's manifest |
| Teardown (`goatbuilder teardown`) | Takes a site down | measured: 5 min 54 s against a 15-minute target | adopt (target met) |

### Written or designed earlier, and not used by the stand-up

| Mechanism | What it was for | Status today | What does the same job now | Verdict |
|---|---|---|---|---|
| Bootstrap bundle ([bootstrap-bundle.md](bootstrap-bundle.md); `ops/bootstrap/rebuild.sh`) | One encrypted, signed archive of the repository at a tag, the image tarballs and the tools, held in the cloud, for a new site or a restart after total loss | written: the builder script exists; no bundle has been published and no stand-up step reads one | Nothing. The pre-pull and the cache both assume the internet | **combine**: its content list is the site bundle's. Its storage and signing are replaced, as ADR 1277 decides |
| Container-set archive (`ops/bootstrap/publish-containers.sh`) | Resolve one goatnet's pinned images once, save them as one signed archive, put it on the mirror | written: the producer exists; nothing loads the archive | Nothing | **combine**: this is the site bundle's image half, already keyed on the pins |
| Closed-network image bundle (`ops/scripts/goat-image-bundle-build.sh`, `goat-image-import.sh`) | Carry a signed set of images to a site with no internet and load them into its registry | written: both halves exist; no stand-up step calls them | Nothing | **combine**: this is the site bundle's carried-media path. It signs with a held key; the release path signs keylessly. ADR 1277 keeps one scheme, the release path's |
| Three-tier image fetch (`ops/bootstrap/pull-image.sh`, `publish-image.sh`) | Fetch a pre-baked operating-system image from an edge store, then a primary store, then a mirror, verifying its signature | written: producer and fetcher exist; the stores hold nothing the stand-up reads | The node seed, for the install; nothing for the fetch | **combine**: keep the fallback order (site, then another site, then object storage) as the bundle's fetch order. The pre-baked image itself is superseded (next row) |
| Pre-baked operating-system images (`ops/bootstrap/build-image-*.sh`, [image-builder-host.md](../operations/image-builder-host.md)) | A disk image with the baseline already applied | written for two machine families; not used | The node seed. [goatbuilder-speed.md](goatbuilder-speed.md) rules out a cloned disk: a cloned encrypted volume gives every host the same volume key | **superseded** by the node seed for machines with per-host disk encryption |
| First-boot script and USB user-data (`ops/bootstrap/first-boot/`) | A host configures itself at first boot from a file on removable media | written, with a container test; not used | `goatbuilder standup` and `join-node` driving the host over SSH; the node seed for the installer | **superseded** for sites an operator can reach. Its one distinct property, a host that starts with no operator connection, belongs to the pull converger |
| Rapid-bringup drill ([rapid-bringup-drill-runbook.md](../operations/rapid-bringup-drill-runbook.md), `ops/bootstrap/drill/`) | A timed drill of the first-generation path | written | The timed run of row C1 ([goatbuilder-hardware-measurement.md](goatbuilder-hardware-measurement.md)) | **superseded** by the C1 measurement. Its per-step log format is worth keeping as an input to that record |
| Per-site image registry kept in sync from the cloud (`oci-substrate-client`, `oci-substrate-t2-sync`, `ops/ansible/oci-substrate-deploy.yml`) | A registry at each site, filled on a timer from a cloud store, so image pulls stay on the site | written and switched off everywhere (`oci_t2_sync_enabled: false`); no credential, no first sync | The site cache for pull-through; the pull converger for "what should this site hold" | **superseded**: the cache is the site-local source, and the pull converger's signed index says what it must contain. A second registry and a second timer are not added |
| Boot-time self-update of a host ([self-healing-substrate-sync.md](self-healing-substrate-sync.md)) | A host that was powered off for weeks applies the current baseline by itself when it returns | designed; no code | The pull converger (designed, first after 1.0): an agent on each machine fetches its signed index on a timer and applies its class's release | **superseded**, with two requirements carried over (below) |
| Offline enrollment bundles ([offline-enrollment.md](offline-enrollment.md), `ops/enrollment/`) | Enroll a machine with no path to the management service | built (`ops/enrollment/bundle`, with its runbook) | Itself | **adopt**. It carries identity material, so it is never part of a site bundle |
| Signed downloads from object storage ([goat-client-distribution.md](goat-client-distribution.md), [goatbuilder-distribution.md](goatbuilder-distribution.md)) | People download the client and the tool from a public store, verified by signature | built for the client and the tool | Itself | **adopt** (in use). It is the precedent for holding a stateless signed artifact in a public store |

Three documents that the files above cite are not in this repository:
`rapid-site-bringup.md`, `oci-goat-substrate.md` with its decision record 0838,
and `docs/project/phase3-logistics.md`. Where a row above depends on them, the
status given is what the code in this repository shows.

## The three questions this reconciliation was asked

### Is the bootstrap bundle the missing pre-fill for the site cache?

**Yes for the idea, with two corrections.**

The cache starts empty: [goatbuilder-speed.md](goatbuilder-speed.md) says "the
first standup of a site fills it". A first site therefore pulls everything from
the internet, and a site with no internet cannot stand up at all. An archive
that arrives with the operator removes both limits.

Correction 1: filling the cache is not enough. The registry mirror serves
Docker Hub only, and the goat images are on `ghcr.io`. The bundle's images are
therefore loaded onto each host by digest (the pre-pull step reading the bundle
in place of a registry), and the cache is filled from the same bundle for apt
and for the machines that come later.

Correction 2: the artifact is not the April bootstrap bundle as written. That
design encrypts the bundle to a roster of people and signs it with a held key.
The bundle holds nothing secret, so encryption adds a key ceremony and no
protection, and the tree now has three signature schemes for artifacts (a held
SSH key for images and archives, a held cosign key for the closed-network
bundle, keyless attestation for release files). ADR 1277 keeps one: the release
path produces the bundle, and the `release-file` verifier checks it.

What the combined artifact gives:

| Need | How the site bundle meets it |
|---|---|
| A faster first stand-up | Images load from local media or one large download in place of dozens of registry pulls per host. The saving is unmeasured: no timed run has recorded image-pull time on its own |
| Gate group D, "Zero-SaaS standup, measured" (in, as part of C1) | A stand-up from a carried bundle reaches no registry. The relay tier and the first sign-in still need what they need; the bundle removes the image and package dependency only |
| A stand-up at a site that is cut off | The same bundle on carried media, or fetched from another site of the same goatnet |
| A join | The joining site reads the bundle from the founder's cache |

### Are the per-site git mirror and the boot-time self-update superseded by the pull converger?

**Yes, with two requirements carried over, and one clarification.**

The clarification: a git host at each site already exists for a different job.
`goatbuilder standup` places one on the obs host as the home of the
publish-subscribe state repository (`crates/goatbuilder/src/standup.rs`, the
obs leg's utility step). That is in use and stays. What is superseded is using
a per-site registry or mirror, on its own timer, as the way a site learns what
to run.

The pull converger covers the self-update's problem: an agent on each machine
fetches a signed index for its environment and applies its class's release.
Two things the self-update design states, and the pull converger's design must
keep:

1. **It runs at boot, before the services that depend on it.** A host that
   returns after weeks must converge before it rejoins, not at the next timer
   tick.
2. **It converges the host baseline, not only the containers.** The cases
   that motivated the design were host-level: the clock service order and the
   container engine's data directory. The agent's unit of convergence therefore
   includes the class's host plays.

### Is the bundle's total-loss restart the same problem as a rebuild from an extract?

**The same run, two artifacts that must stay separate.**

[ADR 1237](../adr/1237-a-site-is-rebuilt-from-nothing-and-a-combed-extract-of-its-identities.md)
rebuilds a site "from nothing and a combed extract of its identities". Its
"nothing" still needs the code, the pins and the images, and that is the site
bundle. The total-loss scenario of the April design is the same rebuild with no
extract.

They do not share one artifact. The bundle is stateless and may be held
anywhere. The extract names people, machines and services, so it stays on
sovereign hardware and is never placed in a cloud store or inside a bundle. A
rebuild takes both: the bundle supplies what to run, the extract supplies who
belongs.

## What to build, in order, sized against the three timings

Row C1 has three targets: a first site in 60 minutes, a machine added in 15, a
site taken down in 15. Measured so far on a bench of real machines: taking a
site down, 5 min 54 s; phases 1 to 10 of a first site, 26 min 39 s unattended;
first sign-in done 29 min 12 s into the clock; the relay tier in the cloud,
about 10 min 30 s of that; phases 12 to 14 without a stop, unmeasured.

| Order | Item | Timing it changes | Minutes | Where it sits |
|---|---|---|---|---|
| 1 | Record per-lever times in the timed run: image pulls, package installs, the relay tier, each wave of phase 12 | all three | none removed; every figure below is unmeasured until this exists | inside C1 as written (the run "kept as a measurement, with the per-phase times the binary prints") |
| 2 | Declare the site cache on every site that has an obs host, and prove it on hardware | machine added | unmeasured; it removes the Docker Hub and apt downloads from every machine after the first | inside C1 ("the site cache on hardware" is already listed under "What remains" in the speed design) |
| 3 | The site bundle, produced by the release path from the pin table and verified by the `release-file` verifier; the pre-pull step loads from it | first site; machine added at a site with no internet | unmeasured; bounded above by the image-pull share of phases 2 to 5 | the "Zero-SaaS standup, measured" row of group D, which is in as part of C1. Building the bundle is one way to pass that row, not a new row |
| 4 | Start the relay tier beside the host baseline, if the timed record shows they run one after the other | first site | up to about 10 min; unmeasured until item 1 shows whether they overlap today | inside C1 as written |
| 5 | The node seed as a stand-up phase, with its timed comparison | preparation before the clock | 10 min 10 s per reinstall measured; the baseline saving of 3 to 5 min per host is modelled | the seed's own gate in the speed design; outside C1's clock |
| 6 | The pull converger, with the two requirements above | none of the three; it removes the operator's machine from later updates | not applicable | first after 1.0, as decided |
| 7 | A rebuild that takes the site bundle and an extract | none of the three | not applicable | after 1.0 (ADR 1237) |

Items 1 to 4 are inside gate rows as they are written today. This document
adds no row and removes none. The order is tracked in
[issue 1489](https://github.com/USAREUR-AF/goatcore/issues/1489).

## What this document changes elsewhere

- [bootstrap-bundle.md](bootstrap-bundle.md), [self-healing-substrate-sync.md](self-healing-substrate-sync.md)
  and `ops/bootstrap/README.md` gain a first line that points here.
- [goatbuilder-speed.md](goatbuilder-speed.md), "What remains", gains the site
  bundle and the per-lever record.
