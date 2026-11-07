#!/usr/bin/env bash
# ops/ci/check-site-config-bleed.sh — multi-site config-bleed guard.
#
# WHY: goatnet instances are forked from each other. Files that are COPIED or
# TEMPLATED to every site must NOT hardcode any one site's reachability literals
# — a hostname zone or a mesh IP — or the next fork silently inherits the wrong
# site (a Grafana copied verbatim reaches another goatnet's Hydra).
# This guard closes that class: shared deploy artifacts must derive site values
# from env vars / Jinja, never literals.
#
# Role DEFAULTS are in scope too, because that is where bleed is most dangerous.
# A default is inherited by every goatnet that forgets to override: a
# `portal_router_endpoint` default pointing at another goatnet's router makes
# the portal mint real bundles aimed at the wrong fabric. Overriding sites mask it.
#
# SELF-EXTENDING (docs/design/environment-parity.md §4, principle 3): the
# protected-literal set is DERIVED from every environment's own group_vars — it
# is never a hand-maintained list. Creating `group_vars/<newenv>/` with a DNS
# zone or a fixed mesh IP protects that env's literals on the next CI run, with
# no edit here. A hand-kept list is one the next environment escapes.
# in miniature: a guard that only knows the envs someone remembered to type.
#
# Derived from each `group_vars/<env>/`:
#   - zones     `*dns_zone` / `*cloudflare_zone` values → registrable apex
#               (`netbird.<zone>` → `<zone>`)
#   - mesh IPs  `mesh_ip_a` / `mesh_ip_b` / `*_mesh_ip` literal IPv4 values
# Skipped as non-literals: Jinja (`{{ … }}`), `TBD`, empty, and non-routable
# TLDs (.lan/.local/.internal/...) which are not globally-unique reachability
# facts and would match far too broadly.
#
# Scope = the trees copied/templated to ANY site:
#   ops/compose/**                        (copied verbatim by *-obs.yml etc.)
#   ops/ansible/roles/*/templates/**      (rendered per host, must be Jinja)
#   ops/ansible/roles/*/defaults/**       (inherited by EVERY goatnet that does
#   ops/ansible/roles/*/vars/**            not override — the fail-open seat)
#   ops/ansible/templates/**
#
# NOT scanned (legitimately site-specific or generated, not shared verbatim):
#   ops/dns/<site>/      ops/serve/ (generated from services.yaml)
#   *.md / README        comment lines (`#` and Jinja `{# … #}`)   .envrc.local*
#   */tests/*            self-contained fixtures (see the skip below)
#
# Usage:
#   ops/ci/check-site-config-bleed.sh             # scan tracked tree
#   ops/ci/check-site-config-bleed.sh --staged    # staged diff only
#   ops/ci/check-site-config-bleed.sh --list      # print the derived literal set
#   ops/ci/check-site-config-bleed.sh path ...    # specific paths
#
# Exit: 0 clean · 1 bleed found (file:line listed) · 2 usage/dep error
set -euo pipefail

# Environment discovery — read by ops/ci/check-env-coverage.sh, which asserts
# this glob still matches every group_vars/<env>/ dir. Keep the marker in sync
# with the glob used in derive_site_literals().
# env-discovery: ops/ansible/group_vars/*/
GV_ROOT="${GV_ROOT:-ops/ansible/group_vars}"

