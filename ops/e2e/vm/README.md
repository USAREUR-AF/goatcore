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
run, with the reason. A VM here has no founder to enrol with, so join-node's
phases 3, 7 to 12, 14 and 15 are not run: a pass is the host baseline, the kernel
modules, the type's own phase 13 and `doctor --target`, and nothing else.

| Type | What its VM proves |
|---|---|
| network-tap | phases 1, 2, 4, 5, 6 and 13 (the declared skip); `doctor --target --live` refuses the unprepared machine, and passes every row but the encrypted root once the machine is prepared by hand as section 3.8 says |
| breakglass | phases 1, 2, 4, 5, 6 and 13 (its play) |
| federation-boundary | phases 1, 2, 4, 5, 6 and 13 (the declared skip: `goatbuilder federate` makes each peering from an agreement, so nothing is installed); the machine has no WireGuard interface. A peering over it is not run here |
| octo-pod | phases 1, 2, 4, 5, 6 and 13: octo-pod's published release, verified against the pin in `types/octo-pod.sh` and installed by `octo-pod-deploy.yml`. On the image as published, octo-pod's `apply` fails (0.6.0 names its Zenoh image by a short name) and the play leaves no file current; given one container search registry, phase 13 ends in octo-pod's own `verify`, and a second run keeps the pod's identity |
| simple-boundary | phases 1, 2, 4, 5, 6 and 13 (`simple-bn.yml`, stage 0, notional input, a stand-in `wt0`); the sender and monitor are active, the monitor answers on the mesh address only, a receiver on the machine gets each feed's CAT034 and CAT048 from its declared source port, and a second phase 13 changes nothing |

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
