#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# phone-node-guest-prepare.sh — make a phone's Linux VM a host a goat node can
# run on. Runs IN the VM, as its first user (`droid`);
# ops/scripts/phone-node-provision.sh pipes it there over ssh, with
#   USER_NAME   the deploy user to create
#   HOST_NAME   the hostname to set
#   PUBKEY      the public key the deploy user accepts
#   MEMORY_MIB  the VM's memory in MiB, or empty to leave it
# docs/design/phone-node.md, "How the host is made".
#
# Every step is safe to repeat:
#   1. install Debian's own arm64 kernel and copy it over the kernel the
#      Terminal app boots (the kernel Android supplies has no PID or IPC
#      namespaces, so no container starts on it); the files Android shipped
#      are kept once, in ~/orig-boot
#   2. boot settings: SVE off, the memory balloon off, no disk discard
#      (Debian's kernel returned wrong file data in this VM with all three
#      left on; which one is the cause is not measured)
#   3. a kernel package upgrade copies the new kernel to the same place
#   4. Docker Engine from Docker's Debian repository
#   5. the deploy user: key login, passwordless sudo; the hostname
#
# The new kernel boots at the next start of the VM; the caller restarts it.
#
# For the hermetic test only: PHONE_NODE_GUEST_ROOT is put before every
# absolute path this script reads or writes.
#
# The script arrives on standard input (bash -s), so every command that could
# read standard input is given /dev/null.
set -euo pipefail

: "${USER_NAME:?USER_NAME is not set}" "${HOST_NAME:?HOST_NAME is not set}" "${PUBKEY:?PUBKEY is not set}"
MEMORY_MIB="${MEMORY_MIB:-}"
R="${PHONE_NODE_GUEST_ROOT:-}"
L="$R/mnt/internal/linux"

sudo test -w "$L/vm_config.json" || { echo "the Terminal app's boot folder is not writable from the VM at $L"; exit 1; }

# 1. the kernel. Wait out the apt run the VM starts at boot.
for _ in $(seq 1 60); do
  sudo fuser "$R/var/lib/dpkg/lock-frontend" >/dev/null 2>&1 || break
  sleep 5
done
sudo dpkg --configure -a </dev/null
sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq linux-image-cloud-arm64 </dev/null >/dev/null
V=$(find "$R/boot" -maxdepth 1 -name 'vmlinuz-*-cloud-arm64' | sort -V | tail -1)
[ -n "$V" ] || { echo "linux-image-cloud-arm64 installed no kernel under $R/boot"; exit 1; }
V=${V##*/vmlinuz-}
mkdir -p ~/orig-boot
if [ ! -f ~/orig-boot/vmlinuz ]; then
  for f in vmlinuz initrd.img vm_config.json build_id; do
    if sudo test -f "$L/$f"; then sudo cp "$L/$f" ~/orig-boot/; fi
  done
fi
if [ "$(uname -r)" != "$V" ]; then
  sudo cp "$R/boot/vmlinuz-$V" "$L/vmlinuz"
  sudo cp "$R/boot/initrd.img-$V" "$L/initrd.img"
fi

# 2. boot settings
sudo python3 - "$L/vm_config.json" "$R/etc/fstab" "$MEMORY_MIB" <<'PY'
import json, os, re, sys

config, fstab, memory = sys.argv[1:4]
c = json.load(open(config))
before = json.dumps(c, sort_keys=True)
want = ["arm64.nosve", "arm64.nosme", "modprobe.blacklist=virtio_balloon"]
params = c.get("params", "").split()
c["params"] = " ".join(params + [w for w in want if w not in params])
c["auto_memory_balloon"] = False
if memory:
    c["memory_mib"] = int(memory)
if json.dumps(c, sort_keys=True) != before:
    json.dump(c, open(config, "w"), indent=4)

if os.path.exists(fstab):
    text = open(fstab).read()
    new = re.sub(r",discard\b|\bdiscard,", "", text)
    if new != text:
        open(fstab, "w").write(new)
PY
sudo systemctl mask storage-balloon-agent.service >/dev/null 2>&1 || true

# 3. a kernel package upgrade also replaces the files the Terminal app boots
sudo mkdir -p "$R/etc/kernel/postinst.d"
sudo tee "$R/etc/kernel/postinst.d/zz-phone-node" >/dev/null <<'HOOK'
#!/bin/sh
# Copy a newly installed kernel and initrd to the folder the Android Terminal
# app boots the VM from.
set -e
L=/mnt/internal/linux
case "$1" in *-cloud-arm64) ;; *) exit 0 ;; esac
[ -w "$L/vm_config.json" ] || exit 0
[ -f "/boot/initrd.img-$1" ] || exit 0
cp "/boot/vmlinuz-$1" "$L/vmlinuz"
cp "/boot/initrd.img-$1" "$L/initrd.img"
HOOK
sudo chmod 755 "$R/etc/kernel/postinst.d/zz-phone-node"

# 4. Docker Engine (the node's services are containers; the integrity checks run one)
if ! command -v dockerd >/dev/null 2>&1; then
  sudo mkdir -p "$R/etc/apt/keyrings" "$R/etc/apt/sources.list.d"
  sudo chmod 0755 "$R/etc/apt/keyrings"
  sudo rm -f "$R/etc/apt/sources.list.d/docker.list"
  curl -fsSL https://download.docker.com/linux/debian/gpg | sudo gpg --dearmor --yes -o "$R/etc/apt/keyrings/docker.gpg"
  sudo chmod a+r "$R/etc/apt/keyrings/docker.gpg"
  codename=$(. "$R/etc/os-release"; echo "$VERSION_CODENAME")
  echo "deb [arch=arm64 signed-by=/etc/apt/keyrings/docker.gpg] https://download.docker.com/linux/debian $codename stable" \
    | sudo tee "$R/etc/apt/sources.list.d/docker.list" >/dev/null
  sudo apt-get update -qq </dev/null
  sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq docker-ce docker-ce-cli containerd.io docker-compose-plugin </dev/null >/dev/null
fi

# 5. the deploy user and the hostname
id "$USER_NAME" >/dev/null 2>&1 || sudo useradd -m -s /bin/bash -G sudo "$USER_NAME"
sudo mkdir -p "$R/etc/sudoers.d"
echo "$USER_NAME ALL=(ALL) NOPASSWD:ALL" | sudo tee "$R/etc/sudoers.d/90-$USER_NAME" >/dev/null
sudo chmod 440 "$R/etc/sudoers.d/90-$USER_NAME"
sudo mkdir -p "$R/home/$USER_NAME/.ssh"
echo "$PUBKEY" | sudo tee "$R/home/$USER_NAME/.ssh/authorized_keys" >/dev/null
sudo chmod 700 "$R/home/$USER_NAME/.ssh"
sudo chmod 600 "$R/home/$USER_NAME/.ssh/authorized_keys"
sudo chown -R "$USER_NAME:$USER_NAME" "$R/home/$USER_NAME/.ssh"
sudo hostnamectl set-hostname "$HOST_NAME"
grep -q "127.0.1.1 $HOST_NAME" "$R/etc/hosts" 2>/dev/null || echo "127.0.1.1 $HOST_NAME" | sudo tee -a "$R/etc/hosts" >/dev/null

echo "kernel staged: $V"
