#!/usr/bin/env bash
# F-424 repair 3 — a new trust root must be a REVIEWABLE EVENT, not a group_vars line.
#
# WHY. The Zenoh ACL matches a peer on its certificate Common Name and cannot see
# which CA issued it (zenoh-config AclConfigSubjects has no issuer axis). The
# router's trust store is a CONCATENATION (roles/goat-zenoh/tasks/main.yml), so
# every root added to it can mint any name the ACL recognises — including
# CN=goat-zenoh-router. Proven, not argued: ops/federation-smoke/assert-pki.sh.
#
# So this guard does two things:
#   1. every `goat_zenoh_extra_ca_paths` entry must be on the declared allow-list
#   2. the trust-root key FORM must be consistent, because load_trust_anchors
#      tries RAW then BASE64 then FILE and RETURNS ON FIRST HIT
#      (zenoh-link-tls/src/utils.rs:557-581) while inspect_config only rejects
#      duplicates in the GLOBAL block — so mixing forms silently trusts the wrong
#      root with no error and no log.
set -uo pipefail
cd "$(dirname "$0")/../.." || exit 1
fail=0
note() { printf "  %s\n" "$1"; }

# --- 1. declared in-governance issuers -------------------------------------
# Add a path here ONLY with a reviewer who understands it grants mint authority
# for every CN in the subject table of any router that loads it.
#
# Declared issuers, one per line. Each entry below has a reviewer who accepted
# that it grants mint authority for every CN in the subject table of any router
# that loads it.
#
#   /opt/spire/data/server/spire-ca.pem
#       The SPIRE server CA of the goatnet. Admits SVID-bearing clients (fabric-health).
#       Ours, on our host, rotated by our own server.
#
#   {{ goat_zenoh_tls_dir }}/<site>-root-cert.pem
#       The goatnet's own offline-CA root, delivered from ops/enrollment/ca/<site>/.
#       Present ALONGSIDE the shared root during the additive phase of a CA
#       separation. Accepted because BOTH roots are ours. It would NOT
#       be acceptable as a resting state with a root we do not govern.
#
#   {{ goat_zenoh_tls_dir }}/shared-root-cert.pem
#       The shared root a goatnet migrates AWAY from during a CA separation. Without
#       this entry a converge emits a bundle the router cannot validate its own
#       leaf against. Ours. THIS ENTRY IS THE ONE THAT LEAVES: the last step of the
#       separation deletes it here and in both groups' spire.yml, once no leaf
#       chains to it.
ALLOWED='/opt/spire/data/server/spire-ca.pem
{{ goat_zenoh_tls_dir }}/zagros-root-cert.pem
{{ goat_zenoh_tls_dir }}/shared-root-cert.pem'

# A deployment declares its own goatnets' issuers — the <site>-root-cert.pem
# entry above, for each goatnet that carries one — in
# ops/ci/check-zenoh-trust-anchors.d/*.allow, one path per line (blank lines and
# `#` comments ignored), under the same review rule.
for f in ops/ci/check-zenoh-trust-anchors.d/*.allow; do
  [ -f "$f" ] || continue
  while IFS= read -r p; do
    p="${p%%#*}"; p="${p%"${p##*[![:space:]]}"}"; p="${p#"${p%%[![:space:]]*}"}"
    [ -n "$p" ] && ALLOWED="$ALLOWED
$p"
  done < "$f"
done

echo "[trust-anchors] extra CA paths"
while IFS= read -r line; do
  f="${line%%:*}"; rest="${line#*:}"; n="${rest%%:*}"
  # collect the list items that follow the key
  awk -v start="$n" 'NR>start && /^[[:space:]]*-[[:space:]]*"/{gsub(/^[[:space:]]*-[[:space:]]*"|"[[:space:]]*$/,""); print; next} NR>start{exit}' "$f" \
  | while IFS= read -r p; do
      [ -z "$p" ] && continue
      if grep -qxF "$p" <<<"$ALLOWED"; then
        note "ok    $f -> $p"
      else
        note "FAIL  $f -> $p  (not a declared in-governance issuer)"
        echo "FAILMARK" >> /tmp/.zta-fail
      fi
    done
done < <(grep -rn '^goat_zenoh_extra_ca_paths:' ops/ansible/group_vars/ 2>/dev/null || true)
[ -f /tmp/.zta-fail ] && { fail=1; rm -f /tmp/.zta-fail; }

# --- 2. one trust-root key form only ---------------------------------------
echo "[trust-anchors] root-CA key form"
mixed=$(grep -rln 'root_ca_certificate_base64' ops/ansible/roles/*/templates/ 2>/dev/null || true)
if [ -n "$mixed" ]; then
  for f in $mixed; do
    if grep -q 'root_ca_certificate[^_]' "$f"; then
      note "FAIL  $f mixes root_ca_certificate and root_ca_certificate_base64"
      note "      load_trust_anchors returns on FIRST HIT (raw > base64 > file):"
      note "      the other form is silently ignored. Pick one."
      fail=1
    fi
  done
else
  note "ok    no base64 trust-root form in use; file form is unambiguous"
fi

if [ "$fail" -eq 0 ]; then
  echo "[trust-anchors] PASS"
else
  echo "[trust-anchors] FAIL — see docs/findings/F-424-*.md"
fi
exit $fail