# Zones that are NOT a per-site literal and are therefore never protected.
# Shrink-only: add nothing here without a reason on the line.
#   legacy.example — the fleet-wide LEGACY/default mesh zone predating
#     multi-goatnet. The original estate's shared artifacts (coturn realm,
#     authelia, ory schema $id) reference it by design; that estate is
#     teardown-only and out of scope for parity work, which is
#     forward-only (docs/design/environment-parity.md §scope). Retrofitting it
#     is explicitly not required by this guard.
#   A deployment adds its own shared zones in
#   ops/ci/check-site-config-bleed.d/*.zones, one apex per line (blank lines
#   and `#` comments ignored), each with its reason in a comment.
SHARED_ZONES=('legacy.example')
for _f in "$(dirname "${BASH_SOURCE[0]}")"/check-site-config-bleed.d/*.zones; do
  [[ -f "$_f" ]] || continue
  while IFS= read -r _z; do
    _z="${_z%%#*}"; _z="${_z//[[:space:]]/}"
    [[ -n "$_z" ]] && SHARED_ZONES+=("$_z")
  done < "$_f"
done

# Non-routable / reserved TLDs: a `.lan` or `.local` zone is not a globally
# unique reachability fact, and short forms ("lan") would match everything.
NONROUTABLE_TLDS='^(lan|local|localdomain|internal|home|corp|test|example|invalid|localhost)$'

# Pre-derivation baseline. The derived set MUST remain a superset of this — it
# is a floor against a group_vars refactor silently shrinking coverage, not a
# list to extend (extend the tree, not this).
# The floor is empty: the derived set is the whole set.
LEGACY_FLOOR=()

SCAN_GLOBS=(
  'ops/compose'
  'ops/ansible/roles'
  'ops/ansible/templates'
)

# Pre-existing bleeds in role defaults/vars, waived at the moment that scope was
# added so widening the guard was non-breaking. RATCHET, not
# amnesty: the list is append-never. Anything NEW in defaults/vars fails, and
# each entry below is a triage owed by that component's owner, not a blessing.
#
# Keyed on file + variable name (line numbers drift). Two of these are worse
# than the rest and should be fixed first: the operator-idp roles default
# `site_dns_zone` ITSELF, which every other per-site derivation keys off.
PREEXISTING_WAIVERS=(
  'ops/ansible/roles/budget-notify/defaults/main.yml|budget_notify_smtp_from'
  'ops/ansible/roles/host-net-publish/defaults/main.yml|host_net_ingest_urls'
  'ops/ansible/roles/onboard-runner/defaults/main.yml|onboard_runner_collab_kanidm_url'
  'ops/ansible/roles/operator-idp-hydra/defaults/main.yml|site_dns_zone'
  'ops/ansible/roles/operator-idp-kratos/defaults/main.yml|site_dns_zone'
  'ops/ansible/roles/relay-health/defaults/main.yml|relay_health_public_oidc_issuer'
)

# is_waived <file> <line-text> — true if this is a recorded pre-existing bleed.
is_waived() {
  local w wf wv
  for w in "${PREEXISTING_WAIVERS[@]}"; do
    wf="${w%%|*}"; wv="${w##*|}"
    [[ "$1" == "$wf" ]] || continue
    [[ "$2" == *"$wv"* ]] && return 0
  done
  return 1
}

repo_root() { git rev-parse --show-toplevel 2>/dev/null || pwd; }
cd "$(repo_root)"

# is_shared_zone <zone> — true if the zone is on the SHARED_ZONES exemption.
is_shared_zone() {
  local z
  for z in "${SHARED_ZONES[@]}"; do [[ "$1" == "$z" ]] && return 0; done
  return 1
}

# derive_site_literals — print one protected literal per line, deduped.
derive_site_literals() {
  local dir env raw val apex tld
  for dir in "$GV_ROOT"/*/; do
    [[ -d "$dir" ]] || continue
    env="$(basename "$dir")"
    [[ "$env" == _* ]] && continue          # _examples/ is documentation, not an env

    # --- DNS zones -----------------------------------------------------------
    while IFS= read -r raw; do
      val="$(printf '%s' "$raw" | sed -E 's/^[^:]*:[[:space:]]*//; s/[[:space:]]*#.*$//; s/^["'"'"']//; s/["'"'"']$//; s/[[:space:]]*$//')"
      [[ -z "$val" || "$val" == *'{{'* || "$val" == TBD ]] && continue
      [[ "$val" != *.* ]] && continue        # single label ("lan") — far too broad
      apex="$(printf '%s' "$val" | awk -F. '{ if (NF>=2) print $(NF-1)"."$NF; else print $0 }')"
      tld="${apex##*.}"
      [[ "$tld" =~ $NONROUTABLE_TLDS ]] && continue
      is_shared_zone "$apex" && continue
      printf '%s\n' "$apex"
    done < <(command grep -rhE '^[[:space:]]*[a-z0-9_]*(dns_zone|cloudflare_zone)[[:space:]]*:' "$dir" 2>/dev/null || true)

    # --- fixed mesh IPs ------------------------------------------------------
    while IFS= read -r raw; do
      val="$(printf '%s' "$raw" | sed -E 's/^[^:]*:[[:space:]]*//; s/[[:space:]]*#.*$//; s/^["'"'"']//; s/["'"'"']$//; s/[[:space:]]*$//')"
      [[ "$val" =~ ^[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}$ ]] || continue
      printf '%s\n' "$val"
    done < <(command grep -rhE '^[[:space:]]*([a-z0-9_]+_)?mesh_ip(_[a-z0-9]+)?[[:space:]]*:' "$dir" 2>/dev/null || true)
  done | sort -u
}

LITERALS=()
while IFS= read -r literal; do
  LITERALS+=("$literal")
done < <(derive_site_literals)

