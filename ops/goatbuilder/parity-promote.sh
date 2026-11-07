#!/usr/bin/env bash
# parity-promote.sh — the parity gate for `goatbuilder promote` against
# ops/release/graduate.py (goatbuilder-conformance.md §The parity gate: the pin
# per rung; the record graduated).
#
# Read-only on the committed tree, line for line: status, status --json,
# check, review, review --json, readiness (modulo its clock). Then on two
# scratch copies, one per side, the record mutations: open, hotfix, the three
# refusals (hotfix with previous == candidate, graduate not ready, graduate of
# a set member), graduate once the record is ready, abandon — comparing every
# printed line, and the records as DATA (parsed), since the two YAML dumpers
# fold long strings differently. The only normalisation is the tool's own
# name in the commands it prints (`python3 ops/release/graduate.py` /
# `goatbuilder promote`).
#
# usage: parity-promote.sh <goatbuilder binary>
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"
BIN="${1:?}"
SCRATCH="$(mktemp -d)"; trap 'rm -rf "$SCRATCH"' EXIT
export COLUMNS=100

norm() { sed -E 's#python3 ops/release/graduate.py#TOOL#g; s#goatbuilder promote#TOOL#g; s#graduate\.py (graduate|check|abandon)#TOOL \1#g; s#`promote (check|graduate|abandon)`#`TOOL \1`#g; s#`check`#`TOOL check`#g; s#python3 ops/pins/generate-pins.py#PINS#g; s#goatbuilder pins#PINS#g'; }
rows() { sed -E 's/^(ok    note|FAIL  refused)  //' | grep -v '^promote: '; }
py() { local d="$1"; shift; ( cd "$d" && python3 ops/release/graduate.py "$@" 2>&1 ) | norm || true; }
rs() { local d="$1"; shift; "$BIN" -C "$d" --yes promote "$@" 2>&1 | rows | norm || true; }
fail=0
compare() { # $1 label $2 a $3 b
  if [ "$2" != "$3" ]; then fail=1; echo "PARITY FAIL: $1"; { diff <(printf '%s\n' "$2") <(printf '%s\n' "$3") || true; } | sed 's/^/    /' | head -30
  else echo "parity ok: $1 ($(printf '%s\n' "$2" | grep -c .) lines identical)"; fi
}
same_record() { # $1 label $2 a $3 b
  if python3 - "$2" "$3" <<'PY'
import sys, yaml
a = yaml.safe_load(open(sys.argv[1])); b = yaml.safe_load(open(sys.argv[2]))
sys.exit(0 if a == b else 1)
PY
  then echo "parity ok: $1 (record identical as data)"; else fail=1; echo "PARITY FAIL: $1 — records differ as data"; diff "$2" "$3" | head -20 | sed 's/^/    /'; fi
}

# --- read-only, on the committed tree
compare "promote status" "$(py "$REPO" status)" "$(rs "$REPO" status)"
compare "promote status --json" "$(py "$REPO" status --json)" "$("$BIN" -C "$REPO" promote status --json 2>&1 || true)"
compare "promote check" "$(py "$REPO" check)" "$(rs "$REPO" check)"
compare "promote review" "$(py "$REPO" review)" "$(rs "$REPO" review)"
compare "promote review --json" "$(py "$REPO" review --json)" "$("$BIN" -C "$REPO" promote review --json 2>&1 | norm || true)"
compare "promote readiness (modulo generated_unix_ms)" "$(py "$REPO" readiness | grep -v generated_unix_ms)" "$("$BIN" -C "$REPO" promote readiness 2>&1 | grep -v generated_unix_ms || true)"

# --- mutations, each side on its own copy
for side in py rs; do
  mkdir -p "$SCRATCH/$side"
  ( cd "$REPO" && tar cf - ops/pins ops/ansible/group_vars ops/ansible/roles ops/release ops/consistency 2>/dev/null ) | ( cd "$SCRATCH/$side" && tar xf - )
