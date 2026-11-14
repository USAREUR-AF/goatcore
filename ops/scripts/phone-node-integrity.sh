#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# phone-node-integrity.sh — the integrity checks of a phone's Linux VM. Runs
# IN the VM, as the deploy user; ops/scripts/phone-node-provision.sh pipes it
# there over ssh. docs/design/phone-node.md, "How the host is made".
#
# Debian's kernel returned wrong file data in this VM until three boot
# settings were changed. A node is not built on a VM unless every check here
# passes:
#   1. the VM runs Debian's kernel, not the one Android supplies
#   2. the kernel has the namespaces containers need
#   3. the boot settings are on the kernel command line
#   4. three files hash the same across six drops of the page cache
#   5. 512 MiB written, synced and read back after a cache drop hash the same
#   6. no installed file fails `dpkg --verify` (configuration files aside)
#   7. the kernel log holds no filesystem or I/O error
#   8. the WireGuard module loads
#   9. a container runs
#
# usage (in the VM):
#   bash -s             < phone-node-integrity.sh   every check; exit 0 or 1
#   bash -s -- --prepared < phone-node-integrity.sh exit 0 when the VM is
#                                                   already prepared (checks
#                                                   1 and 3, the container
#                                                   engine, passwordless sudo)
#
# The first check that fails prints one line, `integrity: <what>`, and the
# script exits 1.
#
# For the hermetic test only: PHONE_NODE_GUEST_ROOT is put before every
# absolute path the checks read, and PHONE_NODE_GATE_MIB sets the size of the
# write in check 5.
set -euo pipefail

R="${PHONE_NODE_GUEST_ROOT:-}"
MIB="${PHONE_NODE_GATE_MIB:-512}"

no() { echo "integrity: $*"; exit 1; }
drop_caches() { sudo sh -c "echo 3 > '$R/proc/sys/vm/drop_caches'"; }
debian_kernel() { case "$(uname -r)" in *-cloud-arm64) return 0 ;; *) return 1 ;; esac; }
boot_settings() { grep -q 'arm64\.nosve' "$R/proc/cmdline"; }

if [ "${1:-}" = --prepared ]; then
  debian_kernel && boot_settings && command -v dockerd >/dev/null 2>&1 && sudo -n true 2>/dev/null
  exit $?
fi

# 1-3. the kernel
debian_kernel || no "still on the kernel Android supplies: $(uname -r)"
for ns in pid ipc user net mnt; do
  [ -e "$R/proc/self/ns/$ns" ] || no "the kernel has no $ns namespace"
done
boot_settings || no "the boot settings are not on the kernel command line (arm64.nosve)"

# 4. the same bytes on every read
n=$(for _ in 1 2 3 4 5 6; do
  drop_caches
  cat "$R/usr/lib/aarch64-linux-gnu/libc.so.6" "$R/usr/lib/aarch64-linux-gnu/libcrypto.so.3" "$R/usr/bin/ssh" | sha256sum
done | sort -u | wc -l | tr -d ' ')
[ "$n" = 1 ] || no "file reads differ between cache drops ($n distinct hashes)"

# 5. what is written is what is read back
gate="$R/var/tmp/.phone-node-gate"
head -c "$((MIB * 1048576))" /dev/urandom > "$gate"
a=$(sha256sum < "$gate")
sync
drop_caches
b=$(sha256sum < "$gate")
rm -f "$gate"
[ "$a" = "$b" ] || no "a $MIB MiB write does not read back the same"

# 6. installed files (a changed configuration file, marked `c`, is not a fault)
bad=$({ sudo dpkg --verify 2>/dev/null || true; } | { grep -v ' c /' || true; } | wc -l | tr -d ' ')
[ "$bad" = 0 ] || no "$bad installed files fail dpkg --verify"

# 7. the kernel log
if sudo dmesg | grep -E 'EXT4-fs error|I/O error' >/dev/null; then
  no "the kernel log reports filesystem or I/O errors"
fi

# 8-9. what a node needs of the kernel
sudo modprobe wireguard || no "the WireGuard kernel module does not load"
sudo docker run --rm hello-world 2>&1 | grep -q 'Hello from Docker' || no "a container does not run"

echo "integrity: ok. kernel $(uname -r); $(nproc) cores; $(free -m | awk '/Mem:/{print $2}') MiB RAM; $(df -h / | awk 'END{print $4}') free disk; namespaces, reads, write and read back, packages, kernel log, WireGuard, container run"
