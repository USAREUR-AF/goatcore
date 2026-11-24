#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# Synthetic CLI responses for the default browser harness.
set -eu

for argument in "$@"; do
  case "$argument" in
    --to=ZAGROS-OBSERVER)
      printf '%s\n' '{"from_group":"ZAGROS-DEVELOPER","to_group":"ZAGROS-OBSERVER","movers":[{"peer_id":"e1f2a3b4c5d6e7f8091a2b3c","name":"peer-move-target-acme-zagros-20260604-laptop","other_groups":[]}],"already_at_destination":0}'
      exit 0
      ;;
    --to=ZAGROS-INFRA)
      printf '%s\n' 'error: COVERAGE GATE: fixture destination has no enabled policy' >&2
      exit 1
      ;;
  esac
done

printf '%s\n' 'unexpected move planner fixture arguments' >&2
exit 2
