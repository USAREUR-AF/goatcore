#!/usr/bin/env bash
# units/bilateral.sh — B2 of docs/design/goatbuilder-v1-gate.md: the command completes the
# partner's half of acceptance.
#
# nothing-hand-written (units/nohand.sh), then both operators run
# `goatbuilder federate test <partner> --bilateral` at the same time, each on its own driver
# with only its own surfaces: no counterparty value (FED_THEIRS_REST, FED_SSH_THEIRS,
# FED_SPIRE_THEIRS, FED_PROBE_LAUNCHER) and no reach into the partner's network. Each side
# has its own probe launcher (FED_PROBE_LAUNCHER_OURS): sessions on its own edge host,
# holding its own identities, dialling the partner's edge, which the partner judges on its
# own edge's trace (docs/design/federation-acceptance.md §6.4). Each side's
# phase 14 derives the session from the agreement, signs its result with its federation key,
# publishes it on its own core, reads the partner's from its own core and verifies it under
# the key bound at phase 5 (docs/design/federation-acceptance.md §6.3). The checks read what
# each side's evidence directory holds afterwards: the partner's signed result, imported and
# authenticated under the partner's phase 5 key, and a combined verdict both sides share.
# shellcheck source=/dev/null
. "$HERE/units/nohand.sh"

# ba_own_launcher US THEM: our own probe launcher on our driver host, set in BA_PL_ENV.
# Its probes run on OUR edge host with OUR identities: our agreed leaf (from our driver's
# working directory), a name nobody agreed and a leaf naming the partner's core, both from
# our CA; they trust the partner's edge by the partner's CA certificate, which is public.
# Nothing of the partner's that is not public is copied.
ba_own_launcher() {
  local s="$1" them="$2" h pl far
  h="$(drv_host "$s")"; pl="$WORK/$s/own-launcher-$them"
  rm -rf "$pl"; mkdir -p "$pl"
  docker cp -q "$h:/var/lib/goat-federation/$them/pki/peering-outward.pem" "$pl/agreed-cert.pem"
  docker cp -q "$h:/var/lib/goat-federation/$them/pki/peering-outward-key.pem" "$pl/agreed-key.pem"
  cp "$WORK/$s/keys/forged.pem" "$pl/forged-cert.pem"; cp "$WORK/$s/keys/forged-key.pem" "$pl/forged-key.pem"
  cp "$WORK/$s/keys/impersonate-$them.pem" "$pl/impersonate-cert.pem"
  cp "$WORK/$s/keys/impersonate-$them-key.pem" "$pl/impersonate-key.pem"
  cp "$WORK/$them/keys/ca.pem" "$pl/far-ca.pem"; chmod 0644 "$pl"/*-cert.pem "$pl/far-ca.pem"; chmod 0600 "$pl"/*-key.pem
  docker cp -q "$h:/var/lib/goat-federation/$them/agreement.json" "$pl/agreement.json"
  # The partner's edge's link address: the probe dials it across the link.
  far="$(python3 -c 'import json,sys,ipaddress as i; ag=json.load(open(sys.argv[1])); h=list(i.ip_network(ag["transport"]["link_subnet"])); ps=[p["sovereign_id"] for p in ag["parties"]]; print(h[ps.index(sys.argv[2])])' "$pl/agreement.json" "$them")"
  host_exec "$h" install -d -m 0700 /root/fe2e-own-launcher
  for f in "$pl"/*-cert.pem "$pl"/*-key.pem "$pl/far-ca.pem"; do docker cp -q "$f" "$h:/root/fe2e-own-launcher/"; done
  docker cp -q "$HERE/lib/probe-launcher.sh" "$h:/usr/local/sbin/fe2e-probe-launcher"
  docker cp -q "$HERE/probe.json5" "$h:/root/fe2e-own-launcher/probe.json5"
  host_exec "$h" chmod 0755 /usr/local/sbin/fe2e-probe-launcher
  BA_PL_ENV=(-e FED_PROBE_LAUNCHER_OURS=/usr/local/sbin/fe2e-probe-launcher
    -e "FE2E_PL_REST_IP=$(edge_ip "$s")" -e "FE2E_PL_FAR_IP=$far"
    -e FE2E_PL_DIR=/root/fe2e-own-launcher -e FE2E_PL_PORT=8201
    -e FE2E_PL_TEMPLATE=/root/fe2e-own-launcher/probe.json5 -e "FE2E_PL_IMAGE=$ZENOH_IMAGE")
  # The probe's host is our own edge: this host when the driver runs there, else over ssh.
  if [ "$h" = "$HP-$s-edge" ]; then BA_PL_ENV+=(-e FE2E_PL_LOCAL=1)
  else BA_PL_ENV+=(-e "FE2E_PL_SSH=goat@$(edge_ip "$s")"); fi
}

# ba_run US THEM: phase 14 on a derived session, from our driver, with our own surfaces only.
ba_run() {
  local h; h="$(drv_host "$1")"
  oc_env "$1" "$2"
  ba_own_launcher "$1" "$2"
  # shellcheck disable=SC2046 # oc_as gives nothing or two words (--as <us>)
  docker exec "${OC_ENV[@]}" "${BA_PL_ENV[@]}" -e FED_ACCEPTANCE_STRICT=1 -w /opt/goat "$h" \
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
# probe_launcher is a drill's launcher, holding the partner's identities; spire_theirs is
# the partner's spire-server. probe_launcher_ours is this side's own (§6.4) and is required.
given = [k for k in ("theirs_rest", "ssh_theirs", "probe_launcher", "spire_theirs") if inputs.get(k)]
print("inputs:", json.dumps(inputs, sort_keys=True))
if given:
    print("given a counterparty surface: %s" % ", ".join(given)); sys.exit(1)
if not inputs.get("probe_launcher_ours"):
    print("no probe launcher of this side's own (FED_PROBE_LAUNCHER_OURS)"); sys.exit(1)
print("the run held only its own surfaces, with its own probe launcher")
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
  # Required: each side observes the partner's own probes on its own edge and the
  # partner's staged SVID against its own server (§6.4), so with own surfaces only both
  # combined verdicts are PASS.
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
