#!/usr/bin/env bash
# founder.sh: a goatnet's founder and one more machine, in VMs, for the phases
# run.sh cannot run because nothing there can enrol a machine (gates J1 and J3;
# ops/e2e/vm/README.md).
#
#   founder.sh --work DIR --image GOATLITE.qcow2 --record DIR [--phase-11-stand-in [--joiner-site NAME]]
#              [--avenue cli|scripts] [--acceptance] [--assay] [--keep]
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
# join-node phase, then doctor --target. With --joiner-site the node is the first core
# machine of a new site that joins the founder's goatnet (the harness declares the site
# as a person would); without it, a second core machine at the founder's own site. The
# node runs join-node phase by phase.
# Each phase is recorded with its result; the phases not run are named, with the
# reason.
#
# --avenue picks who stands the founder up (ADR 1201: both avenues run the founder
# golden path): cli (the default) is `goatbuilder standup`; scripts is
# ops/scripts/goat-standup.sh, phase for phase the same plays; the stand-in's phase-10
# re-run takes the same avenue. --acceptance runs a goatlite's acceptance phase (13,
# --check on either avenue): gate row C1's founder clock ends when the standup prints
# its result. After it, `goatbuilder status` runs, for the record only, and the phase
# timings are copied into the record. --assay then does what tutorial 00 sections 6 and
# 7 have the person do to prove a goatlite finished: the laptop trusts the goatnet's
# root, the assay's values and probe identity are written, and `goatbuilder assay` runs;
# every applicable row PASS and none UNKNOWN is exit 0. It runs before the node is
# named, so nothing the harness gives the box later (its resolver, the root) is in it.
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
WORK=""; IMAGE=""; RECORD=""; KEEP=0; ANSWER11=0; JSITE=""; CLASS=core; AVENUE=cli; CHECK=""; ASSAY=0
while [ $# -gt 0 ]; do
  case "$1" in
    --work) WORK="$2"; shift 2 ;;
    --image) IMAGE="$2"; shift 2 ;;
    --record) RECORD="$2"; shift 2 ;;
    --keep) KEEP=1; shift ;;
    --phase-11-stand-in) ANSWER11=1; shift ;;
    --joiner-site) JSITE="$2"; shift 2 ;;
    --avenue) AVENUE="${2:-}"; shift 2 ;;
    --acceptance) CHECK=" --check"; shift ;;
    --assay) ASSAY=1; shift ;;
    -h|--help) sed -n '2,50p' "$0"; exit 0 ;;
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
NB_VERSION="0.75.1"   # the NetBird client every machine of the run is pinned to
NODE_N=3; NODE_ADDR="10.88.0.$NODE_N"   # the node VM: seed() gives it 10.88.0.<n>
PORT="${FOUNDER_SSH_PORT:-2331}"; SEG="${FOUNDER_SEGMENT_PORT:-23310}"; NET=zagros; NODE=zagros-node-2
case "$AVENUE" in
  cli) TO10="goatbuilder --offline standup $NET --to 10 --ack vm-founder --yes"; FROM12="goatbuilder --offline standup $NET --from 12 --ack vm-founder --yes$CHECK" ;;
  scripts) TO10="bash ops/scripts/goat-standup.sh $NET --offline --to 10"; FROM12="bash ops/scripts/goat-standup.sh $NET --offline --from 12$CHECK" ;;
  *) die "--avenue is cli or scripts" ;;
esac
UPN="$(printf '%s' "$NET" | tr '[:lower:]' '[:upper:]')"
# A joining site's node joins its own site; otherwise the node joins the founder's.
if [ -n "$JSITE" ]; then
  case "$JSITE" in *[!a-z0-9-]*|"$NET") die "--joiner-site names a new site: lower-case letters, digits and hyphens, not $NET" ;; esac
  [ "$ANSWER11" = 1 ] || die "--joiner-site needs --phase-11-stand-in: a joining site's node enrols only once the founder has stood up whole"
  NODE="$JSITE-$CLASS-1"; NODE_SITE="$JSITE"
