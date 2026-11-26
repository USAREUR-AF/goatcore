# cable-install: Ubuntu 24.04 on a board with no usable console

The builder for the `zimaboard-2` platform of [`hardware-classes.yaml`](../hardware-classes.yaml).
It installs the Ubuntu 24.04 baseline over one Ethernet cable to a helper machine. No USB stick,
KVM, monitor or keyboard.

| You have | Run on the helper | Result | Status |
|---|---|---|---|
| A board already on a baseline this builder installed | `./cable-install.sh <hostname>` | wiped; plain LVM root (6 to 11 minutes) | run on a ZimaBoard 2 (16 GB) |
| Same, encrypted | `CABLE_MODE=luks ./cable-install.sh <hostname>` | wiped; LUKS2 root unlocked by the TPM at boot (about 14 minutes) | run on a ZimaBoard 2 (16 GB); unattended unlock held across first boot and a reboot |
| A factory board on ZimaOS 1.5.0 | `HOSTNAME_NEW=<hostname> ./from-zimaos.sh <board-ip>` | plain LVM root | WIP: its commands worked by hand; not yet run as one script |

Why this exists: a ZimaBoard 2's video is mini-DisplayPort only, so an HDMI KVM captures nothing,
and its firmware boots the internal disk before USB. Every install is blind. The builder
therefore never needs the console: the board's own bootloader starts the stock Ubuntu installer,
which pulls the ISO into RAM (4.1 GB) and the unattended seed from the helper.

The result is the hand-off state [`bare-metal-install.md`](../../../docs/operations/bare-metal-install.md)
section 4 checks: the hostname you chose, key-only SSH, NOPASSWD sudo, and in `luks` mode
`mmcblk0p3 crypto_LUKS → dm_crypt → ubuntu-vg`. goatbuilder starts from there.

## Set up the helper once

The helper is any Linux machine with a spare Ethernet port, systemd-networkd and internet.

1. Cable the spare port straight to the board (or through a switch used for nothing else).
2. `helper/05-cable-install-link.network` to `/etc/systemd/network/`, with the port name, the
   board's MAC address and a DNS server filled in; `networkctl reload`. The helper becomes
   10.77.0.1/24 on that port, runs a DHCP server, gives the board 10.77.0.5 and NAT to the
   internet.
3. If the helper runs Docker, its FORWARD policy is DROP: install and enable
   `helper/cable-install-forward.service` (fill in the port name).
4. Copy this directory to `~/cable-install` on the helper. In it:
   - `www/ubuntu.iso`: the stock `ubuntu-24.04.x-live-server-amd64.iso`, checked against
     Ubuntu's `SHA256SUMS`;
   - `kb/vmlinuz` and `kb/initrd`, extracted from that ISO:
     `xorriso -osirrox on -indev www/ubuntu.iso -extract /casper/vmlinuz kb/vmlinuz -extract /casper/initrd kb/initrd`;
   - `authorized_keys`: one SSH public key per line, for every person and machine that must
     reach the board;
   - `console.hash`: a SHA-512 crypt hash of the console password (`mkpasswd -m sha-512`);
     SSH stays key-only;
   - for `luks` mode, `luks.pass` (mode 600): the disk passphrase. Its home is your secret
     store; this copy exists so the seed can carry it for the minutes an install runs.
5. `helper/cable-install-http.service` to `~/.config/systemd/user/`, then
   `systemctl --user enable --now cable-install-http.service`. It serves `www/` on
   10.77.0.1:8099, on the cable only.

Nothing in this directory holds an address, key or name of yours: those live in the files of
step 4, on the helper.

## What makes it hold

- **A way into the installer.** The seed puts your keys on the installer session's root. While
  an install runs, `ssh root@10.77.0.5` works, and
  `/var/log/installer/subiquity-server-info.log` shows the state: `RUNNING`, `UU_RUNNING` while
  it applies package updates, `LATE_COMMANDS`, `DONE` or `ERROR`. Without it a slow install and
  a failed one look the same from outside, and the disk is already erased. **Never power-cycle
  a board that looks stuck: log in and look.** An interrupted install leaves a board with
  nothing to boot.
- **One-shot boot entries.** From a baseline the installer entry is selected with
  `grub-reboot`; from ZimaOS by a flag in `grubenv` that GRUB clears before booting. A boot that
  fails before the installer touches the disk returns to the previous system on the next power
  cycle.
- **Only the cabled port answers DHCP in the installer.** The kernel line carries
  `modprobe.blacklist=cdc_ncm,cdc_ether,rndis_host`: a USB network device on the board (a KVM's
  virtual network card does this) otherwise wins the installer's `ip=dhcp`, and the installer
  then has no route to the helper.
- **Short DHCP leases.** Each phase of an install asks for the board's static address under a
  new client id, and systemd-networkd refuses it until the previous lease has expired. Leases
  are 120 s.
- **`luks` mode binds the TPM during the install.** `late.sh.tmpl` runs in the new system before
  its first boot: `clevis luks bind` to TPM 2.0 (sha256, PCR 7) on the installer's LUKS
  partition, with `clevis-initramfs`. That is the bind the `luks-unlock` role makes for
  `unlock_class: tpm2`; it is made here because a board with no console cannot be given its
  passphrase at first boot. `dropbear-initramfs` with your keys is the remote way in if the TPM
  does not release the key (`ssh root@<board>`, then `cryptroot-unlock`).

## ZimaBoard 2 facts

| | |
|---|---|
| Hardware | Intel N150, 4 cores, 8 or 16 GB, 64 GB eMMC (57.6 GB usable), two RJ45 ports (`enp1s0`, `enp2s0`), TPM 2.0 |
| Disk | The eMMC only, unless a disk is added on the PCIe slot. The installer takes the smallest disk, so it lands on the eMMC |
| Firmware | Boots the eMMC before USB. Secure Boot was off on the board this ran on |
| As delivered | ZimaOS 1.5.0. `PUT /v2/zimaos/ssh?status=enable` with a login token turns on SSH. Its GRUB reads an lzo squashfs on the unused second system slot (`mmcblk0p5`), which is where `from-zimaos.sh` stages the installer kernel |
| RAM | The installer holds the 4.1 GB ISO in RAM. Run on a 16 GB board; the 8 GB board has not been tried |

## When a board has nothing to boot

An erased disk with no installer running cannot be reached over the cable. It then needs an
installer on USB or a KVM's virtual CD, with the boot menu keys (F7 or F11) sent blind, since
the screen cannot be seen. Build that installer from the same seed: remaster the ISO with
`autoinstall ds=nocloud\;s=/cdrom/nocloud/` on the kernel line and `www/nocloud/` copied to
`/nocloud`.
