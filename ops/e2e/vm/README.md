# A target type's machine, in a VM

`run.sh <type> --goatbuilder BIN` takes one machine of a target type from a
published Ubuntu cloud image to `doctor --target` (gate J1). It runs the real
goatbuilder and the real plays against a fresh clone whose only goatnet is
synthetic. It needs no root, and changes nothing outside `--work` and its image
cache (`VM_IMAGES`, default `~/.cache/goat-vm-images`).

- The image is held to the release's `SHA256SUMS`, and that file to Ubuntu's
  signature where `gpgv` and the `ubuntu-cloudimage-keyring` package are
  installed. The record says which.
- KVM when the VM's architecture is this machine's and `/dev/kvm` is usable, else
  emulation. A machine with no emulator for the architecture runs it from the
  container in `Dockerfile`. The record's `accel` says which.
- `types/<type>.sh` says what the machine needs and what its checks must read. It
  may give the synthetic goatnet release components (`COMPONENTS`, rows of
  `fleet-pins.yaml`) and the pins rendered for the machine's group
  (`GROUP_PINS`), or keep rows of the checkout's own pins (`PUBLIC`, each rendered
  at its fleet pin) and list opt-in components (`OPT_IN`); the public pins are not
  changed.
  Its `type_tree` function, where it has one, writes more of the synthetic
  goatnet's files into the clone before it is committed.

**What a pass is.** `result.json` lists every step and every phase that was not
run, with the reason. One machine alone has no founder to enrol with, so in `run.sh` join-node's
phases 3, 7 to 12, 14 and 15 are not run: a pass is the host baseline, the kernel
modules, the type's own phase 13 and `doctor --target`, and nothing else.

