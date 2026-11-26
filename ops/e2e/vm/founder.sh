#!/usr/bin/env bash
# founder.sh: a goatnet's founder and one more machine, in VMs, for the phases
# run.sh cannot run because nothing there can enrol a machine (gates J1 and J3;
# ops/e2e/vm/README.md).
#
#   founder.sh --work DIR --image GOATLITE.qcow2 --record DIR [--phase-11-stand-in] [--keep]
#
# DIR/laptop.qcow2 is `ops/vm-image/zero-saas/lab.sh prepare-laptop --work DIR`;
# the image is ops/vm-image/build.sh's. Three VMs on one private segment, which is the
# box's and the node's only network, as a site's LAN is:
#   laptop 10.88.0.1   runs goatbuilder, from the checkout it was prepared with, and is
#                      the segment's way out, for packages
#   box    10.88.0.2   the founder: a goatlite
#   node   10.88.0.3   a published Ubuntu cloud image, which joins
#
# The founder stands up to phase 10. Phase 11 is a person's step. By default it is
# the DECLARED STOP: nothing here answers it, and what needs an administrator after
# it is not run. With --phase-11-stand-in the lab's stand-in for that person answers
# it (ops/vm-image/zero-saas/laptop-phase11.sh: a browser signs in with the one-time
# password, the phase-10 re-run promotes the account, the service user and its token
# are made in the dashboard and stored under the declared posture; LAB ONLY, and the
# record names it), the founder stands up from phase 12, and the node runs every
# join-node phase, then doctor --target. The node runs join-node phase by phase.
# Each phase is recorded with its result; the phases not run are named, with the
# reason.
#
# FOUNDER_HOOK names a command to run on this machine after the VMs are up and before
# the standup, for an experiment; the record names it. A run with one is not a run of
# the checkout as it is.
#
# Needs Linux, qemu, KVM (emulation is refused: it would show nothing in the time),
# about 14 GB of memory. No root. Exit 0: every phase that ran passed; 1: one did
# not; 2: usage or environment.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
WORK=""; IMAGE=""; RECORD=""; KEEP=0; ANSWER11=0
while [ $# -gt 0 ]; do
  case "$1" in
    --work) WORK="$2"; shift 2 ;;
    --image) IMAGE="$2"; shift 2 ;;
    --record) RECORD="$2"; shift 2 ;;
    --keep) KEEP=1; shift ;;
    --phase-11-stand-in) ANSWER11=1; shift ;;
    -h|--help) sed -n '2,36p' "$0"; exit 0 ;;
    *) echo "founder: unknown argument $1" >&2; exit 2 ;;
  esac
