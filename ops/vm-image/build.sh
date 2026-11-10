#!/usr/bin/env bash
# build.sh — the goatlite prepared VM image (docs/design/goatlite.md §11 measure 5;
# ops/vm-image/README.md).
#
#   ops/vm-image/build.sh --out <dir> [--disk-gb 40] [--memory-mb 4096] [--cpus 4]
#
# Starts from Ubuntu 24.04's published cloud image (checked against its SHA256SUMS),
# boots it in QEMU, and runs on it what a goatlite standup runs before phase 5:
#   - phase 2's host baseline, ops/ansible/mgmt-host-baseline.yml, with the values a
#     goatlite with disk_encryption: false declares (Docker on /var/lib/docker)
#   - measure 2's pull, ops/scripts/standup-image-pull.sh, of every image any goatlite
#     runs (ops/scripts/standup-images.py --preset lite)
# then empties the guest of everything that belongs to one machine (cloud-init state,
# machine-id, SSH host keys, the build user, Docker's engine id), shuts it down and
# writes a compressed qcow2.
#
# A first-boot check follows: the written image is booted again, as a learner would,
# with a fresh cloud-init user, and must (1) take SSH as that user, (2) hold every
# listed image by digest, so the standup's pull step pulls nothing, and (3) run Docker
# on /var/lib/docker.
#
# Output in --out:
#   goatlite-ubuntu-24.04-amd64.qcow2   the image
#   goatlite-ubuntu-24.04-amd64.json    what it holds: source commit, Ubuntu serial and
#                                       checksum, the baseline and pin tables' sha256, the
#                                       image list, the first-boot check's result
#   SHA256SUMS                          for both files
#   build.log, console.log              the run
#
# Needs: qemu-system-x86_64, qemu-img, cloud-localds (cloud-image-utils), curl,
# ssh, ansible-playbook with ops/ansible/requirements.yml, python3 + pyyaml + jsonschema.
# Uses KVM when /dev/kvm is usable, else plain emulation (slow). No cloud account, no
# registry credential: every pinned goat image pulls anonymously.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
UBUNTU_BASE="${UBUNTU_BASE:-https://cloud-images.ubuntu.com/releases/24.04/release}"
UBUNTU_IMG=ubuntu-24.04-server-cloudimg-amd64.img
NAME=goatlite-ubuntu-24.04-amd64
OUT=""; DISK_GB=40; MEM=4096; CPUS=4; PORT="${VM_IMAGE_SSH_PORT:-2222}"
die() { echo "vm-image: $*" >&2; exit 2; }
while [ $# -gt 0 ]; do
  case "$1" in
    --out) OUT="$2"; shift 2 ;;
    --disk-gb) DISK_GB="$2"; shift 2 ;;
    --memory-mb) MEM="$2"; shift 2 ;;
    --cpus) CPUS="$2"; shift 2 ;;
    *) die "unknown argument: $1" ;;
  esac
done
[ -n "$OUT" ] || die "usage: build.sh --out <dir>"
for t in qemu-system-x86_64 qemu-img cloud-localds curl ssh ssh-keygen ansible-playbook python3 sha256sum; do
  command -v "$t" >/dev/null || die "$t is not installed"
done
mkdir -p "$OUT"; OUT="$(cd "$OUT" && pwd)"
WORK="$(mktemp -d)"
QPID=""
cleanup() { [ -n "$QPID" ] && kill "$QPID" 2>/dev/null || true; rm -rf "$WORK"; }
trap cleanup EXIT
exec > >(tee -a "$OUT/build.log") 2>&1
t_start=$(date +%s)
step() { printf '\n=== [%4ss] %s ===\n' "$(( $(date +%s) - t_start ))" "$*"; }

ACCEL=tcg
if [ -r /dev/kvm ] && [ -w /dev/kvm ]; then ACCEL=kvm; fi

