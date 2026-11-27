#!/usr/bin/env bash
# run.sh: one machine of a target type, from a published Ubuntu cloud image to
# `doctor --target`, in a VM (gate J1; docs/design/goatbuilder-build-targets.md
# section 3.6). The real goatbuilder and the real plays run on a fresh clone of
# this commit whose only goatnet is synthetic.
#
#   ops/e2e/vm/run.sh <type> --goatbuilder BIN [--arch amd64|arm64]
#                     [--ubuntu 24.04] [--prepared] [--work DIR] [--record DIR] [--keep]
#
# A VM here has no goatnet to join: there is no founder in it to enrol with, so
# join-node's phases 3 and 7 to 12 and 15 are not run. The record names every
# phase not run and why; a pass is a pass of what ran and of nothing else.
# types/<type>.sh says what a type's machine needs and what its checks must read.
#
# KVM when the VM's architecture is this machine's and /dev/kvm is usable, else
# emulation: the record's `accel` says which. Emulated arm64 shows that the arm64
# packages and the plays work; it shows no timing.
#
# --prepared boots the image prepare-image.sh made from the same published one,
# held to its recorded digest: its packages are already upgraded.
#
# Writes <record>/result.json, one log per step and the VM's console. Exit 0 when
# every step read as expected, 1 otherwise, 2 for a usage error or a missing tool.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../../.." && pwd)"
TYPE=""; GB=""; VM_ARCH=amd64; VM_UBUNTU=24.04; WORK=""; RECORD=""; KEEP=0
while [ $# -gt 0 ]; do
  case "$1" in
    --goatbuilder) GB="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"; shift 2 ;;
    --arch) VM_ARCH="$2"; shift 2 ;;
    --ubuntu) VM_UBUNTU="$2"; shift 2 ;;
    --work) WORK="$2"; shift 2 ;;
    --record) RECORD="$2"; shift 2 ;;
    --keep) KEEP=1; shift ;;
    --prepared) export VM_PREPARED=yes; shift ;;
    -h|--help) sed -n '2,25p' "$0"; exit 0 ;;
    -*) echo "vm harness: unknown argument $1" >&2; exit 2 ;;
    *) TYPE="$1"; shift ;;
  esac
done
[ -n "$TYPE" ] && [ -f "$HERE/types/$TYPE.sh" ] || { echo "vm harness: name a type that has ops/e2e/vm/types/<type>.sh: $(cd "$HERE/types" && printf '%s ' *.sh | sed 's/\.sh//g')" >&2; exit 2; }
[ -n "$GB" ] && [ -x "$GB" ] || { echo "vm harness: --goatbuilder names a goatbuilder binary" >&2; exit 2; }
for t in qemu-img curl ssh ssh-keygen git ansible-playbook python3 sha256sum; do
  command -v "$t" >/dev/null 2>&1 || { echo "vm harness: $t is needed" >&2; exit 2; }