done
die() { echo "founder: $*" >&2; exit 2; }
[ -n "$WORK" ] && [ -f "$WORK/laptop.qcow2" ] || die "--work holds laptop.qcow2 (ops/vm-image/zero-saas/lab.sh prepare-laptop --work DIR)"
[ -n "$IMAGE" ] && [ -f "$IMAGE" ] || die "--image names the goatlite image (ops/vm-image/build.sh)"
[ -n "$RECORD" ] || die "--record names the directory for the run's record"
for t in qemu-system-x86_64 qemu-img ssh scp curl python3 sha256sum xorriso; do command -v "$t" >/dev/null || die "$t is needed"; done
[ -r /dev/kvm ] && [ -w /dev/kvm ] || die "no usable /dev/kvm"
WORK="$(cd "$WORK" && pwd)"; IMAGE="$(cd "$(dirname "$IMAGE")" && pwd)/$(basename "$IMAGE")"
mkdir -p "$RECORD"; RECORD="$(cd "$RECORD" && pwd)"
BOX_MEM="${FOUNDER_BOX_MB:-8192}"; LAPTOP_MEM="${FOUNDER_LAPTOP_MB:-2560}"; NODE_MEM="${FOUNDER_NODE_MB:-3072}"
PORT="${FOUNDER_SSH_PORT:-2331}"; SEG="${FOUNDER_SEGMENT_PORT:-23310}"; NET=zagros; NODE=zagros-node-2
R="$WORK/founder"; rm -rf "$R"; mkdir -p "$R"
export VM_WORK="$R" VM_ARCH=amd64 VM_UBUNTU=24.04 VM_IMAGES="${VM_IMAGES:-$HOME/.cache/goat-vm-images}"
# shellcheck source=ops/e2e/vm/vm.sh
. "$HERE/vm.sh"
vm_image || die "the node's cloud image"
SSH_O=(-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=10 -o ServerAliveInterval=30)
on_laptop() { ssh "${SSH_O[@]}" -i "$WORK/laptop-key" -p "$PORT" lab@127.0.0.1 "$@"; }
PIDS=()
finish() {
  local rc=$?
  # A throwaway goatnet's keys open nothing outside the run, but the record is read by others.
  for f in "$RECORD"/*.log; do [ ! -f "$f" ] || sed -E -i 's/[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}/<uuid-masked>/g; s/nbp_[A-Za-z0-9]+/<token-masked>/g' "$f"; done
  if [ "$KEEP" = 1 ]; then echo "kept: ssh -i $WORK/laptop-key -p $PORT lab@127.0.0.1 (pids ${PIDS[*]:-none})"; else for p in "${PIDS[@]}"; do kill "$p" 2>/dev/null || true; done; fi
  exit "$rc"
}
trap finish EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

mac() { printf '52:54:00:00:%02x:%02x' "$1" "$2"; }   # opsec-allow: QEMU's own prefix
seed() { # seed <name> <user> <n>: the key is the laptop's, the LAN address 10.88.0.<n>
  local s="$R/seed-$1"; mkdir -p "$s"
  printf '#cloud-config\nhostname: %s\nusers:\n  - name: %s\n    sudo: "ALL=(ALL) NOPASSWD:ALL"\n    shell: /bin/bash\n    ssh_authorized_keys: ["%s"]\nssh_pwauth: false\nwrite_files:\n  - path: /etc/systemd/resolved.conf.d/out.conf\n    content: "[Resolve]\\nDNS=10.0.2.3\\nDomains=~.\\n"\nruncmd: [[systemctl, restart, systemd-resolved]]\n' "$1" "$2" "$(cat "$WORK/laptop-admin.pub")" > "$s/user-data"
  printf 'instance-id: %s-%s\nlocal-hostname: %s\n' "$1" "$(date +%s)" "$1" > "$s/meta-data"
  printf 'version: 2\nethernets:\n  lan:\n    match: {macaddress: "%s"}\n    set-name: lan0\n    addresses: [10.88.0.%s/24]\n    routes: [{to: default, via: 10.88.0.1}]\n' "$(mac "$3" 2)" "$3" > "$s/network-config"
  xorriso -as mkisofs -quiet -output "$R/$1-seed.iso" -volid cidata -joliet -rock "$s/user-data" "$s/meta-data" "$s/network-config" 2>/dev/null
}
boot() { # boot <name> <disk> <n> <memory> <cpus> <the way out, or nothing>
  local -a out=()
  [ -z "$6" ] || out=(-netdev "user,id=out,$6" -device "virtio-net-pci,netdev=out,mac=$(mac "$3" 1)")
  qemu-system-x86_64 -machine q35,accel=kvm -cpu host -m "$4" -smp "$5" -display none -serial "file:$RECORD/$1.console" \
    -drive "file=$2,if=virtio,format=qcow2" -drive "file=$R/$1-seed.iso,if=virtio,format=raw,readonly=on" \
    ${out[@]+"${out[@]}"} \
    -netdev "socket,id=lan,mcast=230.0.0.1:$SEG,localaddr=127.0.0.1" -device "virtio-net-pci,netdev=lan,mac=$(mac "$3" 2)" \
    > "$R/$1.qemu" 2>&1 &
  PIDS+=("$!")
}
qemu-img create -q -f qcow2 -F qcow2 -b "$WORK/laptop.qcow2" "$R/laptop.qcow2"
qemu-img create -q -f qcow2 -F qcow2 -b "$IMAGE" "$R/box.qcow2"
qemu-img create -q -f qcow2 -F qcow2 -b "$VM_IMAGE" "$R/node.qcow2" 30G
seed box ubuntu 2; seed node ubuntu 3
# The laptop's network is the one laptop-provision.sh wrote.
mkdir -p "$R/seed-laptop"
printf '#cloud-config\nusers:\n  - name: lab\n    sudo: "ALL=(ALL) NOPASSWD:ALL"\n    ssh_authorized_keys: ["%s"]\n' "$(cat "$WORK/laptop-key.pub")" > "$R/seed-laptop/user-data"
printf 'instance-id: laptop-%s\nlocal-hostname: laptop\n' "$(date +%s)" > "$R/seed-laptop/meta-data"; printf 'version: 2\n' > "$R/seed-laptop/network-config"
xorriso -as mkisofs -quiet -output "$R/laptop-seed.iso" -volid cidata -joliet -rock "$R/seed-laptop/user-data" "$R/seed-laptop/meta-data" "$R/seed-laptop/network-config" 2>/dev/null
boot laptop "$R/laptop.qcow2" 1 "$LAPTOP_MEM" 2 "hostfwd=tcp:127.0.0.1:$PORT-:22"
boot box "$R/box.qcow2" 2 "$BOX_MEM" 4 ""
boot node "$R/node.qcow2" 3 "$NODE_MEM" 2 ""
wait_for() { local _i; for _i in $(seq 1 120); do "$@" >/dev/null 2>&1 && return 0; sleep 5; done; die "not after 10 minutes: $*"; }
wait_for on_laptop true
# The laptop's own resolver is the box, which serves no name yet, and phase 5 builds
# a Go program: names outside resolve the way out.
on_laptop 'out=$(ip -o -4 addr | awk "/10.0.2.15/{print \$2}"); lan=$(ip -o -4 addr | awk "/10.88.0.1\//{print \$2}"); sudo resolvectl dns "$lan" ""; sudo resolvectl dns "$out" 10.0.2.3; sudo sysctl -qw net.ipv4.ip_forward=1; sudo iptables -t nat -A POSTROUTING -s 10.88.0.0/24 -o "$out" -j MASQUERADE; sudo iptables -P FORWARD ACCEPT'
for h in 2 3; do wait_for on_laptop "ssh -o StrictHostKeyChecking=accept-new -o ConnectTimeout=5 -o BatchMode=yes ubuntu@10.88.0.$h 'cloud-init status --wait >/dev/null 2>&1; true'"; done

STEPS="$RECORD/steps.jsonl"; : > "$STEPS"; FAILS=0; STOPPED=""; NOT_RUN=(); STAND_INS=()
# step <name> <command on the laptop>
step() {
  local rc=0 t0; t0=$(date +%s)
  on_laptop "bash -lc 'cd ~/goatcore && timeout ${FOUNDER_STEP_TIMEOUT:-3600} $2'" > "$RECORD/$1.log" 2>&1 </dev/null || rc=$?
  python3 -c 'import json,sys; print(json.dumps({"step": sys.argv[1], "ok": sys.argv[2] == "0", "exit": int(sys.argv[2]), "seconds": int(sys.argv[3])}))' "$1" "$rc" "$(( $(date +%s) - t0 ))" >> "$STEPS"
  if [ "$rc" = 0 ]; then echo "PASS $1 ($(( $(date +%s) - t0 )) s)"; else echo "FAIL $1: exit $rc"; tail -25 "$RECORD/$1.log" | cut -c1-240 | sed 's/^/     | /'; FAILS=$((FAILS + 1)); return 1; fi
}
zone="$NET.lan"
on_laptop "cat > ~/goatcore/goatlite.yaml" <<EOF
schema: goatbuilder/v1
preset: lite
lite:
  name: $NET
  host: 10.88.0.2
  ssh_user: ubuntu
  admin: admin@example.org
  disk_encryption: false
EOF
on_laptop "echo '10.88.0.2 dashboard.$NET.$zone auth.$NET.$zone oauth2.$NET.$zone portal.$NET.$zone mgmt.$NET.$zone netbird.$zone' | sudo tee -a /etc/hosts >/dev/null"
# What a person declares for a LAN site whose machines join a mesh: where they enrol,
# and the client they enrol with.
on_laptop "mkdir -p ~/goatcore/ops/ansible/group_vars/${NET}_hosts && cat > ~/goatcore/ops/ansible/group_vars/${NET}_hosts/zz-vm-founder.yml" <<EOF
mesh_enrollment_url: "https://netbird.$zone:443"
netbird_version: "0.75.1"
EOF
if [ -n "${FOUNDER_HOOK:-}" ]; then FOUNDER_PORT="$PORT" FOUNDER_KEY="$WORK/laptop-key" $FOUNDER_HOOK || die "FOUNDER_HOOK"; fi
step founder-to-10 "goatbuilder --offline standup $NET --to 10 --ack vm-founder --yes" || STOPPED="the founder did not reach phase 10"
[ "$ANSWER11" = 1 ] || NOT_RUN+=("standup phases 11 to 15: phase 11 is a person's step, the declared stop")

if [ -z "$STOPPED" ]; then
  # What a person declares before the node joins: its line, where it enrols, and the
  # configuration join-node reads (a goatlite has none).
  on_laptop "cd ~/goatcore && cat > ops/goatbuilder/goatbuilder.yaml && sed -i '/^mgmt-$NET-01 /a $NODE ansible_host=10.88.0.3 ansible_user=ubuntu site_name=$NET-b router_role=B signal_uri=netbird.$zone:443' ops/ansible/inventory/mgmt-hosts.ini" <<EOF
schema: goatbuilder/v1
repo: {source: example-org/example-repo, image_namespace: ghcr.io/example-org}
components:
  netbird-mgmt: {declared_in: ops/pins/fleet-pins.yaml}
environments:
  - {name: $NET, converger: push-ansible}
promotion:
  path:
    - {name: $NET, kind: rung, environment: $NET}
deploy: {ssh_user: ubuntu}
EOF
  # The site's names and its certificate authority, which a machine of the site has
  # from its LAN: the zone resolves on the box.
  for h in 2 3; do
    on_laptop "ssh ubuntu@10.88.0.$h 'sudo mkdir -p /etc/systemd/network/10-netplan-lan.network.d && printf \"[Network]\nDNS=10.88.0.2\nDomains=~$zone\n\" | sudo tee /etc/systemd/network/10-netplan-lan.network.d/dns.conf >/dev/null && sudo networkctl reload'; ssh ubuntu@10.88.0.$h 'sudo tee /usr/local/share/ca-certificates/$NET.crt >/dev/null && sudo update-ca-certificates >/dev/null 2>&1; sudo systemctl try-restart netbird 2>/dev/null || true' < ~/goatcore/ops/enrollment/ca/$NET/root-cert.pem"
  done
  # join-node gives the node the groups of the founder's own peer. A LAN standup skips
  # phase 8, which enrols the founder: its play, on the founder alone.
  step founder-enrol "env -C ops/ansible GOAT_AWS_MODE=none ../scripts/with-aws default -- ansible-playbook -i inventory/mgmt-hosts.ini -i inventory/mgmt-hosts-mesh.ini mesh-enroll-host.yml -e target=${NET}_hosts -l mgmt-$NET-01 -e mesh_ips_out=/tmp/founder-mesh-ips.json" || STOPPED="the founder's own enrolment"
  if [ -z "$STOPPED" ] && [ "$ANSWER11" = 1 ]; then
    # LAB STAND-IN for the person at standup phase 11; the record says so.
    step founder-11-stand-in "GB_FLAGS=--offline bash ops/vm-image/zero-saas/laptop-phase11.sh $NET $zone admin@example.org ops/vm-image/zero-saas/phase11-answer.mjs" || STOPPED="the phase 11 stand-in"
    STAND_INS+=("the person at standup phase 11: ops/vm-image/zero-saas/laptop-phase11.sh (lab only), which signs in at the dashboard with the one-time password, re-runs phase 10 once and makes the service user and its token in the dashboard")
    [ -n "$STOPPED" ] || step founder-from-12 "goatbuilder --offline standup $NET --from 12 --ack vm-founder --yes" || STOPPED="the founder from phase 12"
  fi
  # join-node gives the node the groups of the founder's peer, and phase 12 makes those
  # groups, after the person's step. Without the phase 11 stand-in, LAB STAND-IN: one
  # group, holding that peer, with no policy. The record says so.
  if [ -z "$STOPPED" ] && [ "$ANSWER11" = 0 ]; then
    on_laptop "ssh ubuntu@10.88.0.2 'sudo docker exec -i postgres-a psql -v ON_ERROR_STOP=1 -q -U postgres -d netbird'" <<'SQL' || STOPPED="the stand-in group"
INSERT INTO groups (id, account_id, name, issued) SELECT 'vm-founder-infra', id, 'VM-FOUNDER-INFRA', 'api' FROM accounts;
INSERT INTO group_peers (account_id, group_id, peer_id) SELECT account_id, 'vm-founder-infra', id FROM peers;
SQL
    STAND_INS+=("the group of the founder's peer, which standup phase 12 makes: one group with no policy, written to the database")
  fi
  LAST=12; [ "$ANSWER11" = 0 ] || LAST=15
  [ -n "$STOPPED" ] || for n in $(seq 1 "$LAST"); do
    step "join-$n" "goatbuilder --offline --yes join-node $NET --site $NET --node $NODE --class core --address 10.88.0.3 --from $n --to $n" || { STOPPED="join-node phase $n"; break; }
  done
  [ -n "$STOPPED" ] || [ "$ANSWER11" = 0 ] || step doctor-target "goatbuilder --offline doctor --target core --site $NET --node $NODE --all --live" || STOPPED="doctor --target core"
fi
if [ -z "$STOPPED" ] && [ "$ANSWER11" = 0 ]; then
  NOT_RUN+=("join-node phase 13: a core machine at the founder's site runs the site's bootstrap again, which needs the administrator of standup phase 11")
  NOT_RUN+=("join-node phase 14: it reaches the node at its mesh address, and the mesh carries nothing before standup phase 12 applies the declared ACL")
  NOT_RUN+=("join-node phase 15: its rows read a whole site, and this one stops before standup phase 11")
fi
ANSWER11="$ANSWER11" python3 - "$RECORD" "$STOPPED" "$VM_IMAGE_SHA256" "$(sha256sum "$IMAGE" | cut -d' ' -f1)" "$(cat "$WORK/laptop-commit")" "$(printf '%s\n' ${STAND_INS[@]+"${STAND_INS[@]}"})" ${NOT_RUN[@]+"${NOT_RUN[@]}"} <<'PY'
import json, os, sys
rec, stopped, node, box, commit, stand = sys.argv[1:7]
steps = [json.loads(l) for l in open(os.path.join(rec, "steps.jsonl"))]
json.dump({"schema": "goat-vm-founder-run/v1", "class": "core", "hook": os.environ.get("FOUNDER_HOOK", ""),
           "declared_stop": "none: a lab stand-in answered standup phase 11" if os.environ.get("ANSWER11") == "1" else "standup phase 11, a person's step",
           "goatcore": commit, "box_image_sha256": box, "node_image_sha256": node, "steps": steps,
           "stopped_at": stopped, "stand_ins": stand.split("\n") if stand else [], "not_run": sys.argv[7:], "passed": not stopped and all(s["ok"] for s in steps)},
          open(os.path.join(rec, "result.json"), "w"), indent=1)
PY
for n in ${NOT_RUN[@]+"${NOT_RUN[@]}"}; do echo "NOT RUN $n"; done
if [ "$ANSWER11" = 1 ]; then stop="phase 11 answered by the lab stand-in"; else stop="DECLARED STOP before standup phase 11"; fi
echo "founder: $(grep -c '"ok": true' "$STEPS") of $(wc -l < "$STEPS" | tr -d ' ') steps passed${STOPPED:+; stopped at $STOPPED}; $stop; record $RECORD/result.json"
[ "$FAILS" = 0 ] && [ -z "$STOPPED" ]
