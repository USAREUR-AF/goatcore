#!/bin/sh
# verify-patched.sh — CI smoke test: spawn the built image and confirm ALL
# FOUR patches are present in the running consent handler:
#
#   1. goat-maintainer-groups-forwarding-patch — metadata_public.groups → id_token.groups
#   4. goat-maintainer-aff-forwarding-patch    — metadata_public.aff → id_token.aff (ADR 1043 Phase D)
#   2. goat-maintainer-audience-fallback-patch — fall back to client.audience when
#      requested_access_token_audience is empty
#   3. goat-maintainer-scope-fallback-patch — fall back to client.scope.split(' ')
#      when grantScope is empty on acceptOAuth2ConsentRequest (F-161 —
#      the empty-granted_scope-strips-userinfo-claims gap that blocked
#      Grafana login under skip_consent: true clients).
#
# Used by the ory-consent-image.yml workflow as a post-build gate so a
# silently-broken patch can't ship.
#
# Patches 1 and 4 are checked for the FIXED reader shape (GHSA-9v3v), not only
# the marker: the claim comes from metadata_public alone, and consent.js holds
# no identity.traits.groups / identity.traits.aff anywhere. A stale image
# carrying the pre-GHSA-9v3v traits fallback fails here.
#
# Usage: verify-patched.sh <image-ref>
#        VERIFY_PATCHED_ROOT=<dir> verify-patched.sh   (tests: read <dir>/lib/...,
#        <dir>/views/... instead of running the image)

set -eu

if [ -n "${VERIFY_PATCHED_ROOT:-}" ]; then
  IMAGE="${1:-$VERIFY_PATCHED_ROOT}"
else
  IMAGE="${1:?usage: verify-patched.sh <image-ref>}"
fi

# in_image CMD: run CMD with $APP set to the app root, inside the built image
# (or against VERIFY_PATCHED_ROOT when set).
in_image() {
  if [ -n "${VERIFY_PATCHED_ROOT:-}" ]; then
    APP="$VERIFY_PATCHED_ROOT" /bin/sh -c "$1"
  else
    docker run --rm --entrypoint=/bin/sh -e APP=/usr/src/app "$IMAGE" -c "$1"
  fi
}

echo "verify-patched.sh: inspecting $IMAGE"

# GHSA-9v3v: no reader of self-editable traits for groups or aff may remain.
outT=$(in_image '
  grep -n -e "identity\.traits\.groups" -e "identity\.traits\.aff" "$APP/lib/routes/consent.js" || true
' 2>&1)
if [ -n "$outT" ]; then
  echo "verify-patched.sh: FAIL — consent.js still reads identity.traits.groups/aff (pre-GHSA-9v3v reader)" >&2
  echo "$outT" >&2
  exit 1
fi

# Patch 1: groups-forwarding
out1=$(in_image '
  grep -A 2 "goat-maintainer-groups-forwarding-patch" "$APP/lib/routes/consent.js"
' 2>&1) || {
  echo "verify-patched.sh: FAIL — groups-forwarding marker not found in built image" >&2
  echo "$out1" >&2
  exit 1
}
case "$out1" in
  *'__groups = __gm.groups;'*'session.id_token.groups = __groups'*) ;;
  *)
    echo "verify-patched.sh: FAIL — groups marker present but not the metadata_public-only reader (GHSA-9v3v)" >&2
    echo "$out1" >&2
    exit 1
    ;;
esac
echo "verify-patched.sh: groups-forwarding patch present"

# Patch 2: audience-fallback
out2=$(in_image '
  grep "goat-maintainer-audience-fallback-patch" "$APP/lib/routes/consent.js"
' 2>&1) || {
  echo "verify-patched.sh: FAIL — audience-fallback marker not found in built image" >&2
  echo "$out2" >&2
  exit 1
}
# Should appear exactly twice (once for skip-consent handler, once for full-consent handler).
count=$(printf '%s\n' "$out2" | grep -c 'goat-maintainer-audience-fallback-patch' || true)
if [ "$count" -lt 2 ]; then
  echo "verify-patched.sh: FAIL — expected audience-fallback marker on both consent handlers (got $count occurrences)" >&2
  echo "$out2" >&2
  exit 1
fi
case "$out2" in
  *'body.client.audience'*) ;;
  *)
    echo "verify-patched.sh: FAIL — audience marker present but body.client.audience fallback missing" >&2
    echo "$out2" >&2
    exit 1
    ;;