done
command -v cloud-localds >/dev/null 2>&1 || command -v xorriso >/dev/null 2>&1 || { echo "vm harness: cloud-localds or xorriso is needed" >&2; exit 2; }
[ -n "$WORK" ] || WORK="$(mktemp -d "${TMPDIR:-/tmp}/goat-vm.XXXXXX")"
mkdir -p "$WORK"; WORK="$(cd "$WORK" && pwd)"
[ -n "$RECORD" ] || RECORD="$WORK/record"
mkdir -p "$RECORD"; RECORD="$(cd "$RECORD" && pwd)"
export VM_WORK="$WORK/vm" VM_ARCH VM_UBUNTU VM_IMAGES="${VM_IMAGES:-$HOME/.cache/goat-vm-images}" VM_MEM="${VM_MEM:-2048}" VM_CPUS="${VM_CPUS:-2}" VM_TAPS=0 VM_NAME=vm-node-1 VM_BOOT_WAIT="${VM_BOOT_WAIT:-300}"
# An emulated machine answers SSH slowly, and not at all for minutes while a package installs.
[ "$VM_ARCH" != arm64 ] || { VM_BOOT_WAIT=1800; export ANSIBLE_TIMEOUT=180 ANSIBLE_SSH_RETRIES=5 ANSIBLE_SSH_ARGS="-o ControlMaster=auto -o ControlPersist=600s -o ServerAliveInterval=60 -o ServerAliveCountMax=20 -o ConnectTimeout=180"; }
VM_PORT="${VM_PORT:-$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')}"
mkdir -p "$VM_WORK" "$VM_IMAGES"
# shellcheck source=ops/e2e/vm/vm.sh
. "$HERE/vm.sh"
STEPS="$RECORD/steps.jsonl"; : > "$STEPS"; FAILS=0; NOT_RUN=()
say() { printf '\n==> %s\n' "$*"; }
# step <name> <exit> <says|!says>... -- <command...>
step() {
  local name="$1" want="$2" rc=0 bad="" t0 w; shift 2
  local -a words=()
  while [ "$1" != -- ]; do words+=("$1"); shift; done
  shift; t0=$(date +%s)
  "$@" > "$RECORD/$name.log" 2>&1 </dev/null || rc=$?
  [ "$rc" = "$want" ] || bad="exit $rc, not $want"
  for w in ${words[@]+"${words[@]}"}; do
    case "$w" in
      !*) if grep -qF -- "${w#!}" "$RECORD/$name.log"; then bad="$bad; says: ${w#!}"; fi ;;
      *) grep -qF -- "$w" "$RECORD/$name.log" || bad="$bad; does not say: $w" ;;
    esac
  done
  python3 -c 'import json,sys; print(json.dumps({"step": sys.argv[1], "ok": sys.argv[2] == "", "why": sys.argv[2], "exit": int(sys.argv[3]), "seconds": int(sys.argv[4])}))' "$name" "$bad" "$rc" "$(( $(date +%s) - t0 ))" >> "$STEPS"
  if [ -z "$bad" ]; then echo "PASS $name ($(( $(date +%s) - t0 )) s)"; else echo "FAIL $name: $bad"; tail -25 "$RECORD/$name.log" | cut -c1-220 | sed 's/^/     | /'; FAILS=$((FAILS + 1)); fi
}
not_run() { NOT_RUN+=("$1"); }

# The goatnet this clone has: one founder site, and the one machine, on the VM's address.
NODE=vm-node-1; SITE=zagros; ENV=zagros; GROUP="zagros_${TYPE//-/_}_hosts"; HOST_VARS=""; GROUP_VARS=""
# A type file may give the goatnet release components (fleet-pins rows, indented as
# components' entries) and the pins rendered for the machine's group. PUBLIC names
# rows of this checkout's own ops/pins/fleet-pins.yaml to keep as they are, each
# rendered for the group at its fleet pin; OPT_IN, the opt-in components the
# goatnet lists.
COMPONENTS=""; GROUP_PINS=""; PUBLIC=""; OPT_IN=""
# A type file may say its checkout doctor exits otherwise, and what it must show.
DOCTOR_CHECKOUT_EXIT=0; DOCTOR_CHECKOUT_SAYS=()
# shellcheck disable=SC1090 # the type's own file
. "$HERE/types/$TYPE.sh"

export HOME_REAL="$HOME"; export HOME="$WORK/home"; mkdir -p "$HOME/.ssh" "$WORK/bin" "$WORK/tmp"; chmod 700 "$HOME/.ssh"
export TMPDIR="$WORK/tmp" GOAT_MESH_PROBE=0 GOAT_DOOR_OPEN=0 GOAT_AWS_MODE=none GOAT_SECRET_STORE=env GOAT_NO_FUN=1 GOAT_EXPLAIN=0 GOAT_ANSIBLE_TRUNK_REMOTE=origin GIT_LFS_SKIP_SMUDGE=1
export ANSIBLE_HOST_KEY_CHECKING=False ANSIBLE_CACHE_PLUGIN_CONNECTION="$WORK/facts" ANSIBLE_SSH_CONTROL_PATH_DIR="$WORK/cp" GOATBUILDER_BIN="$GB"
# The operator's machine already installs collections under its own home; an isolated home still reads them.
export ANSIBLE_COLLECTIONS_PATH="${ANSIBLE_COLLECTIONS_PATH:-$HOME_REAL/.ansible/collections:/usr/share/ansible/collections}"
trap 'rc=$?; if [ "$KEEP" = 0 ]; then vm_down; else echo "kept: ssh -i $VM_WORK/key -p $VM_PORT goatvm@127.0.0.1"; fi; cp "$VM_WORK/console.log" "$RECORD/" 2>/dev/null || true; exit $rc' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

