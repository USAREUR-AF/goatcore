# Jetson AGX Orin — Ubuntu-for-Tegra flash

Self-contained automation for flashing stock **Ubuntu-for-Tegra 22.04**
(NVIDIA L4T R36.4.x) onto an NVIDIA Jetson AGX Orin from an x86 Ubuntu
host on the LAN, optionally driven from a Mac on the same LAN.

This is the `goat` project's pinned procedure — no external repos
needed to replay it. See `docs/operations/pilot-bringup.md`
for the wider runbook this fits inside (this directory automates the
flash step itself; the Ansible chain takes over once the Orin boots).

> This directory is the Ubuntu-only path. The OS choice is driven by FIPS 140-3
> and DISA STIG compliance.

## Topology

```
  ┌─────────┐  ssh   ┌──────────────────────┐  USB-C   ┌──────────────┐
  │   Mac   │ ─────> │ x86 Ubuntu host      │ ───────> │ Jetson AGX   │
  │  admin  │  LAN   │ (drives flash.sh,    │ recovery │ Orin (target │
  └─────────┘        │  has qemu, lsusb,    │   mode   │  device)     │
                     │  Linux-only tools)   │          └──────────────┘
                     └──────────────────────┘
```

- **Mac** runs [`flash-orin-remote.sh`](flash-orin-remote.sh) — pushes the host-side
  script, ensures host prereqs, kicks off the flash, streams output back.
- **x86 Ubuntu host** runs [`flash-jetson-ubuntu.sh`](flash-jetson-ubuntu.sh) — does
  the actual L4T BSP download, NVIDIA-binary apply, ARM64 chroot
  pre-bake, default-user creation, and `flash.sh` invocation.
- **Jetson AGX Orin** is the target. It boots into stock Ubuntu 22.04
  with Docker, netbird, rathole, and a curated set of system + network
  tools already installed.

## Pre-bake or lean?

This script **pre-bakes** a curated set of tools into the rootfs
before flash. Trade-off:

| | Pre-bake (this script) | Lean (Option B in pilot-bringup §1) |
|---|---|---|
| Host-side cost | One-time ~5–10 min for `qemu-user-static` chroot | None |
| Orin-side cost post-flash | Less Ansible work | Ansible installs everything |
| First-boot SSH | Works immediately (key + password) | Same |
| Failure surface | Host chroot can break on network blips | Simpler |

We default to pre-bake because the host-side cost is one-time and the
Orin-side savings recur on every redeploy. The lean path is documented
in `docs/operations/pilot-bringup.md` §1
Option B if you ever need to skip the chroot dance.

## Project posture (vs upstream NVIDIA L4T flow)

The pre-bake step deliberately **omits** what's in the upstream
JetPack / community Ubuntu flash recipes:

- **No Kubernetes tooling** (kubectl / k9s / helm / kubectx / kubens).
  Project rejects k8s/k3s/swarm/nomad in favour of Docker Compose —
  see `CLAUDE.md` "Container orchestration."
- **No cloudflared.** Project does not use Cloudflare Tunnel/WARP —
  see `CLAUDE.md` "Cloudflare Tunnel / WARP not used."
- **No plaintext password default in the script.** First-boot
  password is required via env var, not committed —
  `CLAUDE.md` "No plaintext secrets in committed
  files."

What **is** baked: Docker, netbird, rathole, openssh-server, ufw,
fail2ban, system tools (htop / iotop / tmux / vim / jq / git), network
tools (nmap / iperf3 / tcpdump / ethtool / mtr / dig /
wireguard-tools).

## Prerequisites

### On the Mac

- `sshpass` (`brew install sshpass`) — only needed for the one-time
  `seed-ssh-key.sh` call before flash, not for the flash itself.
- `direnv`-loaded `.envrc.local` with the values below.

### On the x86 Ubuntu host

The remote helper installs these automatically the first time it runs,
but you can pre-install:

```bash
sudo apt update
sudo apt install -y openssh-server wget tar usbutils \
                    qemu-user-static curl unzip ca-certificates
sudo systemctl enable --now ssh
```

Disk: ~30 GB free in the operator user's `$HOME`.

### On the Jetson AGX Orin

- USB-C cable from the Jetson's flashing port (the one **above** the
  power port, not the regular USB ports) to the x86 host.
- Power adapter.
- Recovery mode (see "Recovery mode steps" below).

## Environment variables

> **Compliance note — notional defaults.** The committed defaults
> below (`goatnet` / `goatnet`) are **notional placeholders** that let
> an unattended flash succeed end-to-end. Real deployments MUST
> override both `JETSON_FLASH_USER` and `JETSON_FLASH_USER_PASS` via
> env var (in `.envrc.local`, gitignored), select operator-chosen
> values, and rotate the password immediately post-flash via the
> `ops/scripts/seed-ssh-key.sh` chain. Treat the committed defaults
> the way you'd treat raspbian's `pi/raspberry` — known-publicly,
> never to be used in production.

