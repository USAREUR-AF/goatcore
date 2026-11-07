#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# ansible-preflight.sh — wrapper around `ansible-playbook` that refuses
# to run from a stale local trunk worktree.
#
# Problem this fixes
# ------------------
# `ansible-playbook` reads role defaults (image pins, blocklists, etc.)
# from cwd, NOT from origin/main. When the local trunk worktree is N
# commits behind origin, the playbook templates with stale defaults —
# silently. The pull-image task shows `ok` not `changed`, easy to miss;
# the wrong image gets templated; the rollback ledger records a false
# history entry. Hit twice in the panoscope deploy arc and
# at least once in the prior stethoscope arc. The fix lives
# at the substrate level — refuse-when-stale, not "remember to pull
# first."
#
# Usage
# -----
# Drop-in replacement for `ansible-playbook`. From `ops/ansible`:
#
#   OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES \
#     ../scripts/ansible-preflight.sh -i inventory/hosts.ini <playbook.yml>
#
# Enforcement
# -----------
# Every branch: the fresh-source check (below). On `main` or `master`
# the whole tree must also equal the trunk; a feature branch is SUPPOSED
# to carry its own commits, so it is judged only on what a deploy reads.
#
# - HEAD == origin/<branch>      → exec ansible-playbook with args.
# - HEAD ancestor of origin      → REFUSE; print the fast-forward
#                                  command operator must run first.
# - Diverged (local commits ahead OR forked) → REFUSE; list local-only
#                                  commits; "investigate before
#                                  applying."
#
# Override sentinel
# -----------------
# For the rare intentionally-stale-run case:
#
#   GOAT_SKIP_ANSIBLE_PREFLIGHT=1 ../scripts/ansible-preflight.sh ...
#
# The override appears in the command itself (not the parent shell) so
# it's deliberate and per-invocation. Stderr-prints the bypass so the
# operator can see they took the override.
#
# Fresh source (contract C3), every branch
# ----------------------------------------
# ops/scripts/fresh-source.sh runs first on every branch, detached HEAD
# included: it fetches the trunk and refuses a checkout behind it in
# ops/ansible or ops/pins (GB-064). A trunk that cannot be fetched is a
# refusal too (GB-065); an islanded site runs with GOAT_DEPLOY_OFFLINE=1
# (the drivers' --offline) and is judged against the cached trunk.
#
# Assembled tree (goatcore + overlay)
# -----------------------------------
# A tree with `.goat-assembled` at its root is a goatcore checkout at the
# commit in goatcore.ref with an environment overlay copied over it. Its
# source of record is that pair: fresh-source.sh judges it against
# goatcore.ref and the overlay's main (ops/scripts/lib/source_of_record.py),
# the safety-critical paths must be goatcore's own (an overlay copy is
# refused), there is no trunk-branch check, and secret references load from
# the overlay checkout's master worktree. Every play exports
# GOAT_DEPLOY_SOURCE_TREE, which the deploy ledger row records as
# `source_tree` (both avenues, clone or assembled).

set -euo pipefail
# Hold the Mac awake for the whole run (#2122; ops/scripts/lib/keep-awake.sh).
_keep_awake="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/keep-awake.sh"
# shellcheck source=ops/scripts/lib/keep-awake.sh
if [ -f "$_keep_awake" ]; then . "$_keep_awake"; goat_keep_awake; fi

# Every DELIBERATE refusal below prints its reason to stderr and exits 1/2/3.
# This trap catches the OTHER case: a command failing unexpectedly under
# `set -e`/pipefail, which `exit` does not trigger but a bare non-zero does.
# It existed to be needed — a role-name grep that matched nothing returned 1
# under pipefail and killed this wrapper with ZERO output, which masked a
# stale-checkout deploy. Never die mute again.
goat_preflight_internal_abort() {
  local rc="$1" line="$2"
  echo "ansible-preflight: INTERNAL ABORT (exit $rc) near line $line — a wrapper bug, not a deliberate refusal. Bypass with GOAT_SKIP_ANSIBLE_PREFLIGHT=1 while it is fixed." >&2
}
trap 'goat_preflight_internal_abort "$?" "$LINENO"' ERR

