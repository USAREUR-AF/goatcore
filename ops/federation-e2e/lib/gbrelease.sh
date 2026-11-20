#!/usr/bin/env bash
# The goatbuilder a unit runs (sourced by env). By default it is the one built from the ref.
# With --goatbuilder-version VER (GB_VERSION), it is the published release asset instead,
# verified against its .sha256 by release_tool, so a run measures the binary operators
# install, not a local build of the same commit (the two differ byte for byte).
#
# The rest of the tree under test (roles, playbooks, catalog) still comes from the ref, so the
# ref must be that release: its crates/goatbuilder version must equal VER, or the run is refused.
# Needs crate_version and release_tool from env.

gb_under_test() { # $1 sha -> the goatbuilder path a unit runs, or fails with the reason on stderr
  local sha="$1" v p
  if [ -z "${GB_VERSION:-}" ]; then echo "$FE2E_HOME/bin/$sha/goatbuilder"; return 0; fi
  v="$(crate_version "$FE2E_HOME/src/$sha" goatbuilder)"
  if [ "$v" != "$GB_VERSION" ]; then
    echo "--goatbuilder-version $GB_VERSION: the ref ${sha:0:12} is goatbuilder ${v:-of no version}, not $GB_VERSION; run the release's own commit" >&2
    return 1
  fi
  p="$(release_tool goatbuilder "$GB_VERSION")" || return 1
  echo "$p"
}

gb_tools() { # $1 the path gb_under_test gave -> tools.txt words naming the release, nothing for a build
  [ -n "${GB_VERSION:-}" ] || return 0
  echo "GOATBUILDER_RELEASE=$GB_VERSION"
  echo "GOATBUILDER_SHA256=$(sha256sum "$1" | cut -d' ' -f1)"
}
