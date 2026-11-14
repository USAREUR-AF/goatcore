#!/usr/bin/env bash
# acceptance.sh — phase 14 of a peering standup. The terminal gate.
#
# FOUR TIERS, and the ordering matters (docs/design/federation-acceptance.md):
#
#   tier 2  POSITIVE   a fresh sample on every offered release set carries, in every
#                      direction the agreement has, read back from the RECEIVING
#                      core's own store and compared byte for byte
#   PERSISTENCE        a second sample on every offered set reaches the receiving
#                      core's store when only the store subscribes to its key, so
#                      no diagnostic interest can be what carried it
#   tier 3  NEGATIVE   the refusals fire — asserted only ALONGSIDE the positives,
#                      because a refusal checked against a dead path passes
#                      vacuously and proves nothing. Silence proves nothing
#                      either: a refusal counts when the receiving edge's
#                      ACL trace records it, beside an allow for the same identity.
#                      An unoffered release set is not lifted by the republisher,
#                      in each direction, as its own check
#   tier 4  ROUND TRIP a nonce staged here is acknowledged by the far side, and the
#                      far side's nonce by ours, where the agreement has a return path
#
# Tier 1 (per-phase end state) belongs to the phases themselves, not here.
#
# CHECKS COME FROM THE AGREEMENT'S FLOWS. A direction the agreement does not have
# (a send-only or receive-only peering) is NOT APPLICABLE, with its reason; a
# direction it has is always required.
#
# OUR OWN SURFACES ARE READ FROM THE DECLARATIONS (acceptance_surfaces.py), each only
# when the environment does not set it, so these stay overrides:
#   FED_OURS_REST            our core's REST surface: goat_zenoh_rest_host and
#                            goat_zenoh_rest_port of <them>_core, on its inventory host
#                            (read on the core over FED_SSH_OURS, derived from the same
#                            host, when the surface is bound to loopback)
#   FED_EDGE_LOG_OURS        `docker logs <goat_zenoh_edge_stack>` on the <them>_edge host
#   FED_SPIRE_OURS           `docker exec <peering_spire_container> <peering_spire_bin>`
#                            on peering_spire_server_host, when peering_spire_enabled
# from ops/ansible/inventory/peering-<edge group>.ini (when allocate rendered it) and
# peering-edges.ini, and ops/ansible/group_vars (groups from
# FED_EDGE_GROUP / FED_CORE_GROUP, which both avenues export for phase 14). stderr says where each came from, or
# what was looked for when it could not be derived.
#
# WHAT A DRILL ADDS, because it owns both sides. Each is optional; absent, its checks
# print BLOCKED or skip, and a check that did not run is not a pass: the run exits 3.
#
#   FED_THEIRS_REST          their core's REST surface
#   FED_EDGE_LOG_OURS        command printing OUR edge router's log, ACL trace on
#                            (goat_zenoh_edge_acl_trace)
#   FED_PROBE_THEIRS_REST    REST surface of a client session across the link holding
#                            THEIR agreed peering identity, connected to our edge
#   FED_PROBE_THEIRS_ZID     that session's zenoh id (optional): when set, the allowed
#                            control must have come from exactly that session
#   FED_PROBE_LAUNCHER       command that starts, stops and checks the three probe
#                            sessions for this run only (see RUN-OWNED PROBES below);
#                            FED_PROBE_TTL their lifetime in seconds (default 900)
#   FED_SSH_KNOWN_HOSTS      known_hosts file: ssh verifies each core's host key
#   FED_PROBE_FORGED_REST    the same, holding a certificate from THEIR CA with a
#                            common name the agreement never named
#   FED_PROBE_FORGED_ZID     that session's zenoh id, the only name the router logs for
#                            a peer matching no subject
#   FED_PROBE_IMPERSONATE_REST  the same, holding a certificate from THEIR CA bearing
#                            OUR core's common name
#   FED_PROBE_IMPERSONATE_ZID   that session's zenoh id
#   FED_SPIRE_OURS           command prefix running OUR `spire-server` CLI
#   FED_SPIRE_THEIRS         command prefix running THEIR `spire-server` CLI
#   FED_TEST_SUBSCRIBERS_PAUSE_OURS / _THEIRS    command, run on that core's side,
#                            that stops the test subscribers a drill keeps there
#   FED_TEST_SUBSCRIBERS_RESUME_OURS / _THEIRS   command that starts them again
#
# EXISTING PARTNERSHIPS, opt-in. FED_EXISTING_WORKS names the work directories of
# the peerings this side already has (colon-separated, each holding its
# agreement.json). `acceptance.sh <agreement> --before` checks only those, ahead of
# the deployment, and keeps the result in <evidence>/before/; the run after the
# deployment checks them again and compares. FED_EXISTING_REST (`<id>=<url>,...`,
# a drill's) lets it read an existing partner's core for cross-partner leakage.
#
# WHERE THE PROBES RUN. From the operator's machine. FED_OURS_REST / FED_THEIRS_REST
# are read through FED_SSH_OURS / FED_SSH_THEIRS when set (ssh to the core host, which
# runs curl there), else from here. The round-trip script is streamed over that ssh
# (`bash -s`); nothing is copied to a host.
#
# OUTCOMES. Each check ends PASS (the expected evidence was observed), FAIL (a
# violation was observed), BLOCKED (a prerequisite or the evidence was unavailable,
# including a REST surface that did not answer), SKIP (not exercised) or
# NOT APPLICABLE (excluded by the agreement's own flows).
#
# EVIDENCE. Every check is recorded with its name, direction, verdict, reason, times
# and raw observations, and the run writes one bundle (acceptance_evidence.py):
#   <dir of the agreement>/evidence/acceptance/result.json   schema goat-federation-acceptance/v1
#   .../summary.md                                           the same, for a person
#   .../history/<run id>.json                                every run, kept
# FED_EVIDENCE_DIR overrides the directory. A bundle that cannot be written fails the run.
#
# EXIT STATUS (from the bundle's verdict). 0 only when every required check passed.
# 1 when a check failed. 3 when none failed but a required check was BLOCKED or
# SKIPPED, so the peering is not proven. 2 when the agreement cannot be read or the
# evidence cannot be written. FED_ACCEPTANCE_STRICT is still accepted and changes
# nothing: a check that did not run fails the run with or without it.
#
# TIMING, for tests only: FED_ACCEPTANCE_WAIT replaces every settle wait (seconds;
# default the wait each step names), FED_ACCEPTANCE_GAP the pause between two
# samples (default from the agreement's lowest hz, at least 0.5),
# FED_ACCEPTANCE_RT_TIMEOUT the round-trip wait (default 25). A real edge needs the
# defaults.
#
# THE REPUBLISHER IS THE ONLY WRITER OF OUR WIRE NAME. Every positive row publishes
# under a `release/<set>/...` staging name on the sending core and reads the sample
# back under `from/<sender>/<set>/...` on the receiving side, once per offered set,
# so a pass proves the republisher lifted it, the core admitted it, and the edges
# carried it. A row that wrote the wire name itself would pass with no republisher
# running at all.
#
# Every key expression is READ FROM THE AGREEMENT. Typing them here would make
# this suite agree with itself rather than with the peering: a test that carries
# its own copy of the grants passes when the deployment is wrong in the same way
# the test is.
set -uo pipefail

AGREEMENT="${1:?usage: acceptance.sh <agreement.json> [--as <sovereign>] [--before]}"; shift || true
AS=""; BEFORE=0
while [ $# -gt 0 ]; do
  case "$1" in --as) AS="$2"; shift 2 ;; --before) BEFORE=1; shift ;; *) shift ;; esac
done
[ -f "$AGREEMENT" ] || { echo "no agreement at $AGREEMENT" >&2; exit 2; }

: "${FED_THEIRS_REST:=}"                        # theirs, when the drill owns both sides
: "${FED_SSH_OURS:=}"                           # or reach them over ssh
: "${FED_SSH_THEIRS:=}"
# A drill that names a command to stop its test subscribers names the one that starts
# them again too; otherwise they would stay stopped after the run with no row saying so.
for side in OURS THEIRS; do
  eval "pause=\${FED_TEST_SUBSCRIBERS_PAUSE_$side:-}; resume=\${FED_TEST_SUBSCRIBERS_RESUME_$side:-}"
  if [ -n "$pause" ] && [ -z "$resume" ]; then
    echo "acceptance: FED_TEST_SUBSCRIBERS_PAUSE_$side is set without FED_TEST_SUBSCRIBERS_RESUME_$side; set both or neither" >&2
    exit 2
  fi
done

settle() { sleep "${FED_ACCEPTANCE_WAIT:-$1}"; }
now()    { date -u +%Y-%m-%dT%H:%M:%SZ; }

# --- the evidence ------------------------------------------------------------
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EVIDENCE="$HERE/acceptance_evidence.py"
EVID="${FED_EVIDENCE_DIR:-$(cd "$(dirname "$AGREEMENT")" && pwd)/evidence/acceptance}"
mkdir -p "$EVID" 2>/dev/null && [ -w "$EVID" ] \
  || { echo "acceptance: cannot write the evidence directory $EVID (set FED_EVIDENCE_DIR)" >&2; exit 2; }
RUN_STARTED="$(now)"
# The run id is the nonce in every key and payload this run writes: unique per run,
# so a value retained from an earlier run can never satisfy a check of this one.
RUN_ID="acc-$(date -u +%Y%m%dT%H%M%SZ)-$$-$(od -An -N4 -tx1 /dev/urandom | tr -d ' \n')"
S="$RUN_ID"
CHECKS="$EVID/.checks-$RUN_ID.jsonl"; OBSF="$EVID/.obs-$RUN_ID"
REST_ERR="$EVID/.rest-$RUN_ID"; TRACE="$EVID/.trace-$RUN_ID"
ADM_R="$EVID/.adm-router-$RUN_ID"; ADM_S="$EVID/.adm-subs-$RUN_ID"
: > "$CHECKS"; : > "$OBSF"
trap 'rm -f "$CHECKS" "$OBSF" "$REST_ERR" "$TRACE" "$ADM_R" "$ADM_S"' EXIT
T0="$RUN_STARTED"
pass=0; fail=0; skip=0; blocked=0; RECORD_FAILED=""

# begin: the start time of the checks that follow. obs KEY VALUE: one raw observation
# for the next check recorded.
begin() { T0="$(now)"; }
obs()   { printf '%s\t%s\n' "$1" "$(printf '%s' "$2" | base64 | tr -d '\n')" >> "$OBSF"; }
# record NAME DIRECTION TIER VERDICT REASON [--optional]
record() {
  if ! python3 "$EVIDENCE" record "$CHECKS" --name "$1" --direction "$2" --tier "$3" --verdict "$4" \
       --reason "$5" --title "$5" --started "$T0" --obs-file "$OBSF" ${6:+"$6"}; then
    printf '  \033[31mFAIL\033[0m  the evidence for %s could not be recorded\n' "$1"
    fail=$((fail+1)); RECORD_FAILED="$RECORD_FAILED $1"
  fi
  : > "$OBSF"
}
# ok|no|skp|blk|na NAME DIRECTION TIER MESSAGE [DETAIL]
ok()  { printf '  \033[32mpass\033[0m  %s\n' "$4"; pass=$((pass+1)); record "$1" "$2" "$3" PASS "$4"; }
no()  { printf '  \033[31mFAIL\033[0m  %s\n' "$4"; [ -n "${5:-}" ] && printf '        %s\n' "$5"; fail=$((fail+1))
        record "$1" "$2" "$3" FAIL "$4${5:+ — $5}"; }