# ---------------------------------------------------------------------------
# SAFE_ARGV — this script's own argv with every `-e key=value` VALUE redacted.
#
# Every refusal below prints a bypass line so an operator can re-run the exact
# command. Those lines used to interpolate `$*`, which is the full argv — and
# an ansible-playbook argv routinely carries a live secret, because `-e
# key=value` is how a password reaches a play. this script's
# stale-tree refusal printed a Postgres superuser password to a terminal and
# into a session transcript, from the script whose whole job is to stop unsafe
# runs. A guard that leaks the thing it is guarding is worse than no guard.
#
# The `-e @file` form is safe and stays intact: the file path is not a secret
# and is what the runbooks now use (see goat-standup.sh phase 7). Only the
# inline `key=value` form is redacted, and only its value, so the bypass line
# still shows WHICH variable has to be supplied again.
__safe_argv() {
  local out=() prev="" a
  for a in "$@"; do
    if [ "$prev" = "-e" ] || [ "$prev" = "--extra-vars" ]; then
      case "$a" in
        @*) out+=("$a") ;;                      # -e @file — a path, not a secret
        *=*) out+=("${a%%=*}=<redacted>") ;;    # -e key=value — keep the key
        *)  out+=("<redacted>") ;;
      esac
    else
      out+=("$a")
    fi
    prev="$a"
  done
  printf '%s' "${out[*]}"
}
SAFE_ARGV="$(__safe_argv "$@")"


# ---------------------------------------------------------------------------
# Command-shape guards — the two ways a run does NOTHING and still exits 0.
#
# These are NOT about staleness, so they run on every path INCLUDING the
# GOAT_SKIP_ANSIBLE_PREFLIGHT bypass. A stale tree is a wrong-content problem;
# these are wrong-command problems, and the bypass is not a claim that the
# command is right.
#
# Both fail silently:
#
#   1. `--tags trust-anchors` against a refresh play that delegates to
#      site-refresh.yml, a play that does not carry the goat-zenoh role at
#      all. Zero tasks match. Ansible gathers facts, changes nothing, and
#      prints a GREEN PLAY RECAP. A run that fails by succeeding is worse than
#      one that errors, because the operator ticks the step off.
#
#   2. `-i inventory/mgmt-hosts-mesh.ini` combined with the env-door
#      inventory. env-door.sh prints "Do NOT also pass mgmt-hosts-mesh.ini"
#      because the mesh file re-overrides the transport the door just set.
#
# Opt out of these specifically with GOAT_SKIP_ARG_GUARDS=1.
# ---------------------------------------------------------------------------
goat_arg_guards() {
  [ "${GOAT_SKIP_ARG_GUARDS:-}" = "1" ] && return 0
  local -a argv=("$@") invs=()
  local tags="" i n="${#argv[@]}"
  for ((i=0; i<n; i++)); do
    case "${argv[$i]}" in
      --tags)          tags="${argv[$((i+1))]:-}" ;;
      --tags=*)        tags="${argv[$i]#--tags=}" ;;
      -i)              invs+=("${argv[$((i+1))]:-}") ;;
      --inventory)     invs+=("${argv[$((i+1))]:-}") ;;
      --inventory=*)   invs+=("${argv[$i]#--inventory=}") ;;
    esac
  done

  # --- Guard 1: contradictory inventories -----------------------------------
  local has_door=0 has_mesh=0 v
  for v in ${invs[@]+"${invs[@]}"}; do
    case "$v" in
      *doors-groupvars*|*doors.ini*) has_door=1 ;;
      *mgmt-hosts-mesh*)             has_mesh=1 ;;
    esac
  done
  if [ "$has_door" = 1 ] && [ "$has_mesh" = 1 ]; then
    cat >&2 <<EOF

ansible-preflight: REFUSE — door inventory combined with mgmt-hosts-mesh.ini.

  The door file overrides transport to 127.0.0.1:<port>; mgmt-hosts-mesh.ini
  overrides it straight back to the mesh IP, which the door exists because you
  cannot reach. Whichever loads last silently wins.

  Use the pattern env-door.sh prints:
    -i inventory/mgmt-hosts.ini -i inventory/doors-groupvars.ini <play>.yml -e target=<group>

  Bypass with GOAT_SKIP_ARG_GUARDS=1 if you know better.

