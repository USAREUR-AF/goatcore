#!/usr/bin/env bash
# ops/ci/test-rpi-flash-linux.sh — run ops/rpi-base-image/01-flash.sh on Linux
# against real kernel block devices, none of which hold data.
#
#   1. Refusals, before anything is written:
#        a regular file, a loop device, the disk holding /, and a scratch disk
#        with a mounted partition.
#   2. A full flash of a small Pi-shaped image (MBR, FAT32 "system-boot"
#      partition, .img.xz) onto a scsi_debug RAM disk that reports itself as
#      removable: write, partition re-read, mount, copy, unmount. The boot
#      partition is then checked for the full copy list.
#
# Needs root through sudo, the scsi_debug kernel module, dosfstools, xz,
# util-linux and sfdisk. Linux only; tests/test_rpi_flash.py covers the same
# checks with stubbed commands on any OS.
#
# Exit: 0 pass, 1 a check failed, 2 could not run.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SRC="$REPO_ROOT/ops/rpi-base-image"

[ "$(uname -s)" = Linux ] || { echo "SKIP-NOT-POSSIBLE: Linux only" >&2; exit 2; }
SUDO=""
[ "$(id -u)" -eq 0 ] || SUDO=sudo
for t in sfdisk mkfs.vfat xz lsblk findmnt losetup modprobe; do
    command -v "$t" >/dev/null 2>&1 || { echo "FAIL: missing $t" >&2; exit 2; }
done

WORK="$(mktemp -d)"
LOOP=""
SCSI_DEBUG=0
CHECK_MNT=""
cleanup() {
    [ -n "$CHECK_MNT" ] && { $SUDO umount "$CHECK_MNT" 2>/dev/null || true; }
    [ -n "$LOOP" ] && { $SUDO losetup -d "$LOOP" 2>/dev/null || true; }
    if [ "$SCSI_DEBUG" = 1 ]; then
        $SUDO udevadm settle 2>/dev/null || true
        $SUDO modprobe -r scsi_debug 2>/dev/null || true
    fi
    $SUDO rm -rf "$WORK"
}
trap cleanup EXIT

fails=0
pass() { echo "ok:   $*"; }
fail() { echo "FAIL: $*"; fails=$((fails + 1)); }

# expect_refusal <description> <expected stderr text> <disk>: a dry run must exit 1 with the text.
expect_refusal() {
    local what="$1" text="$2" disk="$3" out rc=0
    out="$(bash "$RPI/01-flash.sh" --dry-run "$disk" "$WORK/pi.img.xz" 2>&1 </dev/null)" || rc=$?
    if [ "$rc" -eq 1 ] && printf '%s' "$out" | grep -qF "$text"; then
        pass "$what refused ($text)"
    else
        fail "$what: expected exit 1 with '$text', got exit $rc"
        printf '%s\n' "$out" | sed 's/^/      /'
    fi
}

# A copy of the builder with the SSH key placeholder filled, as 00-prerequisites.sh leaves it.
RPI="$WORK/rpi-base-image"
cp -R "$SRC" "$RPI"
sed -i.bak 's|REPLACE_WITH_YOUR_PUBLIC_KEY|ssh-ed25519 TEST-ONLY-NOT-A-KEY ci|' "$RPI/cloud-init-configs/user-data"
rm -f "$RPI/cloud-init-configs/user-data.bak"

# --- A Pi-shaped image: MBR, partition 1 FAT32 labelled system-boot, partition 2 empty ---
truncate -s 96M "$WORK/pi.img"
printf 'label: dos\nstart=2048, size=131072, type=c\nstart=133120, type=83\n' | sfdisk -q "$WORK/pi.img"
mkfs.vfat -F 32 -n system-boot --offset 2048 "$WORK/pi.img" 65536 >/dev/null
xz -T0 -1 "$WORK/pi.img"

echo "== refusals before any write"
printf 'x' > "$WORK/target.img"
expect_refusal "a regular file" "Invalid disk path" "$WORK/target.img"
if [ "$(cat "$WORK/target.img")" = x ]; then pass "the regular file is untouched"; else fail "the regular file was changed"; fi

LOOP="$($SUDO losetup -f --show "$WORK/target.img")"
expect_refusal "a loop device ($LOOP)" "Invalid disk path" "$LOOP"