done
COMP=netvis
# Two synthetic refs, distinct from each other and from any pin, so open,
# hotfix and the previous == candidate refusal each exercise their own path.
REF='ghcr.io/{{ ghcr_org }}/netvis:sha-abcdef0@sha256:2222222222222222222222222222222222222222222222222222222222222222'  # oss-allow: synthetic fixture ref
HOT='ghcr.io/{{ ghcr_org }}/netvis:sha-1234567@sha256:3333333333333333333333333333333333333333333333333333333333333333'  # oss-allow: synthetic fixture ref
compare "promote open" "$(py "$SCRATCH/py" open $COMP --ref "$REF" --reason "parity gate")" "$(rs "$SCRATCH/rs" open $COMP --ref "$REF" --reason "parity gate")"
R=""; for f in "$SCRATCH/py/ops/release/records/$COMP"/*.yaml; do case "$f" in *hotfix*) ;; *) R="$(basename "$f")" ;; esac; done
same_record "the opened record" "$SCRATCH/py/ops/release/records/$COMP/$R" "$SCRATCH/rs/ops/release/records/$COMP/$R"
compare "promote hotfix" "$(py "$SCRATCH/py" hotfix $COMP --ref "$HOT" --reason "hot" --ack "Parity Gate")" "$(rs "$SCRATCH/rs" hotfix $COMP --ref "$HOT" --reason "hot" --ack "Parity Gate")"
H=""; for f in "$SCRATCH/py/ops/release/records/$COMP"/*hotfix*.yaml; do H="$(basename "$f")"; done
same_record "the hotfix record" "$SCRATCH/py/ops/release/records/$COMP/$H" "$SCRATCH/rs/ops/release/records/$COMP/$H"
compare "hotfix refused (previous == candidate)" "$(py "$SCRATCH/py" hotfix $COMP --ref "$HOT" --reason "hot" --ack A --previous-ref "$HOT")" "$(rs "$SCRATCH/rs" hotfix $COMP --ref "$HOT" --reason "hot" --ack A --previous-ref "$HOT")"
compare "graduate refused (not ready)" "$(py "$SCRATCH/py" graduate $COMP --ack A)" "$(rs "$SCRATCH/rs" graduate $COMP --ack A)"
SETM="$(python3 -c "import yaml,glob; d=[yaml.safe_load(open(p)) for p in glob.glob('$REPO/ops/release/sets/*.yaml')]; o=[x for x in d if x.get('state')=='open']; print(o[0]['members'][0]['component'] if o else '')")"
if [ -n "$SETM" ]; then
  compare "graduate refused (open set member)" "$(py "$SCRATCH/py" graduate "$SETM" --ack A)" "$(rs "$SCRATCH/rs" graduate "$SETM" --ack A)"
fi
for side in py rs; do python3 - "$SCRATCH/$side/ops/release/records/$COMP/$R" <<'PY'
import sys, yaml
p = sys.argv[1]; d = yaml.safe_load(open(p))
d["opened"] = "2026-10-01"; d["evidence"] = {"validation": "rows PASS", "preprod_convergence": "console"}
open(p, "w").write(yaml.safe_dump(d, sort_keys=False))
PY
done
compare "promote graduate (ready)" "$(py "$SCRATCH/py" graduate $COMP --ack "Parity Gate")" "$(rs "$SCRATCH/rs" graduate $COMP --ack "Parity Gate")"
same_record "the graduated record" "$SCRATCH/py/ops/release/records/$COMP/$R" "$SCRATCH/rs/ops/release/records/$COMP/$R"
compare "promote abandon" "$(py "$SCRATCH/py" abandon "ops/release/records/$COMP/$H" --reason "parity" --superseded-by "$R")" "$(rs "$SCRATCH/rs" abandon "ops/release/records/$COMP/$H" --reason "parity" --superseded-by "$R")"
same_record "the abandoned record" "$SCRATCH/py/ops/release/records/$COMP/$H" "$SCRATCH/rs/ops/release/records/$COMP/$H"
compare "promote check after the mutations" "$(py "$SCRATCH/py" check)" "$(rs "$SCRATCH/rs" check)"
compare "promote status after the mutations" "$(py "$SCRATCH/py" status)" "$(rs "$SCRATCH/rs" status)"
exit $fail
