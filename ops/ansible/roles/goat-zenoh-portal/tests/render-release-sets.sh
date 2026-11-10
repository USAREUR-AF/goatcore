#!/usr/bin/env bash
# Renders templates/release-sets.yaml.j2 from tests/release-sets-vars.yml and
# compares it with the golden file the portal's Go test parses
# (ops/goat-zenoh-portal/internal/access/testdata/release-sets.role-render.yaml,
# TestLoadReleaseSets_RoleRender). Together the two prove a goatnet's
# portal_release_sets reaches the portal in a shape LoadReleaseSets accepts.
# Regenerate the golden file: render-release-sets.sh --update

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROLE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_DIR="$(cd "$ROLE_DIR/../../../.." && pwd)"
GOLDEN="$REPO_DIR/ops/goat-zenoh-portal/internal/access/testdata/release-sets.role-render.yaml"

out="$(mktemp -d)"
trap 'rm -rf "$out"' EXIT

ansible localhost -i localhost, --connection local -m ansible.builtin.template \
  -a "src=$ROLE_DIR/templates/release-sets.yaml.j2 dest=$out/release-sets.yaml mode=0644" \
  -e "@$SCRIPT_DIR/release-sets-vars.yml" >/dev/null

if [[ "${1:-}" == "--update" ]]; then
  cp "$out/release-sets.yaml" "$GOLDEN"
  echo "goat-zenoh-portal: release-sets golden file updated"
  exit 0
fi

if ! diff -u "$GOLDEN" "$out/release-sets.yaml"; then
  echo "goat-zenoh-portal: rendered release-sets.yaml differs from $GOLDEN" >&2
  echo "  (if the template change is intended: $0 --update, then go test ./internal/access/)" >&2
  exit 1
fi
echo "goat-zenoh-portal: release-sets render OK"
