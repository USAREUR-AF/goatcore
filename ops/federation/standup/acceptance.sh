#!/usr/bin/env bash
# acceptance.sh — phase 14 of a peering standup. The terminal gate.
#
# FOUR TIERS, and the ordering matters (docs/design/federation-acceptance.md):
#
#   tier 2  POSITIVE   the granted lanes carry, both directions, read back from
#                      the RECEIVING side's own storage
#   tier 3  NEGATIVE   the refusals fire — asserted only ALONGSIDE the positives,
#                      because a refusal checked against a dead path passes
#                      vacuously and proves nothing. Silence proves nothing
#                      either: a refusal counts when the receiving edge's
#                      ACL trace records it, beside an allow for the same identity
#   tier 4  ROUND TRIP the check the sending side can make alone
#
# Tier 1 (per-phase end state) belongs to the phases themselves, not here.
#
# WHAT A DRILL ADDS, because it owns both sides. Each is optional; absent, its checks
# print `skip`, and a skip is not a pass: a run with any skip exits 3, never 0.
#
#   FED_THEIRS_REST          their core's REST surface
#   FED_EDGE_LOG_OURS        command printing OUR edge router's log, ACL trace on
#                            (goat_zenoh_edge_acl_trace)
#   FED_PROBE_THEIRS_REST    REST surface of a client session across the link holding
#                            THEIR agreed peering identity, connected to our edge
#   FED_PROBE_FORGED_REST    the same, holding a certificate from THEIR CA with a
#                            common name the agreement never named
#   FED_PROBE_FORGED_ZID     that session's zenoh id, the only name the router logs for
#                            a peer matching no subject
#   FED_PROBE_IMPERSONATE_REST  the same, holding a certificate from THEIR CA bearing
#                            OUR core's common name
#   FED_PROBE_IMPERSONATE_ZID   that session's zenoh id
#   FED_SPIRE_OURS           command prefix running OUR `spire-server` CLI
#   FED_SPIRE_THEIRS         command prefix running THEIR `spire-server` CLI
#
# OUTCOMES. Each check ends PASS (the expected evidence was observed), FAIL (a
# violation was observed), BLOCKED (a prerequisite or the evidence was unavailable),
# SKIP (not exercised) or NOT APPLICABLE (excluded by the agreed scope).
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
# samples (default 0.5), FED_ACCEPTANCE_RT_TIMEOUT the round-trip wait (default 25).
# A real edge needs the defaults.
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

settle() { sleep "${FED_ACCEPTANCE_WAIT:-$1}"; }
gap()    { sleep "${FED_ACCEPTANCE_GAP:-0.5}"; }
now()    { date -u +%Y-%m-%dT%H:%M:%SZ; }

# --- the evidence ------------------------------------------------------------
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EVIDENCE="$HERE/acceptance_evidence.py"
EVID="${FED_EVIDENCE_DIR:-$(cd "$(dirname "$AGREEMENT")" && pwd)/evidence/acceptance}"
mkdir -p "$EVID" 2>/dev/null && [ -w "$EVID" ] \
  || { echo "acceptance: cannot write the evidence directory $EVID (set FED_EVIDENCE_DIR)" >&2; exit 2; }
RUN_STARTED="$(now)"
RUN_ID="acc-$(date -u +%Y%m%dT%H%M%SZ)-$$"
CHECKS="$EVID/.checks-$RUN_ID.jsonl"; OBSF="$EVID/.obs-$RUN_ID"
: > "$CHECKS"; : > "$OBSF"
trap 'rm -f "$CHECKS" "$OBSF"' EXIT
T0="$RUN_STARTED"
pass=0; fail=0; skip=0; blocked=0; RECORD_FAILED=""

