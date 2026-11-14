#!/usr/bin/env bash
# units/bilateral.sh — B2 of docs/design/goatbuilder-v1-gate.md: the command completes the
# partner's half of acceptance.
#
# nothing-hand-written (units/nohand.sh), then both operators run
# `goatbuilder federate test <partner> --bilateral` at the same time, each on its own driver
# with only its own surfaces: no counterparty value (FED_THEIRS_REST, FED_SSH_THEIRS,
# FED_SPIRE_THEIRS, FED_PROBE_LAUNCHER) and no reach into the partner's network. Each side's
# phase 14 derives the session from the agreement, signs its result with its federation key,
# publishes it on its own core, reads the partner's from its own core and verifies it under
# the key bound at phase 5 (docs/design/federation-acceptance.md §6.3). The checks read what
# each side's evidence directory holds afterwards: the partner's signed result, imported and
# authenticated under the partner's phase 5 key, and a combined verdict both sides share.
# shellcheck source=/dev/null
. "$HERE/units/nohand.sh"

# ba_run US THEM: phase 14 on a derived session, from our driver, with our own surfaces only.
ba_run() {
  local h; h="$(drv_host "$1")"
  oc_env "$1" "$2"
  # shellcheck disable=SC2046 # oc_as gives nothing or two words (--as <us>)
  docker exec "${OC_ENV[@]}" -e FED_ACCEPTANCE_STRICT=1 -w /opt/goat "$h" \
    goatbuilder -C /opt/goat --yes federate test "$2" --bilateral $(oc_as "$1")
}

# ba_block REASON: every check of scenario bilateral-acceptance BLOCKED with one reason.
ba_block() {
  local side c
  for side in first second; do
    for c in run own-surfaces holds-signed-result verdict-pass; do
      mark "bilateral-acceptance.$side-$c" BLOCKED "$1"
    done
  done
  mark bilateral-acceptance.verdicts-agree BLOCKED "$1"
}

ba_main() {
  local s them h out pids=() rc
  if ! oc_passed "$RECORD/status-$A.json" 13 || ! oc_passed "$RECORD/status-$B.json" 13; then
    ba_block "phase 13 did not pass on both sides: $A: $(oc_stopped_at "$A"); $B: $(oc_stopped_at "$B")"
    return 0
  fi
  if ! docker exec "$(drv_host "$A")" goatbuilder federate --help 2>/dev/null | grep -q -- '--bilateral'; then
    ba_block "the tree under test has no \`goatbuilder federate test <partner> --bilateral\`"
    return 0
  fi
  # Both start together, as the design asks of the two operators: each waits up to
  # FED_SESSION_WAIT for the other's sample and signed result.
  for s in "$A" "$B"; do
    them="$(other "$s")"
    ( rc=0; ba_run "$s" "$them" > "$RECORD/bilateral-as-$s.out" 2>&1 || rc=$?; echo "$rc" > "$RECORD/bilateral-as-$s.rc" ) &
    pids+=("$!")
  done
  wait "${pids[@]}" || true
  for s in "$A" "$B"; do
    them="$(other "$s")"; h="$(drv_host "$s")"; out="$WORK/$s/ba-$them"
    rm -rf "$out"; mkdir -p "$out"
    docker cp -q "$h:/var/lib/goat-federation/$them/evidence/acceptance" "$out/evidence" 2>/dev/null || true
    docker cp -q "$h:/var/lib/goat-federation/$them/keys.txt" "$out/keys.txt" 2>/dev/null || true
    rc="$(cat "$RECORD/bilateral-as-$s.rc" 2>/dev/null || echo 2)"
    # Exit 2 is a refusal (the partner's result did not verify, or ours could not be signed).
    rec "ba-$s-$them-run" --expect-in "published our signed result" --expect-not-in "ACCEPTANCE REFUSED" \
      -- bash -c "cat '$RECORD/bilateral-as-$s.out'; echo \"exit $rc\"; [ '$rc' != 2 ]"
    rec "ba-$s-$them-own-surfaces" -- python3 - "$out/evidence/result.json" <<'PY'
import json, sys
try:
    b = json.load(open(sys.argv[1]))
except Exception as e:
    print("no result of this side's run: %s" % e); sys.exit(1)
inputs = b.get("inputs", {})
given = [k for k in ("theirs_rest", "ssh_theirs", "probe_launcher") if inputs.get(k)]
print("inputs:", json.dumps(inputs, sort_keys=True))
if given:
    print("given a counterparty surface: %s" % ", ".join(given)); sys.exit(1)
print("the run held only its own surfaces")
PY
    rec "ba-$s-$them-holds-signed-result" -- python3 - "$out" "$them" <<'PY'
import glob, json, os, sys
out, them = sys.argv[1], sys.argv[2]
combined = sorted(glob.glob(os.path.join(out, "evidence", "bilateral", "*.json")), key=os.path.getmtime)
if not combined:
    print("no combined result under evidence/bilateral/"); sys.exit(1)
c = json.load(open(combined[-1]))
bound = ""
for line in open(os.path.join(out, "keys.txt")) if os.path.exists(os.path.join(out, "keys.txt")) else []:
    k, _, v = line.strip().partition("=")
    if k == them:
        bound = v
peer = sorted(glob.glob(os.path.join(out, "evidence", "peer", them, "*.json")))
print("combined:", os.path.basename(combined[-1]), "verdict", c.get("verdict"),
      "authenticated", c.get("authenticated"), "peer_run", c.get("peer_run"))
print("peer key:", (c.get("peer_key") or {}).get("b64"), "bound at phase 5:", bound or "none")
print("partner results kept:", [os.path.basename(p) for p in peer])
ok = (c.get("authenticated") is True and bound and (c.get("peer_key") or {}).get("b64") == bound and peer)
sys.exit(0 if ok else 1)
PY
  done
  rec ba-verdicts-agree -- python3 - "$WORK/$A/ba-$B" "$WORK/$B/ba-$A" <<'PY'
import glob, json, os, sys
v = []
for d in sys.argv[1:]:
    f = sorted(glob.glob(os.path.join(d, "evidence", "bilateral", "*.json")), key=os.path.getmtime)
    v.append(json.load(open(f[-1])).get("verdict") if f else None)
print("combined verdicts:", v)
sys.exit(0 if v[0] is not None and v[0] == v[1] else 1)
PY
  # Recorded, not required: a combined PASS also needs the refusal probes, which hold the
  # partner's agreed identity, and the partner's SPIRE command, so without them it is open.
  for s in "$A" "$B"; do
    them="$(other "$s")"
    rec "ba-$s-$them-verdict-pass" --expect-in "verdict PASS" -- python3 - "$WORK/$s/ba-$them" <<'PY'
import glob, json, os, sys
f = sorted(glob.glob(os.path.join(sys.argv[1], "evidence", "bilateral", "*.json")), key=os.path.getmtime)
if not f:
    print("no combined result"); sys.exit(1)
c = json.load(open(f[-1]))
print("verdict", c.get("verdict"), "failed", c.get("failed"), "open", c.get("open"))
PY
  done
}

unit_main() {
  nh_main
  ba_main
  collect ba
}
