#!/usr/bin/env bash
# cluster-secrets-init.sh — one-time genesis for cluster-shared Ory secrets.
#
# Run ONCE per cluster, the first time you stand up Ory anywhere. Generates
# the four cluster-shared secrets (KRATOS_COOKIE, KRATOS_CIPHER, HYDRA_SYSTEM,
# HYDRA_COOKIE), appends a labeled block to ~/.envrc.local under the operator's
# account (or the repo's .envrc.local — see USAGE), and prompts the operator
# to persist to 1Password.
#
# Why a script and not "the playbook just generates them at deploy time":
#
#   The four secrets MUST be byte-identical across every site that joins the
#   Spock-replicated kratos.* / hydra.* schemas. If a playbook
#   silently auto-generated values per-site, a second-site deploy would
#   produce fresh values that decrypt none of the rows replicated from site
#   #1. The break is silent at deploy time, surfaces at first cross-site
#   identity flow. So genesis is operator-fired, one-time, NOT in any
#   playbook; deploy-time playbooks only ASSERT the values are present and
#   READ them.
#
# Idempotent: if any of the four secrets are already in .envrc.local, refuses
# to overwrite. Recovery path is at docs/operations/ory-deploy-handoff.md
# §"Cluster secrets" — pull from 1Password OR from a deployed peer's
# /opt/mgmt-host/.env.
#
# Usage:
#   ops/scripts/cluster-secrets-init.sh --sites <site1,site2,...>      # writes to repo's .envrc.local
#   ops/scripts/cluster-secrets-init.sh --sites <site1,site2,...> --target ~/.envrc.local
#
# --sites names the cluster's sites; it is required, because the sites are the
# cluster's own. The first is the source of truth and the rest mirror it.
#
# After running:
#   1. Persist the four <FIRST_SITE>_* values to 1Password (operator-side).
#   2. `direnv allow` to load the new env vars.
#   3. Run `site-preflight.yml -e site_id=<site>` to confirm pre-flight passes.
#   4. Run the Ory deploy chain per docs/operations/ory-deploy-handoff.md.

set -euo pipefail

# ---------------------------------------------------------------------------
# Defaults
# ---------------------------------------------------------------------------

target=".envrc.local"
sites=""

# ---------------------------------------------------------------------------
# Arg parsing
# ---------------------------------------------------------------------------

while [[ $# -gt 0 ]]; do
  case "$1" in
    --target)
      target="$2"; shift 2 ;;
    --sites)
      sites="$2"; shift 2 ;;
    -h|--help)
      sed -n '2,/^set -euo pipefail/p' "$0" | sed 's/^# \?//' | head -n -1
      exit 0 ;;
    *)
      echo "error: unknown arg: $1" >&2
      echo "usage: $0 --sites <site1,site2,...> [--target <path>]" >&2
      exit 64 ;;
  esac
done

if [[ -z "$sites" ]]; then
  echo "error: --sites is required: the cluster's sites, comma-separated, source of truth first." >&2
  echo "usage: $0 --sites <site1,site2,...> [--target <path>]" >&2
  exit 64
fi

# ---------------------------------------------------------------------------
# Pre-flight checks
# ---------------------------------------------------------------------------

if ! command -v openssl >/dev/null 2>&1; then
  echo "error: openssl not found in PATH." >&2
  exit 65
fi

if [[ ! -f "$target" ]]; then
  echo "error: target file does not exist: $target" >&2
  echo "  create it first (touch $target && chmod 600 $target) or pass" >&2
  echo "  --target <path> to point at an existing .envrc.local." >&2
  exit 66
fi

# Refuse to overwrite if any cluster-shared secret already exists in target.
# Catches the "operator forgot we already did this" footgun.
existing=0
if grep -hqE '^(export )?[A-Z_]+_(KRATOS_COOKIE|KRATOS_CIPHER|HYDRA_SYSTEM|HYDRA_COOKIE)=' "$target" 2>/dev/null; then
  existing=$(grep -hcE '^(export )?[A-Z_]+_(KRATOS_COOKIE|KRATOS_CIPHER|HYDRA_SYSTEM|HYDRA_COOKIE)=' "$target")