EOF
    exit 4
  fi

  # --- Guard 2: --tags that matches no task ---------------------------------
  # Ask ansible itself, before running anything. Fail OPEN if --list-tasks
  # cannot answer (bad play, missing vars): that is the real run's job to
  # report, and this guard must never invent a refusal it cannot justify.
  if [ -n "$tags" ]; then
    local listing rc
    listing="$(ansible-playbook --list-tasks "$@" 2>/dev/null)" && rc=0 || rc=$?
    # Task lines are indented 6+ spaces; the `play #N` header is indented 2 and
    # also carries "TAGS:", so a bare TAGS: match would never fire. Verified
    # against real --list-tasks output both the matching and the
    # non-matching case.
    if [ "${rc:-1}" = 0 ] && ! grep -qE '^ {6,}[^ ].*TAGS:' <<<"$listing"; then
      cat >&2 <<EOF

ansible-preflight: REFUSE — --tags "$tags" matches NO task in this playbook.

  Ansible would gather facts, change nothing, and exit 0 with a green PLAY
  RECAP. That reads as success and is how a skipped step gets ticked off.

  Check the playbook actually carries the role you are tagging. A play that
  only import_playbook's another one does not necessarily reach it, and tags
  do not cross an include_role without \`apply:\`.

  See for yourself:
    ansible-playbook --list-tasks $* 

  Bypass with GOAT_SKIP_ARG_GUARDS=1.

EOF
      exit 5
    fi
  fi
  return 0
}
# Resolve repo root from this script's own location.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# An ASSEMBLED tree (ops/scripts/lib/source_of_record.py): goatcore at the commit
# in goatcore.ref with an environment overlay copied over it, marked by
# `.goat-assembled` at the root. Its git checkout is goatcore's, so the trunk-remote
# reasoning below does not apply to it: fresh-source.sh judges it against the pair
# (goatcore.ref, the overlay's main), and the operator's secret references live in
# the overlay checkout's master worktree, not in this tree's.
ASSEMBLED=0; OVERLAY_DIR=""
if [ -f "$REPO_ROOT/.goat-assembled" ]; then
  ASSEMBLED=1
  OVERLAY_DIR="$(awk -F= '$1=="overlay_dir"{print substr($0, index($0, "=") + 1); exit}' "$REPO_ROOT/.goat-assembled" || true)"
  [ -n "$OVERLAY_DIR" ] || OVERLAY_DIR="${GOAT_OVERLAY_DIR:-}"
fi

# ---------------------------------------------------------------------
# Secrets are per-LAPTOP, not per-worktree.
#
# `.envrc.local.d/*.{refs,envrc}` is gitignored, so it exists only in the master
# worktree. direnv loads it there. A play launched from a track worktree —
# which is where ADR 0013 says work happens — sees none of it, and every
# `lookup('env', ...)` in group_vars resolves to the empty string.
#
# An assert on a required key then fails a deploy from a worktree, for a
# reason that has nothing to do with the key being wrong. Roles WITHOUT such an
# assert
# quietly template an empty secret instead — the worse outcome, and the
# reason this belongs in the wrapper rather than in an operator's memory.
#
# So: load the master worktree's secret files when the current worktree
# has none. Loudly — file names only, never values.
# In an assembled tree the master worktree is the overlay checkout's.
SECRETS_FROM="$REPO_ROOT"
[ "$ASSEMBLED" = 1 ] && [ -n "$OVERLAY_DIR" ] && SECRETS_FROM="$OVERLAY_DIR"
GIT_COMMON="$(git -C "$SECRETS_FROM" rev-parse --path-format=absolute --git-common-dir 2>/dev/null || echo "")"
if [ -n "$GIT_COMMON" ]; then
  MASTER_ROOT="$(cd "$(dirname "$GIT_COMMON")" 2>/dev/null && pwd || echo "")"
  SECRET_DIR="$MASTER_ROOT/.envrc.local.d"
  # Blue/green (ADR 1203): the green store keeps its reference tree one
  # directory down, and its references start with bao://.
  if [ "${GOAT_SECRET_STORE:-1password}" = "openbao" ]; then
    SECRET_DIR="$SECRET_DIR/openbao"
    : "${GOAT_SECRET_REF_PREFIX:=bao://}"; export GOAT_SECRET_REF_PREFIX
  fi
  if [ -n "$MASTER_ROOT" ] && [ "$MASTER_ROOT" != "$REPO_ROOT" ] \
     && [ ! -d "$REPO_ROOT/.envrc.local.d" ] && [ -d "$SECRET_DIR" ]; then
    LOADED=""
    # Validate the complete file before exporting anything. References are data,
    # never sourced shell code; diagnostics must not echo rejected values.
    for _f in "$SECRET_DIR"/*.refs; do
      [ -e "$_f" ] || [ -L "$_f" ] || continue
      # Quote characters use hex escapes below: Bash 3.2 also scans this heredoc
      # while parsing the surrounding quoted command substitution.
      if ! _refs="$(python3 - "$_f" "${GOAT_SECRET_REF_PREFIX:-}" <<'PYREFS'
import pathlib, re, sys
path, prefix = pathlib.Path(sys.argv[1]), sys.argv[2]
# One grammar, scheme-dispatched (ADR 1204): op:// and bao:// may share a file.
prefixes = (prefix,) if prefix else ("op://", "bao://")
try:
    if path.is_symlink() or not path.is_file():
        raise ValueError()
    entries = {}
    for line in path.read_text().splitlines():
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        match = re.fullmatch(r'''\s*(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)=([\x27\x22]?)([A-Za-z0-9:/_.@%+\-]+)\2\s*(?:#.*)?''', line)
        scheme = next((p for p in prefixes if match and match[3].startswith(p)), None)
        if not match or not scheme or len(match[3]) <= len(scheme) or match[1] in entries:
            raise ValueError()
        entries[match[1]] = match[3]
    for name, reference in entries.items():
        print(name + "=" + reference)
except (OSError, ValueError, UnicodeError):
    sys.exit(1)
PYREFS
)"; then
        echo "ansible-preflight: refused $(basename "$_f"): invalid reference file; values withheld" >&2
        exit 2
      fi
      while IFS='=' read -r _name _ref; do
        [ -n "$_name" ] || continue
        _current="$(printenv "$_name" || true)"
        # A surrounding with-secrets invocation may already have resolved it.
        if [ -n "$_current" ] && [[ "$_current" != op://* ]] && [[ "$_current" != bao://* ]] \
           && { [ -z "${GOAT_SECRET_REF_PREFIX:-}" ] || [[ "$_current" != "${GOAT_SECRET_REF_PREFIX}"* ]]; }; then
          continue
        fi
        export "$_name=$_ref"
        unset GOAT_SECRETS_RESOLVED
      done <<< "$_refs"
      LOADED="$LOADED $(basename "$_f")"
    done
    unset _refs _name _ref _current
    # During migration, legacy values win as they do in .envrc. References-only
    # mode must never source these files, including on the injector's re-entry.
    if [ "${GOAT_REFERENCES_ONLY:-0}" != "1" ]; then
      set +u
      for _f in "$SECRET_DIR"/*.envrc; do
        [ -e "$_f" ] || continue
        set -a
        # shellcheck disable=SC1090
        . "$_f" 2>/dev/null || echo "ansible-preflight: WARN — could not source $(basename "$_f")" >&2
        set +a
        LOADED="$LOADED $(basename "$_f")"
      done
      set -u
    fi
    [ -n "$LOADED" ] && echo "ansible-preflight: loaded operator secrets from the master worktree (this worktree has none):$LOADED" >&2
  fi
fi

# Resolve every inherited reference before the play sees it. Re-entry retains the
# resolved environment rather than replacing it with the references just read
# from disk. GOAT_REFERENCES_ONLY decides which FILES a shell loads (plaintext
# envrc or references only); it does not decide whether a reference already in
# the environment is resolved. A reference that reaches the play unresolved is
# sent as the literal string — a registry login with "op://…" as its password
# fails on the host with no_log hiding why — so any reference present is resolved
# here, whatever the shell's migration state.
if [ -z "${GOAT_SECRETS_RESOLVED:-}" ] \
   && { [[ "$(env)" == *"=${GOAT_SECRET_REF_PREFIX:-op://}"* ]] || [[ "$(env)" == *"=bao://"* ]] || [[ "$(env)" == *"=bw://"* ]]; }; then
  exec "$SCRIPT_DIR/with-secrets" "$0" "$@"
fi
# A value store (ADR 1204: env-file, git-crypt, sops) holds the values themselves,
# so the play gets them through the same injector, once, whatever the shell holds.
if [ -z "${GOAT_SECRETS_RESOLVED:-}" ]; then
  case "${GOAT_SECRET_STORE:-$(python3 "$SCRIPT_DIR/../secrets/posture.py" store 2>/dev/null || echo 1password)}" in
    env-file|git-crypt|sops) exec "$SCRIPT_DIR/with-secrets" "$0" "$@" ;;
  esac
fi

goat_arg_guards "$@"

# The source of record this play renders from, for the deploy ledger row
# (rollback-ledger records it as `source_tree`): {"commit": HEAD} from a clone,
# {"goatcore", "goatcore_ref", "overlay"} from an assembled tree. Set here, the
# one wrapper every play of both avenues passes through, so a deploy-component.sh
# run, a goatbuilder run and a bare preflight run all record it. A caller's value
# wins; an empty one records nothing.
if [ -z "${GOAT_DEPLOY_SOURCE_TREE+x}" ]; then
  GOAT_DEPLOY_SOURCE_TREE="$(python3 "$SCRIPT_DIR/lib/source_of_record.py" describe --repo "$REPO_ROOT" 2>/dev/null || true)"
  export GOAT_DEPLOY_SOURCE_TREE
fi

# The override bypasses staleness checks, not credential loading or argument guards.
if [ "${GOAT_SKIP_ANSIBLE_PREFLIGHT:-}" = "1" ]; then
  echo "ansible-preflight: BYPASS (GOAT_SKIP_ANSIBLE_PREFLIGHT=1) — running ansible-playbook from possibly-stale tree." >&2
  exec ansible-playbook "$@"
fi

# Current branch on the cwd's worktree. (cwd determines what
# ansible-playbook will read — that's what we're checking.)
BRANCH="$(git -C "$REPO_ROOT" rev-parse --abbrev-ref HEAD 2>/dev/null || echo "")"
# A detached HEAD is judged like a branch by the fresh-source check below
# (contract C3); only the trunk-branch whole-tree check at the end needs a name.
[ "$BRANCH" = "HEAD" ] && BRANCH=""

# ---------------------------------------------------------------------------
# SAFETY-CRITICAL PATHS — checked on EVERY branch, including feature branches.
#
# F-312, twice (and 08-11). A play that recreates traefik without its
# ACME overlay takes a goatnet off management — including the mgmt hosts
# themselves. The fix landed in main; 28 live worktrees still carried the broken
# task the same day, and feature branches skip the staleness check below
# entirely, because a feature branch is SUPPOSED to differ from trunk.
#
# That reasoning is right for ordinary files and wrong for these. A stale copy of
# a file in this list does not produce a stale deploy — it produces an outage. So
# these few paths must match origin/main regardless of branch.
#
# Keep this list SHORT. It is not "important files"; it is "files whose staleness
# breaks the control plane". Each entry costs every feature branch a rebase, so
# each should name the failure it prevents.
SAFETY_CRITICAL_PATHS="
ops/ansible/roles/traefik/tasks/main.yml
ops/ansible/roles/admit-audit/tasks/main.yml
ops/ansible/roles/admit-audit/tasks/deploy.yml
ops/ansible/roles/observability/tasks/main.yml
ops/ansible/mgmt-host.yml
"

# WHICH REMOTE IS TRUNK. Hardcoded `origin` is wrong wherever `origin` is a
# mirror. A guard that measures staleness against a mirror
# does not merely nag — it INVERTS: it refuses a checkout that is identical to
# the real trunk (measured that day: mirror 30 commits behind), and it PASSES a
# checkout that matches the mirror while being 30 commits behind the primary,
# which is precisely the stale-control-plane deploy F-312 is about.
#
# Resolution order: $GOAT_ANSIBLE_TRUNK_REMOTE, else the remote whose URL is the
# source repository this checkout declares (repo.source in
# ops/goatbuilder/goatbuilder.yaml, read by project-setting.sh), else `origin`.
# Repos with a single remote are unaffected.
TRUNK_BRANCH="${GOAT_ANSIBLE_TRUNK_BRANCH:-main}"
if [ "$ASSEMBLED" = 1 ]; then
  TRUNK_REMOTE="goatcore.ref"   # a label for the messages; no remote of this checkout is the trunk
elif [ -n "${GOAT_ANSIBLE_TRUNK_REMOTE:-}" ]; then
  TRUNK_REMOTE="$GOAT_ANSIBLE_TRUNK_REMOTE"
else
  TRUNK_REMOTE="$("$SCRIPT_DIR/project-setting.sh" --repo "$REPO_ROOT" trunk-remote 2>/dev/null || true)"
  [ -n "$TRUNK_REMOTE" ] || TRUNK_REMOTE="origin"
fi
if [ "$ASSEMBLED" = 0 ] \
   && ! git -C "$REPO_ROOT" rev-parse --verify --quiet "$TRUNK_REMOTE/$TRUNK_BRANCH" >/dev/null 2>&1 \
   && [ "$TRUNK_REMOTE" != "origin" ] && [ "$TRUNK_BRANCH" = main ]; then
  echo "ansible-preflight: $TRUNK_REMOTE/main not present, falling back to origin/main" >&2
  TRUNK_REMOTE="origin"
fi

# FRESH SOURCE (contract C3) — checked on EVERY branch, detached HEAD included.
# ops/scripts/fresh-source.sh fetches the trunk and refuses a checkout that is
# behind it in ops/ansible or ops/pins (GB-064), or that cannot be shown current
# because the trunk cannot be fetched (GB-065; --offline / GOAT_DEPLOY_OFFLINE=1
# on an islanded site). It subsumes the per-role staleness check this script
# carried for F-410: group_vars, inventory and pins go stale the same way roles
# do. A driver that already ran it before its first host passes the trunk commit
# it judged against in GOAT_FRESH_SOURCE_TRUNK, so every play of one run is
# judged against the same commit and nothing is fetched again.
FS_ARGS=(--repo "$REPO_ROOT")
[ "$ASSEMBLED" = 1 ] || FS_ARGS+=(--remote "$TRUNK_REMOTE")
[ -n "${GOAT_FRESH_SOURCE_TRUNK:-}" ] && FS_ARGS+=(--quiet)
if ! trunk_sha="$("$SCRIPT_DIR/fresh-source.sh" "${FS_ARGS[@]}")"; then
  echo "ansible-preflight: REFUSE — see fresh-source above. Bypass only with a reason: GOAT_SKIP_ANSIBLE_PREFLIGHT=1 <your command>" >&2
  exit 1
fi
TRUNK_CMP="${trunk_sha:-$TRUNK_REMOTE/$TRUNK_BRANCH}"
STALE_CRITICAL=""
for path in $SAFETY_CRITICAL_PATHS; do
  [ -f "$REPO_ROOT/$path" ] || continue
  if ! git -C "$REPO_ROOT" diff --quiet "$TRUNK_CMP" -- "$path" 2>/dev/null; then
    STALE_CRITICAL="$STALE_CRITICAL  $path
"
  fi
done
if [ -n "$STALE_CRITICAL" ] && [ "$ASSEMBLED" = 1 ]; then
  {
    echo "ansible-preflight: REFUSE — the overlay replaces a SAFETY-CRITICAL goatcore file:"
    printf '%s' "$STALE_CRITICAL"
    echo ""
    echo "These files decide whether traefik keeps its ACME certificate resolver."
    echo "They belong to goatcore ($TRUNK_CMP); an overlay that carries its own copy"
    echo "deploys a control plane no goatcore release has. Remove the file from the"
    echo "overlay (or land the change in goatcore and move goatcore.ref), re-assemble,"
    echo "and re-run. Bypass only with a reason: GOAT_SKIP_ANSIBLE_PREFLIGHT=1 <your command>"
  } >&2
  exit 1
fi
if [ -n "$STALE_CRITICAL" ]; then
  {
    echo "ansible-preflight: REFUSE — this checkout differs from $TRUNK_REMOTE/$TRUNK_BRANCH in a SAFETY-CRITICAL path:"
    printf '%s' "$STALE_CRITICAL"
    echo ""
    echo "These files decide whether traefik keeps its ACME certificate resolver."
    echo "Running a play from a stale copy recreates traefik WITHOUT it, every vhost"
    echo "falls to TRAEFIK DEFAULT CERT, and the goatnet goes off management —"
    echo "including the mgmt hosts you would use to fix it. That is F-312, which has"
    echo "now happened twice."
    echo ""
    echo "If you did not deliberately change these files, fast-forward this worktree"
    echo "onto $TRUNK_REMOTE/$TRUNK_BRANCH (or rebase your branch onto it) and re-run."
    echo ""
    echo "If you ARE deliberately changing them, this refusal is the point: land the"
    echo "change first, then deploy from a checkout that carries it. Bypass only with"
    echo "a reason you would defend in the incident review:"
    echo ""
    echo "  GOAT_SKIP_ANSIBLE_PREFLIGHT=1 <your command>"
  } >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# GHCR NAMESPACE REACHABILITY — checked on EVERY branch, like the safety-critical
# paths above and for the same reason: the failure it prevents is a half-applied
# host, not a stale one.
#
# `ghcr_org` (ops/ansible/group_vars/all.yml) is the org default unless GHCR_ORG
# overrides it. This confirms every live pin
# exists under the namespace the play will pull from.
#
# A digest the host is already running under another namespace is the same
# bytes at an unreachable name: `docker compose pull` fails with 403. In the
# ory role that pull sits after the config-render and .env tasks and before the
# handlers, so the play leaves kratos.yaml and /opt/mgmt-host/.env rewritten
# with the OLD config still in the container. It reads as "deploy failed,
# nothing happened" and is not.
#
# Refuse only on gaps in the namespace THIS checkout will pull from; a gap on the
# other side gets a warning (somebody should fix it, but it is not this run's
# problem). Anything that is not a gap — no crane and no buildx, no network, a
# credential that reads neither namespace — WARNS and proceeds, so island-mode
# and airgap runs are never blocked by a check that could not reach the registry.
if [ "${GOAT_SKIP_GHCR_PARITY:-}" != "1" ]; then
  PARITY="$REPO_ROOT/ops/ci/check-ghcr-namespace-parity.sh"
  # Same rule as group_vars/all.yml: GHCR_ORG wins, else the organisation of the
  # image namespace this checkout declares (repo.image_namespace in
  # ops/goatbuilder/goatbuilder.yaml, as the builder reads it). No declaration
  # under ghcr.io: there is no namespace to check, and the check is skipped.
  RESOLVED_ORG="${GHCR_ORG:-}"
  if [ -z "$RESOLVED_ORG" ]; then
    DECLARED_NS="$("$SCRIPT_DIR/project-setting.sh" --repo "$REPO_ROOT" repo image_namespace 2>/dev/null || true)"
    case "$DECLARED_NS" in ghcr.io/*) RESOLVED_ORG="${DECLARED_NS#ghcr.io/}" ;; esac
  fi
  if [ -x "$PARITY" ] && [ -n "$RESOLVED_ORG" ]; then
    # `|| PARITY_RC=$?` rather than set +e: a failing command inside a bare
    # assignment still fires the ERR trap above, which printed "INTERNAL ABORT"
    # for every probe that merely could not decide (rc 75) and then went on.
    # A command on the left of `||` fires no ERR trap.
    PARITY_RC=0
    PARITY_OUT="$("$PARITY" --live-only --require "$RESOLVED_ORG" --quiet 2>&1)" || PARITY_RC=$?
    if [ "$PARITY_RC" = 1 ]; then
      {
        printf '%s\n' "$PARITY_OUT"
        echo ""
        echo "ansible-preflight: REFUSE — this checkout resolves ghcr_org to '$RESOLVED_ORG'"
        echo "and at least one live pin does not exist under that namespace."
        echo ""
        echo "Two ways out, in order of preference:"
        echo ""
        echo "  1. Repair the registry (right fix, and it fixes every other deploy too):"
        echo "       ops/ci/check-ghcr-namespace-parity.sh --backfill"
        echo ""
        echo "  2. Pull from the other namespace for this run only:"
        echo "       GHCR_ORG=<other-org> <your command>"
        echo ""
        echo "Bypass entirely with GOAT_SKIP_GHCR_PARITY=1 — but read the refusal first:"
        echo "the pull that fails is not the first task in the play, so the host does not"
        echo "stay untouched."
      } >&2
      exit 3
    elif [ "$PARITY_RC" != 0 ]; then
      echo "ansible-preflight: WARN — could not verify ghcr namespace parity (rc=$PARITY_RC); proceeding." >&2
      [ -n "$PARITY_OUT" ] && printf '%s\n' "$PARITY_OUT" | tail -4 >&2
    fi
  fi
fi

# Only enforce whole-tree staleness on trunk branches. Feature-branch worktrees
# are SUPPOSED to be ahead/behind of origin/<branch> — the safety-critical check
# above is what covers them.
# An assembled tree has no trunk branch of its own: fresh-source.sh above judged
# it against goatcore.ref and the overlay's main, which is the whole-tree check.
[ "$ASSEMBLED" = 1 ] && exec ansible-playbook "$@"
case "$BRANCH" in
  main|master) ;;
  *) exec ansible-playbook "$@" ;;
esac

# Fetch <trunk-remote>/<branch>. Soft-fail: airgap / network blip shouldn't
# break the play; warn + proceed with what we have locally.
#
# TRUNK_REMOTE, not a hardcoded `origin`: where `origin` is a mirror, a checkout
# sitting exactly on the trunk reads as "diverged" when the mirror is behind,
# and a checkout matching a stale mirror passes.
#
# On main the fresh-source check above already fetched (or was pinned to, or
# judged offline against) the trunk commit; this check uses that same commit.
if [ "$BRANCH" != main ] || [ -z "$trunk_sha" ]; then
  if ! git -C "$REPO_ROOT" fetch --quiet "$TRUNK_REMOTE" "$BRANCH" 2>/dev/null; then
    echo "ansible-preflight: WARN — fetch $TRUNK_REMOTE $BRANCH failed; checking against cached $TRUNK_REMOTE ref." >&2
  fi
fi

HEAD_SHA="$(git -C "$REPO_ROOT" rev-parse HEAD)"
ORIGIN_REF="$TRUNK_REMOTE/$BRANCH"
if [ "$BRANCH" = main ] && [ -n "$trunk_sha" ]; then
  ORIGIN_SHA="$trunk_sha"; ORIGIN_REF="$trunk_sha"
elif ! ORIGIN_SHA="$(git -C "$REPO_ROOT" rev-parse "$ORIGIN_REF" 2>/dev/null)"; then
  echo "ansible-preflight: WARN — no $ORIGIN_REF ref locally (never fetched?). Proceeding without check." >&2
  exec ansible-playbook "$@"
fi

# Happy path.
if [ "$HEAD_SHA" = "$ORIGIN_SHA" ]; then
  exec ansible-playbook "$@"
fi

# HEAD is ancestor of the trunk remote → cleanly behind, can fast-forward.
if git -C "$REPO_ROOT" merge-base --is-ancestor HEAD "$ORIGIN_REF" 2>/dev/null; then
  BEHIND_COUNT="$(git -C "$REPO_ROOT" rev-list --count "HEAD..$ORIGIN_REF")"
  cat >&2 <<EOF

ansible-preflight: REFUSE — local $BRANCH is $BEHIND_COUNT commit(s) behind $ORIGIN_REF.

  local  HEAD: $HEAD_SHA
  $ORIGIN_REF: $ORIGIN_SHA

ansible-playbook reads ops/ansible/ role defaults (image pins,
blocklists, group_vars) from cwd. Running now would template with
stale defaults — silently (the pull-image task would show 'ok' not
'changed', easy to miss).

Fast-forward to $ORIGIN_REF and re-run:

  GOAT_ALLOW_MASTER_GIT_MUTATE=1 \\
    git -C $REPO_ROOT pull --ff-only $TRUNK_REMOTE $BRANCH

(The GOAT_ALLOW_MASTER_GIT_MUTATE override is required by the
destructive-git hook for master-worktree fast-forwards per ADR 0013.)

If you genuinely intend to run from this stale tree, bypass with:

  GOAT_SKIP_ANSIBLE_PREFLIGHT=1 $0 $SAFE_ARGV

EOF
  exit 2
fi

# Diverged: local has commits not on the trunk remote (or branches forked).
# Investigate before applying — likely a worktree mistake.
LOCAL_ONLY_COUNT="$(git -C "$REPO_ROOT" rev-list --count "$ORIGIN_REF..HEAD" 2>/dev/null || echo "?")"
LOCAL_ONLY="$(git -C "$REPO_ROOT" log --oneline "$ORIGIN_REF..HEAD" 2>/dev/null | head -5 || true)"
cat >&2 <<EOF

ansible-preflight: REFUSE — local $BRANCH has diverged from $ORIGIN_REF.

  local  HEAD: $HEAD_SHA
  $ORIGIN_REF: $ORIGIN_SHA
  $LOCAL_ONLY_COUNT commit(s) on HEAD not on $ORIGIN_REF (last 5):

$LOCAL_ONLY

This is an unusual state on a trunk branch — investigate before
applying. Typical cause: a local commit was made on master instead
of a feature branch.

If you genuinely intend to run from this state, bypass with:

  GOAT_SKIP_ANSIBLE_PREFLIGHT=1 $0 $SAFE_ARGV

EOF
exit 2
