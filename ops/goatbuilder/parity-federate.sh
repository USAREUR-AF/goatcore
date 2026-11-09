#!/usr/bin/env bash
# parity-federate.sh — the parity gate for `goatbuilder federate` against
# ops/scripts/peering-standup.sh (goatbuilder-conformance.md §The parity gate).
#
# Both sides plan the same peering — the script with --plan, the binary with
# --dry-run — in the same empty working directory (FED_WORK), so both read no
# keyring and no agreement and phase 11 refuses on both sides with the same
# words. The binary reads the declaration from the tutorial-3 fixture
# because a peering must be declared (C1) and the repository's own config
# declares none; the fixture changes no command either side plans. Every phase header and command is compared in order after
# normalisation: the repository path, the working directory and the goat
# binary's path are masked, runs of whitespace are collapsed. The banner and
# the plan footer are not compared.
#
# Both avenues run the federation verbs (phases 3, 4, 6, 7, 9) one of two ways,
# chosen by FED_VERBS: `goatbuilder` (the default, `goatbuilder federate
# <verb>`; the binary runs them in process) or `goat` (`goat federate
# <verb>`). Phase 3 measures one of two ways, chosen by FED_MEASURE: `here`
# (the machine the run is on) or `ssh` (`goatbuilder federate init
# --over-ssh`, the edge and core over SSH). The gate compares the two
# avenues in all four combinations, so none can drift while it is not the
# default, and once more with FED_VERBS and FED_MEASURE unset, so both
# avenues always have the same defaults.
#
# usage: parity-federate.sh <goatbuilder binary> <peering> [<peering> ...]
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"
BIN="${1:?}"; shift
[ $# -gt 0 ] || { echo "usage: parity-federate.sh <binary> <peering> ..."; exit 2; }
WORK="$(mktemp -d -t goatbuilder-parity-federate.XXXXXX)"
trap 'rm -rf "$WORK"' EXIT
export FED_WORK="$WORK/work"
GOAT="$(command -v goat || echo goat)"
GBBIN="$(command -v goatbuilder || echo goatbuilder)"
MOCK="$REPO/ops/goatbuilder/fixtures/goatnet-from-scratch/03-boundary-node"

norm() {
  sed -E 's#\x1b\[[0-9;]*m##g' \
  | sed -E 's#'"$REPO"'#REPO#g; s#'"$FED_WORK"'#WORK#g; s#'"$GOAT"' federate#GOAT federate#g; s#'"$GBBIN"' federate#goatbuilder federate#g' \
  | grep -E '^(=== |    )' \
  | sed -E 's/^[[:space:]]+//; s/[[:space:]]+$//; s/[[:space:]]+/ /g'
}
script_side() { "$REPO/ops/scripts/peering-standup.sh" "$1" --plan 2>&1 | norm; }
# The binary's rows come from -o json: the human view of a dry run is a plan
# table (federate.rs plan_view), and the rows are the same phase headers and
# commands the table used to print, one per row.
binary_side() {
  "$BIN" -C "$REPO" -o json --mock "$MOCK" federate "$1" --dry-run 2>/dev/null \
  | python3 -c 'import json,sys
for c in json.load(sys.stdin)["checks"]:
    if c["name"] == "phase": print(c["detail"])
    elif c["name"] == "cmd": print("    " + c["detail"])' \
  | norm
}

fail=0
# Each combination, then both unset: the last pass compares the two avenues'
# DEFAULTS, so one avenue's default cannot move without the other's.
for mode in "goat here" "goatbuilder here" "goat ssh" "goatbuilder ssh" "unset unset"; do
  verbs="${mode% *}"; measure="${mode#* }"
  if [ "$verbs" = unset ]; then unset FED_VERBS; else export FED_VERBS="$verbs"; fi
  if [ "$measure" = unset ]; then unset FED_MEASURE; else export FED_MEASURE="$measure"; fi
  for peering in "$@"; do
    a="$(script_side "$peering" || true)"; b="$(binary_side "$peering" || true)"
    if [ -z "$a" ]; then
      fail=1; echo "PARITY FAIL: federate $peering (FED_VERBS=$verbs FED_MEASURE=$measure) — the script produced no phases"; continue
    fi
    if [ "$verbs" != unset ] && ! printf '%s\n' "$b" | grep -q "^$( [ "$verbs" = goat ] && echo GOAT || echo goatbuilder) federate lint "; then
      fail=1; echo "PARITY FAIL: federate $peering (FED_VERBS=$verbs FED_MEASURE=$measure) — phase 4 does not run through $verbs"; continue
    fi
    if [ "$measure" = ssh ] && ! printf '%s\n' "$b" | grep -q "^goatbuilder federate init .*--over-ssh '$peering' "; then
      fail=1; echo "PARITY FAIL: federate $peering (FED_MEASURE=ssh) — phase 3 does not measure over SSH"; continue
    fi
    if [ "$a" != "$b" ]; then
      fail=1; echo "PARITY FAIL: federate $peering (FED_VERBS=$verbs FED_MEASURE=$measure)"
      { diff <(printf '%s\n' "$a") <(printf '%s\n' "$b") || true; } | sed 's/^/    /' | cut -c1-220 | head -40
    else
      echo "parity ok: federate $peering --dry-run, FED_VERBS=$verbs FED_MEASURE=$measure ($(printf '%s\n' "$a" | grep -c '^=== ') phases, $(printf '%s\n' "$a" | grep -c .) lines identical)"
    fi
  done
done
exit $fail
