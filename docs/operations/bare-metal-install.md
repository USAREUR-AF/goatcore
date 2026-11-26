# Bare-metal install — from an unboxed machine to an SSH-reachable, LUKS-encrypted host

For a failed install, boot, unlock, console, or recovery-access symptom, start
with [`site-node-troubleshooting.md`](site-node-troubleshooting.md) and return
here when it selects the canonical bare-metal recovery path. For an existing
node already stopped in its boot or unlock transaction, use
[`site-node-boot-recovery.md`](site-node-boot-recovery.md).

This is the step **before phase 1** of a host-site standup: it turns a machine straight
out of the box into the state `goat-standup.sh` and the turnkey chain assume —
Ubuntu 24.04 Server, **full-disk LUKS**, the operator's key installed, key-only
SSH, and the box reachable on the LAN. Everything after this lands in
[goatnet-standup.md](goatnet-standup.md); everything before it is here.

It is written so the result is **repeatable**, not a hand install that happens to
work once. x86 mini-PCs autoinstall from the
answer file below; the Raspberry Pi path differs and is called out where it does.

## What you need

- The target machine, its power supply, and a wired Ethernet drop on the build LAN.
- A USB stick (8 GB+).
- The Ubuntu 24.04.x **Live Server** ISO — amd64 for an x86 host. The Pi 5 has two
  routes (see §Pi): the current Imager flash, or the arm64 Live Server ISO under
  EDK2 UEFI (the superseded COA C reference procedure). For COA C you also need
  the **arm64** ISO and EDK2 firmware for the Pi.
- Your operator SSH public key (`~/.ssh/id_ed25519.pub`).
- A LUKS passphrase for this box, stored in `.envrc.local.d/20-host-access.envrc`
  as `<SITE>_<ROLE>_LUKS_PASS` **before** you install — you type it at first boot
  until TPM2 auto-unlock is enrolled by the standup's LUKS role.

## 1. BIOS / firmware — three settings that matter

Enter firmware setup (commonly `Del` or `F2` at power-on) and set:

| Setting | Value | Why |
|---|---|---|
| Boot order | USB first (temporarily), then the NVMe | boot the installer |
| **TPM / PTT** | **Enabled** | the LUKS role enrolls TPM2 for unattended unlock; without it the box needs a console passphrase every reboot |
| **Restore on AC power loss** | **Power On** | a host site must come back by itself after a power cut (Design Principle: resilient) |
| Secure Boot | Off, unless you have signed everything | Ubuntu installs either way; off avoids a signing detour |

Save and exit.

## 2. Write the installer USB

- **macOS / Linux:** `balenaEtcher` (handles the raw write and the permission
  prompt), or `dd if=ubuntu-24.04-live-server-amd64.iso of=/dev/rdiskN bs=4m` on
  macOS after moving the ISO out of a TCC-protected folder (Downloads/Desktop) —
  raw `dd` to a disk needs Full Disk Access.
- Boot the target from the USB (one-time boot menu, usually `F7`/`F11`).

## 3. Autoinstall — the answer file does the LUKS install for you

Rather than click through the installer, drive it from
[`ops/bootstrap/autoinstall/user-data.yaml`](../../ops/bootstrap/autoinstall/user-data.yaml).
It sets: a DHCP install, the operator key, **`storage: layout: {name: lvm,
password: ...}`** which is subiquity's full-disk **LUKS-on-LVM** layout (the exact
`nvme0n1p3 crypto_LUKS → dm_crypt → ubuntu-vg` shape the LUKS node class expects),
key-only SSH, and no swap-on-disk. Fill the three placeholders first
(`<HOSTNAME>`, `<OPERATOR_SSH_PUBKEY>`, `<LUKS_PASSPHRASE>` — the last from your
env file, never committed).

Two ways to feed it:

- **Interactive-served:** put `user-data` and an empty `meta-data` on a second USB
  labelled `CIDATA`; the installer picks it up automatically.
