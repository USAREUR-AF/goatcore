#!/usr/bin/env bash
# Mint the two service identities machine-provenance-guard needs, on one goatnet.
#
# OPERATOR-RUN, NEVER A PLAYBOOK SIDE EFFECT (live-state-guard-scheduling.md §4).
# Creating an admin-scoped mesh credential is a decision with an audit trail
# attached; it should happen because somebody ran a command that says so, not as
# a quiet consequence of converging a host. The role consumes what this prints
# and REFUSES to install without it.
#
# Two planes, because this guard joins them:
#   1. a NetBird service user + PAT   — to list peers and their groups. Minted by
#      ops/scripts/netbird-service-identities.py (identity provenance-guard in
#      ops/netbird/service-identities.yaml), which this script calls.
#   2. a portal operator service key  — to read /api/machines, the custody registry
#
# On privilege, stated plainly so nobody reads "service user" as "least
# privilege": the NetBird PAT is necessarily `role: admin`, because NetBird has
# no read-only role that can list what this guard compares against. A dedicated
# identity does not reduce that privilege. It scopes REVOCATION (cut this guard
# without taking down netvis or microscope) and restores ATTRIBUTION (its reads
# appear as this guard rather than an anonymous admin uuid). Both are worth
# having. Neither is containment.
#
# Usage:
#   ops/scripts/create-provenance-guard-identity.sh <goatnet> [--dry-run]
#
# Needs, in the environment:
#   <SITE>_NB_API_TOKEN        the goatnet's bootstrap NetBird identity (read by
#                              netbird-service-identities.py from the envrc)
#   MGMT_HOST                  ssh target of the mgmt host, for the portal key
#   MGMT_SSH_OPTS              extra ssh options, UNQUOTED and word-split on
#                              purpose — e.g. through an env door (ADR 1117):
#                                MGMT_HOST=<door user>@127.0.0.1
#                                MGMT_SSH_OPTS="-p 12202 -o StrictHostKeyChecking=no"
#                              Folding these into MGMT_HOST does not work: ssh
#                              gets one quoted argument and reports "Bad port".
#   PORTAL_URL                 that goatnet's portal base URL, for the load
#                              check (required unless --dry-run; no built-in zone)
#   PORTAL_KEYS_DIR            portal operator-key dir (default below)
set -euo pipefail

GOATNET="${1:-}"
DRY_RUN=0
[ "${2:-}" = "--dry-run" ] && DRY_RUN=1

if [ -z "$GOATNET" ]; then
  echo "usage: $0 <goatnet> [--dry-run]" >&2
  exit 64
fi

# NOTE: no apostrophes inside ${VAR:?...}. Bash starts a quote context on one
# even inside double quotes, and the whole script fails to parse with an error
# pointing at a line far below the real one.
: "${MGMT_HOST:?set MGMT_HOST — ssh target of the mgmt host for this goatnet}"
# Asked for before anything is minted, so a run cannot stop after step 1 for
# want of the address step 2 proves itself against.
[ "$DRY_RUN" = "1" ] || : "${PORTAL_URL:?set PORTAL_URL — the base URL of this goatnet portal, for the load check}"
PORTAL_URL="${PORTAL_URL:-}"; PORTAL_URL="${PORTAL_URL%/}"
PORTAL_KEYS_DIR="${PORTAL_KEYS_DIR:-/opt/goat-zenoh-portal/operator-keys.d}"
# Deliberately unquoted at use: these are multiple ssh words, not one argument.
MGMT_SSH_OPTS="${MGMT_SSH_OPTS:-}"

# Shell var names take no hyphens; site-a -> SITE_A. Two trs, not ${x^^}:
# case conversion is bash 4+ and this script is operator-run, so it has to work
# on stock macOS /bin/bash 3.2 (ops/ci/check-bash-portability.sh enforces it).
ENV_PREFIX="$(printf %s "$GOATNET" | tr "[:lower:]" "[:upper:]" | tr -c "A-Z0-9" "_")"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

say() { printf '\n\033[1m%s\033[0m\n' "$*"; }

say "1/2  NetBird service identity: provenance-guard"
if [ "$DRY_RUN" = "1" ]; then
  python3 "$SCRIPT_DIR/netbird-service-identities.py" "$GOATNET" --only provenance-guard || true
