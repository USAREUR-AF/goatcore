#!/usr/bin/env bash
# units/twodrivers.sh — gate row B10 (docs/design/goatbuilder-v1-gate.md), against the contract
# in docs/design/federation-two-drivers.md: two drivers per sovereign.
#
#   the controller   ($HP-<s>-ops)   holds the CA key, the plan, every approval and the journal;
#                                    reaches the core host, and has no route to the edge host
#   the edge driver  ($HP-<s>-edrv)  holds no CA key and no lab key but its ssh key; reaches the
#                                    edge host only, on a network it shares with nothing else
#
# Both are enforced by the network: the edge driver sits on its own network with the edge host,
# and the controller's route to the edge host is a blackhole. Each side then follows the
# hand-offs the command gives, from phase 1 to phase 14.
#
# Steps, today (nothing of the two-driver run is built): the topology checks, then the
# controller's phase 1 must stop with a hand-off naming the edge driver's `--side edge` run, and
# the edge driver's goatbuilder must offer `--side`. While either fails the unit stops there,
# and the contract's end-state checks (catalog: two-drivers-per-sovereign) read BLOCKED.
# shellcheck source=/dev/null
. "$HERE/units/workspace.sh"

for _p in $PARTIES; do HOSTS+=("$HP-$_p-edrv"); NETS+=("$NP-$_p-edrv"); done
edrv_ip() { echo "$NB.$(($(octet "$1") + 60)).30"; }
edrv_edge_ip() { echo "$NB.$(($(octet "$1") + 60)).20"; }
export -f edrv_ip edrv_edge_ip

# The edge driver: the host image, its own network with the edge host, and only an ssh key.
td_edge_driver() { # $1 sovereign
  local s="$1" o k="$WORK/$1/edrv-keys"; o="$(octet "$s")"
  mkdir -p "$k"; cp "$WORK/$s/keys/id_ed25519" "$WORK/$s/keys/id_ed25519.pub" "$k/"; chmod 0600 "$k/id_ed25519"
  docker network create "${LABEL[@]}" --internal --subnet "$NB.$((o + 60)).0/24" "$NP-$s-edrv" >/dev/null
  docker network connect --ip "$(edrv_edge_ip "$s")" "$NP-$s-edrv" "$HP-$s-edge"
  docker run -d --name "$HP-$s-edrv" --hostname "$HP-$s-edrv" "${LABEL[@]}" "${NOPROXY[@]}" --privileged --cgroupns=private \
    --network "$NP-$s-edrv" --ip "$(edrv_ip "$s")" \
    --tmpfs /run --tmpfs /run/lock -v /var/lib/docker \
    -v "$k:/etc/fed-e2e:ro" -v "$WORK/$s/origin.git:/srv/goat.git:ro" \
    -v "$(goat_of "$s"):/usr/local/bin/goat:ro" -v "$GB:/usr/local/bin/goatbuilder:ro" \
    -e container=docker "$HOST_IMAGE" >/dev/null
  wait_ready "$HP-$s-edrv"
  # The controller has no route to the edge host.
  docker exec "$HP-$s-ops" ip route add blackhole "$(edge_ip "$s")/32"
}

td_topology_checks() { # $1 sovereign
  local s="$1" ssh="ssh -o BatchMode=yes -o ConnectTimeout=5 -i /etc/fed-e2e/id_ed25519"
  rec "td-$s-edge-driver-reaches-the-edge" --expect-in "$HP-$s-edge" -- docker exec "$HP-$s-edrv" $ssh "goat@$(edrv_edge_ip "$s")" hostname
  rec "td-$s-edge-driver-cannot-reach-the-core" --expect-exit 1 -- docker exec "$HP-$s-edrv" bash -c "exec 3<>/dev/tcp/$(core_ip "$s")/22"
  rec "td-$s-controller-cannot-reach-the-edge" --expect-exit 1 -- docker exec "$HP-$s-ops" bash -c "timeout 5 bash -c 'exec 3<>/dev/tcp/$(edge_ip "$s")/22'"
  rec "td-$s-controller-reaches-the-core" --expect-in "$HP-$s-core" -- docker exec "$HP-$s-ops" ssh -o BatchMode=yes -o ConnectTimeout=5 "goat@$(core_ip "$s")" hostname
  rec "td-$s-edge-driver-holds-no-ca-key" -- docker exec "$HP-$s-edrv" sh -c \
    "found=\$(grep -rlE 'BEGIN (EC |RSA )?PRIVATE KEY' /etc /root /opt 2>/dev/null | grep -v '/etc/fed-e2e/id_ed25519\$' | grep -v '^/etc/ssh/' || true); [ -z \"\$found\" ] && echo 'no private key but its ssh key' || { echo \"\$found\"; exit 1; }"
}

