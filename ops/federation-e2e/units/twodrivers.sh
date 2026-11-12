#!/usr/bin/env bash
# units/twodrivers.sh — gate row B10 (docs/design/goatbuilder-v1-gate.md), against the contract
# in docs/design/federation-two-drivers.md: two drivers per sovereign.
#
#   the controller   ($HP-<s>-ops)   holds the CA key, the plan, every approval and the journal;
#                                    reaches the core host, and has no route to the edge host
#   the edge driver  ($HP-<s>-edrv)  holds no CA key and no lab key but its ssh key; reaches the
#                                    edge host only, on a network it shares with nothing else
#
# Both are enforced by the network: the edge driver sits on its own network with the edge host,
# and the controller's route to the edge host is a blackhole. Each side then follows the
# hand-offs the command gives, from phase 1 to phase 14.
#
# Steps, today (nothing of the two-driver run is built): the topology checks, then the
# controller's phase 1 must stop with a hand-off naming the edge driver's `--side edge` run, and
# the edge driver's goatbuilder must offer `--side`. While either fails the unit stops there,
# and the contract's end-state checks (catalog: two-drivers-per-sovereign) read BLOCKED.
# shellcheck source=/dev/null
. "$HERE/units/workspace.sh"

for _p in $PARTIES; do HOSTS+=("$HP-$_p-edrv"); NETS+=("$NP-$_p-edrv"); done
edrv_ip() { echo "$NB.$(($(octet "$1") + 60)).30"; }
edrv_edge_ip() { echo "$NB.$(($(octet "$1") + 60)).20"; }
export -f edrv_ip edrv_edge_ip

# The edge driver: the host image, its own network with the edge host, and only an ssh key.
td_edge_driver() { # $1 sovereign
  local s="$1" o k="$WORK/$1/edrv-keys"; o="$(octet "$s")"
  mkdir -p "$k"; cp "$WORK/$s/keys/id_ed25519" "$WORK/$s/keys/id_ed25519.pub" "$k/"; chmod 0600 "$k/id_ed25519"
  docker network create "${LABEL[@]}" --internal --subnet "$NB.$((o + 60)).0/24" "$NP-$s-edrv" >/dev/null
  docker network connect --ip "$(edrv_edge_ip "$s")" "$NP-$s-edrv" "$HP-$s-edge"
  docker run -d --name "$HP-$s-edrv" --hostname "$HP-$s-edrv" "${LABEL[@]}" "${NOPROXY[@]}" --privileged --cgroupns=private \
    --network "$NP-$s-edrv" --ip "$(edrv_ip "$s")" \
    --tmpfs /run --tmpfs /run/lock -v /var/lib/docker \
    -v "$k:/etc/fed-e2e:ro" -v "$WORK/$s/origin.git:/srv/goat.git:ro" \
    -v "$(goat_of "$s"):/usr/local/bin/goat:ro" -v "$GB:/usr/local/bin/goatbuilder:ro" \
    -e container=docker "$HOST_IMAGE" >/dev/null
  wait_ready "$HP-$s-edrv"
  # The controller has no route to the edge host.
  docker exec "$HP-$s-ops" ip route add blackhole "$(edge_ip "$s")/32"
}

td_topology_checks() { # $1 sovereign
  local s="$1" ssh="ssh -o BatchMode=yes -o ConnectTimeout=5 -i /etc/fed-e2e/id_ed25519"
  rec "td-$s-edge-driver-reaches-the-edge" --expect-in "$HP-$s-edge" -- docker exec "$HP-$s-edrv" $ssh "goat@$(edrv_edge_ip "$s")" hostname
  rec "td-$s-edge-driver-cannot-reach-the-core" --expect-exit 1 -- docker exec "$HP-$s-edrv" bash -c "exec 3<>/dev/tcp/$(core_ip "$s")/22"
  rec "td-$s-controller-cannot-reach-the-edge" --expect-exit 1 -- docker exec "$HP-$s-ops" bash -c "timeout 5 bash -c 'exec 3<>/dev/tcp/$(edge_ip "$s")/22'"
  rec "td-$s-controller-reaches-the-core" --expect-in "$HP-$s-core" -- docker exec "$HP-$s-ops" ssh -o BatchMode=yes -o ConnectTimeout=5 "goat@$(core_ip "$s")" hostname
  rec "td-$s-edge-driver-holds-no-ca-key" -- docker exec "$HP-$s-edrv" sh -c \
    "found=\$(grep -rlE 'BEGIN (EC |RSA )?PRIVATE KEY' /etc /root /opt 2>/dev/null | grep -v '/etc/fed-e2e/id_ed25519\$' | grep -v '^/etc/ssh/' || true); [ -z \"\$found\" ] && echo 'no private key but its ssh key' || { echo \"\$found\"; exit 1; }"
}

unit_main() {
  local n=td s them
  [ "$AVENUE" = goatbuilder ] || die "the two-driver unit drives goatbuilder only, not $AVENUE"
  set_partners "$A" "$B"; set_partners "$B" "$A"; set_partners "$C" ""
  # shellcheck disable=SC2034 # NODIR is read by lib/engine.sh (nodir)
  NODIR="$A:$B $B:$A"
  export_helpers
  setup_hosts "$n"
  for s in "$A" "$B"; do td_edge_driver "$s"; done
  # The edge driver's key reaches the edge host (the goat account the inventory uses).
  for s in "$A" "$B"; do
    docker exec "$HP-$s-edge" sh -c "cat >> /home/goat/.ssh/authorized_keys" < "$WORK/$s/keys/id_ed25519.pub"
  done
  say "two drivers per sovereign: the topology the contract asks for, enforced by the network"
  for s in "$A" "$B"; do td_topology_checks "$s"; done

  say "two drivers per sovereign: the controller hands off what only the edge reaches"
  local f0="$FAILS"
  for s in "$A" "$B"; do
    them="$(other "$s")"
    rec "td-$s-controller-phase-1-hands-off-to-the-edge-driver" --expect-in="--side edge" -- bash -c "drive '$s' '$them' goatbuilder 1 1"
    rec "td-$s-edge-driver-offers-side-edge" --expect-in="--side" -- docker exec "$HP-$s-edrv" goatbuilder federate --help
  done
  [ "$FAILS" = "$f0" ] || die "the two-driver run is not built: the controller's phase 1 gives no hand-off to an edge driver (docs/design/federation-two-drivers.md, What has to be built)"
  die "the hand-off sequence after phase 1 is written once the hand-off object is in the contract"
}
