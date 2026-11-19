#!/bin/sh
# apply-patch.sh — idempotently inject FOUR patches into kratos-selfservice-ui-node's
# compiled consent handler:
#
#   1. goat-maintainer-groups-forwarding-patch (documented in patches/0001-...):
#      forward the login groups (admin-managed metadata_public.groups since
#      F-572 / ADR 1188; never traits.groups, GHSA-9v3v)
#      into session.id_token.groups so the JWT carries the groups claim every
#      downstream RBAC consumer needs.
#
#   4. goat-maintainer-aff-forwarding-patch (documented in patches/0004-...):
#      forward the aff block (metadata_public.aff since F-572 / ADR 1188)
#      into session.id_token.aff — the affiliation
#      dimension of the identity grammar (ADR 1043), so a relying party can
#      authorize by WHO an identity belongs to without it leaking into the
#      role/groups checks. Additive; a no-op for identities with no aff trait.
#      (Numbered 4 but applied next to patch 1 since both forward id_token
#      traits at the same anchor.)
#
#   2. goat-maintainer-audience-fallback-patch (documented in patches/0002-...):
#      fall back to body.client.audience when Hydra delivers an empty
#      requested_access_token_audience in the consent challenge body —
#      otherwise tokens land with aud:[] and every JWT validator rejects
#      with "token invalid".
#
#   3. goat-maintainer-scope-fallback-patch (documented in patches/0003-...):
#      fall back to body.client.scope.split(' ') when grantScope is
#      empty on the acceptOAuth2ConsentRequest call. Same shape as the
#      audience-fallback bug, different field: when an OAuth2 client
#      has `skip_consent: true` (F-077 mitigation for first-party
#      clients), Hydra delivers an empty body.requested_scope in the
#      consent challenge, so the upstream code lands `grant_scope: []`
#      in the stored consent session. Hydra then strips ALL claims
#      from the /userinfo response for scopes that weren't granted —
#      including the groups claim that patch 1 puts there — so every
#      consumer sees an empty userinfo and rejects the login. See
#      F-161 for the full root-cause trace (Grafana "user not a
#      member of one of the required groups" symptom).
#
# All three patches are unified-diff documented in patches/ for human
# review, but applied here via awk text transforms so we don't need a
# node toolchain in the final image. Each patch carries an idempotency
# marker comment so re-running this script is a no-op when the marker
# is already present.
#
# Portable POSIX awk only; no python/sed-multiline / other tooling assumed.
#
# Usage:
#   apply-patch.sh /usr/src/app/lib/routes/consent.js

set -eu

TARGET="${1:-/usr/src/app/lib/routes/consent.js}"

if [ ! -f "$TARGET" ]; then
  echo "apply-patch.sh: target not found: $TARGET" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Patch 1: groups-forwarding (insert before identity.updated_at handler)
# ---------------------------------------------------------------------------

MARKER1='goat-maintainer-groups-forwarding-patch'
ANCHOR1='if (identity.updated_at)'

if grep -q "$MARKER1" "$TARGET"; then
  # GHSA-9v3v: an older copy of this patch fell back to identity.traits.groups.
  # The marker alone does not make a target current; refuse a stale one.
  if grep -q "identity\.traits\.groups" "$TARGET"; then
    echo "apply-patch.sh: $TARGET carries the pre-GHSA-9v3v groups-forwarding patch (reads identity.traits.groups); rebuild from the upstream image" >&2
    exit 1
  fi
  echo "apply-patch.sh: groups-forwarding marker already present; skipping patch 1"
else
  if ! grep -q "$ANCHOR1" "$TARGET"; then
    echo "apply-patch.sh: anchor '$ANCHOR1' not found in $TARGET — upstream layout changed; re-author patch 0001" >&2
    exit 1
  fi

  TMP="${TARGET}.patched1.$$"
  awk -v anchor="$ANCHOR1" -v marker="$MARKER1" '
  BEGIN { inserted = 0 }
  {
    if (!inserted && index($0, anchor) > 0) {
      indent = $0
      sub(/[^[:space:]].*/, "", indent)
      print indent "// F-572 / ADR 1188, GHSA-9v3v: groups come ONLY from admin-managed metadata_public."
      print indent "// Self-editable traits never grant a role, even for a not-yet-backfilled identity."
      print indent "// " marker " — forward the groups claim into the JWT id_token."
      print indent "var __gm = identity.metadata_public || {}, __groups = __gm.groups;"
      print indent "if (Array.isArray(__groups) && __groups.length > 0) session.id_token.groups = __groups;"
      print ""
      inserted = 1
    }
    print
  }
  END {
    if (!inserted) {
      print "apply-patch.sh: groups-forwarding anchor not matched during awk pass" > "/dev/stderr"
      exit 1
    }
  }
  ' "$TARGET" > "$TMP"
  mv "$TMP" "$TARGET"
  echo "apply-patch.sh: applied groups-forwarding patch to $TARGET"
fi

# ---------------------------------------------------------------------------
# Patch 4: aff-forwarding (insert before identity.updated_at handler, same
# anchor as patch 1 — forward identity.traits.aff into the JWT id_token so a
# relying party can authorize by AFFILIATION without it leaking into role/
# groups checks). Identity grammar Phase D / ADR 1043. Additive: a no-op for
# identities that carry no aff trait (everything minted before Phase D).
# ---------------------------------------------------------------------------

MARKER4='goat-maintainer-aff-forwarding-patch'
ANCHOR4='if (identity.updated_at)'