else
  NODE_SITE="$NET"
fi
R="$WORK/founder"; rm -rf "$R"; mkdir -p "$R"
export VM_WORK="$R" VM_ARCH=amd64 VM_UBUNTU=24.04 VM_IMAGES="${VM_IMAGES:-$HOME/.cache/goat-vm-images}"
# shellcheck source=ops/e2e/vm/vm.sh
. "$HERE/vm.sh"
vm_image || die "the node's cloud image"
SSH_O=(-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=10 -o ServerAliveInterval=30)
on_laptop() { ssh "${SSH_O[@]}" -i "$WORK/laptop-key" -p "$PORT" lab@127.0.0.1 "$@"; }
PIDS=()
mask() { sed -E -i 's/[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}/<uuid-masked>/g; s/nbp_[A-Za-z0-9]+/<token-masked>/g' "$1"; }
finish() {
  local rc=$?
  # A throwaway goatnet's keys open nothing outside the run, but the record is read by others.
  for f in "$RECORD"/*.log; do [ ! -f "$f" ] || mask "$f"; done
  if [ "$KEEP" = 1 ]; then echo "kept: ssh -i $WORK/laptop-key -p $PORT lab@127.0.0.1 (pids ${PIDS[*]:-none})"
  else
    for p in "${PIDS[@]}"; do kill "$p" 2>/dev/null || true; done
    for p in "${PIDS[@]}"; do wait "$p" 2>/dev/null || true; done
    # The disks hold the goatnet's secrets store, its admin token among them, and the
    # box's one-time password: they go with the VMs.
    rm -f "$R"/*.qcow2
  fi
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
seed box ubuntu 2; seed node ubuntu "$NODE_N"
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

STEPS="$RECORD/steps.jsonl"; : > "$STEPS"; FAILS=0; STOPPED=""; NOT_RUN=(); STAND_INS=(); DECLS=()
# step <name> <command on the laptop>
step() {
  local rc=0 t0; t0=$(date +%s)
  on_laptop "bash -lc 'cd ~/goatcore && timeout ${FOUNDER_STEP_TIMEOUT:-3600} $2'" > "$RECORD/$1.log" 2>&1 </dev/null || rc=$?
  python3 -c 'import json,sys; print(json.dumps({"step": sys.argv[1], "ok": sys.argv[2] == "0", "exit": int(sys.argv[2]), "seconds": int(sys.argv[3])}))' "$1" "$rc" "$(( $(date +%s) - t0 ))" >> "$STEPS"
  if [ "$rc" = 0 ]; then echo "PASS $1 ($(( $(date +%s) - t0 )) s)"; else echo "FAIL $1: exit $rc"; mask "$RECORD/$1.log"; tail -25 "$RECORD/$1.log" | cut -c1-240 | sed 's/^/     | /'; FAILS=$((FAILS + 1)); return 1; fi
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
netbird_version: "$NB_VERSION"
EOF
if [ -n "${FOUNDER_HOOK:-}" ]; then FOUNDER_PORT="$PORT" FOUNDER_KEY="$WORK/laptop-key" $FOUNDER_HOOK || die "FOUNDER_HOOK"; fi
step founder-to-10 "$TO10" || STOPPED="the founder did not reach phase 10"
[ "$ANSWER11" = 1 ] || NOT_RUN+=("standup phases 11 to 15: phase 11 is a person's step, the declared stop")

# The person at standup phase 11, then the rest of the standup, before any line names the
# node: a phase run over the site's group would reach a machine that has not joined.
if [ -z "$STOPPED" ] && [ "$ANSWER11" = 1 ]; then
  # LAB STAND-IN for the person; the record says so. The harness's own copy runs.
  scp "${SSH_O[@]}" -q -i "$WORK/laptop-key" -P "$PORT" "$HERE/../../vm-image/zero-saas/laptop-phase11.sh" "$HERE/../../vm-image/zero-saas/phase11-answer.mjs" lab@127.0.0.1:/tmp/ || die "copying the phase 11 stand-in"
  step founder-11-stand-in "env GB_FLAGS=--offline PHASE10_AVENUE=$AVENUE bash /tmp/laptop-phase11.sh $NET $zone admin@example.org /tmp/phase11-answer.mjs" || STOPPED="the phase 11 stand-in"
  STAND_INS+=("the person at standup phase 11: ops/vm-image/zero-saas/laptop-phase11.sh (lab only), which signs in at the dashboard with the one-time password, re-runs phase 10 once and makes the service user and its token in the dashboard")
  [ -n "$STOPPED" ] || step founder-from-12 "$FROM12" || STOPPED="the founder from phase 12"
  step founder-status "goatbuilder --offline status --env $NET" || true
  on_laptop "cat ~/.goat/state/standup-$NET-timings.tsv 2>/dev/null" > "$RECORD/timings.tsv" || true
  if [ -z "$STOPPED" ] && [ "$ASSAY" = 1 ]; then
    step founder-trusts-root "sudo cp ops/enrollment/ca/$NET/root-cert.pem /usr/local/share/ca-certificates/$NET-root.crt && sudo update-ca-certificates" || STOPPED="the laptop trusting the root"
    [ -n "$STOPPED" ] || step founder-assay-values "python3 ops/validation/generate-values.py $NET && ops/scripts/create-assay-probe-identity.sh $NET" || STOPPED="the assay values and probe identity"
    [ -n "$STOPPED" ] || step founder-assay "goatbuilder --offline assay $NET" || STOPPED="goatbuilder assay"
  fi
fi

if [ -z "$STOPPED" ]; then
  # What a person declares before the node joins: its line, where it enrols, and the
  # configuration join-node reads (a goatlite has none).
  if [ -n "$JSITE" ]; then
    # A joining site, as a person declares one for a goatlite's goatnet, which a goatlite
    # does not make (ops/goatbuilder/fixtures/join-node-third-site has the shape). The
    # record lists each of these as a declaration.
    JUP="$(printf '%s' "$JSITE" | tr '[:lower:]-' '[:upper:]_')"; JG="$(printf '%s' "$JSITE" | tr '-' '_')_hosts"
    on_laptop "cd ~/goatcore && cat > ops/goatbuilder/goatbuilder.yaml" <<EOF
schema: goatbuilder/v1
repo: {source: example-org/example-repo, image_namespace: ghcr.io/example-org}
components:
  netbird-mgmt: {declared_in: ops/pins/fleet-pins.yaml}
environments:
  - name: $NET
    converger: push-ansible
    sites:
      - {name: $NET, role: founder, classes: {core: {}}}
      - {name: $JSITE, role: joiner, joins: $NET, classes: {$CLASS: {}}}
promotion:
  path:
    - {name: $NET, kind: rung, environment: $NET}
deploy: {ssh_user: ubuntu}
EOF
    DECLS+=("goatbuilder.yaml names both sites")
    # Its line, in the management group (Spock finds peers there), with the facts a
    # management host's line carries. No mesh line: phase 12 writes it.
    on_laptop "cd ~/goatcore && sed -i '/^\[mgmt_hosts:children\]/a $JG' ops/ansible/inventory/mgmt-hosts.ini && printf '\n[$JG]\n$NODE ansible_host=$NODE_ADDR ansible_user=ubuntu site_name=$JSITE-a router_role=A signal_uri=netbird.$zone:443\n' >> ops/ansible/inventory/mgmt-hosts.ini"
    DECLS+=("the node's inventory line, its group a child of [mgmt_hosts]")
    # The site's group_vars: the founder's, with what a joining site says differently.
    # join-node reads a site's facts from the top level of its vars.yml.
    on_laptop "cd ~/goatcore/ops/ansible/group_vars && mkdir -p $JG && for f in ${NET}_hosts/*.yml; do [ \"\${f##*/}\" = vars.yml ] || cp \"\$f\" $JG/; done && cat > /tmp/joiner.yml" <<EOF
