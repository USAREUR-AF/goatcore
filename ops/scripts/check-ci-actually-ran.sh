#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# check-ci-actually-ran.sh — did CI RUN, or is it merely not failing?
#
# WHY THIS EXISTS. GitHub Actions was disabled repository-wide for ~40
# minutes . Every merge in that window took ZERO workflow runs —
# not failed runs, no runs — while the PRs showed green and the merges
# succeeded. It let a misspelling this repo's own linter rejects onto main, and
# an ADR whose INDEX status disagreed with its file.
#
# The reason nobody noticed for forty minutes is that a MISSING check and a
# PENDING check render identically everywhere an operator looks:
#
#   gh pr checks <n>                    -> "no checks reported"
#                                          (also what it prints 2s after a push)
#   gh pr view --json statusCheckRollup -> []   AN EMPTY ARRAY, NOT AN ERROR
#
# so every wrapper that counts failures counts ZERO and concludes green. Branch
# protection did not object either. The absence of evidence renders identically
# to the evidence of absence.
#
# WHY THIS IS NOT A GITHUB ACTION. A workflow cannot detect that workflows are
# not running. Any guard for this class has to execute somewhere Actions does
# not — an operator's shell before a merge, or a mgmt-host timer. That is the
# same reasoning as the live-state guards: the thing being checked must not be
# the thing doing the checking.
#
# usage:
#   ops/scripts/check-ci-actually-ran.sh                  # repo-wide posture
#   ops/scripts/check-ci-actually-ran.sh --pr 2689        # gate one merge
#   ops/scripts/check-ci-actually-ran.sh --ref main --last 5
#
# exit 0 = CI demonstrably ran. exit 1 = it did not. exit 2 = could not tell,
# which is NOT the same as pass and is reported as its own outcome.
set -uo pipefail

# The repository this checkout releases from (repo.source in
# ops/goatbuilder/goatbuilder.yaml), read with project-setting.sh.
REPO="${REPO:-$("$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/project-setting.sh" repo source 2>/dev/null)}"
[ -n "$REPO" ] || { echo "check-ci-actually-ran: no repository: set REPO=<owner>/<name>, or declare repo.source in ops/goatbuilder/goatbuilder.yaml" >&2; exit 2; }
PR=""; REF="main"; LAST=3
while [ $# -gt 0 ]; do
  case "$1" in
    --pr) PR="$2"; shift 2 ;;
    --ref) REF="$2"; shift 2 ;;
    --last) LAST="$2"; shift 2 ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

command -v gh >/dev/null || { echo "gh not installed — cannot tell." >&2; exit 2; }


# starved_job — did a FAILING check on this PR execute zero steps?
#
# Budget starvation creates the check suite and starts nothing: status=completed,
# conclusion=failure, steps=[], runner empty. An emptiness test passes that
# straight through because the rollup is FULL. Probing the first check overall
# is not enough — a PR that was closed and reopened carries old passed suites
# alongside new starved ones, so this looks at a FAILING one specifically.
starved_job() {
  local pr="$1" url jid steps
  url=$(gh pr view "$pr" --repo "$REPO" --json statusCheckRollup \
        --jq '[.statusCheckRollup[]|select((.conclusion//.state)=="FAILURE" and .detailsUrl)][0].detailsUrl' 2>/dev/null)
  [ -n "$url" ] || return 1
  jid=$(gh api "repos/${REPO}/actions/runs/$(echo "$url" | grep -oE 'runs/[0-9]+' | cut -d/ -f2)/jobs" \
        --jq '.jobs[0].id' 2>/dev/null)
  [ -n "$jid" ] || return 1
  steps=$(gh api "repos/${REPO}/actions/jobs/${jid}" --jq '.steps|length' 2>/dev/null)
  [ "${steps:-x}" = "0" ]
}

rc=0

# ── 1. is the machine switched on at all ────────────────────────────────────
# The single call that would have settled F-348 in seconds and was never made.
# Checked FIRST because everything below is meaningless if this is false.
perm=$(gh api "repos/${REPO}/actions/permissions" --jq '.enabled' 2>/dev/null)
case "$perm" in
  true)  echo "OK    actions enabled repo-wide" ;;
  false) echo "FAIL  ACTIONS ARE DISABLED REPO-WIDE."
         echo "      Every merge right now takes zero workflow runs and shows green."
         echo "      Re-enable: gh api -X PUT repos/${REPO}/actions/permissions -f enabled=true"
         rc=1 ;;
  *)     echo "WARN  could not read actions/permissions (got '${perm:-empty}')"
         [ $rc -eq 0 ] && rc=2 ;;
esac

