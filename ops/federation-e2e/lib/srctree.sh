#!/usr/bin/env bash
# srctree.sh — the source tree of the commit under test (sourced by ../env).
#
# src_tree SRC_GIT_DIR SHA DEST: DEST is a clone of SRC_GIT_DIR, checked out at SHA, that holds
# every object of SHA itself, with the source's refs and tags (units/upgrade.sh checks out a
# release tag from it, and each sovereign's repository is cloned from it).
#
# A clone made with --shared owned no objects and read them from the source through
# alternates. From a shallow source git ignores --shared (and --local) and clones by transfer:
# no alternates, and only the objects the source's refs reach. A commit fetched by sha that no
# ref holds was then missing, and the checkout failed with "unable to read tree". So the clone
# is a plain one (hard-linked objects from a full source, a transfer from a shallow one), and
# when it cannot read SHA's tree, SHA is fetched into it from the source. A clone kept from an
# earlier run is used only when it is checked out at SHA and can read SHA's tree; otherwise it
# is made again.
src_tree() {
  local common="$1" sha="$2" src="$3" head
  if [ -d "$src/.git" ]; then
    head="$(git -C "$src" rev-parse -q --verify HEAD 2>/dev/null || true)"
    if [ "$head" = "$sha" ] && git -C "$src" cat-file -e "$sha^{tree}" 2>/dev/null; then
      return 0
    fi
  fi
  rm -rf "$src"
  git clone -q --no-checkout "$common" "$src" || return 1
  if ! git -C "$src" cat-file -e "$sha^{tree}" 2>/dev/null; then
    git -C "$src" fetch -q --no-tags --update-shallow "$common" "$sha" || { rm -rf "$src"; return 1; }
  fi
  GIT_LFS_SKIP_SMUDGE=1 git -C "$src" checkout -q --detach "$sha" || { rm -rf "$src"; return 1; }
}