goatnet_founder: false
join_founder_site: $NET
site_id: $JSITE
site_id_upper: $JUP
site_dns_zone: $zone
mesh_ip_a: TBD_${JUP}_MESH_IP_A
has_breakglass_node: false
breakglass_absent_reason: the lab's operator machine sits on the same segment
has_network_jump_node: false
network_jump_absent_reason: the lab has no switch or router of its own
signal_proto: https
auth_issuer: https://oauth2.$NET.$zone/
auth_audience: netbird-dashboard
auth_oidc_config_endpoint: https://oauth2.$NET.$zone/.well-known/openid-configuration
netbird_mgmt_trust_certs:
  - "{{ playbook_dir }}/../enrollment/ca/$NET/root-cert.pem"
kratos_secrets_cookie_env: ${UPN}_KRATOS_COOKIE
site_enrollment_dns: netbird.$zone
mesh_enrollment_url: https://netbird.$zone:443
signal_federation_absent_reason: one management node at $JSITE holds every peer that signals through it
participates_in_spock: true
inter_site_spock_topology: explicit
inter_site_spock_peers: [$NET]
ory_spock_enabled: true
spock_mesh_publisher_host: $NODE_ADDR
spock_mesh_databases:
  - name: netbird
    replication_sets: [netbird_mgmt]
    repset:
      name: netbird_mgmt
      tables: "{{ spock_mesh_netbird_mgmt_tables }}"