skp() { printf '  \033[33mskip\033[0m  %s\n' "$4"; [ -n "${5:-}" ] && printf '        %s\n' "$5"; skip=$((skip+1))
        record "$1" "$2" "$3" SKIP "$4${5:+ — $5}"; }
blk() { printf '  \033[33mBLOCKED\033[0m  %s\n' "$4"; [ -n "${5:-}" ] && printf '        %s\n' "$5"; blocked=$((blocked+1))
        record "$1" "$2" "$3" BLOCKED "$4${5:+ — $5}"; }
na()  { printf '  n/a   %s\n' "$4"; record "$1" "$2" "$3" "NOT APPLICABLE" "$4" --optional; }

# --- the lanes, from the agreement -------------------------------------------
read -r US THEM OUR_WIRE THEIR_WIRE THEIR_TD OUR_SETS THEIR_SETS OUR_WILD THEIR_WILD GAP_DEFAULT <<EOF
$(python3 - "$AGREEMENT" "$AS" <<'PY'
import json, re, sys
ag = json.load(open(sys.argv[1])); want = sys.argv[2] or ""
# Accept only well-formed names from the agreement. They become keys, payloads and
# arguments of the commands below, so anything outside these shapes stops the run.
SOVEREIGN = re.compile(r"[a-z0-9][a-z0-9-]{1,15}")
RELEASE = re.compile(r"release/[A-Za-z0-9._/*-]+")
TRUST_DOMAIN = re.compile(r"spiffe://[A-Za-z0-9._-]+")
def refuse(what, value):
    print("acceptance: the agreement holds %s that is not well-formed: %r" % (what, value), file=sys.stderr)
    sys.exit(1)
for p in ag.get("parties", []):
    if not SOVEREIGN.fullmatch(str(p.get("sovereign_id", ""))):
        refuse("a sovereign id", p.get("sovereign_id"))
    if not TRUST_DOMAIN.fullmatch(str(p.get("trust_domain", ""))):
        refuse("a trust domain", p.get("trust_domain"))
for f in ag.get("flows", []):
    for r in f.get("release", []):
        if not RELEASE.fullmatch(str(r)):
            refuse("a release set name", r)
parties = [p["sovereign_id"] for p in ag.get("parties", [])]
us = want if want in parties else parties[0]
them = [p for p in parties if p != us][0]
# The WIRE name each side publishes under. `release/**` is the staging name and
# never appears on the link, so asserting it here would assert a
# key the peering is correct to refuse.
td = {p["sovereign_id"]: p["trust_domain"] for p in ag.get("parties", [])}
# The staging prefix of every offered set, `release/<set>/**` less its `/**`, one
# comma-separated list per direction ("-" for none). A pattern with a wildcard
# before its tail names no single key to publish: it goes to the second list, and
# its lane is reported as not proven rather than silently left out.
def sets(frm):
    out, wild = [], []
    for f in ag.get("flows", []):
        if f.get("from") != frm:
            continue
        for r in f.get("release", []):
            stem = r[:-3] if r.endswith("/**") else r
            (out if stem.startswith("release/") and "*" not in stem else wild).append(stem)
    return ",".join(out) or "-", ",".join(wild) or "-"
# Two samples closer than the receiving edge's agreed `hz` are one too many for its
# downsampling, and the second is dropped with nothing logged. Half a second, or
# longer when an agreed rate is slower than three in two seconds.
hz = [f.get("envelope", {}).get("hz") for f in ag.get("flows", [])]
hz = [h for h in hz if isinstance(h, (int, float)) and h > 0]
gap = max([0.5] + [1.5 / h for h in hz])
o, ow = sets(us); t, tw = sets(them)
print(us, them, f"from/{us}", f"from/{them}", td[them], o, t, ow, tw, f"{gap:.2f}")
PY
)
EOF
gap() { sleep "${FED_ACCEPTANCE_GAP:-$GAP_DEFAULT}"; }
# staging prefix -> the wire key it crosses as: `from/<sender>/` + the staging key
# less its first segment (ops/federation/republisher/README.md, the key mapping).
wire_of() { printf 'from/%s/%s' "$1" "${2#release/}"; }
list() { [ "$1" = "-" ] || printf '%s\n' "$1" | tr ',' '\n'; }
[ -n "$US" ] || { echo "acceptance: could not read well-formed parties and sets from the agreement" >&2; exit 2; }
echo "acceptance — $US <-> $THEM  (run $RUN_ID)"
echo "  our wire name:   $OUR_WIRE/**"
echo "  their wire name: $THEIR_WIRE/**"
echo "  our offered sets:   $OUR_SETS"
echo "  their offered sets: $THEIR_SETS"
echo "  evidence: $EVID"
echo

# OUR SURFACES, FROM THE DECLARATIONS. Our core's REST surface, our edge router's
# log and our SPIRE server are read from the inventory and group_vars the plays read
# (acceptance_surfaces.py names each source). A value already in the environment is
# an override and is kept. The derived edge log and SPIRE commands run on their own
# hosts over ssh (EDGE_LOG_VIA, SPIRE_VIA); a value set by hand runs where it always
# has. What cannot be derived stays unset, and its row says what to set.
EDGE_LOG_VIA=""; SPIRE_VIA=""
if surfaces=$(python3 "$HERE/acceptance_surfaces.py" --peering "$THEM"); then eval "$surfaces"; fi
: "${FED_OURS_REST:=http://127.0.0.1:8000}"     # our fabric's REST surface

# With FED_SSH_KNOWN_HOSTS (a known_hosts file holding the cores' host keys) every ssh
# verifies the host's identity and refuses an unknown or changed key. Unset, ssh runs
# as it always has here, without checking; making verification the default is its
# own change.
ssh_opts() {
  if [ -n "${FED_SSH_KNOWN_HOSTS:-}" ]; then
    printf '%s\n' -o StrictHostKeyChecking=yes -o "UserKnownHostsFile=$FED_SSH_KNOWN_HOSTS"
  else
    printf '%s\n' -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null
  fi
}
side_ssh() { # TARGET COMMAND
  local IFS='
'
  # shellcheck disable=SC2046
  timeout 60 ssh $(ssh_opts) "$1" "$2" 2>/dev/null
}
ours()   { if [ -n "$FED_SSH_OURS" ]; then side_ssh "$FED_SSH_OURS" "$1"; else eval "$1"; fi; }
theirs() { if [ -n "$FED_SSH_THEIRS" ]; then side_ssh "$FED_SSH_THEIRS" "$1"; else eval "$1"; fi; }

# A read prints the value and exits 0 (nothing printed: the surface answered and
# holds no such key), or exits 4 when the surface did not answer properly. The two
# must never be confused: a dead surface reads as "nothing arrived", and every
# refusal check would pass on it. The JSON is parsed, not grepped.
value()      { python3 "$EVIDENCE" rest-value 2>"$REST_ERR"; }
# Each argument is quoted with printf %q before it joins a command string that runs
# here or on the core host.
get_ours()   { ours   "curl -s -m 8 -w '\n%{http_code}' $(printf %q "$FED_OURS_REST/$1")"   | value; }
get_theirs() { theirs "curl -s -m 8 -w '\n%{http_code}' $(printf %q "$FED_THEIRS_REST/$1")" | value; }
put_ours()   { ours   "curl -s -m 8 -X PUT -d $(printf %q "$2") $(printf %q "$FED_OURS_REST/$1") -o /dev/null"; }
put_theirs() { theirs "curl -s -m 8 -X PUT -d $(printf %q "$2") $(printf %q "$FED_THEIRS_REST/$1") -o /dev/null"; }
sha()        { printf '%s' "$1" | { if command -v sha256sum >/dev/null 2>&1; then sha256sum; else shasum -a 256; fi; } | cut -d' ' -f1; }

# get_side ours|theirs KEY: sets GOT, GOT_RC (0 read, else unreadable) and GOT_ERR.
get_side() {
  if [ "$1" = ours ]; then GOT=$(get_ours "$2"); else GOT=$(get_theirs "$2"); fi
  GOT_RC=$?; GOT_ERR=$(cat "$REST_ERR" 2>/dev/null)
}

# absent NAME DIRECTION TIER SIDE KEY PASS-MESSAGE FAIL-MESSAGE: a key that must not
# have arrived. A surface that cannot be read is BLOCKED, never a pass.
absent() {
  get_side "$4" "$5"; obs key "$5"; obs received "$GOT"
  if [ "$GOT_RC" -ne 0 ]; then blk "$1" "$2" "$3" "$6: not proven, the receiving core's REST surface did not answer" "$GOT_ERR"
  elif [ -z "$GOT" ]; then ok "$1" "$2" "$3" "$6"
  else no "$1" "$2" "$3" "$7" "read '$GOT'"; fi
}

# Whether each direction exists in the agreement.
HAVE_OUT=1; [ "$OUR_SETS" = "-" ] && [ "$OUR_WILD" = "-" ] && HAVE_OUT=0
HAVE_IN=1;  [ "$THEIR_SETS" = "-" ] && [ "$THEIR_WILD" = "-" ] && HAVE_IN=0
NO_OUT="the agreement offers no release set from $US (a receive-only peering)"
NO_IN="the agreement offers no release set from $THEM (a send-only peering)"

# ---- A BILATERAL SESSION (FED_ACCEPTANCE_SESSION, acceptance_session.py). Each side
#      runs on its own core and edge only, with no counterparty surface. The manifest
#      both imported fixes the keys and the bytes: each side stages under
#      release/<set>/acceptance/<session id>/<itself> a value carrying the challenge
#      the manifest issued to it, and reads the partner's from its own core. What
#      only the partner can observe (our samples arriving on their core) is BLOCKED
#      here and names the partner's row that observes it (`observed_by_peer`), for
#      `acceptance_session.py combine` once their bundle is imported.
# ---- A DERIVED SESSION (FED_ACCEPTANCE_ATTEMPT, docs/design/federation-acceptance.md
#      §6.2, §6.3). No manifest passes between the operators: each side derives the
#      same session from the agreement it holds and the attempt number, draws a new
#      challenge for this run from the OS CSPRNG, and learns the partner's from the
#      partner's sample. After the rows, each side signs its result, publishes it on
#      its own core under the session's key, reads and verifies the partner's, and
#      combines. Both sides start phase 14 within FED_SESSION_WAIT of each other.
#      FED_ACCEPTANCE_SIGNER and FED_ACCEPTANCE_VERIFIER are the commands that sign
#      our result and verify theirs (a command and its fixed words, split on spaces);
#      the mode refuses to start without both, so no result is published unsigned.
DERIVED=""
if [ -n "${FED_ACCEPTANCE_ATTEMPT:-}" ]; then
  if [ -n "${FED_ACCEPTANCE_SESSION:-}" ]; then
    echo "acceptance: FED_ACCEPTANCE_ATTEMPT (a derived session) and FED_ACCEPTANCE_SESSION (a manifest file) both name a session; give one" >&2
    exit 2
  fi
  for v in FED_ACCEPTANCE_SIGNER FED_ACCEPTANCE_VERIFIER; do
    if [ -z "${!v:-}" ]; then
      echo "acceptance: a derived session needs $v (the command that signs our result, and the one that verifies the partner's); no result is published unsigned" >&2
      exit 2
    fi
  done
  SDIR="$EVID/session"
  mkdir -p "$SDIR" || { echo "acceptance: cannot write $SDIR" >&2; exit 2; }
  python3 "$HERE/acceptance_session.py" derive --agreement "$AGREEMENT" --us "$US" --attempt "$FED_ACCEPTANCE_ATTEMPT" \
    --out "$SDIR/derived.json" >/dev/null || { echo "acceptance: no session could be derived from $AGREEMENT" >&2; exit 2; }
  SESSION_ID=$(python3 -c 'import json, sys; print(json.load(open(sys.argv[1]))["session_id"])' "$SDIR/derived.json")
  AGREEMENT_HEX=$(python3 -c 'import json, sys; print(json.load(open(sys.argv[1]))["agreement_sha256"].split(":", 1)[1])' "$SDIR/derived.json")
  MY_CHAL=$(python3 "$HERE/acceptance_session.py" challenge)
  THEIR_CHAL=""
  DERIVED=1
  echo "  session: $SESSION_ID (derived, attempt $FED_ACCEPTANCE_ATTEMPT)"
  echo