# ---- 1. the published Ubuntu cloud image, checked ------------------------------
step "Ubuntu 24.04 cloud image from $UBUNTU_BASE"
curl -fsSL "$UBUNTU_BASE/SHA256SUMS" -o "$WORK/SHA256SUMS"
curl -fsSL "$UBUNTU_BASE/$UBUNTU_IMG" -o "$WORK/$UBUNTU_IMG"
want="$(grep " \*\?$UBUNTU_IMG\$" "$WORK/SHA256SUMS" | cut -d' ' -f1)"
got="$(sha256sum "$WORK/$UBUNTU_IMG" | cut -d' ' -f1)"
[ -n "$want" ] && [ "$want" = "$got" ] || die "$UBUNTU_IMG sha256 $got does not match SHA256SUMS ($want)"
serial="$(curl -fsSL "$UBUNTU_BASE/unpacked/build-info.txt" 2>/dev/null | sed -n 's/^serial=//p' || true)"
echo "ok: $UBUNTU_IMG sha256 $got (serial ${serial:-unknown})"
qemu-img convert -O qcow2 "$WORK/$UBUNTU_IMG" "$WORK/disk.qcow2"
qemu-img resize "$WORK/disk.qcow2" "${DISK_GB}G" >/dev/null

# ---- boot helpers ----------------------------------------------------------------
seed() {  # seed <user> <pubkey-file> <iso>
  cat > "$WORK/user-data" <<EOF
#cloud-config
users:
  - name: $1
    sudo: "ALL=(ALL) NOPASSWD:ALL"
    shell: /bin/bash
    ssh_authorized_keys:
      - $(cat "$2")
ssh_pwauth: false
EOF
  printf 'instance-id: %s\nlocal-hostname: goatlite\n' "$(basename "$3" .iso)-$(date +%s)" > "$WORK/meta-data"
  cloud-localds "$3" "$WORK/user-data" "$WORK/meta-data"
}
boot() {  # boot <disk> <seed-iso>
  qemu-system-x86_64 -machine "q35,accel=$ACCEL" -cpu "$( [ "$ACCEL" = kvm ] && echo host || echo max)" \
    -m "$MEM" -smp "$CPUS" -display none -serial "file:$OUT/console.log" \
    -drive "file=$1,if=virtio,format=qcow2,discard=unmap" \
    -drive "file=$2,if=virtio,format=raw,readonly=on" \
    -netdev "user,id=n0,hostfwd=tcp:127.0.0.1:$PORT-:22" -device virtio-net-pci,netdev=n0 &
  QPID=$!
}
SSH_OPTS=(-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=5 -p "$PORT")
wait_ssh() {  # wait_ssh <user> <key>
  for _ in $(seq 1 180); do
    if ssh "${SSH_OPTS[@]}" -i "$2" "$1@127.0.0.1" true 2>/dev/null; then return 0; fi
    kill -0 "$QPID" 2>/dev/null || die "the VM exited before SSH came up (console.log)"
    sleep 5
  done
  die "no SSH on 127.0.0.1:$PORT after 15 minutes (console.log)"
}
wait_off() {
  for _ in $(seq 1 120); do kill -0 "$QPID" 2>/dev/null || { QPID=""; return 0; }; sleep 2; done
  die "the VM did not power off within 4 minutes"
}

# ---- 2. boot the build VM -----------------------------------------------------------
step "boot the build VM (accel=$ACCEL)"
ssh-keygen -q -t ed25519 -N '' -C goat-vm-image-build -f "$WORK/build_key"
seed goatbuild "$WORK/build_key.pub" "$WORK/build-seed.iso"
boot "$WORK/disk.qcow2" "$WORK/build-seed.iso"
wait_ssh goatbuild "$WORK/build_key"
ssh "${SSH_OPTS[@]}" -i "$WORK/build_key" goatbuild@127.0.0.1 'sudo cloud-init status --wait >/dev/null; cloud-init status'