EOF
    on_laptop "cd ~/goatcore/ops/ansible/group_vars && python3 - ${NET}_hosts/vars.yml /tmp/joiner.yml $JG/vars.yml" <<'PY'
import re, sys
src, extra, out = sys.argv[1:4]
add = open(extra).read()
keys = set(re.findall(r"^([a-z_][a-z0-9_]*):", add, re.M))
kept, skip = [], False
for line in open(src).read().split("\n"):
    m = re.match(r"^([a-z_][a-z0-9_]*):", line)
    if m:
        skip = m.group(1) in keys
    elif line and not line.startswith((" ", "-", "#")):
        skip = False
    if not skip:
        kept.append(line)
open(out, "w").write("\n".join(kept).rstrip("\n") + "\n\n# The joining site: what it says differently from the founder this was copied from.\n" + add)
PY
    DECLS+=("the site's group_vars: the founder's, with the joining site's own facts (founder, site, sign-in at the founder's public address, the goatnet's CA for management, signal federation off with its reason)")
    # Replication with the joining site, which a goatlite does not run: what growth
    # changes in the founder's group_vars first (docs/design/goatlite-growth.md §5).
    # Written after the copy above: a copied file sorts after the site's vars.yml and
    # would name the joining site as its own peer.
    on_laptop "cd ~/goatcore && printf 'participates_in_spock: true\ninter_site_spock_topology: explicit\ninter_site_spock_peers: [$JSITE]\nory_spock_enabled: true\n' >> ops/ansible/group_vars/${NET}_hosts/zz-vm-founder.yml"
    on_laptop "cd ~/goatcore && cat >> ops/ansible/group_vars/${NET}_hosts/zz-vm-founder.yml" <<'EOF'
spock_mesh_databases:
  - name: netbird
    replication_sets: [netbird_mgmt]
    repset:
      name: netbird_mgmt
      tables: "{{ spock_mesh_netbird_mgmt_tables }}"
