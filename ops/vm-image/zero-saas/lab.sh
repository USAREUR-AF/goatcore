#!/usr/bin/env bash
# lab.sh — a goatlite stood up with no route to any outside service, and every packet
# of the run recorded (ops/vm-image/zero-saas/README.md). LAB ONLY: nothing here is
# part of goatbuilder.
#
#   lab.sh prepare-laptop --work DIR [--ubuntu-img FILE]
#   lab.sh run --work DIR --image GOATLITE.qcow2 --record DIR
#
# prepare-laptop  (before the clock, with outside network) builds DIR/laptop.qcow2: an
#                 Ubuntu 24.04 VM with what tutorial 00 section 1 installs on a laptop,
#                 the checkout of this repository's HEAD, goatbuilder and goat built
#                 from it (laptop-provision.sh).
# run             the clock. Two VMs on one private segment and nothing else:
#                   laptop  10.88.0.1  its second NIC is qemu user networking with
#                                      restrict=on: the runner drives it over SSH, and
#                                      its gateway drops every other packet
#                   box     10.88.0.2  the goatlite prepared image (ops/vm-image/build.sh),
#                                      default route via the laptop, which forwards nothing
#                 qemu's filter-dump writes every frame of both laptop NICs to pcap, so
#                 every packet either machine sends is in the record whatever the guests
#                 do. On the laptop: goatlite.yaml, `goatbuilder standup`, the phase-11
#                 stand-in (phase11-answer.mjs, lab only), the resume from 12, then
#                 `goatbuilder verify` and `status`. judge.py then reads the pcaps over the
#                 clock's window: PASS only with no packet to an outside address and no DNS
#                 question for an outside name.
#
# Env: LAB_BOX_MEMORY_MB (10240), LAB_LAPTOP_MEMORY_MB (3072), LAB_CPUS (2 each),
#      LAB_SSH_PORT (2321), LAB_NAME (zsaas), LAB_STEP_TIMEOUT (5400 s per standup call),
#      LAB_GOATBUILDER_FLAGS (--offline; empty runs the default, which fetches origin).
# Needs: qemu-system-x86_64, qemu-img, xorriso or cloud-localds, ssh, curl, python3.
# Exit: 0 the standup verified and the judge passed; 1 the judge failed (the record names
# every outside packet and name); 3 the standup or verify failed; 2 usage or environment.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../../.." && pwd)"
BOX_MEM="${LAB_BOX_MEMORY_MB:-10240}"; LAPTOP_MEM="${LAB_LAPTOP_MEMORY_MB:-3072}"; CPUS="${LAB_CPUS:-2}"
PORT="${LAB_SSH_PORT:-2321}"; NAME="${LAB_NAME:-zsaas}"; STEP_TIMEOUT="${LAB_STEP_TIMEOUT:-5400}"
# goatbuilder's own flag for a site with no route to its source of record (C3,
# ops/scripts/fresh-source.sh): the run is judged against the checkout's cached origin/main.
GB_FLAGS="${LAB_GOATBUILDER_FLAGS---offline}"
UBUNTU_BASE="${UBUNTU_BASE:-https://cloud-images.ubuntu.com/releases/24.04/release}"
UBUNTU_IMG=ubuntu-24.04-server-cloudimg-amd64.img
LAPTOP_MAC_UP=52:54:00:00:01:01; LAPTOP_MAC_LAN=52:54:00:00:01:02; BOX_MAC_LAN=52:54:00:00:02:02
LAB_NETS="10.88.0.0/24,fd88::/64,10.0.2.0/24,fec0::/64"
die() { echo "lab: $*" >&2; exit 2; }
say() { printf '\n=== [%s] %s ===\n' "$(date -u +%H:%M:%SZ)" "$*"; }

CMD="${1:-}"; [ $# -gt 0 ] && shift
WORK=""; IMAGE=""; RECORD=""; UB_FILE=""
while [ $# -gt 0 ]; do
  case "$1" in
    --work) WORK="$2"; shift 2 ;;
    --image) IMAGE="$2"; shift 2 ;;
    --record) RECORD="$2"; shift 2 ;;
    --ubuntu-img) UB_FILE="$2"; shift 2 ;;
    *) die "unknown argument: $1" ;;
  esac