unit_main() {
  local n=td s them
  [ "$AVENUE" = goatbuilder ] || die "the two-driver unit drives goatbuilder only, not $AVENUE"
  set_partners "$A" "$B"; set_partners "$B" "$A"; set_partners "$C" ""
  # shellcheck disable=SC2034 # NODIR is read by lib/engine.sh (nodir)
  NODIR="$A:$B $B:$A"
  export_helpers
  setup_hosts "$n"
  for s in "$A" "$B"; do td_edge_driver "$s"; done
  # The edge driver's key reaches the edge host (the goat account the inventory uses).
  for s in "$A" "$B"; do
    docker exec "$HP-$s-edge" sh -c "cat >> /home/goat/.ssh/authorized_keys" < "$WORK/$s/keys/id_ed25519.pub"
  done
  say "two drivers per sovereign: the topology the contract asks for, enforced by the network"
  for s in "$A" "$B"; do td_topology_checks "$s"; done

  # The operator's declaration that this peering is run from two drivers: `drivers: two` on
  # its peerings: entry, in the controller's checkout, committed (a hand-off needs a clean tree).
  for s in "$A" "$B"; do td_declare_two "$s" "$(other "$s")"; done
  say "two drivers per sovereign: the controller hands off what only the edge reaches"
  local f0="$FAILS"
  for s in "$A" "$B"; do
    them="$(other "$s")"
    rec "td-$s-controller-phase-1-hands-off-to-the-edge-driver" --expect-in="--side edge" -- bash -c "drive '$s' '$them' goatbuilder 1 1"
    rec "td-$s-edge-driver-offers-side-edge" --expect-in="--side" -- docker exec "$HP-$s-edrv" goatbuilder federate --help
  done
  [ "$FAILS" = "$f0" ] || die "the two-driver run is not built: the controller's phase 1 gives no hand-off to an edge driver (docs/design/federation-two-drivers.md, What has to be built)"
  for s in "$A" "$B"; do td_follow "$s" "$(other "$s")"; done
  td_end_state
}

TD_HANDOFF="$HERE/lib/handoff.py"

td_declare_two() { # $1 us, $2 them
  local s="$1" them="$2"
  docker exec -i "$HP-$s-ops" python3 - "$them" <<'PY'
import sys, yaml
p = "/opt/goat/ops/goatbuilder/goatbuilder.yaml"
cfg = yaml.safe_load(open(p)); them = sys.argv[1]
peers = [e for e in (cfg.get("peerings") or []) if e.get("with") == them]
if len(peers) != 1:
    sys.exit(f"no single peerings: entry with {them} in {p}")
peers[0]["drivers"] = "two"
open(p, "w").write(yaml.safe_dump(cfg, sort_keys=False))
print(f"declared drivers: two for the peering with {them}")
PY
  docker exec "$HP-$s-ops" sh -c "cd /opt/goat && git config user.name '$s operator' && git config user.email 'ops@$s.test' && git add ops/goatbuilder/goatbuilder.yaml && git commit -q -m '$s: the peering with $them is run from two drivers' && git log --oneline -1"
}

# The controller's goatbuilder, as an operator runs it there; $1 us, $2 them, then arguments.
td_ctl() {
  local s="$1" them="$2"; shift 2
  docker exec -e "FED_WORK=/var/lib/goat-federation/$them" -e HOME=/root -e GOAT_AWS_MODE=none -w /opt/goat \
    "$HP-$s-ops" goatbuilder -C /opt/goat --yes "$@"
}
# The edge driver's, the same way.
td_edge() {
  local s="$1" them="$2"; shift 2
  docker exec -e "FED_WORK=/var/lib/goat-federation/$them" -e HOME=/root -w /opt/goat "$HP-$s-edrv" goatbuilder -C /opt/goat --yes "$@"
}

