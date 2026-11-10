# luks-unlock

Configure LUKS unlock for Tier-2 and Tier-3 volumes per [`docs/design/os-baseline-and-luks.md`](../../../../docs/design/os-baseline-and-luks.md). Multi-mode: the same role configures passphrase / TPM 2.0 / FIDO2 / USB-keyfile / SSS-threshold unlock depending on variables, so interim → target migrations are config flips, not rewrites.

## What it does

Given a block device on the target host (LUKS-formatted or blank):

1. **Optional: format-if-unformatted** (per-volume opt-in via `format_if_unformatted: true`). Idempotent — `cryptsetup isLuks` probe gates the destructive `luksFormat`; only runs when no LUKS header is present. Passphrase comes from `vol.passphrase` (typically `lookup('env', '<HOST>_LUKS_PASS')`). Same opt-in covers `luksOpen` (only if mapper not already open) and `mkfs.<filesystem>` (only if `blkid` reports no FS).
2. Install required tooling (`cryptsetup`, `clevis`, `clevis-tpm2`, `clevis-luks`, or `libfido2-1` per mode).
3. **Enroll FIDO2 keyslot** automatically when `fido2` is in `unlock_modes`. Idempotent — luksDump JSON probe gates new-slot enrollment; only adds when no `systemd-fido2` token present. The role pauses on the enrollment task — operator must insert the YubiKey + tap the button when prompted.
4. **Enroll TPM2 keyslot** automatically when `tpm2` is in `unlock_modes`. Idempotent — luksDump JSON probe gates new-slot enrollment; only adds when no `systemd-tpm2` token present. Non-interactive on systemd 249 (Ubuntu 22.04 / JetPack 6.1) via the `files/tpm2-enroll-helper.sh` ask-password-socket-answer pattern. PCR profile from `luks_tpm2_pcrs` (default `[0,2,4,7,11]`).
5. Render `/etc/crypttab` entries (with real UUIDs, looked up via `blkid`). When `tpm2` is in `unlock_modes`, the entry includes `tpm2-device=auto,tpm2-measure-pcr=yes` so the boot path actually uses the TPM2 keyslot for silent auto-unseal. The template writes the whole file, so on a node that also sets `luks_installer_partitions` the installer's own lines (matched by `UUID=<luks uuid>`) are read first and written back unchanged; if one is missing the role **refuses** to render rather than leave the initramfs with no root unlock.
6. Render `/etc/fstab` entries.
7. Render a systemd mount unit that gates downstream services on the volume mount.
8. Mount the volume now (so the rest of the play can write to the mount point without an interim reboot).

**Operator-must-be-present steps:**

- FIDO2 enrollment — role pauses for YubiKey insert + button-tap (turnkey, but interactive).
- SSS-threshold enrollment (Phase 3+) — debug message points at the manual `clevis luks bind sss` invocation.

**Non-interactive (no operator presence required):** `passphrase`, `tpm2`, `usb_keyfile`, `tang_client`.

## Supported modes

| Mode | Use case | Platform |
|---|---|---|
| `passphrase` | Interim; also disaster fallback | All |
| `tpm2` | Routine unlock on x86 with discrete TPM 2.0 or fTPM; **Block 65A** target on Orin via NVIDIA's fTPM-over-OP-TEE if `/dev/tpm0` is exposed | x86 mgmt hosts, Orin (pending 65A.0 fTPM spike) |
| `tegra_se` | Routine unlock on Orin / Thor via custom OP-TEE TA — fallback path if 65A.0 fTPM spike fails | Jetson AGX Orin, Thor |
| `fido2` | Break-glass slot (YubiKey 5 FIPS); **Block 65D** flips to presence-via-removal doctrine on Orin via `luks_fido2_user_presence: false` | Any x86 mgmt host, Orin |
| `usb_keyfile` | Interim RPi unlock without a TPM | RPi 5 |
| `tang_client` | **Block 65C** — peer-Orin Tang/NBDE auto-unlock. Unlock runs **POST-BOOT** via `goat-unlock-<vol>.service`, NOT in the boot transaction. URLs must be on a private substrate: wg-cp0-internal (`203.0.113.38/24` / legacy `10.99.0.0/24`), **or** any same-site private LAN declared with the explicit `class: lan` opt-in. Public Tang is rejected by pre-flight either way | Orin pair/trio within site, cross-site |
| `sss` | Threshold composition (e.g., 2-of-{tpm2, tang, fido2}) | Phase 3+ |

