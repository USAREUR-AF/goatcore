#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# check-no-agent-references-in-commits.sh BASE HEAD
#
# The commit-message half of the agent-references guard
# (.github/workflows/agent-references.yml). History cannot be scrubbed later,
# so a vendor reference in a commit message is worse than one in a file:
# ops/ci/check-no-agent-references.sh scans files, and this scans the
# messages of the commits in the range.
#
# Prints each matching line and exits 1 on a hit, 0 when there is none, and 2
# on a usage error. It reads only commit messages; it executes nothing from
# the range.
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: check-no-agent-references-in-commits.sh <base> <head>" >&2
  exit 2
fi
base=$1
head=$2

# The pattern is the workflow's own. The trailer is written in two halves so
# that this file, which check-no-agent-references.sh scans like any other,
# does not hold it.
pattern='claude\.ai|Claude''-Session:|anthropic\.com'

if git log --format='%B' "${base}...${head}" | grep -nEi "$pattern"; then
  exit 1
fi
echo "ok: no agent-tooling references in commit messages"
