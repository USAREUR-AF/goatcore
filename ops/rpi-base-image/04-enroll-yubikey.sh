#!/usr/bin/env bash
set -euo pipefail

# =============================================================
# 04-enroll-yubikey.sh
# Enroll a YubiKey for tap-to-boot LUKS unlock using HMAC-SHA1
# challenge-response (slot 2). This is the YubiKey path that
# actually works on Ubuntu 24.04's stock cryptsetup-initramfs —
# unlike systemd-cryptenroll FIDO2, which the initramfs cryptroot
# script doesn't speak.
#
# Workflow at boot:
#   1. cryptroot reads /etc/crypttab, finds keyscript=yubikey_unlock
#   2. keyscript calls ykchalresp -2 with a fixed challenge
#   3. YubiKey blinks; user touches it
#   4. HMAC response (40 hex chars) becomes the LUKS unlock key
#   5. Pi continues boot
#
# Security model: possession of YubiKey = boot. Anyone with the
# YubiKey can unlock (no PIN, but touch is required). The HMAC
# secret never leaves the YubiKey. Original passphrase remains
# in slot 0 as recovery.
#
# Run AFTER 03-prepare-encryption.sh + encrypt-rootfs.sh, once the
# encrypted rootfs is unlocked with its recovery passphrase.
#
# Usage: sudo bash 04-enroll-yubikey.sh
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

[ "$(id -u)" -eq 0 ] || { error "Run as root (sudo bash 04-enroll-yubikey.sh)"; exit 1; }

banner "YubiKey HMAC Enrollment"

CHALLENGE="${CHALLENGE:-luks-rootfs-unlock-challenge-v1}"
ROOT_PART="${ROOT_PART:-/dev/nvme0n1p2}"

# Pre-flight
cryptsetup status rootfs >/dev/null || { error "rootfs mapper inactive — run encrypt-rootfs.sh first"; exit 1; }
if cryptsetup luksDump "$ROOT_PART" 2>/dev/null | grep -q 'online-reencrypt'; then
    error "LUKS reencryption incomplete. Resume from initramfs first."
    exit 1
fi
ok "rootfs is unlocked + reencryption complete"

step "Step 1/6: Install yubikey-personalization"
apt-get update -qq
apt-get install -y -qq yubikey-personalization

step "Step 2/6: Detect YubiKey"
ykinfo -a 2>&1 | grep -E "^(serial|version|slot[12]_status):" || { error "YubiKey not detected"; exit 1; }

step "Step 3/6: Program YubiKey slot 2 (HMAC-SHA1 + button-trigger)"
warn "This OVERWRITES whatever was in YubiKey slot 2."
warn "Slot 1 (static password / OTP) is untouched."
read -rp "Press Enter to continue, or Ctrl+C to abort..."

echo y | ykpersonalize -2 -ochal-resp -ochal-hmac -ohmac-lt64 \
                       -ochal-btn-trig -oserial-api-visible
ok "Slot 2 programmed"

step "Step 4/6: Generate HMAC response (TOUCH YUBIKEY when it blinks)"
echo ""
printf "${YELLOW}You have ~15 seconds. Watch the YubiKey gold disc, touch when it pulses.${NC}\n"
for i in 5 4 3 2 1; do printf "  %d...\n" "$i"; sleep 1; done
printf "${BOLD}>>> CHALLENGE SENT — TOUCH NOW <<<${NC}\n"

mkdir -p /run/yk-enroll && chmod 700 /run/yk-enroll
if ! ykchalresp -2 "$CHALLENGE" > /run/yk-enroll/response 2>/run/yk-enroll/err; then
    error "ykchalresp failed: $(cat /run/yk-enroll/err)"
    rm -rf /run/yk-enroll
    exit 1
fi
[ -s /run/yk-enroll/response ] || { error "Empty response"; exit 1; }
ok "Got $(wc -c < /run/yk-enroll/response)-byte response"

step "Step 5/6: Add response as LUKS slot (FIPS-friendly pbkdf2)"
echo ""
printf "${YELLOW}Enter the EXISTING LUKS passphrase to authenticate the new key:${NC}\n"
cryptsetup luksAddKey "$ROOT_PART" /run/yk-enroll/response \
    --pbkdf pbkdf2 --pbkdf-force-iterations 600000

# Verify
if ! cryptsetup --test-passphrase open --type luks2 "$ROOT_PART" --key-file=/run/yk-enroll/response 2>/dev/null; then
    error "YubiKey response does NOT unlock — aborting before changing crypttab"
    rm -rf /run/yk-enroll
    exit 1