# begin: the start time of the checks that follow. obs KEY VALUE: one raw observation
# for the next check recorded.
begin() { T0="$(now)"; }
obs()   { printf '%s\t%s\n' "$1" "$(printf '%s' "$2" | base64 | tr -d '\n')" >> "$OBSF"; }
# record NAME DIRECTION TIER VERDICT REASON
record() {
  if ! python3 "$EVIDENCE" record "$CHECKS" --name "$1" --direction "$2" --tier "$3" --verdict "$4" \
       --reason "$5" --title "$5" --started "$T0" --obs-file "$OBSF"; then
    printf '  \033[31mFAIL\033[0m  the evidence for %s could not be recorded\n' "$1"
    fail=$((fail+1)); RECORD_FAILED="$RECORD_FAILED $1"
  fi
  : > "$OBSF"
}
# ok|no|skp|blk NAME DIRECTION TIER MESSAGE [DETAIL]
ok()  { printf '  \033[32mpass\033[0m  %s\n' "$4"; pass=$((pass+1)); record "$1" "$2" "$3" PASS "$4"; }
no()  { printf '  \033[31mFAIL\033[0m  %s\n' "$4"; [ -n "${5:-}" ] && printf '        %s\n' "$5"; fail=$((fail+1))
        record "$1" "$2" "$3" FAIL "$4${5:+ — $5}"; }
skp() { printf '  \033[33mskip\033[0m  %s\n' "$4"; [ -n "${5:-}" ] && printf '        %s\n' "$5"; skip=$((skip+1))
        record "$1" "$2" "$3" SKIP "$4${5:+ — $5}"; }
blk() { printf '  \033[33mBLOCKED\033[0m  %s\n' "$4"; [ -n "${5:-}" ] && printf '        %s\n' "$5"; blocked=$((blocked+1))
        record "$1" "$2" "$3" BLOCKED "$4${5:+ — $5}"; }

# --- the lanes, from the agreement -------------------------------------------
read -r US THEM OUR_WIRE THEIR_WIRE THEIR_TD OUR_SETS THEIR_SETS <<EOF
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
# before its tail names no single key to publish, so it is left out here.
def sets(frm):
    out = []
    for f in ag.get("flows", []):
        if f.get("from") != frm:
            continue
        for r in f.get("release", []):
            stem = r[:-3] if r.endswith("/**") else r
            if stem.startswith("release/") and "*" not in stem:
                out.append(stem)
    return ",".join(out) or "-"
print(us, them, f"from/{us}", f"from/{them}", td[them], sets(us), sets(them))
PY
)
EOF
# staging prefix -> the wire key it crosses as: `from/<sender>/` + the staging key
# less its first segment (ops/federation/republisher/README.md, the key mapping).
wire_of() { printf 'from/%s/%s' "$1" "${2#release/}"; }
list() { [ "$1" = "-" ] || printf '%s\n' "$1" | tr ',' '\n'; }
[ -n "$US" ] || { echo "acceptance: could not read well-formed parties and sets from the agreement" >&2; exit 2; }
echo "acceptance — $US <-> $THEM"
echo "  our wire name:   $OUR_WIRE/**"
echo "  their wire name: $THEIR_WIRE/**"
echo "  our offered sets:   $OUR_SETS"
echo "  their offered sets: $THEIR_SETS"
echo

ours()   { if [ -n "$FED_SSH_OURS" ]; then timeout 60 ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null "$FED_SSH_OURS" "$1" 2>/dev/null; else eval "$1"; fi; }
theirs() { if [ -n "$FED_SSH_THEIRS" ]; then timeout 60 ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null "$FED_SSH_THEIRS" "$1" 2>/dev/null; else eval "$1"; fi; }

# The REST value comes back base64 whenever it is not JSON, and nothing says so.
# A literal grep finds nothing and reads as "the data never arrived".
decode() { grep -oE '"value":"[^"]*"' | head -1 | sed 's/.*"value":"//;s/"$//' \
         | { read -r b || true; [ -n "${b:-}" ] && printf '%s' "$b" | base64 -d 2>/dev/null || true; }; }
