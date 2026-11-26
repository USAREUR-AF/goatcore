#!/usr/bin/env bash
# fed-peering.sh: a peering's acceptance over two federation-boundary machines
# (gate J8; docs/design/goatbuilder-build-targets.md section 3.6). Two VMs from a
# published Ubuntu cloud image, one per synthetic sovereign (pasture and meadow),
# each built by `join-node --class federation-boundary` as run.sh builds one. A
# wire between them is the only path from one to the other. Then
# ops/federation-e2e/live.sh runs its scenario against the two machines
# (--images-present): the real driver federates them, and phase 14's acceptance runs from
# both sides, strict.
#
#   ops/e2e/vm/fed-peering.sh --goat BIN --goatbuilder BIN [--work DIR] [--record DIR]
#                             [--scenario 1|2|both] [--keep] [--machines-only]
#
# The binaries are shipped to the machines, so they must run on Ubuntu 24.04 (the
# federation-live-binaries artifact of a federation-e2e run is such a pair).
# live.sh's router image is private. This run never holds a token: it saves the
# image from this machine's docker, which must already hold it at live.sh's pinned
# digest (pull it by that digest first), loads it into each machine's docker,
# checks it there by that digest, and runs live.sh --images-present.
#
# Not like a real goatnet, and the record says so: live.sh puts each sovereign's
# core router and SPIRE server on its edge machine (its README, "One host per
# sovereign departs from the runbook"), and neither machine is enrolled in a mesh
# (run.sh does not run phases 3, 7 to 12, 14 or 15).
#
# Writes <record>/result.json, run.sh's record for each machine and live.sh's
# record. Exit 0 when both machines and the scenario passed, 1 otherwise, 2 for a
# usage error or a missing tool. --machines-only stops before live.sh: the two
# machines and the wire, which need no image.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../../.." && pwd)"
GOAT=""; GB=""; WORK=""; RECORD=""; KEEP=0; SCEN=1; ONLY=0
while [ $# -gt 0 ]; do
  case "$1" in
    --goat) GOAT="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"; shift 2 ;;
    --goatbuilder) GB="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"; shift 2 ;;
    --work) WORK="$2"; shift 2 ;;
    --record) RECORD="$2"; shift 2 ;;
    --scenario) SCEN="$2"; shift 2 ;;
    --keep) KEEP=1; shift ;;
    --machines-only) ONLY=1; shift ;;
    -h|--help) sed -n '2,29p' "$0"; exit 0 ;;
    *) echo "fed-peering: unknown argument $1" >&2; exit 2 ;;
  esac
done
[ -n "$GB" ] && [ -x "$GB" ] || { echo "fed-peering: --goatbuilder names a goatbuilder binary" >&2; exit 2; }
[ "$ONLY" = 1 ] || { [ -n "$GOAT" ] && [ -f "$GOAT" ]; } || { echo "fed-peering: --goat names a goat binary" >&2; exit 2; }
ZENOH_IMAGE="$(sed -n 's/^ZENOH_IMAGE="\(.*\)"$/\1/p' "$REPO/ops/federation-e2e/live.sh")"
case "$ZENOH_IMAGE" in *@sha256:*) ;; *) echo "fed-peering: live.sh pins no zenoh image by digest" >&2; exit 2 ;; esac
[ "$ONLY" = 1 ] || docker image inspect "$ZENOH_IMAGE" >/dev/null 2>&1 || { echo "fed-peering: this machine's docker does not hold $ZENOH_IMAGE; pull it by that digest first" >&2; exit 2; }
[ -n "$WORK" ] || WORK="$(mktemp -d "${TMPDIR:-/tmp}/goat-fed-vm.XXXXXX")"
mkdir -p "$WORK"; WORK="$(cd "$WORK" && pwd)"
[ -n "$RECORD" ] || RECORD="$WORK/record"
mkdir -p "$RECORD"; RECORD="$(cd "$RECORD" && pwd)"
free_port() { python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])'; }
free_udp() { python3 -c 'import socket; s=socket.socket(socket.AF_INET, socket.SOCK_DGRAM); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])'; }

