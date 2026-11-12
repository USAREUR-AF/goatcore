#!/usr/bin/env bash
# check-rust-toolchain-pinned.sh: refuse a Rust toolchain that floats in a workflow.
#
# WHY THIS EXISTS. A CI job's toolchain is a declared input, never whatever
# was released this week (docs/design/ci-results-follow-inputs.md, rule 1).
# `dtolnay/rust-toolchain@stable` or `toolchain: stable` takes the newest
# release on the day the job runs. A new release with a new lint then fails
# a pull request that changed nothing about Rust, as Rust 1.100.0 did. A
# toolchain is moved on purpose, in a change of its own.
#
# WHAT IS ALLOWED:
#   - dtolnay/rust-toolchain@<x.y.z> or @<40-hex commit>
#   - toolchain: '<x.y.z>' (quoted or not)
# Refused: @stable, @beta, @nightly, and `toolchain:` set to a channel name.
set -euo pipefail
cd "$(dirname "$0")/../.."

# The guard proves itself red against a broken fixture before its verdict on
# the real tree is trusted.
if [ "${1:-}" = "--self-test" ]; then
  tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
  mkdir -p "$tmp/.github/workflows" "$tmp/ops/ci"
  cp "$0" "$tmp/ops/ci/"
  for bad in 'uses: dtolnay/rust-toolchain@stable' \
             'uses: dtolnay/rust-toolchain@nightly' \
             "toolchain: stable" "toolchain: 'beta'"; do
    printf 'jobs:\n  a:\n    steps:\n      - %s\n' "$bad" > "$tmp/.github/workflows/w.yml"
    if ( cd "$tmp" && bash ops/ci/"$(basename "$0")" >/dev/null 2>&1 ); then
      echo "SELF-TEST FAILED: the guard passed a floating toolchain: $bad"
      exit 1
    fi
  done
  printf 'jobs:\n  a:\n    steps:\n      - uses: dtolnay/rust-toolchain@1.100.0\n      - with:\n          toolchain: %s\n' "'1.88.0'" \
    > "$tmp/.github/workflows/w.yml"
  if ! ( cd "$tmp" && bash ops/ci/"$(basename "$0")" >/dev/null 2>&1 ); then
    echo "SELF-TEST FAILED: the guard refused a pinned toolchain."
    exit 1
  fi
  echo "check-rust-toolchain-pinned: self-test ok"
  exit 0
fi

bad="$(grep -rnE \
  "rust-toolchain@(stable|beta|nightly)\b|toolchain:[[:space:]]*['\"]?(stable|beta|nightly)['\"]?[[:space:]]*(#.*)?$" \
  .github/workflows .github/actions 2>/dev/null || true)"
if [ -n "$bad" ]; then
  echo "check-rust-toolchain-pinned: a workflow takes a floating Rust toolchain:"
  printf '%s\n' "$bad" | sed 's/^/  /'
  echo "Name an exact release (dtolnay/rust-toolchain@x.y.z, or toolchain: 'x.y.z'),"
  echo "and move it on purpose in a change of its own."
  exit 1
fi
echo "check-rust-toolchain-pinned: ok, every Rust toolchain in the workflows is pinned"
