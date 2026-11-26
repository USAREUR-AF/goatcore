# shellcheck shell=bash disable=SC2034 # read by ops/e2e/vm/run.sh
# federation-boundary (gate J8), on the host-class family: the edge router machine
# of each peering. join-node builds the machine; a peering is made by
# `goatbuilder federate` from an agreement both sides approve, so phase 13 is a
# declared skip by design, and the machine then has no tunnel and peers with nobody.
# Its small-computer variant waits for the small-computer builder.
HOST_VARS="platform_family: host-class"
type_cases() {
  step phase-13 0 "Phase 13 · SKIP" "this machine peers with nobody until it has run" "!peering-" -- "${JOIN[@]}" --from 13 --to 13
  not_run "a peering over this machine: goatbuilder federate makes one from an agreement, and this run has no partner goatnet"
  # What the skip says, on the machine: no WireGuard interface, so no tunnel to any partner.
  step no-tunnel 0 "no wireguard interface" -- vm_ssh 'ip -o link show type wireguard | grep -q . && { ip -o link show type wireguard; exit 1; }; echo "no wireguard interface"'
}
