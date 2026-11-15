#!/usr/bin/env bash
# parity-federate-verbs.sh — `goatbuilder federate <verb>` and `goat federate <verb>`
# give the same answer: the same stdout, byte for byte, and the same exit code,
# over every conformance vector in ops/federation/vectors/ (goatbuilder-conformance.md
# §"The parity gate").
#
# lint runs on every package and every malformed file, text and --json;
# compose on every pair, both orders, text, --json and --print; diff on a
# match, a field difference and a byte difference; verify and apply with --as.
# Both binaries read the same empty goat context (GOAT_PROFILE_DIR), so init
# compares too. stderr is not compared.
#
# usage: parity-federate-verbs.sh <goat binary> <goatbuilder binary>
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"
GOAT="${1:?usage: parity-federate-verbs.sh <goat> <goatbuilder>}"
GB="${2:?usage: parity-federate-verbs.sh <goat> <goatbuilder>}"
V="$REPO/ops/federation/vectors"
FAR_DIR="$REPO/ops/federation/fixtures/far-term"
FAR="$FAR_DIR/agreement.canonical.json"
T="$(mktemp -d -t parity-federate-verbs.XXXXXX)"
trap 'rm -rf "$T"' EXIT
mkdir -p "$T/profile/contexts/default"
export GOAT_PROFILE_DIR="$T/profile"
unset GOAT_CONTEXT GOAT_CONTEXT_DIR

KEYS="--key xaa=$(cat "$V/testkeys/xaa.public.b64") --key xab=$(cat "$V/testkeys/xab.public.b64")"
NOW="--now $(cat "$V/now.txt")"

runs() {   # one verb invocation per line, the words after `federate`
  cd "$V" || exit 2
  for f in */package-*.canonical.json malformed/*.json; do
    case "$f" in *signing-input*) continue ;; esac
    echo "lint $f"; echo "lint --json $f"
  done
  for d in compose-* refuse-*; do
    # The pair's two packages (a refusal pair may name its second differently).
    set --
    for f in "$d"/package-*.canonical.json; do
      case "$f" in *signing-input*) ;; *) set -- "$@" "$f" ;; esac
    done
    [ $# = 2 ] || { echo "PAIR $d"; continue; }
    echo "compose $1 $2 $KEYS $NOW"
    echo "compose $2 $1 $KEYS $NOW --json"
    echo "compose $1 $2 $KEYS $NOW --print"
  done
  echo "compose compose-full/package-xaa.canonical.json malformed/no-signature.json $KEYS $NOW"
  echo "diff compose-full/agreement.canonical.json compose-full/agreement.canonical.json"
  echo "diff compose-full/agreement.canonical.json compose-no-flows/agreement.canonical.json --json"
  echo "diff compose-full/agreement.canonical.json compose-full/agreement.pretty.json"
  # The normative vector agreement is judged on the wall clock here, so these
  # rows compare whatever both binaries say on the day: an attestation before
  # its term ends (2026-11-20), the same refusal after.
  echo "verify compose-full/agreement.canonical.json --as xaa"
  echo "verify compose-full/agreement.canonical.json --json"
  echo "apply compose-full/agreement.canonical.json --as xaa --edge-group g"
  echo "apply compose-full/agreement.canonical.json --as xab --json"
  # The far-term test fixture (expires 2099-12-31) is the agreement both
  # binaries must attest and render on any date the gate runs.
  echo "verify $FAR --as xta"
  echo "verify $FAR --as xta --json"
  echo "apply $FAR --as xta --edge-group g"
  echo "apply $FAR --as xtb --json"
  echo "init"
  echo "init --json"
}

fail=0; n=0
while IFS= read -r line; do
  n=$((n + 1))
  if [ "${line%% *}" = PAIR ]; then fail=1; echo "PARITY FAIL: ${line#PAIR } does not hold exactly two packages"; continue; fi
  # shellcheck disable=SC2086 # the words are split on purpose
  (cd "$V" && "$GOAT" federate $line >"$T/a.out" 2>"$T/a.err"); ra=$?
  # shellcheck disable=SC2086
  (cd "$V" && "$GB" federate $line >"$T/b.out" 2>"$T/b.err"); rb=$?
  # The far-term fixture is one both parties can deliver on any date: verify
  # attests and apply renders, so a refusal on both sides is a failure, not parity.
  case "$line" in
    "verify $FAR --as "*|"apply $FAR --as "*)
      if [ "$ra" != 0 ] || [ "$rb" != 0 ]; then
        fail=1; echo "PARITY FAIL: federate $line — goat exit $ra, goatbuilder exit $rb; the far-term fixture must attest and render (exit 0)"
        sed 's/^/    /' "$T/a.out" | cut -c1-200 | head -20
        continue
      fi
      case "$line" in
        *--json*) if [ "${line%% *}" = verify ]; then want='"attestation":"conformance ok for sha256:'; else want='"files":["ops/'; fi ;;
        verify*) want='would publish `conformance ok for sha256:' ;;
        *) want='--- ' ;;
      esac
      if ! grep -qF -- "$want" "$T/a.out" || ! grep -qF -- "$want" "$T/b.out"; then
        fail=1; echo "PARITY FAIL: federate $line — exit 0 but no '$want' in the output; it did not attest or render"
        continue
      fi ;;
  esac
  if [ "$ra" != "$rb" ]; then
    fail=1; echo "PARITY FAIL: federate $line — goat exit $ra, goatbuilder exit $rb"
  elif ! cmp -s "$T/a.out" "$T/b.out"; then
    fail=1; echo "PARITY FAIL: federate $line — stdout differs"
    { diff "$T/a.out" "$T/b.out" || true; } | sed 's/^/    /' | cut -c1-200 | head -20
  fi
done < <(set +e; runs)

# verify and apply decide acceptance, so neither binary takes a caller-supplied
# evaluation date for them: an expired agreement must never be judged current.
for verb in "verify $FAR --as xta" "apply $FAR --as xta"; do
  for bin in "$GOAT" "$GB"; do
    # shellcheck disable=SC2086 # the words are split on purpose
    (cd "$V" && "$bin" federate $verb --now 2026-09-05 >/dev/null 2>&1); rc=$?
    if [ "$rc" != 2 ]; then
      fail=1; echo "PARITY FAIL: $(basename "$bin") federate $verb --now — exit $rc; a date flag on verify or apply must be refused as unknown (exit 2)"
    fi
  done
done

# The far-term fixture is what its committed generator makes, byte for byte.
if ! python3 "$FAR_DIR/generate.py" --check "$GB" >"$T/gen.out" 2>&1; then
  fail=1; echo "PARITY FAIL: ops/federation/fixtures/far-term is not what its generate.py makes"
  sed 's/^/    /' "$T/gen.out" | head -20
fi

[ "$n" -gt 100 ] || { echo "PARITY FAIL: only $n runs; the vectors were not found"; exit 1; }
[ "$fail" = 0 ] && echo "parity ok: goat federate and goatbuilder federate agree on $n runs over the vectors (stdout and exit code)"
exit $fail