EOF
    DECLS+=("the founder's replication with the joining site (goatlite-growth.md section 5): participates_in_spock, an explicit topology naming $JSITE, ory_spock_enabled, and the NetBird tables in the netbird_mgmt replication set (spock_mesh_databases, ADR 1084 D1), on both sites")
    DECLS+=("the joining site's Spock publisher host, its own address $NODE_ADDR (spock_mesh_publisher_host): its signal_uri names the founder")
    # The joining site's group is a member of the goatnet in the goatlite's pin
    # sidecar: the end-state model then reads its services as the goatnet's (a
    # service the goatlite declares absent is absent here too), and goatbuilder
    # reads its deploy ledgers under the goatnet's environment.
    on_laptop "cd ~/goatcore && sed -i '/^  $NET:\$/,/^# END goatlite $NET\$/{/^    members:\$/{n;s/^      - ${NET}_hosts\$/      - ${NET}_hosts\n      - $JG/}}' ops/pins/lite-pins.yaml && grep -qx '      - $JG' ops/pins/lite-pins.yaml"
    DECLS+=("the joining site's group, $JG, a member of the goatnet $NET in ops/pins/lite-pins.yaml")
    # The founder's management host and the joining site's reach each other (ADR 1131:
    # a management host is INFRA, and INFRA reaches INFRA; ADR 1114): the rule growth
    # declares in the founder's rule intents, applied by standup phase 12 again,
    # before the node enrols.
    on_laptop "cd ~/goatcore && python3 - ops/warrant/rule-intents/netbird-acl.yaml $NET && python3 ops/warrant/generate.py" <<'PY'
import sys
path, net = sys.argv[1:3]
lines = open(path).read().split("\n")
# The block that takes effect: the last one for this goatnet in the matrix section
# (the render appends its own block after any the checkout already has).
end = lines.index("netbird_acl_tests:") if "netbird_acl_tests:" in lines else len(lines)
start = max(i for i in range(end) if lines[i] == f"  {net}:")
at = next(i for i in range(start + 1, end) if lines[i] == "    rules:")
rule = [
    "      - name: \"infra ↔ infra (management hosts, ADRs 1131 and 1114)\"",
    "        description: \"A goatnet's management hosts reach each other on any port: Spock, the routers, mesh DNS.\"",
    "        sources: [INFRA]",
    "        destinations: [INFRA]",
    "        protocol: all",
    "        action: accept",
    "        bidirectional: true",
    "",
]
if not any("infra ↔ infra" in l for l in lines[start:]):
    lines[at + 1:at + 1] = rule
