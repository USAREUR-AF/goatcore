#!/usr/bin/env bash
# units/three.sh — three parties: {A} peers with {B} and with {C} on one edge host and one
# core host (scenario 3 of run.sh). Under avenue goatbuilder the first peering goes through
# goatbuilder federate and the second through peering-standup.sh; under avenue script both
# go through peering-standup.sh. Sourced by lib/unit.sh.
unit_main() {
  local av1 av2
  if [ "$AVENUE" = goatbuilder ]; then av1=goatbuilder; av2=script; else av1=script; av2=script; fi
  scenario3 "$av1" "$av2"
}
