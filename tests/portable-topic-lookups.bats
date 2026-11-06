#!/usr/bin/env bats
# Exercise the real helpers without running portal requests or deployment steps.

@test "portable topic lookups preserve last-wins, literal keys and empty registries" {
  local repo="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
  local script helper keys ids
  for script in \
    ops/zenoh-trial-peers/bootstrap-notional-portal.sh \
    ops/zenoh-trial-peers/bootstrap-airdefenders-portal.sh \
    ops/zenoh-trial-peers/synth-vendor-schemas/bootstrap-notional-schemas.sh; do
    helper=existing_topic_id_for; keys=EXISTING_TOPIC_KEYS; ids=EXISTING_TOPIC_IDS
    if [[ "$script" == *bootstrap-notional-schemas.sh ]]; then
      helper=topic_id_for; keys=TOPIC_KEYS; ids=TOPIC_IDS
    fi
    awk -v name="$helper" '$0 == name "() {" {emit=1} emit {print} emit && $0 == "}" {exit}' \
      "$repo/$script" > "$BATS_TEST_TMPDIR/helper.sh"
    run "${BASH_UNDER_TEST:-/bin/bash}" -euo pipefail -c '
      source "$1"
      eval "$3=(); $4=()"
      if "$2" absent; then exit 10; fi
      eval "$3=(duplicate other duplicate \"literal * [key]\"); $4=(first middle last literal)"
      [ "$("$2" duplicate)" = last ]
      [ "$("$2" other)" = middle ]
      [ "$("$2" "literal * [key]")" = literal ]
      if "$2" missing; then exit 11; fi
    ' _ "$BATS_TEST_TMPDIR/helper.sh" "$helper" "$keys" "$ids"
    [ "$status" -eq 0 ]
  done
}
