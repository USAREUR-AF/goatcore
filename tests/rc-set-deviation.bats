#!/usr/bin/env bats

# rc.sh's set_deviation, on its own: the edit that puts a release candidate on a
# rung as a declared deviation in ops/pins/fleet-pins.yaml. Its reason carries
# the day it was declared, so a re-run on a later day (a run that crosses
# midnight) must leave a deviation that already declares the same value as it
# stands, or the dev step lands a pin PR for nothing.

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
OLD="ghcr.io/example-org/portal:v1.0.0@sha256:1111111111111111111111111111111111111111111111111111111111111111"
NEW="ghcr.io/example-org/portal:v1.1.0-rc.1@sha256:2222222222222222222222222222222222222222222222222222222222222222"

setup() {
  TMP="$(mktemp -d)"
  WT="$TMP/wt"
  # shellcheck disable=SC2034 # COMP and REF are read by the sourced set_deviation
  COMP=portal
  mkdir -p "$WT/ops/pins"
  cat > "$WT/ops/pins/fleet-pins.yaml" <<EOF
components:
  portal:
    var: portal_image
    # the fleet value
    fleet: "$OLD"
  other:
    var: other_image
    fleet: "$OLD"
EOF
  # The function as rc.sh defines it, with rc.sh's \`run\` reduced to running.
  sed -n '/^set_deviation() {/,/^}/p' "$REPO_ROOT/ops/scripts/rc.sh" > "$TMP/fn.sh"
  [ -s "$TMP/fn.sh" ]
}

teardown() {
  rm -rf "$TMP"
}

dev() { # $1 value, $2 reason
  run() { "$@"; }
  # shellcheck disable=SC2034
  REF="$1"
  # shellcheck disable=SC1091
  . "$TMP/fn.sh"
  set_deviation e2e_dev "$2"
}

@test "a first declaration adds the deviation with its reason" {
  dev "$NEW" "RC portal/v1.1.0-rc.1 (2026-11-13, ADR 1173), nominated by A Person: the dev-rung candidate."
  grep -q "      e2e_dev:" "$WT/ops/pins/fleet-pins.yaml"
  grep -q "        value: \"$NEW\"" "$WT/ops/pins/fleet-pins.yaml"
  grep -q "(2026-11-13, ADR 1173)" "$WT/ops/pins/fleet-pins.yaml"
}

@test "re-declaring the same value on a later day leaves the table byte-identical" {
  dev "$NEW" "RC portal/v1.1.0-rc.1 (2026-11-13, ADR 1173), nominated by A Person: the dev-rung candidate."
  cp "$WT/ops/pins/fleet-pins.yaml" "$TMP/before.yaml"
  dev "$NEW" "RC portal/v1.1.0-rc.1 (2026-11-14, ADR 1173), nominated by A Person: the dev-rung candidate."
  cmp "$TMP/before.yaml" "$WT/ops/pins/fleet-pins.yaml"
}

@test "a different value replaces the deviation and its reason where it stands" {
  dev "$OLD" "an earlier candidate (2026-11-01)"
  dev "$NEW" "RC portal/v1.1.0-rc.1 (2026-11-14, ADR 1173), nominated by A Person: the dev-rung candidate."
  grep -q "        value: \"$NEW\"" "$WT/ops/pins/fleet-pins.yaml"
  grep -q "(2026-11-14, ADR 1173)" "$WT/ops/pins/fleet-pins.yaml"
  if grep -q "an earlier candidate" "$WT/ops/pins/fleet-pins.yaml"; then false; fi
  [ "$(grep -c '      e2e_dev:' "$WT/ops/pins/fleet-pins.yaml")" = 1 ]
}
