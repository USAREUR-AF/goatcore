#!/usr/bin/env bats

@test "offboard driver fails closed and persists receipts" {
  run python3 ops/scripts/test_offboard_driver.py
  [ "$status" -eq 0 ]
}

@test "offboarding plug-ins hold the contract against synthetic peers" {
  run python3 ops/offboard/test_plugins.py
  [ "$status" -eq 0 ]
}
