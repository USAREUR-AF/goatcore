#!/usr/bin/env bash
# prepare-image.sh: a published cloud image with its packages already upgraded,
# for a run that cannot afford the upgrade (an emulated arm64 machine: the
# baseline's full-upgrade alone took more than an hour). ops/e2e/vm/README.md.
#
#   ops/e2e/vm/prepare-image.sh --arch arm64 [--ubuntu 24.04] [--timeout 7200]
#
# Boots the image, held to its release's signed checksums, runs the two package
# steps the host baseline runs (the cache refresh and the full upgrade), reboots
# into the new kernel, removes what belongs to one machine, and writes
#   <VM_IMAGES>/<ubuntu>/prepared-<arch>-<base sha256, 16>.qcow2 and .json
# The .json is the record: the base image's digest, the prepared image's digest,
# every package and version, and how long each step took. run.sh takes a prepared
# image only for the base image it was made from, and only at its recorded digest;
# its record says so. The baseline still runs whole: it finds nothing to upgrade,
# or what was published since.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
VM_ARCH=arm64; VM_UBUNTU=24.04; LIMIT=7200
while [ $# -gt 0 ]; do
  case "$1" in
    --arch) VM_ARCH="$2"; shift 2 ;;
    --ubuntu) VM_UBUNTU="$2"; shift 2 ;;
    --timeout) LIMIT="$2"; shift 2 ;;
    -h|--help) sed -n '2,20p' "$0"; exit 0 ;;
    *) echo "prepare-image: unknown argument $1" >&2; exit 2 ;;
  esac
done
WORK="$(mktemp -d "${TMPDIR:-/tmp}/goat-vm-prepare.XXXXXX")"
export VM_WORK="$WORK" VM_ARCH VM_UBUNTU VM_IMAGES="${VM_IMAGES:-$HOME/.cache/goat-vm-images}" VM_MEM="${VM_MEM:-2048}" VM_CPUS="${VM_CPUS:-4}" VM_TAPS=0 VM_NAME=prepare VM_BOOT_WAIT=1800 VM_PREPARED=no
VM_PORT="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')"
mkdir -p "$VM_IMAGES"
# shellcheck source=ops/e2e/vm/vm.sh
. "$HERE/vm.sh"
# shellcheck disable=SC2154 # rc is set in the trap itself
trap 'rc=$?; vm_down; rm -rf "$WORK"; exit $rc' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
T0=$(date +%s); vm_up; BOOT=$(( $(date +%s) - T0 ))
echo "up in $BOOT s, accel=$VM_ACCEL"
T1=$(date +%s)
timeout "$LIMIT" ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ServerAliveInterval=30 -i "$VM_WORK/key" -p "$VM_PORT" goatvm@127.0.0.1 'sudo sh -euc "
systemctl mask --now apt-daily.timer apt-daily-upgrade.timer unattended-upgrades.service >/dev/null 2>&1 || true
apt-get -o DPkg::Lock::Timeout=600 update -q
DEBIAN_FRONTEND=noninteractive NEEDRESTART_MODE=l apt-get -o DPkg::Lock::Timeout=600 -o Dpkg::Options::=--force-confold -yq full-upgrade
DEBIAN_FRONTEND=noninteractive apt-get -yq autoremove --purge
apt-get clean
systemctl unmask apt-daily.timer apt-daily-upgrade.timer unattended-upgrades.service >/dev/null 2>&1 || true"'
UPGRADE=$(( $(date +%s) - T1 ))
echo "upgraded in $UPGRADE s"
vm_ssh 'dpkg-query -W -f "\${Package} \${Version}\n"' > "$WORK/packages"
vm_ssh 'sudo sh -c "cloud-init clean --logs --seed >/dev/null 2>&1; rm -f /etc/ssh/ssh_host_* /var/lib/systemd/random-seed; truncate -s 0 /etc/machine-id; userdel -rf goatvm >/dev/null 2>&1; rm -f /etc/sudoers.d/90-cloud-init-users; sync; systemctl poweroff"' || true
for _i in $(seq 1 120); do kill -0 "$VM_PID" 2>/dev/null || break; sleep 5; done
if kill -0 "$VM_PID" 2>/dev/null; then echo "prepare-image: the VM did not power off" >&2; exit 1; fi
OUT="$VM_IMAGES/$VM_UBUNTU/prepared-$VM_ARCH-${VM_IMAGE_SHA256:0:16}"
qemu-img convert -O qcow2 -c "$WORK/disk.qcow2" "$OUT.qcow2.$$"
mv "$OUT.qcow2.$$" "$OUT.qcow2"
python3 - "$OUT" "$VM_ARCH" "$VM_UBUNTU" "$VM_IMAGE_SHA256" "$(sha256sum "$OUT.qcow2" | cut -d' ' -f1)" "$VM_ACCEL" "$BOOT" "$UPGRADE" "$WORK/packages" <<'PY'
import json, sys, datetime
out, arch, ubuntu, base, sha, accel, boot, up, pk = sys.argv[1:10]
json.dump({"schema": "goat-vm-prepared-image/v1", "arch": arch, "ubuntu": ubuntu, "base_sha256": base, "sha256": sha,
           "prepared": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"), "accel": accel,
           "boot_seconds": int(boot), "upgrade_seconds": int(up),
           "packages": dict(l.split(" ", 1) for l in open(pk).read().splitlines() if " " in l)}, open(out + ".json", "w"), indent=1)
PY
echo "prepare-image: $OUT.qcow2 sha256 $(sha256sum "$OUT.qcow2" | cut -d' ' -f1) (upgrade $UPGRADE s, $VM_ACCEL)"
