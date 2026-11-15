#!/usr/bin/env bash
# ops/ci/check-no-agent-references.sh — opsec guard, sibling of
# check-no-identity-spillage.sh.
#
# Refuses agent-tooling references in committed files outside the small set of
# paths that exist FOR that tooling. The repository is intended to split into an
# open-source and a private implementation repo; every agent reference that
# leaks into ordinary docs, code or history is scrub work later, and history
# cannot be scrubbed at all.
#
# Patterns guarded:
#   - claude.ai URLs of any shape (session links are the common one)
#   - Claude-Session:      commit trailer
#   - anthropic.com URLs
#
# Allowed locations (these exist for the tooling and are expected to name it):
#   - CLAUDE.md, **/CLAUDE.md          agent instruction files
#   - .claude/                          agent config, skills, hooks, worktrees
#   - docs/claudeoverflow.md            the CLAUDE.md overflow narrative
#   - ops/work/                         the multi-agent session bus
#   - docs/multi-agent/                 multi-agent substrate docs
#   - docs/adr/0013-*, docs/adr/0017-*  the ADRs that decided the pilot
#   - docs/adr/1018-augmented-design-driven-development.md
#                                       the ADR that names the design tool it adopted
#   - docs/design-assets/, docs/design-system/_origin/
#   - ops/ci/check-no-agent-references.sh + .github/workflows/agent-references.yml
#                                       (the guard itself necessarily names the patterns)
#   - ops/secwatch/internal/analysis/claude.go, ops/scripts/translate-{ftl,po}.py
#                                       (real integrations; see allowed() for why)
#
# NOT guarded here: the words "agent" or "assistant" in ordinary prose, which
# are legitimate architecture vocabulary. This targets the vendor-identifying
# strings only, which are the ones that make a scrub hard.
#
# Usage:
#   ops/ci/check-no-agent-references.sh             # scan tracked tree (fails on new hits)
#   ops/ci/check-no-agent-references.sh --staged    # scan staged diff only
#   ops/ci/check-no-agent-references.sh --census    # report every hit, never fail
#   ops/ci/check-no-agent-references.sh path/...    # scan specific paths
#
#   AGENT_REF_RANGE_BASE=<sha> AGENT_REF_RANGE_HEAD=<ref> ...  path/...
#     scan the ADDED LINES of base...head for those paths, whatever HEAD is
#     checked out (CI: base checkout, head fetched as data).
#
# Exit: 0 clean (or --census), 1 on a disallowed reference.

set -uo pipefail

# A markdown LINK into the agent-memory directory is the fourth form, added
# after three turned up in docs/ at once. It is two faults, not one:
# the target is not in the repository, so the link is permanently dangling and
# lands in the doc-links known-debt baseline where it is ignored forever; and it
# names a private memory filename in a repository heading for open-source.
#
# Matched as the LINK form `](...claude/projects...)` on purpose, not as the
# bare path: several docs discuss that directory in prose (ADRs 0013 / 0017 /
# 0839, docs/multi-agent/), and those mentions are correct and should stay. What
# is never correct is pointing a reader at a file that is not here.
PATTERN='claude\.ai|Claude-Session:|anthropic\.com|\]\([^)]*\.claude/projects'

