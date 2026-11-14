# A goat node on a phone

| Field | Value |
|---|---|
| Status | **WIP, not for production use.** Measured once, on one phone (a Google Pixel 10a, Android 16), 2026-11-13: the phone joined a standing goatnet as a `core` joiner through `goatbuilder join-node` 0.5.1 and replicates with the founder in both directions. That run needed steps outside the tool; "What the run needed outside the tool" lists them. Gate that lifts the WIP line: group H of [goatbuilder-v1-gate.md](goatbuilder-v1-gate.md) |
| Proves | Gate rows H1 to H8 |
| Design | [goatbuilder.md](goatbuilder.md) · [tutorial 2](../tutorials/02-first-federated-node.md) (the sequence `join-node` runs) · [service-criticality-tiers.md](service-criticality-tiers.md) (the tier ceiling) · [goatbuilder-hardware-measurement.md](goatbuilder-hardware-measurement.md) |
| Audience | whoever builds the phone class into `goatbuilder`, and whoever decides what a phone node carries |

## What it is

A phone node is a site node of a goatnet that runs on an Android phone: light,
carried in a pocket, with its own battery and radios. It is built the way a
node on a small computer is built, with `goatbuilder join-node`, and runs the
same containers from the same plays.

The phone keeps Android. Android 16 on a recent Pixel includes a hypervisor and
a Debian virtual machine (the "Linux Terminal" app). The node is that virtual
machine, with one change: it boots Debian's own kernel in place of the kernel
Android supplies. No root, no bootloader unlock and no wipe of the phone is
involved.

## How the host is made

The kernel Android supplies for its Debian VM cannot run containers. The VM's
boot files are writable from inside the VM, so a provisioning step replaces the
kernel and the next start boots it.

| Fact | Status | Detail |
|---|---|---|
| The supplied guest kernel has no PID, IPC or user namespaces | Measured | `6.12.60-android16`; the Docker daemon exits at start |
| The Terminal app shares its boot folder into the guest | Measured | `/mnt/internal/linux/` holds `vmlinuz`, `initrd.img` and `vm_config.json`; root in the guest can write them |
| Debian's `linux-image-cloud-arm64`, copied over those two files, boots on the next VM start with the app's networking unchanged | Measured | `6.12.116+deb13-cloud-arm64` |
| With default settings that kernel returns wrong file data in this VM | Measured, one run | reads differed between cache drops, binaries crashed, then ext4 I/O errors |
| With SVE off, the memory balloon off and disk discard off it is stable | Measured | `arm64.nosve arm64.nosme modprobe.blacklist=virtio_balloon` on the kernel command line, `auto_memory_balloon: false`, no `discard` mount option: identical hashes across six cache drops, a 1.5 GB write and read back, `dpkg --verify` clean, and clean again after a hard kill of the VM |
| Which of the three settings is the necessary one | Unmeasured | a failed try costs a VM reset |
| Docker Engine and kernel WireGuard work on that kernel | Measured | bridge networks, published ports, outbound TLS |
| The VM | Measured | Debian 13, 8 vCPU, 3,923 MiB RAM (`memory_mib` in `vm_config.json`), about 97 GB free on one ext4 root |
| A kernel package upgrade does not change what boots | Measured | the booted kernel is the file in the app's folder; a `postinst` hook copies each new kernel there |

Three other routes were tried and do not work on this phone: Linux on the bare
phone (no port for its processor), containers on the supplied kernel (above),
and a custom VM started with Android's `vm` tool from the debug shell (it boots
and gets no network device).

## How it is reached

The VM sits behind the phone's own address translation. It makes outbound
connections only, has no address another machine on the LAN can reach, and its
private address changes between boots.

| Stage | Path | Status |
|---|---|---|
| Before enrolment | SSH carried over the Android debug bridge on a USB cable (`adb shell nc <vm address> 22` as the SSH proxy command) | Measured: Ansible ran the host baseline through it |
| After enrolment | The node's mesh address, like every other node | Measured: the three-pass join and the fabric plays ran over the mesh |
| On the mesh | Relayed connections, not direct ones | Measured: about 50 ms from the founder |

A phone node therefore takes no public ingress. It registers no tunnel with the
relays, has no public name, and serves nothing except on the mesh.

## What it runs, and what it costs

The first phone ran the full `core` joiner set at tier ceiling 1: Postgres with
Spock, the NetBird management and signal services, Traefik, Kratos, Hydra, the
admission audit, its own Zenoh router and the node-scoped health publishers.