- **Kernel arg:** at the GRUB installer prompt, add
  `autoinstall ds=nocloud;s=/cdrom/server/` pointing at wherever you served the
  files.

The install reboots into a machine that halts at the **LUKS passphrase prompt**
on the console — type the passphrase once. After it boots you have an
SSH-reachable host on the LAN, key-only, with `~1.8 TB` free in the encrypted VG.

## 4. Verify the hand-off state

From your laptop — resolve each box by its **hostname**, never a pinned address.
DHCP roams the IP, and matching by MAC is easy to get wrong. SSH to a
candidate and read `hostname`; the node is whichever answers to `<site>-core-a` /
`<site>-obs-b` / etc., wherever DHCP put it:

```
ssh <user>@<lan-ip> 'hostname; lsblk -o NAME,SIZE,FSTYPE,TYPE,MOUNTPOINT; ls /dev/tpm*; vgs'
```

Do this for **every** node before proceeding — a card keyed in the imager but
skipped on one unit comes up unreachable, and the standup only discovers it at
the first privileged task. The phase-1 host preflight
(`standup-host-preflight.py`) now asserts each node is encrypted and key+sudo
reachable, so a miss fails there rather than mid-play.

You want: the hostname you set, `nvme0n1p3` as `crypto_LUKS` over an `ubuntu-vg`
with free space, `/dev/tpm0` present, and key-auth working. That is the exact
state `seed-ssh-key.sh` → `mgmt-host-baseline.yml` → `mgmt-host-luks.yml` (which
enrolls TPM2 and carves the `docker-data` LV) → `goat-standup.sh <site>` take over
from. **Do not** enable TPM2 auto-unlock by hand here — the LUKS node class does
it idempotently and records it.

## 5. Standards for every node: it will move

Nodes are built at one site and run at another. Each of these is checked before a node leaves the bench:

| Standard | How |
|---|---|
| **Every wired port takes DHCP** | The installed netplan matches `en*` with `dhcp4: true` and cloud-init's network config is disabled (autoinstall `late-commands` do both). One port configured means a cable in any other port gives nothing. |
| **The boot image takes DHCP too** | `dropbear-initramfs` and `luks-unlock` default to `IP=dhcp`. A static `ip=` string names one site's LAN: a moved node boots unreachable and keeps that route after boot. |
| **The disk passphrase is in the team store before the disk is encrypted** | Put it in the store first, install second, then prove the stored value with `cryptsetup open --test-passphrase` before the node leaves the bench. Automatic unlock (TPM, Tang) hides a missing passphrase until the day it is needed. |
| **No login lives only in a file** | Build and console logins go into the team store with the node's name; local env files hold references only. |
| **Every MAC and the port map are recorded** | All NIC MACs (not just the cabled one), which interface is which case port, and which port is cabled, in the site inventory in the team store. |
| **Labels survive handling** | Hostname and first MAC on the case, durable (engraved or laminated), and the same on the box. A hardware key (YubiKey) is labelled with the node it unlocks and its serial is recorded against that node. |
| **Operator keys on every node** | Every platform admin's SSH key on the node and in its unlock shell, so any of them can reach it from a jump host on the same switch. |
| **One setting per goatnet** | Mesh client settings such as quantum resistance match the goatnet's declared value on every node. |

## 6. Moving nodes to another site

A site's nodes travel as one set, arrive as one set, and come up one at a time against a written list.

| Step | Rule |
|---|---|
| **Before packing** | Every node meets section 5. The site's manifest is in the team store: node name, hardware, every MAC and the port map, disk-unlock method, hardware-key serials, logins by reference. |
| **One box per site** | Never mix two sites' nodes, or built and unbuilt machines, in one box. Label the box with the site and the node list. |
| **Keys stay with their node** | A hardware key (YubiKey) travels in a labelled bag taped to its node, or stays in custody with the store recording which node it unlocks. A key is never pulled loose. |
| **Borrowing a node** | A node taken for another use is recorded in the store (who, where, what changed) and is rebuilt before it rejoins its site. |
| **On arrival** | Cable and power one node at a time; match each to the manifest by MAC and label before the next. Report each node as it comes up. |
| **Unbuilt machines** | Do not image a machine on arrival. Imaging follows the documented build, on a plan agreed by the platform admins, once the site's assigned nodes are up. |

