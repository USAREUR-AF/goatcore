#!/usr/bin/env bash
# units/upgrade.sh — gate row E2, acceptance half (docs/design/goatbuilder-v1-gate.md): a
# peering finished by a released goatbuilder is accepted by the build under test.
#
# Both sovereigns are stood up by the released binary named in FE2E_UPGRADE_FROM (a file;
# its sha256 must equal FE2E_UPGRADE_FROM_SHA256), each by `goatbuilder federate <partner>`
# through phase 14, as units/onecmd.sh drives it. Then each driver host's goatbuilder is
# replaced by the build under test, nothing of phases 1-13 runs again, and phase 14 runs once
# more on each side against the same working directory, state.json, journal and agreement.
# Sourced by lib/unit.sh. Scenario: finished-on-release-accepted-by-build.
# shellcheck source=/dev/null
. "$HERE/units/onecmd.sh"

: "${FE2E_UPGRADE_FROM:?units/upgrade.sh: FE2E_UPGRADE_FROM names the released goatbuilder binary to stand up with}"
: "${FE2E_UPGRADE_FROM_SHA256:?units/upgrade.sh: FE2E_UPGRADE_FROM_SHA256 is that binary s published sha256}"
UP_BUILD="$GB"

up_work() { echo "/var/lib/goat-federation/$1"; } # $1 them

unit_main() {
  local s them h got
  got="$(sha256sum "$FE2E_UPGRADE_FROM" | cut -d' ' -f1)"
  [ "$got" = "$FE2E_UPGRADE_FROM_SHA256" ] || die "units/upgrade.sh: $FE2E_UPGRADE_FROM has sha256 $got, not the published $FE2E_UPGRADE_FROM_SHA256"
  # The hosts mount $GB: the released binary until the swap.
  cp "$FE2E_UPGRADE_FROM" "$WORK/goatbuilder-released"; chmod 0755 "$WORK/goatbuilder-released"
  GB="$WORK/goatbuilder-released"; export GB
  set_partners "$A" "$B"; set_partners "$B" "$A"; export NODIR=""
  export_helpers
  export -f oc_passed
  export OC_PREFIX HP ZENOH_IMAGE GB
  : > "$OUTSIDE"
  say "finished-on-release: $A and $B stood up by $("$GB" --version), each by \`goatbuilder federate <partner>\`"
  setup_hosts "$OC_PREFIX"
  prepare_edge "$A" "$B" true
  prepare_edge "$B" "$A" false
  for s in "$A" "$B"; do oc_core_staging "$s"; done
  if [ "$FE2E_CORE" = role ]; then oc_core_declare_role "$A" "$B"; oc_core_declare_role "$B" "$A"; fi
  for s in "$A" "$B"; do
    them="$(other "$s")"
    host_exec "$HP-$s-core" sh -c 'cat > /usr/local/bin/fed-responder.py' < "$SRC/ops/federation/roundtrip/responder.py"
    docker exec -d "$HP-$s-core" python3 /usr/local/bin/fed-responder.py "from/$them" "release/all"
  done
  oc_drive "$A" "$B"
  oc_status "$A" "$B"; oc_status "$B" "$A"
  for s in "$A" "$B"; do
    them="$(other "$s")"; h="$(drv_host "$s")"
    rec "up-$s-stood-up-by-the-release" --expect-in "$("$GB" --version)" -- docker exec "$h" goatbuilder --version
    rec "up-$s-release-passed-phases-1-to-13" -- bash -c "oc_passed '$RECORD/status-$s.json' 1 2 3 4 5 6 7 8 9 10 11 12 13"
  done
  # Phase 14 by the release, each side: its result is kept beside the build's.
  for s in "$A" "$B"; do
    them="$(other "$s")"; local arc=0
    oc_accept "$s" "$them" > "$RECORD/acceptance-by-release-as-$s.out" 2>&1 || arc=$?
    rec "up-$s-accepted-by-the-release" --expect-in "acceptance-counts:" --expect-in "skip=0" --expect-in "fail=0" --expect-not-in "ACCEPTANCE FAILED" \
      -- bash -c "cat '$RECORD/acceptance-by-release-as-$s.out'; exit $arc"
    host_exec "$(drv_host "$s")" sh -c "cd $(up_work "$them") && sha256sum state.json agreement.json 2>/dev/null; ls -1 evidence/acceptance 2>/dev/null" > "$RECORD/up-$s-work-before-the-swap.txt" || true
  done

  # The swap: the build under test replaces goatbuilder on each driver host. Nothing else.
  GB="$UP_BUILD"; export GB
  for s in "$A" "$B"; do replace_tools "$(drv_host "$s")" "$(goat_of "$s")" "$UP_BUILD"; done
  for s in "$A" "$B"; do
    them="$(other "$s")"; h="$(drv_host "$s")"
    rec "up-$s-driver-runs-the-build" --expect-in "$("$UP_BUILD" --version)" -- docker exec "$h" goatbuilder --version
    rec "up-$s-state-is-the-release-s" --expect-in "goatbuilder.federate.state/v1" -- docker exec "$h" sh -c "head -c 400 $(up_work "$them")/state.json"
  done

  # Phase 14 by the build, against the release's working directory.
  for s in "$A" "$B"; do
    them="$(other "$s")"; local brc=0 out="$RECORD/acceptance-by-build-as-$s.out"
    oc_accept "$s" "$them" > "$out" 2>&1 || brc=$?
    rec "up-$s-accepted-by-the-build" --expect-in "acceptance-counts:" --expect-in "skip=0" --expect-in "fail=0" \
      --expect-not-in "ACCEPTANCE FAILED" --expect-not-in "BLOCKED" -- bash -c "cat '$out'; exit $brc"
    # A refusal that names the journal, the lock or a schema means the build did not read
    # what the release wrote (GB-182: a state.json it does not read).
    rec "up-$s-build-read-the-release-s-record" -- bash -c \
      "l=\$(sed 's/\x1b\[[0-9;]*m//g' '$out' | grep -iE 'GB-182|refus[a-z]*.*(journal|state\.json|lock|schema|evidence)|(journal|state\.json|lock|schema).*refus' | head -5)
       [ -z \"\$l\" ] && { echo 'no refusal names the journal, the lock, the state or the evidence schema'; exit 0; }; echo \"\$l\"; exit 1"
  done
  collect "$OC_PREFIX"
}
