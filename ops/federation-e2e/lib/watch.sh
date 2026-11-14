#!/usr/bin/env bash
# watch.sh — run the matrix on every new head of the federation branches, forever.
#
#   lib/watch.sh --out DIR [--notes FILE] [--interval S] [--jobs N] [--once]
#                [--pattern GLOB]... [--integration REF] [--reverts]
#                [--first "REF REF..."] [--scenario ID,ID]
#
# --first measures these refs (branch names under origin/) in this order before anything
# else, each round. --scenario limits every measurement to those scenarios; the subset is
# part of the fingerprint, so a head measured on a subset is measured again on the full set.
#
# Each round: git fetch; the baseline (origin/main) first, then the integration branch,
# then a preview ref that merges every origin/ws/* head onto the integration branch
# (branches that pass alone can fail together), then every remote branch matching a
# pattern, newest first. A head already measured (DIR/<name>/<sha12>.md exists) is
# skipped. Results: DIR/<name>/<sha12>.md and -<avenue>.json, DIR/latest.md.
# A scenario that passes on main and not on a branch is appended to --notes.
#
# --reverts also measures, for each module on the integration branch since main (each
# first-parent commit, squash or merge), the integration head with that module reverted
# (name integration-revert-<sha12>) and the integration head before it
# (integration-before-<sha12>); scenario module-revert compares the two.
set -euo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"
FE2E_HOME="${FE2E_HOME:-$HOME/.cache/fe2e}"
OUT=""; NOTES=""; INTERVAL=600; JOBS=2; ONCE=0; REVERTS=0; FIRST=""; SCEN="all"
INTEGRATION="origin/integration/federation-turnkey"
PATTERNS=()
while [ $# -gt 0 ]; do
  case "$1" in
    --out) OUT="$2"; shift 2 ;;
    --notes) NOTES="$2"; shift 2 ;;
    --interval) INTERVAL="$2"; shift 2 ;;
    --jobs) JOBS="$2"; shift 2 ;;
    --once) ONCE=1; shift ;;
    --pattern) PATTERNS+=("$2"); shift 2 ;;
    --integration) INTEGRATION="$2"; shift 2 ;;
    --reverts) REVERTS=1; shift ;;
    --first) FIRST="$2"; shift 2 ;;
    --scenario) SCEN="$2"; shift 2 ;;
    *) echo "watch: unknown argument $1" >&2; exit 2 ;;
  esac
