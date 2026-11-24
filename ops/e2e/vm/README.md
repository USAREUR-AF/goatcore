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
  (`GROUP_PINS`); the public pins are not changed.
  Its `type_tree` function, where it has one, writes more of the synthetic
  goatnet's files into the clone before it is committed.

**What a pass is.** `result.json` lists every step and every phase that was not
run, with the reason. One machine alone has no founder to enrol with, so in `run.sh` join-node's
phases 3, 7 to 12, 14 and 15 are not run: a pass is the host baseline, the kernel
modules, the type's own phase 13 and `doctor --target`, and nothing else.

| Type | What its VM proves |
|---|---|
| network-tap | phases 1, 2, 4, 5, 6 and 13 (the declared skip); `doctor --target --live` refuses the unprepared machine, and passes every row but the encrypted root once the machine is prepared by hand as section 3.8 says |
| breakglass | phases 1, 2, 4, 5, 6 and 13 (its play) |
| federation-boundary | phases 1, 2, 4, 5, 6 and 13 (the declared skip: `goatbuilder federate` makes each peering from an agreement, so nothing is installed); the machine has no WireGuard interface. A peering over it is not run here |
| octo-pod | phases 1, 2, 4, 5, 6 and 13: octo-pod's published release, verified against the pin in `types/octo-pod.sh` and installed by `octo-pod-deploy.yml`. On the image as published, octo-pod's `apply` fails (0.6.0 names its Zenoh image by a short name) and the play leaves no file current; given one container search registry, phase 13 ends in octo-pod's own `verify`, and a second run keeps the pod's identity |
| simple-boundary | phases 1, 2, 4, 5, 6 and 13 (`simple-bn.yml`, stage 0, notional input, a stand-in `wt0`); the sender and monitor are active, the monitor answers on the mesh address only, a receiver on the machine gets each feed's CAT034 and CAT048 from its declared source port, and a second phase 13 changes nothing |

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

`founder.sh` runs the phases a lone machine cannot: a goatlite founder to standup
phase 10, then one cloud-image machine through join-node, on a private segment of
VMs. **It stops before standup phase 11, a person's step. That is the declared stop.**

```bash
ops/vm-image/build.sh --out /path/image
ops/vm-image/zero-saas/lab.sh prepare-laptop --work /path/lab
ops/e2e/vm/founder.sh --work /path/lab --image /path/image/goatlite-ubuntu-24.04-amd64.qcow2 --record /path/record
```

| | |
|---|---|
| Run | standup phases 1 to 6, 9 and 10 (a LAN site has no 7 or 8); the founder's own enrolment, with standup phase 8's play; join-node phases 1 to 12 |
| Lab stand-in, in the record | the group the founder's peer holds, which standup phase 12 makes: one group, no policy |
| Not run | standup 11 to 15 (the declared stop); join-node 13 (needs the administrator of phase 11), 14 (the mesh carries nothing before phase 12's ACL) and 15 (its rows read a whole site) |

It declares, for the site, where its machines enrol, the client they enrol with and
the router's first run on a new site (`zenoh_slot_fragment_bootstrap`).
