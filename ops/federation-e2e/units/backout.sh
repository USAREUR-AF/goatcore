#!/usr/bin/env bash
# units/backout.sh — B7 of docs/design/goatbuilder-v1-gate.md: backing a peering out removes
# everything the command created and nothing else.
#
# nothing-hand-written (units/nohand.sh) up to an accepted peering; before its first phase,
# a snapshot of {A}'s edge host and core host; then `goatbuilder federate backout {B}` on
# {A}; then the same snapshot again. Both hosts must be as they were before the peering. The
# working directory (/var/lib/goat-federation) is left out: the command keeps its record there.
# shellcheck source=/dev/null
. "$HERE/units/nohand.sh"

# What a host holds that a peering could add, in a stable order.
bo_snapshot() { # $1 host -> stdout
  host_exec "$1" sh -c '
    echo "## containers"; docker ps -a --format "{{.Names}} {{.Image}}" 2>/dev/null | sort
    echo "## wireguard"; ip -o link show 2>/dev/null | awk -F": " "{print \$2}" | grep -E "^wg" | sort
    echo "## nftables"; nft list tables 2>/dev/null | sort
    echo "## units"; systemctl list-units --all --plain --no-legend "goat*" "*peering*" "*spire*" 2>/dev/null | awk "{print \$1}" | sort
    echo "## files"; for d in /etc/goat-peering-wg /etc/goat-zenoh-edge-* /opt/goat-zenoh-edge-* /etc/goat-fed-republisher* /opt/goat-fed-republisher* /etc/goat-peering-spire*; do [ -e "$d" ] && find "$d" -type f; done 2>/dev/null | sort
    echo "## core"; for f in /etc/*-zenoh/acl-org-fragment.json5 /etc/*-zenoh/config.json5; do [ -f "$f" ] && sha256sum "$f"; done 2>/dev/null
    echo "## spire federation"; docker exec spire-server /opt/spire/bin/spire-server federation list 2>/dev/null | grep -i "trust domain" | sort
  '
}

# The snapshot is taken at the drive's start, before any phase has run.
eval "$(declare -f oc_drive | sed '1s/^oc_drive /bo_drive_inner /')"
oc_drive() {
  bo_snapshot "$HP-$A-edge" > "$RECORD/bo-$A-edge-before.txt"
  bo_snapshot "$HP-$A-core" > "$RECORD/bo-$A-core-before.txt"
  bo_drive_inner "$@"
}

unit_main() {
  nh_main
  # The back-out, by the command, without a prompt (--yes is its documented confirmation).
  local rc=0
  oc_run "$A" "$B" "bo" backout "$B" --yes || rc=$?
  # The command's own exit status decides; its transcript's tail is the evidence.
  rec "bo-$A-$B-backout-ran" -- bash -c \
    "sed 's/\x1b\[[0-9;]*m//g' '$RECORD/$OC_PREFIX-$A-bo.log' | tr -d '\r' | tail -40; exit $rc"
  bo_snapshot "$HP-$A-edge" > "$RECORD/bo-$A-edge-after.txt"
  bo_snapshot "$HP-$A-core" > "$RECORD/bo-$A-core-after.txt"
  rec "bo-$A-edge-as-before" -- diff "$RECORD/bo-$A-edge-before.txt" "$RECORD/bo-$A-edge-after.txt"
  rec "bo-$A-core-as-before" -- diff "$RECORD/bo-$A-core-before.txt" "$RECORD/bo-$A-core-after.txt"
}
