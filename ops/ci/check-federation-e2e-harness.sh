#!/usr/bin/env bash
# check-federation-e2e-harness.sh — the federation e2e environment's harness, without Docker:
# the scenario catalog and the outcome rules of its verdict (ops/federation-e2e/tests/test_harness.py),
# and bash syntax of every unit. docs/design/federation-e2e-environment.md §Where each scenario runs.
set -euo pipefail
cd "$(dirname "$0")/../.."
for f in ops/federation-e2e/env ops/federation-e2e/lib/*.sh ops/federation-e2e/units/*.sh; do bash -n "$f"; done
python3 ops/federation-e2e/tests/test_harness.py