ROOT_SRC="$(lsblk -nrpo NAME,MOUNTPOINT | awk '$2 == "/" {print $1; exit}')"
ROOT_DISK=""
[ -n "$ROOT_SRC" ] && ROOT_DISK="$(lsblk -nrpso NAME,TYPE "$ROOT_SRC" | awk '$2 == "disk" {print $1; exit}')"
case "$ROOT_DISK" in
    /dev/sd[a-z]*|/dev/mmcblk[0-9]*|/dev/nvme[0-9]*n[0-9]*)
        expect_refusal "the disk holding / ($ROOT_DISK)" "holds the running system's root filesystem" "$ROOT_DISK" ;;
    *)
        fail "the disk holding / is '${ROOT_DISK:-unknown}' (from $ROOT_SRC), outside the accepted names; cannot exercise the refusal" ;;
esac

echo "== scratch disk (scsi_debug, removable)"
before="$(lsblk -dnrpo NAME | sort)"
if ! $SUDO modprobe scsi_debug dev_size_mb=128 removable=1 num_tgts=1 max_luns=1; then
    echo "FAIL: scsi_debug module not available on this kernel ($(uname -r))" >&2
    exit 2
fi
SCSI_DEBUG=1
$SUDO udevadm settle 2>/dev/null || true
DISK=""
for _ in 1 2 3 4 5 6 7 8 9 10; do
    DISK="$(comm -13 <(printf '%s\n' "$before") <(lsblk -dnrpo NAME | sort) | head -1)"
    [ -n "$DISK" ] && break
    sleep 1
done
[ -n "$DISK" ] || { echo "FAIL: the scsi_debug disk did not appear" >&2; exit 2; }
echo "scratch disk: $DISK ($(lsblk -dnro RM,TRAN,SIZE,MODEL "$DISK"))"
case "$(lsblk -dnro MODEL "$DISK")" in
    *scsi_debug*) ;;
    *) echo "FAIL: $DISK is not the scsi_debug disk; stopping" >&2; exit 2 ;;
esac

if out="$(bash "$RPI/01-flash.sh" --dry-run "$DISK" "$WORK/pi.img.xz" 2>&1 </dev/null)"; then
    pass "dry run accepts the removable scratch disk"
else
    fail "dry run refused the scratch disk"
    printf '%s\n' "$out" | sed 's/^/      /'
fi

echo "== full flash"
if printf '\n' | bash "$RPI/01-flash.sh" "$DISK" "$WORK/pi.img.xz"; then
    pass "01-flash.sh completed"
else
    fail "01-flash.sh exited non-zero"
fi
if findmnt -rn -S "${DISK}1" >/dev/null 2>&1; then
    fail "${DISK}1 is still mounted after the run"
fi
if compgen -G "${TMPDIR:-/tmp}/rpi-boot.*" >/dev/null; then
    fail "the temporary mount directory was left behind"
fi

CHECK_MNT="$WORK/check"
mkdir -p "$CHECK_MNT"
$SUDO mount -o ro "${DISK}1" "$CHECK_MNT"
for f in user-data network-config 02-clone-to-nvme.sh 03-prepare-encryption.sh \
         03b-headless-dropbear-unlock.sh 04-enroll-yubikey.sh 05-enable-fips.sh \
         encrypt-rootfs.sh encrypt-rootfs-unlock.sh \
         helper-scripts/encrypt-rootfs.sh helper-scripts/encrypt-rootfs-unlock.sh; do
    if [ -f "$CHECK_MNT/$f" ] && cmp -s "$CHECK_MNT/$f" "$RPI/$(case "$f" in
            user-data|network-config) echo "cloud-init-configs/$f" ;;
            encrypt-rootfs*.sh) echo "helper-scripts/$f" ;;
            *) echo "$f" ;; esac)"; then
        pass "boot partition has $f"
    else
        fail "boot partition is missing $f or it differs from the source"
    fi
done
if [ ! -e "$CHECK_MNT/04-post-encryption-setup.sh" ]; then pass "no 04-post-encryption-setup.sh"; else fail "04-post-encryption-setup.sh present"; fi
if grep -q "ssh-ed25519 TEST-ONLY-NOT-A-KEY" "$CHECK_MNT/user-data"; then pass "user-data carries the key"; else fail "user-data lacks the key"; fi

echo "== a mounted partition is refused"
expect_refusal "the scratch disk with ${DISK}1 mounted" "has mounted filesystems" "$DISK"
$SUDO umount "$CHECK_MNT"
CHECK_MNT=""

if [ "$fails" -gt 0 ]; then
    echo "FAIL: $fails check(s) failed"
    exit 1
fi
echo "ok: 01-flash.sh refuses before writing and flashes a removable disk end to end"
