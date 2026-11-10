# goatlite prepared VM image

Status: WIP, not for production use — gate: one standup from the image, timed with the M-126 method.

An Ubuntu 24.04 image with phase 2 of a goatlite standup already applied and every image a goatlite runs already pulled ([goatlite.md](../../docs/design/goatlite.md) §11, measure 5). Started instead of a stock Ubuntu image, phase 2 finds Docker installed and configured, and the standup's image pull finds every image on the box and pulls none.

## What is in it

| Part | Source |
|---|---|
| Ubuntu 24.04 server cloud image, amd64 | `cloud-images.ubuntu.com/releases/24.04/release`, checked against its `SHA256SUMS` |
| phase 2's host baseline | `ops/ansible/mgmt-host-baseline.yml` with the values a goatlite with `disk_encryption: false` declares: Docker on `/var/lib/docker`, containerd on `/var/lib/containerd` |
| every image a goatlite runs, by digest | `ops/scripts/standup-images.py --preset lite`, pulled by `ops/scripts/standup-image-pull.sh` (the standup's own pull step) |
| what it was built from | `/etc/goat/prepared-image.json` on the image, and `goatlite-ubuntu-24.04-amd64.json` beside it: source commit, Ubuntu checksum and serial, sha256 of the baseline playbook and of `fleet-pins.yaml`, the image list |

Nothing in it belongs to one machine: cloud-init state, machine-id, SSH host keys and the build user are removed, so the first boot is a new machine to cloud-init, and Docker makes a new engine id.

## How it is built

`.github/workflows/goatlite-vm-image.yml` runs `ops/vm-image/build.sh` on a GitHub-hosted Ubuntu runner under QEMU (KVM when the runner has it). It runs on every push to main that changes the baseline playbook, the roles it runs, `group_vars/all.yml`, the pin table, the image list or the builder, and on demand (`gh workflow run goatlite-vm-image.yml`). After the build, the workflow boots the written image as a learner would, with a new cloud-init user, and fails unless that user can SSH in, Docker runs on `/var/lib/docker`, the build user is gone, and the standup's pull step reports every image already on the host by digest.

The image is the run's artifact `goatlite-ubuntu-24.04-amd64` (kept 14 days): the qcow2, its JSON record, `SHA256SUMS`, `build.log` and the VM's `console.log`.

First CI build (PR #2501, run 42043405346, KVM on a 4-vCPU runner): 528 s end to end, a 2.6 GB qcow2, 11 images. The pull step took 69 s on the build VM, where nothing was present, and 2 s on the first boot, where all 11 were present by digest and none was pulled.

## How to use it

1. Download the newest artifact from a main run and check it:

   ```bash
   gh run download --repo USAREUR-AF/goatcore --name goatlite-ubuntu-24.04-amd64 \
     "$(gh run list --repo USAREUR-AF/goatcore --workflow goatlite-vm-image.yml --branch main --status success --limit 1 --json databaseId --jq '.[0].databaseId')"
   sha256sum -c SHA256SUMS
   ```

2. Start a VM from it the way you would start Ubuntu's own cloud image: 4 vCPU and 16 GiB (the image's disk is 40 GiB), and a cloud-init NoCloud seed naming your user and SSH key. On KVM, libvirt or Proxmox the qcow2 is used as is. Other hypervisors take a converted copy:

   | Hypervisor | Convert |
   |---|---|
   | VMware | `qemu-img convert -O vmdk goatlite-ubuntu-24.04-amd64.qcow2 goatlite.vmdk` |
   | Hyper-V | `qemu-img convert -O vhdx goatlite-ubuntu-24.04-amd64.qcow2 goatlite.vhdx` |
   | a disk on your own hardware | `qemu-img convert -O raw goatlite-ubuntu-24.04-amd64.qcow2 goatlite.raw`, then write it to the disk |

   A seed for the first boot, made on any Linux machine with `cloud-image-utils`:

   ```bash
   printf '#cloud-config\nusers:\n  - name: ubuntu\n    sudo: "ALL=(ALL) NOPASSWD:ALL"\n    ssh_authorized_keys:\n      - %s\n' "$(cat ~/.ssh/id_ed25519.pub)" > user-data
   printf 'instance-id: goatlite-1\nlocal-hostname: goatlite\n' > meta-data
   cloud-localds seed.iso user-data meta-data
   ```

3. Write `goatlite.yaml` as [tutorial 00](../../docs/tutorials/00-goatlite.md) says, with `disk_encryption: false`. The image's Docker is on the system disk; with `disk_encryption: true` phase 2 moves Docker onto the encrypted data disk and the standup pulls every image again.

4. Run `goatbuilder standup <name>` as the tutorial says. Phase 2 runs and finds Docker in place; the image pull before phase 5 prints `N of N image(s) present … (N already on the host by digest, not pulled)`.

An image built before a pin changed still works: a pin the image does not hold is pulled by the same step, and only that one. `/etc/goat/prepared-image.json` names the pins the image was built from.

## Not built

| Item | Why |
|---|---|
| a timed standup from the image (the gate above) | M-126 timed a standup on an AWS `m6i.xlarge`; the same shape from this image needs the image imported as an AMI, which needs a cloud account the CI does not hold. The timing is read from `~/.goat/state/standup-<name>-timings.tsv`, which both standup drivers write, and set against M-126's table in goatlite.md §11 |
| cloud images (AMI, Azure, GCE) | each needs an account in that cloud to import into |
| arm64 | the M-126 shape and the runner are amd64; an arm64 build needs an arm64 runner |
