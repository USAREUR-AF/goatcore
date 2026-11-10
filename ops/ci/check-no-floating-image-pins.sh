#!/usr/bin/env bash
# check-no-floating-image-pins.sh — refuse a mutable tag where a digest belongs.
#
# WHY THIS EXISTS. A mutable tag moves underneath a running host: nothing records
# the drift, any `docker compose pull` silently upgrades the mesh control plane
# to code nobody reviewed, and two hosts pulling a week apart diverge.
#
# A pin table with `fleet: null` plus a `:latest` deviation and a note saying
# "tighten at next bump" has the right mechanism and the wrong value. A note is
# not a gate. This is the gate.
#
# WHAT IS ALLOWED, so this does not become a nuisance that gets bypassed:
#   - @sha256:<64 hex>                     a pinned digest, always fine
#   - a jinja expression                   the value comes from elsewhere; that
#                                          elsewhere is what gets checked
#   - anything listed in the waiver block  with a reason, in this file
set -euo pipefail
cd "$(dirname "$0")/../.."

# Doctrine P6: a guard proves itself RED against a broken fixture before its
# verdict on the real tree is trusted. A guard that cannot fail is ind...
# indistinguishable from one that passes, which is the defect class this whole
# file exists to close.
if [ "${1:-}" = "--self-test" ]; then
  tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
  mkdir -p "$tmp/ops/ansible/group_vars/fixture_hosts" "$tmp/ops/ci"
  cp "$0" "$(dirname "$0")/check-image-refs-pinned.py" "$tmp/ops/ci/"
  printf 'netbird_mgmt_image: "ghcr.io/{{ ghcr_org }}/netbird-mgmt:latest"\n' \
    > "$tmp/ops/ansible/group_vars/fixture_hosts/vars.yml"
  if ( cd "$tmp" && bash ops/ci/"$(basename "$0")" >/dev/null 2>&1 ); then
    echo "SELF-TEST FAILED: the guard passed a fixture carrying a floating tag."
    echo "  It would not have caught the netbird-mgmt:latest it was written for."
    exit 1
  fi
  printf 'netbird_mgmt_image: "ghcr.io/{{ ghcr_org }}/netbird-mgmt@sha256:%s"\n' \
    "$(printf 'a%.0s' $(seq 64))" > "$tmp/ops/ansible/group_vars/fixture_hosts/vars.yml"
  if ! ( cd "$tmp" && bash ops/ci/"$(basename "$0")" >/dev/null 2>&1 ); then
    echo "SELF-TEST FAILED: the guard rejected a correctly digest-pinned fixture."
    exit 1
  fi
  # The same red/green proof for each scan the helper adds. Every fixture is
  # clean except the one under test, so a pass can only mean that scan missed.
  d64="$(printf 'b%.0s' $(seq 64))"
  mkdir -p "$tmp/ops/ansible/roles/fx/defaults" "$tmp/ops/ansible/roles/fx/templates" "$tmp/svc"
  clean_tree() {
    printf 'fx_image: "ghcr.io/{{ ghcr_org }}/fx:sha-1@sha256:%s"\nfx_image_platform: linux/arm64\n' "$d64" \
      > "$tmp/ops/ansible/roles/fx/defaults/main.yml"
    printf 'services:\n  fx:\n    image: "{{ fx_image }}"\n' > "$tmp/ops/ansible/roles/fx/templates/docker-compose.yml.j2"
    printf 'FROM golang:1.26@sha256:%s AS build\nFROM build\nFROM scratch\n' "$d64" > "$tmp/svc/Dockerfile"
    mkdir -p "$tmp/.github"
    printf 'version: 2\nupdates:\n  - package-ecosystem: docker\n    directories:\n      - /svc\n' > "$tmp/.github/dependabot.yml"
  }
  red() {  # $1 = what the fixture should have caught
    if ( cd "$tmp" && bash ops/ci/"$(basename "$0")" >/dev/null 2>&1 ); then
      echo "SELF-TEST FAILED: the guard passed $1."; exit 1
    fi
  }
  clean_tree
  if ! ( cd "$tmp" && bash ops/ci/"$(basename "$0")" >/dev/null 2>&1 ); then
    echo "SELF-TEST FAILED: the guard rejected a fully pinned tree."; exit 1
  fi
  printf 'admit_audit_image: ghcr.io/{{ ghcr_org }}/admit-audit:latest\n' > "$tmp/ops/ansible/roles/fx/defaults/main.yml"
  red "a role default on :latest (the admit-audit shape)"; clean_tree
  printf 'services:\n  db:\n    image: postgres:16-alpine\n' > "$tmp/ops/ansible/roles/fx/templates/docker-compose.yml.j2"
  red "a literal floating image in a compose template"; clean_tree
  printf 'FROM cgr.dev/chainguard/glibc-dynamic:latest\n' > "$tmp/svc/Dockerfile"
  red "a Dockerfile FROM on a floating tag"; clean_tree
  printf 'version: 2\nupdates: []\n' > "$tmp/.github/dependabot.yml"
  red "a digest-pinned FROM that dependabot never refreshes"; clean_tree
  printf 'ops/ansible/roles/gone/defaults/main.yml gone_image   # fixture\n' > "$tmp/ops/ci/floating-image-pins.allow"
  red "an allow line whose reference no longer exists (the list must shrink)"
  echo "self-test ok: red on a floating tag in group_vars, role defaults, compose and FROM, on a pin with no refresh, and on a stale allow line; green on digests"
  exit 0
