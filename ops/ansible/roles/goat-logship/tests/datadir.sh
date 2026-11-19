#!/usr/bin/env bash
# GHSA-x44g: the root-run data-dir tasks never follow a link the collector
# planted in its writable data dir, and walk the tree only to hand over a dir
# a root-run collector wrote. Runs tasks/datadir.yml on localhost as the
# invoking user against scratch dirs; a group the invoker belongs to stands in
# for the collector's, so the hand-over is observable without root.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
fails=0
fail() { echo "  FAIL $1"; fails=$((fails + 1)); }
my_gid="$(id -g)"
tg="$(id -G | tr ' ' '\n' | grep -vx "$my_gid" | grep -vx 0 | head -1 || true)"
[ -n "$tg" ] || { echo "[goat-logship/datadir] SKIP (invoker has no supplementary group)"; exit 0; }
play() { ansible-playbook -i localhost, --connection local "$SCRIPT_DIR/datadir.yml" "$@" 2>&1; }
recap() { printf '%s\n' "$1" | grep -E '^localhost +:' | tr -s ' ' || true; }
data="$work/data"; out="$work/outside"
mkdir -p "$data/buffer" "$out"; : > "$data/buffer/chk"; : > "$out/target"; chmod 0644 "$out/target"
ln -s "$out" "$data/escape"; ln -s "$out/target" "$data/escape-file"
args=(-e logship_data_root="$data" -e logship_compose_dir="$work/compose" -e _logship_uid="$(id -u)" -e _logship_gid="$tg"
      -e '{"logship_source_identity_enroll": false, "logship_source_app_http": false, "logship_source_app_auth": false}')
before="$(stat -c '%g %a' "$out" "$out/target" | tr '\n' ' ')"

# 1. A dir a root-run collector wrote (group not yet the collector's): handed
#    over recursively, but the planted links' targets are left alone.
if o="$(play "${args[@]}")"; then
  [ "$(stat -c %g "$data/buffer/chk")" = "$tg" ] && echo "  ok   hand-over regroups the collector's own files" || fail "hand-over did not reach data/buffer/chk"
  [ "$(stat -c '%g %a' "$out" "$out/target" | tr '\n' ' ')" = "$before" ] && echo "  ok   planted symlinks are not followed (targets unchanged)" \
    || fail "a symlink target changed: $before -> $(stat -c '%g %a' "$out" "$out/target" | tr '\n' ' ')"
else fail "hand-over: play failed"; printf '%s\n' "$o" | tail -6; fi

# 2. Already handed over: the tree is not walked again.
if o="$(play "${args[@]}")" && printf '%s\n' "$o" | grep -A1 'Hand the data dir written by a root-run collector' | grep -q '^skipping'; then
  echo "  ok   a handed-over dir is not walked again ($(recap "$o"))"
else fail "second run walked the tree: $(recap "$o")"; fi

# 3. A collector still running as root this run: walked again, links still not followed.
if o="$(play "${args[@]}" -e '{"_logship_running_user": {"rc": 0, "stdout": "root"}}')" \
   && printf '%s\n' "$o" | grep -A1 'Hand the data dir written by a root-run collector' | grep -qE '^(ok|changed)' \
   && [ "$(stat -c '%g %a' "$out" "$out/target" | tr '\n' ' ')" = "$before" ]; then
  echo "  ok   root-run collector: tree walked, links still not followed"
else fail "root-run hand-over: $(recap "$o")"; fi

# 4. The mock sink dir replaced by a link out of the data dir: not followed.
rm -rf "$data/ocsf"; ln -s "$out" "$data/ocsf"
o="$(play "${args[@]}")" || true
[ "$(stat -c '%g %a' "$out" "$out/target" | tr '\n' ' ')" = "$before" ] && echo "  ok   a mock sink dir swapped for a link is not followed" \
  || fail "mock sink link followed: $(stat -c '%g %a' "$out")"

[ "$fails" -eq 0 ] || { echo "goat-logship: datadir FAILED ($fails)"; exit 1; }
echo "goat-logship: datadir OK"
