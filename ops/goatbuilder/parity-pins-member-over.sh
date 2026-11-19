#!/usr/bin/env bash
# parity-pins-member-over.sh — ADR 1256 on both avenues: a member deviation
# names the goatnet value it overrides (`over:`), and a stale or missing one
# is refused the same way by ops/pins/generate-pins.py and `goatbuilder pins`.
#
# Each case in ops/goatbuilder/fixtures/pins-member-over/*.yaml is a whole pin
# table for the synthetic goatnet `zagros`, with an `# expect: <exit> [needle]`
# line. For each case, on two scratch copies of the tree (nothing committed
# moves), both avenues regenerate. The case passes when:
#   - both exit with the expected code, and the message carries the needle
#     (an error code such as GB-357, or WARN for a warning);
#   - both print the same refusal or warning lines;
#   - on exit 0, every generated group_vars/<member>/pins.yml is byte-identical.
#
# usage: parity-pins-member-over.sh <goatbuilder binary>
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"
BIN="${1:?usage: parity-pins-member-over.sh <goatbuilder binary>}"
CASES="$REPO/ops/goatbuilder/fixtures/pins-member-over"
SCRATCH="$(mktemp -d)"; trap 'rm -rf "$SCRATCH"' EXIT

tree() { # $1 destination, $2 case table
  mkdir -p "$1"
  ( cd "$REPO" && for d in ops/pins ops/ansible/group_vars ops/ansible/roles ops/consistency ops/enrollment/public-keys ops/release; do [ -e "$d" ] && printf "%s\n" "$d"; done | tar cf - -T - ) | ( cd "$1" && tar xf - )
  mkdir -p "$1/ops/ansible/group_vars/zagros_a_hosts" "$1/ops/ansible/group_vars/zagros_b_hosts"
  cp "$2" "$1/ops/pins/fleet-pins.yaml"
}
# The refusal or warning lines, with each avenue's own prefix taken off.
lines() { sed -E 's/^(ok    pins|FAIL  pins)  //; s/^(ERROR: |goatbuilder: )//' | grep -E '^(GB-35[78]|WARN)' || true; }

fail=0; n=0
for case in "$CASES"/*.yaml; do
  name="$(basename "$case" .yaml)"; n=$((n + 1))
  read -r want needle < <(sed -n 's/^# expect: //p' "$case")
  tree "$SCRATCH/$name/py" "$case"; tree "$SCRATCH/$name/rs" "$case"
  set +e
  py="$( cd "$SCRATCH/$name/py" && python3 ops/pins/generate-pins.py 2>&1 )"; py_rc=$?
  rs="$( "$BIN" -C "$SCRATCH/$name/rs" pins --yes 2>&1 )"; rs_rc=$?
  set -e
  bad=""
  [ "$py_rc" = "$want" ] || bad="$bad generate-pins.py exit $py_rc (want $want);"
  [ "$rs_rc" = "$want" ] || bad="$bad goatbuilder exit $rs_rc (want $want);"
  if [ -n "${needle:-}" ]; then
    printf '%s\n' "$py" | grep -q -- "$needle" || bad="$bad generate-pins.py says no $needle;"
    printf '%s\n' "$rs" | grep -q -- "$needle" || bad="$bad goatbuilder says no $needle;"
  fi
  [ "$(printf '%s\n' "$py" | lines)" = "$(printf '%s\n' "$rs" | lines)" ] || bad="$bad the two avenues print different lines;"
  if [ "$want" = 0 ]; then
    for m in zagros_a_hosts zagros_b_hosts; do
      cmp -s "$SCRATCH/$name/py/ops/ansible/group_vars/$m/pins.yml" "$SCRATCH/$name/rs/ops/ansible/group_vars/$m/pins.yml" \
        || bad="$bad $m/pins.yml differs;"
    done
  fi
  if [ -n "$bad" ]; then
    fail=1; echo "FAIL $name:$bad"
    printf '  py: %s\n' "$(printf '%s\n' "$py" | lines | head -3)"
    printf '  rs: %s\n' "$(printf '%s\n' "$rs" | lines | head -3)"
  else
    echo "ok   $name (exit $want${needle:+, $needle})"
  fi
done
[ "$n" -gt 0 ] || { echo "FAIL: no case under $CASES"; exit 1; }

# The ladder names the members a hotfix would miss: `graduate.py hotfix` and
# `goatbuilder promote hotfix` print the same advice, and both `status` views list
# the member deviation off its rung's value. A two-rung ladder over zagros.
# shellcheck disable=SC2016  # the backticks are literal text in the tools' output
norm() { sed -E 's#python3 ops/release/graduate.py#TOOL#g; s#goatbuilder promote#TOOL#g; s#graduate\.py (graduate|check|abandon)#TOOL \1#g; s#`promote (check|graduate|abandon)`#`TOOL \1`#g; s#[0-9]{8}-hotfix-#DATE-hotfix-#g; s#HOTFIX \([0-9-]+,#HOTFIX (DATE,#g'; }
rows() { sed -E 's/^(ok    note|FAIL  refused)  //' | grep -v '^promote: '; }
ladder_tree() { # $1 destination
  tree "$1" "$CASES/02-hotfix-shadowed-stale-over.yaml"
  mkdir -p "$1/ops/ansible/group_vars/zagros_pre_hosts"
  python3 - "$1" <<'PY'
import sys, yaml
root = sys.argv[1]
p = root + "/ops/pins/fleet-pins.yaml"
t = yaml.safe_load(open(p))
t["goatnets"]["zagros_pre"] = {"description": "the synthetic example goatnet's preprod", "members": ["zagros_pre_hosts"]}
# Before the hotfix: the member deviation was written over the fleet value.
del t["components"]["signal-server"]["deviations"]["zagros"]
yaml.safe_dump(t, open(p, "w"), sort_keys=False)
yaml.safe_dump({"rungs": [{"name": "preprod", "goatnet": "zagros_pre", "status": "live"},
                          {"name": "prod", "goatnet": "zagros", "status": "live"}],
                "hops": [{"from": "preprod", "to": "prod", "status": "live", "gates": []}],
                "class_overrides": {}, "legacy_skew_allowlist": {}},
               open(root + "/ops/release/ladder.yaml", "w"), sort_keys=False)
PY
}
ladder_tree "$SCRATCH/ladder/py"; ladder_tree "$SCRATCH/ladder/rs"
HOT="ghcr.io/example/signal-server@sha256:$(printf 'b%.0s' $(seq 64))"
for verb in status hotfix; do
  if [ "$verb" = status ]; then args=(status); else args=(hotfix signal-server --ref "$HOT" --reason "example fix" --ack "Parity Gate"); fi
  py="$( ( cd "$SCRATCH/ladder/py" && python3 ops/release/graduate.py "${args[@]}" 2>&1 ) | norm || true )"
  rs="$( "$BIN" -C "$SCRATCH/ladder/rs" --yes promote "${args[@]}" 2>&1 | rows | norm || true )"
  if [ "$py" != "$rs" ]; then
    fail=1; echo "FAIL promote $verb: the two avenues differ"
    { diff <(printf '%s\n' "$py") <(printf '%s\n' "$rs") || true; } | sed 's/^/    /' | head -20
  elif ! printf '%s\n' "$py" | grep -q 'zagros_b_hosts'; then
    fail=1; echo "FAIL promote $verb: the member that shadows the goatnet is not named"
    printf '%s\n' "$py" | sed 's/^/    /' | tail -8
  else
    echo "ok   promote $verb names zagros_b_hosts on both avenues"
  fi
done
exit $fail