fi
SESSION="${FED_ACCEPTANCE_SESSION:-}"
if [ -n "$SESSION" ]; then
  python3 "$HERE/acceptance_session.py" validate --manifest "$SESSION" --agreement "$AGREEMENT" --us "$US" \
    || { echo "acceptance: the session manifest $SESSION is not valid for this run" >&2; exit 2; }
  read -r SESSION_ID MY_CHAL THEIR_CHAL _ <<EOF
$(python3 "$HERE/acceptance_session.py" fields --manifest "$SESSION" --us "$US")
EOF
  echo "  session: $SESSION_ID"
  echo
fi
[ -n "$DERIVED" ] && SESSION=derived
session_lanes() {
  local set sk key val i landed=0 waits ops_key ng_key gops gng
  sk="acceptance/$SESSION_ID"
  waits=$(( ${FED_SESSION_WAIT:-300} / 2 )); [ "$waits" -ge 1 ] || waits=1
  begin
  # What we stage on our own core, for the partner's runner to observe on theirs:
  # one sample per offered set, a second (persist) sample per set, a set nobody
  # offered, and two writes outside every lane.
  for set in $(list "$OUR_SETS"); do
    put_ours "$set/$sk/$US" "$US-$MY_CHAL-${set#release/}"; gap
    put_ours "$set/$sk/$US/persist" "$US-$MY_CHAL-${set#release/}-persist"; gap
  done
  if [ "$HAVE_OUT" = 1 ]; then
    put_ours "release/$US-not-offered/$sk/$US" "nope-$MY_CHAL"; gap
    put_ours "ops/fabric/acceptance-$SESSION_ID-$US" "nope-$MY_CHAL"; gap
    put_ours "from/$US-not-granted/acceptance-$SESSION_ID" "nope-$MY_CHAL"; gap
  fi
  for set in $(list "$OUR_SETS"); do
    begin; val="$US-$MY_CHAL-${set#release/}"
    get_side ours "$set/$sk/$US"; obs key "$set/$sk/$US"; obs staged "$GOT"
    if [ "$GOT_RC" -ne 0 ]; then blk "publish/outbound/$set" outbound positive "our session sample under $set/**: our core's REST surface did not answer" "$GOT_ERR"
    elif [ "$GOT" = "$val" ]; then ok "publish/outbound/$set" outbound positive "our session sample is staged under $set/** with our challenge"
    else no "publish/outbound/$set" outbound positive "our session sample under $set/** was not staged" "read '${GOT:-<nothing>}'"; fi
    begin; obs observed_by_peer "delivery/inbound/$set"; obs key "$(wire_of "$US" "$set")/$sk/$US"
    blk "delivery/outbound/$set" outbound positive "our $set/** arriving on their core is observed by $THEM's runner in this session" \
      "Import their bundle (acceptance_session.py import) and combine."
    begin; obs observed_by_peer "persistence/inbound/$set"
    blk "persistence/outbound/$set" outbound persistence "our $set/** persisting in their core's store is observed by $THEM's runner in this session"
  done
  if [ "$HAVE_OUT" = 1 ]; then
    begin; obs observed_by_peer "republisher-boundary/inbound"
    blk republisher-boundary/outbound outbound negative "an unoffered set of ours not lifted: observed by $THEM's runner in this session"
    begin; obs observed_by_peer "out-of-lane/inbound"
    blk out-of-lane/outbound outbound negative "our writes outside every lane not reaching them: observed by $THEM's runner in this session"
  fi
  for set in $(list "$THEIR_SETS"); do
    begin; key="$(wire_of "$THEM" "$set")/$sk/$THEM"; val="$THEM-$THEIR_CHAL-${set#release/}"
    if [ -n "$DERIVED" ]; then
      wait_for_partner "$key"
      learn_their_challenge "$GOT" "$set"
      val="$THEM-$THEIR_CHAL-${set#release/}"
      i=1
    else
      for i in $(seq 1 "$waits"); do
        get_side ours "$key"
        [ "$GOT_RC" -eq 0 ] && [ -n "$GOT" ] && break
        settle 2
      done
    fi
    obs key "$key"; obs sent_expected "$val"; obs received "$GOT"; obs attempts "$i"
    obs sent_sha256 "$(sha "$val")"; obs received_sha256 "$(sha "$GOT")"
    if [ "$GOT_RC" -ne 0 ]; then blk "delivery/inbound/$set" inbound positive "$THEM's session sample under $set/**: our core's REST surface did not answer" "$GOT_ERR"
    elif [ "$GOT" = "$val" ]; then ok "delivery/inbound/$set" inbound positive "$THEM's session sample on $set/** arrived as $(wire_of "$THEM" "$set")/** with their challenge, byte for byte"; landed=1
    elif [ -z "$GOT" ]; then blk "delivery/inbound/$set" inbound positive "nothing from $THEM on $set/** within ${FED_SESSION_WAIT:-300}s" \
      "Either they have not run this session yet, or it was not delivered; re-run once they have."
    else no "delivery/inbound/$set" inbound positive "$THEM's session sample on $set/** arrived with other bytes" "sent sha256 $(sha "$val"), read sha256 $(sha "$GOT")"; fi
    session_persist "$set"
  done
  if [ "$HAVE_IN" = 1 ]; then
    begin
    if [ "$landed" = 1 ]; then
      # Their unoffered and out-of-lane writes are staged after their offered ones
      # and still have to reach both edges: give them the main path's settle.
      settle 8
      absent republisher-boundary/inbound inbound negative ours "$(wire_of "$THEM" "release/$THEM-not-offered")/$sk/$THEM" \
        "a staging set of theirs the agreement never offered did NOT arrive on our core in this session" \
        "release/$THEM-not-offered/** arrived on our core: their republisher forwarded a set nobody agreed"
      begin; ops_key="ops/fabric/acceptance-$SESSION_ID-$THEM"; ng_key="from/$THEM-not-granted/acceptance-$SESSION_ID"
      get_side ours "$ops_key"; gops="$GOT"; rops=$GOT_RC
      get_side ours "$ng_key"; gng="$GOT"
      obs keys "$ops_key $ng_key"; obs received "$gops$gng"
      if [ "$rops" -ne 0 ] || [ "$GOT_RC" -ne 0 ]; then blk out-of-lane/inbound inbound negative "their writes outside every lane: our core's REST surface did not answer" "$GOT_ERR"
      elif [ -z "$gops$gng" ]; then ok out-of-lane/inbound inbound negative "their writes outside every lane did NOT arrive on our core in this session"
      else no out-of-lane/inbound inbound negative "a write of theirs outside every lane arrived on our core" "read '$gops$gng'"; fi
      begin
      edge_refusals
    else
      skp republisher-boundary/inbound inbound negative "their unoffered set: not asserted" "no session sample of theirs landed, so its absence would prove nothing"
      skp out-of-lane/inbound inbound negative "their writes outside every lane: not asserted" "no session sample of theirs landed"
      skp refusals inbound negative "refusal checks not run" "no session sample of theirs landed, so a refusal here would prove nothing"
    fi
  else
    begin; edge_refusals
  fi
}
# wait_for_partner KEY: in a derived session, read KEY on our core until something is
# there or FED_SESSION_WAIT runs out, saying how long is left, so the operator knows
# to start the other side. Sets GOT, GOT_RC and GOT_ERR as get_side does.
wait_for_partner() {
  local end next=0 left
  end=$((SECONDS + ${FED_SESSION_WAIT:-300}))
  while :; do
    get_side ours "$1"
    [ "$GOT_RC" -eq 0 ] && [ -n "$GOT" ] && return 0
    left=$((end - SECONDS))
    [ "$left" -le 0 ] && return 0
    if [ "$SECONDS" -ge "$next" ]; then
      echo "  waiting for $THEM's phase 14, ${left}s left"
      next=$((SECONDS + 10))
    fi
    settle 2
  done
}

# learn_their_challenge VALUE SET: the partner's challenge is the one their sample
# carries, `<them>-<challenge>-<set>`, read once and held to its shape. It is never
# taken from a file; a sample of another shape teaches nothing.
learn_their_challenge() {
  local v="${1#"$THEM"-}" set="${2#release/}"
  [ -n "$THEIR_CHAL" ] && return 0
  [ "$v" = "$1" ] && return 0
  v="${v%-"$set"}"
  if [[ "$v" =~ ^[0-9a-f]{16,64}$ ]]; then THEIR_CHAL="$v"; fi
  return 0
}

# session_persist SET: the partner's second sample on SET reaches our store when only
# the store subscribes to its key. The scope is read before the sample is read; the
# partner staged it in their own run, so a subscriber that existed then and is gone
# now is not seen (stated in the design document).
session_persist() {
  local set="$1" key val n i rc
  n="persistence/inbound/$set"; key="$(wire_of "$THEM" "$set")/acceptance/$SESSION_ID/$THEM/persist"
  val="$THEM-$THEIR_CHAL-${set#release/}-persist"
  begin
  scope ours "$key"; rc=$?
  obs key "$key"; obs scope "$SCOPE"
  case "$rc" in
    0) ;;
    1) no "$n" inbound persistence "no store on our core subscribes to $key" "$(scope_reason)"; return 0 ;;
    5) blk "$n" inbound persistence "$set/** on our core: a session other than the core's own store subscribes to the key" "$(scope_reason)"; return 0 ;;
    *) blk "$n" inbound persistence "$set/** on our core: who subscribes to the key could not be read" "$(scope_reason)"; return 0 ;;
  esac
  for i in 1 2 3 4 5 6 7 8; do
    get_side ours "$key"
    [ "$GOT_RC" -eq 0 ] && [ "$GOT" = "$val" ] && break
    settle 1
  done
  obs received "$GOT"; obs attempts "$i"
  if [ "$GOT_RC" -ne 0 ]; then blk "$n" inbound persistence "$set/** -> $key: our core's REST surface did not answer" "$GOT_ERR"
  elif [ "$GOT" = "$val" ]; then ok "$n" inbound persistence "$THEM's second sample on $set/** persists on our core with only the core's own store subscribed"
  elif [ -z "$GOT" ]; then blk "$n" inbound persistence "$THEM's second sample on $set/** has not arrived" "Re-run once they have run this session."
  else no "$n" inbound persistence "$THEM's second sample on $set/** arrived with other bytes" "read '$GOT'"; fi
  return 0
}

