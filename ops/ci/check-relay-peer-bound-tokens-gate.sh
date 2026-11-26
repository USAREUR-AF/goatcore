#!/usr/bin/env bash
# Fuse: refuse to enable ADR 1021 Phase C (peer-bound-only relay tokens) while
# the peer-binding patch's REFRESH paths are still broken.
#
# Why this exists: peer binding is correct on the login / initial-sync
# path, and broken on every refresh path. Management re-mints relay tokens at
# 3/4 of CredentialsTTL. If Phase C is enabled, every peer in the fleet is locked
# out of the relay that long after it last logged in, and the relay client retries
# the refused token forever with no re-login edge — a fleet-wide, self-sustaining
# outage that looks like a tunnel wedge but is not.
#
# Three independent breakages, all in ops/netbird/patches/peer-id-bound-relay-tokens.patch:
#
#   1. SetupRefresh() is called with peer.ID (the DB primary key) and the patch
#      feeds that same variable to GenerateRelayToken() as if it were peer.Key
#      (the WireGuard public key). Management then signs HashID(peer.ID)||payload
#      while the client presents HashID(peer.Key) — wrong MAC preimage, so the
#      relay rejects with "invalid signature" for every peer, permanently.
#   2. The client's Engine.handleRelayUpdate() drops AuthAlgo, so a refreshed
#      algo-2 token is stamped algo-1 and validated against the wrong preimage
#      shape — breaks refresh independently of (1).
#   3. pushNewRelayTokens() (the relay-only refresh ticker) was never patched at
#      all: it still mints unbound tokens and omits TokenAuthAlgo.
#
# The patch's own lockout-guard test does not catch any of these because it calls
# the generator directly instead of driving the push/refresh callers.
#
# Lift this fuse only when all three are fixed AND a test drives the refresh
# path end-to-end (mint at refresh -> present as the client would -> validate
# with the relay's validator).
#
# The management build does not carry the patch from upstream v0.83.2 (upstream
# #8493 is its upstream form). Management then mints no peer-bound tokens at all,
# so the relay-side toggle must stay false for the same reason: requiring
# peer-bound tokens would refuse every peer. Check 3 below is skipped while the
# patch file is absent; checks 1 and 2 still apply.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

fail=0

# Match only real ASSIGNMENTS to a truthy value — `key: true` / `key=1` at the
# start of a line. Comments, Jinja conditionals that merely reference the var,
# and explicit falses are all legitimate and must not trip the fuse.
truthy_assignment() {
  grep -rnE "^[[:space:]]*(- )?$1[[:space:]]*[:=][[:space:]]*[\"']?(true|yes|on|1)[\"']?[[:space:]]*(#.*)?$" \
    "${@:2}" 2>/dev/null || true
}

# 1. Ansible: the relay-side enforcement toggle must stay false everywhere.
while IFS= read -r hit; do
  file="${hit%%:*}"
  echo "::error::$file: goat_relay_require_peer_bound is set true — ADR 1021 Phase C is FUSED."
  echo "  ${hit#*:}"
  fail=1
done < <(truthy_assignment "goat_relay_require_peer_bound" ops/ansible/)

# 2. Management-side: PeerBoundTokens must not be turned on in committed config.
while IFS= read -r hit; do
  file="${hit%%:*}"
  case "$file" in
    */ops/netbird/patches/*|*/docs/*|*/ops/ci/*) continue ;;
  esac
  echo "::error::$file: peer-bound tokens set true — ADR 1021 Phase C is FUSED."
  echo "  ${hit#*:}"
  fail=1
done < <(truthy_assignment "(relay_)?peer_bound_tokens" ops/ ; \
         truthy_assignment "PeerBoundTokens" ops/)

# 3. The fuse is only meaningful while the patch is still broken. If someone
#    fixes the refresh path, this check should be updated in the same change —
#    so assert the known-bad line is still present, and say so loudly if not.
patch=ops/netbird/patches/peer-id-bound-relay-tokens.patch
if [ -f "$patch" ]; then
  if ! grep -q 'token, err := m.GenerateRelayToken(peerID)' "$patch"; then
    echo "::notice::$patch no longer contains the known-bad GenerateRelayToken(peerID) refresh call."
    echo "  If the refresh paths are genuinely fixed (peer.Key threaded through SetupRefresh,"
    echo "  pushNewRelayTokens patched, AuthAlgo carried in handleRelayUpdate, and a test that"
    echo "  drives the refresh path end-to-end), retire this fuse in the same change — see F-276."
  fi
fi

if [ "$fail" -ne 0 ]; then
  echo
  echo "ADR 1021 Phase C is fused shut. See docs/findings/F-276-*.md for the three refresh"
  echo "breakages and what a correct fix requires. Dual-accept (the default) is safe: a broken"
  echo "refresh silently degrades to unbound tokens instead of locking peers out."
  exit 1
fi

echo "peer-bound-token gate: OK (Phase C not enabled anywhere)"
