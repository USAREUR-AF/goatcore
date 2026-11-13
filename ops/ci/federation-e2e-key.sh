#!/usr/bin/env bash
# federation-e2e-key.sh: the key of one stage of the two-sovereign job (rule 3 of
# docs/design/ci-results-follow-inputs.md). Identical inputs give one key; any change to a
# declared input gives another.
#
#   ops/ci/federation-e2e-key.sh <tree> <stage>     prints the key: 64 hex characters
#
# <tree> is a checkout of this repository; <stage> names the job (build, scenario-<n>). The
# key covers, from the working tree as it is (committed or not):
#   - every file under the roots below, which are whole directories so that a file the job
#     reads without naming it is still covered (guard 1, "declare broadly"): the Rust
#     workspace the binaries are built from, the harness, the federation tree, every Ansible
#     role and playbook a peering can call, the scripts and the error catalog;
#   - every path the workflow's pull_request `paths:` list names, so that the key never
#     covers less than what starts the job;
#   - the workflow file and this script;
#   - the toolchain the workflow pins and the host image's base, named by digest.
# Nothing here reads a clock, the network or anything outside <tree>.
set -euo pipefail

tree="${1:?usage: federation-e2e-key.sh <tree> <stage>}"
stage="${2:?usage: federation-e2e-key.sh <tree> <stage>}"
wf=".github/workflows/federation-e2e.yml"
[ -f "$tree/$wf" ] || { echo "federation-e2e-key: no $wf under $tree" >&2; exit 2; }

if command -v sha256sum >/dev/null 2>&1; then sha=(sha256sum)
elif command -v shasum >/dev/null 2>&1; then sha=(shasum -a 256)
else echo "federation-e2e-key: neither sha256sum nor shasum is installed" >&2; exit 2; fi

roots=(
  "$wf"
  ops/ci/federation-e2e-key.sh
  ops/ci/check-federation-e2e-harness.sh
  ops/ci/check-federation-e2e-prepare.sh
  crates
  ops/federation-e2e
  ops/federation
  ops/e2e
  ops/ansible
  ops/scripts
  ops/goatbuilder
  ops/errors
)
# The workflow's own trigger paths: the entries of the `paths:` list under pull_request.
while IFS= read -r p; do roots+=("$p"); done < <(
  awk '/^  pull_request:/ { pr = 1; next }
       pr && /^    paths:/ { inp = 1; next }
       inp && /^      - / { sub(/^      - /, ""); gsub(/\x27|"/, ""); print; next }
       inp { exit }' "$tree/$wf")

toolchain="$(sed -n "s/^ *toolchain: *'\{0,1\}\([0-9][0-9.]*\)'\{0,1\} *$/\1/p" "$tree/$wf" | sort -u | tr '\n' ' ')"
[ -n "$toolchain" ] || { echo "federation-e2e-key: no pinned toolchain in $wf" >&2; exit 2; }
hostbase="$(sed -n 's/^FROM //p' "$tree/ops/federation-e2e/host/Dockerfile" | tr '\n' ' ')"
[ -n "$hostbase" ] || { echo "federation-e2e-key: no FROM in the host image's Dockerfile" >&2; exit 2; }

{
  printf 'stage %s\ntoolchain %s\nhost-base %s\n' "$stage" "$toolchain" "$hostbase"
  # Tracked and untracked (not ignored) files under the roots, as they are in the tree. A
  # tracked file deleted from the tree is left out, so deleting it changes the key too.
  git -C "$tree" ls-files -z --cached --others --exclude-standard -- "${roots[@]}" \
    | LC_ALL=C sort -z -u \
    | while IFS= read -r -d '' f; do if [ -f "$tree/$f" ]; then printf '%s\0' "$f"; fi; done \
    | (cd "$tree" && xargs -0 "${sha[@]}" --)
} | "${sha[@]}" | awk '{ print $1 }'