# ---- THE VERDICT, from the evidence. acceptance_evidence.py decides it and the exit
#      status, so the rule "a check that did not run is not a pass" lives in one place.
set_or_not() { if [ -n "$1" ]; then echo 1; else echo 0; fi; }
finish() { # DIR
  if [ -n "$DERIVED" ]; then
    python3 "$HERE/acceptance_session.py" run-manifest --derived "$SDIR/derived.json" --us "$US" \
      --our-challenge "$MY_CHAL" --their-challenge "$THEIR_CHAL" --started "$RUN_STARTED" --out "$SDIR/run.json" \
      || { echo "  ACCEPTANCE FAILED — this run's session could not be written to $SDIR."; exit 2; }
    FED_ACCEPTANCE_SESSION="$SDIR/run.json"
  fi
  python3 "$EVIDENCE" finalize "$CHECKS" --out "$1" --agreement "$AGREEMENT" --us "$US" --them "$THEM" \
    --run-id "$RUN_ID" --started "$RUN_STARTED" --inputs \
    "theirs_rest=$(set_or_not "$FED_THEIRS_REST")" "ssh_ours=$(set_or_not "$FED_SSH_OURS")" \
    "ssh_theirs=$(set_or_not "$FED_SSH_THEIRS")" "edge_log_ours=$(set_or_not "${FED_EDGE_LOG_OURS:-}")" \
    "probe_agreed=$(set_or_not "${FED_PROBE_THEIRS_REST:-}")" "probe_forged=$(set_or_not "${FED_PROBE_FORGED_REST:-}")" \
    "probe_impersonate=$(set_or_not "${FED_PROBE_IMPERSONATE_REST:-}")" \
    "spire=$(set_or_not "${FED_SPIRE_OURS:-}${FED_SPIRE_THEIRS:-}")" \
    "existing=$(set_or_not "${FED_EXISTING_WORKS:-}")" "existing_rest=$(set_or_not "${FED_EXISTING_REST:-}")" \
    "session=$(set_or_not "${FED_ACCEPTANCE_SESSION:-}")" \
    "ssh_host_keys_verified=$(set_or_not "${FED_SSH_KNOWN_HOSTS:-}")" "probe_launcher=$(set_or_not "${FED_PROBE_LAUNCHER:-}")" \
    ${FED_ACCEPTANCE_SESSION:+--session "$FED_ACCEPTANCE_SESSION"} \
    --software "$HERE/acceptance.sh" "$HERE/acceptance_evidence.py" "$HERE/acceptance_session.py" "$HERE/../roundtrip/fedcheck.sh" \
    --config-dir "$(dirname "$AGREEMENT")/applied"
  rc=$?
  [ "$rc" -eq 2 ] && echo "  ACCEPTANCE FAILED — the evidence bundle could not be written to $1."
  # A check whose evidence could not be recorded is missing from the bundle, so the
  # bundle's verdict cannot stand: the run fails as an evidence failure.
  if [ -n "$RECORD_FAILED" ]; then
    echo "  ACCEPTANCE FAILED — the evidence for$RECORD_FAILED could not be recorded; the bundle is incomplete."
    exit 2
  fi
  [ -n "$DERIVED" ] && [ "$rc" -ne 2 ] && bilateral_round "$1" "$rc"
  exit "$rc"
}

# bilateral_round DIR RC: in a derived session, sign our result, publish it on our
# own core under the session's key, read the partner's from our core, verify it under
# the key bound to them at phase 5, and combine (§6.3). Exits: the combined verdict
# (0 PASS, 1 FAIL, 3 open), or 2 when the partner's result is refused.
bilateral_round() {
  local dir="$1" rc="$2" sk pub_ours pub_theirs key end next=0 left last="" res sig verr open
  open=3; [ "$rc" -eq 1 ] && open=1
  sk="acceptance/$SESSION_ID"
  pub_ours=$(list "$OUR_SETS" | head -n 1); pub_theirs=$(list "$THEIR_SETS" | head -n 1)
  if [ -z "$THEIR_CHAL" ] || [ -z "$pub_ours" ] || [ -z "$pub_theirs" ]; then
    echo "  BILATERAL OPEN — nothing of $THEM's arrived in this run, so there is no challenge of theirs to echo and no signed result is published."
    echo "  Both sides start phase 14 within FED_SESSION_WAIT (${FED_SESSION_WAIT:-300}s) of each other, normally on the same call (X-ACCEPT-001)."
    exit "$open"
  fi
  # shellcheck disable=SC2086  # a command and its fixed words, split on spaces
  $FED_ACCEPTANCE_SIGNER --agreement-sha256 "$AGREEMENT_HEX" --session "$SESSION_ID" --attempt "$FED_ACCEPTANCE_ATTEMPT" \
    --by "$US" --for "$THEM" --echoes "$THEIR_CHAL" --result "$dir/result.json" > "$SDIR/ours.signature.json" \
    || { echo "  ACCEPTANCE FAILED — our result could not be signed (FED_ACCEPTANCE_SIGNER); nothing is published."; exit 2; }
  put_ours "$pub_ours/$sk/$US/result" "$(base64 < "$dir/result.json" | tr -d '\n')"
  put_ours "$pub_ours/$sk/$US/signature" "$(cat "$SDIR/ours.signature.json")"
  echo "  published our signed result under $pub_ours/$sk/$US/result"
  key="$(wire_of "$THEM" "$pub_theirs")/$sk/$THEM"
  end=$((SECONDS + ${FED_SESSION_WAIT:-300}))
  while :; do
    get_side ours "$key/result"; res="$GOT"
    get_side ours "$key/signature"; sig="$GOT"
    if [ -n "$res" ] && [ -n "$sig" ] && [ "$res$sig" != "$last" ]; then
      last="$res$sig"
      printf '%s' "$res" | python3 -c 'import base64, sys; sys.stdout.buffer.write(base64.b64decode(sys.stdin.read()))' \
        > "$SDIR/theirs.result.json" 2>/dev/null || : > "$SDIR/theirs.result.json"
      printf '%s' "$sig" > "$SDIR/theirs.signature.json"
      # shellcheck disable=SC2086
      if $FED_ACCEPTANCE_VERIFIER --agreement-sha256 "$AGREEMENT_HEX" --session "$SESSION_ID" \
          --attempt "$FED_ACCEPTANCE_ATTEMPT" --us "$US" --them "$THEM" --our-challenge "$MY_CHAL" \
          --result "$SDIR/theirs.result.json" --signature "$SDIR/theirs.signature.json" \
          > "$SDIR/theirs.verified.json" 2> "$SDIR/verify.err"; then
        break
      fi
      verr=$(cat "$SDIR/verify.err")
      case "$verr" in
        GB-273*) echo "  $THEM's result on our core is not from this run: $verr. Waiting for this run's." ;;
        *) echo "  ACCEPTANCE REFUSED — $THEM's result does not verify: ${verr:-the verifier gave no reason}"; exit 2 ;;
      esac
    fi
    left=$((end - SECONDS))
    if [ "$left" -le 0 ]; then
      echo "  BILATERAL OPEN — no signed result of this run from $THEM within ${FED_SESSION_WAIT:-300}s (X-ACCEPT-001)."
      exit "$open"
    fi
    if [ "$SECONDS" -ge "$next" ]; then
      echo "  waiting for $THEM's phase 14, ${left}s left"
      next=$((SECONDS + 10))
    fi
    settle 2
  done
  python3 "$HERE/acceptance_session.py" import-signed --manifest "$FED_ACCEPTANCE_SESSION" --us "$US" --evidence "$dir" \
    --verified "$SDIR/theirs.verified.json" "$SDIR/theirs.result.json" || exit 2
  python3 "$HERE/acceptance_session.py" combine --manifest "$FED_ACCEPTANCE_SESSION" --us "$US" --evidence "$dir"
  exit $?
}

