#!/usr/bin/env bash
# The colour variables are fixed escape codes, so they are safe in printf formats.
# shellcheck disable=SC2059
set -euo pipefail

# =============================================================
# Flash Ubuntu + cloud-init to SD card for the Pi 5 base image
# Runs on macOS or Linux.
# =============================================================
# Usage:
#   ./01-flash.sh [--dry-run] [--allow-non-removable] <disk> <image.img.xz|image.img>
#
#   macOS:  ./01-flash.sh /dev/diskN  ~/Downloads/ubuntu-24.04-preinstalled-server-arm64+raspi.img.xz
#   Linux:  ./01-flash.sh /dev/sdX    ~/Downloads/ubuntu-24.04-preinstalled-server-arm64+raspi.img.xz
#           (also /dev/mmcblkN for a built-in SD reader, /dev/nvmeXnY)
#
# Find the SD card with `diskutil list` (macOS) or
# `lsblk -d -o NAME,SIZE,RM,TRAN,MODEL` (Linux).
#
# Before anything is written, the script refuses a target that:
#   - is not a whole disk of the form above,
#   - is not removable, USB or an SD card (override: --allow-non-removable),
#   - holds the running system's root filesystem `/`,
#   - (Linux) has any partition mounted. Unmount it first, e.g.
#     `udisksctl unmount -b /dev/sdX1` or `sudo umount /dev/sdX1`.
#
# --dry-run runs every check above, prints what would be written and
# copied, and exits without unmounting, writing or asking for sudo.
#
# Prerequisites:
#   - Ubuntu Server 24.04 ARM64 preinstalled image (for Raspberry Pi)
#     Download: https://ubuntu.com/download/raspberry-pi
#   - SD card inserted
#   - Your SSH public key in user-data (00-prerequisites.sh can set it)
#   - xz; on macOS also pv (progress bar)
#   - Linux: util-linux (lsblk, findmnt, mount); parted (partprobe) or
#     blockdev; udevadm where udev runs
# =============================================================

# --- Colors & helpers ---
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
WHITE='\033[1;37m'
BOLD='\033[1m'
NC='\033[0m'

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

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SYS_BLOCK=/sys/block

# What lands on the boot partition: "<source relative to SCRIPT_DIR> <destination
# relative to the boot partition root>". Every line is required; a missing
# source stops the run instead of being skipped. The helper scripts go both to
# the root (the console path runs /mnt/boot/encrypt-rootfs.sh) and under
# helper-scripts/ (the headless path runs /mnt/boot/helper-scripts/encrypt-rootfs.sh).
BOOT_COPY_LIST='
cloud-init-configs/user-data user-data
02-clone-to-nvme.sh 02-clone-to-nvme.sh
03-prepare-encryption.sh 03-prepare-encryption.sh
03b-headless-dropbear-unlock.sh 03b-headless-dropbear-unlock.sh
04-enroll-yubikey.sh 04-enroll-yubikey.sh
05-enable-fips.sh 05-enable-fips.sh
helper-scripts/encrypt-rootfs.sh encrypt-rootfs.sh
helper-scripts/encrypt-rootfs-unlock.sh encrypt-rootfs-unlock.sh
'
# Copied when present: the WiFi config is optional.
BOOT_OPTIONAL_LIST='
cloud-init-configs/network-config network-config
'

# --- OS and disk checks (each returns non-zero with the reason on stderr) ---

host_os() {
    case "$(uname -s)" in
        Darwin) echo darwin ;;
        Linux)  echo linux ;;
        *)      error "Unsupported OS: $(uname -s) (macOS or Linux only)"; return 1 ;;
    esac
}

# disk_path_valid <os> <disk>: a whole disk, never a partition.
disk_path_valid() {
    case "$1" in
        darwin) [[ "$2" =~ ^/dev/disk[0-9]+$ ]] ;;
        linux)  [[ "$2" =~ ^/dev/(sd[a-z]+|mmcblk[0-9]+|nvme[0-9]+n[0-9]+)$ ]] ;;
        *)      return 1 ;;
    esac
}

disk_path_hint() {
    case "$1" in
        darwin) echo "/dev/diskN" ;;
        *)      echo "/dev/sdX, /dev/mmcblkN or /dev/nvmeXnY" ;;
    esac
}

