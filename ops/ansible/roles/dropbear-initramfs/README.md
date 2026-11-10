# dropbear-initramfs

Block 65B — remote LUKS unlock from the initramfs via dropbear-ssh per ADR 0612. Closes the failure mode where a remote Orin power-cycles, fTPM/Tang both fail, and no operator is nearby — admin SSHes into the initramfs from a peer host on the same site's underlay LAN, types the LUKS passphrase, watches the box join the mesh.

## What it does

Given an Orin (or any Linux mgmt host with `dropbear-initramfs` packaged):

1. Installs `dropbear-initramfs`, `cryptsetup-initramfs`, `busybox-initramfs`.
2. **L4T NIC kmod inclusion** (Orin only) — for AGX Orin on JetPack 6.1 the Tegra Ethernet driver is `nvethernet` (NOT `tegra_eqos`; verified on an AGX Orin running kernel 5.15.148-tegra). It is an out-of-tree NVIDIA module shipped at `/lib/modules/$KVER/updates/drivers/net/ethernet/nvidia/nvethernet/nvethernet.ko` and **`MODULES=most` in `/etc/initramfs-tools/initramfs.conf` correctly includes it in the initramfs by default** — no entry in `/etc/initramfs-tools/modules` is required for the LAN NIC. `dropbear_initramfs_extra_kmods` is therefore left empty for AGX Orin on JetPack 6.x; verify via `lsmod | grep -i ether` + `lsinitramfs /boot/initrd | grep nvethernet` on the running kernel before deploying to a new L4T release.
3. Renders `/etc/dropbear/initramfs/authorized_keys` from the `dropbear_authorized_keys` variable (operator pubkeys delivered via the offline-enrollment bundle per Block 65B.2 in the future; explicit variable today).
4. Renders `/etc/dropbear/initramfs/dropbear.conf` with `DROPBEAR_OPTIONS` constrained to the underlay LAN interface only (no public bind).
5. Configures `/etc/initramfs-tools/initramfs.conf` `IP=` setting to use the underlay LAN's static address from `dropbear_initramfs_ip_config` (DHCP is rejected because a CGNAT uplink can't be relied on for a stable initramfs IP).
6. Triggers `update-initramfs -u` to bake the changes into the initramfs. **L4T-specific note:** the bootloader is U-Boot extlinux and reads `/boot/initrd` (no `-$KVER` suffix; configured by `INITRD /boot/initrd` in `/boot/extlinux/extlinux.conf`). The Tegra `flash-kernel` post-update hook at `/etc/initramfs/post-update.d/flash-kernel` handles the copy from initramfs-tools' default output to `/boot/initrd` automatically — no manual copy step is required.

## What it deliberately does NOT do

- **No WireGuard in initramfs.** The reachability story is "remote admin → wg-cp0 from anywhere → in-site peer host underlay LAN → dropbear on locked Orin." This avoids substantial initramfs-build complexity for a marginal reachability improvement. Revisit trigger: a real recovery scenario where the in-site peer is also down. See ADR 0612 §"Alternatives considered."
- **No public listener.** dropbear binds the underlay LAN interface only. The silent-control-plane invariant ([ADR 0313](../../../../docs/adr/0313-silent-control-plane-outer-wg-wrap.md)) holds in initramfs.
- **No password authentication.** Public-key only; the role asserts `DROPBEAR_OPTIONS` includes `-s` (disable password auth) and `-g` (disable empty-password root login).
- **No implicit forced command.** The role-default `DROPBEAR_OPTIONS` does not
  set `-c`, and the role copies each supplied authorized-key line unchanged.
  With a plain key, SSH therefore opens an initramfs shell. An automatic unlock
  prompt exists only when configuration explicitly supplies Dropbear
  `-c /bin/cryptroot-unlock` or an authorized-key
  `command="/bin/cryptroot-unlock"` restriction.

## Variables