else
  # Creates the service user and token only when missing, writes the envrc and
  # 1Password, prints no token, and exits non-zero unless the token verifies.
  python3 "$SCRIPT_DIR/netbird-service-identities.py" "$GOATNET" --only provenance-guard --mint
fi

say "2/2  Portal operator service key"
portal_key="gpg_$(head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n')"
if [ "$DRY_RUN" = "1" ]; then
  echo "     [dry-run] would write ${PORTAL_KEYS_DIR}/machine-provenance-guard.key on ${MGMT_HOST}"
else
  # The portal hot-reloads this directory, so registering the key is a file
  # write and revoking it is a file delete — no portal redeploy either way.
  #
  # TWO THINGS THAT FAIL SILENTLY FROM THE WRITER'S SIDE, both handled here:
  #
  #   1. FORMAT. The file must contain `handle:key` lines, not a bare key —
  #      config.OperatorAPIKeysDir parses one colon-separated pair per line and
  #      returns a hard error otherwise. A bare key makes the portal log
  #      "operator-key reload failed; SERVING THE PREVIOUS SET" every 30s. No
  #      existing key breaks (that fail-safe is deliberate and good), but the new
  #      one never loaded and the only evidence was in the portal's own log.
  #   2. OWNERSHIP. The portal container runs as uid 65532 and cannot read a
  #      root-owned 0640 file. Every other key in that directory is 65532:65532;
  #      `sudo tee` made this one root:root.
  printf 'machine-provenance-guard:%s\n' "$portal_key" | ssh $MGMT_SSH_OPTS "$MGMT_HOST" \
    "sudo install -d -m 0750 -o 65532 -g 65532 '${PORTAL_KEYS_DIR}' && sudo tee '${PORTAL_KEYS_DIR}/machine-provenance-guard.key' >/dev/null && sudo chown 65532:65532 '${PORTAL_KEYS_DIR}/machine-provenance-guard.key' && sudo chmod 0640 '${PORTAL_KEYS_DIR}/machine-provenance-guard.key'"
  echo "     registered at ${PORTAL_KEYS_DIR}/machine-provenance-guard.key"

  # Prove it loaded rather than assuming the write was the outcome. The portal
  # rereads the directory on its own interval, so poll instead of racing it.
  echo "     waiting for the portal to load it..."
  for _ in $(seq 1 10); do
    code="$(ssh $MGMT_SSH_OPTS "$MGMT_HOST" "curl -s -o /dev/null -w '%{http_code}' -H 'Authorization: Bearer ${portal_key}' '${PORTAL_URL}/api/machines'" 2>/dev/null || true)"
    [ "$code" = "200" ] && { echo "     portal accepts it (200)"; break; }
    sleep 6
  done
  if [ "${code:-}" != "200" ]; then
    echo "WARN: the portal still answers ${code:-<none>} for this key. Check its log:" >&2
    echo "      ssh ${MGMT_SSH_OPTS} ${MGMT_HOST} 'sudo docker logs --since 5m goat-zenoh-portal | grep operator-key'" >&2
    echo "      A reload error there names the offending line and keeps serving the previous set." >&2
  fi
fi

if [ "$DRY_RUN" = "1" ]; then
  say "dry run — nothing created."
  exit 0
fi

cat <<EOF

──────────────────────────────────────────────────────────────────────────────
The NetBird token is already in .envrc.local.d/30-site-services.envrc and
1Password as ${ENV_PREFIX}_PROVENANCE_GUARD_NB_TOKEN. Put the portal key in the
same file, then set the group_vars to read it. Shown ONCE.

  ${ENV_PREFIX}_PROVENANCE_GUARD_PORTAL_KEY=${portal_key}

To revoke later, both halves independently:
  mesh    delete the PAT, or the service user, in the NetBird admin API
  portal  ssh ${MGMT_SSH_OPTS} ${MGMT_HOST} 'sudo rm ${PORTAL_KEYS_DIR}/machine-provenance-guard.key'
          (hot-reloaded; no redeploy)

Then deploy the role. Its first run happens inline with you watching, and the
play FAILS if that run exits 2 — a guard that cannot read its sources must not
be installed quietly.
──────────────────────────────────────────────────────────────────────────────
EOF
