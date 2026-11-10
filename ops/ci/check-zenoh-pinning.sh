#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# check-zenoh-pinning.sh — project-wide zenoh version-of-record lint.
#
# Per docs/design/zenoh-version-pinning.md, zenoh appears in many places
# (Cargo deps, image pin in Ansible role defaults, design doc references).
# They must all agree on the version-of-record.
#
# This lint greps committed files for stray references to a non-pinned
# zenoh version. Fails if:
#   - A Cargo.toml has `zenoh = "1"` / `"1.9"` (range) rather than
#     `"=1.9.0"` (exact-pin).
#   - A deploy-target config references the retired project-built router
#     image `ghcr.io/<org>/zenoh` (any registry namespace). That image was retired in ADR 0858 step 5 in favor of
#     upstream `eclipse/zenoh` (since audit-logger moved out-of-process,
#     the supply-chain-build justification dissolved — F-122 closed).
#   - A deploy-target config references the upstream `eclipse/zenoh` image
#     by tag only (no `@sha256:...` digest pin). Tags are mutable; pin by
#     content digest per zenoh-version-pinning.md.
#
# Deliberately quiet on design-doc references that quote a version inside
# a "What NOT to use" / "Historical" / "Supersedes" section — those are
# legitimate descriptions of the wrong-pattern, not load-bearing pins.
# Skip-marker: a comment containing `zenoh-pin-lint: allow` on the same line.

set -euo pipefail

EXPECTED="1.9.0"
FAIL=0

scan() {
    local desc="$1"; shift
    local match
    match=$(grep -EHn "$@" 2>/dev/null | grep -v 'zenoh-pin-lint: allow' || true)
    if [ -n "$match" ]; then
        echo "[FAIL] $desc:"
        printf '  %s\n' "$match" | head -20
        FAIL=1
    fi
}

cd "$(git rev-parse --show-toplevel)"

# Cargo deps that use a range rather than exact-pin.
scan "Cargo dep uses zenoh range instead of =1.9.0" \
    --include='Cargo.toml' -r \
    'zenoh\s*=\s*"[^=]' \
    -- . \
    || true

# Retired project-built zenoh router image referenced in deploy config
# (ADR 0858 step 5 pivoted to upstream). Matched under any namespace, so the
# rule holds whichever organisation published it; `zenoh-pqc`, `zenoh-sandbox`
# and the other live images carry a suffix and do not match.
scan "Retired ghcr.io/<org>/zenoh image reference (ADR 0858 step 5 pivoted to upstream eclipse/zenoh)" \
    --include='*.yml' --include='*.yaml' --include='*.j2' -r \
    'ghcr\.io/[^/]+/zenoh:' \
    -- ops/ \
    || true

# Upstream image must be digest-pinned, not tag-only.
scan "Upstream eclipse/zenoh image referenced without @sha256: digest pin" \
    --include='*.yml' --include='*.yaml' --include='*.j2' -r \
    'eclipse/zenoh:[^@"'"'"' ]*\s*$|eclipse/zenoh:[^@]*"' \
    -- ops/ \
    || true

if [ "$FAIL" -eq 0 ]; then
    echo "[zenoh-pin-lint] PASS — all committed zenoh references agree on =${EXPECTED}."
fi
exit "$FAIL"
