#!/usr/bin/env bash
# units/shapes-oneway.sh — a one-way agreement: {A} releases release/all and release/{B}
# and accepts nothing; {B} releases nothing and accepts. The agreement carries {A}->{B}
# only. Scenarios: agreement-send-only ({A}'s view), agreement-receive-only ({B}'s view).
# Sourced by lib/unit.sh.
# shellcheck source=shapes.sh
. "$HERE/units/shapes.sh"
unit_main() {
  eval "SH_REL_$A='release/all release/$B'; SH_ACC_$A=no; SH_REL_$B=''; SH_ACC_$B=yes"
  shapes_main h1 agreement-send-only agreement-receive-only
}