if grep -q "$MARKER4" "$TARGET"; then
  # GHSA-9v3v: an older copy of this patch fell back to identity.traits.aff.
  # The marker alone does not make a target current; refuse a stale one.
  if grep -q "identity\.traits\.aff" "$TARGET"; then
    echo "apply-patch.sh: $TARGET carries the pre-GHSA-9v3v aff-forwarding patch (reads identity.traits.aff); rebuild from the upstream image" >&2
    exit 1
  fi
  echo "apply-patch.sh: aff-forwarding marker already present; skipping patch 4"
else
  if ! grep -q "$ANCHOR4" "$TARGET"; then
    echo "apply-patch.sh: anchor '$ANCHOR4' not found in $TARGET — upstream layout changed; re-author patch 0004" >&2
    exit 1
  fi

  TMP="${TARGET}.patched4.$$"
  awk -v anchor="$ANCHOR4" -v marker="$MARKER4" '
  BEGIN { inserted = 0 }
  {
    if (!inserted && index($0, anchor) > 0) {
      indent = $0
      sub(/[^[:space:]].*/, "", indent)
      print indent "// F-572 / ADR 1188, GHSA-9v3v: aff comes ONLY from admin-managed metadata_public;"
      print indent "// self-editable traits are never forwarded."
      print indent "// " marker " — forward the aff claim into the JWT id_token."
      print indent "var __am = identity.metadata_public || {}, __aff = __am.aff;"
      print indent "if (__aff && typeof __aff === \"object\" && Object.keys(__aff).length > 0) session.id_token.aff = __aff;"
      print ""
      inserted = 1
    }
    print
  }
  END {
    if (!inserted) {
      print "apply-patch.sh: aff-forwarding anchor not matched during awk pass" > "/dev/stderr"
      exit 1
    }
  }
  ' "$TARGET" > "$TMP"
  mv "$TMP" "$TARGET"
  echo "apply-patch.sh: applied aff-forwarding patch to $TARGET"
fi

# ---------------------------------------------------------------------------
# Patch 2: audience-fallback (literal-string-replace the two audience-echo
# assignments on the consent-accept body)
# ---------------------------------------------------------------------------

MARKER2='goat-maintainer-audience-fallback-patch'
OLD2='grant_access_token_audience: body.requested_access_token_audience,'
NEW2='grant_access_token_audience: ((body.requested_access_token_audience && body.requested_access_token_audience.length) ? body.requested_access_token_audience : ((body.client && body.client.audience) || [])), // goat-maintainer-audience-fallback-patch'

if grep -q "$MARKER2" "$TARGET"; then
  echo "apply-patch.sh: audience-fallback marker already present; skipping patch 2"
else
  if ! grep -qF "$OLD2" "$TARGET"; then
    echo "apply-patch.sh: anchor for audience-fallback not found in $TARGET — upstream layout changed; re-author patch 0002" >&2
    exit 1
  fi

  # Literal string replace via awk (no regex escaping required).
  TMP="${TARGET}.patched2.$$"
  awk -v old="$OLD2" -v new="$NEW2" '
  {
    line = $0
    out = ""
    while ( (i = index(line, old)) > 0 ) {
      out = out substr(line, 1, i-1) new
      line = substr(line, i + length(old))
    }
    print out line
  }
  ' "$TARGET" > "$TMP"

  # Sanity: confirm both occurrences were rewritten.
  expected=$(grep -cF "$NEW2" "$TMP" || true)
  if [ "$expected" -lt 2 ]; then
    echo "apply-patch.sh: audience-fallback patch produced only $expected replacement(s); expected 2 (skip-consent + full-consent handlers)" >&2
    rm -f "$TMP"
    exit 1
  fi

  mv "$TMP" "$TARGET"
  echo "apply-patch.sh: applied audience-fallback patch to $TARGET ($expected occurrences)"
fi

# ---------------------------------------------------------------------------
# Patch 3: scope-fallback (literal-string-replace the two grant_scope
# assignments on the consent-accept body)
# ---------------------------------------------------------------------------

MARKER3='goat-maintainer-scope-fallback-patch'
OLD3='grant_scope: grantScope,'
NEW3='grant_scope: ((grantScope && grantScope.length) ? grantScope : ((body.client && body.client.scope) ? body.client.scope.split(" ") : [])), // goat-maintainer-scope-fallback-patch'

if grep -q "$MARKER3" "$TARGET"; then
  echo "apply-patch.sh: scope-fallback marker already present; skipping patch 3"
else
  if ! grep -qF "$OLD3" "$TARGET"; then
    echo "apply-patch.sh: anchor for scope-fallback not found in $TARGET — upstream layout changed; re-author patch 0003" >&2
    exit 1
  fi

  # Literal string replace via awk (no regex escaping required).
  TMP="${TARGET}.patched3.$$"
  awk -v old="$OLD3" -v new="$NEW3" '
  {
    line = $0
    out = ""
    while ( (i = index(line, old)) > 0 ) {
      out = out substr(line, 1, i-1) new
      line = substr(line, i + length(old))
    }
    print out line
  }
  ' "$TARGET" > "$TMP"

  # Sanity: confirm both occurrences were rewritten (skip-consent +
  # full-consent handlers, mirroring patch 2's two-spot pattern).
  expected=$(grep -cF "$NEW3" "$TMP" || true)
  if [ "$expected" -lt 2 ]; then
    echo "apply-patch.sh: scope-fallback patch produced only $expected replacement(s); expected 2 (skip-consent + full-consent handlers)" >&2
    rm -f "$TMP"
    exit 1
  fi

  mv "$TMP" "$TARGET"
  echo "apply-patch.sh: applied scope-fallback patch to $TARGET ($expected occurrences)"
fi
