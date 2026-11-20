#!/usr/bin/env bash
# check-goatbuilder-bare-question.sh — no `?` in goatbuilder passes up an error
# without its catalog code (gate row C5). The work is in
# ops/ci/goatbuilder_bare_question.py, which says how: a copy of the tree, an
# error type that converts only from coded errors, and `cargo check`.
#
#   ops/ci/check-goatbuilder-bare-question.sh              the gate
#   ops/ci/check-goatbuilder-bare-question.sh --write      list today's places
#   ops/ci/check-goatbuilder-bare-question.sh --self-test  its own tests
#
# CARGO names the cargo command (default: cargo), e.g. CARGO="cargo +1.100.0".
# Exit 0 ok, 1 a new place or a listed place gone, 2 cannot run.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
exec python3 "$here/goatbuilder_bare_question.py" "$@"