esac
echo "verify-patched.sh: audience-fallback patch present (on both handlers)"

# Patch 3: scope-fallback
out3=$(in_image '
  grep "goat-maintainer-scope-fallback-patch" "$APP/lib/routes/consent.js"
' 2>&1) || {
  echo "verify-patched.sh: FAIL — scope-fallback marker not found in built image" >&2
  echo "$out3" >&2
  exit 1
}
# Should appear exactly twice (once for skip-consent handler, once for full-consent handler).
count3=$(printf '%s\n' "$out3" | grep -c 'goat-maintainer-scope-fallback-patch' || true)
if [ "$count3" -lt 2 ]; then
  echo "verify-patched.sh: FAIL — expected scope-fallback marker on both consent handlers (got $count3 occurrences)" >&2
  echo "$out3" >&2
  exit 1
fi
case "$out3" in
  *'body.client.scope'*) ;;
  *)
    echo "verify-patched.sh: FAIL — scope marker present but body.client.scope fallback missing" >&2
    echo "$out3" >&2
    exit 1
    ;;
esac
echo "verify-patched.sh: scope-fallback patch present (on both handlers)"

# Patch 4: aff-forwarding (identity grammar Phase D / ADR 1043)
out4=$(in_image '
  grep -A 2 "goat-maintainer-aff-forwarding-patch" "$APP/lib/routes/consent.js"
' 2>&1) || {
  echo "verify-patched.sh: FAIL — aff-forwarding marker not found in built image" >&2
  echo "$out4" >&2
  exit 1
}
case "$out4" in
  *'__aff = __am.aff;'*'session.id_token.aff = __aff'*) ;;
  *)
    echo "verify-patched.sh: FAIL — aff marker present but not the metadata_public-only reader (GHSA-9v3v)" >&2
    echo "$out4" >&2
    exit 1
    ;;
esac
echo "verify-patched.sh: aff-forwarding patch present"

# Patch 5: double-submit guard on the shared auth-page scripts partial.
# This one lives in a VIEW, not in consent.js — a login form submitted twice
# orphans the OAuth2 login challenge and lands the operator on Ory's welcome
# page. Asserting the marker alone is not enough: the guard is only correct if
# it defers disabling the submit control, because Kratos carries
# `method=password` as the button's own name/value and a synchronously-disabled
# button is dropped from form serialization.
out5=$(in_image '
  cat "$APP/views/partials/scripts.hbs"
' 2>&1) || {
  echo "verify-patched.sh: FAIL — could not read scripts.hbs from built image" >&2
  echo "$out5" >&2
  exit 1
}
case "$out5" in
  *'goat-maintainer-double-submit-guard'*) ;;
  *)
    echo "verify-patched.sh: FAIL — double-submit-guard marker not found in built image" >&2
    echo "$out5" >&2
    exit 1
    ;;
esac
case "$out5" in
  *'data-dlf-submitted'*) ;;
  *)
    echo "verify-patched.sh: FAIL — guard marker present but the once-only attribute is missing" >&2
    exit 1
    ;;
esac
case "$out5" in
  *'setTimeout'*) ;;
  *)
    echo "verify-patched.sh: FAIL — guard present but disables the submit control SYNCHRONOUSLY." >&2
    echo "  That drops method=password from the POST body and breaks every login." >&2
    exit 1
    ;;
esac
# The scripts partial is included TWICE on the consent page (consent.hbs plus
# layouts/auth.hbs). A guard that registers once per include cancels the FIRST
# submit there, so "Allow" sends nothing (zagros).
case "$out5" in
  *'__dlfSubmitGuardInstalled'*) ;;
  *)
    echo "verify-patched.sh: FAIL — guard present but not once-per-document." >&2
    echo "  The consent page includes this partial twice; the second copy cancels the first Allow click." >&2
    exit 1
    ;;
esac
# Only a browser-fired submit may mark the form. A guard that counts a
# script-dispatched submit event cancels the real submit after a password
# manager's auto-submit, and the page sends nothing.
case "$out5" in
  *'isTrusted'*) ;;
  *)
    echo "verify-patched.sh: FAIL — guard present but counts script-dispatched submit events." >&2
    echo "  A password manager's auto-submit would then cancel the real submit." >&2
    exit 1
    ;;
esac
echo "verify-patched.sh: double-submit guard present (deferred disable, once per document, real submits only)"

echo "verify-patched.sh: PASS"