# ---- 3. phase 2: the host baseline -------------------------------------------------
step "phase 2: mgmt-host-baseline.yml"
cat > "$WORK/inventory.ini" <<EOF
[vmimage_hosts]
goatlite ansible_host=127.0.0.1 ansible_port=$PORT ansible_user=goatbuild ansible_ssh_private_key_file=$WORK/build_key
EOF
# Run directly, not through ansible-preflight.sh: this targets a throwaway VM in CI,
# not a goatnet, so the preflight's goatnet checks (trunk freshness, doors) do not apply.
# The two data-root values are what goatlite-render.py writes for disk_encryption: false.
# The baseline refuses an empty platform-admin roster and seeds it into the SSH user's
# authorized_keys; the roster here is the build key alone, and it leaves the image with
# the build user (step 5). The learner's own key arrives through cloud-init.
python3 -c 'import json,sys; print(json.dumps({"admin_authorized_keys": [open(sys.argv[1]).read().strip()]}))' \
  "$WORK/build_key.pub" > "$WORK/baseline-vars.json"
( cd "$REPO/ops/ansible" && ANSIBLE_HOST_KEY_CHECKING=False \
    ANSIBLE_SSH_ARGS="-o UserKnownHostsFile=/dev/null -o ControlMaster=auto -o ControlPersist=60s" \
    ansible-playbook -i "$WORK/inventory.ini" mgmt-host-baseline.yml \
      -e target=vmimage_hosts -e site_id=vmimage \
      -e docker_engine_data_root=/var/lib/docker -e docker_engine_containerd_root=/var/lib/containerd \
      -e require_docker_data_separate_mount=false -e "@$WORK/baseline-vars.json" )

# ---- 4. measure 2: every pinned image of the lite preset ----------------------------
step "pull every image any goatlite runs"
python3 "$REPO/ops/scripts/standup-images.py" --preset lite > "$WORK/images.txt"
echo "$(wc -l < "$WORK/images.txt" | tr -d ' ') image(s):"; sed 's/^/  /' "$WORK/images.txt"
scp -P "$PORT" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -i "$WORK/build_key" \
  "$REPO/ops/scripts/standup-image-pull.sh" "$WORK/images.txt" goatbuild@127.0.0.1:/tmp/
ssh "${SSH_OPTS[@]}" -i "$WORK/build_key" goatbuild@127.0.0.1 \
  'sudo bash /tmp/standup-image-pull.sh --concurrency 4 $(cat /tmp/images.txt)'

# ---- 5. what the image holds, recorded on it -----------------------------------------
step "record and empty the guest"
commit="$(cd "$REPO" && git rev-parse HEAD 2>/dev/null || echo unknown)"
python3 - "$WORK/images.txt" "$commit" "$got" "${serial:-unknown}" \
  "$(sha256sum "$REPO/ops/ansible/mgmt-host-baseline.yml" | cut -d' ' -f1)" \
  "$(sha256sum "$REPO/ops/pins/fleet-pins.yaml" | cut -d' ' -f1)" > "$WORK/prepared-image.json" <<'PY'
import json, sys, datetime
imgs, commit, ub_sha, serial, base_sha, pins_sha = sys.argv[1:7]
print(json.dumps({
    "schema": "goatlite-prepared-image/v1",
    "built": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "source_commit": commit,
    "ubuntu": {"image": "ubuntu-24.04-server-cloudimg-amd64.img", "sha256": ub_sha, "serial": serial},
    "baseline": {"playbook": "ops/ansible/mgmt-host-baseline.yml", "sha256": base_sha,
                 "values": {"docker_engine_data_root": "/var/lib/docker", "docker_engine_containerd_root": "/var/lib/containerd"}},
    "pins": {"table": "ops/pins/fleet-pins.yaml", "sha256": pins_sha, "list": "ops/scripts/standup-images.py --preset lite"},
    "images": [l.strip() for l in open(imgs) if l.strip()],
}, indent=2))
PY
scp -P "$PORT" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -i "$WORK/build_key" \
  "$WORK/prepared-image.json" goatbuild@127.0.0.1:/tmp/
