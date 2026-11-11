#!/usr/bin/env bash
# units/mixed.sh — mixed versions: {A} drives with a released goat through the script avenue
# (peering-standup.sh from its own clone of the ref, which calls `goat federate` through
# GOAT_BIN), {B} with the ref's goatbuilder federate. Sourced by lib/unit.sh.
#
# The released goat comes from env: GOAT_<A>=<path> (or unavailable:<version>) and
# GOAT_VERSION_<A>=<version>, from `env run --goat-version {A}=<version>` or, when that is not
# given, the catalog's units.mixed.goat_versions. The engine mounts it into {A}'s hosts
# only (goat_of). The scenario reads: mixed-versions.
mixed_released() { local v="GOAT_$A"; echo "${!v:-}"; }
mixed_version() { local v="GOAT_VERSION_$A"; echo "${!v:-}"; }

unit_main() {
  local n=4 rel ver refver
  rel="$(mixed_released)"; ver="$(mixed_version)"
  [ -n "$rel" ] || die "no released goat for $A: run env with --goat-version $A=<version>"
  case "$rel" in unavailable:*) die "goat ${rel#unavailable:} is not published as a release asset" ;; esac
  [ -n "$ver" ] || ver="$("$rel" --version | awk '{print $2}')"
  refver="$("$GOAT" --version | awk '{print $2}')"
  printf 'released goat for %s: %s (%s)\nref goat for %s: %s\n' "$A" "$ver" "$rel" "$B" "$refver" > "$RECORD/mv-versions.txt"

  set_partners "$A" "$B"; set_partners "$B" "$A"; export NODIR=""
  export_helpers
  setup_hosts "$n"
  rec "mv-$A-goat-version" --expect-in "goat $ver" -- docker exec "$HP-$A-edge" /usr/local/bin/goat --version
  rec "mv-$B-goat-version" --expect-in "goat $refver" -- docker exec "$HP-$B-edge" /usr/local/bin/goat --version
  # {A} listens and goes first, through the script avenue; {B} through goatbuilder.
  peer_up "$n" "$A" "$B" script goatbuilder
  collect "$n"
}