open(path, "w").write("\n".join(lines))
PY
    step founder-acl-growth "goatbuilder --offline standup $NET --from 12 --to 12 --ack vm-founder --yes" || STOPPED="the founder's ACL with INFRA reaching INFRA"
    DECLS+=("the founder's rule intents: INFRA reaches INFRA on any port (ADRs 1131, 1114), regenerated (ops/warrant/generate.py) and applied by standup phase 12 again")
    # Written after the joining site's copy, as the replication keys are: a copied
    # isitup_services would make the joining site a second writer of the goatnet's zone.
    # The goatnet's mesh DNS zone, which the joining site serves a replica of: a goatlite
    # has a LAN zone only. The founder declares its services, and the generator makes it.
    on_laptop "cd ~/goatcore && printf 'isitup_services:\n  - {name: dashboard, fqdn: dashboard.$NET.$zone}\n  - {name: portal, fqdn: portal.$NET.$zone}\n' >> ops/ansible/group_vars/${NET}_hosts/zz-vm-founder.yml && python3 ops/dns/generate-mesh-hosts.py --init $NET"
    DECLS+=("the goatnet's mesh DNS zone: two services in the founder's isitup_services, then ops/dns/generate-mesh-hosts.py --init $NET")
    # The goatnet's CA signs the joining site's leaves: the founder's manifest, for this site.
    on_laptop "cd ~/goatcore && sed -e 's/^site_id: $NET\([[:space:]]\|\$\)/site_id: $JSITE\1/' -e 's|^out_dir: \(.*\)/$NET-leaves|out_dir: \1/$JSITE-leaves|' -e 's/urn:goat:$NET:/urn:goat:$JSITE:/g' -e 's/\\.$NET\\.$zone/.$JSITE.$zone/g' ops/enrollment/manifests/$NET-certs.yaml > ops/enrollment/manifests/$JSITE-certs.yaml"
    DECLS+=("the site's certificate manifest: the founder's, under the goatnet's CA")
  else
  on_laptop "cd ~/goatcore && cat > ops/goatbuilder/goatbuilder.yaml && sed -i '/^mgmt-$NET-01 /a $NODE ansible_host=$NODE_ADDR ansible_user=ubuntu site_name=$NET-b router_role=B signal_uri=netbird.$zone:443' ops/ansible/inventory/mgmt-hosts.ini" <<EOF
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
  fi
  # The site's names, which a machine of the site has from its LAN: the zone resolves
  # on the box. The goatnet's root is not put in any system store: the NetBird client
  # gets it from mesh-enroll-host.yml, for that client alone.
  for h in 2 3; do
    on_laptop "ssh ubuntu@10.88.0.$h 'sudo mkdir -p /etc/systemd/network/10-netplan-lan.network.d && printf \"[Network]\nDNS=10.88.0.2\nDomains=~$zone\n\" | sudo tee /etc/systemd/network/10-netplan-lan.network.d/dns.conf >/dev/null && sudo networkctl reload'"
  done
  # The operator machine joins the goatnet's mesh as the founder's admin peer, which is
  # what join-node's default transport (--via mesh) assumes it is: from phase 12 on,
  # join-node reaches the node at its mesh address.
  # Its group exists once standup phase 12 has run, so only with the phase 11 stand-in.
  if [ -z "$STOPPED" ] && [ "$ANSWER11" = 1 ]; then
    scp "${SSH_O[@]}" -q -i "$WORK/laptop-key" -P "$PORT" "$HERE/laptop-mesh-enrol.sh" lab@127.0.0.1:/tmp/ || die "copying the laptop's mesh enrolment"
    step laptop-mesh "bash /tmp/laptop-mesh-enrol.sh $NET $zone $NB_VERSION" || STOPPED="the laptop joining the mesh"
    DECLS+=("the operator machine (the laptop) on the goatnet's mesh as the founder's admin peer: NetBird $NB_VERSION from its verified release, the goatnet's root for that client alone, a single-use key with $UPN-PLATFORM-ADMIN (ops/e2e/vm/laptop-mesh-enrol.sh)")
  fi
  # With the phase 11 stand-in the founder stands up whole, and standup phase 12 enrols
  # the site's own hosts and puts the founder's peer in the site's INFRA group
  # (netbird-infra-peers.yml), whose groups join-node phase 10 mints the node's key with.
  # Without it the founder stops at phase 10, so the run does both: the founder's
  # enrolment play, and a group with no policy written to the database.
  if [ -z "$STOPPED" ] && [ "$ANSWER11" = 0 ]; then
    step founder-enrol "env -C ops/ansible GOAT_AWS_MODE=none ../scripts/with-aws default -- ansible-playbook -i inventory/mgmt-hosts.ini -i inventory/mgmt-hosts-mesh.ini mesh-enroll-host.yml -e target=${NET}_hosts -l mgmt-$NET-01 -e mesh_ips_out=/tmp/founder-mesh-ips.json" || STOPPED="the founder's own enrolment"
    [ -n "$STOPPED" ] || on_laptop "ssh ubuntu@10.88.0.2 'sudo docker exec -i postgres-a psql -v ON_ERROR_STOP=1 -q -U postgres -d netbird'" <<'SQL' || STOPPED="the stand-in group"