| Type | What its VM proves |
|---|---|
| network-tap | a tap's mode refused before any phase when it is missing or puts management on a tap port (GB-380); phases 1, 2, 4, 5, 6 and 13: goatrope's published release, verified against the pin in `types/network-tap.sh`, its play run from the verified archive, inline. `goatrope check` matches and `doctor --target --live` passes every row but the encrypted root; a second phase 13 leaves the machine alone; declared listening, phase 13 installs it again, the same checks pass, and in 30 seconds neither port sends a frame. The run installs tcpdump, ethtool and the watchdog's module first, which goatrope needs and installs none of. Not run: a feed across the link (each link has no far end) |
| breakglass | on the small-computer family, with a second wired interface as its out-of-band link (an address and a default route of its own above the primary's): phases 1, 2, 4, 5, 6 and 13 (its play, the clock step armed as at a site that blocks NTP); acceptance rows `breakglass-clock-recovery-armed` and `breakglass-oob-is-never-the-default-route` pass, and the route row fails once a default route below the primary's is added by hand; `breakglass-oob-binds-no-service` passes, and fails once a service listens on the out-of-band address; `doctor --target --live`'s small-computer rows fail, as a cloud image is not a built Pi |
| federation-boundary | phases 1, 2, 4, 5, 6 and 13 (the declared skip: `goatbuilder federate` makes each peering from an agreement, so nothing is installed); the machine has no WireGuard interface. A peering over it is not run here |
| octo-pod | phases 1, 2, 4, 5, 6 and 13: octo-pod's published release, verified against the pin in `types/octo-pod.sh` and installed by `octo-pod-deploy.yml`. On the image as published, octo-pod's `apply` fails (0.6.0 names its Zenoh image by a short name) and the play leaves no file current; given one container search registry, phase 13 ends in octo-pod's own `verify`, and a second run keeps the pod's identity; `doctor --target --live` then fails its automatic-updates row |
| simple-boundary | phases 1, 2, 4, 5, 6 and 13 (`simple-bn.yml`, stage 0, notional input, a stand-in `wt0`); the sender and monitor are active, the monitor answers on the mesh address only, a receiver on the machine gets each feed's CAT034 and CAT048 from its declared source port, and a second phase 13 changes nothing |
| first-bite | phases 1, 2, 4, 5, 6 and 13: first-bite's published release, each archive verified against its own attestation, installed by `first-bite-deploy.yml` with the archive's own installer, capturing on one extra interface; a heartbeat of the release's weather-station dictionary sent on that link is decoded; an undeclared interface, a dictionary the release lacks and settings on another interface are each refused, and a second run passes |
| example-overlay | a synthetic overlay's type (gate J26): its file in `ops/goatbuilder/overlay/targets/`, its play in a synthetic repository at a full commit. Phases 1, 2, 4, 5, 6 and 13; phase 13 runs the play from the fetched copy and the run reads back the file it wrote on the machine; a kept copy changed afterwards is refused before its play (GB-420). Its type file and repository are written into the clone by `type_tree` |
| goat-processors | phases 1, 2, 4, 5, 6 and 13: the runner's public opt-in row, which the synthetic goatnet lists, installs its published release, verified against its Sigstore bundle; its fabric router is a container on this machine. Then the conformance vectors the package carries are compared with the repository's at the release's commit, and the runner's `selftest` must reproduce every one; a second phase 13 passes |

## A prepared image

`prepare-image.sh --arch arm64` boots the published image, upgrades its packages
and writes a prepared image beside it in the cache, with a record: the base
image's digest, the prepared image's own, every package and version, and how long
it took. `run.sh --prepared` boots it only when the published image is still the
one it was made from and its digest is the recorded one. `result.json` says so
(`prepared_image`, `disk_sha256`). The baseline still runs whole and finds nothing
to upgrade, or only what was published since. An arm64 runner of GitHub's has no
`/dev/kvm` (read on 24 Nov 2026), so arm64 is emulated everywhere. Measured on a 16-core machine with other work on it: making the image took 45 minutes for the upgrade; from it, the baseline takes 15 minutes, where from the published image it had not finished after an hour.

## A founder

`founder.sh` runs the phases a lone machine cannot: a goatlite founder, then one
cloud-image machine through join-node, on a private segment of VMs.

```bash
ops/vm-image/build.sh --out /path/image
ops/vm-image/zero-saas/lab.sh prepare-laptop --work /path/lab
ops/e2e/vm/founder.sh --work /path/lab --image /path/image/goatlite-ubuntu-24.04-amd64.qcow2 --record /path/record \
  [--phase-11-stand-in [--joiner-site beta]] [--keep]
```

Three shapes, each named in the record (`declared_stop`, `node_site`, `stand_ins`,
`declarations`, `not_run`):

| | Run | Lab stand-ins | Not run |
|---|---|---|---|
| no flag | standup 1 to 6, 9 and 10 (a LAN site has no 7 or 8); the founder's own enrolment, with standup phase 8's play; join-node 1 to 12 for a second core machine at the founder's site. **It stops before standup phase 11, a person's step: the declared stop** | the group the founder's peer holds, which phase 12 makes: one group, no policy, written to the database | standup 11 to 15; join-node 13 to 15 |
| `--phase-11-stand-in` | the whole standup; the founder's enrolment; join-node 1 to 15 for a second core machine at the founder's site, then `doctor --target core` | the person at phase 11 (`ops/vm-image/zero-saas/laptop-phase11.sh`, lab only: a browser signs in with the one-time password, phase 10 runs once more, the service user and its token are made in the dashboard); the founder's peer put in the site's `<SITE>-INFRA` group through the API, since no phase gives it a group and join-node phase 10 mints the node's key with that peer's groups | what the run did not reach |
| `--phase-11-stand-in --joiner-site beta` | as above, for the first core machine of a new site `beta` that joins the founder's goatnet | as above | what the run did not reach |

With `--joiner-site` it declares the site as a person would for a goatlite's goatnet,
which a goatlite does not make: `goatbuilder.yaml` naming both sites; the node's line
with its group under `[mgmt_hosts:children]` and no mesh line before phase 12 writes
it; the goatnet's mesh DNS zone (two services in the founder's `isitup_services`,
then `ops/dns/generate-mesh-hosts.py --init`); the site's group_vars, copied from the
founder's with the joining site's own facts (join-node reads a site's facts from the
top level of its `vars.yml`), signing in at the founder's public `oauth2.` name with
the goatnet's CA in `netbird_mgmt_trust_certs`; and its certificate manifest under the
goatnet's CA. For the founder it declares where machines enrol and the client they
enrol with. For both, it declares the replication that join-node phase 13 wires, as a goatlite's
growth declares it ([goatlite-growth.md](../../../docs/design/goatlite-growth.md#5-the-database-one-site-to-two)):
`participates_in_spock`, an explicit topology naming the other site, and `ory_spock_enabled`. `--keep` leaves the VMs up after the run, to read or resume by hand.

Measured on KVM here, 26 Nov 2026, from a checkout that was main of that afternoon
plus this harness, the phase 11 stand-in's fixes, the client-hold retry, and a
first-render check for phase 9's router in `zenoh-deploy.yml`, which main now has
in its reviewed form; this harness has not run on that form yet.

- founder, every shape: standup 1 to 10 pass with no hook (phase 6 accepts the pinned
  sign-in image; phase 9 renders the router before its portal); with the phase 11
  stand-in, phases 12 to 14 pass.
- `--joiner-site beta`, whole run: 16 of 17 steps; join-node 1 to 12 pass; **it stops
  in join-node phase 13**, at the check that the seed is current (GB-367): the joining
  site's Hydra makes its own signing key before the seed, so `hydra_jwk` differs.
- Found by hand in the kept VMs after that stop, not fixed: a forced re-seed makes the
  tables equal, and a resume from 13 then refuses the seed again (the account is
  populated); run step by step from the check on, phase 13 stops where the seed should
  reach the wire, at the founder, whose `kratos` and `hydra` databases hold no Spock
  node.
- At the founder's own site (`--phase-11-stand-in`, no `--joiner-site`), found by hand:
  join-node phase 13 needs `OWNER_EMAIL` (standup takes goatlite.yaml's admin), then
  the node's management service never starts: a goatlite names its in-compose Hydra
  as the OIDC endpoint, and Ory runs only on `router_role` A.
- Not reached: join-node 14 and 15, the core type's acceptance, `doctor --target core`.

Once the founder has stood up whole (the phase 11 stand-in), the laptop joins the goatnet's mesh as the founder's admin peer (`laptop-mesh-enrol.sh`: the pinned NetBird client from its verified release, the goatnet's root for that client alone, a single-use key with the goatnet's PLATFORM-ADMIN group). From phase 12 on, `join-node` reaches the node at its mesh address over that path, as its default transport assumes of an operator machine; the step and its declaration are in the record.

`goatlite-vm-image.yml` has a job for it, started by hand: three VMs need about 14 GB
of memory, and a hosted runner has 16.

Both avenues and the finish (ADR 1201). `--avenue scripts` stands the founder up with
`ops/scripts/goat-standup.sh` instead of `goatbuilder standup` (the phase-10 re-run of
the phase 11 stand-in follows it). `--acceptance` runs a goatlite's acceptance phase
(`--check`). `--assay` ends the run as tutorial 00 sections 6 and 7 end a goatlite: the
laptop trusts the goatnet's root, the assay's values and probe identity are written,
and `goatbuilder assay` must exit 0. `goatbuilder status` and the standup's phase
timings (`timings.tsv`) go into the record, which names the avenue.

## A peering over two federation-boundary machines

`fed-peering.sh --goat BIN --goatbuilder BIN` (gate J8) builds two
federation-boundary machines at once with `run.sh --keep`, one per synthetic
sovereign. Each gets a second interface, `link0` (`VM_LINK` in `vm.sh`), and the
wire between them is the only path from one to the other: 192.0.2.11 and
192.0.2.12, each machine's public address. It then runs
`ops/federation-e2e/live.sh` scenario 1 against them, to phase 14's strict
acceptance from both sides.

- **The router image.** It is private, and this run holds no token. The image
  is saved from this machine's docker, which must already hold it at live.sh's
  pinned digest, and loaded into each machine's docker. The loaded id must be
  that digest. The run then uses `live.sh --images-present`.
- **The binaries** are shipped to the machines, so they must run on Ubuntu
  24.04: the `federation-live-binaries` artifact of a `federation-e2e` run.
- **Not like a real goatnet.** live.sh puts each sovereign's core router and
  SPIRE server on its edge machine. Neither machine is enrolled in a mesh.
- **Where it runs.** It runs locally only. A CI job would need a read secret
  for the private image.
- **`--machines-only`** stops before live.sh: the two machines and the wire,
  which need no image.