allowed() {
  case "$1" in
    CLAUDE.md|*/CLAUDE.md) return 0 ;;
    .claude/*|*/.claude/*) return 0 ;;
    docs/claudeoverflow.md) return 0 ;;
    ops/work/*) return 0 ;;
    docs/multi-agent/*) return 0 ;;
    docs/adr/0013-*|docs/adr/0017-*) return 0 ;;
    # ADR 1018 records the decision to author design specs in an AI design
    # tool, and names that tool's site as the one the first instance used. A
    # product site, not a session link or a trailer, and an ADR is never
    # rewritten, so the record stays as it was decided. This one file only.
    docs/adr/1018-augmented-design-driven-development.md) return 0 ;;
    docs/design-assets/*) return 0 ;;
    docs/design-system/_origin/*) return 0 ;;
    ops/ci/check-no-agent-references.sh) return 0 ;;
    .github/workflows/agent-references.yml) return 0 ;;
    # pr-readiness.sh runs this same commit-message check locally, pre-push
    # (goat #988 follow-up 1), so it necessarily names the patterns too, same
    # reason as this script's own exemption above. Its bats test fixture
    # plants a literal AI co-author
    # trailer to prove the check actually fires — a fixture, not a leak.
    ops/scripts/pr-readiness.sh) return 0 ;;
    tests/pr-readiness.bats) return 0 ;;
    # Real product integrations that call the vendor API on purpose. These are
    # NOT leaks and must stay maintainable — blocking edits to them would make
    # this guard a tax on working code. They need a DECISION at open/private
    # split time (does the open repo carry that dependency?), not a scrub.
    # Tracked deliberately here so the list is visible rather than implicit.
    ops/secwatch/internal/analysis/claude.go) return 0 ;;
    ops/scripts/translate-ftl.py) return 0 ;;
    ops/scripts/translate-po.py) return 0 ;;
    # The knowledge-pack user guide tells a reader where to sign in to use the
    # pack with an AI assistant; naming the product's site is its purpose, not
    # a session link or an authorship trace.
    docs/briefs/knowledge-pack/*-guide.html|docs/briefs/knowledge-pack/*-guide.pdf) return 0 ;;
    *) return 1 ;;
  esac
}

MODE="tree"; CENSUS=0; PATHS=()
for a in "$@"; do
  case "$a" in
    --staged)  MODE="staged" ;;
    --census)  CENSUS=1 ;;
    *)         PATHS+=("$a"); MODE="paths" ;;
  esac
done

# git grep does the scan in one pass; a per-file loop over the whole tree is
# minutes rather than seconds on a repo this size.
case "$MODE" in
  staged) CAND=$(git diff --cached --name-only --diff-filter=ACM) ;;
  paths)  CAND=$(printf '%s\n' "${PATHS[@]}") ;;
  *)      CAND="" ;;
esac

# In paths mode (how CI invokes this) scan ADDED LINES ONLY, not whole files.
#
# WHY. Scanning whole files refuses a pull request for content it never wrote:
# 13 tracked files carry legacy references, including docs/project/in-flight.md,
# docs/project/implementation-plan.md and docs/design/design-system.md -- three
# of the most-edited documents in the repo. Once this check became a REQUIRED
# status check that stopped being a red badge and became a hard block, and the
# only ways past it were to scrub someone else's legitimate provenance or to
# merge with --admin. Both defeat the guard.
#
# Deriving the base here ALSO fixes a second failure mode independently: the
# workflow computes changed files against the PINNED pull_request.base.sha, so a
# branch that merges newer main commits gets main's own files scanned as its own.
# Those files contribute zero ADDED lines against the true merge-base, so they
# now pass rather than failing on content the branch never touched.
#
# Falls back to whole-file scanning when no base can be resolved (local runs
# with an explicit path argument), so the CLI contract is unchanged.
# The range is settable from outside, and CI sets it. The workflow checks out
# the BASE commit and fetches the pull request head as data (see
# .github/workflows/agent-references.yml for why), so "HEAD" here is the base,
# not the branch, and no merge-base search against HEAD can find the branch's
# additions. Previously the workflow worked around that by copying the
# head-side files into a scratch directory outside any git tree and running
# this script there -- where every git call fails, resolve_base returns 1, and
# the scan silently falls back to WHOLE FILES. That is the exact false positive
# the added-lines mode was written to remove (#625), and it refused #1027 for
# three documents' pre-existing provenance lines. With the two
# variables set, the script diffs the real range inside the real repository.
RANGE_HEAD="${AGENT_REF_RANGE_HEAD:-HEAD}"

resolve_base() {
  if [ -n "${AGENT_REF_RANGE_BASE:-}" ]; then
    # git's three-dot diff below takes the merge-base itself, so a pinned base
    # sha that the branch has since merged is fine here.
    printf '%s\n' "$AGENT_REF_RANGE_BASE"
    return 0
  fi
  # Pick the NEWEST valid merge-base among the candidate base refs.
  #
  # Two remotes exist in working clones: the primary and a mirror that lags it.
  # CI has only "origin". Taking the first match would sometimes resolve against
  # the LAGGING mirror, and a stale base makes commits already on the primary
  # look like this branch's additions -- reintroducing exactly the false
  # positive this change exists to remove. So collect every candidate and keep
  # the one that is not an ancestor of another.
  local ref remote mb best=""
  for ref in "${GITHUB_BASE_REF:-}" main master; do
    [ -n "$ref" ] || continue
    for remote in origin usareur-af; do
      git rev-parse --verify --quiet "$remote/$ref" >/dev/null 2>&1 || continue
      mb=$(git merge-base "$remote/$ref" "$RANGE_HEAD" 2>/dev/null) || continue
      [ -n "$mb" ] || continue
      if [ -z "$best" ] || git merge-base --is-ancestor "$best" "$mb" 2>/dev/null; then
        best="$mb"
      fi
    done
  done
  [ -n "$best" ] || return 1
  printf '%s\n' "$best"
}

added_lines() {
  # Added lines for one path, base...range-head, minus the +++ file header.
  git diff "$1...$RANGE_HEAD" -- "$2" 2>/dev/null | grep '^+' | grep -v '^+++'
}

# Whether the path exists on the range head. In line-scoped mode the working
# tree may be the BASE checkout (CI), where a file the branch ADDED does not
# exist on disk; testing -f there would skip exactly the new files.
exists_at_head() {
  if [ "${LINE_SCOPED:-0}" = "1" ]; then
    git cat-file -e "$RANGE_HEAD:$1" 2>/dev/null
  else
    [ -f "$1" ]
  fi
}

if [ "$MODE" = "tree" ]; then
  FILES=$(git grep -lIE "$PATTERN" -- . 2>/dev/null)
elif [ "$MODE" = "paths" ] && BASE=$(resolve_base); then
  echo "scanning ADDED LINES only, ${BASE:0:12}...${RANGE_HEAD}"
  FILES=$(printf '%s\n' "$CAND" | while IFS= read -r f; do
    [ -n "$f" ] || continue
    added_lines "$BASE" "$f" | grep -qIE "$PATTERN" && printf '%s\n' "$f"
  done)
  LINE_SCOPED=1
else
  FILES=$(printf '%s\n' "$CAND" | while IFS= read -r f; do
    [ -n "$f" ] && [ -f "$f" ] && grep -lIEq "$PATTERN" "$f" 2>/dev/null && printf '%s\n' "$f"
  done)
fi

hits=0
allowed_hits=0
while IFS= read -r f; do
  [ -n "$f" ] || continue
  exists_at_head "$f" || continue
  if allowed "$f"; then
    allowed_hits=$((allowed_hits + 1))
    [ "$CENSUS" = "1" ] && printf '  allowed  %s\n' "$f"
    continue
  fi
  hits=$((hits + 1))
  printf '  REFUSED  %s\n' "$f"
  if [ "${LINE_SCOPED:-0}" = "1" ]; then
    added_lines "$BASE" "$f" | grep -E "$PATTERN" | head -3 | sed 's/^/             /'
  else
    grep -nE "$PATTERN" "$f" 2>/dev/null | head -3 | sed 's/^/             /'
  fi
done <<< "$FILES"

if [ "$CENSUS" = "1" ]; then
  printf 'census: %d file(s) outside the allowlist, %d inside it\n' "$hits" "$allowed_hits"
  exit 0
fi

if [ "$hits" -gt 0 ]; then
  cat <<'EOF'

refused: agent-tooling references in files that are not for the tooling.

These make the open-source scrub harder, and anything committed to history
cannot be scrubbed later at all. Remove the reference, or if the file genuinely
exists for the agent tooling, add its path to allowed() in this script.

Most common cause: a session URL appended to a PR body or a commit trailer.
Neither belongs in this repository.
EOF
  exit 1
fi

echo "ok: no agent-tooling references outside the allowlist"