# boot_partition_of <disk>: /dev/sdb -> /dev/sdb1, /dev/mmcblk0 -> /dev/mmcblk0p1.
boot_partition_of() {
    case "$1" in
        /dev/disk*)                 echo "${1}s1" ;;
        *[0-9])                     echo "${1}p1" ;;
        *)                          echo "${1}1" ;;
    esac
}

is_block_device() { [ -b "$1" ]; }

# linux_disk_refusal <disk> <allow_non_removable 0|1>
linux_disk_refusal() {
    local disk="$1" allow="$2" name dtype rmflag tran sdcard=0 rootsrc line node mp mounted=""
    name="${disk#/dev/}"

    if ! is_block_device "$disk"; then
        error "$disk is not a block device"
        return 1
    fi
    dtype="$(lsblk -dnro TYPE "$disk" 2>/dev/null || true)"
    if [ "$dtype" != "disk" ]; then
        error "$disk is not a whole disk (lsblk TYPE: ${dtype:-unknown})"
        return 1
    fi

    # The disk holding / (through any partition, LUKS or LVM layer).
    rootsrc="$(findmnt -nvo SOURCE / 2>/dev/null || true)"
    if [ -n "$rootsrc" ] && lsblk -nrpso NAME "$rootsrc" 2>/dev/null | grep -qxF "$disk"; then
        error "$disk holds the running system's root filesystem / ($rootsrc). Refusing."
        return 1
    fi

    # Any mounted filesystem on the disk or its partitions.
    while IFS= read -r line; do
        [ -z "$line" ] && continue
        node="${line%% *}"
        mp=""
        [ "$node" != "$line" ] && mp="${line#* }"
        [ -z "$mp" ] && continue
        if [ "$mp" = "/" ]; then
            error "$disk holds the running system's root filesystem / ($node). Refusing."
            return 1
        fi
        mounted="${mounted}  ${node} on ${mp}
"
    done < <(lsblk -nrpo NAME,MOUNTPOINT "$disk" 2>/dev/null || true)
    if [ -n "$mounted" ]; then
        error "$disk has mounted filesystems. Unmount them first (udisksctl unmount -b <partition>, or sudo umount <partition>):"
        printf '%s' "$mounted" >&2
        return 1
    fi

    rmflag="$(lsblk -dnro RM "$disk" 2>/dev/null || true)"
    tran="$(lsblk -dnro TRAN "$disk" 2>/dev/null || true)"
    case "$name" in
        mmcblk*)
            # Built-in readers often report RM=0; the card type says SD vs soldered eMMC.
            if [ "$(cat "$SYS_BLOCK/$name/device/type" 2>/dev/null || true)" = "SD" ]; then
                sdcard=1
            fi
            ;;
    esac
    if [ "$rmflag" != "1" ] && [ "$tran" != "usb" ] && [ "$sdcard" != "1" ]; then
        if [ "$allow" = "1" ]; then
            warn "$disk is not removable, USB or an SD card (RM=${rmflag:-?} TRAN=${tran:-none}); continuing because --allow-non-removable was given"
        else
            error "$disk is not removable, USB or an SD card (RM=${rmflag:-?} TRAN=${tran:-none}). Refusing; pass --allow-non-removable if this really is the target."
            return 1
        fi
    fi
    return 0
}

# darwin_disk_refusal <disk> <allow_non_removable 0|1>
darwin_disk_refusal() {
    local disk="$1" allow="$2" name details rootinfo location removable protocol
    name="${disk#/dev/}"

    if ! details="$(diskutil info "$disk" 2>/dev/null)"; then
        error "diskutil does not know $disk"
        return 1
    fi
    rootinfo="$(diskutil info / 2>/dev/null || true)"
    if printf '%s\n' "$rootinfo" | grep -Eq "^[[:space:]]*(Part of Whole|APFS Physical Store):[[:space:]]+${name}(s[0-9]+)?[[:space:]]*$"; then
        error "$disk holds the running system's root filesystem /. Refusing."
        return 1
    fi

    location="$(printf '%s\n' "$details" | sed -n 's/^[[:space:]]*Device Location:[[:space:]]*//p' | head -1)"
    removable="$(printf '%s\n' "$details" | sed -n 's/^[[:space:]]*Removable Media:[[:space:]]*//p' | head -1)"
    protocol="$(printf '%s\n' "$details" | sed -n 's/^[[:space:]]*Protocol:[[:space:]]*//p' | head -1)"
    case "$removable|$protocol|$location" in
        Removable\|*|*\|USB\|*|*\|Secure\ Digital\|*|*\|External) return 0 ;;
    esac
    if [ "$allow" = "1" ]; then
        warn "$disk is not removable, USB or an SD card (${location:-?}, ${removable:-?}, ${protocol:-?}); continuing because --allow-non-removable was given"
        return 0
    fi
    error "$disk is not removable, USB or an SD card (${location:-?}, ${removable:-?}, ${protocol:-?}). Refusing; pass --allow-non-removable if this really is the target."
    return 1
}

