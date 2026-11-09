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

AGREEMENT="${1:?usage: acceptance.sh <agreement.json> [--as <sovereign>]}"; shift || true
AS=""
while [ $# -gt 0 ]; do
  case "$1" in --as) AS="$2"; shift 2 ;; *) shift ;; esac
done
[ -f "$AGREEMENT" ] || { echo "no agreement at $AGREEMENT" >&2; exit 2; }

: "${FED_OURS_REST:=http://127.0.0.1:8000}"     # our fabric's REST surface
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

ours()   { if [ -n "$FED_SSH_OURS" ]; then timeout 60 ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null "$FED_SSH_OURS" "$1" 2>/dev/null; else eval "$1"; fi; }
theirs() { if [ -n "$FED_SSH_THEIRS" ]; then timeout 60 ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null "$FED_SSH_THEIRS" "$1" 2>/dev/null; else eval "$1"; fi; }

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

if [ -z "$FED_THEIRS_REST" ]; then
  why="no counterparty surface (FED_THEIRS_REST)"
  hint="Only a drill owns both sides; in production the counterparty runs their own half."
  for set in $(list "$OUR_SETS");   do blk "delivery/outbound/$set" outbound positive "our $set/** read back on their core: $why" "$hint"; done
  for set in $(list "$THEIR_SETS"); do blk "delivery/inbound/$set" inbound positive "their $set/** read back on our core: $why" "$hint"; done
  [ "$HAVE_OUT" = 1 ] && blk republisher-boundary/outbound outbound negative "an unoffered set of ours is not lifted: $why"
  [ "$HAVE_IN" = 1 ]  && blk republisher-boundary/inbound inbound negative "an unoffered set of theirs is not lifted: $why"
  blk out-of-lane/inbound inbound negative "their write outside the agreement does not cross: $why"
  blk out-of-lane/outbound outbound negative "a prefix the agreement never named does not reach them: $why"
  blk refusal/acl-trace inbound negative "the refusals on our edge's ACL trace: $why"
  for set in $(list "$OUR_SETS");   do blk "persistence/outbound/$set" outbound persistence "our $set/** persists in their core's store: $why"; done
  for set in $(list "$THEIR_SETS"); do blk "persistence/inbound/$set" inbound persistence "their $set/** persists in our core's store: $why"; done
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
      # One read of the log per question, kept for the lines each check quotes.
      trace()   { ours "$FED_EDGE_LOG_OURS" > "$TRACE" 2>&1; }
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
        else
          no refusal/impersonation inbound negative "their certificate bearing our core's name was authorized on our edge" "matched no subject $unmatched time(s), authorized $allows time(s), our core read '$landed'"
        fi
      else
        skp refusal/impersonation inbound negative "the F-424 impersonation was not exercised" "Set FED_PROBE_IMPERSONATE_REST and FED_PROBE_IMPERSONATE_ZID."
      fi
    fi
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
  begin; blk roundtrip/inbound inbound round-trip "their nonce, acknowledged by our responder: no counterparty surface (FED_THEIRS_REST)"
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
  refreshed=""; out=""
  for _ in $(seq 1 18); do
    if out=$(eval "$FED_SPIRE_OURS federation refresh -id $(printf %q "$THEIR_TD")" 2>&1); then refreshed=1; break; fi
    sleep 5
  done
  obs refresh "$out"
  if [ -n "$refreshed" ]; then ok spire/refresh inbound spire "our SPIRE server refreshed $THEIR_TD from its bundle endpoint across the link"
  else no spire/refresh inbound spire "our SPIRE server could not refresh $THEIR_TD" "$out"; fi
  tmp="$(mktemp -d)"
  eval "$FED_SPIRE_OURS bundle list -id $(printf %q "$THEIR_TD") -format pem" > "$tmp/held.pem" 2>/dev/null
  eval "$FED_SPIRE_THEIRS bundle show -format pem" > "$tmp/theirs.pem" 2>/dev/null
  eval "$FED_SPIRE_OURS bundle show -format pem" > "$tmp/ours.pem" 2>/dev/null
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

# ---- THE VERDICT, from the evidence. acceptance_evidence.py decides it and the exit
#      status, so the rule "a check that did not run is not a pass" lives in one place.
set_or_not() { if [ -n "$1" ]; then echo 1; else echo 0; fi; }
python3 "$EVIDENCE" finalize "$CHECKS" --out "$EVID" --agreement "$AGREEMENT" --us "$US" --them "$THEM" \
  --run-id "$RUN_ID" --started "$RUN_STARTED" --inputs \
  "theirs_rest=$(set_or_not "$FED_THEIRS_REST")" "ssh_ours=$(set_or_not "$FED_SSH_OURS")" \
  "ssh_theirs=$(set_or_not "$FED_SSH_THEIRS")" "edge_log_ours=$(set_or_not "${FED_EDGE_LOG_OURS:-}")" \
  "probe_agreed=$(set_or_not "${FED_PROBE_THEIRS_REST:-}")" "probe_forged=$(set_or_not "${FED_PROBE_FORGED_REST:-}")" \
  "probe_impersonate=$(set_or_not "${FED_PROBE_IMPERSONATE_REST:-}")" \
  "spire=$(set_or_not "${FED_SPIRE_OURS:-}${FED_SPIRE_THEIRS:-}")"
rc=$?
[ "$rc" -eq 2 ] && echo "  ACCEPTANCE FAILED — the evidence bundle could not be written to $EVID."
# A check whose evidence could not be recorded is missing from the bundle, so the
# bundle's verdict cannot stand: the run fails as an evidence failure.
if [ -n "$RECORD_FAILED" ]; then
  echo "  ACCEPTANCE FAILED — the evidence for$RECORD_FAILED could not be recorded; the bundle is incomplete."
  exit 2
fi
exit "$rc"