### `tang_client` unlocks AFTER boot — read before changing crypttab

`tang_client` is deliberately **not** treated as a boot-time auto-unseal mode: it does not
suppress `noauto`, and the volume stays off the boot path entirely. The unlock is a
post-boot oneshot (`goat-unlock-<vol>.service`) ordered after `network-online.target`.

Two reasons, both measured on real hardware, both non-obvious:

1. **`headless` makes in-boot clevis impossible.** It disables the systemd password
   *query*, which is exactly how `clevis-luks-askpass` delivers its key. This blocks
   **clevis-delivered modes only** — `tpm2`/`fido2` unseal inside `systemd-cryptsetup` and
   are unaffected. A host enrolling tpm2 *and* tang unlocks via tpm2 and never exercises
   the tang path, so a passing power-cycle test does **not** prove tang works.
2. **`_netdev` is hostile on Jetson.** It pulls `local-fs.target` into
   `network-online.target` ordering; systemd then deleted seven jobs to break cycles
   (including `nvpmodel`, `dbus`, `sockets.target` and the mount itself) and boot took
   ~20 minutes.

A post-boot failure leaves the host **up and reachable**; an in-boot failure takes it off
the network. Do not "simplify" this back into crypttab.

Every consumer of the volume must carry `RequiresMountsFor=` (the role emits this per
`systemd_requires` entry). The mount unit's `[Install] RequiredBy=` alone gives `Requires=`
with **no ordering**, which is how containerd once started against a bare mountpoint.

Multiple modes can be enabled simultaneously — each becomes a separate LUKS2 keyslot. Silent auto-unseal via TPM is the routine path; FIDO2 / passphrase / USB-keyfile / Tang are fallbacks. Per ADR 0612, Block 65 activates four modes in parallel (tpm2 + dropbear-initramfs (separate role) + tang_client + fido2-no-presence) for continuity priority on Orin.

## Variables

Per-volume configuration under `luks_volumes`:

```yaml
luks_volumes:
  - name: docker-data
    device: /dev/nvme0n1
    mount_point: /opt/docker-data
    filesystem: ext4
    unlock_modes:
      - passphrase                      # interim
      # - tpm2                          # activate when TPM enrolled
      # - fido2                         # activate when FIDO2 keys are enrolled
    systemd_requires:
      - docker.service                  # services that require this mount
```

Per-mode variables:

| Variable | Default | Description |
|---|---|---|
| `luks_tpm2_device` | `auto` | `systemd-cryptenroll --tpm2-device` value |
| `luks_tpm2_pcrs` | `[0, 2, 4, 7, 11]` | PCR profile (see os-baseline-and-luks.md §3.1) |
| `luks_fido2_device` | `auto` | FIDO2 token device |
| `luks_fido2_user_presence` | `true` | Whether to require a button-tap at unlock. Set to `false` for the Block 65D presence-via-removal doctrine. **YubiKey 5 FIPS-series authenticators override this and re-enable user-presence regardless** — see F-054. The role detects the override via `register: fido2_enroll_result` + sets the per-host fact `luks_fido2_up_override_observed` and surfaces a debug notice. |
| `luks_fido2_with_pin` | `true` | Whether to require a PIN at unlock. Set to `false` for hands-off cold boots. |
| `luks_fido2_force_reenroll` | `false` | When `true`, the role wipes any existing FIDO2 keyslot before enrolling the new one. Use to flip `luks_fido2_user_presence` / `luks_fido2_with_pin` without manually running `systemd-cryptenroll --wipe-slot=fido2`. Preserves slot 0 (passphrase) as the recovery path during the wipe + re-enroll window. |
| `luks_usb_keyfile_uuid` | _(required if mode=usb_keyfile)_ | UUID of the USB flash drive holding the keyfile |
| `luks_sss_threshold` | `2` | Shamir threshold (t) |
| `luks_sss_pins` | `{}` | clevis SSS pin config dict |