done
[ -n "$WORK" ] || die "usage: lab.sh prepare-laptop|run --work DIR ..."
mkdir -p "$WORK"; WORK="$(cd "$WORK" && pwd)"
for t in qemu-system-x86_64 qemu-img ssh scp ssh-keygen curl python3 sha256sum; do
  command -v "$t" >/dev/null || die "$t is not installed"
done
ACCEL=tcg; CPU=max
if [ -r /dev/kvm ] && [ -w /dev/kvm ]; then ACCEL=kvm; CPU=host; fi

PIDS=""
cleanup() { for p in $PIDS; do kill "$p" 2>/dev/null || true; done; }
trap cleanup EXIT

seed_iso() { # seed_iso <iso> <dir holding user-data, meta-data, network-config>
  if command -v xorriso >/dev/null; then
    xorriso -as mkisofs -quiet -output "$1" -volid cidata -joliet -rock \
      "$2/user-data" "$2/meta-data" "$2/network-config" 2>/dev/null
  elif command -v cloud-localds >/dev/null; then
    cloud-localds -N "$2/network-config" "$1" "$2/user-data" "$2/meta-data"
  else
    die "neither xorriso nor cloud-localds is installed"
  fi
}
user_data() { # user_data <file> <user> <pubkey>
  cat > "$1" <<EOF
#cloud-config
users:
  - name: $2
    sudo: "ALL=(ALL) NOPASSWD:ALL"
    shell: /bin/bash
    ssh_authorized_keys:
      - $3
ssh_pwauth: false
EOF
}
SSH_O=(-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=10 -o ServerAliveInterval=30)
on_laptop() { ssh "${SSH_O[@]}" -i "$WORK/laptop-key" -p "$PORT" lab@127.0.0.1 "$@"; }
to_laptop() { scp -q "${SSH_O[@]}" -i "$WORK/laptop-key" -P "$PORT" "$@"; }
wait_laptop() { # wait_laptop <qemu pid>
  for _ in $(seq 1 180); do
    on_laptop true 2>/dev/null && return 0
    kill -0 "$1" 2>/dev/null || die "the laptop VM exited before SSH came up"
    sleep 5
  done
  die "no SSH to the laptop VM on 127.0.0.1:$PORT after 15 minutes"
}
wait_gone() { for _ in $(seq 1 120); do kill -0 "$1" 2>/dev/null || return 0; sleep 2; done; kill "$1" 2>/dev/null || true; }

prepare_laptop() {
  local img="$WORK/$UBUNTU_IMG"
  say "the laptop VM: Ubuntu 24.04 cloud image"
  if [ -n "$UB_FILE" ]; then cp "$UB_FILE" "$img"; else curl -fsSL "$UBUNTU_BASE/$UBUNTU_IMG" -o "$img"; fi
  curl -fsSL "$UBUNTU_BASE/SHA256SUMS" -o "$WORK/SHA256SUMS"
  local want got
  want="$(grep " \*\?$UBUNTU_IMG\$" "$WORK/SHA256SUMS" | cut -d' ' -f1)"; got="$(sha256sum "$img" | cut -d' ' -f1)"
  [ -n "$want" ] && [ "$want" = "$got" ] || die "$UBUNTU_IMG sha256 $got does not match SHA256SUMS ($want)"
  qemu-img convert -O qcow2 "$img" "$WORK/laptop.qcow2"; rm -f "$img"
  qemu-img resize -q "$WORK/laptop.qcow2" 40G
  [ -f "$WORK/laptop-key" ] || ssh-keygen -q -t ed25519 -N '' -C lab-driver -f "$WORK/laptop-key"
  local s="$WORK/seed-laptop"; mkdir -p "$s"
  user_data "$s/user-data" lab "$(cat "$WORK/laptop-key.pub")"
  printf 'instance-id: lab-laptop-%s\nlocal-hostname: laptop\n' "$(date +%s)" > "$s/meta-data"
  printf 'version: 2\nethernets:\n  up:\n    match: {macaddress: "%s"}\n    dhcp4: true\n' "$LAPTOP_MAC_UP" > "$s/network-config"
  seed_iso "$WORK/laptop-seed.iso" "$s"
  say "boot it with outside network (before the clock)"
  qemu-system-x86_64 -machine "q35,accel=$ACCEL" -cpu "$CPU" -m 6144 -smp 4 -display none \
    -serial "file:$WORK/laptop-prepare.console" \
    -drive "file=$WORK/laptop.qcow2,if=virtio,format=qcow2,discard=unmap" \
    -drive "file=$WORK/laptop-seed.iso,if=virtio,format=raw,readonly=on" \
    -netdev "user,id=up,hostfwd=tcp:127.0.0.1:$PORT-:22" -device "virtio-net-pci,netdev=up,mac=$LAPTOP_MAC_UP" &
  local q=$!; PIDS="$PIDS $q"
  wait_laptop "$q"
  local commit; commit="$(git -C "$REPO" rev-parse HEAD)"
  git -C "$REPO" bundle create -q "$WORK/src.bundle" HEAD
  to_laptop "$WORK/src.bundle" "$HERE/laptop-provision.sh" lab@127.0.0.1:/tmp/
  say "provision: tutorial 00 section 1 on the laptop VM, the checkout at $commit"
  on_laptop "bash /tmp/laptop-provision.sh /tmp/src.bundle $commit && rm -f /tmp/src.bundle /tmp/laptop-provision.sh" \
    2>&1 | tee "$WORK/laptop-provision.log" || die "laptop provisioning failed (laptop-provision.log)"
  scp -q "${SSH_O[@]}" -i "$WORK/laptop-key" -P "$PORT" lab@127.0.0.1:.ssh/id_ed25519.pub "$WORK/laptop-admin.pub"
  echo "$commit" > "$WORK/laptop-commit"
  on_laptop 'sudo cloud-init clean --logs >/dev/null; sudo systemctl poweroff' >/dev/null 2>&1 || true
  wait_gone "$q"
  say "done: $WORK/laptop.qcow2 (checkout $commit)"
}