# Each machine's public address is a documentation address on the wire between them.
ADDR_pasture=192.0.2.11; ADDR_meadow=192.0.2.12
U1="$(free_udp)"; U2="$(free_udp)"
SSH_pasture="$(free_port)"; SSH_meadow="$(free_port)"
addr() { eval "echo \$ADDR_$1"; }
sshport() { eval "echo \$SSH_$1"; }
# The two ends of the wire, each the other way round, and a MAC address of its own.
link() { if [ "$1" = pasture ]; then echo "$U1:$U2"; else echo "$U2:$U1"; fi; }
linkmac() { if [ "$1" = pasture ]; then echo 16; else echo 17; fi; }

down() {
  local s pid
  for s in pasture meadow; do
    for pid in $(pgrep -f "file=$WORK/$s/vm/disk.qcow2" || true); do kill "$pid" 2>/dev/null || true; done
  done
}
trap 'rc=$?; if [ "$KEEP" = 0 ]; then down; else echo "kept: ssh -i $WORK/<side>/vm/key -p <port> goatvm@127.0.0.1 (pasture $SSH_pasture, meadow $SSH_meadow)"; fi; exit $rc' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

STEPS="$RECORD/steps.jsonl"; : > "$STEPS"; FAILS=0
note() {
  python3 -c 'import json,sys; print(json.dumps({"step": sys.argv[1], "ok": sys.argv[2] == "0", "exit": int(sys.argv[2])}))' "$1" "$2" >> "$STEPS"
  if [ "$2" = 0 ]; then echo "PASS $1"; else echo "FAIL $1 (exit $2)"; FAILS=$((FAILS + 1)); fi
}

# 1. Each machine, from the cloud image through join-node to doctor --target,
#    kept up for the peering. Both are built at once.
PIDS=()
for s in pasture meadow; do
  (
    export VM_PORT; VM_PORT="$(sshport "$s")"
    export VM_LINK VM_LINK_ADDR VM_LINK_MAC
    VM_LINK="$(link "$s")"; VM_LINK_ADDR="$(addr "$s")/24"; VM_LINK_MAC="$(linkmac "$s")"
    exec "$HERE/run.sh" federation-boundary --goatbuilder "$GB" --work "$WORK/$s" --record "$RECORD/$s-machine" --keep
  ) > "$RECORD/$s-machine.log" 2>&1 &
  PIDS+=($!)
done
i=0
for s in pasture meadow; do
  rc=0; wait "${PIDS[$i]}" || rc=$?; i=$((i + 1))
  note "$s-machine" "$rc"