## Example invocations

### Interim: passphrase only

```yaml
luks_volumes:
  - name: docker-data
    device: /dev/nvme0n1
    mount_point: /opt/docker-data
    filesystem: ext4
    unlock_modes: [passphrase]
    systemd_requires: [docker.service]
```

Operator enters passphrase via serial console at every boot. Acceptable for interim; not acceptable for target baseline.

### Target on x86 with a TPM: TPM-sealed + FIDO2 break-glass

```yaml
luks_volumes:
  - name: docker-data
    device: /dev/nvme0n1
    mount_point: /opt/docker-data
    filesystem: ext4
    unlock_modes: [tpm2, fido2]
    systemd_requires: [docker.service]
luks_tpm2_pcrs: [0, 2, 4, 7, 11]
```

TPM silent auto-unseal at boot. FIDO2 slot available as break-glass. No human gesture routine.

### Phase 3: SSS threshold (tpm2 + tang + fido2)

```yaml
luks_volumes:
  - name: docker-data
    device: /dev/nvme0n1
    mount_point: /opt/docker-data
    filesystem: ext4
    unlock_modes: [sss, fido2]
luks_sss_threshold: 2
luks_sss_pins:
  tpm2:
    pcr_bank: sha256
    pcr_ids: "0,2,4,7,11"
  tang:
    url: https://tang.peer-site.netbird.legacy.example
```

Requires 2-of-{tpm2, tang} for routine unlock — box unlocks only on its own network (via tang) AND with untampered boot (via tpm2). FIDO2 as break-glass.

## Disabling a mode for a security review

Modes are toggled via two per-volume lists:

- `unlock_modes` — modes that should be **enrolled and active** at this volume.
- `unlock_modes_disabled` — modes that should be **actively wiped** from the LUKS header. Use when a security review or operational rotation pass decides a previously-enrolled mode should no longer be available.