# ---- EXISTING PARTNERSHIPS. A deployment for one partner must leave every other
#      partnership as it was. For each peering named in FED_EXISTING_WORKS: its
#      agreement is byte for byte the one recorded before the deployment, a round
#      trip on each set we offer it still comes back acknowledged (it needs only our
#      core and that partner's responder), and, where a drill can read that
#      partner's core, a sample on a set offered to THIS partner only does not reach
#      it. Run with --before ahead of the deployment; the run after compares.
BEFORE_DIR="$EVID/before"
RT_SCRIPT="$HERE/../roundtrip/fedcheck.sh"
RT_WAIT="${FED_ACCEPTANCE_RT_TIMEOUT:-25}"
before_get() { python3 "$EVIDENCE" lookup --bundle "$BEFORE_DIR/result.json" "$@" 2>/dev/null; }
existing_rest() { # ID: the drill's REST surface for that partner's core, or nothing
  printf '%s' "${FED_EXISTING_REST:-}" | tr ',' '\n' | sed -n "s|^$1=||p" | head -1
}
# existing_parties AGREEMENT: "<partner> <our sets> <their sets>", or "- - -" when it
# is not a two-party agreement naming us with well-formed ids and set names (the same
# shapes the run's own agreement must have).
existing_parties() {
  python3 -c '
import json, re, sys
ag = json.load(open(sys.argv[1])); us = sys.argv[2]
parties = [x.get("sovereign_id") for x in ag.get("parties", [])]
ok = all(re.fullmatch(r"[a-z0-9][a-z0-9-]{1,15}", str(x)) for x in parties) and all(
    re.fullmatch(r"release/[A-Za-z0-9._/*-]+", str(r)) for f in ag.get("flows", []) for r in f.get("release", []))
if not ok or us not in parties or len(parties) != 2:
    print("- - -"); sys.exit(0)
p = [x for x in parties if x != us][0]
def sets(frm):
    out = []
    for f in ag.get("flows", []):
        if f.get("from") == frm:
            for r in f.get("release", []):
                stem = r[:-3] if r.endswith("/**") else r
                if stem.startswith("release/") and "*" not in stem:
                    out.append(stem)
    return ",".join(out) or "-"
print(p, sets(us), sets(p))
' "$1" "$US" 2>/dev/null || echo "- - -"
}
existing_checks() {
  local works work ag p osets psets sha bsha set out bv prest key before_why n cset ckey CTL
  [ -n "${FED_EXISTING_WORKS:-}" ] || return 0
  before_why=""
  if [ "$BEFORE" != 1 ]; then
    if [ ! -f "$BEFORE_DIR/result.json" ]; then
      before_why="no snapshot from before the deployment at $BEFORE_DIR (run acceptance.sh <agreement> --before ahead of it)"
    elif [ "$(before_get --agreement-sha)" != "$(python3 "$EVIDENCE" sha256 "$AGREEMENT")" ]; then
      before_why="the snapshot at $BEFORE_DIR was taken for another agreement than this one"
    fi
  fi
  works=$(printf '%s' "$FED_EXISTING_WORKS" | tr ':' '\n')
  for work in $works; do
    ag="$work/agreement.json"
    begin
    if [ ! -f "$ag" ]; then
      blk "existing/$(basename "$work")/agreement" both existing "no agreement at $ag" "FED_EXISTING_WORKS names directories that each hold a peering's agreement.json."
      continue
    fi
    read -r p osets psets <<EOF
$(existing_parties "$ag")
EOF
    if [ "$p" = "-" ] || [ -z "$p" ]; then
      blk "existing/$(basename "$work")/agreement" both existing "$ag is not a well-formed two-party agreement that names $US"
      continue
    fi
    [ "$p" = "$THEM" ] && continue
    sha=$(python3 "$EVIDENCE" sha256 "$ag"); obs agreement_sha256 "$sha"
    n="existing/$p/agreement"
    if [ "$BEFORE" = 1 ]; then
      ok "$n" both existing "$p's agreement recorded before the deployment ($sha)"
    elif [ -n "$before_why" ]; then
      blk "$n" both existing "$p's agreement is unchanged: not proven" "$before_why"
    else
      bsha=$(before_get --name "$n" --obs agreement_sha256)
      if [ -z "$bsha" ]; then blk "$n" both existing "$p's agreement is unchanged: not proven" "the snapshot from before holds no row for $p"
      elif [ "$bsha" = "$sha" ]; then ok "$n" both existing "$p's agreement is the one recorded before the deployment"
      else no "$n" both existing "$p's agreement changed during this deployment" "before $bsha, now $sha"; fi
    fi
    if [ "$osets" = "-" ] || [ "$psets" = "-" ]; then
      begin; na "existing/$p/roundtrip" both existing "no return path with $p: a round trip needs a set each way"
    elif [ ! -f "$RT_SCRIPT" ]; then
      begin; blk "existing/$p/roundtrip" outbound existing "no round-trip check found" "expected $RT_SCRIPT"
    else
      for set in $(list "$osets"); do
        begin; n="existing/$p/roundtrip/$set"
        out=$(ours "BASE=$(printf %q "$FED_OURS_REST") bash -s -- $(printf '%q %q %q' "$set" "from/$p" "$RT_WAIT")" < "$RT_SCRIPT" 2>&1)
        obs fedcheck "$out"
        bv=""; [ "$BEFORE" = 1 ] || bv=$(before_get --name "$n")
        obs before "${bv:-<none>}"
        case "$out" in
          *PASS*) ok "$n" outbound existing "$p still answers a round trip via $set/**${bv:+ (before: $bv)}" ;;
          *) if [ "$bv" = PASS ]; then no "$n" outbound existing "$p answered a round trip via $set/** before this deployment, and does not now" "$out"
             elif [ -n "$bv" ]; then no "$n" outbound existing "$p does not answer a round trip via $set/** (already $bv before the deployment)" "$out"
             else no "$n" outbound existing "$p does not answer a round trip via $set/**" "$out"; fi ;;
        esac
      done
    fi
    # Leakage: only after the deployment, when this partner's lanes exist.
    [ "$BEFORE" = 1 ] && continue
    prest=$(existing_rest "$p")
    for set in $(list "$OUR_SETS"); do
      case ",$osets," in *",$set,"*) continue ;; esac
      begin; n="existing/$p/leak/$set"; key="$(wire_of "$US" "$set")/acceptance/$S/leak-$p"
      if [ -z "$prest" ]; then
        printf '  \033[33mskip\033[0m  %s\n' "$set/** (offered to $THEM only) not read on $p's core: no surface for it"
        record "$n" outbound existing SKIP "$set/** (offered to $THEM only) was not read on $p's core: one side cannot read another sovereign's core; the multi-partner test environment proves it (FED_EXISTING_REST)" --optional
        continue
      fi
      # The control: a sample on a set this partner IS offered, staged beside the
      # probe, must arrive on its core. Without it, absence proves nothing.
      cset=$(list "$osets" | head -1); ckey="$(wire_of "$US" "${cset:-release/-}")/acceptance/$S/leak-control-$p"
      [ -n "$cset" ] && { put_ours "$cset/acceptance/$S/leak-control-$p" "control-$S"; gap; }
      put_ours "$set/acceptance/$S/leak-$p" "leak-$S"
      settle 4
      CTL=""
      [ -n "$cset" ] && CTL=$(curl -s -m 8 -w '\n%{http_code}' "$prest/$ckey" | value)
      GOT=$(curl -s -m 8 -w '\n%{http_code}' "$prest/$key" | value); GOT_RC=$?; GOT_ERR=$(cat "$REST_ERR" 2>/dev/null)
      obs key "$key"; obs received "$GOT"; obs control_key "$ckey"; obs control_received "$CTL"
      if [ "$GOT_RC" -ne 0 ]; then blk "$n" outbound existing "$set/** on $p's core: not proven, its REST surface did not answer" "$GOT_ERR"
      elif [ -z "$GOT" ] && [ "$CTL" != "control-$S" ]; then
        blk "$n" outbound existing "$set/** on $p's core: not proven, the control on ${cset:-a set offered to $p} did not arrive there either" \
          "Absence beside a control that did not land proves nothing."
      elif [ -z "$GOT" ]; then ok "$n" outbound existing "$set/**, offered to $THEM only, did NOT reach $p"
      else no "$n" outbound existing "$set/**, offered to $THEM only, reached $p: cross-partner release leakage" "read '$GOT'"; fi
    done
  done
  return 0
}
if [ "$BEFORE" = 1 ]; then
  [ -n "${FED_EXISTING_WORKS:-}" ] || { echo "acceptance --before: FED_EXISTING_WORKS names no existing peering to record" >&2; exit 2; }
  echo "  existing partnerships, before the deployment (evidence: $BEFORE_DIR)"
  existing_checks
  finish "$BEFORE_DIR"
fi

# ---- RUN-OWNED PROBES (FED_PROBE_LAUNCHER, opt-in). The refusal rows need client
#      sessions across the link holding the partner's agreed identity, a name nobody
#      agreed, and our core's name. A drill used to start them by hand and leave
#      them running. With a launcher, this run starts them itself, each with a
#      deadline, records every one in a resource manifest of its own
#      (<evidence>/probes/<run id>.json), stops exactly those at the end, and checks
#      they are gone. Incomplete cleanup is a required FAIL. A manifest an earlier
#      run left behind (killed before its cleanup) is cleaned first, and only the
#      resources it names. Production edges, cores and tunnels are never touched.
#
#      The launcher contract (the deployment or drill provides the command):
#        <launcher> start <agreed|forged|impersonate> <run id> <deadline, epoch s>
#             prints REST=<url> ZID=<zenoh id> HANDLE=<opaque id>, one per line;
#             the session must end by itself at the deadline
#        <launcher> stop <handle>     ends it
#        <launcher> alive <handle>    exits 0 while it exists, 1 once it is gone
PROBES_DIR="$EVID/probes"
PROBES_FILE="$PROBES_DIR/$RUN_ID.json"
PROBES_STARTED=""; PROBES_DONE=""
# proc_start PID: the process's start time as ps prints it, so a manifest names its
# run by (pid, start) and a pid reused by another process is told apart.
proc_start() { ps -o lstart= -p "$1" 2>/dev/null | tr -s ' ' | sed 's/^ //;s/ $//'; }
PROBES_PID=$$; PROBES_PID_START=$(proc_start $$)
probes_note() { # KIND HANDLE STATUS DEADLINE: record one resource in this run's manifest
  python3 - "$PROBES_FILE" "$RUN_ID" "$1" "$2" "$3" "$4" "$PROBES_PID" "$PROBES_PID_START" <<'PY'
import json, os, sys, time
path, run, kind, handle, status, deadline, pid, pid_start = sys.argv[1:]
m = {"run": run, "pid": int(pid), "pid_start": pid_start, "resources": {}}
if os.path.exists(path):
    m = json.load(open(path))
r = m["resources"].setdefault(handle, {"kind": kind, "deadline": int(deadline or 0), "started_at": int(time.time())})
r["status"] = status
tmp = path + ".tmp"
json.dump(m, open(tmp, "w"), indent=2, sort_keys=True)
os.replace(tmp, path)
PY
}
probes_left() { # MANIFEST: the handles it names that were not confirmed gone; exit 2 when unreadable
  python3 -c 'import json,sys; m=json.load(open(sys.argv[1])); print("\n".join(h for h,r in m.get("resources",{}).items() if r.get("status")!="gone"))' "$1" 2>/dev/null || return 2
}
probes_owner() { # MANIFEST: 0 its run is still going here, 1 it is not, 2 cannot tell
  local pid want now
  pid=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("pid") or "")' "$1" 2>/dev/null)
  want=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("pid_start") or "")' "$1" 2>/dev/null)
  [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null || return 1
  now=$(proc_start "$pid")
  [ -n "$want" ] && [ -n "$now" ] || return 2
  [ "$want" = "$now" ] && return 0
  return 1    # the pid now belongs to another process: the run that wrote it is gone
}
probes_stop() { # MANIFEST: stop and check every resource it names; prints the ones still there
  local h left="" arc
  for h in $(probes_left "$1"); do
    $FED_PROBE_LAUNCHER stop "$h" >/dev/null 2>&1
    # Only exit 1 from `alive` means gone; any other answer (0, or an error) leaves
    # it listed, so a launcher that cannot answer never reads as clean.
    $FED_PROBE_LAUNCHER alive "$h" >/dev/null 2>&1; arc=$?
    if [ "$arc" -eq 1 ]; then PROBES_FILE="$1" probes_note "-" "$h" gone 0
    else left="$left $h"; fi
  done
  printf '%s' "$left"
}
probes_cleanup_on_signal() { # EXIT-STATUS
  [ -n "$PROBES_STARTED" ] && [ -f "$PROBES_FILE" ] && probes_stop "$PROBES_FILE" >/dev/null
  exit "$1"
}
if [ -n "${FED_PROBE_LAUNCHER:-}" ]; then
  mkdir -p "$PROBES_DIR"
  # An earlier run's manifest with resources not confirmed gone: clean them first.
  earlier=""; left=""; unsure=""
  for f in "$PROBES_DIR"/*.json; do
    [ -f "$f" ] || continue
    [ "$f" = "$PROBES_FILE" ] && continue
    pl=$(probes_left "$f"); prc=$?
    if [ "$prc" -ne 0 ]; then
      # A manifest that cannot be read cannot show its resources are gone.
      earlier="$earlier $(basename "$f" .json)"; left="$left <unreadable manifest $(basename "$f")>"; continue
    fi
    [ -n "$pl" ] || continue
    probes_owner "$f"; orc=$?
    if [ "$orc" -eq 0 ]; then echo "  probes of $(basename "$f" .json) belong to a run still going here; left alone"; continue; fi
    if [ "$orc" -eq 2 ]; then unsure="$unsure $(basename "$f" .json)"; continue; fi
    earlier="$earlier $(basename "$f" .json)"
    left="$left$(probes_stop "$f")"
  done
  if [ -n "$earlier" ]; then
    begin; obs runs "$earlier"; obs left "$left"
    if [ -z "$left" ]; then ok cleanup/earlier-runs local precondition "probes an interrupted run left behind are gone:$earlier"
    else no cleanup/earlier-runs local precondition "probes an interrupted run left behind are still there:$left" \
      "Only resources named in those runs' manifests were stopped; stop these by hand."; fi
  fi
  if [ -n "$unsure" ]; then
    begin; obs runs "$unsure"
    blk cleanup/earlier-runs-unknown local precondition "probes of$unsure: the process id that wrote them is in use and its start time cannot be compared, so whether that run is still going is not known" \
      "Left alone. Check those runs by hand and remove their manifests once their probes are gone."
  fi
  trap 'probes_cleanup_on_signal 130' INT
  trap 'probes_cleanup_on_signal 143' TERM
  # Any other way out (an error, a plain exit) also stops this run's probes.
  trap 'rm -f "$CHECKS" "$OBSF" "$REST_ERR" "$TRACE" "$ADM_R" "$ADM_S"; [ -n "$PROBES_STARTED" ] && [ -z "$PROBES_DONE" ] && probes_stop "$PROBES_FILE" >/dev/null' EXIT
  deadline=$(( $(date +%s) + ${FED_PROBE_TTL:-900} ))
  for kind in agreed forged impersonate; do
    case "$kind" in
      agreed)      [ -z "${FED_PROBE_THEIRS_REST:-}" ] || continue ;;
      forged)      [ -z "${FED_PROBE_FORGED_REST:-}" ] || continue ;;
      impersonate) [ -z "${FED_PROBE_IMPERSONATE_REST:-}" ] || continue ;;
    esac
    out=$($FED_PROBE_LAUNCHER start "$kind" "$RUN_ID" "$deadline" 2>&1); lrc=$?
    h=$(printf '%s\n' "$out" | sed -n 's/^HANDLE=//p' | head -1)
    url=$(printf '%s\n' "$out" | sed -n 's/^REST=//p' | head -1)
    zid=$(printf '%s\n' "$out" | sed -n 's/^ZID=//p' | head -1)
    if [ -n "$h" ]; then probes_note "$kind" "$h" started "$deadline"; PROBES_STARTED=1; fi
    if [ "$lrc" -ne 0 ] || [ -z "$h" ] || [ -z "$url" ]; then
      echo "  probe $kind did not start: $(printf '%s' "$out" | tail -1)"
      continue
    fi
    case "$kind" in
      agreed)      FED_PROBE_THEIRS_REST="$url"; FED_PROBE_THEIRS_ZID="${FED_PROBE_THEIRS_ZID:-$zid}" ;;
      forged)      FED_PROBE_FORGED_REST="$url"; FED_PROBE_FORGED_ZID="$zid" ;;
      impersonate) FED_PROBE_IMPERSONATE_REST="$url"; FED_PROBE_IMPERSONATE_ZID="$zid" ;;
    esac
  done
  echo "  probes: this run's own, until $(date -u -d "@$deadline" +%H:%M:%SZ 2>/dev/null || echo "$deadline") (manifest $PROBES_FILE)"
fi
# probes_finish: stop this run's probes and record whether every one is gone.
probes_finish() {
  local left
  [ -n "$PROBES_STARTED" ] || return 0
  begin
  left=$(probes_stop "$PROBES_FILE")
  obs manifest "$PROBES_FILE"; obs left "$left"
  if [ -z "$left" ]; then ok cleanup/probes local precondition "every probe this run started is stopped and gone"
  else no cleanup/probes local precondition "probes this run started are still there:$left" \
    "Incomplete cleanup fails the run. The next run with this launcher stops them first."; fi
  PROBES_DONE=1
  trap - INT TERM
}

begin
if [ "$HAVE_OUT" = 0 ] && [ "$HAVE_IN" = 0 ]; then
  no lanes both precondition "the agreement has no flow in either direction" "There is no lane to prove; the agreement carries nothing."
fi
for set in $(list "$OUR_WILD"); do
  blk "delivery/outbound/$set" outbound positive "$set/** names no single key to publish" "Its lane cannot be proven by a sample; prove it by hand."
done
for set in $(list "$THEIR_WILD"); do
  blk "delivery/inbound/$set" inbound positive "$set/** names no single key to publish" "Its lane cannot be proven by a sample; prove it by hand."
done

# ---- Helpers the persistence and refusal checks share, in both the drill path and a
#      bilateral session. Defined here, called below.
  side_run() { if [ "$1" = ours ]; then ours "$2"; else theirs "$2"; fi; }
  side_get() { # SIDE KEY > curl output with its status line
    if [ "$1" = ours ]; then ours "curl -s -m 8 -w '\n%{http_code}' $(printf %q "$FED_OURS_REST/$2")"
    else theirs "curl -s -m 8 -w '\n%{http_code}' $(printf %q "$FED_THEIRS_REST/$2")"; fi
  }
  # scope SIDE KEY: sets SCOPE (one JSON line) and returns the outcome's status.
  scope() {
    side_get "$1" "@/*/router" > "$ADM_R" 2>/dev/null
    side_get "$1" "@/*/router/subscriber/$2" > "$ADM_S" 2>/dev/null
    SCOPE=$(python3 "$EVIDENCE" persistence-scope --key "$2" --router "$ADM_R" --subscribers "$ADM_S")
  }
  scope_reason() { printf '%s' "$SCOPE" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("reason",""))' 2>/dev/null; }
  # persist DIRECTION RECEIVING-SIDE SENDING-SIDE SENDER SET
  persist() {
    local dir="$1" rx="$2" tx="$3" sender="$4" set="$5" key n there pause resume paused="" val i rc
    n="persistence/$dir/$set"; key="$(wire_of "$sender" "$set")/acceptance/$S/persist"; val="persist-$S-${set#release/}"
    there="our core"; pause="${FED_TEST_SUBSCRIBERS_PAUSE_OURS:-}"; resume="${FED_TEST_SUBSCRIBERS_RESUME_OURS:-}"
    [ "$rx" = theirs ] && { there="their core"; pause="${FED_TEST_SUBSCRIBERS_PAUSE_THEIRS:-}"; resume="${FED_TEST_SUBSCRIBERS_RESUME_THEIRS:-}"; }
    begin
    scope "$rx" "$key"; rc=$?
    if [ "$rc" -eq 5 ] && [ -n "$pause" ]; then
      side_run "$rx" "$pause" >/dev/null 2>&1; paused=1; settle 2
      obs paused "$pause"
      scope "$rx" "$key"; rc=$?
    fi
    obs key "$key"; obs scope "$SCOPE"
    case "$rc" in
      0) ;;
      1) no "$n" "$dir" persistence "no store on $there subscribes to $key" "$(scope_reason)" ;;
      5) blk "$n" "$dir" persistence "$set/** on $there: a session other than the core's own store subscribes to the key, so reception cannot be shown to rest on the store" \
           "$(scope_reason). Stop it, or set FED_TEST_SUBSCRIBERS_PAUSE_$(printf '%s' "$rx" | tr '[:lower:]' '[:upper:]') and its _RESUME_ twin." ;;
      *) blk "$n" "$dir" persistence "$set/** on $there: who subscribes to the key could not be read" "$(scope_reason)" ;;
    esac
    if [ "$rc" -eq 0 ]; then
      if [ "$tx" = ours ]; then put_ours "$set/acceptance/$S/persist" "$val"; else put_theirs "$set/acceptance/$S/persist" "$val"; fi
      for i in 1 2 3 4 5 6 7 8; do
        settle 1
        get_side "$rx" "$key"
        [ "$GOT_RC" -eq 0 ] && [ "$GOT" = "$val" ] && break
      done
      obs sent "$val"; obs received "$GOT"; obs attempts "$i"
      if [ "$GOT_RC" -ne 0 ]; then
        blk "$n" "$dir" persistence "$set/** -> $key: $there's REST surface did not answer" "$GOT_ERR"
      elif [ "$GOT" = "$val" ]; then
        ok "$n" "$dir" persistence "$set/** persists on $there with only the core's own store subscribed to the key"
      else
        no "$n" "$dir" persistence "$set/** did not persist on $there with only its store subscribed" \
          "expected '$val', read '${GOT:-<nothing>}'. The store's own interest is not carrying this lane."
      fi
    fi
    [ -n "$paused" ] && [ -n "$resume" ] && side_run "$rx" "$resume" >/dev/null 2>&1
    return 0
  }

