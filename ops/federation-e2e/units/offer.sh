#!/usr/bin/env bash
# units/offer.sh — B5 of docs/design/goatbuilder-v1-gate.md: an offer narrower than the
# deployment's release sets is carried out for one partner.
#
# nothing-hand-written (units/nohand.sh) with one more line of operator input: {A}'s peering
# entry offers only release/all/**, while {A}'s environment releases release/all/** and
# release/{B}/**. Then, through each core's own REST surface (loopback, read on the host):
# the agreement names only the offered set for {A}'s flow; a value staged under it on {A}
# arrives at {B}; a value staged under the set it no longer offers does not, with an offered
# control showing the path is live.
# shellcheck source=/dev/null
. "$HERE/units/nohand.sh"

# Each core's REST surface, on the core host itself (a goat-zenoh role core binds loopback).
of_put() { host_exec "$HP-$1-core" curl -s -m 8 -X PUT -d "$3" "http://127.0.0.1:8000/$2" -o /dev/null -w '%{http_code}'; } # $1 core, $2 key, $3 value
of_get() { # $1 core, $2 key or key expression: each stored key and value, decoded
  host_exec "$HP-$1-core" curl -s -m 5 "http://127.0.0.1:8000/$2" | python3 -c 'import base64,json,sys
try: d=json.load(sys.stdin)
except Exception: sys.exit(0)
for r in d if isinstance(d,list) else []:
    v=r.get("value","")
    try: v=base64.b64decode(v, validate=True).decode()
    except Exception: pass
    print(r.get("key",""), v)'
}
of_arrives() { # $1 sender, $2 receiver, $3 set (release/<set>/), $4 seconds
  local n t0; n="b5$(date +%s)$RANDOM"; t0="$(date +%s)"
  while [ $(($(date +%s) - t0)) -lt "$4" ]; do
    of_put "$1" "release/$3/b5/$n" "$n" >/dev/null
    if of_get "$2" "from/$1/$3/b5/$n" | grep -qx "from/$1/$3/b5/$n $n"; then
      echo "from/$1/$3/b5/$n stored at $2 after $(($(date +%s) - t0))s"; return 0
    fi
    sleep 3
  done
  echo "from/$1/$3/b5/$n not stored at $2 within $4s"; return 1
}
export -f of_put of_get of_arrives

unit_main() {
  eval "NH_OFFER_$A=release/all/**"; export "NH_OFFER_$A"
  nh_main
  host_exec "$(drv_host "$A")" cat "/var/lib/goat-federation/$B/agreement.json" > "$WORK/$A/b5-agreement.json" 2>/dev/null || true
  rec "b5-$A-offers-only-the-narrowed-set" -- python3 - "$WORK/$A/b5-agreement.json" "$A" <<'PY'
import json, sys
a = json.load(open(sys.argv[1])); us = sys.argv[2]
rel = [f.get("release") for f in a.get("flows", []) if f.get("from") == us]
print("flows from", us, ":", rel)
sys.exit(0 if rel == [["release/all/**"]] else 1)
PY
  rec "b5-$A-$B-offered-set-arrives" -- bash -c "of_arrives '$A' '$B' all 90"
  # The set no longer offered: staged on {A}, with release/all as the live control.
  rec "b5-$A-$B-narrowed-out-set-does-not-arrive" -- bash -c "
    x=b5x\$(date +%s)\$RANDOM
    of_put '$A' 'release/$B/b5/'\$x \$x >/dev/null
    of_arrives '$A' '$B' all 90 || { echo 'the control did not arrive: the path is dead, nothing to compare'; exit 1; }
    sleep 10
    v=\$(of_get '$B' 'from/$A/**')
    case \"\$v\" in *\$x*) echo \"crossed: \$(printf '%s\n' \"\$v\" | grep \$x)\"; exit 1 ;; esac
    echo \"release/$B/b5/\$x staged on $A is absent at $B (\$(printf '%s' \"\$v\" | grep -c .) values under from/$A/**)\""
}