Set in `.envrc.local` (gitignored, 1Password-backed):

```bash
# x86 Ubuntu flash host on the LAN
export ORIN3_FLASH_HOST=192.0.2.27
export ORIN3_FLASH_USER=flash-admin
export ORIN3_FLASH_PASS='<the host login password>'    # used once, by seed-ssh-key.sh

# Jetson first-boot user + password — operator-chosen.
# Pick values; record in 1Password; rotate immediately post-flash.
export JETSON_FLASH_USER='<your-user>'
export JETSON_FLASH_USER_PASS='<your-strong-password>'

# OR: match an existing per-site password (e.g. Orin-C joining an
# Orin-A/B pair):
#   export JETSON_FLASH_USER_PASS=$VEGA_A_PASS

# Optional overrides
export JETSON_FLASH_HOSTNAME=orin-c
export JETSON_FLASH_BOARD=jetson-agx-orin-devkit
# JETSON_FLASH_SSH_KEY is auto-detected from ssh-agent if unset.
```

Then `direnv allow`.

## End-to-end procedure

### 1. One-time: install + enable SSH on the x86 host

On the host's own console (keyboard + monitor, not over SSH because
SSH isn't up yet):

```bash
sudo apt update
sudo apt install -y openssh-server
sudo systemctl enable --now ssh

# If ufw is active:
sudo ufw allow OpenSSH
sudo ufw status
```

Verify from the Mac:

```bash
ssh -o PreferredAuthentications=password "$ORIN3_FLASH_USER@$ORIN3_FLASH_HOST" whoami
```

### 2. One-time: seed key auth + NOPASSWD sudo

```bash
ops/scripts/seed-ssh-key.sh "$ORIN3_FLASH_USER@$ORIN3_FLASH_HOST" ORIN3_FLASH_PASS
```

This pushes your Mac SSH pubkey, validates key auth, then installs
`/etc/sudoers.d/$USER` with NOPASSWD ALL. After this the password is
no longer needed for any operation here. Disable password auth once
this works.

### 3. Put the Orin in recovery mode

1. Unplug power from the Jetson.
2. Connect USB-C from the Jetson's flashing port (the port **above**
   the power port) to the x86 host.
3. Hold the **RECOVERY** button.
4. Plug in power while still holding RECOVERY.
5. Wait 2 seconds, release RECOVERY.

Verify from the Mac:

```bash
ssh "$ORIN3_FLASH_USER@$ORIN3_FLASH_HOST" 'lsusb | grep NVIDIA'
# expect: Bus 001 Device NNN: ID 0955:7023 NVIDIA Corp. APX
```

`APX` = "Application Processor in eXternal mode" = recovery mode
confirmed.

### 4. Flash

```bash
./ops/jetson-flash/flash-orin-remote.sh
```

The helper:

1. Probes key auth (fails fast if step 2 wasn't done).
2. Verifies host prereqs (`qemu-user-static`, disk space, Jetson
   visibility); installs missing apt packages.
3. SCPs `flash-jetson-ubuntu.sh` to the host.
4. Runs it under `sudo -E` with the right env vars.
5. Streams stdout/stderr back to your Mac terminal.

Total time: ~30–45 min (683 MiB L4T BSP download + 1.7 GiB rootfs +
chroot pre-bake + flash). The Orin reboots automatically when flash
completes. Pre-staging both archives (see "Pre-staging a flash" below)
takes the download out of that window.

### 5. Find the Orin on the LAN

The Orin's display name will be the value of `JETSON_FLASH_HOSTNAME`
(`orin-c` by default). DHCP gives it an IP — find it via:

- Your DHCP server's lease list, **or**
- ARP scan from any host on the LAN: `arp -a | grep -i tegra` or
  `nmap -sn 192.168.1.0/24` (slow), **or**
- `avahi-browse -art` if mDNS is running on the LAN.

### 6. Hand off to the Ansible chain

```bash
# Replace <orin-ip> with the IP you found in step 5, and <user> with
# the value of $JETSON_FLASH_USER (notional default 'goatnet';
# whatever you actually picked).
ops/scripts/seed-ssh-key.sh "<user>@<orin-ip>" JETSON_FLASH_USER_PASS

# Then continue at docs/operations/pilot-bringup.md §3 onward:
#   - mgmt-host-baseline.yml  (apt upgrade, hostname, /etc/hosts)
#   - mgmt-host-luks.yml      (NVMe LUKS enrollment)
#   - <site>-bootstrap.yml    (mgmt-plane stack — only on Orin-A/B)
```

For Orin-C (observability + storage tier per
ADR 0826) the
final step is the observability bootstrap, not a mgmt-plane bootstrap.

## Re-running

The flash script is **idempotent on workspace state** but not on the
device:

- L4T BSP + rootfs tarballs are cached in `WORK_DIR` (default
  `$HOME/jetson-ubuntu-flash`) — re-runs skip downloads.
- The pre-bake chroot is a fresh chroot each run (mount + apt + unmount).
- `flash.sh` rewrites the eMMC every time. Safe to re-run; takes the
  full 10–20 min flash time again.

If you need to re-bake the rootfs but skip the flash (e.g. to retry
after a network blip during apt), delete `WORK_DIR/Linux_for_Tegra/rootfs`
and re-run — the rootfs gets re-extracted, re-applied, and re-baked.

## Troubleshooting

### `Permission denied (publickey,password)` on first SSH

Username case is wrong. The Mac's `~/.ssh/config` may default to your
Mac username; use `$ORIN3_FLASH_USER@$ORIN3_FLASH_HOST` explicitly.

### qemu-user-static install hangs

Ubuntu's apt occasionally stalls on first `apt update` after install.
Re-run `flash-orin-remote.sh` — the prereq check is idempotent.

### `Jetson not detected via USB`

- USB-C cable is plugged into the Orin's regular USB port instead of
  the flashing port (it's the one **above** the power port, not the
  array of USB-A's).
- Cable is power-only. Try a different USB-C cable known to carry data.
- Recovery mode wasn't held long enough. Repeat steps in §3.

### Flash hangs at ~50%

USB issue mid-flash. Power-cycle Orin back into recovery mode, restart
the flash. The L4T BSP cache survives so the second run is fast to the
flash step.

### First boot doesn't show on monitor / no LAN IP

The pre-bake skips OOBE so a monitor isn't needed. If DHCP doesn't
give the Orin an IP, attach a USB keyboard + monitor (HDMI), log in as
`$JETSON_FLASH_USER` with `$JETSON_FLASH_USER_PASS`, run `ip a` to
diagnose, and configure netplan as in
`pilot-bringup.md` §4.

## Files in this directory

- [`flash-orin-remote.sh`](flash-orin-remote.sh) — Mac-side helper. Drives the flash
  over SSH from the Mac.
- [`flash-jetson-ubuntu.sh`](flash-jetson-ubuntu.sh) — host-side script. Does the
  L4T download / apply_binaries / chroot pre-bake / `flash.sh`.
- [`README.md`](README.md) — this file.

## Pinned versions

| Component | Version | Rationale |
|---|---|---|
| L4T | R36.4.3 (JetPack 6.1.x) | Matches `pilot-bringup.md` §1 (JetPack 6.1 baseline). Bump in lockstep when moving to a newer JetPack. |
| netbird | 0.68.3 | Same version as the rest of the fleet. |
| rathole | 0.5.0 | Same version as the relay VPSes. |

The two L4T archives are additionally pinned **by SHA-256** in
`flash-jetson-ubuntu.sh`, and the script refuses to flash on a mismatch:

| Archive | Size | SHA-256 |
|---|---|---|
| `Jetson_Linux_r36.4.3_aarch64.tbz2` | 683 MiB | `949a4404…1873d2` |
| `Tegra_Linux_Sample-Root-Filesystem_R36.4.3_aarch64.tbz2` | 1.7 GiB | `67b85bae…52156d` |

Recorded from `developer.nvidia.com` over TLS. NVIDIA publishes no
checksum for these files — no `.sha256`/`.sha512`/`.md5` sibling exists — so
these are trust-on-first-use digests, not vendor-attested ones. They catch a
corrupt download and any later in-place re-spin of the same filename; they
cannot vouch for the original bytes. Treat a mismatch as "stop and find out
why," never as "bump the constant and continue."

When bumping any of these: edit the constants at the top of
`flash-jetson-ubuntu.sh`, re-run an end-to-end flash on a test Orin,
and update both tables in the same PR.

## Pre-staging a flash

Nothing here needs an Orin attached, and the archives are the slow part
(~2.4 GB combined, several minutes on a good link). Two ways to have them
ready:

- **On the flash host**, which is where the script actually reads them:
  drop both files in `$WORK_DIR` (default `$HOME/jetson-ubuntu-flash`) named
  `tegra.bz2` and `rootfs.bz2`. The script's cache check verifies both the
  bzip2 stream and the pinned digest before skipping the download, so a
  pre-staged file that is wrong gets re-fetched rather than used.
- **On the operator laptop**, if the flash host is not reachable yet. There
  is a staged copy under `~/.goat/state/jetson-flash-staging/` with a
  `SHA256SUMS` beside it; copy it across when the host exists.

What cannot be pre-staged without the flash host: the apt prerequisites and
the `qemu-user-static` chroot pre-bake. Both are host-side and one-time.

## References

- `docs/operations/pilot-bringup.md` — wider Orin bringup runbook
- [`docs/design/os-baseline-and-luks.md`](../../docs/design/os-baseline-and-luks.md) — fleet OS baseline + LUKS
- `docs/adr/0826-per-site-three-orin-topology.md` — A/B/C role split
- [`ops/scripts/seed-ssh-key.sh`](../scripts/seed-ssh-key.sh) — first-contact SSH bootstrap
- NVIDIA Developer L4T R36.4.3 BSP — pinned URL inside `flash-jetson-ubuntu.sh`