# disk_refusal <os> <disk> <allow>: path, then device checks. Non-zero = refuse.
disk_refusal() {
    if ! disk_path_valid "$1" "$2"; then
        error "Invalid disk path: $2 (expected $(disk_path_hint "$1"), a whole disk, not a partition)"
        return 1
    fi
    case "$1" in
        darwin) darwin_disk_refusal "$2" "$3" ;;
        linux)  linux_disk_refusal "$2" "$3" ;;
    esac
}

# image_size <file>: bytes, on BSD and GNU alike.
image_size() { wc -c < "$1" | tr -d '[:space:]'; }

# copy_boot_files <boot mount> [cmd prefix]: copy the list, then verify it landed.
copy_boot_files() {
    local dest="$1" pfx="${2:-}" src dst missing=0
    # Check every source first so a missing one stops before anything is copied.
    while read -r src dst; do
        [ -z "$src" ] && continue
        if [ ! -f "$SCRIPT_DIR/$src" ]; then
            error "Missing from the repo checkout: $SCRIPT_DIR/$src"
            missing=1
        fi
    done <<< "$BOOT_COPY_LIST"
    [ ! -d "$SCRIPT_DIR/helper-scripts" ] && { error "Missing: $SCRIPT_DIR/helper-scripts/"; missing=1; }
    [ "$missing" = 0 ] || return 1

    while read -r src dst; do
        [ -z "$src" ] && continue
        $pfx cp "$SCRIPT_DIR/$src" "$dest/$dst"
        ok "Copied $dst"
    done <<< "$BOOT_COPY_LIST"
    while read -r src dst; do
        [ -z "$src" ] && continue
        if [ -f "$SCRIPT_DIR/$src" ]; then
            $pfx cp "$SCRIPT_DIR/$src" "$dest/$dst"
            ok "Copied $dst"
        fi
    done <<< "$BOOT_OPTIONAL_LIST"
    $pfx mkdir -p "$dest/helper-scripts"
    $pfx cp -R "$SCRIPT_DIR/helper-scripts/." "$dest/helper-scripts/"
    ok "Copied helper-scripts/"

    while read -r src dst; do
        [ -z "$src" ] && continue
        [ -f "$dest/$dst" ] || { error "Not on the boot partition after copy: $dst"; missing=1; }
    done <<< "$BOOT_COPY_LIST"
    for src in "$SCRIPT_DIR"/helper-scripts/*; do
        [ -f "$dest/helper-scripts/$(basename "$src")" ] || { error "Not on the boot partition after copy: helper-scripts/$(basename "$src")"; missing=1; }
    done
    [ "$missing" = 0 ]
}

print_plan() {
    local os="$1" disk="$2" image="$3" target="$2"
    [ "$os" = darwin ] && target="/dev/r${disk#/dev/}"
    info "Would write $image to $target"
    if [ "$os" = linux ]; then
        info "  dd of=$target bs=4M conv=fsync status=progress iflag=fullblock"
        info "Would mount $(boot_partition_of "$disk") at a temporary directory"
    else
        info "  pv | dd of=$target bs=4m"
        info "Would mount the boot partition with diskutil mountDisk $disk"
    fi
    info "Would copy to the boot partition:"
    local src dst
    while read -r src dst; do
        [ -z "$src" ] && continue
        printf '    %s\n' "$dst"
    done <<< "$BOOT_COPY_LIST"
    while read -r src dst; do
        [ -z "$src" ] && continue
        printf '    %s (if present)\n' "$dst"
    done <<< "$BOOT_OPTIONAL_LIST"
    printf '    helper-scripts/\n'
}

usage() {
    echo "Usage: $0 [--dry-run] [--allow-non-removable] <disk> /path/to/ubuntu-image.img.xz"
    echo ""
    echo "  macOS disk: /dev/diskN          find it with: diskutil list"
    echo "  Linux disk: /dev/sdX, /dev/mmcblkN, /dev/nvmeXnY"
    echo "                                  find it with: lsblk -d -o NAME,SIZE,RM,TRAN,MODEL"
}

# --- State for cleanup ---
LINUX_MNT=""
LINUX_MOUNTED=0
SUDO=""

cleanup() {
    local rc=$?
    if [ "$LINUX_MOUNTED" = 1 ]; then
        $SUDO umount "$LINUX_MNT" 2>/dev/null || warn "Could not unmount $LINUX_MNT"
    fi
    if [ -n "$LINUX_MNT" ]; then rmdir "$LINUX_MNT" 2>/dev/null || true; fi
    if [ "$rc" -ne 0 ] && [ "${WRITE_STARTED:-0}" = 1 ]; then
        error "Script failed. SD card may be in an inconsistent state."
    fi
    return "$rc"
}

write_image() {
    local os="$1" target="$2" image="$3" size
    if [ "$os" = linux ]; then
        if [[ "$image" == *.xz ]]; then
            info "Decompressing and writing..."
            xz -dc "$image" | $SUDO dd of="$target" bs=4M conv=fsync status=progress iflag=fullblock
        else
            info "Writing image..."
            $SUDO dd if="$image" of="$target" bs=4M conv=fsync status=progress iflag=fullblock
        fi
        return
    fi
    if [[ "$image" == *.xz ]]; then
        # Get decompressed size for progress, decompress and write
        size=$(xz --robot --list "$image" 2>/dev/null | awk '/^totals/{print $5}')
        info "Decompressing and writing..."
        if [ -n "$size" ] && [ "$size" -gt 0 ] 2>/dev/null; then
            xz -dc "$image" | pv -s "$size" -petra | sudo dd of="$target" bs=4m
        else
            xz -dc "$image" | pv -petra | sudo dd of="$target" bs=4m
        fi
    else
        size=$(image_size "$image")
        info "Writing image..."
        pv -s "$size" -petra "$image" | sudo dd of="$target" bs=4m
    fi
}

main() {
    local dry_run=0 allow=0 os disk image user_data placeholder=0
    while [ $# -gt 0 ]; do
        case "$1" in
            --dry-run) dry_run=1; shift ;;
            --allow-non-removable) allow=1; shift ;;
            -h|--help) usage; exit 0 ;;
            --) shift; break ;;
            -*) error "Unknown option: $1"; usage; exit 1 ;;
            *) break ;;
        esac
    done
    disk="${1:-}"
    image="${2:-}"
    if [ -z "$disk" ] || [ -z "$image" ]; then
        usage
        exit 1
    fi

    trap cleanup EXIT
    banner "Base Image SD Card Flash"
    os="$(host_os)"

    case "$image" in
        *.img.xz|*.img) ;;
        *) error "Image must be .img or .img.xz: $image"; exit 1 ;;
    esac
    if [ ! -f "$image" ]; then
        error "Image not found: $image"
        exit 1
    fi

    step "Checking $disk"
    disk_refusal "$os" "$disk" "$allow" || exit 1
    ok "$disk passes the target checks"

    user_data="$SCRIPT_DIR/cloud-init-configs/user-data"
    if [ ! -f "$user_data" ]; then
        error "user-data not found at $user_data"
        exit 1
    fi
    grep -q "REPLACE_WITH_YOUR_PUBLIC_KEY" "$user_data" && placeholder=1

    if [ "$dry_run" = 1 ]; then
        if [ "$placeholder" = 1 ]; then
            warn "user-data still has the SSH key placeholder; a real run stops here"
        fi
        print_plan "$os" "$disk" "$image"
        ok "Dry run: nothing written"
        exit 0
    fi

    # Check SSH key is configured
    if [ "$placeholder" = 1 ]; then
        error "Edit user-data and replace the SSH key placeholder with your actual public key"
        info "Your key: $(cat ~/.ssh/id_ed25519.pub 2>/dev/null || echo 'not found')"
        exit 1
    fi

    if [ "$os" = darwin ] && ! command -v pv &>/dev/null; then
        error "pv not found. Install it: brew install pv"
        exit 1
    fi
    if [[ "$image" == *.xz ]] && ! command -v xz &>/dev/null; then
        error "xz not found. Run ./00-prerequisites.sh"
        exit 1
    fi

    # Safety check
    printf "${YELLOW}${BOLD}WARNING:${NC} This will erase ${BOLD}%s${NC}\n" "$disk"
    echo "Press Ctrl+C to cancel, Enter to continue..."
    read -r

    # Cache sudo credentials before starting the pipe so the password
    # prompt doesn't get buried under progress output
    if [ "$(id -u)" -ne 0 ]; then
        SUDO=sudo
        sudo -v
    fi

    if [ "$os" = darwin ]; then
        flash_darwin "$disk" "$image"
    else
        flash_linux "$disk" "$image"
    fi

    printf "\n${GREEN}${BOLD}=== Done ===${NC}\n"
    echo "1. Insert SD card into Pi and boot"
    echo "2. Wait 3-5 minutes for cloud-init to complete"
    echo "3. Find Pi IP: nmap -sn 192.168.x.0/24 (or check router)"
    echo "4. SSH in: ssh goatnet@<PI_IP>  (or ubuntu@<PI_IP> as fallback)"
    echo ""
    echo "If deploying to NVMe (recommended for Pi 5):"
    printf "5. ${CYAN}sudo bash 02-clone-to-nvme.sh${NC}\n"
    echo "   Then remove SD card and boot from NVMe."
    echo ""
    echo "Then continue with encryption:"
    printf "6. ${CYAN}sudo bash 03-prepare-encryption.sh${NC}\n"
    echo ""
    echo "See README.md for full steps."
}

flash_darwin() {
    local disk="$1" image="$2" raw boot="" mount_point
    step "Step 1: Unmounting $disk"
    diskutil unmountDisk "$disk"
    ok "Unmounted"

    step "Step 2: Flashing image to $disk"
    raw="/dev/r${disk#/dev/}"
    WRITE_STARTED=1
    write_image darwin "$raw" "$image"
    ok "Flash complete"

    step "Step 3: Waiting for partitions to mount"
    sleep 3
    diskutil mountDisk "$disk"

    # Find the boot partition (system-boot)
    for mount_point in /Volumes/system-boot /Volumes/BOOT /Volumes/boot-firmware; do
        if [ -d "$mount_point" ]; then
            boot="$mount_point"
            break
        fi
    done
    if [ -z "$boot" ]; then
        error "Could not find boot partition mount point"
        info "Check: ls /Volumes/"
        exit 1
    fi
    ok "Boot partition: $boot"

    step "Step 4: Copying configs to $boot"
    copy_boot_files "$boot"

    step "Step 5: Ejecting $disk"
    diskutil eject "$disk"
    ok "Ejected"
}

flash_linux() {
    local disk="$1" image="$2" part i
    part="$(boot_partition_of "$disk")"

    step "Step 1: Flashing image to $disk"
    WRITE_STARTED=1
    write_image linux "$disk" "$image"
    sync
    ok "Flash complete"

    step "Step 2: Re-reading the partition table"
    if command -v partprobe &>/dev/null; then
        $SUDO partprobe "$disk" || warn "partprobe $disk failed"
    else
        $SUDO blockdev --rereadpt "$disk" || warn "blockdev --rereadpt $disk failed"
    fi
    if command -v udevadm &>/dev/null; then
        $SUDO udevadm settle || true
    fi
    for i in 1 2 3 4 5 6 7 8 9 10; do
        is_block_device "$part" && break
        [ "$i" = 10 ] && { error "Boot partition $part did not appear"; exit 1; }
        sleep 1
    done
    ok "Boot partition: $part"

    step "Step 3: Mounting $part"
    LINUX_MNT="$(mktemp -d "${TMPDIR:-/tmp}/rpi-boot.XXXXXX")"
    $SUDO mount "$part" "$LINUX_MNT"
    LINUX_MOUNTED=1
    ok "Mounted at $LINUX_MNT"

    step "Step 4: Copying configs to $part"
    copy_boot_files "$LINUX_MNT" "$SUDO"
    sync

    step "Step 5: Unmounting $part"
    $SUDO umount "$LINUX_MNT"
    LINUX_MOUNTED=0
    rmdir "$LINUX_MNT"
    LINUX_MNT=""
    ok "Unmounted. The SD card can be removed."
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    main "$@"
fi
