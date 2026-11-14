#!/usr/bin/env bash
# srctree.sh — the source tree of the commit under test (sourced by ../env).
#
# src_tree SRC_GIT_DIR SHA DEST: DEST is a clone of SRC_GIT_DIR checked out at SHA.
src_tree() {
  local common="$1" sha="$2" src="$3"
  [ -d "$src/.git" ] && return 0
  rm -rf "$src"
  git clone -q --shared --no-checkout "$common" "$src"
  GIT_LFS_SKIP_SMUDGE=1 git -C "$src" checkout -q --detach "$sha"
}