# ── 2. one PR, before merging it ────────────────────────────────────────────
if [ -n "$PR" ]; then
  n=$(gh pr view "$PR" --repo "$REPO" --json statusCheckRollup \
        --jq '[.statusCheckRollup[]]|length' 2>/dev/null)
  f=$(gh pr view "$PR" --repo "$REPO" --json statusCheckRollup \
        --jq '[.statusCheckRollup[]|select((.conclusion//.state)=="FAILURE")]|length' 2>/dev/null)
  if [ -z "$n" ]; then
    echo "WARN  #${PR}: could not read the rollup"; [ $rc -eq 0 ] && rc=2
  elif [ "$n" = "0" ]; then
    # THE POINT OF THIS SCRIPT. Zero checks is not zero failures.
    echo "FAIL  #${PR}: the check rollup is EMPTY — no workflow ran."
    echo "      This is indistinguishable from 'green' to anything counting failures."
    rc=1
  elif [ "${f:-0}" != "0" ] && starved_job "$PR"; then
    # ALL of them red is the STARVED signature, not a broken diff. When the
    # Actions budget is exhausted GitHub creates every check suite and starts
    # none of them: the rollup is FULL and entirely failing, so an emptiness
    # test passes it straight through and the author goes hunting a defect that
    # does not exist. Three sessions lost time to this before
    # anyone read the annotation.
    #
    # The cheap distinguishing signal is steps=0 with no runner on the job —
    # cheaper than the annotations endpoint and on the same id. A check that is
    # structurally incapable of failing ("PR base must be main" on a PR whose
    # base IS main, DCO on a signed commit) going red is the human-readable
    # version of the same smell.
    jid=$(gh api "repos/${REPO}/actions/runs/$(gh pr view "$PR" --repo "$REPO"             --json statusCheckRollup --jq '[.statusCheckRollup[]|select(.detailsUrl)][0].detailsUrl' 2>/dev/null             | grep -oE 'runs/[0-9]+' | cut -d/ -f2)/jobs" --jq '.jobs[0].id' 2>/dev/null)
    steps=$(gh api "repos/${REPO}/actions/jobs/${jid}" --jq '.steps|length' 2>/dev/null)
    if [ "${steps:-x}" = "0" ]; then
      echo "FAIL  #${PR}: STARVED — ${f} of ${n} checks red and ZERO steps executed."
      echo "      The Actions budget is exhausted; nothing ran. This is NOT your diff."
      echo "      Confirm: gh api repos/${REPO}/actions/jobs/${jid} --jq .steps"
      echo "      Raising the budget unblocks every open PR, not just this one."
    else
      echo "FAIL  #${PR}: all ${n} checks failed (steps=${steps:-unknown}) — check the diff"
    fi
    rc=1
  elif [ "${f:-0}" != "0" ]; then
    echo "FAIL  #${PR}: ${f} of ${n} checks failed"; rc=1
  else
    echo "OK    #${PR}: ${n} checks present, none failed"
  fi
fi

# ── 3. recent history on the branch ─────────────────────────────────────────
# A commit on main with no check runs is a commit nothing verified. Reported per
# commit rather than in aggregate: one bare commit in a run of green ones is
# exactly the shape the outage produced.
echo "      last ${LAST} commit(s) on ${REF}:"
# Resolve REF as a branch first, then as a bare rev, so a specific commit can be
# audited directly ("did THIS merge get verified?") and not only branch tips.
range="origin/${REF}"
git rev-parse --verify -q "$range" >/dev/null 2>&1 || range="$REF"
for sha in $(git rev-list -n "$LAST" "$range" 2>/dev/null); do
  # check-SUITES, not check-runs: the runs endpoint 404s under a token without
  # the checks scope, and `gh api` prints the 404 JSON to STDOUT. The first
  # version of this loop took that blob as the count and printed "ok" — a guard
  # reading green on an error, which is the exact class it exists to catch.
  # Hence the numeric test below: anything not a plain integer is UNKNOWN.
  short="${sha:0:9}"
  full=$(git log -1 --format=%s "$sha" 2>/dev/null)
  subj=$(printf '%.46s' "$full")
  # VERIFY VIA THE PULL REQUEST. Most guards are pull_request-only by design
  #, so a merge commit legitimately
  # carries ZERO check suites and checking the commit reports every
  # workflow-only change as unverified. What matters is whether the CHANGE was
  # checked before it landed, which lives on its PR. Squash merges carry the
  # number as "(#1234)" at the END of the subject — read it off the FULL
  # subject, never the truncated display copy.
  pr=$(printf '%s' "$full" | grep -oE '\(#[0-9]+\)$' | tr -dc '0-9')
  if [ -n "$pr" ]; then
    c=$(gh pr view "$pr" --repo "$REPO" --json statusCheckRollup \
          --jq '[.statusCheckRollup[]]|length' 2>/dev/null)
  else
    c=$(gh api "repos/${REPO}/commits/${sha}/check-suites" --jq '.total_count' 2>/dev/null)
  fi
  case "$c" in
    ''|*[!0-9]*)
      printf '        WARN  %s  unreadable (%s)  %s\n' "$short" "$(printf '%.24s' "${c:-empty}")" "$subj"
      [ $rc -eq 0 ] && rc=2
      continue ;;
  esac
  if false; then :
  elif [ "$c" = "0" ]; then
    printf '        FAIL  %s  NO CHECKS%s  %s\n' "$short" \
      "$([ -n "$pr" ] && printf ' (PR #%s)' "$pr" || printf ' (no PR)')" "$subj"
    rc=1
  else
    printf '        ok    %s  %-3s checks   %s\n' "$short" "$c" "$subj"
  fi
done

case $rc in
  0) echo "PASS — CI demonstrably ran." ;;
  1) echo "FAIL — something merged or is about to merge without verification." ;;
  2) echo "UNKNOWN — could not establish that CI ran. This is NOT a pass." ;;
esac
exit $rc
