#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# ci-run-volume.sh — how many workflow runs a day, and which workflows.
#
# WHY THIS EXISTS. Run volume went from 2,173/day to 4,787/day over two
# weeks and nothing reported it. The first signal was a bill at roughly six times
# the historical monthly spend; the second was the Actions budget running out and
# failing every check on every open PR, which reads as a broken change rather
# than a broken wallet.
#
# The trend was visible in the API the whole time. Nothing asked.
#
# NOT a GitHub Action, deliberately: when the budget is exhausted, a workflow
# does not run, so a workflow cannot be what tells you the budget is exhausted.
# Same reasoning as check-ci-actually-ran.sh — the thing being measured must not
# be the thing doing the measuring.
#
# usage:
#   ops/scripts/ci-run-volume.sh                 # last 7 days
#   ops/scripts/ci-run-volume.sh --days 14
#   ops/scripts/ci-run-volume.sh --top 20        # busiest workflows, one day
set -uo pipefail

# The repository this checkout releases from (repo.source in
# ops/goatbuilder/goatbuilder.yaml), read with project-setting.sh.
REPO="${REPO:-$("$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/project-setting.sh" repo source 2>/dev/null)}"
[ -n "$REPO" ] || { echo "ci-run-volume: no repository: set REPO=<owner>/<name>, or declare repo.source in ops/goatbuilder/goatbuilder.yaml" >&2; exit 2; }
DAYS=7; TOP=0
while [ $# -gt 0 ]; do
  case "$1" in
    --days) DAYS="$2"; shift 2 ;;
    --top)  TOP="$2";  shift 2 ;;
    --repo) REPO="$2"; shift 2 ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done
command -v gh >/dev/null || { echo "gh not installed" >&2; exit 2; }

echo "workflow run volume — ${REPO}"
echo
prev=""
for i in $(seq "$DAYS" -1 0); do
  d=$(date -u -v-"${i}"d +%Y-%m-%d 2>/dev/null || date -u -d "${i} days ago" +%Y-%m-%d)
  n=$(gh api "repos/${REPO}/actions/runs?per_page=1&created=${d}" --jq '.total_count' 2>/dev/null)
  case "$n" in ''|*[!0-9]*) printf '  %s  unreadable\n' "$d"; continue ;; esac
  # Trend arrow against the previous day, so a doubling is visible without
  # anyone having to hold last week's number in their head — which is the
  # failure this tool exists for.
  arrow="  "
  if [ -n "$prev" ]; then
    if   [ "$n" -gt $(( prev * 130 / 100 )) ]; then arrow="^^"
    elif [ "$n" -gt "$prev" ];                 then arrow="^ "
    elif [ "$n" -lt $(( prev * 70 / 100 )) ];  then arrow="vv"
    fi
  fi
  printf '  %s  %6d  %s\n' "$d" "$n" "$arrow"
  prev="$n"
done

if [ "$TOP" != "0" ]; then
  d=$(date -u +%Y-%m-%d)
  echo
  echo "busiest workflows on ${d}:"
  for pg in 1 2 3 4 5; do
    gh api "repos/${REPO}/actions/runs?per_page=100&created=${d}&page=${pg}" \
      --jq '.workflow_runs[].name' 2>/dev/null
  done | sort | uniq -c | sort -rn | head -"$TOP" | sed 's/^/    /'
fi

echo
echo "context: 119 workflow files; 38 fire on both pull_request and push (publishers),"
echo "         81 on pull_request only. A PR costs roughly 1,200-1,400 job-seconds."
echo "         F-354 has the arithmetic; F-355 has why none of them can block a merge."