# Each argument is quoted with printf %q before it joins a command string that runs
# here or on the core host.
get_ours()   { ours   "curl -s -m 8 $(printf %q "$FED_OURS_REST/$1")"   | decode; }
get_theirs() { theirs "curl -s -m 8 $(printf %q "$FED_THEIRS_REST/$1")" | decode; }
put_ours()   { ours   "curl -s -m 8 -X PUT -d $(printf %q "$2") $(printf %q "$FED_OURS_REST/$1") -o /dev/null"; }
put_theirs() { theirs "curl -s -m 8 -X PUT -d $(printf %q "$2") $(printf %q "$FED_THEIRS_REST/$1") -o /dev/null"; }

if [ -z "$FED_THEIRS_REST" ]; then
  blk counterparty-surface both precondition "no counterparty surface" \
      "Set FED_THEIRS_REST (and FED_SSH_THEIRS) to assert tiers 2 and 3. Only a
        drill owns both sides; in production the counterparty runs their own half
        and tier 4 below is what you have."
else
  S="$RUN_ID"
  begin

  # ---- TIER 2, POSITIVE. Read back from the RECEIVING side's own storage: an
  #      ACL allow proves the write was permitted, only a read-back proves it
  #      was delivered. Each sample is STAGED under a release set and must arrive
  #      under the wire name, so each row also proves that side's republisher.
  [ "$OUR_SETS" != "-" ] || no lanes/outbound outbound precondition "the agreement offers no release set from us" "Nothing would cross outbound; there is no lane to prove."
  [ "$THEIR_SETS" != "-" ] || no lanes/inbound inbound precondition "the agreement offers no release set from them" "Nothing would cross inbound; there is no lane to prove."
  # Half a second apart. The receiving edge enforces the agreed `hz` as a
  # downsampling interval over the whole wire prefix, so two samples a few
  # milliseconds apart under different sets are one too many for it, and the
  # second is dropped before its access check, with nothing logged.
  for set in $(list "$THEIR_SETS"); do put_theirs "$set/acceptance/$S" "in-$S"; gap; done
  for set in $(list "$OUR_SETS");   do put_ours   "$set/acceptance/$S" "out-$S"; gap; done
  settle 8
  for set in $(list "$THEIR_SETS"); do
    w="$(wire_of "$THEM" "$set")/acceptance/$S"
    got=$(get_ours "$w"); obs key "$w"; obs sent "in-$S"; obs received "$got"
    [ "$got" = "in-$S" ] && ok "delivery/inbound/$set" inbound positive "their $set/** staged there arrived as $(wire_of "$THEM" "$set")/** and reads back on our side" \
                         || no "delivery/inbound/$set" inbound positive "not delivered inbound: $set/** -> $w" "expected 'in-$S', read '${got:-<nothing>}'"
  done
  for set in $(list "$OUR_SETS"); do
    w="$(wire_of "$US" "$set")/acceptance/$S"
    got=$(get_theirs "$w"); obs key "$w"; obs sent "out-$S"; obs received "$got"
    [ "$got" = "out-$S" ] && ok "delivery/outbound/$set" outbound positive "our $set/** staged here arrived as $(wire_of "$US" "$set")/** and reads back on theirs" \
                          || no "delivery/outbound/$set" outbound positive "not delivered outbound: $set/** -> $w" "expected 'out-$S', read '${got:-<nothing>}'"
  done

  # ---- TIER 3, NEGATIVE. Only meaningful because tier 2 just passed.
  if [ "$fail" -eq 0 ]; then
    begin
    put_theirs "ops/fabric/acceptance-$S" "nope-$S"
    put_ours   "from/$US-not-granted/acceptance-$S" "nope-$S"
    put_ours   "release/$US-not-offered/acceptance-$S" "nope-$S"
    settle 8
    got=$(get_theirs "$(wire_of "$US" "release/$US-not-offered")/acceptance-$S"); obs received "$got"
    [ -z "$got" ] && ok republisher-boundary/outbound outbound negative "a staging name the agreement never offered was NOT lifted onto our wire name" \
                  || no republisher-boundary/outbound outbound negative "release/$US-not-offered/** crossed as $(wire_of "$US" "release/$US-not-offered")/** — the republisher forwarded a set nobody agreed"
    got=$(get_ours "ops/fabric/acceptance-$S"); obs received "$got"
    [ -z "$got" ] && ok out-of-lane/inbound inbound negative "their write outside the agreement did NOT cross" \
                  || no out-of-lane/inbound inbound negative "a write to ops/** crossed the edge — the boundary is not holding"
    got=$(get_theirs "from/$US-not-granted/acceptance-$S"); obs received "$got"
    [ -z "$got" ] && ok out-of-lane/outbound outbound negative "a prefix the agreement never named did NOT reach them" \
                  || no out-of-lane/outbound outbound negative "an ungranted prefix crossed outbound"

    # THE ROUTER'S OWN RECORD. The two checks above read silence, and silence is what
    # a dead path produces too. A client holding the partner's agreed
    # identity writes one key inside its lane and one outside, on the same session,
    # so the allow is the control for the denial. Read from OUR edge, the receiving
    # side, and never from the sender, who gets HTTP 200 for both.
    if [ -z "${FED_EDGE_LOG_OURS:-}" ] || [ -z "${FED_PROBE_THEIRS_REST:-}" ]; then
      blk refusal/acl-trace inbound negative "the ACL trace was not read" \
          "Set FED_EDGE_LOG_OURS and FED_PROBE_THEIRS_REST. Without them the refusals
        above rest on silence."
    else
      trace() { ours "$FED_EDGE_LOG_OURS" 2>&1; }
      verdict() { trace | grep -F "$1" | grep -E 'is (un)?authorized to ' | grep -oE 'is (un)?authorized' | tail -1; }
      probe() { curl -s -m 8 -X PUT -d "$3" "$1/$2" -o /dev/null; }
      IN="$THEIR_WIRE/acceptance-probe/$S"; OUT="ops/fabric/acceptance-probe-$S"; SPOOF="$OUR_WIRE/acceptance-spoof/$S"
      begin
      probe "$FED_PROBE_THEIRS_REST" "$IN" "probe-$S"
      probe "$FED_PROBE_THEIRS_REST" "$OUT" "nope-$S"
      probe "$FED_PROBE_THEIRS_REST" "$SPOOF" "nope-$S"
      settle 6
      v_in=$(verdict "Put (ingress) on $IN"); v_out=$(verdict "Put (ingress) on $OUT"); v_spoof=$(verdict "Put (ingress) on $SPOOF")
      got=$(get_ours "$IN")
      obs key "$IN"; obs trace "$(trace | grep -F "Put (ingress) on $IN" | tail -3)"; obs received "$got"
      if [ "$v_in" = "is authorized" ] && [ "$got" = "probe-$S" ]; then
        ok refusal/control-allowed inbound negative "the agreed identity's write in its lane: ALLOWED in our edge's trace, and it reads back on our core"
      else
        no refusal/control-allowed inbound negative "the control did not land" "trace verdict '${v_in:-<none>}', our core read '${got:-<nothing>}'. The denials below would prove nothing."
      fi
      obs key "$OUT"; obs trace "$(trace | grep -F "Put (ingress) on $OUT" | tail -3)"
      [ "$v_out" = "is unauthorized" ] && ok refusal/out-of-lane-denied inbound negative "the same identity's write to ops/**: DENIED in our edge's trace" \
        || no refusal/out-of-lane-denied inbound negative "no denial recorded for $OUT" "trace verdict '${v_out:-<none>}'"
      obs key "$SPOOF"; obs trace "$(trace | grep -F "Put (ingress) on $SPOOF" | tail -3)"
      [ "$v_spoof" = "is unauthorized" ] && ok refusal/spoof-denied inbound negative "the same identity writing under OUR wire name: DENIED in our edge's trace" \
        || no refusal/spoof-denied inbound negative "no denial recorded for $SPOOF" "trace verdict '${v_spoof:-<none>}'"

      # A name their CA can mint and the agreement never named: admitted to the
      # transport, because TLS cannot see a name policy, and authorized for nothing.
      # The router names such a peer by its zenoh id, once, at connect.
      if [ -n "${FED_PROBE_FORGED_REST:-}" ] && [ -n "${FED_PROBE_FORGED_ZID:-}" ]; then
        FORGED="$THEIR_WIRE/acceptance-forged/$S"; begin
        probe "$FED_PROBE_FORGED_REST" "$FORGED" "nope-$S"
        settle 6
        unmatched=$(trace | grep -F "$FED_PROBE_FORGED_ZID" | grep -c 'did not match any configured ACL subject' || true)
        allows=$(trace | grep -F "$FED_PROBE_FORGED_ZID" | grep -c 'is authorized to ' || true)
        got=$(get_ours "$FORGED")
        obs zid "$FED_PROBE_FORGED_ZID"; obs unmatched "$unmatched"; obs authorizations "$allows"; obs received "$got"
        if [ "${unmatched:-0}" -gt 0 ] && [ "${allows:-0}" -eq 0 ] && [ -z "$got" ]; then
          ok refusal/forged-name inbound negative "a name their CA minted and nobody agreed: our edge matched it to no subject, ZERO authorizations, nothing landed"
        else
          no refusal/forged-name inbound negative "the forged name was not refused" "matched no subject $unmatched time(s), authorized $allows, our core read '${got:-<nothing>}'"
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
        unmatched=$(trace | grep -F "$FED_PROBE_IMPERSONATE_ZID" | grep -c 'did not match any configured ACL subject' || true)
        allows=$(trace | grep -F "$FED_PROBE_IMPERSONATE_ZID" | grep -c 'is authorized to ' || true)
        got=$(get_ours "$I_OWN"); got2=$(get_ours "$I_OUT"); got3=$(get_ours "$I_IN")
        obs zid "$FED_PROBE_IMPERSONATE_ZID"; obs unmatched "$unmatched"; obs authorizations "$allows"; obs received "$got$got2$got3"
        if [ "${unmatched:-0}" -gt 0 ] && [ "${allows:-0}" -eq 0 ] && [ -z "$got$got2$got3" ]; then
          ok refusal/impersonation inbound negative "their certificate bearing our core's name: our edge matched it to no subject, ZERO authorizations on any key, nothing landed"
        else
          no refusal/impersonation inbound negative "their certificate bearing our core's name was authorized on our edge" "matched no subject $unmatched time(s), authorized $allows time(s), our core read '${got:-}${got2:-}${got3:-}'"
        fi
      else
        skp refusal/impersonation inbound negative "the F-424 impersonation was not exercised" "Set FED_PROBE_IMPERSONATE_REST and FED_PROBE_IMPERSONATE_ZID."
      fi
    fi
  else
    skp refusals both negative "refusal checks not run" \
        "A direction is broken, so a refusal here would pass vacuously and prove
        nothing. Fix the delivery failure above and re-run."
  fi
