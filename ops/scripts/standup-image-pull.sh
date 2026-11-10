#!/usr/bin/env bash
# standup-image-pull.sh — pull a list of images on THIS host, in parallel, and name
# every one that could not be pulled. Run on the box by ops/ansible/standup-image-pull.yml
# (ansible.builtin.script) with the list ops/scripts/standup-images.py prints, between
# phase 2 and phase 5 of a standup (docs/design/goatlite.md §11 measure 2).
#
#   standup-image-pull.sh [--concurrency N] IMAGE [IMAGE ...]
#
# At most N pulls run at once (default 4). Each failure is printed as
#   could not pull <image>: <the last line docker printed>
# on stdout (Ansible's script module over SSH folds stderr into stdout), and the
# script exits 1 after every pull has finished, so one run names them all.
# A digest-pinned image (`...@sha256:<64 hex>`) already in the host's image store is not
# pulled again: its digest names its content, so nothing newer can exist under it. That
# is how a box started from the prepared VM image (ops/vm-image/, goatlite.md §11
# measure 5) skips every pull its image already holds, while a pin that moved since the
# image was built is pulled as before. A tag-only image is always pulled.
# Exit: 0 every image present; 1 one or more could not be pulled; 2 usage.
set -uo pipefail
n=4
if [ "${1:-}" = "--concurrency" ]; then n="${2:-}"; shift 2 || true; fi
case "$n" in ''|*[!0-9]*|0) echo "standup-image-pull: --concurrency wants a whole number above 0" >&2; exit 2 ;; esac
[ $# -gt 0 ] || { echo "usage: standup-image-pull.sh [--concurrency N] IMAGE [IMAGE ...]" >&2; exit 2; }

out="$(mktemp -d)"; trap 'rm -rf "$out"' EXIT
t0=$(date +%s)
i=0
for img in "$@"; do
  # bounded: wait for a slot before starting the next pull
  while [ "$(jobs -rp | wc -l)" -ge "$n" ]; do sleep 0.2; done
  ( case "$img" in
      *@sha256:*) if docker image inspect "$img" >/dev/null 2>&1; then : > "$out/ok.$i"; : > "$out/had.$i"; exit 0; fi ;;
    esac
    if msg="$(docker pull -q "$img" 2>&1)"; then : > "$out/ok.$i"
    else printf 'could not pull %s: %s\n' "$img" "$(printf '%s\n' "$msg" | grep -v '^[[:space:]]*$' | tail -n 1)" > "$out/fail.$i"; fi ) &
  i=$((i+1))
done
wait
ok=$(find "$out" -name 'ok.*' | wc -l | tr -d ' ')
failed=$(find "$out" -name 'fail.*' | wc -l | tr -d ' ')
had=$(find "$out" -name 'had.*' | wc -l | tr -d ' ')
echo "standup-image-pull: $ok of $# image(s) present after $(( $(date +%s) - t0 ))s, at most $n at a time ($had already on the host by digest, not pulled)"
if [ "$failed" -gt 0 ]; then
  cat "$out"/fail.*
  echo "standup-image-pull: $failed image(s) could not be pulled; phase 5 and later need them"
  exit 1
fi
exit 0