say "the VM: Ubuntu $VM_UBUNTU $VM_ARCH"
T0=$(date +%s)
vm_up || exit 1
BOOT=$(( $(date +%s) - T0 ))
echo "up in $BOOT s, accel=$VM_ACCEL, image sha256 $VM_IMAGE_SHA256, booted from sha256 $VM_DISK_SHA256, its checksums signed by Ubuntu's key: $VM_SIGNED"
# Every ssh and every play reach the VM by one name, through the run's own configuration.
printf 'Host %s\n  HostName 127.0.0.1\n  Port %s\n  User goatvm\nHost *\n  IdentityFile %s\n  IdentitiesOnly yes\n  StrictHostKeyChecking no\n  UserKnownHostsFile /dev/null\n  LogLevel ERROR\n' "$NODE" "$VM_PORT" "$VM_WORK/key" > "$HOME/.ssh/config"
chmod 600 "$HOME/.ssh/config"
REAL_SSH="$(command -v ssh)"
printf '#!/bin/sh\nexec "%s" -F "%s" "$@"\n' "$REAL_SSH" "$HOME/.ssh/config" > "$WORK/bin/ssh"; chmod +x "$WORK/bin/ssh"
export PATH="$WORK/bin:$PATH"

say "source of record: a bare remote and a clone of $(git -C "$REPO" rev-parse --short HEAD), one synthetic goatnet"
ORIGIN="$WORK/origin.git"; CLONE="$WORK/goat"
git clone -q --bare "$REPO" "$ORIGIN"
git -C "$ORIGIN" update-ref refs/heads/main "$(git -C "$REPO" rev-parse HEAD)"
git -C "$ORIGIN" symbolic-ref HEAD refs/heads/main
git clone -q --branch main "$ORIGIN" "$CLONE"
python3 - "$CLONE" "$ENV" "$GROUP" "$NODE" "$(cat "$VM_WORK/key.pub")" "$COMPONENTS" "$GROUP_PINS" "$PUBLIC" "$OPT_IN" <<'PY'
import os, sys, yaml
clone, env, group, node, key, components, group_pins, public, opt_in = sys.argv[1:10]
table = yaml.safe_load(open(os.path.join(clone, "ops/pins/fleet-pins.yaml"))) if public else {}
for name in public.split():
    row = table["components"][name]
    components += "".join("  %s\n" % l for l in yaml.safe_dump({name: row}, sort_keys=False).splitlines())
    group_pins += ("\n" if group_pins else "") + "%s: \"%s\"" % (row["var"], row["fleet"])
def put(p, t):
    p = os.path.join(clone, p); os.makedirs(os.path.dirname(p), exist_ok=True); open(p, "w").write(t)
put("ops/goatbuilder/goatbuilder.yaml", "schema: goatbuilder/v1\nrepo: {source: example-org/example-repo, image_namespace: ghcr.io/example-org}\ncomponents:\n  netbird-mgmt: {declared_in: ops/pins/fleet-pins.yaml}\nenvironments:\n  - name: %s\n    converger: push-ansible\npromotion:\n  path:\n    - {name: %s, kind: rung, environment: %s}\ndeploy: {ssh_user: goatvm}\n" % (env, env, env))
put("ops/pins/fleet-pins.yaml", "goatnets:\n  %s:\n    members: [%s_hosts, %s]\n%scomponents:\n  svc:\n    var: svc_image\n    fleet: null\n%s" % (env, env, group, ("    opt_in: [%s]\n" % ", ".join(opt_in.split())) if opt_in else "", components))
if group_pins:
    put("ops/ansible/group_vars/%s/pins.yml" % group, group_pins + "\n")