# edge_refusals: the refusal rows read from OUR edge's own record (below, tier 3).
edge_refusals() {
    # THE ROUTER'S OWN RECORD. The checks above read silence, and silence is what
    # a dead path produces too. A client holding the partner's agreed
    # identity writes one key inside its lane and one outside, on the same session,
    # so the allow is the control for the denial. Read from OUR edge, the receiving
    # side, and never from the sender, who gets HTTP 200 for both.
    if [ "$HAVE_IN" = 0 ]; then
      # No inbound lane: our edge grants their identity nothing, so there is no
      # allowed write to set a denial beside. Their edge holds the lanes; the
      # refusal on a granted lane is proven by their run.
      for n in refusal/control-allowed refusal/out-of-lane-denied refusal/spoof-denied; do
        na "$n" inbound negative "$NO_IN: no allowed control exists on our edge"
      done
    fi
    if [ -z "${FED_EDGE_LOG_OURS:-}" ] || [ -z "${FED_PROBE_THEIRS_REST:-}" ]; then
      blk refusal/acl-trace inbound negative "the ACL trace was not read" \
          "Set FED_EDGE_LOG_OURS and FED_PROBE_THEIRS_REST. Without them the refusals
        above rest on silence."
    else
      # One read of the log per question, kept for the lines each check quotes. The
      # whole log is read: the control and the denials are found by keys that carry
      # this run's id, so no earlier run's line can match them; and the edge names a
      # peer that matches no subject once, when its session connects, which may be
      # before this run began. That line is tied to the probe's own session id.
      trace()   { if [ -n "$EDGE_LOG_VIA" ]; then side_ssh "$EDGE_LOG_VIA" "$FED_EDGE_LOG_OURS" > "$TRACE" 2>&1
                  else ours "$FED_EDGE_LOG_OURS" > "$TRACE" 2>&1; fi; }
      lines()   { grep -F "$1" "$TRACE" | grep -E 'is (un)?authorized to ' | tail -3; }
      # verdict [SESSION-FILTER] KEY-FILTER: the last decision, of one session when given.
      verdict() {
        if [ $# -eq 2 ]; then grep -F "$2" "$TRACE" | grep -F "$1" | grep -oE 'is (un)?authorized' | tail -1
        else lines "$1" | grep -oE 'is (un)?authorized' | tail -1; fi
      }
      # session_of KEY-FILTER: the zenoh id zenoh 1.9 prints first on a decision line,
      # `<zid> on Interface(..)+CertCommonName(..)+..+<zid> is authorized to ...`.
      session_of() { lines "$1" | grep -E 'is authorized to ' | tail -1 \
                     | sed -nE 's/^(.*[^0-9a-f])?([0-9a-f]{4,32}) on .* is authorized to .*/\2/p'; }
      probe()   { curl -s -m 8 -X PUT -d "$3" "$1/$2" -o /dev/null; }
      begin
      IN="$THEIR_WIRE/acceptance-probe/$S"; OUT="ops/fabric/acceptance-probe-$S"; SPOOF="$OUR_WIRE/acceptance-spoof/$S"
      if [ "$HAVE_IN" = 1 ]; then
        probe "$FED_PROBE_THEIRS_REST" "$IN" "probe-$S"
        probe "$FED_PROBE_THEIRS_REST" "$OUT" "nope-$S"
        probe "$FED_PROBE_THEIRS_REST" "$SPOOF" "nope-$S"
        settle 6
        trace
        v_in=$(verdict "Put (ingress) on $IN")
        # The session the edge names on the allow line. The denials below count only
        # when the edge records them for this same session: an allow from one session
        # beside a deny from another proves nothing about either.
        CZID=$(session_of "Put (ingress) on $IN")
        get_side ours "$IN"
        obs key "$IN"; obs trace "$(lines "Put (ingress) on $IN")"; obs received "$GOT"; obs session "$CZID"
        if [ "$v_in" = "is authorized" ] && [ "$GOT_RC" -eq 0 ] && [ "$GOT" = "probe-$S" ] \
           && [ -n "${FED_PROBE_THEIRS_ZID:-}" ] && [ -n "$CZID" ] && [ "$CZID" != "$FED_PROBE_THEIRS_ZID" ]; then
          no refusal/control-allowed inbound negative "the allow on our edge came from another session" \
            "the trace names session '${CZID:-<none>}', the probe is '$FED_PROBE_THEIRS_ZID'. The denials below would prove nothing."
          CONTROL=0
        elif [ "$v_in" = "is authorized" ] && [ "$GOT_RC" -eq 0 ] && [ "$GOT" = "probe-$S" ]; then
          ok refusal/control-allowed inbound negative "the agreed identity's write in its lane: ALLOWED in our edge's trace for session ${CZID:-<unnamed>}, and it reads back on our core"
          CONTROL=1
        else
          no refusal/control-allowed inbound negative "the control did not land" "trace verdict '${v_in:-<none>}', our core read '${GOT:-<nothing>}'. The denials below would prove nothing."
          CONTROL=0
      fi
      # deny NAME KEY MESSAGE: a denial counts only beside a control that landed, and
      # only when the edge recorded it for the control's own session.
      deny() {
        local v vs
        v=$(verdict "Put (ingress) on $2"); vs=$(verdict "$CZID on " "Put (ingress) on $2")
        obs key "$2"; obs trace "$(lines "Put (ingress) on $2")"; obs session "$CZID"
        if [ "$CONTROL" != 1 ]; then skp "$1" inbound negative "$3: not asserted" "the allowed control on the same session did not land"
        elif [ -z "$CZID" ]; then
          blk "$1" inbound negative "$3: not tied to the control's session" "the edge's allow line names no session id, so a denial cannot be correlated with it"
        elif [ "$vs" = "is unauthorized" ]; then ok "$1" inbound negative "$3: DENIED in our edge's trace for the control's session $CZID"
        elif [ "$v" = "is unauthorized" ]; then
          no "$1" inbound negative "the denial of $2 was recorded for another session, not the control's ($CZID)" "trace verdict for $CZID: '${vs:-<none>}'"
        else no "$1" inbound negative "no denial recorded for $2" "trace verdict '${v:-<none>}'"; fi
      }
      deny refusal/out-of-lane-denied "$OUT" "the same identity's write to ops/**"
      deny refusal/spoof-denied "$SPOOF" "the same identity writing under OUR wire name"
      fi

      # A name their CA can mint and the agreement never named: admitted to the
      # transport, because TLS cannot see a name policy, and authorized for nothing.
      # The router names such a peer by its zenoh id, once, at connect.
      if [ -n "${FED_PROBE_FORGED_REST:-}" ] && [ -n "${FED_PROBE_FORGED_ZID:-}" ]; then
        FORGED="$THEIR_WIRE/acceptance-forged/$S"; begin
        probe "$FED_PROBE_FORGED_REST" "$FORGED" "nope-$S"
        settle 6
        trace
        unmatched=$(grep -F "$FED_PROBE_FORGED_ZID" "$TRACE" | grep -c 'did not match any configured ACL subject' || true)
        allows=$(grep -F "$FED_PROBE_FORGED_ZID" "$TRACE" | grep -c 'is authorized to ' || true)
        get_side ours "$FORGED"
        obs zid "$FED_PROBE_FORGED_ZID"; obs unmatched "$unmatched"; obs authorizations "$allows"; obs received "$GOT"
        obs trace "$(grep -F "$FED_PROBE_FORGED_ZID" "$TRACE" | tail -3)"
        if [ "${unmatched:-0}" -gt 0 ] && [ "${allows:-0}" -eq 0 ] && [ "$GOT_RC" -eq 0 ] && [ -z "$GOT" ]; then
          ok refusal/forged-name inbound negative "a name their CA minted and nobody agreed: our edge matched it to no subject, ZERO authorizations, nothing landed"
        elif [ "${allows:-0}" -eq 0 ] && [ "$GOT_RC" -ne 0 ]; then
          blk refusal/forged-name inbound negative "the forged name: our core's REST surface did not answer" "$GOT_ERR"
        elif [ "${allows:-0}" -eq 0 ] && [ "${unmatched:-0}" -eq 0 ]; then
          blk refusal/forged-name inbound negative "the forged name: our edge's log names that session nowhere" \
            "Its decision lines are the evidence; without them the refusal is not proven."
        else
          no refusal/forged-name inbound negative "the forged name was not refused" "matched no subject $unmatched time(s), authorized $allows, our core read '${GOT:-<nothing>}'"
        fi
      else
        skp refusal/forged-name inbound negative "the forged-name refusal was not exercised" "Set FED_PROBE_FORGED_REST and FED_PROBE_FORGED_ZID."
      fi

      # THE F-424 CASE. Their CA mints OUR core's name, and TLS passes it.
      # The edge binds its core subject to the core link's interface, so over the
      # partner link that name matches no subject: not one authorization, for any
      # message on any key, and nothing lands on our core.
      if [ -n "${FED_PROBE_IMPERSONATE_REST:-}" ] && [ -n "${FED_PROBE_IMPERSONATE_ZID:-}" ]; then
        begin; I_OUT="ops/fabric/acceptance-impersonate-$S"; I_OWN="$OUR_WIRE/acceptance-impersonate/$S"; I_IN="$THEIR_WIRE/acceptance-impersonate/$S"
        probe "$FED_PROBE_IMPERSONATE_REST" "$I_OUT" "nope-$S"
        probe "$FED_PROBE_IMPERSONATE_REST" "$I_OWN" "nope-$S"
        probe "$FED_PROBE_IMPERSONATE_REST" "$I_IN" "nope-$S"
        settle 6
        trace
        unmatched=$(grep -F "$FED_PROBE_IMPERSONATE_ZID" "$TRACE" | grep -c 'did not match any configured ACL subject' || true)
        allows=$(grep -F "$FED_PROBE_IMPERSONATE_ZID" "$TRACE" | grep -c 'is authorized to ' || true)
        landed=""; unread=""
        for k in "$I_OWN" "$I_OUT" "$I_IN"; do
          get_side ours "$k"; landed="$landed$GOT"; [ "$GOT_RC" -eq 0 ] || unread="$unread $k"
        done
        obs zid "$FED_PROBE_IMPERSONATE_ZID"; obs unmatched "$unmatched"; obs authorizations "$allows"; obs received "$landed"
        obs trace "$(grep -F "$FED_PROBE_IMPERSONATE_ZID" "$TRACE" | tail -3)"
        if [ "${unmatched:-0}" -gt 0 ] && [ "${allows:-0}" -eq 0 ] && [ -z "$landed" ] && [ -z "$unread" ]; then
          ok refusal/impersonation inbound negative "their certificate bearing our core's name: our edge matched it to no subject, ZERO authorizations on any key, nothing landed"
        elif [ "${allows:-0}" -eq 0 ] && [ -z "$landed" ] && [ -n "$unread" ]; then
          blk refusal/impersonation inbound negative "the impersonation: our core's REST surface did not answer for$unread"
        elif [ "${allows:-0}" -eq 0 ] && [ "${unmatched:-0}" -eq 0 ] && [ -z "$landed" ]; then
          blk refusal/impersonation inbound negative "the impersonation: our edge's log names that session nowhere" \
            "Its decision lines are the evidence; without them the refusal is not proven."
        else
          no refusal/impersonation inbound negative "their certificate bearing our core's name was authorized on our edge" "matched no subject $unmatched time(s), authorized $allows time(s), our core read '$landed'"
        fi
      else
        skp refusal/impersonation inbound negative "the F-424 impersonation was not exercised" "Set FED_PROBE_IMPERSONATE_REST and FED_PROBE_IMPERSONATE_ZID."
      fi
    fi
}

if [ -z "$FED_THEIRS_REST" ]; then
  why="no counterparty surface (FED_THEIRS_REST)"
  hint="Only a drill owns both sides; in production the counterparty runs their own half."
  if [ -n "$SESSION" ]; then
    session_lanes
  else
    for set in $(list "$OUR_SETS");   do blk "delivery/outbound/$set" outbound positive "our $set/** read back on their core: $why" "$hint"; done
    for set in $(list "$THEIR_SETS"); do blk "delivery/inbound/$set" inbound positive "their $set/** read back on our core: $why" "$hint"; done
    [ "$HAVE_OUT" = 1 ] && blk republisher-boundary/outbound outbound negative "an unoffered set of ours is not lifted: $why"
    [ "$HAVE_IN" = 1 ]  && blk republisher-boundary/inbound inbound negative "an unoffered set of theirs is not lifted: $why"
  fi
  begin
  if [ -z "$SESSION" ]; then
  blk out-of-lane/inbound inbound negative "their write outside the agreement does not cross: $why"
  blk out-of-lane/outbound outbound negative "a prefix the agreement never named does not reach them: $why"
  blk refusal/acl-trace inbound negative "the refusals on our edge's ACL trace: $why"
  for set in $(list "$OUR_SETS");   do blk "persistence/outbound/$set" outbound persistence "our $set/** persists in their core's store: $why"; done
  for set in $(list "$THEIR_SETS"); do blk "persistence/inbound/$set" inbound persistence "their $set/** persists in our core's store: $why"; done
  fi
else
  # ---- TIER 2, POSITIVE. Read back from the RECEIVING side's own storage: an
  #      ACL allow proves the write was permitted, only a read-back proves it
  #      was delivered. Each sample is STAGED under a release set and must arrive
  #      under the wire name, so each row also proves that side's republisher.
  #      The read is a query against the receiving core's store: the runner declares
  #      no subscriber, so no diagnostic interest can carry a sample the store would
  #      not have taken.
  begin
  [ "$HAVE_OUT" = 1 ] || na delivery/outbound outbound positive "$NO_OUT"
  [ "$HAVE_IN" = 1 ]  || na delivery/inbound inbound positive "$NO_IN"
  # Before sending: each target key must be readable and empty, so the value read
  # afterwards can only have come from this run.
  PRE_BAD=""
  for set in $(list "$THEIR_SETS"); do
    get_side ours "$(wire_of "$THEM" "$set")/acceptance/$S"
    { [ "$GOT_RC" -eq 0 ] && [ -z "$GOT" ]; } || PRE_BAD="$PRE_BAD inbound:$set"
  done
  for set in $(list "$OUR_SETS"); do
    get_side theirs "$(wire_of "$US" "$set")/acceptance/$S"
    { [ "$GOT_RC" -eq 0 ] && [ -z "$GOT" ]; } || PRE_BAD="$PRE_BAD outbound:$set"
  done
  for set in $(list "$THEIR_SETS"); do put_theirs "$set/acceptance/$S" "in-$S-${set#release/}"; gap; done
  for set in $(list "$OUR_SETS");   do put_ours   "$set/acceptance/$S" "out-$S-${set#release/}"; gap; done
  # deliver DIRECTION SIDE SENDER SET PAYLOAD: bounded retries, then the verdict.
  deliver() {
    local w n from there i
    w="$(wire_of "$3" "$4")/acceptance/$S"; n="delivery/$1/$4"
    from="their"; there="our core"
    [ "$1" = outbound ] && { from="our"; there="their core"; }
    for i in 1 2 3 4 5 6 7 8; do
      get_side "$2" "$w"
      [ "$GOT_RC" -eq 0 ] && [ "$GOT" = "$5" ] && break
      settle 1
    done
    obs key "$w"; obs sent "$5"; obs received "$GOT"; obs attempts "$i"
    obs sent_sha256 "$(sha "$5")"; obs received_sha256 "$(sha "$GOT")"
    if [ "$GOT_RC" -ne 0 ]; then
      blk "$n" "$1" positive "$4/** -> $w: $there's REST surface did not answer" "$GOT_ERR"
    elif [ "$GOT" = "$5" ]; then
      ok "$n" "$1" positive "$from $4/** staged arrived as $(wire_of "$3" "$4")/** and reads back byte for byte on $there"
    elif [ -z "$GOT" ]; then
      no "$n" "$1" positive "not delivered $1: $4/** -> $w" "expected '$5', read <nothing>"
    else
      no "$n" "$1" positive "delivered $1 with other bytes: $4/** -> $w" "sent sha256 $(sha "$5"), read sha256 $(sha "$GOT")"
    fi
  }
  settle 2
  for set in $(list "$THEIR_SETS"); do begin; deliver inbound ours "$THEM" "$set" "in-$S-${set#release/}"; done
  for set in $(list "$OUR_SETS");   do begin; deliver outbound theirs "$US" "$set" "out-$S-${set#release/}"; done
  if [ -n "$PRE_BAD" ]; then
    begin; obs targets "$PRE_BAD"
    no fresh-keys both precondition "a target key was unreadable or already held a value before this run sent anything:$PRE_BAD" \
      "The run id is unique, so a value there is not from this run; the reads above prove nothing for those sets."
  fi

  # ---- PERSISTENCE. The reads above are store queries, and the runner declares no
  #      subscriber. That is not enough: a session someone else left subscribed on
  #      the receiving core (a drill's responder, a person's `z_sub`) creates the
  #      interest that pulls a sample across, and a store whose own subscription the
  #      edge refuses then looks healthy for as long as that session lives. So, per
  #      offered set: read who on the receiving core subscribes to the exact key
  #      (the admin space, `@/*/router/subscriber/<key>`; no storage status query),
  #      require that the core's own store does and that no other session does,
  #      stopping a drill's test subscribers when it says how, and only then publish
  #      a NEW sample and read it back from the store.
  # The refusals below are gated on the delivery rows alone: a persistence row that
  # is BLOCKED (a closed admin space, a test subscriber left running) says nothing
  # about whether the lanes carry, so it must not take the refusal rows with it.
  T2_FAIL=$fail; T2_BLOCKED=$blocked
  [ "$HAVE_OUT" = 1 ] || { begin; na persistence/outbound outbound persistence "$NO_OUT"; }
  [ "$HAVE_IN" = 1 ]  || { begin; na persistence/inbound inbound persistence "$NO_IN"; }
  for set in $(list "$THEIR_SETS"); do persist inbound ours theirs "$THEM" "$set"; gap; done
  for set in $(list "$OUR_SETS");   do persist outbound theirs ours "$US" "$set"; gap; done

  # ---- TIER 3, NEGATIVE. Only meaningful because tier 2 just passed.
  if [ "$T2_FAIL" -eq 0 ] && [ "$T2_BLOCKED" -eq 0 ]; then
    begin
    # The republisher boundary, in each direction the agreement has: a set the
    # agreement never offered, staged beside the offered ones that just crossed, is
    # not lifted onto the wire name.
    [ "$HAVE_OUT" = 1 ] && { put_ours   "release/$US-not-offered/acceptance-$S" "nope-$S"; gap; }
    [ "$HAVE_IN" = 1 ]  && { put_theirs "release/$THEM-not-offered/acceptance-$S" "nope-$S"; gap; }
    put_theirs "ops/fabric/acceptance-$S" "nope-$S"; gap
    put_ours   "from/$US-not-granted/acceptance-$S" "nope-$S"
    settle 8
    if [ "$HAVE_OUT" = 1 ]; then
      absent republisher-boundary/outbound outbound negative theirs "$(wire_of "$US" "release/$US-not-offered")/acceptance-$S" \
        "a staging set of ours the agreement never offered was NOT lifted onto our wire name" \
        "release/$US-not-offered/** crossed as $(wire_of "$US" "release/$US-not-offered")/** — our republisher forwarded a set nobody agreed"
    else
      na republisher-boundary/outbound outbound negative "$NO_OUT"
    fi
    if [ "$HAVE_IN" = 1 ]; then
      absent republisher-boundary/inbound inbound negative ours "$(wire_of "$THEM" "release/$THEM-not-offered")/acceptance-$S" \
        "a staging set of theirs the agreement never offered did NOT arrive on our core" \
        "release/$THEM-not-offered/** arrived as $(wire_of "$THEM" "release/$THEM-not-offered")/** — their republisher forwarded a set nobody agreed"
    else
      na republisher-boundary/inbound inbound negative "$NO_IN"
    fi
    absent out-of-lane/inbound inbound negative ours "ops/fabric/acceptance-$S" \
      "their write outside the agreement did NOT cross" "a write to ops/** crossed the edge — the boundary is not holding"
    absent out-of-lane/outbound outbound negative theirs "from/$US-not-granted/acceptance-$S" \
      "a prefix the agreement never named did NOT reach them" "an ungranted prefix crossed outbound"

    edge_refusals
  else
    begin
    skp refusals both negative "refusal checks not run" \
        "A direction is broken or unproven, so a refusal here would pass vacuously and
        prove nothing. Fix the delivery result above and re-run."
  fi
fi

# ---- TIER 4, ROUND TRIP. The only check the sending side can make alone, and
#      the only one that fails when an earlier phase silently half-works. It needs
#      a return path: the probe goes out on one side's set, the ack comes back on
#      the other's. fedcheck.sh is streamed to the host that runs it, never copied.
RT="$HERE/../roundtrip/fedcheck.sh"
RT_WAIT="${FED_ACCEPTANCE_RT_TIMEOUT:-25}"
# roundtrip DIRECTION SIDE SENDER-SETS FAR-WIRE SENDER-REST
roundtrip() {
  local dir="$1" side="$2" sets="$3" far="$4" rest="$5" set out
  begin
  if [ ! -f "$RT" ]; then
    blk "roundtrip/$dir" "$dir" round-trip "no round-trip check found" "expected $RT"; return
  fi
  for set in $(list "$sets"); do
    begin
    if [ "$side" = ours ]; then
      out=$(ours "BASE=$(printf %q "$rest") bash -s -- $(printf '%q %q %q' "$set" "$far" "$RT_WAIT")" < "$RT" 2>&1)
    else
      out=$(theirs "BASE=$(printf %q "$rest") bash -s -- $(printf '%q %q %q' "$set" "$far" "$RT_WAIT")" < "$RT" 2>&1)
    fi
    obs fedcheck "$out"
    case "$out" in
      *PASS*) ok "roundtrip/$dir/$set" "$dir" round-trip "round trip $dir via $set/**: the nonce crossed and came back acknowledged by the far side$(printf '%s' "$out" | grep -oE ' in [0-9]+ms')" ;;
      *)      no "roundtrip/$dir/$set" "$dir" round-trip "the round trip $dir via $set/** did not complete" "$out" ;;
    esac
  done
}
if [ "$HAVE_OUT" = 0 ]; then
  begin; na roundtrip/outbound outbound round-trip "$NO_OUT"
