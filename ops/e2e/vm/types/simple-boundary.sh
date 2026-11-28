# shellcheck shell=bash disable=SC2034 # read by ops/e2e/vm/run.sh
# simple-boundary (gate J10, ADR 1253), on the host-class family: phase 13 runs
# simple-bn.yml from the group's simple_bn declaration, at encryption stage 0 with
# notional input, the only data stage 0 may carry. The receiver is a UDP listener
# on the machine itself. Nothing enrols the machine here, so its mesh interface is
# a stand-in: a dummy wt0 with a mesh address, which the monitor binds to.
# Its small-computer variant waits for the small-computer builder.
HOST_VARS="platform_family: host-class"
GROUP_VARS="simple_bn:
  node: $NODE
  crossing: {kind: simple-bn, mode: plain}
  receiver: {label: vm-receiver, dest_env: ZAGROS_SBN_RECEIVER, transport: udp}
  encryption: {stage: 0}
  feeds: [radar-a, radar-b]
  profiles: ops/goatbuilder/overlay/simple-bn/profiles.json
  sac: 7
  input: notional"
# The receiver's HOST:PORT, as with-secrets would export it from the store.
export ZAGROS_SBN_RECEIVER=127.0.0.1:47001

type_tree() {
  mkdir -p "$1/ops/goatbuilder/overlay/simple-bn"
  printf '%s\n' '{"radar-a": {"base": "hemispheric-cuas", "title": "Radar A (notional)", "sic": 11, "src_port": 40011},' \
    ' "radar-b": {"base": "surveillance-3d", "title": "Radar B (notional)", "sic": 12, "src_port": 40012}}' > "$1/ops/goatbuilder/overlay/simple-bn/profiles.json"
}

# The mesh stand-in, and a receiver that counts ASTERIX blocks by category and source port.
sbn_prepare() {
  vm_ssh 'sudo sh -eux' <<'VM'
ip link add wt0 type dummy
ip addr add 100.127.0.9/32 dev wt0
ip link set wt0 up
cat > /usr/local/bin/vm-receiver <<'PY'
import collections, json, socket, sys, time
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); s.bind(("127.0.0.1", 47001)); s.settimeout(1)
seen = collections.Counter(); end = time.time() + 3600
while time.time() < end:
    try:
        b, a = s.recvfrom(65535)
    except socket.timeout:
        continue
    seen["cat%03d from %d" % (b[0], a[1])] += 1
    json.dump(seen, open("/run/vm-receiver.json.tmp", "w")); __import__("os").replace("/run/vm-receiver.json.tmp", "/run/vm-receiver.json")
PY
systemd-run --unit vm-receiver /usr/bin/python3 /usr/local/bin/vm-receiver
VM
}

type_cases() {
  step prepare 0 -- sbn_prepare
  step phase-13 0 "Phase 13 " "simple-bn | summary" "monitor ok on port 8740" "!fatal:" -- "${JOIN[@]}" --from 13 --to 13
  # The design's checks on the host (simple-boundary-node.md section 8).
  step services 0 "active" "!inactive" "!failed" -- vm_ssh 'systemctl is-active simple-bn-sender bn-monitor'
  step monitor 0 "ok" -- vm_ssh 'curl -fsS http://100.127.0.9:8740/healthz'
  step monitor-mesh-only 7 -- vm_ssh 'curl -fsS -m 5 http://127.0.0.1:8740/healthz'
  step receiver 0 "cat034 from 40011" "cat048 from 40011" "cat034 from 40012" "cat048 from 40012" -- vm_ssh 'sleep 5; cat /run/vm-receiver.json'
  step receiver-file-mode 0 "600 root" -- vm_ssh 'sudo stat -c "%a %U" /etc/simple-bn/receiver.env'
  # Run it a second time: that run must report no changes.
  step phase-13-again 0 "Phase 13 " "changed=0" "!fatal:" -- "${JOIN[@]}" --from 13 --to 13
}