| Variable | Default | Description |
|---|---|---|
| `dropbear_authorized_keys` | `[]` | List of operator SSH pubkey strings authorized to unlock this host via initramfs. Per the offline-enrollment bundle path (Block 65B.2), this will be populated from the bundle's `dropbear_authorized_keys` CBOR field at first-boot. Today: explicit variable in group_vars. |
| `dropbear_initramfs_ip_config` | `""` (empty — operator must set) | initramfs `IP=` config line. Format: `<client-ip>::<gateway-ip>:<netmask>:<hostname>:<device>:<autoconf>` per [`linux/Documentation/admin-guide/nfs/nfsroot.rst`](https://docs.kernel.org/admin-guide/nfs/nfsroot.html). Example: `192.168.1.10::192.168.1.1:255.255.255.0:vega-A:eth0:off`. DHCP is intentionally not the default. |
| `dropbear_initramfs_ssh_port` | `22` | Port for dropbear in initramfs. Distinct from the running-OS sshd port (which can be different). |
| `dropbear_initramfs_extra_kmods` | `[]` | Extra kernel modules to include in initramfs. **Empty by default for AGX Orin on JetPack 6.x** — `MODULES=most` already includes `nvethernet` (verified 65B.0). Use only when adding hardware whose driver is excluded by the `MODULES=most` heuristic. Verify with `lsinitramfs /boot/initrd \| grep <module>` after `update-initramfs -u`. |
| `dropbear_options` | `-s -g -j -k -I 300 -p {{ dropbear_initramfs_ssh_port }}` | Dropbear options rendered unchanged. The default disables password and empty-password root login, but does not force `cryptroot-unlock`. |

## Example

```yaml
# group_vars/vega_hosts/vars.yml
dropbear_authorized_keys:
  - "ssh-ed25519 AAAA... operator-1@admin"
  - "ssh-ed25519 AAAA... operator-2@admin"
dropbear_initramfs_ip_config: "{{ underlay_lan_ip }}::{{ underlay_gateway }}:{{ underlay_netmask }}:{{ inventory_hostname }}:eno1:off"
# dropbear_initramfs_extra_kmods left empty — `MODULES=most` includes
# `nvethernet` in the L4T initramfs by default (verified Block 65B.0).
```

```yaml
# vega-bootstrap.yml (or refresh)
- hosts: vega_hosts
  roles:
    - role: dropbear-initramfs
```

## Operational use

After a power cycle, if the Orin is hung at the LUKS prompt, first confirm the
declared initramfs address, port, operator key, and effective profile. Verify
the SSH host key by the site's approved out-of-band method before accepting the
connection or entering a passphrase.

For the role-default profile with a plain authorized key, explicitly invoke
the installed helper:

```bash
# From a peer host on the same site's underlay LAN
ssh -t -p 22 -o StrictHostKeyChecking=ask \
  root@<orin-underlay-ip> cryptroot-unlock
```

If the declared `DROPBEAR_OPTIONS` contains
`-c /bin/cryptroot-unlock`, or the enrolled key explicitly contains
`command="/bin/cryptroot-unlock"`, connect without a remote command; the
declared forced command invokes the helper:

```bash
ssh -p 22 -o StrictHostKeyChecking=ask root@<orin-underlay-ip>
```

In either profile, enter the approved passphrase once. A successful unlock
ends the SSH session as boot continues. If authentication, routing, host-key
verification, or the passphrase fails, stop rather than changing keys or
rebuilding the initramfs during the recovery attempt.

Full recovery walkthrough:
[`site-node-boot-recovery.md`](../../../../docs/operations/site-node-boot-recovery.md).

## Caveats and trigger-to-revisit

- **Initramfs is a smaller threat surface but not a zero one.** A compromised dropbear binary in initramfs would have keystroke-level access to the LUKS passphrase. Mitigated by: pinned-digest package install, supply-chain attestation per Block 50, Block 38 chaos probe to verify expected initramfs hash.
- **Static IP in initramfs is a per-site config.** When site networking changes, this role must be re-run + initramfs rebuilt. The smoke-test fixture exercises the rebuild path.
- **Tegra kmod name may vary by JetPack version.** 65B.0 verifies the right name on the target host. If JetPack ships a different driver in a future release, update `dropbear_initramfs_extra_kmods` accordingly.

## References

- ADR 0612 — multi-modal LUKS unlock continuity-priority posture
- Block 65B — dropbear-initramfs implementation tracking
- [`dropbear-initramfs(8)`](https://manpages.ubuntu.com/manpages/jammy/man8/dropbear.8.html)
- [`initramfs.conf(5)`](https://manpages.ubuntu.com/manpages/jammy/man5/initramfs.conf.5.html)
