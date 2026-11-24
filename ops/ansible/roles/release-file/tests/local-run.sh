#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# local-run.sh: the release-file role, run for real on localhost against
# scratch directories, with no privilege escalation:
#   1. install v1: current names v1's file, no previous, one ledger row (v1's pin);
#   2. install v2: current names v2, previous names v1, a row for v2;
#   3. the same v2 again: nothing moves, no new row;
#   4. roll back: current names v1, previous names v2, a row for v1's pin,
#      read from v1's own directory;
#   5. a copy whose SHA-256 is not the one passed is refused: current and the
#      file it names are unchanged;
#   6. a previous file changed on disk is refused, current unchanged;
#   7. a rollback with no previous file is refused.
set -euo pipefail
cd "$(dirname "$0")"
if ! command -v ansible-playbook >/dev/null 2>&1; then
  echo "[release-file/local-run] SKIP (ansible-playbook not on PATH)"
  exit 0
fi

S="$(mktemp -d)"
trap 'rm -rf "$S"' EXIT
fails=0
ok() { echo "  ok    $1"; }
bad() { echo "  FAIL  $1"; fails=$((fails + 1)); }

sha() { python3 -c 'import hashlib,sys; print(hashlib.sha256(open(sys.argv[1],"rb").read()).hexdigest())' "$1"; }
mkdir -p "$S/src" "$S/root" "$S/ledger"
printf 'v1 build\n' > "$S/src/v1"
printf 'v2 build\n' > "$S/src/v2"
V1="$(sha "$S/src/v1")"
V2="$(sha "$S/src/v2")"
PIN1="example-org/fixture-tool@v1.0.0@sha256:$(printf '1%.0s' $(seq 64))"
PIN2="example-org/fixture-tool@v2.0.0@sha256:$(printf '2%.0s' $(seq 64))"
HOME_DIR="$S/root/fixture-tool"
LEDGER="$S/ledger/deploy-history.jsonl"

# play <log> <extra -e words...>: one run of the role.
play() {
  local log="$1"
  shift
  ANSIBLE_ROLES_PATH="$(cd ../.. && pwd)" ANSIBLE_LOCALHOST_WARNING=False ANSIBLE_INVENTORY_UNPARSED_WARNING=False \
    ansible-playbook -i 'localhost,' -c local playbook.yml \
      -e ansible_become=false -e 'ansible_python_interpreter={{ ansible_playbook_python }}' \
      -e release_file_root="$S/root" \
      -e release_file_owner="$(id -un)" -e release_file_group="$(id -gn)" \
      -e rollback_ledger_dir="$S/ledger" \
      -e rollback_ledger_owner="$(id -un)" -e rollback_ledger_group="$(id -gn)" \
      -e rollback_assembled_marker="$S/no-marker" \
      "$@" > "$S/$log.log" 2>&1
}

install() { play "$1" -e release_file_src="$S/src/$2" -e release_file_name=fixture-tool-linux-amd64 -e release_file_sha256="$3" -e release_file_ref="$4"; }
target() { readlink "$HOME_DIR/$1" 2>/dev/null || echo none; }
rows() { [ -f "$LEDGER" ] && wc -l < "$LEDGER" | tr -d ' ' || echo 0; }
last_ref() { python3 -c 'import json,sys; print(json.loads(open(sys.argv[1]).read().splitlines()[-1]).get("ref",""))' "$LEDGER"; }
last_type() { python3 -c 'import json,sys; print(json.loads(open(sys.argv[1]).read().splitlines()[-1]).get("artifact_type",""))' "$LEDGER"; }
F1="$HOME_DIR/$V1/fixture-tool-linux-amd64"
F2="$HOME_DIR/$V2/fixture-tool-linux-amd64"