Pattern (example — disabling FIDO2 on vega fleet after a security review concludes mode D's value-vs-attack-surface tradeoff has changed):

```yaml
luks_volumes_mgmt_host:
  - name: docker-data
    device: /dev/nvme0n1
    mount_point: /opt/docker-data
    filesystem: ext4
    unlock_modes: [passphrase, tpm2, tang_client]   # active set
    unlock_modes_disabled: [fido2]                  # actively wipe
    # ...
```

Re-running the role (`mgmt-host-luks.yml`) after the toggle:

1. Wipes the `fido2` LUKS keyslot via `systemd-cryptenroll --wipe-slot=fido2`.
2. Idempotent — no-op if the keyslot is already absent.
3. Per-task pre-flight asserts at least one OTHER unlock path remains. The role refuses to wipe ALL modes (slot 0 passphrase is the always-available recovery path; the role never wipes passphrase via this mechanism).

**Rollback** — if you regret the wipe:

1. Remove the mode from `unlock_modes_disabled`.
2. Add it back to `unlock_modes`.
3. Re-run the role; the regular enrollment path adds the keyslot back.
4. For modes that require interactive setup (FIDO2 with operator tap), the role pauses at the enrollment task as documented above.

**Audit trail** — every wipe writes a journal entry via `systemd-cryptenroll`. Boot-time observability comes from the existing systemd-cryptsetup events — a wiped slot won't appear in `cryptsetup luksDump` and won't be tried at boot.

**What's NOT covered by `unlock_modes_disabled`**:

- Passphrase slot 0 wipe — operator must use `cryptsetup luksRemoveKey` directly with explicit confirmation (intentional friction; passphrase is the recovery path).
- USB-keyfile wipe — variable not yet wired (the underlying `systemd-cryptenroll --wipe-slot=usb_keyfile` form doesn't exist; would need `cryptsetup luksKillSlot` with explicit slot number).

Supported wipe modes today: `fido2`, `tpm2`, `tang_client` (Clevis Tang binding).

## Host-site node-class paths (installer-LUKS enroll + encrypted-LV carve)

Two opt-in paths bring up a host-site node whose **root disk the OS installer already
encrypted** (Ubuntu guided full-disk encryption), so it boots unattended after a power cycle.
They are separate from `luks_volumes` (which formats a blank *second* disk): they never
format, they edit the installer's crypttab in place, and they are fail-closed. Both default to
`[]`; setting only `luks_volumes` is unaffected. Full design +
certification invariant: `docs/design/host-site-node-disk-encryption.md`.

> **NOT CERTIFIED until run on one real box + a cold boot.** This code governs disk encryption
> and the boot transaction; offline lint proves structure only. See the design doc §7.

### `luks_installer_partitions` — enroll an unattended-unlock mode onto the installer LUKS

Per entry, the role asserts the partition is `crypto_LUKS` and the expected VG is present
(**refuses** otherwise — no override), then, by `unlock_class`:

- **`tpm2`** (x86 with a TPM 2.0): binds a TPM2 keyslot with **Clevis**
  (`clevis luks bind ... tpm2`, additive, idempotent; PCR **7** by default, see
  `luks_clevis_tpm2_pcr_ids`) and installs `clevis-initramfs` so `clevis-luks-askpass`
  unlocks it in the initrd. Makes **no** crypttab edit — the installer's plain `luks` line
  is answered by clevis. NOT `systemd-cryptenroll`: Ubuntu's classic initramfs ignores a
  `systemd-cryptenroll` TPM2 keyslot (`ignoring unknown option 'tpm2-device'`). Boots
  unattended; a PCR 7 change falls back to the passphrase.
  (This is separate from the `luks_volumes` `tpm2` mode above,
  which is the post-boot data-disk path and still uses `systemd-cryptenroll`.)
- **`tang_initramfs`** (headless arm64 with **no** TPM, Raspberry Pi 5): binds the LUKS key to
  a same-site Tang server with Clevis, installs `clevis-initramfs`, configures initramfs static
  networking (`initramfs_ip` + `initramfs_kmods`), and adds `_netdev` so the unlock runs **in
  the initramfs**. This is distinct from `tang_client` (which unlocks a *data* disk POST-boot,
  F-285) and is **new to this repo**.

```yaml
luks_installer_partitions:
  - name: cryptroot
    device: /dev/nvme0n1p3
    expected_vg: ubuntu-vg
    unlock_class: tpm2
    passphrase: "{{ lookup('env', '<HOST>_LUKS_PASS') }}"   # existing installer slot-0
  # tang_initramfs adds: tang_servers[{name,url,class}], initramfs_ip, initramfs_kmods
```

### `luks_encrypted_lvs` — carve a data LV in the existing encrypted VG

On a single-NVMe host there is no second disk for Tier-3, so carve `docker-data` out of the
free space in the installer's encrypted VG. Encryption is inherited from the PV on the
full-disk LUKS — no new LUKS header, no new unlock mode. `mkfs` is guarded: it runs only on an
LV with no filesystem and **refuses** an LV carrying a mounted filesystem. Docker/containerd
`data-root` wiring stays with the `docker-engine` role group_vars; this only guarantees the
encrypted mount exists first.

```yaml
luks_encrypted_lvs:
  - name: docker-data
    vg: ubuntu-vg
    size_percent_free: "90"
    filesystem: ext4
    mount_point: /opt/docker-data
    data_root_subdirs: [/opt/docker-data/docker, /opt/docker-data/containerd]
```

## TODO

- Implement `tegra_se` mode via OP-TEE TA (Block 6D sub-task 3) — 2-4 week effort.
- Add `systemd-pcrlock` integration for automatic TPM re-seal on approved kernel updates.
- Add dm-verity root-partition setup (separate role or extension).
- Add first-boot Tier-3 unlock share consumption (offline enrollment bundle, Phase 3).
- Operational tooling: `./scripts/luks-rekey.sh` wrapper for periodic rotation.
