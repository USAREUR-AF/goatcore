#!/usr/bin/env bash
# tasks/repo-ownership.yml: a clean repository passes; one file owned by
# another uid (root, written through a container as the sidecar did) stops
# the run, and the message names that file. Needs ansible-playbook and docker.
# Runs with no privilege on this machine: the root-owned file is made and
# removed inside a container.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
AP="${ANSIBLE_PLAYBOOK:-ansible-playbook}"
IMG="${PGBR_GUARD_TEST_IMAGE:-alpine:3.24@sha256:f59abe656b692d901e184be3445e084786602108599d2a51d229c0f5a48c71bf}"
command -v "$AP" >/dev/null 2>&1 || { echo "[repo-ownership] SKIP (ansible-playbook not on PATH)"; exit 0; }
command -v docker >/dev/null 2>&1 || { echo "[repo-ownership] SKIP (docker not on PATH)"; exit 0; }
export ANSIBLE_BECOME=False
W="$(mktemp -d)"
cleanup() {
  docker run --rm -v "$W:/w" "$IMG" rm -rf /w/repo /w/log /w/spool >/dev/null 2>&1
  rm -rf "$W"
}
trap cleanup EXIT
mkdir -p "$W/repo/backup/st" "$W/log" "$W/spool"
echo x > "$W/repo/backup/st/backup.info"
fails=0
run() {
  (cd "$HERE" && "$AP" -i 'localhost,' repo-ownership.yml \
    -e "pgbackrest_repo1_dir=$W/repo" -e "pgbackrest_log_dir=$W/log" \
    -e "pgbackrest_spool_dir=$W/spool" -e "pgbackrest_repo_owner_uid=$(id -u)" 2>&1)
}
if out=$(run); then echo "ok   a repository owned by the database user passes"; else
  echo "FAIL a clean repository was refused:"; echo "$out" | tail -5; fails=$((fails+1)); fi
docker run --rm -v "$W:/w" "$IMG" sh -c 'echo x > /w/repo/backup/st/backup.info.copy'
if out=$(run); then echo "FAIL a root-owned file in the repository passed"; fails=$((fails+1))
elif grep -q "repo/backup/st/backup.info.copy" <<<"$out" && grep -q "Nothing was changed" <<<"$out"; then
  echo "ok   a root-owned file stops the run, and the message names it"
else echo "FAIL the refusal does not name the file:"; echo "$out" | grep -E 'msg|FAILED' | tail -3; fails=$((fails+1)); fi
if [ "$(docker run --rm -v "$W:/w" "$IMG" stat -c %u /w/repo/backup/st/backup.info.copy)" = 0 ]; then
  echo "ok   the guard changed no ownership"
else echo "FAIL ownership changed"; fails=$((fails+1)); fi
if [ "$fails" -ne 0 ]; then echo "[repo-ownership] FAIL ($fails)"; exit 1; fi
echo "[repo-ownership] PASS"
