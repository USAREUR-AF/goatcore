#!/usr/bin/env bash
# The colour variables are fixed escape codes, so they are safe in printf formats.
# shellcheck disable=SC2059
set -euo pipefail

# =============================================================
# 01b-verify-image.sh
# Run on your workstation after 01-flash.sh, before the card goes into the Pi.
# Reads the card back and proves it holds what 01-flash.sh wrote:
#
#   1. the card's partition table is the Ubuntu image's;
#   2. the root partition is the image's, byte for byte;
#   3. on the boot partition, every file of the image's own is the image's,
#      every file 01-flash.sh copies is this checkout's, byte for byte, there
#      is nothing else, and user-data names an SSH public key.
#
# It writes nothing. Reading a card needs root: it asks sudo for the reads
# when you cannot read the disk yourself. A regular file (an image written
# for a test) is read the same way.
#
# Usage: ./01b-verify-image.sh <disk> /path/to/ubuntu-image.img.xz
# Exit: 0 every check holds, 1 a check does not, 2 it could not run.
# =============================================================

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# The copy list is 01-flash.sh's own, so the two cannot differ. Sourcing it
# runs nothing of it but its definitions.
# shellcheck source=ops/rpi-base-image/01-flash.sh
. "$SCRIPT_DIR/01-flash.sh"

# One byte order for sort and comm, whatever the caller's locale.
export LC_ALL=C

bad()   { printf "${RED}[FAIL]${NC} %s\n" "$*" >&2; FAILED=$((FAILED + 1)); }
die()   { printf "${RED}[error]${NC} %s\n" "$*" >&2; exit 2; }

FAILED=0

sha256() {
    if command -v sha256sum >/dev/null 2>&1; then sha256sum | cut -d' ' -f1
    else shasum -a 256 | cut -d' ' -f1
    fi
}

# image_stream: the image, decompressed. A reader that stops early ends it
# with SIGPIPE, which is not a failure: the file's integrity is tested first.
image_stream() {
    case "$IMAGE" in
        *.xz) xz -dc "$IMAGE" || true ;;
        *)    cat "$IMAGE" || true ;;
    esac
}

# bytes <offset> <length>: that range of standard input. head stopping first
# ends tail with SIGPIPE, which is not a failure.
bytes() { { tail -c +"$(( $1 + 1 ))" || true; } | head -c "$2"; }

# entry <mbr file> <n>: "<type> <first sector> <sectors>" of primary partition n (1-4).
entry() {
    local at=$(( 446 + ($2 - 1) * 16 ))
    od -An -tu1 -j "$at" -N 16 "$1" | tr -s ' \n' ' ' | awk '{
        printf "%d %d %d\n", $5, $9 + $10*256 + $11*65536 + $12*16777216, $13 + $14*256 + $15*65536 + $16*16777216 }'
}

WORK=""
cleanup_work() { [ -z "$WORK" ] || rm -rf "$WORK"; }

