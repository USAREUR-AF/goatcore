#!/usr/bin/env bash
# ops/ci/check-goatbuilder-compat.sh — goatbuilder keeps its promise to the last release.
#
# The four surfaces other people build on (docs/design/goatbuilder-v1-gate.md,
# "What 1.0 promises", rows A2 to A5): A3 to A5 are checked here against the
# baseline kept in ops/goatbuilder/compat/, which names the release it was
# recorded from. A2, the configuration files of every release, is the cargo
# test crates/goatbuilder/tests/config_compat.rs.
#
#   A3  no command or flag of the last release is gone or renamed without an
#       entry in ops/goatbuilder/compat/deprecations.yaml
#   A4  every kept -o json / -o yaml document is still produced as a superset
#   A5  every GB code of the last release keeps its title in
#       ops/errors/catalog.yaml, unless the entry is marked `retired:`
#
# First the check's own tests (each rule refuses a violation of it), then the
# check against the build named on the command line.
#
# usage: ops/ci/check-goatbuilder-compat.sh <path to a built goatbuilder>
# Needs python3 with PyYAML. Exit 0 every promise holds; 1 one is broken; 2 usage.
#
# Rollback: delete this file and its step in .github/workflows/goatbuilder-pr.yml.
set -euo pipefail

if [ "$#" -ne 1 ] || [ ! -x "$1" ]; then
  echo "usage: $0 <path to a built goatbuilder>" >&2
  exit 2
fi
here="$(cd "$(dirname "$0")/../.." && pwd)"
bin="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"

python3 "$here/ops/goatbuilder/compat/test_compat.py"
python3 "$here/ops/goatbuilder/compat/compat.py" check --repo "$here" --bin "$bin"
