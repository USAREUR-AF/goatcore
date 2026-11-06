#!/usr/bin/env bats

# rc.sh from an assembled tree (a checkout with the .goat-assembled marker an
# overlay's assemble.sh writes). The goatnets' pins and records belong to the
# overlay, so the run lands its edits there; these cases pin the refusals that
# keep it from acting on a tree no deploy accepts, or cutting a tag where the
# image is not built.

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"

setup() {
  TMP="$(mktemp -d)"
  TREE="$TMP/tree"; OVL="$TMP/overlay"
  mkdir -p "$TREE/ops/scripts" "$OVL"
  cp "$REPO_ROOT/ops/scripts/rc.sh" "$TREE/ops/scripts/rc.sh"
  git -C "$TREE" init -q
  git -C "$TREE" remote add origin https://example.invalid/core/core.git
  git -C "$OVL" init -q
  git -C "$OVL" remote add origin https://example.invalid/team/overlay.git
  printf '#!/usr/bin/env bash\n' > "$OVL/assemble.sh"; chmod +x "$OVL/assemble.sh"
}

teardown() {
  rm -rf "$TMP"
}

marker() { # $1 goatcore_ref, $2 overlay_path
  printf 'goatcore_ref=%s\ngoatstuff_ref=%s\noverlay_commit=%s\noverlay_path=%s\n' \
    "$1" "$1" 0123456789abcdef0123 "$2" > "$TREE/.goat-assembled"
}

rc() {
  GOAT_RC_ROLE_CHECK=0 bash "$TREE/ops/scripts/rc.sh" portal --ack "Bats Tester" --dry-run "$@"
}

@test "a tree assembled from local directories is refused" {
  marker local "$OVL"
  run rc --tag portal/v1.0.0-rc.1 --from dev
  [ "$status" -ne 0 ]
  [[ "$output" == *"assembled from local directories"* ]]
}

@test "a marker naming no overlay checkout is refused" {
  marker 0123456789abcdef0123456789abcdef01234567 "$TMP/missing"
  run rc --tag portal/v1.0.0-rc.1 --from dev
  [ "$status" -ne 0 ]
  [[ "$output" == *"names no overlay checkout"* ]]
}

@test "cutting the tag is refused from an assembled tree" {
  marker 0123456789abcdef0123456789abcdef01234567 "$OVL"
  run rc --tag portal/v1.0.0-rc.1
  [ "$status" -ne 0 ]
  [[ "$output" == *"--from tag cuts the tag on this tree's origin"* ]]
}

@test "an assembled tree needs the candidate named with --tag" {
  marker 0123456789abcdef0123456789abcdef01234567 "$OVL"
  run rc --version 1.0.0 --from dev
  [ "$status" -ne 0 ]
  [[ "$output" == *"name the candidate with --tag"* ]]
}

@test "a single-repository checkout off main is still refused" {
  git -C "$TREE" checkout -q -b feature
  run rc --tag portal/v1.0.0-rc.1 --from dev
  [ "$status" -ne 0 ]
  [[ "$output" == *"run rc.sh from a checkout on main"* ]]
}