fi

# ---- TIER 4, ROUND TRIP. The only check the sending side can make alone, and
#      the only one that fails when an earlier phase silently half-works.
RT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../roundtrip" && pwd)/fedcheck.sh"
begin
if [ ! -x "$RT" ]; then
  blk roundtrip/outbound outbound round-trip "no round-trip check found" "expected $RT"
elif ! ours "test -x /tmp/fedcheck.sh" >/dev/null 2>&1 && [ -n "$FED_SSH_OURS" ]; then
  blk roundtrip/outbound outbound round-trip "round trip not runnable on the publishing host" \
      "copy ops/federation/roundtrip/fedcheck.sh there, and responder.py to the far side"
elif [ "$OUR_SETS" = "-" ]; then
  no roundtrip/outbound outbound round-trip "no round trip possible" "the agreement offers no release set from us, so no nonce can cross"
else
  # One nonce per offered set, staged under it: each pass proves our republisher
  # lifted that set, the link carried it, and THEIR side answered.
  for set in $(list "$OUR_SETS"); do
    begin
    out=$( [ -n "$FED_SSH_OURS" ] && ours "/tmp/fedcheck.sh $(printf '%q %q' "$set" "$THEIR_WIRE") ${FED_ACCEPTANCE_RT_TIMEOUT:-25}" \
                                  || BASE="$FED_OURS_REST" "$RT" "$set" "$THEIR_WIRE" "${FED_ACCEPTANCE_RT_TIMEOUT:-25}" )
    obs fedcheck "$out"
    case "$out" in
      *PASS*) ok "roundtrip/outbound/$set" outbound round-trip "round trip via $set/**: a nonce staged here crossed as $(wire_of "$US" "$set")/** and came back acknowledged by THEIR side$(printf '%s' "$out" | grep -oE ' in [0-9]+ms')" ;;
      *)      no "roundtrip/outbound/$set" outbound round-trip "the round trip via $set/** did not complete" "$out" ;;
    esac
  done
