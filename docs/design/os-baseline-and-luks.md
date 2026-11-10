# OS baseline and LUKS unlock

**Scope:** every managed host — x86_64 management hosts (Ubuntu 24.04), Jetson-class arm64 hosts (Ubuntu for Tegra 22.04) and Raspberry Pi 5 edge nodes. The roles that implement it are [`luks-unlock`](../../ops/ansible/roles/luks-unlock/), [`tang-server`](../../ops/ansible/roles/tang-server/), [`docker-engine`](../../ops/ansible/roles/docker-engine/) and [`ntp-hygiene`](../../ops/ansible/roles/ntp-hygiene/); the playbooks are `mgmt-host-baseline.yml` and `mgmt-host-luks.yml`.

**The two properties that must hold together:** data at rest is encrypted on every host that holds non-public data, and a host returns to full service after a power cycle with nobody at it.

---

## 1. Requirements

| # | Requirement |
|---|---|
| R1 | Data-at-rest encryption on every host with non-public data |
| R2 | Unattended recovery from a power cycle: a routine reboot needs no human gesture |
| R3 | A break-glass path a non-specialist can follow from one printed page |
| R4 | Resists disk extraction, boot-chain tampering and live-boot-to-read-disk attacks |
| R5 | A stolen running host is no harder for its owner to recover than one stolen after shutdown |
| R6 | The same roles deploy on x86 and arm64; the unlock mechanism is a variable |
| R7 | A management host ignores a short press of its power button (`HandlePowerKey=ignore`, set by `mgmt-host-baseline.yml`); a long press still powers off |

---

## 2. Three-tier storage model

| Tier | What lives here | At-rest protection | Unlock path |
|---|---|---|---|
| **1 — Root** | OS, container runtime, mesh client binaries, public configuration | Installer LUKS or signed read-only root | Unlocked by the installer's keyslots (TPM2 where present) |
| **2 — Config-small** | Host keys, machine ID, mesh peer key, client certificates, secrets rendered at deploy time | LUKS2, TPM-sealed | Silent unseal when the measured boot chain matches |
| **3 — Data** (the NVMe at `/opt/docker-data`) | Postgres, audit logs, `.env` files, TLS private keys, application state | LUKS2, TPM-sealed and/or Tang-bound | Silent unseal; post-boot Tang unlock (§3.2) |

**Services depend on Tier 3.** Every service that reads it carries `RequiresMountsFor=/opt/docker-data`, so nothing starts on an empty directory.

**Docker `data-root` and containerd `root` both point at Tier 3.** Two daemons, two settings, both written by the `docker-engine` role:

- `/etc/docker/daemon.json`: `"data-root": "/opt/docker-data/docker"` (`docker_engine_data_root`)
- `/etc/containerd/config.toml`: `root = "/opt/docker-data/containerd"` (`docker_engine_containerd_root`)

Setting one without the other fills the root partition slowly. `site-bootstrap.yml` fails its pre-flight if a running daemon's root does not match.

**Clock before chrony and docker.** Hosts without an RTC battery boot at 1970-01-01, and every TLS handshake (including NTS key exchange) then fails. The `ntp-hygiene` role installs `chrony-presync.service`, a oneshot that steps the clock from plain NTP only when the year is below `chrony_presync_min_year`; chrony then runs NTS-only.

**inotify budget.** The `docker-engine` role raises `fs.inotify.max_user_instances` to 1024 (`/etc/sysctl.d/99-goat-inotify.conf`). The kernel default of 128 is shared by every root container on the host; when it runs out, the next service to start silently comes up without its file watcher (Traefik then serves no dynamic routes).

---

## 3. Unlock mechanisms

Each mechanism is a separate LUKS keyslot, so any one can be added or removed without touching the others. The `luks-unlock` role selects them per volume with `unlock_modes`.

### 3.1 TPM2 (routine)

`clevis luks bind … tpm2 '{"pcr_bank":…,"pcr_ids":…}'` (installer partitions) or `systemd-cryptenroll --tpm2-device=auto` (role-managed volumes) seals a key that the TPM releases only when the measured boot chain matches enrollment.

| PCR | Measures | Included because |
|---|---|---|
| 0 | Platform firmware | Detects a firmware change |
| 2 | Option ROMs | Detects an added PCIe device |
| 4 | Bootloader | Detects a swapped bootloader |
| 7 | Secure Boot state and keys | Detects Secure Boot off or a rogue key |
| 11 | Unified kernel image | When a UKI is used: detects a kernel or initramfs swap |

PCR 8 and 9 are left out: they change on every kernel update and would break unattended unseal.

Variables: `luks_clevis_tpm2_pcr_bank`, `luks_clevis_tpm2_pcr_ids`, `luks_tpm2_pcrs`.

### 3.2 Tang (network-bound)

`clevis` wraps the key so reconstructing it needs an exchange with a Tang server on the network. Tang never sees the key. Combined with TPM2 through `clevis sss`, the host unlocks only on its own hardware **and** its own network.