# Everything that makes this one machine goes: the next boot is a new machine to
# cloud-init (new user, new host keys, new machine-id) and Docker makes a new engine id.
ssh "${SSH_OPTS[@]}" -i "$WORK/build_key" goatbuild@127.0.0.1 'sudo sh -euc "
  install -d -m 0755 /etc/goat && install -m 0644 /tmp/prepared-image.json /etc/goat/prepared-image.json
  rm -f /tmp/standup-image-pull.sh /tmp/images.txt /tmp/prepared-image.json
  systemctl stop docker.socket docker.service containerd.service
  rm -f /var/lib/docker/engine-id
  rm -rf /root/.docker
  apt-get clean
  rm -f /etc/ssh/ssh_host_*
  rm -f /etc/sudoers.d/90-cloud-init-users
  cloud-init clean --logs --seed --machine-id
  journalctl --rotate >/dev/null 2>&1 || true; journalctl --vacuum-time=1s >/dev/null 2>&1 || true
  userdel -rf goatbuild || true
  fstrim -av || true
  sync
  (sleep 2; systemctl poweroff) >/dev/null 2>&1 &
"' || true
wait_off

step "write $NAME.qcow2"
qemu-img convert -O qcow2 -c "$WORK/disk.qcow2" "$OUT/$NAME.qcow2"
ls -l "$OUT/$NAME.qcow2"

# ---- 6. first boot, as a learner would ----------------------------------------------
step "first-boot check: a new user, every image present, nothing pulled"
cp "$OUT/$NAME.qcow2" "$WORK/check.qcow2"
ssh-keygen -q -t ed25519 -N '' -C learner -f "$WORK/learner_key"
seed learner "$WORK/learner_key.pub" "$WORK/learner-seed.iso"
boot "$WORK/check.qcow2" "$WORK/learner-seed.iso"
wait_ssh learner "$WORK/learner_key"
scp -P "$PORT" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -i "$WORK/learner_key" \
  "$REPO/ops/scripts/standup-image-pull.sh" "$WORK/images.txt" learner@127.0.0.1:/tmp/
check="$(ssh "${SSH_OPTS[@]}" -i "$WORK/learner_key" learner@127.0.0.1 '
  set -e
  sudo cloud-init status --wait >/dev/null || true
  echo "root=$(sudo docker info --format "{{.DockerRootDir}}")"
  echo "goatbuild=$(id goatbuild >/dev/null 2>&1 && echo present || echo gone)"
  echo "machine_id=$(cat /etc/machine-id)"
  sudo bash /tmp/standup-image-pull.sh --concurrency 4 $(cat /tmp/images.txt)
  test -s /etc/goat/prepared-image.json && echo manifest=present
')"
echo "$check" | sed 's/^/  | /'
n="$(wc -l < "$WORK/images.txt" | tr -d ' ')"
echo "$check" | grep -q "^root=/var/lib/docker$" || die "first boot: Docker is not on /var/lib/docker"
echo "$check" | grep -q "^goatbuild=gone$" || die "first boot: the build user is still on the image"
echo "$check" | grep -q "^manifest=present$" || die "first boot: /etc/goat/prepared-image.json is missing"
echo "$check" | grep -q "$n of $n image(s) present .*($n already on the host by digest, not pulled)" \
  || die "first boot: the standup's pull step would still pull an image (see above)"
ssh "${SSH_OPTS[@]}" -i "$WORK/learner_key" learner@127.0.0.1 'sudo systemctl poweroff' >/dev/null 2>&1 || true
wait_off

# ---- 7. the record beside the image ---------------------------------------------------
python3 - "$WORK/prepared-image.json" "$OUT/$NAME.json" "$n" "$(( $(date +%s) - t_start ))" "$ACCEL" <<'PY'
import json, sys
m = json.load(open(sys.argv[1]))
m["first_boot_check"] = {"result": "pass", "images_present_by_digest": int(sys.argv[3]),
                         "pulled": 0, "docker_root": "/var/lib/docker", "build_user_removed": True}
m["build_seconds"] = int(sys.argv[4]); m["accel"] = sys.argv[5]
json.dump(m, open(sys.argv[2], "w"), indent=2)
PY
( cd "$OUT" && sha256sum "$NAME.qcow2" "$NAME.json" > SHA256SUMS && cat SHA256SUMS )
step "done: $OUT/$NAME.qcow2"