fi

# ---- ADR 1157 STEP 2. Recognition is proved from the side doing the recognising: a
#      workload their server issued chains to the bundle OUR server holds for them,
#      and the same SVID does NOT chain to our own, which is what keeps it a check.
if [ -z "${FED_SPIRE_OURS:-}" ] || [ -z "${FED_SPIRE_THEIRS:-}" ]; then
  skp spire local spire "SPIRE federation not checked" "Set FED_SPIRE_OURS and FED_SPIRE_THEIRS."
else
  begin; refreshed=""
  for _ in $(seq 1 18); do
    if out=$(eval "$FED_SPIRE_OURS federation refresh -id $(printf %q "$THEIR_TD")" 2>&1); then refreshed=1; break; fi
    sleep 5
  done
  obs refresh "$out"
  [ -n "$refreshed" ] && ok spire/refresh inbound spire "our SPIRE server refreshed $THEIR_TD from its bundle endpoint across the link" \
                      || no spire/refresh inbound spire "our SPIRE server could not refresh $THEIR_TD" "$out"
  tmp="$(mktemp -d)"
  eval "$FED_SPIRE_OURS bundle list -id $(printf %q "$THEIR_TD") -format pem" > "$tmp/held.pem" 2>/dev/null
  eval "$FED_SPIRE_THEIRS bundle show -format pem" > "$tmp/theirs.pem" 2>/dev/null
  eval "$FED_SPIRE_OURS bundle show -format pem" > "$tmp/ours.pem" 2>/dev/null
  fp() { openssl x509 -in "$1" -noout -fingerprint -sha256 2>/dev/null; }
  if [ -s "$tmp/held.pem" ] && [ "$(fp "$tmp/held.pem")" = "$(fp "$tmp/theirs.pem")" ]; then
    ok spire/bundle-held inbound spire "the bundle we hold for $THEIR_TD is the bundle their server publishes"
  else
    obs held "$(fp "$tmp/held.pem")"; obs theirs "$(fp "$tmp/theirs.pem")"
    no spire/bundle-held inbound spire "the bundle we hold for $THEIR_TD is not theirs"
  fi
  # `x509 mint` prints the SVID first; the first certificate block is the leaf.
  eval "$FED_SPIRE_THEIRS x509 mint -spiffeID $(printf %q "$THEIR_TD/federation-acceptance")" 2>/dev/null \
    | awk '/-----BEGIN CERTIFICATE-----/{f=1} f{print} /-----END CERTIFICATE-----/{if(f)exit}' > "$tmp/svid.pem"
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
set_or_not() { [ -n "$1" ] && echo 1 || echo 0; }
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
