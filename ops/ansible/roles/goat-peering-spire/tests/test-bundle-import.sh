#!/usr/bin/env bash
# test-bundle-import.sh — phase 13 imports the partner's SPIFFE bundle from the
# exchanged file only when our SPIRE server holds none for that trust domain, or
# when the file's spiffe_sequence is newer than the one held. A bundle the server
# already refreshed from the partner's bundle endpoint is never replaced by the
# older copy exchanged at the start, unless peering_spire_bundle_replace is set.
#
# A stand-in docker plays the spire-server CLI: `bundle list -id` prints the
# bundle held (or fails when none is), `bundle set` stores what it reads.
#
# Usage:  ops/ansible/roles/goat-peering-spire/tests/test-bundle-import.sh
# Exit:   0 = pass (or SKIP when ansible-playbook is missing); 1 = fail.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
if ! command -v ansible-playbook >/dev/null 2>&1; then
  echo "goat-peering-spire test-bundle-import: SKIP (ansible-playbook not on PATH)"
  exit 0
fi

T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
mkdir -p "$T/bin"
cat > "$T/bin/docker" <<SH
#!/bin/sh
# docker exec [-i] <container> <bin> bundle list|set ...
for a in "\$@"; do
  case "\$a" in
    list) if [ -f "$T/held" ]; then cat "$T/held"; exit 0; fi
          echo "rpc error: code = NotFound desc = no such bundle" >&2; exit 1 ;;
    set)  cat > "$T/held"; echo set >> "$T/calls"; exit 0 ;;
  esac
done
exit 2
SH
chmod 755 "$T/bin/docker"

bundle() { printf '{"keys":[{"use":"x509-svid","kty":"EC","crv":"P-256","x":"%s","y":"QUJD"}]%s}\n' "$1" "${2:+,\"spiffe_sequence\":$2}"; }
LOG="$T/play.log"
run() { # rest: extra arguments
  rm -f "$T/calls"
  ANSIBLE_NOCOLOR=1 ANSIBLE_RETRY_FILES_ENABLED=0 ANSIBLE_LOCALHOST_WARNING=0 ANSIBLE_INVENTORY_UNPARSED_WARNING=0 \
    ansible-playbook -i localhost, "$HERE/bundle-import.yml" -e "stand_in_bin=$T/bin" \
    -e "peering_spire_their_bundle_file=$T/theirs.spiffe-bundle" -e peering_spire_server_host=localhost \
    -e peering_spire_partner_trust_domain=spiffe://xab.example.org "$@" > "$LOG" 2>&1
}
fails=0
pass() { echo "  PASS  $1"; }
fail() { echo "  FAIL  $1"; tail -25 "$LOG"; fails=$((fails + 1)); }
set_calls() { if [ -f "$T/calls" ]; then wc -l < "$T/calls" | tr -d ' '; else echo 0; fi; }

echo "goat-peering-spire test-bundle-import"

bundle EXCH 3 > "$T/theirs.spiffe-bundle"
rm -f "$T/held"
if run && [ "$(set_calls)" = 1 ] && cmp -s "$T/held" "$T/theirs.spiffe-bundle"; then
  pass "no bundle held: the exchanged one is imported"
else fail "no bundle held: the exchanged one was not imported"; fi

bundle FRESH 5 > "$T/held"; cp "$T/held" "$T/fresh"
if run && [ "$(set_calls)" = 0 ] && cmp -s "$T/held" "$T/fresh" && grep -qF "keeps the bundle our server holds" "$LOG"; then
  pass "a newer bundle held (sequence 5 over 3): kept, and the run says so"
else fail "a newer bundle held was replaced by the older exchanged one"; fi

bundle EXCH 3 > "$T/held"
if run && [ "$(set_calls)" = 0 ]; then
  pass "the same sequence held: nothing set"
else fail "the same sequence held: set again"; fi

bundle OLD 2 > "$T/held"
if run && [ "$(set_calls)" = 1 ] && cmp -s "$T/held" "$T/theirs.spiffe-bundle"; then
  pass "an older bundle held (sequence 2 under 3): the exchanged one is imported"
else fail "an older bundle held was not replaced by the newer exchanged one"; fi

bundle NOSEQ > "$T/held"; cp "$T/held" "$T/noseq"
if run && [ "$(set_calls)" = 0 ] && cmp -s "$T/held" "$T/noseq"; then
  pass "a held bundle with no sequence: kept, since the two cannot be ordered"
else fail "a held bundle with no sequence was replaced"; fi

bundle FRESH 5 > "$T/held"
if run -e peering_spire_bundle_replace=true && [ "$(set_calls)" = 1 ] && cmp -s "$T/held" "$T/theirs.spiffe-bundle"; then
  pass "peering_spire_bundle_replace: the exchanged one is imported over any held"
else fail "peering_spire_bundle_replace did not import the exchanged bundle"; fi

if [ "$fails" -eq 0 ]; then
  echo "goat-peering-spire test-bundle-import: PASS"
else
  echo "goat-peering-spire test-bundle-import: $fails FAILED"
  exit 1
fi