put("ops/ansible/group_vars/%s_hosts/vars.yml" % env, "goatnet_founder: true\n")
inv = "[%s_hosts]\nmgmt-%s-01-a\n[%s]\n%s ansible_host=%s ansible_user=goatvm ansible_python_interpreter=/usr/bin/python3\n" % (env, env, group, node, node)
put("ops/ansible/inventory/mgmt-hosts.ini", inv)
put("ops/ansible/inventory/mgmt-hosts-mesh.ini", "[%s]\n%s ansible_host=%s\n" % (group, node, node))
put("ops/ansible/group_vars/%s/key.yml" % group, "admin_authorized_keys:\n  - \"%s\"\n" % key)
PY
[ -z "$HOST_VARS" ] || { mkdir -p "$CLONE/ops/ansible/host_vars"; printf '%s\n' "$HOST_VARS" > "$CLONE/ops/ansible/host_vars/$NODE.yml"; }
[ -z "$GROUP_VARS" ] || printf '%s\n' "$GROUP_VARS" > "$CLONE/ops/ansible/group_vars/$GROUP/vars.yml"
# A type file may write more of the goatnet's own files into the clone (an overlay's data).
! declare -F type_tree >/dev/null || type_tree "$CLONE"
git -C "$CLONE" add -A
git -C "$CLONE" -c user.name="vm harness" -c user.email=vm@localhost -c commit.gpgsign=false commit -q -m "vm harness: a synthetic goatnet"
git -C "$CLONE" push -q origin HEAD:main
cd "$CLONE"
JOIN=("$GB" --yes join-node "$ENV" --site "$SITE" --node "$NODE" --class "$TYPE" --address "$NODE")
DOCTOR=("$GB" doctor --target "$TYPE" --site "$SITE" --node "$NODE" --all)
# acceptance <row,row> [key=value]...: those rows of ops/validation/acceptance-tests.yaml,
# run against this machine by the runner phase 15 uses; the values are what
# generate-values would read from a site that declares the machine.
acceptance() {
  local only="$1" kv; shift
  local -a vars=()
  for kv in "$@"; do vars+=(--var "$kv"); done
  python3 ops/validation/run-acceptance.py --env "$ENV" --only "$only" ${vars[@]+"${vars[@]}"}
}

say "cases"
# A type that requires an encrypted scope reads UNKNOWN for it from the checkout
# (never a pass), so its checkout doctor exits 1; its file says so.
step doctor-checkout "$DOCTOR_CHECKOUT_EXIT" "$TYPE at $SITE" ${DOCTOR_CHECKOUT_SAYS[@]+"${DOCTOR_CHECKOUT_SAYS[@]}"} -- "${DOCTOR[@]}"
step plan 0 "Phase 1 " "Phase 15 " "PLAN complete" -- "${JOIN[@]}" --dry-run
step phases-1-2 0 "Phase 1 " "Phase 2 " -- "${JOIN[@]}" --from 1 --to 2
not_run "phase 3: the config preflight reads a whole goatnet's group_vars, and this one is one machine"
step phase-4 0 "Phase 4 " -- "${JOIN[@]}" --from 4 --to 4
step phase-5-baseline 0 "Phase 5 " "!fatal:" -- "${JOIN[@]}" --from 5 --to 5
for _i in $(seq 1 60); do vm_ssh true 2>/dev/null && break; sleep 5; done   # the baseline may reboot it
step phase-6-kmod 0 "Phase 6 " -- "${JOIN[@]}" --from 6 --to 6
not_run "phases 7 to 12: enrolment needs a goatnet's mesh control plane, and nothing here runs one"
type_cases
not_run "phase 14: it reaches the machine at a mesh address, which needs phase 10"
not_run "phase 15: the acceptance rows read a whole site"

python3 - "$RECORD" "$TYPE" "$VM_ARCH" "$VM_UBUNTU" "$VM_ACCEL" "$VM_IMAGE_SHA256" "$VM_DISK_SHA256" "$VM_SIGNED" "$BOOT" "$(git -C "$REPO" rev-parse HEAD)" "$("$GB" --version 2>/dev/null | head -1)" "${NOT_RUN[@]}" <<'PY'
import json, os, sys
rec, typ, arch, ubuntu, accel, sha, disk, signed, boot, commit, gb = sys.argv[1:12]
steps = [json.loads(l) for l in open(os.path.join(rec, "steps.jsonl"))]
json.dump({"schema": "goat-vm-type-run/v1", "type": typ, "arch": arch, "ubuntu": ubuntu, "accel": accel,
           "image_sha256": sha, "disk_sha256": disk, "prepared_image": disk != sha, "image_signature_checked": signed == "yes", "boot_seconds": int(boot),
           "goatcore": commit, "goatbuilder": gb, "steps": steps, "not_run": sys.argv[12:],
           "passed": all(s["ok"] for s in steps)}, open(os.path.join(rec, "result.json"), "w"), indent=1)
PY
for n in "${NOT_RUN[@]}"; do echo "NOT RUN $n"; done
echo "vm harness: $TYPE on $VM_ARCH ($VM_ACCEL): $(grep -c '"ok": true' "$STEPS") of $(wc -l < "$STEPS" | tr -d ' ') steps as expected; record $RECORD/result.json"
[ "$FAILS" = 0 ]