**Peer unlock.** At a site with two or more hosts, each runs Tang (`tang-server` role) and binds its data volume to its peers, so any surviving peer unlocks the rest and a whole-site power restore heals itself.

**The unlock runs after boot, not during it.** The `luks-unlock` role renders `goat-unlock-<vol>.service`, a oneshot ordered `After=network-online.target tangd.socket`. crypttab keeps the volume `noauto`, so the boot path is the same as a passphrase-only host. If the unlock fails, the host is still up and reachable and can be fixed remotely; an in-boot failure would take it off the network. (In-boot clevis also conflicts with `headless` crypttab, and `_netdev` creates ordering cycles with the Jetson platform units.)

| Rule | Enforced by |
|---|---|
| A socket unit never orders on `network-online.target`; a late address uses `FreeBind=true` | `tang-server` role drop-in |
| Tang servers are installed before volumes bind to them | standup driver runs `tang-server.yml` before `mgmt-host-luks.yml` |
| A `tang_client` volume binds to at least `luks_tang_min_reachable` (default 2) servers | `luks-unlock` pre-flight |
| Every 2 minutes each host measures volume open, Tang servers answering, ordering cycles and its own Tang listener, and restarts a stalled unlock | `goat-unlock-health.timer` → textfile `goat_luks_unlock.prom` |

**Tang addressing.** `tang_listen_address` must be stable, so usually not a DHCP site LAN; give the hosts a dedicated secondary address. Avoid `172.16.0.0/12` (Docker bridge networks). Plain HTTP is correct for Tang: the exchange is blinded ECDH, so an observer learns nothing.

**Recovery when a volume stays locked:** start `tangd.socket` on any server named by `clevis luks list -d <device>`, then `systemctl restart goat-unlock-<vol>.service` (the health timer does this by itself within 2 minutes). With no Tang server, `cryptsetup open <device> <vol>` with the passphrase, mount, and start the consumers.

**Acceptance test:** a cold boot with the power cut. Nothing short of that proves the unlock path.

### 3.3 FIDO2 hardware token (break-glass)

`systemd-cryptenroll --fido2-device=auto`. The token releases the key on touch (and PIN, when `luks_fido2_with_pin: true`). It is inserted only when TPM2 unseal has failed (firmware update, TPM reset) and stays out of the machine otherwise. Keep a primary and a backup token per site.

### 3.4 USB keyfile (hosts without a TPM)

A Raspberry Pi 5 has no TPM. Until one is fitted (a TPM 2.0 HAT), its volume uses a keyslot bound to a keyfile on a USB drive inserted at boot and removed after unlock. When a TPM is fitted, re-enroll to `tpm2` and keep the keyfile slot as break-glass.

### 3.5 Passphrase

Every volume keeps a passphrase slot as the last resort, typed at the local or serial console.

### 3.6 Recommended keyslots per volume

| Slot | Type | Use |
|---|---|---|
| 0 | 32-character random passphrase, kept in your secret store | Recovery of last resort |
| 1 | Memorable passphrase (≥ 20 characters) | Manual unlock at the console |
| 2 | FIDO2 token | Break-glass when TPM2 unseal fails |
| 3+ | TPM2, Tang (via clevis) | Routine unattended unlock |

Rotate any slot alone with `cryptsetup luksRemoveKey` + `luksAddKey`, or `systemd-cryptenroll --wipe-slot=<type>` and re-enroll.

### 3.7 Example: a two-host site

```yaml
# group_vars/<site>_hosts/vars.yml
luks_volumes:
  - name: docker-data
    device: /dev/nvme0n1
    mount_point: /opt/docker-data
    filesystem: ext4
    unlock_modes: [passphrase, tpm2, fido2, tang_client]
luks_tpm2_pcrs: [0, 2, 4, 7, 11]
luks_tang_servers:
  - name: peer
    url: "http://<peer tang address>:7500"
```

A single-host site drops `tang_client` (no peer to bind to). Remote unlock over SSH into the initramfs is a separate role, `dropbear-initramfs`, not a `luks-unlock` mode.

**Installer-encrypted root plus a role-managed volume.** List the root under `luks_installer_partitions`; the role keeps the installer's crypttab line for it and refuses to render crypttab if that line is missing.

---

## 4. Break-glass order

The host stands up the network that makes it remotely reachable, so routine unlock must not depend on a remote person. Tier 1 and 2 unseal silently, the host joins the mesh, and an administrator can reach it from there.

| # | Path | Who acts | When |
|---|---|---|---|
| 1 | TPM2 unseal | No one | Routine reboot |
| 2 | Tang peer unlock | No one; needs a peer up | TPM2 unseal failed, or Tang-only volume |
| 3 | Initramfs SSH unlock (`dropbear-initramfs`) | Remote administrator | Auto-unlock failed; host's LAN reachable |
| 4 | FIDO2 token | Someone on site, token from the site kit | PCR drift after a firmware update, TPM reset |
| 5 | USB keyfile | Someone on site | Hosts without a TPM |
| 6 | Passphrase at the console | Someone on site with the printed runbook | Everything else has failed |