## A board with no usable console (ZimaBoard 2)

Sections 1 to 3 assume you can reach the firmware and boot a USB stick. A ZimaBoard 2 gives you
neither: its video is mini-DisplayPort only, and its firmware boots the internal disk before USB.
Install it over one Ethernet cable instead, with
[`ops/bootstrap/cable-install/`](../../ops/bootstrap/cable-install/README.md): a helper machine
on the cable serves the Ubuntu ISO and the unattended seed, and the board's own bootloader
starts the installer. `CABLE_MODE=luks` gives the same `crypto_LUKS → dm_crypt → ubuntu-vg`
shape as section 3, on `mmcblk0p3`, bound to the TPM during the install so the first boot needs
nobody. Then continue at section 4.

## Raspberry Pi (arm64, no TPM)

The Pi path is not yet turnkey the way the x86 autoinstall is. Read this
before flashing, so the box lands encrypted rather than as a plaintext install
that has to be redone.

**Flashing + key.** Raspberry Pi Imager writes Ubuntu Server 24.04 arm64; preset
the hostname, your SSH key, and enable SSH in its advanced options. Imager seeds
the key per card — apply it to **every** Pi, or one comes up unkeyed. The imager's
user is whatever you set (e.g. `goatnet`), not `goat`; the standup
converges the user later.

**Historical baseline.** Pi Imager produces an **unencrypted** root
(`nvme0n1p2 ext4`, no LUKS). The LUKS
role's `tang_initramfs` class enrolls an unlock mode onto an *existing*
`crypto_LUKS` partition — it does not create one — so the Pi must land encrypted
before the standup. Three courses of action were weighed:

| COA | Method | Status |
|---|---|---|
| **A** | in-place `cryptsetup reencrypt` of the Imager root, run once from a first-boot service | **Rejected** — brick-risk if interrupted on a headless box, and these boxes get wiped and re-imaged anyway, so re-encrypting a throwaway install buys nothing |
| **B** | use the Pi base-image builder to flash, clone and encrypt Ubuntu arm64 | **Current path** — `ops/rpi-base-image/`; see the builder and the adoption procedure below |
| **C** | UEFI-boot the arm64 Live Server ISO and run subiquity autoinstall | **Superseded** — retained as an unvalidated reference procedure, not the Pi golden path |

The phase-1 host preflight now **fails** on an unencrypted root, so a plaintext
Pi cannot slip past silently the way it did.