elif [ "$HAVE_IN" = 0 ]; then
  begin; na roundtrip/outbound outbound round-trip "no return path: $NO_IN, so no acknowledgement can come back"
else
  roundtrip outbound ours "$OUR_SETS" "$THEIR_WIRE" "$FED_OURS_REST"
fi
if [ "$HAVE_IN" = 0 ]; then
  begin; na roundtrip/inbound inbound round-trip "$NO_IN"
elif [ "$HAVE_OUT" = 0 ]; then
  begin; na roundtrip/inbound inbound round-trip "no return path: $NO_OUT, so no acknowledgement can come back"
elif [ -z "$FED_THEIRS_REST" ]; then
  if [ -n "$SESSION" ]; then
    # Their nonce, acknowledged by our responder, is their run's outbound round trip.
    for set in $(list "$THEIR_SETS"); do
      begin; obs observed_by_peer "roundtrip/outbound/$set"
      blk "roundtrip/inbound/$set" inbound round-trip "their nonce via $set/**, acknowledged by our responder: observed by $THEM's runner in this session"
    done
  else
    begin; blk roundtrip/inbound inbound round-trip "their nonce, acknowledged by our responder: no counterparty surface (FED_THEIRS_REST)"
  fi
else
  roundtrip inbound theirs "$THEIR_SETS" "$OUR_WIRE" "$FED_THEIRS_REST"