if [[ ${#LITERALS[@]} -eq 0 ]]; then
  echo "FAIL: derived zero site literals from $GV_ROOT — derivation is broken," >&2
  echo "      which would make this guard silently pass on everything." >&2
  exit 2
fi

# Superset assertion against the pre-derivation baseline.
missing_floor=()
for want in "${LEGACY_FLOOR[@]}"; do
  found=0
  for have in "${LITERALS[@]}"; do [[ "$have" == "$want" ]] && { found=1; break; }; done
  [[ $found -eq 1 ]] || missing_floor+=("$want")
done
if [[ ${#missing_floor[@]} -gt 0 ]]; then
  echo "FAIL: derived literal set lost pre-derivation coverage: ${missing_floor[*]}" >&2
  echo "      A group_vars change removed a zone/mesh IP this guard used to protect." >&2
  echo "      Restore the declaration, or drop it from LEGACY_FLOOR with a reason." >&2
  exit 2
fi

# Build one alternation regex with the literals' dots escaped.
SITE_LITERALS="$(printf '%s\n' "${LITERALS[@]}" | sed 's/\./\\./g' | paste -sd'|' -)"

mode="tree"; paths=()
case "${1:-}" in
  --staged) mode="staged" ;;
  --list)
    printf 'derived site literals (%d) from %s:\n' "${#LITERALS[@]}" "$GV_ROOT"
    printf '  %s\n' "${LITERALS[@]}"
    exit 0 ;;
  "" ) ;;
  * ) paths=("$@") ;;
esac

# Build the candidate file list.
files=()
if [[ "$mode" == "staged" ]]; then
  while IFS= read -r f; do
    files+=("$f")
  done < <(git diff --cached --name-only --diff-filter=d)
elif [[ ${#paths[@]} -gt 0 ]]; then
  files=("${paths[@]}")
else
  while IFS= read -r f; do
    files+=("$f")
  done < <(git ls-files "${SCAN_GLOBS[@]}")
fi

# Narrow to in-scope files that contain at least one literal, so the per-line
# comment-state pass below only runs on the handful that can possibly fail.
candidates=()
if (( ${#files[@]} > 0 )); then
for f in "${files[@]}"; do
  [[ -f "$f" ]] || continue
  # Only enforce on the shared deploy trees (so --staged on unrelated files is a no-op).
  case "$f" in
    ops/compose/*|ops/ansible/roles/*/templates/*|ops/ansible/templates/*) ;;
    ops/ansible/roles/*/defaults/*|ops/ansible/roles/*/vars/*) ;;
    *) continue ;;
  esac
  # Skip docs + roles' own templates dir READMEs.
  case "$f" in *.md|*/README*) continue ;; esac
  # Skip test fixtures (promtool unit tests under ops/compose/*/tests/). A
  # fixture declares its own input series AND its own expectations: the literal
  # inside it configures nothing, resolves nothing, and is never rendered to a
  # host — and asserting on the REAL label shape is the whole point of the test
  # (a matcher that quietly selects nothing is what those tests exist to catch).
  # Roles' tests/ dirs are already outside the scan; only ops/compose has any.
  case "$f" in */tests/*) continue ;; esac
  grep -qE "$SITE_LITERALS" "$f" 2>/dev/null && candidates+=("$f")
done
fi

hits=0
if (( ${#candidates[@]} > 0 )); then
for f in "${candidates[@]}"; do
  # Jinja `{# … #}` block comments: a template's own header block routinely
  # documents example zones ("e.g. relay-a.example.net for prod sites"). Those
  # are prose, not rendered output. Track block state across lines so the
  # continuation lines are skipped too, not just the `{#` opener.
  in_jinja_comment=0
  lineno=0
  while IFS= read -r content || [[ -n "$content" ]]; do
    lineno=$((lineno+1))
    line_was_comment=$in_jinja_comment
    # Update block state: last unmatched opener/closer on this line wins.
    if [[ $in_jinja_comment -eq 0 && "$content" == *'{#'* && "$content" != *'#}'* ]]; then
      in_jinja_comment=1
    elif [[ $in_jinja_comment -eq 1 && "$content" == *'#}'* ]]; then
      in_jinja_comment=0
    fi
    # A single-line `{# … #}` is a comment for this line only.
    [[ "$content" == *'{#'*'#}'* ]] && line_was_comment=1
    [[ $line_was_comment -eq 1 ]] && continue

    stripped="${content#"${content%%[![:space:]]*}"}"   # left-trim
    [[ "$stripped" == \#* ]] && continue                  # whole-line comment
    # Skip Jinja graceful fallbacks `... | default('nova...')`: the PRIMARY value
    # derives from a var; the literal is a last-resort default, not verbatim
    # bleed. A literal that is NOT inside a default() is a real primary bleed.
    [[ "$content" == *"default("* ]] && continue
    [[ "$content" =~ $SITE_LITERALS ]] || continue
    # Recorded pre-existing bleed in a role default/vars (see PREEXISTING_WAIVERS).
    is_waived "$f" "$content" && continue

    printf '  %s:%s: %s\n' "$f" "$lineno" "$stripped"
    hits=$((hits+1))
  done < "$f"
done
fi

if [[ $hits -gt 0 ]]; then
  echo ""
  echo "FAIL: $hits site-config bleed(s) — a shared deploy artifact hardcodes another"
  echo "site's zone/IP. Derive it from an env var (\${VAR} in compose, rendered per-site"
  echo "in the .env) or Jinja ({{ site_dns_zone }}/{{ mesh_ip_a }}). See F-188." >&2
  echo "Protected literals are derived from group_vars; \`--list\` prints the set." >&2
  exit 1
fi
echo "OK: no site-config bleed in shared deploy artifacts (${#LITERALS[@]} derived site literals)."