# The edge driver's checkout at the controller's commit: a bundle of the controller's checkout,
# fetched and checked out on the edge driver (the lab's origin is read-only to both).
td_sync_checkout() { # $1 us, $2 commit
  local s="$1" c="$2" b="$WORK/$1/td-checkout.bundle"
  docker exec "$HP-$s-ops" git -C /opt/goat bundle create /tmp/td.bundle --all >/dev/null 2>&1
  docker cp -q "$HP-$s-ops:/tmp/td.bundle" "$b"
  docker exec "$HP-$s-edrv" sh -c "[ -d /opt/goat/.git ] || git clone -q /srv/goat.git /opt/goat"
  docker cp -q "$b" "$HP-$s-edrv:/tmp/td.bundle"
  docker exec "$HP-$s-edrv" sh -c "cd /opt/goat && git fetch -q /tmp/td.bundle '+refs/*:refs/remotes/td/*' && git checkout -q --detach '$c' && git status --porcelain | grep -q . && exit 1 || exit 0"
}

# One side, from phase 1 to the end, following every hand-off the command gives.
td_follow() { # $1 us, $2 them
  local s="$1" them="$2" i=0 out nx way run_on cmd commit res code imp thn rp f0
  out="$RECORD/td-$s-ctl-0.json"
  rec "td-$s-controller-run" -- bash -c "td_ctl '$s' '$them' -o json federate '$them' --as '$s' > '$out'; rc=\$?; tail -c 2000 '$out'; exit \$rc"
  while [ "$i" -lt 20 ]; do
    i=$((i + 1))
    nx="$(python3 "$TD_HANDOFF" next "$out")" || { rec "td-$s-handoff-$i-reads" -- false; return 0; }
    run_on="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["run_on"] or "")' "$nx")"
    [ -n "$run_on" ] || { echo "$s: no hand-off after $((i - 1)) hand-off(s)" >> "$RECORD/td-$s-follow.txt"; return 0; }
    printf '%s\n' "$nx" >> "$RECORD/td-$s-handoffs.jsonl"
    [ "$run_on" = edge ] || { rec "td-$s-handoff-$i-run-on-$run_on" -- bash -c "echo 'a controller-side hand-off from the controller itself: $nx'; exit 1"; return 0; }
    cmd="$(python3 -c 'import json,sys; print(json.dumps(json.loads(sys.argv[1])["command"]))' "$nx")"
    commit="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["commit"])' "$nx")"
    # The controller's tree is clean before a hand-off: the command emits none otherwise.
    rec "td-$s-handoff-$i-controller-clean" -- docker exec "$HP-$s-ops" sh -c "cd /opt/goat && test -z \"\$(git status --porcelain)\" && git rev-parse HEAD | grep -qx '$commit'"
    rec "td-$s-handoff-$i-edge-at-the-commit" -- bash -c "td_sync_checkout '$s' '$commit'"
    : > "$RECORD/td-$s-carry-mismatch.txt"
    # Each carried work file, copied to the same path under the edge driver's <work> and checked.
    python3 -c 'import json,sys; [print(c["file"], c["sha256"]) for c in json.loads(sys.argv[1])["carry"] if c["in"] == "work"]' "$nx" | while read -r f h; do
      docker cp -q "$HP-$s-ops:/var/lib/goat-federation/$them/$f" "$WORK/$s/td-carry"
      docker exec "$HP-$s-edrv" mkdir -p "/var/lib/goat-federation/$them/$(dirname "$f")"
      docker cp -q "$WORK/$s/td-carry" "$HP-$s-edrv:/var/lib/goat-federation/$them/$f"
      [ "$(sha256sum "$WORK/$s/td-carry" | cut -d' ' -f1)" = "$h" ] || echo "carry $f: sha256 differs" >> "$RECORD/td-$s-carry-mismatch.txt"
    done
    rec "td-$s-handoff-$i-carry-matches" -- bash -c "[ ! -s '$RECORD/td-$s-carry-mismatch.txt' ] || { cat '$RECORD/td-$s-carry-mismatch.txt'; exit 1; }"
    way="$RECORD/td-$s-edge-$i.json"
    rec "td-$s-handoff-$i-edge-ran" -- bash -c \
      "docker exec -e FED_WORK=/var/lib/goat-federation/$them -e HOME=/root -w /opt/goat $HP-$s-edrv \$(python3 -c 'import json,shlex,sys; print(shlex.join(json.loads(sys.argv[1])))' '$cmd') -o json > '$way'; rc=\$?; tail -c 2000 '$way'; exit \$rc"
    # The confirm code from the edge run's own output, and nothing else.
    f0="$FAILS"
    rec "td-$s-handoff-$i-way-back" -- python3 "$TD_HANDOFF" wayback "$way"
    [ "$FAILS" = "$f0" ] || return 0
    res="$(python3 "$TD_HANDOFF" wayback "$way")"
    rp="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["result_path"])' "$res")"
    code="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["confirm_code"])' "$res")"
    docker cp -q "$HP-$s-edrv:$rp" "$WORK/$s/td-result-$i"
    docker cp -q "$WORK/$s/td-result-$i" "$HP-$s-ops:/tmp/td-result-$i"
    imp="$(python3 "$TD_HANDOFF" fill "$(python3 -c 'import json,sys; print(json.dumps(json.loads(sys.argv[1])["import"]))' "$nx")" --result "/tmp/td-result-$i" --code "$code")"
    thn="$(python3 -c 'import json,sys; print(json.dumps(json.loads(sys.argv[1])["then"]))' "$nx")"
    rec "td-$s-handoff-$i-imported" -- bash -c \
      "docker exec -e FED_WORK=/var/lib/goat-federation/$them -e HOME=/root -w /opt/goat $HP-$s-ops \$(python3 -c 'import json,shlex,sys; print(shlex.join(json.loads(sys.argv[1])))' '$imp')"
    out="$RECORD/td-$s-ctl-$i.json"
    rec "td-$s-handoff-$i-then" -- bash -c \
      "docker exec -e FED_WORK=/var/lib/goat-federation/$them -e HOME=/root -e GOAT_AWS_MODE=none -w /opt/goat $HP-$s-ops \$(python3 -c 'import json,shlex,sys; print(shlex.join(json.loads(sys.argv[1])))' '$thn') -o json > '$out'; rc=\$?; tail -c 2000 '$out'; exit \$rc"
  done
  rec "td-$s-hand-offs-ended" -- bash -c "echo 'more than 20 hand-offs on $s'; exit 1"
}

