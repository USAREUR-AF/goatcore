#!/usr/bin/env bash
# lib/joinsite-tree.sh — the files of the tree under test that join-node's and standup's dry
# runs call or read, copied into a checkout laid out from a fixture. Sourced by
# units/join-site.sh and units/join-site-hosts.sh.

# js_real_files <dest>: the tree's end-state model, the gates the dry runs call and the plays
# whose presence they read. Fails naming the first file the tree under test does not have.
js_real_files() {
  local d="$1" s
  mkdir -p "$d/ops/scripts" "$d/ops/secrets" "$d/ops/federation/standup" "$d/ops/goatbuilder"
  cp -a "$SRC/ops/goatbuilder/endstate" "$d/ops/goatbuilder/"
  for s in ops/scripts/standup-config-preflight.py ops/scripts/join-node-clock-check.py \
           ops/federation/standup/preflight_live.py ops/scripts/signal-federation-check.py \
           ops/scripts/standup-mesh-record.py ops/scripts/inventory-hosts.py \
           ops/scripts/with-secrets ops/scripts/seed-ssh-key.sh ops/secrets/posture.py \
           ops/scripts/require-secret.sh ops/ansible/fabric-health-deploy.yml \
           ops/ansible/mesh-health-deploy.yml ops/ansible/presence-health-deploy.yml \
           ops/ansible/time-health-deploy.yml ops/ansible/consistency-health-deploy.yml \
           ops/ansible/fabric-crawler-deploy.yml; do
    [ -e "$SRC/$s" ] || { echo "join-site: the tree under test has no $s" >&2; return 1; }
    cp "$SRC/$s" "$d/$s"
  done
}
