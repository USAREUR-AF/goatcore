#!/usr/bin/env bash
# check-no-goatnet-defaults.sh — a SHARED role must not default to one goatnet.
#
# THE CLASS. A role default is what every goatnet gets when it does not say
# otherwise. When that default names a specific goatnet, a new environment
# silently inherits the old one's identity and nothing reports it: the play is
# green, the file renders, the container starts. It is discovered later, by a
# service refusing to trust something, or not at all.
#
# The shapes this catches, each a default that names one goatnet:
#
#   goat-zenoh          zenoh_stack: "<goatnet>-zenoh"
#                       zenoh_router_leaf_cert_path -> ../enrollment/<goatnet>/
#   goat-zenoh-portal   portal_cert_sign_ca_id: "dev-<goatnet>-ca-ecdsa-..."
#                       portal_ca_bundle_dir: "/etc/<goatnet>-ca"
#   lan-coredns         lan_coredns_zone_dir: ../dns/<goatnet>-lan
#
# A new goatnet built from such defaults advertises another goatnet's URL as its
# OIDC issuer, telling every client it is that goatnet. oauth2-proxy refuses to
# start, which may be the only signal.
#
# THE RULE. A shared role's default may be empty, may be derived from
# {{ site_id }} / {{ site_dns_zone }}, or may be absent so the role fails
# loudly. It may not name a goatnet. A goatnet-specific role is
# exempt: its name says who it is for.
#
# Exit 0 clean, 1 on a violation. --self-test proves it goes RED.
set -euo pipefail
cd "$(git rev-parse --show-toplevel 2>/dev/null || echo .)"

export GOATNETS='nova|nova-zone|nova-lan|nova_hosts|lab-node|hobby_hosts|zagros-backbone|zagros-prod\.net|team\.example|vega-domain|vega-zone|legacy'
# Roles whose NAME declares the goatnet they serve. Not shared, so not a leak.
EXEMPT='^ops/ansible/roles/nova-lan-[a-z-]+/'

# Match the VALUE only. A comment is allowed to name a goatnet — the ones on
# the fixed lines say which goatnet the default USED to name, which is the most
# useful thing a reader can be told. Matching the whole line flagged this
# guard's own remediation.
scan() {
  # VALUE side only. A variable NAME may carry a legacy goatnet word
  # (goat_ca_root_cert_path) while its value is correct; renaming those
  # keys is a separate change with its own blast radius across every group_vars
  # that sets them. What must not name a goatnet is what the role USES.
  grep -rnE "^[a-z_]+:" ops/ansible/roles/*/defaults/main.yml 2>/dev/null \
    | grep -vE "$EXEMPT" \
    | python3 -c '
import sys,re,os
pat=re.compile(os.environ["GOATNETS"])
for line in sys.stdin:
    body=line.split("#",1)[0].rstrip()
    m=re.match(r"^([^:]+:\d+):([a-z_]+):(.*)$", body)
    if m and pat.search(m.group(3)):
        sys.stdout.write(line)
' || true
}

if [ "${1:-}" = "--self-test" ]; then
  t=$(mktemp -d); trap 'rm -rf "$t"' EXIT
  mkdir -p "$t/ops/ansible/roles/planted/defaults" "$t/ops/ansible/roles/nova-lan-planted/defaults"
  echo 'zenoh_stack: "nova-zenoh"' > "$t/ops/ansible/roles/planted/defaults/main.yml"
  echo 'nova_lan_thing: "/opt/nova-lan-thing"' > "$t/ops/ansible/roles/nova-lan-planted/defaults/main.yml"
  mkdir -p "$t/ops/ansible/roles/fixed/defaults"
  echo 'zenoh_stack: "{{ site_id }}-zenoh"   # was a nova default' > "$t/ops/ansible/roles/fixed/defaults/main.yml"
  cd "$t"
  hits=$(grep -rnE "^[a-z_]+:" ops/ansible/roles/*/defaults/main.yml 2>/dev/null | grep -vE "$EXEMPT" | awk -F'#' '{print $1}' | grep -E "(${GOATNETS})" || true)
  pass=0; fail=0
  if printf '%s' "$hits" | grep -q 'roles/planted/'; then pass=$((pass+1)); echo "  ok    flags a shared role defaulting to a goatnet"
  else fail=$((fail+1)); echo "  FAIL  did not flag the planted shared-role default"; fi
  if printf '%s' "$hits" | grep -q 'nova-lan-planted'; then fail=$((fail+1)); echo "  FAIL  flagged a goatnet-specific role, which is exempt"
  else pass=$((pass+1)); echo "  ok    exempts a role whose name declares its goatnet"; fi
  if printf '%s' "$hits" | grep -q 'roles/fixed/'; then fail=$((fail+1)); echo "  FAIL  flagged a fixed default because its COMMENT names a goatnet"
  else pass=$((pass+1)); echo "  ok    ignores a goatnet named only in a comment"; fi
  echo "  $pass passed, $fail failed"
  [ "$fail" = 0 ] || exit 1
  exit 0
fi

# Deliberate exceptions live in ops/ci/goatnet-defaults.allow, one
# `roles/<role>/defaults/main.yml:<variable>` per line. Shrink-only: an entry
# whose default no longer names a goatnet is stale and refused, so the list can
# only get shorter. The first two entries are the provisioning tier on the
# provisioning tier, which is documented as a shared
# cross-goatnet service rather than config bleed.
ALLOW=ops/ci/goatnet-defaults.allow
HITS="$(scan)"
if [ -f "$ALLOW" ]; then
  # A hit line is `<path>:<lineno>:<var>: <value>`; an allow entry is `<path>:<var>`.
  stale=""
  while IFS= read -r entry; do
    [ -z "$entry" ] && continue; case "$entry" in \#*) continue;; esac
    epath="${entry%%:*}"; evar="${entry##*:}"
    if printf '%s\n' "$HITS" | grep -qE "^${epath}:[0-9]+:${evar}:"; then
      HITS="$(printf '%s\n' "$HITS" | grep -vE "^${epath}:[0-9]+:${evar}:" || true)"
    else
      stale="$stale  $entry\n"
    fi
  done < "$ALLOW"
  if [ -n "$stale" ]; then
    echo "REFUSED: stale entries in $ALLOW (the default no longer names a goatnet; remove the line):"
    printf '%b' "$stale"; exit 1
  fi
fi
if [ -z "$HITS" ]; then
  echo "ok: no shared role defaults to a specific goatnet."
  exit 0
fi
n=$(printf '%s\n' "$HITS" | grep -c .)
echo "REFUSED: $n shared-role default(s) name a specific goatnet."
echo
printf '%s\n' "$HITS" | sed 's/^/  /'
echo
echo "A default is what a goatnet gets when it says nothing. One naming another"
echo "goatnet hands a NEW environment the OLD one's identity, silently."
echo "Make it empty, derive it from {{ site_id }}, or delete it so the role fails loudly."
exit 1