fi

# ---- ADR 1157 STEP 2. Recognition is proved from the side doing the recognising: a
#      workload their server issued chains to the bundle OUR server holds for them,
#      and the same SVID does NOT chain to our own, which is what keeps it a check.
begin
if [ -z "${FED_SPIRE_OURS:-}" ] || [ -z "${FED_SPIRE_THEIRS:-}" ]; then
  skp spire local spire "SPIRE federation not checked" "Set FED_SPIRE_OURS and FED_SPIRE_THEIRS."
else
  # A derived SPIRE command runs on the server's host over ssh; one set by hand, here.
  spire_ours() { if [ -n "$SPIRE_VIA" ]; then local IFS='
'
      # shellcheck disable=SC2046
      timeout 60 ssh $(ssh_opts) "$SPIRE_VIA" "$FED_SPIRE_OURS $1"
    else eval "$FED_SPIRE_OURS $1"; fi; }
  refreshed=""; out=""
  for _ in $(seq 1 18); do
    if out=$(spire_ours "federation refresh -id $(printf %q "$THEIR_TD")" 2>&1); then refreshed=1; break; fi
    sleep 5
  done
  obs refresh "$out"
  if [ -n "$refreshed" ]; then ok spire/refresh inbound spire "our SPIRE server refreshed $THEIR_TD from its bundle endpoint across the link"
  else no spire/refresh inbound spire "our SPIRE server could not refresh $THEIR_TD" "$out"; fi
  tmp="$(mktemp -d)"
  spire_ours "bundle list -id $(printf %q "$THEIR_TD") -format pem" > "$tmp/held.pem" 2>/dev/null
  eval "$FED_SPIRE_THEIRS bundle show -format pem" > "$tmp/theirs.pem" 2>/dev/null
  spire_ours "bundle show -format pem" > "$tmp/ours.pem" 2>/dev/null
  fp() { openssl x509 -in "$1" -noout -fingerprint -sha256 2>/dev/null; }
  begin; obs held "$(fp "$tmp/held.pem")"; obs theirs "$(fp "$tmp/theirs.pem")"
  if [ -s "$tmp/held.pem" ] && [ "$(fp "$tmp/held.pem")" = "$(fp "$tmp/theirs.pem")" ]; then
    ok spire/bundle-held inbound spire "the bundle we hold for $THEIR_TD is the bundle their server publishes"
  else
    no spire/bundle-held inbound spire "the bundle we hold for $THEIR_TD is not theirs"
  fi
  # `x509 mint` prints the SVID first; the first certificate block is the leaf.
  eval "$FED_SPIRE_THEIRS x509 mint -spiffeID $(printf %q "$THEIR_TD/federation-acceptance")" 2>/dev/null \
    | awk '/-----BEGIN CERTIFICATE-----/{f=1} f{print} /-----END CERTIFICATE-----/{if(f)exit}' > "$tmp/svid.pem"
  begin
  if [ ! -s "$tmp/svid.pem" ]; then
    no spire/svid-chains inbound spire "their server minted no workload SVID to validate"
  elif ! openssl verify -CAfile "$tmp/held.pem" "$tmp/svid.pem" >/dev/null 2>&1; then
    no spire/svid-chains inbound spire "their workload does NOT validate against the bundle we hold"
  elif openssl verify -CAfile "$tmp/ours.pem" "$tmp/svid.pem" >/dev/null 2>&1; then
    no spire/svid-chains inbound spire "their workload validates against OUR OWN bundle: the two domains share a root"
  else
    ok spire/svid-chains inbound spire "their workload $THEIR_TD/federation-acceptance validates against the bundle we hold, and not against our own"
  fi
  rm -rf "$tmp"
fi

# ---- EXISTING PARTNERSHIPS, after the deployment (defined above; nothing unless named).
existing_checks

# ---- THIS RUN'S PROBES, stopped and checked gone (nothing unless a launcher started any).
probes_finish

finish "$EVID"
