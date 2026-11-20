#!/usr/bin/env bash
# goat-teardown.sh — a site's standup, reversed: the scripts avenue of
# `goatbuilder teardown` (ADR 1201). Both read ops/goatbuilder/teardown.yaml;
# the teardown parity gate compares their plans line for line.
#
#   ops/scripts/goat-teardown.sh <site> --plan [--services] [--from N] [--to N] [--profile P]
#   ops/scripts/goat-teardown.sh <site> --services [--from N] ...   # run it
#
# Without --services a teardown refuses (exit 3) as soon as any reached step
# would destroy data, is not built yet, or is undeclared — a half-removed site
# is worse than a standing one. --services runs every step that stops a
# service (compose-down, service-stop, systemd-stop), volumes kept, and lists
# what it left in place. Each stop is proved on the host (nothing of it still
# runs) and recorded in /var/lib/goat/<component>/teardown.jsonl;
# deploy-history.jsonl is untouched.
set -uo pipefail
# Hold the Mac awake for the whole run (#2122; ops/scripts/lib/keep-awake.sh).
_keep_awake="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/keep-awake.sh"
# shellcheck source=ops/scripts/lib/keep-awake.sh
if [ -f "$_keep_awake" ]; then . "$_keep_awake"; goat_keep_awake; fi
SITE="${1:-}"; [ -n "$SITE" ] || { echo "usage: goat-teardown.sh <site> --plan|--services [--from N] [--to N] [--profile P]"; exit 2; }
shift
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PLAN=0; ARGS=()
while [ $# -gt 0 ]; do case "$1" in
  --plan) PLAN=1; shift ;;
  --services) ARGS+=(--services); shift ;;
  --from|--to|--profile) ARGS+=("$1" "$2"); shift 2 ;;
  *) echo "unknown arg: $1"; exit 2 ;;
esac; done

python3 "$REPO/ops/goatbuilder/teardown-plan.py" "$SITE" "${ARGS[@]}"; rc=$?
[ "$PLAN" = 1 ] && exit "$rc"
[ "$rc" = 0 ] || exit "$rc"

# Each stop's play writes what it found on the host to this file
# (ops/ansible/tasks/teardown-result.yml): `stopped`, or `nothing to stop` for
# a component never deployed there.
GOAT_TEARDOWN_RESULT="$(mktemp "${TMPDIR:-/tmp}/goat-teardown-result.XXXXXX")" || exit 1
export GOAT_TEARDOWN_RESULT
trap 'rm -f "$GOAT_TEARDOWN_RESULT"' EXIT
n=0; stopped=0; absent=0; unreported=0
while IFS= read -r cmd; do
  [ -n "$cmd" ] || continue
  n=$((n+1)); t0=$(date +%s)
  : > "$GOAT_TEARDOWN_RESULT"
  printf '\n--- teardown step %s ---\n    %s\n' "$n" "$cmd"
  eval "$cmd"; rc=$?
  printf '    took %ss (exit %s)\n' "$(( $(date +%s) - t0 ))" "$rc"
  if [ "$rc" != 0 ]; then
    echo "  !! teardown step $n FAILED (exit $rc). Steps before it are down; nothing after it ran. Fix, then re-run — every stop is idempotent." >&2
    exit "$rc"
  fi
  case "$(cat "$GOAT_TEARDOWN_RESULT")" in
    stopped) stopped=$((stopped+1)) ;;
    "nothing to stop") absent=$((absent+1)); echo "    nothing of it is on the host (never deployed there); nothing stopped, no teardown.jsonl row" ;;
    *) unreported=$((unreported+1)) ;;
  esac
done < <(python3 "$REPO/ops/goatbuilder/teardown-plan.py" "$SITE" "${ARGS[@]}" --commands)
echo "teardown of $SITE: $n step(s) run, volumes kept: $stopped took a component down, $absent found nothing of theirs on the host, $unreported reported nothing of the host."