run() {
  [ -f "$WORK/laptop.qcow2" ] || die "no $WORK/laptop.qcow2: run prepare-laptop first"
  [ -n "$IMAGE" ] && [ -f "$IMAGE" ] || die "--image names the goatlite prepared image (ops/vm-image/build.sh)"
  [ -n "$RECORD" ] || die "--record names the directory for the run's record"
  mkdir -p "$RECORD"; RECORD="$(cd "$RECORD" && pwd)"
  IMAGE="$(cd "$(dirname "$IMAGE")" && pwd)/$(basename "$IMAGE")"
  local r="$WORK/run"; rm -rf "$r"; mkdir -p "$r"
  qemu-img create -q -f qcow2 -F qcow2 -b "$WORK/laptop.qcow2" "$r/laptop.qcow2"
  qemu-img create -q -f qcow2 -F qcow2 -b "$IMAGE" "$r/box.qcow2"
  local s="$r/seed-box"; mkdir -p "$s"
  user_data "$s/user-data" ubuntu "$(cat "$WORK/laptop-admin.pub")"
  printf 'instance-id: lab-box-%s\nlocal-hostname: goatlite\n' "$(date +%s)" > "$s/meta-data"
  cat > "$s/network-config" <<EOF
version: 2
ethernets:
  lan:
    match: {macaddress: "$BOX_MAC_LAN"}
    addresses: [10.88.0.2/24, "fd88::2/64"]
    routes: [{to: default, via: 10.88.0.1}, {to: "::/0", via: "fd88::1"}]
    nameservers: {addresses: [10.88.0.1]}
EOF
  seed_iso "$r/box-seed.iso" "$s"
  # A seed for the laptop with no network section: its network is the one
  # laptop-provision.sh wrote, and cloud-init's network config is disabled.
  local sl="$r/seed-laptop"; mkdir -p "$sl"
  user_data "$sl/user-data" lab "$(cat "$WORK/laptop-key.pub")"
  printf 'instance-id: lab-laptop-run-%s\nlocal-hostname: laptop\n' "$(date +%s)" > "$sl/meta-data"
  printf 'version: 2\n' > "$sl/network-config"
  seed_iso "$r/laptop-seed.iso" "$sl"

  say "boot the laptop and the box on the private segment (accel=$ACCEL)"
  local sock="$r/lan.sock"
  qemu-system-x86_64 -machine "q35,accel=$ACCEL" -cpu "$CPU" -m "$LAPTOP_MEM" -smp "$CPUS" -display none \
    -serial "file:$RECORD/laptop.console" \
    -drive "file=$r/laptop.qcow2,if=virtio,format=qcow2" -drive "file=$r/laptop-seed.iso,if=virtio,format=raw,readonly=on" \
    -netdev "user,id=up,restrict=on,ipv6-net=fec0::/64,hostfwd=tcp:127.0.0.1:$PORT-:22" \
    -device "virtio-net-pci,netdev=up,mac=$LAPTOP_MAC_UP" -object "filter-dump,id=dup,netdev=up,file=$RECORD/laptop-up.pcap" \
    -netdev "stream,id=lan,server=on,addr.type=unix,addr.path=$sock" \
    -device "virtio-net-pci,netdev=lan,mac=$LAPTOP_MAC_LAN" -object "filter-dump,id=dlan,netdev=lan,file=$RECORD/segment.pcap" &
  local ql=$!; PIDS="$PIDS $ql"
  for _ in $(seq 1 100); do [ -S "$sock" ] && break; sleep 0.2; done
  [ -S "$sock" ] || die "the laptop VM did not open the segment socket"
  qemu-system-x86_64 -machine "q35,accel=$ACCEL" -cpu "$CPU" -m "$BOX_MEM" -smp "$CPUS" -display none \
    -serial "file:$RECORD/box.console" \
    -drive "file=$r/box.qcow2,if=virtio,format=qcow2" -drive "file=$r/box-seed.iso,if=virtio,format=raw,readonly=on" \
    -netdev "stream,id=lan,server=off,addr.type=unix,addr.path=$sock" -device "virtio-net-pci,netdev=lan,mac=$BOX_MAC_LAN" &
  local qb=$!; PIDS="$PIDS $qb"
  wait_laptop "$ql"
  for _ in $(seq 1 120); do
    on_laptop "ssh -o StrictHostKeyChecking=accept-new -o ConnectTimeout=5 -o BatchMode=yes ubuntu@10.88.0.2 'cloud-init status --wait >/dev/null 2>&1; true'" 2>/dev/null && break
    kill -0 "$qb" 2>/dev/null || die "the box VM exited before SSH came up (box.console)"
    sleep 5
  done
  on_laptop "ssh -o BatchMode=yes ubuntu@10.88.0.2 true" || die "the laptop cannot reach the box over SSH"

  local zone="$NAME.lan"
  say "goatlite.yaml, and the hosts line of tutorial 00 section 5 step 1"
  on_laptop "cat > ~/goatcore/goatlite.yaml" <<EOF
schema: goatbuilder/v1
preset: lite
lite:
  name: $NAME
  host: 10.88.0.2
  ssh_user: ubuntu
  admin: admin@example.org
  disk_encryption: false
EOF
  on_laptop "echo '10.88.0.2 dashboard.$NAME.$zone auth.$NAME.$zone oauth2.$NAME.$zone portal.$NAME.$zone mgmt.$NAME.$zone netbird.$zone' | sudo tee -a /etc/hosts >/dev/null"
  to_laptop "$HERE/phase11-answer.mjs" "$HERE/laptop-phase11.sh" lab@127.0.0.1:/tmp/

  # The laptop's own housekeeping would reach outside for reasons of the laptop, not of
  # the goatnet: unattended upgrades, the apt timers, the MOTD news, the firmware-update
  # check, the snap store and the clock sync are stopped before the clock. The box is
  # left as the image built it.
  on_laptop "sudo systemctl disable --now unattended-upgrades.service apt-daily.timer apt-daily-upgrade.timer motd-news.timer fwupd-refresh.timer >/dev/null 2>&1; sudo systemctl mask --now snapd.service snapd.socket snapd.seeded.service systemd-timesyncd.service >/dev/null 2>&1; true"

  local rc=0 t0 t1
  # Every command on the laptop runs in a login shell, so the tutorial's environment
  # (the PATH, the Python environment) is in force, from the root of the checkout.
  gb() { on_laptop "bash -lc 'cd ~/goatcore && timeout $STEP_TIMEOUT $*'" ; }
  t0="$(date +%s.%N)"; echo "$t0" > "$RECORD/clock-start"
  say "the clock starts: goatbuilder standup $NAME"
  gb "goatbuilder $GB_FLAGS standup $NAME --ack zero-saas-lab --yes" 2>&1 | tee "$RECORD/standup-1.log" || rc=$?
  if [ "$rc" = 0 ] && grep -q "OPERATOR STEP" "$RECORD/standup-1.log"; then
    say "phase 11: the lab's stand-in for the person (laptop-phase11.sh, lab only)"
    on_laptop "bash -lc 'GB_FLAGS=\"$GB_FLAGS\" bash /tmp/laptop-phase11.sh $NAME $zone admin@example.org /tmp/phase11-answer.mjs'" 2>&1 \
      | tee "$RECORD/phase11.log" || rc=$?
    if [ "$rc" = 0 ]; then
      gb "goatbuilder $GB_FLAGS standup $NAME --site $NAME --from 12 --ack zero-saas-lab --yes" 2>&1 | tee "$RECORD/standup-12.log" || rc=$?
    fi
  elif [ "$rc" = 0 ]; then
    echo "lab: the standup did not stop at phase 11; nothing to answer"
  fi
  if [ "$rc" = 0 ]; then
    say "verify and status"
    gb "goatbuilder $GB_FLAGS verify --env $NAME" 2>&1 | tee "$RECORD/verify.log" || rc=$?
    gb "goatbuilder $GB_FLAGS status --env $NAME" 2>&1 | tee "$RECORD/status.log" || true
  fi
  t1="$(date +%s.%N)"; echo "$t1" > "$RECORD/clock-stop"
  local stand=$rc
  say "the clock stops (standup and verify exit $stand); collect, then power off"
  on_laptop "bash -lc 'cat ~/.goat/state/standup-$NAME-timings.tsv 2>/dev/null'" > "$RECORD/timings.tsv" || true
  # The box first: it is reached through the laptop.
  on_laptop "ssh -o BatchMode=yes ubuntu@10.88.0.2 'sudo systemctl poweroff'" >/dev/null 2>&1 || true
  wait_gone "$qb"
  on_laptop 'sudo systemctl poweroff' >/dev/null 2>&1 || true
  wait_gone "$ql"
  # A throwaway goatnet's setup key and tokens open nothing outside the lab, but the
  # record is shared: every value shaped like a key or token is masked.
  for f in "$RECORD"/*.log; do
    sed -E -i 's/[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}/<uuid-masked>/g; s/nbp_[A-Za-z0-9]+/<token-masked>/g' "$f"
  done
  say "the judge: every packet of the clock's window"
  local jrc=0
  python3 "$HERE/judge.py" --lab "$LAB_NETS" --zone "$zone" --from "$t0" --to "$t1" \
    --out "$RECORD/verdict.json" "$RECORD/laptop-up.pcap" "$RECORD/segment.pcap" > /dev/null || jrc=$?
  python3 - "$RECORD" "$stand" "$jrc" "$t0" "$t1" "$(git -C "$REPO" rev-parse HEAD)" "$(cat "$WORK/laptop-commit")" "$ACCEL" <<'PY'
import json, sys
rec, stand, jrc, t0, t1, head, laptop, accel = sys.argv[1:9]
v = json.load(open(f"{rec}/verdict.json"))
summary = {
    "schema": "goatlite-zero-saas-run/v1",
    "standup_and_verify_exit": int(stand),
    "judge_exit": int(jrc),
    "result": "pass" if int(stand) == 0 and int(jrc) == 0 else "fail",
    "clock_seconds": round(float(t1) - float(t0)),
    "commit": head, "laptop_checkout": laptop, "accel": accel,
    "outside_destinations": len(v["outside_packets"]), "outside_names": len(v["outside_names"]),
}
json.dump(summary, open(f"{rec}/summary.json", "w"), indent=2)
print(json.dumps(summary, indent=2))
print("outside packets:", json.dumps(v["outside_packets"], indent=2))
print("outside names:", json.dumps(v["outside_names"], indent=2))
PY
  if [ "$stand" != 0 ]; then exit 3; fi
  [ "$jrc" = 0 ] || exit 1
}

case "$CMD" in
  prepare-laptop) prepare_laptop ;;
  run) run ;;
  *) die "usage: lab.sh prepare-laptop|run --work DIR ..." ;;
esac
