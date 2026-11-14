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
  # A full sha, before anything is removed: with an empty one, DEST would be the directory of
  # every cached tree. (A case pattern, not =~, for bash 3.2.)
  case "$sha" in
    *[!0-9a-f]* | "") echo "src_tree: not a full sha: '$sha'" >&2; return 1 ;;
  esac
  [ "${#sha}" -eq 40 ] || { echo "src_tree: not a full sha: '$sha'" >&2; return 1; }
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

# src_clone_ref SRC_GIT_DIR REF DEST: DEST is a clone of SRC_GIT_DIR checked out at REF (a
# branch, a tag, or a sha the source holds), owning its objects: lib/watch.sh's preview merges
# and reverts in it, and units/upgrade.sh checks out a release tag in it. A --shared clone
# read every object from the source through alternates, so it lost a commit once the source
# dropped that commit's loose objects (git prune, git gc). A plain local clone hard-links the
# source's object files instead, and keeps them. A shallow source is refused: its history is
# cut, so a merge or a revert there would read too little, and a clone of it is cut the same
# way. Nothing is removed before the arguments and the source are checked.
src_clone_ref() {
  local common="$1" ref="$2" dest="$3"
  [ -n "$common" ] && [ -n "$ref" ] && [ -n "$dest" ] || { echo "src_clone_ref: needs SRC_GIT_DIR REF DEST" >&2; return 1; }
  if [ "$(git -C "$common" rev-parse --is-shallow-repository 2>/dev/null)" != false ]; then
    echo "src_clone_ref: $common is a shallow repository, or not one git can read: its history is cut, so a merge, a revert or a release checkout there would read too little; run git -C $common fetch --unshallow" >&2
    return 1
  fi
  rm -rf "$dest"
  git clone -q --no-checkout "$common" "$dest" || return 1
  GIT_LFS_SKIP_SMUDGE=1 git -C "$dest" checkout -q --detach "$ref" || { rm -rf "$dest"; return 1; }
}