done
[ -n "$OUT" ] || { echo "watch: --out DIR is required" >&2; exit 2; }
[ ${#PATTERNS[@]} -gt 0 ] || PATTERNS=("origin/ws/*" "origin/ws-*" "origin/fed/*" "origin/track/federate-in-goatbuilder" "origin/fix/federation-*" "origin/test/federation-*")
mkdir -p "$OUT"
log() { printf '%s %s\n' "$(date -u +%H:%M:%SZ)" "$*" >&2; }

# What a result depends on besides the ref: the catalog, the units and the engine. A head is
# measured again when this changes, so every row of latest.md comes from the same harness.
fingerprint() { { cat "$HERE"/scenarios/*.yaml "$HERE"/units/*.sh "$HERE"/lib/engine.sh "$HERE"/lib/unit.sh "$HERE"/lib/verdict.py; echo "$SCEN"; } | sha256sum | cut -c1-12; }
measured() { [ -f "$OUT/$1/${2:0:12}.md" ] && [ "$(cat "$OUT/$1/${2:0:12}.fp" 2>/dev/null)" = "$(fingerprint)" ]; } # $1 name, $2 sha

measure() { # $1 name, $2 ref or sha, [$3...] extra env run arguments
  local name="$1" ref="$2"; shift 2
  local sha; sha="$(git -C "$REPO" rev-parse -q --verify "$ref^{commit}")" || { log "no commit for $ref"; return 0; }
  measured "$name" "$sha" && return 0
  local fp; fp="$(fingerprint)" # the harness this run copies now, not whatever is here when it ends
  log "measure $name @ ${sha:0:12}"
  "$HERE/env" run --ref "$sha" --name "$name" --avenue both --scenario "$SCEN" --jobs "$JOBS" --out "$OUT" "$@" \
    > "$FE2E_HOME/watch-$(echo "$name" | tr '/' '_')-${sha:0:12}.log" 2>&1 || true
  [ -f "$OUT/$name/${sha:0:12}.md" ] || { mkdir -p "$OUT/$name"; echo "# $name @ ${sha:0:12}: the run produced no result; see $FE2E_HOME/watch-*-${sha:0:12}.log" > "$OUT/$name/${sha:0:12}.md"; }
  echo "$fp" > "$OUT/$name/${sha:0:12}.fp"
  regressions "$name" "$sha"
}

regressions() { # $1 name, $2 sha: append to the notes what passes on main and not here
  [ -n "$NOTES" ] || return 0
  [ "$1" = main ] && return 0
  local lines
  lines="$(python3 "$HERE/lib/matrix.py" --out "$OUT" | grep -F "$1@${2:0:12} " || true)"
  [ -n "$lines" ] || return 0
  { echo; echo "### fed-e2e-env: REGRESSION on $1 @ ${2:0:12} ($(date -u +%Y-%m-%dT%H:%MZ))"; echo
    echo "Passes on main, not on this head (details: fed-e2e/$1/${2:0:12}.md):"
    printf '%s\n' "$lines" | sed 's/^/- /'; } >> "$NOTES"
}

# The preview: every ws/* head merged onto the integration branch, in a scratch clone, then
# fetched into this repository as refs/fe2e/preview so that env can build it.
preview() {
  local d="$FE2E_HOME/preview" b merged="" failed=""
  rm -rf "$d"
  git clone -q --shared --no-checkout "$(git -C "$REPO" rev-parse --path-format=absolute --git-common-dir)" "$d"
  GIT_LFS_SKIP_SMUDGE=1 git -C "$d" checkout -q --detach "$(git -C "$REPO" rev-parse "$INTEGRATION")"
  for b in $(git -C "$REPO" for-each-ref --format='%(refname:short)' 'refs/remotes/origin/ws/*' 'refs/remotes/origin/ws-*'); do
    if git -C "$d" -c user.name=fe2e -c user.email=fe2e@example.org merge -q --no-edit "$(git -C "$REPO" rev-parse "$b")" >/dev/null 2>&1; then
      merged="$merged ${b#origin/}@$(git -C "$REPO" rev-parse --short=12 "$b")"
    else
      git -C "$d" merge --abort >/dev/null 2>&1 || true
      failed="$failed ${b#origin/}"
    fi
  done
  git -C "$REPO" fetch -q "$d" "+HEAD:refs/fe2e/preview"
  printf '%s\n' "$merged" > "$FE2E_HOME/preview.merged"; printf '%s\n' "$failed" > "$FE2E_HOME/preview.conflicts"
  [ -z "$failed" ] || log "preview: these did not merge cleanly and are left out:$failed"
  local m args=(); for m in $merged; do args+=(--merged "$m"); done
  measure integration-preview refs/fe2e/preview ${args[@]+"${args[@]}"}
  if [ -n "$failed" ] && [ -n "$NOTES" ] && ! grep -qF "preview conflicts:$failed" "$NOTES"; then
    printf '\n### fed-e2e-env: preview conflicts:%s (they do not merge cleanly onto %s; left out of integration-preview)\n' "$failed" "$INTEGRATION" >> "$NOTES"
  fi
}

reverts() { # each merge on the integration branch, reverted on its head
  local m d="$FE2E_HOME/revert"
  # Each module landed on the integration branch as one first-parent commit (a squash or a merge).
  for m in $(git -C "$REPO" rev-list --first-parent "origin/main..$INTEGRATION"); do
    rm -rf "$d"
    git clone -q --shared --no-checkout "$(git -C "$REPO" rev-parse --path-format=absolute --git-common-dir)" "$d"
    GIT_LFS_SKIP_SMUDGE=1 git -C "$d" checkout -q --detach "$(git -C "$REPO" rev-parse "$INTEGRATION")"
    local mainline=""; [ "$(git -C "$REPO" rev-list --parents -n1 "$m" | wc -w)" -gt 2 ] && mainline="-m 1"
    # shellcheck disable=SC2086
    if git -C "$d" -c user.name=fe2e -c user.email=fe2e@example.org revert -q --no-edit $mainline "$m" >/dev/null 2>&1; then
      git -C "$REPO" fetch -q "$d" "+HEAD:refs/fe2e/revert-${m:0:12}"
      measure "integration-revert-${m:0:12}" "refs/fe2e/revert-${m:0:12}"
      measure "integration-before-${m:0:12}" "$m^1"
    else
      log "revert of ${m:0:12} does not apply cleanly"
    fi
  done
}

# The integration branch is measured after every merge: its head is fetched and measured
# before each other ref, so a merge waits for at most one measurement.
integration_now() {
  git -C "$REPO" fetch -q origin "+refs/heads/${INTEGRATION#origin/}:refs/remotes/$INTEGRATION" 2>/dev/null || true
  measure integration "$INTEGRATION"
}

while :; do
  git -C "$REPO" fetch -q --prune origin || log "fetch failed; using the refs already here"
  measure main origin/main
  integration_now
  for b in $FIRST; do integration_now; measure "$b" "origin/$b"; done
  preview
  # shellcheck disable=SC2046 # one ref pattern per word
  for ref in $(git -C "$REPO" for-each-ref --sort=-committerdate --format='%(refname:short)' $(for p in "${PATTERNS[@]}"; do echo "refs/remotes/$p"; done)); do
    [ "$ref" = "$INTEGRATION" ] && continue
    integration_now
    measure "${ref#origin/}" "$ref"
  done
  [ "$REVERTS" = 1 ] && reverts
  python3 "$HERE/lib/matrix.py" --out "$OUT" >/dev/null || true
  [ "$ONCE" = 1 ] && break
  sleep "$INTERVAL"
done
