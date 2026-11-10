#!/usr/bin/env bash
# cross-env-apply.sh — put ops/goatnets/cross-env.yaml on every relay-health box.
#
# The goatnet overview on each box is rendered from group_vars at deploy time.
# Changing the roster in the repo changes nothing live until every box is
# redeployed, so a removed goatnet stays on the overviews until then. This
# script is that redeploy, for all boxes at once, and it is the step
# goat-standup.sh runs
#
# Usage:
#   ops/goatnets/cross-env-apply.sh [--plan] [--check] [--require <goatnet>]
#     --plan      print the box list and the command per box; run nothing
#     --check     ansible --check (NEVER --diff: the role stages secrets)
#     --require   refuse unless <goatnet> has a cross-env.yaml entry (standup)
#     --absent    refuse while <goatnet> still has a cross-env.yaml entry
#                 (teardown: remove its row by pull request first; this then
#                 redeploys every box so the overviews drop it)
#
# Order of operations it enforces:
#   1. render-cross-env.py --check — the committed generated files match
#      cross-env.yaml, and the roster matches fleet-pins + goatnet-runners.
#      A dirty or stale tree is refused here rather than deployed.
#   2. relay-health-cloud-deploy.yml against each box through its inventory,
#      via ops/scripts/ansible-preflight.sh (refuses a stale trunk).
#
# Secrets: the plays read GOAT_CROSS_FLEET_TOKEN and each box's own secrets from
# the environment. Run it where direnv has loaded the operator envrc, or let
# ansible-preflight.sh load the master worktree's .envrc.local.d.
#
# Environment:
#   GOAT_AWS_PROFILE   aws-vault profile (default: usareur-af). The boxes are
#                      reached over SSM.
set -uo pipefail

REPO="$(cd "$(dirname "$0")/../.." && pwd)"
PLAN=0; CHECK=(); REQUIRE=""; ABSENT=""
while [ $# -gt 0 ]; do
  case "$1" in
    --plan) PLAN=1 ;;
    --check) CHECK=(--check) ;;
    --require) REQUIRE="${2:?--require needs a goatnet}"; shift ;;
    --absent) ABSENT="${2:?--absent needs a goatnet}"; shift ;;
    -h|--help) sed -n '2,30p' "$0"; exit 0 ;;
    *) echo "cross-env-apply: unknown argument $1" >&2; exit 2 ;;
  esac
  shift
done
PROFILE="${GOAT_AWS_PROFILE:-usareur-af}"
RENDER="$REPO/ops/goatnets/render-cross-env.py"

if [ -n "$REQUIRE" ]; then
  python3 "$RENDER" --require "$REQUIRE" || exit 1
fi
if [ -n "$ABSENT" ] && python3 "$RENDER" --require "$ABSENT" >/dev/null 2>&1; then
  echo "cross-env-apply: refusing — ops/goatnets/cross-env.yaml still lists $ABSENT. Remove its row (and re-render with render-cross-env.py) by pull request, then re-run; every relay-health box is then redeployed without it." >&2
  exit 1
fi
python3 "$RENDER" --check || {
  echo "cross-env-apply: refusing to deploy — the generated relay-health config is stale or the roster is inconsistent (above)." >&2
  exit 1
}

BOXES=()
while IFS= read -r l; do [ -n "$l" ] && BOXES+=("$l"); done < <(python3 "$RENDER" --boxes)
[ "${#BOXES[@]}" -gt 0 ] || { echo "cross-env-apply: no relay-health boxes declared; nothing to deploy"; exit 0; }

rc_all=0
for line in "${BOXES[@]}"; do
  group="${line%% *}"; inv="${line#* }"
  cmd=(aws-vault exec "$PROFILE" --no-session -- ../scripts/ansible-preflight.sh
       -i "$inv" relay-health-cloud-deploy.yml -e "target=$group" ${CHECK[@]+"${CHECK[@]}"})
  printf '\n=== %s (%s) ===\n    cd ops/ansible && OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES %s\n' "$group" "$inv" "${cmd[*]}"
  [ "$PLAN" = 1 ] && continue
  ( cd "$REPO/ops/ansible" && OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES "${cmd[@]}" )
  rc=$?
  if [ "$rc" -ne 0 ]; then
    echo "cross-env-apply: $group FAILED (exit $rc); continuing with the remaining boxes" >&2
    rc_all=1
  fi
done

if [ "$PLAN" = 1 ]; then
  echo; echo "PLAN only. Remove --plan to deploy."
  exit 0
fi
[ "$rc_all" -eq 0 ] && echo "cross-env-apply: every box deployed. Open each box's /goatnets.html and read the footer's registry commit."
exit "$rc_all"