verify_image_main() {
    [ $# -eq 2 ] || { echo "Usage: $0 <disk> /path/to/ubuntu-image.img.xz"; exit 2; }
    DISK="$1"; IMAGE="$2"
    [ -e "$DISK" ] || die "$DISK does not exist"
    if [ -b "$DISK" ] || [ -c "$DISK" ]; then
        # A card is a whole disk, as 01-flash.sh writes it: never a partition.
        disk_path_valid "$(host_os)" "$DISK" || die "$DISK is not a whole disk ($(disk_path_hint "$(host_os)"))"
    elif [ ! -f "$DISK" ]; then
        die "$DISK is neither a disk nor a file"
    fi
    [ -f "$IMAGE" ] || die "$IMAGE is not a file"
    local t
    for t in od tail head awk sort comm mcopy mdir; do
        command -v "$t" >/dev/null 2>&1 || die "$t is needed (mcopy, mdir: the mtools package; brew install mtools)"
    done
    case "$IMAGE" in *.xz) command -v xz >/dev/null 2>&1 || die "xz is needed" ;; esac
    command -v sha256sum >/dev/null 2>&1 || command -v shasum >/dev/null 2>&1 || die "sha256sum or shasum is needed"
    SUDO=""
    if [ ! -r "$DISK" ]; then
        [ "$(id -u)" -eq 0 ] && die "$DISK cannot be read"
        SUDO=sudo
        sudo -v || die "reading $DISK needs root, and sudo did not run"
    fi
    # sudo resets the environment, so mtools' setting goes on its command line.
    MT="$SUDO env MTOOLS_SKIP_CHECK=1"

    WORK="$(mktemp -d "${TMPDIR:-/tmp}/verify-image.XXXXXX")"
    trap cleanup_work EXIT

    case "$IMAGE" in
        *.xz) xz -t "$IMAGE" 2>/dev/null || die "$IMAGE is not a whole xz file (a partial download?)" ;;
    esac

    # 1. The partition table.
    image_stream | head -c 512 > "$WORK/image.mbr"
    $SUDO head -c 512 "$DISK" > "$WORK/disk.mbr" || die "$DISK cannot be read"
    if [ "$(bytes 446 66 < "$WORK/image.mbr" | sha256)" = "$(bytes 446 66 < "$WORK/disk.mbr" | sha256)" ]; then
        ok "partition table: the image's"
    else
        bad "partition table: $DISK's is not the image's (a different image, or the card was written over since)"
    fi
    local p1 p2 boot_type boot_start boot_size root_start root_size
    p1="$(entry "$WORK/image.mbr" 1)"; p2="$(entry "$WORK/image.mbr" 2)"
    read -r boot_type boot_start boot_size <<< "$p1"
    read -r _ root_start root_size <<< "$p2"
    [ "$root_size" -gt 0 ] || die "the image has no second partition: is $IMAGE a Raspberry Pi preinstalled image?"
    case "$boot_type" in 11|12|14) ;; *) die "the image's first partition is type $boot_type, not FAT" ;; esac

    # 2. The root partition, byte for byte.
    local want got
    want="$(image_stream | bytes $(( root_start * 512 )) $(( root_size * 512 )) | sha256)"
    got="$({ $SUDO tail -c +"$(( root_start * 512 + 1 ))" "$DISK" || true; } | head -c $(( root_size * 512 )) | sha256)"
    if [ "$want" = "$got" ]; then
        ok "root partition: the image's ($(( root_size / 2048 )) MiB, sha256 ${want:0:16}...)"
    else
        bad "root partition: sha256 $got on $DISK, $want in the image (a failed write, a worn card, or the card has booted since: a desktop that mounts it changes it)"
    fi

    # 3. The boot partition: the image's own files unchanged, 01-flash.sh's
    #    copy list as this checkout has it, and nothing else.
    image_stream | bytes $(( boot_start * 512 )) $(( boot_size * 512 )) > "$WORK/boot.img"
    local card="$DISK@@$(( boot_start * 512 ))" src dst f
    # listing <mtools image>: its files, one path a line, no directories.
    listing() { { $MT mdir -/ -b -i "$1" :: 2>/dev/null || echo "LISTING-FAILED"; } | { grep -v '/$' || true; } | sed 's#^::/##' | sort; }
    listing "$card" > "$WORK/card.list"
    MT="env MTOOLS_SKIP_CHECK=1" listing "$WORK/boot.img" > "$WORK/image.list"
    ! grep -qx "LISTING-FAILED" "$WORK/image.list" || die "the image's boot partition cannot be read as FAT"
    ! grep -qx "LISTING-FAILED" "$WORK/card.list" || bad "boot partition: $DISK's cannot be read as FAT"
    # The copy list: destination, and its source in this checkout.
    : > "$WORK/copy.list"
    while read -r src dst; do
        [ -z "$src" ] && continue
        printf '%s %s\n' "$dst" "$src" >> "$WORK/copy.list"
    done <<< "$BOOT_COPY_LIST"
    while read -r src dst; do
        [ -z "$src" ] && continue
        [ -f "$SCRIPT_DIR/$src" ] && printf '%s %s\n' "$dst" "$src" >> "$WORK/copy.list"
    done <<< "$BOOT_OPTIONAL_LIST"
    for src in "$SCRIPT_DIR"/helper-scripts/*; do
        printf 'helper-scripts/%s helper-scripts/%s\n' "$(basename "$src")" "$(basename "$src")" >> "$WORK/copy.list"
    done
    cut -d' ' -f1 "$WORK/copy.list" | sort > "$WORK/copied.list"
    while read -r dst src; do
        if ! $MT mcopy -i "$card" "::/$dst" - > "$WORK/copy" 2>/dev/null; then
            bad "boot partition: $dst is missing"
        elif [ "$(sha256 < "$WORK/copy")" = "$(sha256 < "$SCRIPT_DIR/$src")" ]; then
            ok "boot partition: $dst is this checkout's $src"
        else
            bad "boot partition: $dst is not this checkout's $src (flash again from this checkout)"
        fi
    done < "$WORK/copy.list"
    local same=0
    while read -r f; do
        grep -qxF -- "$f" "$WORK/copied.list" && continue
        if ! $MT mcopy -i "$card" "::/$f" - > "$WORK/copy" 2>/dev/null; then
            bad "boot partition: the image's $f is missing"
        elif [ "$(sha256 < "$WORK/copy")" != "$(env MTOOLS_SKIP_CHECK=1 mcopy -i "$WORK/boot.img" "::/$f" - 2>/dev/null | sha256)" ]; then
            bad "boot partition: $f is not the image's (it was changed after the flash)"
        else
            same=$((same + 1))
        fi
    done < "$WORK/image.list"
    ok "boot partition: $same of the image's own files are the image's"
    comm -23 "$WORK/card.list" "$WORK/image.list" | comm -23 - "$WORK/copied.list" > "$WORK/extra.list"
    while read -r f; do
        bad "boot partition: $f is in neither the image nor 01-flash.sh's copy list"
    done < "$WORK/extra.list"
    # user-data must let a key in: a placeholder or no key at all is a card nobody can log in to.
    $MT mcopy -i "$card" "::/user-data" - > "$WORK/user-data" 2>/dev/null || true
    if grep -q "REPLACE_WITH_YOUR_PUBLIC_KEY" "$WORK/user-data"; then
        bad "boot partition: user-data still has the SSH key placeholder, so no key can log in (run ./00-prerequisites.sh, then flash again)"
    elif ! grep -Eq '^[[:space:]]*-[[:space:]]*"?(ssh-ed25519|ssh-rsa|ecdsa-sha2-nistp(256|384|521)|sk-ssh-ed25519@openssh\.com|sk-ecdsa-sha2-nistp256@openssh\.com) [A-Za-z0-9+/]{20,}' "$WORK/user-data"; then
        bad "boot partition: user-data names no SSH public key, so no key can log in"
    fi

    if [ "$FAILED" -eq 0 ]; then
        ok "$DISK holds the image and this checkout's files: put it in the Pi"
        exit 0
    fi
    exit 1
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    verify_image_main "$@"
fi