fi

# Deliberate, named exceptions. A waiver is a decision with an owner, not a TODO.
# Format: <path-glob>:<var>  # reason
WAIVERS=(
  # Tiers that float ON PURPOSE, to catch upstream breakage before a pinned
  # goatnet inherits it. Floating is the point; a digest here would defeat it.
  "ops/ansible/group_vars/hobby_hosts/vars.yml:*"          # hobby tier tracks upstream head by design
  "ops/ansible/group_vars/integdev_hosts/vars.yml:*"       # integration rig; floats to surface breakage early
  "ops/ansible/group_vars/goat_client_assay_hosts/vars.yml:*"  # assay rig pins per-run, deliberately unset

  # The mgmt-host-bootstrap CLI. Its floating tag was the fallback written in
  # ops/compose/mgmt-host/docker-compose.yml; that file now falls back to the
  # public build and these two files carry the earlier reference for the host
  # groups outside the pin table, so no host runs a different image. A digest
  # of record is a value change with its own deploy; these lines go when it
  # lands.
  "ops/ansible/group_vars/mgmt_hosts.yml:mgmt_host_bootstrap_image"
  "ops/ansible/group_vars/goat_client_assay_hosts/oss-generic-fin-compose-mgmt.yml:mgmt_host_bootstrap_image"

)

fail=0
while IFS= read -r line; do
  f="${line%%:*}"; rest="${line#*:}"; ln="${rest%%:*}"; body="${rest#*:}"
  var="$(printf '%s' "$body" | sed -E 's/^[[:space:]]*([a-z0-9_]+):.*/\1/')"
  val="$(printf '%s' "$body" | sed -E 's/^[^:]*:[[:space:]]*//; s/^"//; s/"$//')"

  # a digest, or a value computed elsewhere, is fine
  [[ "$val" == *"@sha256:"* ]] && continue
  [[ "$val" == *"{{"*"}}"* && "$val" != *"ghcr.io/{{ ghcr_org }}/"*":"* ]] && continue

  waived=0
  for w in "${WAIVERS[@]}"; do
    wp="${w%%:*}"; wv="${w#*:}"; wv="${wv%%#*}"; wv="$(printf '%s' "$wv" | tr -d ' ')"
    # The waiver path is a glob pattern, not a literal filename.
    # shellcheck disable=SC2053
    [[ "$f" == $wp && ( "$wv" == "*" || "$wv" == "$var" ) ]] && { waived=1; break; }
  done
  [ "$waived" = 1 ] && continue

  echo "FLOATING PIN  $f:$ln"
  echo "    $var = $val"
  echo "    A tag is mutable: whoever can push can change what this runs, with no commit."
  echo "    Pin the multi-arch index digest (@sha256:…), or add a waiver with a reason."
  fail=$((fail+1))
done < <(grep -rnE '^[[:space:]]*[a-z0-9_]*image[a-z0-9_]*:[[:space:]]*"?[^"]*ghcr\.io' \
           ops/ansible/group_vars 2>/dev/null | grep -v '/pins.yml:' || true)

if [ "$fail" -gt 0 ]; then
  echo
  echo "$fail floating image pin(s). Generated pins.yml files are excluded — they render"
  echo "from ops/pins/fleet-pins.yaml, so fix the table there rather than the output."
  exit 1
fi
echo "ok: no floating image pins in hand-authored group_vars"

# Role defaults, compose templates and Dockerfile FROM lines (ADR 0703).
# A role default with no override is what deploys; the rest decide
# what a rebuild of the same commit produces. The floats that existed when this
# landed are on a shrink-only list the helper enforces.
python3 ops/ci/check-image-refs-pinned.py
