#!/usr/bin/env bash
# units/shapes-asym.sh — an asymmetric agreement: both sides accept, {A} releases only
# release/all and {B} releases only release/{A}. Each set must cross in its own direction
# only. Scenario: agreement-asymmetric. Sourced by lib/unit.sh.
# shellcheck source=shapes.sh
. "$HERE/units/shapes.sh"
unit_main() {
  eval "SH_REL_$A='release/all'; SH_ACC_$A=yes; SH_REL_$B='release/$A'; SH_ACC_$B=yes"
  shapes_main h2 agreement-asymmetric agreement-asymmetric
}
