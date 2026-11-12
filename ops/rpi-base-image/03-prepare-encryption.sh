#!/usr/bin/env bash
set -euo pipefail

# =============================================================
# 03-prepare-encryption.sh
# Prepares passphrase-based LUKS encryption before optional YubiKey
# HMAC challenge-response enrollment in 04-enroll-yubikey.sh.
# Run ON THE PI before rebooting into initramfs.
#
# This console path does not install dropbear-initramfs and removes any
# existing IP= line from initramfs.conf. For headless remote unlock, run
# 03b-headless-dropbear-unlock.sh after this script.
#
# Workflow:
#   1. Run this script (configures fstab, cmdline.txt, initramfs)
#   2. sudo reboot                  → Pi drops to (initramfs)
#   3. Run encrypt-rootfs.sh — it'll prompt for a passphrase
#   4. Pi auto-reboots → drops to (initramfs) again
#   5. Manually open with passphrase, exit → Pi boots Ubuntu
#   6. Run 04-enroll-yubikey.sh → future boots use HMAC challenge-response
#
# Usage: sudo bash 03-prepare-encryption.sh
# =============================================================

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[0;33m'
BLUE='\033[0;34m'; CYAN='\033[0;36m'; WHITE='\033[1;37m'
BOLD='\033[1m'; NC='\033[0m'

info()  { printf "${CYAN}>${NC} %s\n" "$*"; }
step()  { printf "\n${BOLD}${BLUE}=== %s ===${NC}\n" "$*"; }
ok()    { printf "${GREEN}[ok]${NC} %s\n" "$*"; }
warn()  { printf "${YELLOW}[warn]${NC} %s\n" "$*" >&2; }
error() { printf "${RED}[error]${NC} %s\n" "$*" >&2; }

banner() {
    echo ""
    printf "${BLUE}${BOLD}  ┌──────────────────────────────────┐${NC}\n"
    printf "${BLUE}${BOLD}  │  ${WHITE}ODT ${NC}${BLUE}${BOLD}│${NC} ${GREEN}%-24s${BLUE}${BOLD}  │${NC}\n" "$1"
    printf "${BLUE}${BOLD}  └──────────────────────────────────┘${NC}\n\n"
}

if [ "$(id -u)" -ne 0 ]; then
    error "Run as root (sudo bash 03-prepare-encryption.sh)"
    exit 1
fi

banner "YubiKey Encryption Prep"

step "Phase 1: Install cryptsetup + initramfs hooks"
info "Installing packages..."
apt-get update -qq
apt-get install -y -qq cryptsetup busybox

info "Configuring kernel modules in initramfs..."
sed -i 's/^MODULES=dep/MODULES=most/' /etc/initramfs-tools/initramfs.conf
# Strip any leftover IP= line from a previous USB-path build
sed -i '/^IP=/d' /etc/initramfs-tools/initramfs.conf

MODFILE="/etc/initramfs-tools/modules"
REQUIRED_MODS=(algif_skcipher aes_arm64 aes_ce_blk aes_ce_ccm aes_ce_cipher sha256_arm64 cbc dm-crypt)
for mod in "${REQUIRED_MODS[@]}"; do
    grep -q "^${mod}$" "$MODFILE" 2>/dev/null || echo "$mod" >> "$MODFILE"
done
ok "Kernel modules added"

info "Creating luks_hooks initramfs hook..."
cat > /etc/initramfs-tools/hooks/luks_hooks <<'HOOKEOF'
#!/bin/sh -e
PREREQS=""
case $1 in
  prereqs) echo "${PREREQS}"; exit 0;;
esac
. /usr/share/initramfs-tools/hook-functions
copy_exec /sbin/e2fsck /sbin
copy_exec /sbin/resize2fs /sbin
copy_exec /sbin/fdisk /sbin
copy_exec /sbin/cryptsetup /sbin
HOOKEOF
chmod +x /etc/initramfs-tools/hooks/luks_hooks

# Ensure cryptsetup is included in initramfs
if ! grep -q '^CRYPTSETUP=y' /etc/initramfs-tools/initramfs.conf; then
    echo 'CRYPTSETUP=y' >> /etc/initramfs-tools/initramfs.conf
fi
ok "luks_hooks created"

step "Phase 2: Detect root device + update fstab/cmdline.txt"

ROOT_SOURCE=$(findmnt -n -o SOURCE /)
if [[ "$ROOT_SOURCE" == *"nvme"* ]]; then
    ROOT_PART="/dev/nvme0n1p2"
    BOOT_PART="/dev/nvme0n1p1"
    ROOT_DEVICE="NVMe"
else
    ROOT_PART="/dev/mmcblk0p2"
    BOOT_PART="/dev/mmcblk0p1"
    ROOT_DEVICE="SD card"
fi
ok "Root device: ${ROOT_DEVICE} (${ROOT_PART})"

info "Updating /etc/fstab..."
if grep -q 'LABEL=writable' /etc/fstab; then
    sed -i 's|LABEL=writable.*|/dev/mapper/rootfs / ext4 defaults 0 1|' /etc/fstab
elif grep -qP 'UUID=\S+\s+/\s' /etc/fstab; then
    sed -i 's|UUID=\S\+\s\+/.*|/dev/mapper/rootfs / ext4 defaults 0 1|' /etc/fstab
else
    warn "Could not find root mount in /etc/fstab — adding manually"
    echo "/dev/mapper/rootfs / ext4 defaults 0 1" >> /etc/fstab
fi
ok "fstab updated"

info "Updating cmdline.txt..."
CMDLINE="/boot/firmware/cmdline.txt"
[ -f "$CMDLINE" ] || CMDLINE="/boot/cmdline.txt"

sed -i 's|root=LABEL=writable|root=/dev/mapper/rootfs|' "$CMDLINE"
sed -i 's|root=UUID=[^ ]*|root=/dev/mapper/rootfs|' "$CMDLINE"
sed -i 's| splash||' "$CMDLINE"
if ! grep -q 'cryptdevice=' "$CMDLINE"; then
    sed -i "s|$| cryptdevice=${ROOT_PART}:rootfs|" "$CMDLINE"
fi
ok "cmdline.txt updated"

step "Phase 3: Rebuild initramfs"
update-initramfs -u
ok "initramfs rebuilt"

printf "\n${GREEN}${BOLD}=========================================${NC}\n"
printf "${GREEN}${BOLD}  Preparation complete.${NC}\n"
echo ""
echo "  Next steps (HDMI console — WiFi won't work in initramfs):"
echo ""
printf "    1. ${CYAN}sudo reboot${NC}\n"
echo "    2. At (initramfs), run:"
printf "       ${CYAN}mkdir -p /mnt/boot && mount -t vfat ${BOOT_PART} /mnt/boot${NC}\n"
printf "       ${CYAN}sh /mnt/boot/encrypt-rootfs.sh${NC}\n"
echo "    3. encrypt-rootfs.sh prompts for a passphrase (use a strong one"
echo "       — you only need it for the build window; YubiKey replaces it)"
echo "    4. ~40 min encryption, then auto-reboot"
echo "    5. At (initramfs) again, manually unlock with the passphrase:"
printf "       ${CYAN}cryptsetup luksOpen ${ROOT_PART} rootfs${NC}\n"
printf "       ${CYAN}exit${NC}\n"
echo "    6. Pi boots Ubuntu. Then enroll YubiKey:"
printf "       ${CYAN}sudo bash /boot/firmware/04-enroll-yubikey.sh${NC}\n"
printf "${GREEN}${BOLD}=========================================${NC}\n"