done
[ "$FAILS" = 0 ] || { tail -n 20 "$RECORD"/*-machine.log; exit 1; }

# Every ssh, live.sh's included, reaches a machine by its public address, through this run's configuration.
mkdir -p "$WORK/bin"
for s in pasture meadow; do
  printf 'Host %s\n  HostName 127.0.0.1\n  Port %s\n  User goatvm\n  IdentityFile %s\n' "$(addr "$s")" "$(sshport "$s")" "$WORK/$s/vm/key"
done > "$WORK/ssh_config"
printf 'Host *\n  IdentitiesOnly yes\n  LogLevel ERROR\n' >> "$WORK/ssh_config"
REAL_SSH="$(command -v ssh)"
printf '#!/bin/sh\nexec "%s" -F "%s" "$@"\n' "$REAL_SSH" "$WORK/ssh_config" > "$WORK/bin/ssh"; chmod +x "$WORK/bin/ssh"
export PATH="$WORK/bin:$PATH"
on() { ssh -o BatchMode=yes -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null "$(addr "$1")" "${@:2}"; }

# 2. The wire is the only path, and it carries the tunnel's port.
rc=0; on pasture "ping -c 3 -W 2 $ADDR_meadow" > "$RECORD/wire.log" 2>&1 || rc=$?
on meadow "ping -c 3 -W 2 $ADDR_pasture" >> "$RECORD/wire.log" 2>&1 || rc=$?
note wire "$rc"
rc=0; { on meadow 'sudo timeout 10 python3 -c "import socket; s=socket.socket(socket.AF_INET, socket.SOCK_DGRAM); s.bind((\"0.0.0.0\", 51820)); print(s.recvfrom(64)[0].decode())"' & sleep 3; on pasture "python3 -c 'import socket; socket.socket(socket.AF_INET, socket.SOCK_DGRAM).sendto(b\"udp 51820 crosses\", (\"$ADDR_meadow\", 51820))'"; wait; } > "$RECORD/wire-udp.log" 2>&1 || rc=$?
grep -q 'udp 51820 crosses' "$RECORD/wire-udp.log" || rc=1
note wire-udp-51820 "$rc"
[ "$FAILS" = 0 ] || exit 1

if [ "$ONLY" = 0 ]; then
# 3. The router image, from this machine's docker to each machine's. The docker the
#    host baseline installed keeps images by content (the containerd image store),
#    so the image loaded there is held to live.sh's digest: its id must be that
#    digest before it is given the image's name, and the name at that digest must
#    then resolve.
DIGEST="${ZENOH_IMAGE##*@}"; REPO_NAME="${ZENOH_IMAGE%@*}"
for s in pasture meadow; do
  rc=0
  (
    on "$s" 'docker --version; sudo docker info -f "{{.DriverStatus}}"'
    id="$(docker save "$ZENOH_IMAGE" | on "$s" 'sudo docker load -q' | sed -n 's/^Loaded image ID: //p')"
    echo "loaded: $id"
    [ "$id" = "$DIGEST" ] || { echo "the image loaded on $s is $id, not $DIGEST"; exit 1; }
    on "$s" "sudo docker tag $id $REPO_NAME:fed-live && sudo docker image inspect -f 'held at its digest: {{.Id}}' $ZENOH_IMAGE"
  ) > "$RECORD/$s-image.log" 2>&1 || rc=$?
  grep -q "held at its digest: $DIGEST" "$RECORD/$s-image.log" || rc=1
  note "$s-image" "$rc"
done
[ "$FAILS" = 0 ] || exit 1

# 4. The peering, by live.sh, against the two machines. Its forwards on this
#    machine go on the first two port bases with every port free.
bases="$(python3 - <<'PY'
import socket
def free(b):
    for p in [b * 1000] + [b * 1000 + 101 + i for i in range(3)]:
        s = socket.socket()
        try:
            s.bind(("127.0.0.1", p))
        except OSError:
            return False
        finally:
            s.close()
    return True
print(" ".join(str(b) for b in [b for b in range(18, 65) if free(b)][:2]))
PY
)"
export FED_LIVE_PORT_BASE_PASTURE="${bases%% *}" FED_LIVE_PORT_BASE_MEADOW="${bases##* }"
rc=0
"$REPO/ops/federation-e2e/live.sh" --goat "$GOAT" --goatbuilder "$GB" --scenario "$SCEN" --images-present \
  --pasture-host "$ADDR_pasture" --meadow-host "$ADDR_meadow" \
  --local-goatbuilder "$GB" --pasture-user goatvm --meadow-user goatvm --pasture-key "$WORK/pasture/vm/key" --meadow-key "$WORK/meadow/vm/key" \
  --record "$RECORD/peering" > "$RECORD/peering.log" 2>&1 || rc=$?
note peering "$rc"
fi

python3 - "$RECORD" "$(git -C "$REPO" rev-parse HEAD)" "$("$GB" --version 2>/dev/null | head -1)" "$SCEN" "$ONLY" <<'PY'
import json, os, sys
rec, commit, gb, scen = sys.argv[1:5]
steps = [json.loads(l) for l in open(os.path.join(rec, "steps.jsonl"))]
json.dump({"schema": "goat-vm-fed-peering/v1", "type": "federation-boundary", "goatcore": commit, "goatbuilder": gb,
           "scenario": scen if sys.argv[5] == "0" else None, "machines": {"pasture": "192.0.2.11", "meadow": "192.0.2.12"}, "steps": steps,
           "departures": ["each sovereign's core router and SPIRE server run on its edge machine (live.sh)",
                          "neither machine is enrolled in a mesh: join-node phases 3, 7 to 12, 14 and 15 are not run"],
           "passed": all(s["ok"] for s in steps)}, open(os.path.join(rec, "result.json"), "w"), indent=1)
PY
echo "fed-peering: $(grep -c '"ok": true' "$STEPS") of $(wc -l < "$STEPS" | tr -d ' ') steps passed; record $RECORD/result.json"
[ "$FAILS" = 0 ]