fi
ok "YubiKey response verified working as LUKS key"
rm -rf /run/yk-enroll

step "Step 6/6: Install initramfs keyscript + hook + update crypttab"

# Keyscript — runs at boot, outputs HMAC response as LUKS unlock key
cat > /lib/cryptsetup/scripts/yubikey_unlock <<KEYSCRIPT
#!/bin/sh
# Sends fixed challenge to YubiKey slot 2 (HMAC-SHA1 + button-trigger).
# stdout (HMAC response, hex) becomes the LUKS unlock key.
exec /usr/bin/ykchalresp -2 "$CHALLENGE"
KEYSCRIPT
chmod +x /lib/cryptsetup/scripts/yubikey_unlock
ok "Wrote /lib/cryptsetup/scripts/yubikey_unlock"

# Initramfs hook — copies ykchalresp + libusb + libykpers into initrd
cat > /etc/initramfs-tools/hooks/yubikey <<'HOOK'
#!/bin/sh -e
PREREQS=""
case $1 in
  prereqs) echo "${PREREQS}"; exit 0;;
esac

. /usr/share/initramfs-tools/hook-functions

# Find the right multiarch lib dir (aarch64-linux-gnu on Pi)
LIBDIR="/lib/$(dpkg-architecture -qDEB_HOST_MULTIARCH 2>/dev/null || echo aarch64-linux-gnu)"

copy_exec /usr/bin/ykchalresp /usr/bin
copy_exec "$LIBDIR/libykpers-1.so.1" "$LIBDIR/"
copy_exec "$LIBDIR/libyubikey.so.0"  "$LIBDIR/"
copy_exec "$LIBDIR/libusb-1.0.so.0"  "$LIBDIR/"
copy_exec "$LIBDIR/libjson-c.so.5"   "$LIBDIR/"  2>/dev/null || true
copy_exec "$LIBDIR/libudev.so.1"     "$LIBDIR/"  2>/dev/null || true
copy_exec "$LIBDIR/libcap.so.2"      "$LIBDIR/"  2>/dev/null || true

manual_add_modules usbhid hid_generic
HOOK
chmod +x /etc/initramfs-tools/hooks/yubikey
ok "Wrote /etc/initramfs-tools/hooks/yubikey"

# Update crypttab: replace existing rootfs line with keyscript-driven one
sed -i '/^rootfs/d' /etc/crypttab
echo "rootfs  ${ROOT_PART}  -  luks,initramfs,keyscript=yubikey_unlock,noauto" >> /etc/crypttab
ok "Wrote /etc/crypttab with keyscript=yubikey_unlock"

# Make sure CRYPTSETUP=y so keyscripts get bundled into initrd
grep -q '^CRYPTSETUP=y' /etc/initramfs-tools/initramfs.conf \
    || echo 'CRYPTSETUP=y' >> /etc/initramfs-tools/initramfs.conf

update-initramfs -u

# Verify ykchalresp made it into the initrd
KERNEL_VERSION=$(uname -r)
if lsinitramfs "/boot/initrd.img-${KERNEL_VERSION}" 2>/dev/null | grep -q 'usr/bin/ykchalresp'; then
    ok "ykchalresp confirmed in initramfs"
else
    error "ykchalresp NOT in initramfs — boot will fail. Inspect /etc/initramfs-tools/hooks/yubikey"
    exit 1
fi

printf "\n${GREEN}${BOLD}=========================================${NC}\n"
printf "${GREEN}${BOLD}  YubiKey HMAC enrollment complete.${NC}\n"
echo ""
echo "  Boot test:"
printf "    1. ${CYAN}sudo reboot${NC}\n"
echo "    2. At unlock prompt, the YubiKey gold disc will blink"
echo "    3. Touch it — Pi unlocks and continues to Ubuntu"
echo ""
echo "  Backup unlock methods (still work):"
echo "    - Original passphrase (LUKS slot 0)"
echo "    - Inspect with: ${CYAN}sudo cryptsetup luksDump $ROOT_PART${NC}"
echo ""
echo "  Optional: remove passphrase slot once YubiKey works (irreversible):"
printf "    ${CYAN}sudo cryptsetup luksKillSlot $ROOT_PART 0${NC}\n"
echo ""
echo "  Next: ${CYAN}sudo bash 05-enable-fips.sh${NC} for FIPS hardening"
printf "${GREEN}${BOLD}=========================================${NC}\n"