INSERT INTO groups (id, account_id, name, issued) SELECT 'vm-founder-infra', id, 'VM-FOUNDER-INFRA', 'api' FROM accounts;
INSERT INTO group_peers (account_id, group_id, peer_id) SELECT account_id, 'vm-founder-infra', id FROM peers;
SQL
    STAND_INS+=("the group of the founder's peer, which standup phase 12 makes: one group with no policy, written to the database")
  fi
  # join-node reads the founder's owner from the environment, where standup reads
  # goatlite.yaml's admin: a person declares it.
  LAST=12; [ "$ANSWER11" = 0 ] || LAST=15
  [ -n "$STOPPED" ] || for n in $(seq 1 "$LAST"); do
    step "join-$n" "env OWNER_EMAIL=admin@example.org goatbuilder --offline --yes join-node $NET --site $NODE_SITE --node $NODE --class $CLASS --address $NODE_ADDR --from $n --to $n" || { STOPPED="join-node phase $n"; break; }
  done
  [ -n "$STOPPED" ] || [ "$ANSWER11" = 0 ] || step doctor-target "goatbuilder --offline doctor --target $CLASS --site $NODE_SITE --node $NODE --all --live" || STOPPED="doctor --target $CLASS"
fi
if [ -z "$STOPPED" ] && [ "$ANSWER11" = 0 ]; then
  NOT_RUN+=("join-node phase 13: a core machine at the founder's site runs the site's bootstrap again, which needs the administrator of standup phase 11")
  NOT_RUN+=("join-node phase 14: it reaches the node at its mesh address, and the mesh carries nothing before standup phase 12 applies the declared ACL")
  NOT_RUN+=("join-node phase 15: its rows read a whole site, and this one stops before standup phase 11")
fi
HARNESS="$(git -C "$HERE" rev-parse HEAD 2>/dev/null || echo unknown)"; git -C "$HERE" diff --quiet HEAD -- . 2>/dev/null || HARNESS="$HARNESS+changes"
AVENUE="$AVENUE" CLASS="$CLASS" NODE_SITE="$NODE_SITE" ANSWER11="$ANSWER11" HARNESS="$HARNESS" python3 - "$RECORD" "$STOPPED" "$VM_IMAGE_SHA256" "$(sha256sum "$IMAGE" | cut -d' ' -f1)" "$(cat "$WORK/laptop-commit")" "$(printf '%s\n' ${STAND_INS[@]+"${STAND_INS[@]}"})" "$(printf '%s\n' ${DECLS[@]+"${DECLS[@]}"})" ${NOT_RUN[@]+"${NOT_RUN[@]}"} <<'PY'
import json, os, sys
rec, stopped, node, box, commit, stand, decls = sys.argv[1:8]
steps = [json.loads(l) for l in open(os.path.join(rec, "steps.jsonl"))]
json.dump({"schema": "goat-vm-founder-run/v1", "avenue": os.environ.get("AVENUE", "cli"), "class": os.environ.get("CLASS", "core"), "node_site": os.environ.get("NODE_SITE", ""), "hook": os.environ.get("FOUNDER_HOOK", ""),
           "declared_stop": "none: a lab stand-in answered standup phase 11" if os.environ.get("ANSWER11") == "1" else "standup phase 11, a person's step",
           "goatcore": commit, "harness": os.environ.get("HARNESS", ""), "box_image_sha256": box, "node_image_sha256": node, "steps": steps,
           "stopped_at": stopped, "stand_ins": stand.split("\n") if stand else [], "declarations": decls.split("\n") if decls else [], "not_run": sys.argv[8:], "passed": not stopped and all(s["ok"] for s in steps)},
          open(os.path.join(rec, "result.json"), "w"), indent=1)
PY
for n in ${NOT_RUN[@]+"${NOT_RUN[@]}"}; do echo "NOT RUN $n"; done
if [ "$ANSWER11" = 1 ]; then stop="phase 11 answered by the lab stand-in"; else stop="DECLARED STOP before standup phase 11"; fi
echo "founder: $(grep -c '"ok": true' "$STEPS") of $(wc -l < "$STEPS" | tr -d ' ') steps passed${STOPPED:+; stopped at $STOPPED}; $stop; record $RECORD/result.json"
[ "$FAILS" = 0 ] && [ -z "$STOPPED" ]
