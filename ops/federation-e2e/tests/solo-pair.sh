#!/usr/bin/env bash
# tests/solo-pair.sh — the solo unit validated lab to lab: two lab sovereigns, each run as
# the solo unit (FE2E_PARTNER=external) in its own lab, each the other's outside partner.
#
#   tests/solo-pair.sh --ref REF --dir DIR [--goat-version VER] [--step-timeout S]
#
# Their exchange folders are crossed (one side's OUT is the other's IN), and this script
# answers each pause the way an operator's laptop would: voice-call.ok after comparing the
# key and CA fingerprint read aloud with the files received, digest.ok with the other
# side's agreement digest, admit.ok, then tunnel.ok, accept.ok and persist.ok once both
# sides have reached that step. Both edges' public sides sit on one docker network created
# here (labelled org.goat.fe2e.lab=fe2e-solo-wan-<pid>, 198.51.100.0/24, removed at the
# end), standing in for the internet: xae dials, xaf listens.
#
# DIR receives xae-out/ and xaf-out/ (the folders), facts-xae.txt and facts-xaf.txt,
# results/ (env run --out) and logs/. Nothing here reaches anything outside this machine.
set -euo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
REF=""; DIR=""; VER="0.5.19"; STEP_TIMEOUT=3600
while [ $# -gt 0 ]; do
  case "$1" in
    --ref) REF="$2"; shift 2 ;;
    --dir) DIR="$2"; shift 2 ;;
    --goat-version) VER="$2"; shift 2 ;;
    --step-timeout) STEP_TIMEOUT="$2"; shift 2 ;;
    *) echo "solo-pair: unknown argument $1" >&2; exit 2 ;;
  esac
done
[ -n "$REF" ] && [ -n "$DIR" ] || { sed -n '2,16p' "$0"; exit 2; }
mkdir -p "$DIR"; DIR="$(cd "$DIR" && pwd)"
rm -rf "$DIR/xae-out" "$DIR/xaf-out"; mkdir -p "$DIR/xae-out" "$DIR/xaf-out" "$DIR/results" "$DIR/logs"

WAN="fe2e-solo-wan-$$"; X=xae; Y=xaf; XIP=198.51.100.11; YIP=198.51.100.12; PORT=51820
docker network create --internal --label "org.goat.fe2e.lab=$WAN" --subnet 198.51.100.0/24 "$WAN" >/dev/null
PIDS=""
cleanup() {
  local p
  for p in $PIDS; do kill "$p" 2>/dev/null || true; done
  for p in $PIDS; do wait "$p" 2>/dev/null || true; done
  docker network rm "$WAN" >/dev/null 2>&1 || true
}
trap cleanup EXIT
trap 'exit 130' INT TERM

# Each side's facts about the other. xaf listens on the stand-in internet; xae dials it.
printf 'sovereign_id=%s\nedge_address=%s\nedge_port=%s\nlistens=true\nrelease=release/all/** release/%s/**\n' "$Y" "$YIP" "$PORT" "$X" > "$DIR/facts-$X.txt"
printf 'sovereign_id=%s\nedge_address=%s\nedge_port=%s\nlistens=false\nrelease=release/all/** release/%s/**\n' "$X" "$XIP" "$PORT" "$Y" > "$DIR/facts-$Y.txt"

side() { # $1 us, $2 them, $3 our address on the stand-in internet
  FE2E_PARTNER=external FE2E_PARTNER_FACTS="$DIR/facts-$1.txt" \
  FE2E_EXCHANGE_OUT="$DIR/$1-out" FE2E_EXCHANGE_IN="$DIR/$2-out" \
  FE2E_SOLO_WAN="$WAN" FE2E_SOLO_WAN_IP="$3" FE2E_SOLO_STEP_TIMEOUT="$STEP_TIMEOUT" \
    "$HERE/env" run --ref "$REF" --name "solo-$1" --avenue goatbuilder --scenario solo-external-partner \
      --jobs 1 --out "$DIR/results" --goat-version "$1=$VER" --parties "$1 $2" > "$DIR/logs/$1.log" 2>&1
}
side "$X" "$Y" "$XIP" & PX=$!
side "$Y" "$X" "$YIP" & PY=$!
PIDS="$PX $PY"
echo "solo-pair: $X (pid $PX) and $Y (pid $PY) started; logs in $DIR/logs"

kv() { sed -n "s/^$2=//p" "$1" 2>/dev/null | tail -1; }
fp() { echo "sha256:$(openssl x509 -in "$1" -outform DER 2>/dev/null | sha256sum | cut -d' ' -f1)"; }
answer() { # $1 side, $2 file name in its IN, text on stdin
  local in="$DIR/$(other "$1")-out"
  cat > "$in/.$2.tmp" && mv "$in/.$2.tmp" "$in/$2"
  echo "solo-pair: $(date -u +%H:%M:%SZ) answered $1's $2"
}
other() { [ "$1" = "$X" ] && echo "$Y" || echo "$X"; }
waiting() { [ -e "$DIR/$1-out/$2.waiting" ]; }
answered() { [ -e "$DIR/$(other "$1")-out/$2" ]; }
REACHED=""
reach() { case " $REACHED " in *" $1:$2 "*) ;; *) REACHED="$REACHED $1:$2" ;; esac; }
reached() { case " $REACHED " in *" $1:$2 "*) return 0 ;; *) return 1 ;; esac; }

alive() { kill -0 "$PX" 2>/dev/null || kill -0 "$PY" 2>/dev/null; }
while alive; do
  for s in "$X" "$Y"; do
    t="$(other "$s")"; in="$DIR/$t-out"
    # The voice call: what the other side read aloud must match the files this side received.
    if waiting "$s" voice-call && ! answered "$s" voice-call.ok && [ -s "$in/$t-voice-call.txt" ]; then
      heard_key="$(kv "$in/$t-voice-call.txt" signing_key)"; heard_fp="$(kv "$in/$t-voice-call.txt" ca_fingerprint)"
      if [ "$heard_fp" = "$(fp "$in/$t-ca.pem")" ]; then
        printf 'signing_key=%s\nca_fingerprint=%s\n' "$heard_key" "$heard_fp" | answer "$s" voice-call.ok
      else
        echo "solo-pair: $t read aloud $heard_fp, but $s received a CA with $(fp "$in/$t-ca.pem"); not confirmed" >&2
      fi
    fi
    if waiting "$s" digest && ! answered "$s" digest.ok && [ -s "$in/$t-agreement-digest.txt" ]; then
      answer "$s" digest.ok < "$in/$t-agreement-digest.txt"
    fi
    if waiting "$s" admit && ! answered "$s" admit.ok; then echo "admit" | answer "$s" admit.ok; fi
    for step in tunnel accept persist; do waiting "$s" "$step" && reach "$s" "$step"; done
  done
  # Steps that need the other side to have got as far: answered for both at once.
  for step in tunnel accept persist; do
    if reached "$X" "$step" && reached "$Y" "$step"; then
      for s in "$X" "$Y"; do answered "$s" "$step.ok" || echo "$step" | answer "$s" "$step.ok"; done
    fi
  done
  sleep 5
done
rc=0; wait "$PX" || rc=1; wait "$PY" || rc=1; PIDS=""
for s in "$X" "$Y"; do
  echo "=== $s"
  grep -E '^## Federation e2e|^run: ' "$DIR/logs/$s.log" || tail -5 "$DIR/logs/$s.log"
done
exit "$rc"