# The contract's end state, once both sides have followed every hand-off.
td_end_state() {
  local s them
  for s in "$A" "$B"; do
    them="$(other "$s")"
    td_ctl "$s" "$them" -o json federate status "$them" > "$RECORD/td-$s-status.json" 2>&1 || true
    rec "td-$s-controller-status-at-phase-14" -- python3 - "$RECORD/td-$s-status.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1])); ps = (d.get("peerings") or [{}])[0].get("phases", [])
p14 = [x for x in ps if x.get("phase") == 14]
print(p14[-1] if p14 else "no phase 14 in the status")
sys.exit(0 if p14 and p14[-1].get("outcome") == "passed" else 1)
PY
    rec "td-$s-controller-journal-holds-every-phase-and-edge-digest" -- python3 - "$RECORD/td-$s-status.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1])); ps = (d.get("peerings") or [{}])[0].get("phases", [])
passed = {x.get("phase") for x in ps if x.get("outcome") == "passed"}
missing = [n for n in range(1, 15) if n not in passed]
edge = {x.get("phase") for x in ps if x.get("edge_result_sha256") or x.get("imported_sha256")}
print("not passed:", missing, "; phases with an imported edge result:", sorted(edge))
sys.exit(0 if not missing and {1, 2, 3, 11, 12, 13, 14} <= edge else 1)
PY
    rec "td-acceptance-as-$s-with-$them" --expect-in "acceptance-counts:" --expect-in "skip=0" --expect-in "fail=0" \
      --expect-not-in "ACCEPTANCE FAILED" --expect-not-in "ACCEPTANCE INCOMPLETE" -- bash -c "cat '$RECORD'/td-$s-ctl-*.json '$RECORD'/td-$s-edge-*.json 2>/dev/null | tail -c 20000"
  done
  # The edge's leaf keys were made on the edge driver: none is on a controller.
  rec "td-private-keys-stay-where-they-were-made" -- bash -c \
    "bad=0; for s in $A $B; do o=\$(other \$s); l=\$(docker exec $HP-\$s-ops sh -c \"ls /var/lib/goat-federation/\$o/pki/*outward-key.pem /var/lib/goat-federation/\$o/pki/*inward-key.pem 2>/dev/null\"); [ -z \"\$l\" ] || { echo \"\$s controller holds edge leaf keys: \$l\"; bad=1; }; done; exit \$bad"
  rec "td-edge-half-killed-and-resumed" -- bash -c "echo 'the kill of an edge half is measured once the hand-offs run end to end'; exit 1"
}
export -f td_ctl td_edge td_sync_checkout
