#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# check-zenoh-pinning.sh — kept as the documented entry point; the rules now
# live in ops/upstream/deps/zenoh.yaml and are enforced by ops/upstream/upstream.py
# (ADR 1262). CI runs the check in .github/workflows/upstream-pins.yml.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
exec python3 ops/upstream/upstream.py check zenoh