fi
if [[ "$existing" -gt 0 ]]; then
  echo "error: $target already contains $existing cluster-shared Ory secret line(s)." >&2
  echo "  refusing to overwrite. If you genuinely need to rotate, do it manually" >&2
  echo "  and roll the values across every site's /opt/mgmt-host/.env in lockstep." >&2
  echo "  if you're trying to ADD a new site, copy the existing source-of-truth values" >&2
  echo "  to <NEW_SITE>_* aliases — see docs/operations/ory-deploy-handoff.md" >&2
  echo "  §'Cluster secrets'." >&2
  exit 67
fi

# ---------------------------------------------------------------------------
# Genesis
# ---------------------------------------------------------------------------

today="$(date +%Y-%m-%d)"
kratos_cookie="$(openssl rand -hex 32)"
kratos_cipher="$(openssl rand -hex 16)"  # exactly 32 hex chars
hydra_system="$(openssl rand -hex 32)"
hydra_cookie="$(openssl rand -hex 32)"

# First site in the comma-separated list is the source-of-truth; the rest
# are mirror sites that reference the source-of-truth values via shell expansion.
IFS=',' read -ra site_arr <<< "$sites"
src="${site_arr[0]}"
src_upper="$(echo "$src" | tr '[:lower:]-' '[:upper:]_')"

# ---------------------------------------------------------------------------
# Append labeled block
# ---------------------------------------------------------------------------

{
  echo
  echo "# -------------------------------------------------------------------"
  echo "# Cluster-shared Ory secrets (generated $today by cluster-secrets-init.sh)."
  echo "# Per F-005, KRATOS_CIPHER MUST be byte-identical across every Kratos"
  echo "# instance that shares Spock-replicated kratos.* tables. Cookie + hydra"
  echo "# secrets aligned cluster-wide for simplicity (per-site different is"
  echo "# also valid; see docs/operations/ory-deploy-handoff.md). Mirror sites"
  echo "# expand from the source-of-truth values; do NOT regenerate per-site."
  echo "# TODO operator: persist these to 1Password before proceeding."
  echo "export ${src_upper}_KRATOS_COOKIE='${kratos_cookie}'"
  echo "export ${src_upper}_KRATOS_CIPHER='${kratos_cipher}'"
  echo "export ${src_upper}_HYDRA_SYSTEM='${hydra_system}'"
  echo "export ${src_upper}_HYDRA_COOKIE='${hydra_cookie}'"
  for site in "${site_arr[@]:1}"; do
    site_upper="$(echo "$site" | tr '[:lower:]-' '[:upper:]_')"
    echo "# ${site} mirrors — MUST equal ${src} source-of-truth byte-for-byte"
    echo "export ${site_upper}_KRATOS_COOKIE=\"\$${src_upper}_KRATOS_COOKIE\""
    echo "export ${site_upper}_KRATOS_CIPHER=\"\$${src_upper}_KRATOS_CIPHER\""
    echo "export ${site_upper}_HYDRA_SYSTEM=\"\$${src_upper}_HYDRA_SYSTEM\""
    echo "export ${site_upper}_HYDRA_COOKIE=\"\$${src_upper}_HYDRA_COOKIE\""
  done
} >> "$target"

# ---------------------------------------------------------------------------
# Hand off to operator
# ---------------------------------------------------------------------------

echo
echo "✅ Appended cluster-shared Ory secrets to: $target"
echo "   Source-of-truth: ${src} (4 values)"
echo "   Mirror sites:    ${site_arr[*]:1}"
echo
echo "Next steps (operator):"
echo "  1. Persist the four ${src_upper}_* values to 1Password under a vault"
echo "     entry like 'goat / cluster-shared / ory-secrets'. Same values"
echo "     belong on every operator workstation that runs deploys."
echo "  2. \`direnv allow\` to load the new env vars."
echo "  3. \`ansible-playbook -i ops/ansible/inventory/hosts.ini ops/ansible/site-preflight.yml -e site_id=${src}\`"
echo "     should now PASS — confirms the gate sees the new vars."
echo "  4. Proceed with the Ory deploy chain per docs/operations/ory-deploy-handoff.md."
echo
echo "Reminder: if a future site joins the cluster, add"
echo "its mirror block to the same .envrc.local under the cluster-shared"
echo "section — do NOT re-run this script. It refuses to overwrite by design."