| Fact | Status | Detail |
|---|---|---|
| Memory with the core set running, before the router and publishers | Measured | 1,134 MiB used of 3,923 MiB |
| The same set on a 16 GB joiner at idle | Measured | about 465 MiB of containers |
| Container limits that have a variable were set for a 4 GB host | Built in the site's group_vars | router 1G, Traefik 512M, management 384M, signal 256M |
| Limits that had no variable exceeded the host | Measured; variables built | Postgres 2G, the fabric crawler 3,968M; Postgres `shared_buffers` 768MB and `max_connections` 300 in the image. Each is now a variable with that value as its default: `mgmt_postgres_memory`, `fabric_crawler_memory_limit`, `mgmt_postgres_shared_buffers`, `mgmt_postgres_max_connections` |

**Open decision: what a phone node replicates.** A full joiner holds a copy of
the goatnet's control-plane databases, and the founder holds a replication slot
for it. A phone that is off for days falls behind the founder's WAL limit and
must be seeded again. The alternative is a phone profile that is a mesh peer
with its own router and local services and is not a replication member. The
first phone is a full member, to prove the standard path; which of the two a
phone node is by default is not decided.

## Staying up

| Fact | Status |
|---|---|
| The VM keeps running with the screen off, on power | Measured, about two hours |
| An app-store update of Android's WebView stops the Terminal app, and the VM with it | Measured. The app store is switched off on a node phone |
| After a phone reboot the VM does not start until the phone is unlocked and the Terminal app is opened | Measured |
| A computer on the USB cable restarts the VM when it finds none running | Built as a two-minute cron job outside this repository; it cannot pass the lock screen after a phone reboot |
| Hours on battery, Doze, battery saver, heat under load, a mobile-data uplink | Unmeasured |
| A launcher on the phone that starts the VM at boot (`ops/phone/vm-launcher`) | Built. Measured once, 2026-11-14: after a reboot and one unlock the app opened the Terminal app 14 s later and the VM was running 20 s after the unlock, with no computer acting. Before the first unlock nothing starts; a phone with no lock screen is unmeasured |

## What the run needed outside the tool

Each row is a place the first run left `goatbuilder join-node`, and the change
that removes it. Rows marked "every class" are not about phones.

| # | What happened | Applies to | Change | Gate row |
|---|---|---|---|---|
| 1 | Phases 3 and 7 gate on a config check that refuses the mesh-address placeholder, which is filled only in phase 12 | every class | the two gates accept that one placeholder before enrolment | H1 |
| 2 | Pass 1 of phase 13 ends with a check that the management service is up; that service waits for Hydra, which the next step deploys | every class | pass 1 runs the stack plays only, as `standup` already does with its play tags | H1 |
| 3 | Phase 13's signal-federation gate treats federation as a pair and refuses a third site that declares it off | every class | a site that declares federation off, with its reason, passes | H1 |
| 4 | Phase 9 passed the setup key's secret reference to the node unresolved | every class | the phase resolves references as the plays do | H2 |
| 5 | Phases 8 and 10 are manual: mint a setup key, enrol the node | every class | the tool mints the key into the declared secret store and enrols the node from a key file | H2 |
| 6 | The phone was prepared by a script run before `join-node`, and reached through a hand-written SSH alias | phone | `--via usb`, and a first phase that prepares the VM and refuses unless the integrity checks pass. Built on the fixture: the run writes the node's SSH configuration itself (`ops/scripts/phone-node-ssh.sh`), and phase 2 prepares the VM and runs the integrity checks (`ops/scripts/phone-node-provision.sh`). Unmeasured on hardware | H3, H4 |
| 7 | The site's config repeated, in comments and by absence, that the node has no LAN address, no public ingress, no disk encryption of its own and 4 GB of memory | phone | a machine class `phone` that carries those declarations. Built on the fixture: `ops/ansible/group_vars/class_phone.yml`, checked by the config preflight with `--class phone`. Unmeasured on hardware | H5 |
| 8 | Docker's apt repository was written for Ubuntu; the minimal Debian image has no `cron` | every Debian host | the repository follows the distribution (merged); every role that writes a crontab entry installs `cron` first (built) | H6 |
| 9 | Postgres memory and several container limits could not be set for the host | every small host | variables with the present values as defaults (built) | H6 |

The target is one command, with the phone on a USB cable:

```
goatbuilder join-node <environment> --site <site> --node <node> --class phone --via usb
```

Two things stay outside the tool. Allowing USB debugging is done by hand on
each new phone. Keeping the VM running across a phone reboot needs the launcher
above, or a computer on the cable.