**One more path, for a Pi that is already installed and you do _not_ want to
re-image.** The COA table above is about landing a *blank* flash encrypted before
standup. When the box is already running Ubuntu on its NVMe and is reachable only
over SSH, there is a headless in-place option that encrypts it where it sits —
see [§Adopt an already-installed Pi in place, headlessly](#adopt-an-already-installed-pi-in-place-headlessly)
below. It is the in-place reencrypt approach that COA A set aside, made safe for
the not-re-imaged case by a `dropbear` remote-unlock initramfs plus a fail-closed
verify.

### Pi base-image builder and repair boot order

The current Pi build source is [`ops/rpi-base-image/`](../../ops/rpi-base-image/). Follow its README for a blank machine. It produces a plain LUKS root without LVM; the in-place adoption procedure below preserves the same shape. The encrypted-root preflight and Docker data-directory acceptance are recorded in F-538. Those measurements do not certify every unattended unlock mechanism.

The builder's FIPS-mode posture is not a NIST CMVP validation. Its userspace provider and the stock Raspberry Pi kernel have different validation limits; use the builder's compliance scope and caveats rather than treating a kernel boot flag as proof.

Before an SD-based repair or clone, check the actual boot order. `0xf416` tries NVMe first; `0xf461` tries SD first. A bootable old NVMe can prevent the new SD image from running. Existing NVMe users or host keys are the diagnostic tell. F-577 records the measured repair and the boot-source check.

### COA C — superseded UEFI/subiquity reference procedure

The Pi 5 has no built-in UEFI, so it cannot boot the arm64 Live Server ISO
unaided. The step that unlocks COA C is putting **EDK2 UEFI firmware** on the
Pi's boot medium; from there the install is the x86 host's, verbatim, driven by the
arm64 answer file
[`ops/bootstrap/autoinstall/user-data-arm64.yaml`](../../ops/bootstrap/autoinstall/user-data-arm64.yaml).

> **Honesty note.** Every step below is **designed, not proven.** No Pi has been
> installed through Pi5-UEFI + subiquity in this repo. This is a superseded reference procedure. Use the Pi base-image builder for new
> installations; retain these steps only for deliberate evaluation of the alternative.

1. **Get EDK2 UEFI firmware onto the Pi.** Use the community Pi 5 UEFI firmware
   project **pftf/RPi5** (the Pi 5 line of `raspberrypi/edk2-rpi5`). Download a
   release build (`RPi5_UEFI_Release.zip`), and either:
   - write its contents to the root of a FAT32 **SD card** and leave that card in
     the Pi (the Pi's ROM loads firmware from SD before NVMe) — the simplest
     path; **or**
   - flash it into the Pi's SPI EEPROM with `rpi-eeprom` so no SD card is needed.
     More permanent, and the intended end state, but do the SD-card route first
     to prove the firmware boots before committing it to EEPROM.

   Enter the EDK2 setup at power-on and confirm NVMe is enabled in the boot order
   and that the ISO/USB boots ahead of it.

2. **Boot the arm64 Live Server ISO via UEFI.** Write
   `ubuntu-24.04.x-live-server-arm64.iso` to a USB stick (balenaEtcher, or
   `dd if=... of=/dev/rdiskN bs=4m` on macOS from outside a TCC-protected
   folder). With EDK2 present the Pi shows the ISO as a UEFI boot option, exactly
   like an x86 host.

3. **Feed it the arm64 answer file.** Fill the three placeholders in
   `user-data-arm64.yaml` (`<HOSTNAME>`, `<OPERATOR_SSH_PUBKEY>`,
   `<LUKS_PASSPHRASE>` — the last from your env file, never committed), then feed
   it the same two ways as on x86:
   - **Interactive-served (nocloud):** put `user-data` (a copy of
     `user-data-arm64.yaml`) and an empty `meta-data` on a second USB labelled
     `CIDATA`; the installer picks it up automatically.
   - **Kernel arg:** at the GRUB installer prompt add
     `autoinstall ds=nocloud;s=/cdrom/server/` (or wherever you served the files).

   subiquity's `storage: layout: {name: lvm, password: ...}` runs under UEFI on
   the Pi as it does on x86: it creates an ESP + `/boot` + the
   `crypto_LUKS` partition, giving the same `nvme0n1p3 crypto_LUKS → ubuntu-vg`
   shape. The box reboots and halts at the **LUKS passphrase prompt** — type the
   passphrase once, and you have an SSH-reachable, encrypted Pi in the identical
   hand-off state as an x86 host.

**What still needs validating on-site** (do not report any of these as done until
a real Pi 5 has proven them):

- that EDK2 (pftf/RPi5) boots the Pi and exposes the ISO + NVMe as UEFI options;
- that subiquity's guided `lvm` layout actually produces `nvme0n1p3 crypto_LUKS`
  over `ubuntu-vg` on the arm64 image (device index and VG name are asserted by
  the LUKS role and must match — update the node's group_vars if they differ);
- that the installed Pi cold-boots to the passphrase prompt and, after the LUKS
  role enrols Clevis+Tang, unlocks **in the initramfs** with no console (the
  `tang_initramfs` certification gap in
  host-site-node-disk-encryption.md §7).

**Unlock mechanism.** Encryption at rest and unattended unlock are two steps.
COA C produces the encrypted partition; the unlock method is enrolled afterward
by the standup's LUKS role and is described next.

**Unlock mechanism — proven.** With no TPM, the Pi's unattended unlock is
**Clevis + Tang** bound to a same-site peer (a paired x86 host runs `tangd`). The
Tang key-recovery path is proven on this hardware (an arm64 Pi
recovered a LUKS key from an x86 host's Tang server and the recovered key opened the
volume). What remains to certify is that recovery runs *inside the initramfs*
before pivot, which needs static initramfs networking — see the LUKS role's
`tang_initramfs` notes. Backups (pgBackRest on the Pi NVMe) are unaffected by the
unlock mechanism.

## Adopt an already-installed Pi in place, headlessly

Encrypt a Pi 5 that is **already running Ubuntu on its NVMe** and reachable
**only over SSH** — no free SD slot, no clone, no HDMI/keyboard — without
re-imaging it. This is the path for a box already provisioned (an
observer node) where wiping to run COA C would throw away work. It produces a
plain-partition LUKS root (`cryptdevice=/dev/nvme0n1p2:rootfs`, passphrase
unlock), **not** the COA C `nvme0n1p3 crypto_LUKS → ubuntu-vg` shape, so it does
not feed the LUKS role's installer-partition enrolment the way COA C does.

> **Status.** The prepare + dropbear layer + the
> fail-closed verify are proven on a 1.8 TB Pi NVMe; the full end-to-end cycle is **not
> yet certified.** Treat it as a procedure to confirm on the hardware, not a
> stamped path.

The mechanism lives in [`ops/rpi-base-image/`](../../ops/rpi-base-image/),
the Pi 5 base-image builder. Two scripts, run over SSH on the Pi:

1. **`03-prepare-encryption.sh`** (the builder's) — installs `cryptsetup` + initramfs
   hooks, points `/etc/fstab` at `/dev/mapper/rootfs`, and rewrites
   `cmdline.txt` to `cryptdevice=/dev/nvme0n1p2:rootfs`.
2. **`03b-headless-dropbear-unlock.sh`** (the
   headless layer) — installs `dropbear-initramfs`, drops the operator
   pubkey into `/etc/dropbear/initramfs/authorized_keys`, writes a static
   `IP=<ip>::<gw>:<mask>::<nic>:off` line into `initramfs.conf`, adds the NIC
   driver (`macb`, the Pi 5 onboard NIC) + `dm-crypt` to the initramfs, and
   rebuilds it.

**The fail-closed verify is the safety lesson.** Once step 1 has rewritten
cmdline to `cryptdevice=`, the box halts in the initramfs at every boot. On a
headless node with no working dropbear that is a **brick**. So `03b`, before it
declares success, runs `lsinitramfs` against the initramfs the bootloader
actually loads (`/boot/firmware/initrd.img`, per `initramfs initrd.img
followkernel` in `config.txt`) and confirms it contains `dropbear`,
`authorized_keys`, the NIC driver, and `cryptsetup` — and refuses to exit clean
if any is missing. Do not reboot until that verify passes.

After the reboot the box drops to the initramfs with dropbear listening;
`ssh root@<ip>`, mount the boot partition, and run the reencrypt:

```sh
mount -t vfat /dev/nvme0n1p1 /mnt/boot
sh /mnt/boot/helper-scripts/encrypt-rootfs.sh      # passphrase mode
```

The reencrypt is **resumable, but recovery is not always automatic.** A dropped
SSH session pauses it; reconnect and `cryptsetup reencrypt --resume-only
/dev/nvme0n1p2`. A power-off mid-reencrypt loses data outright — and an *unclean*
interruption of a `datashift` reencrypt (the mode the builder's `--encrypt` uses:
it shifts the data 8 MiB to seat the LUKS2 header) can leave the header needing a
recovery pass first. `--resume-only` then refuses with *"Device requires
reencryption recovery. Run repair first."* The recovery reads the volume key, so
`repair` needs the passphrase — **`cryptsetup repair -q` without a key silently
no-ops** (`rc=0`, nothing recovered; run it without `-q` and it returns `rc=2`,
permission denied). Recover with the key, then resume:

```sh
printf '%s' "$PASS" > /tmp/.p && chmod 600 /tmp/.p
cryptsetup repair -q --key-file /tmp/.p /dev/nvme0n1p2       # clears the in-reencryption flag
cryptsetup reencrypt --resume-only --key-file /tmp/.p /dev/nvme0n1p2
rm -f /tmp/.p
```

Confirm the recovery took before resuming: `cryptsetup luksDump /dev/nvme0n1p2`
should no longer show `flags : in-reencryption`, only the `online-reencrypt-v2`
requirement. Measured — the recovery preserved the
~134 GiB already encrypted (the resume picked up at 7 %, not 0).

**Running it detached — the initramfs has no `nohup`/`setsid`.** The
`dropbear-initramfs` busybox is built without `nohup`, `setsid`, or `timeout`, so
a plain background job dies when the SSH exec returns and the usual detach tricks
are unavailable. Two options: (1) hold one long-lived SSH session open for the
whole run — simple, but the connection must stay up; or (2) orphan the job to
init with a subshell, which survives a non-pty SSH exec:
`( cryptsetup reencrypt --resume-only --key-file /tmp/.p /dev/nvme0n1p2 >/tmp/reenc.log 2>&1 & )`,
then poll `/tmp/reenc.log`. The **durable fix**, folded into `03b`, is a
`copy_exec` initramfs hook that adds `setsid`/`nohup`/`timeout` to the dropbear
initramfs, so a future run backgrounds cleanly with `setsid … &` and needs no
held connection.

**Two measured findings** (a 1.8 TB Pi 5 NVMe):

- **Encryption time scales with partition size.** `cryptsetup reencrypt`
  XTS-encrypts the entire partition. Measured ~150 MiB/s at PCIe **Gen 2** on the
  1.8 TB NVMe ≈ **~3.4 h** — not the ~40 min a small drive implies. Schedule by
  partition size.
- **Gen-3 lever.** The Pi 5 defaults its NVMe slot to PCIe Gen 2 (5.0 GT/s).
  Adding `dtparam=pciex1_gen=3` to `config.txt` + reboot brings the link to Gen 3
  (8.0 GT/s, ~2×), roughly halving the encrypt time. It is **not guaranteed
  stable** on every Pi-5/NVMe/HAT — the cmdline already carries NVMe stability
  params (`nvme_core.default_ps_max_latency_us=0 pcie_aspm=off
  nvme_core.io_timeout=255`), but watch `dmesg` for NVMe errors/resets. On
  the measured box the link came up at 8.0 GT/s with one transient NVMe error at
  link-up, stable after — measured **~210–230 MiB/s** (vs ~150 at Gen 2), an ETA
  of **~2.5 h** for the 1.8 TB partition. Drop back to Gen 2 if resets recur.

The static `IP=` form and the `macb` driver here match the `initramfs_ip` /
`initramfs_kmods` values the LUKS role's `tang_initramfs` example carries in
host-site-node-disk-encryption.md
§9 — this run is the first real-hardware confirmation that `macb` is the correct
Pi 5 NIC driver for an initramfs network unlock.

## Where this sits

Before the run, ahead of [goatnet-standup.md](goatnet-standup.md) §"Before phase 1". The
turnkey chain in CLAUDE.md (`seed-ssh-key.sh` → baseline → LUKS → `<site>-bootstrap`)
begins the moment §4's verification passes.