echo "[release-file/local-run] 1: install v1"
if install one v1 "$V1" "$PIN1"; then ok "the run passes"; else bad "the run fails"; tail -30 "$S/one.log"; fi
if [ "$(target current)" = "$F1" ]; then ok "current names v1"; else bad "current is $(target current)"; fi
if [ "$(target previous)" = none ]; then ok "no previous"; else bad "previous is $(target previous)"; fi
if [ "$(rows)" = 1 ] && [ "$(last_ref)" = "$PIN1" ] && [ "$(last_type)" = release ]; then ok "one ledger row, release, v1's pin"; else bad "ledger: $(rows) rows, last $(last_ref) $(last_type)"; fi

echo "[release-file/local-run] 2: install v2"
if install two v2 "$V2" "$PIN2"; then ok "the run passes"; else bad "the run fails"; tail -30 "$S/two.log"; fi
if [ "$(target current)" = "$F2" ]; then ok "current names v2"; else bad "current is $(target current)"; fi
if [ "$(target previous)" = "$F1" ]; then ok "previous names v1"; else bad "previous is $(target previous)"; fi
if [ "$(rows)" = 2 ] && [ "$(last_ref)" = "$PIN2" ]; then ok "a row for v2's pin"; else bad "ledger: $(rows) rows, last $(last_ref)"; fi

echo "[release-file/local-run] 3: the same v2 again"
if install three v2 "$V2" "$PIN2"; then ok "the run passes"; else bad "the run fails"; tail -30 "$S/three.log"; fi
if [ "$(target current)" = "$F2" ] && [ "$(target previous)" = "$F1" ]; then ok "nothing moves"; else bad "current $(target current), previous $(target previous)"; fi
if [ "$(rows)" = 2 ]; then ok "no new row"; else bad "ledger: $(rows) rows"; fi

echo "[release-file/local-run] 4: roll back"
if play four -e release_file_rollback=true; then ok "the run passes"; else bad "the run fails"; tail -30 "$S/four.log"; fi
if [ "$(target current)" = "$F1" ]; then ok "current names v1"; else bad "current is $(target current)"; fi
if [ "$(target previous)" = "$F2" ]; then ok "previous names v2"; else bad "previous is $(target previous)"; fi
if [ "$(rows)" = 3 ] && [ "$(last_ref)" = "$PIN1" ]; then ok "a row for v1's pin, read from its directory"; else bad "ledger: $(rows) rows, last $(last_ref)"; fi

echo "[release-file/local-run] 5: a copy that is not the verified file"
if install five v2 "$V1" "$PIN1"; then bad "the run passes"; else ok "the run is refused"; fi
if grep -q "is sha256 $V2, not $V1: nothing installed" "$S/five.log"; then ok "it names both digests"; else bad "no digest message"; tail -20 "$S/five.log"; fi
if [ "$(target current)" = "$F1" ]; then ok "current unchanged"; else bad "current is $(target current)"; fi
if [ "$(sha "$F1")" = "$V1" ]; then ok "v1's file is still v1's bytes"; else bad "v1's file was overwritten"; fi

echo "[release-file/local-run] 6: a previous file changed on disk"
chmod u+w "$F2"
printf 'tampered\n' > "$F2"
if play six -e release_file_rollback=true; then bad "the run passes"; else ok "the run is refused"; fi
if grep -q "refusing to switch to it" "$S/six.log"; then ok "it says why"; else bad "no reason"; tail -20 "$S/six.log"; fi
if [ "$(target current)" = "$F1" ]; then ok "current unchanged"; else bad "current is $(target current)"; fi

echo "[release-file/local-run] 7: no previous file"
rm -f "$HOME_DIR/previous"
if play seven -e release_file_rollback=true; then bad "the run passes"; else ok "the run is refused"; fi
if grep -q "has no earlier file of fixture-tool" "$S/seven.log"; then ok "it says so"; else bad "no reason"; tail -20 "$S/seven.log"; fi

if [ "$fails" -ne 0 ]; then
  echo "[release-file/local-run] FAIL ($fails)"
  exit 1
fi
echo "[release-file/local-run] PASS"
