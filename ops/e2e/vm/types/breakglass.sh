# shellcheck shell=bash disable=SC2034 # read by ops/e2e/vm/run.sh
# breakglass (ADR 1114), on the small-computer family: a second wired interface
# is its out-of-band one, and ADR 1114 section 5's dual-homing rows are asked of
# the machine as it is, both ways. The cloud image is not a built Pi, so
# doctor --target --live's small-computer rows say so and fail.
VM_TAPS=1
OOB=tap0
OOB_ADDR=198.51.100.7
# The type requires an encrypted root, which the checkout cannot show: UNKNOWN.
DOCTOR_CHECKOUT_EXIT=1
DOCTOR_CHECKOUT_SAYS=("UNKNOWN — breakglass requires an encrypted root")
HOST_VARS="platform_family: small-computer
breakglass_oob_interface: $OOB"
ROWS=breakglass-clock-recovery-armed,breakglass-oob-is-never-the-default-route,breakglass-oob-binds-no-service
ROW_VARS=(breakglass_host="$NODE" breakglass_oob_interface="$OOB" breakglass_has_oob=true has_breakglass_node=true)

# The out-of-band link as a bearer would give it: an address and a default
# route of its own, with a lower metric than the primary's, which the play must
# raise (ADR 1114 section 5: never the egress path).
oob_up() {
  vm_ssh "sudo sh -eu" <<VM
printf 'network:\n  version: 2\n  ethernets:\n    $OOB:\n      addresses: [$OOB_ADDR/24]\n      routes: [{to: default, via: 198.51.100.1, metric: 50}]\n' > /etc/netplan/60-oob.yaml
chmod 600 /etc/netplan/60-oob.yaml
netplan apply
sleep 3
ip -4 route show default
VM
}

# rows_with <how to break it> <how to put it back>: the rows, with one rule broken by hand.
rows_with() {
  vm_ssh "sudo sh -c '$1'"
  local rc=0
  acceptance "$ROWS" "${ROW_VARS[@]}" || rc=$?
  vm_ssh "sudo sh -c '$2'" || true
  return "$rc"
}

type_cases() {
  step oob-up 0 "dev $OOB" -- oob_up
  step phase-13 0 "Phase 13" "!fatal:" -- "${JOIN[@]}" --from 13 --to 13
  step oob-metric 0 "dev $OOB" "metric 1000" "!metric 50" -- vm_ssh ip -4 route show default
  step acceptance 0 "PASS  breakglass-clock-recovery-armed" "PASS  breakglass-oob-is-never-the-default-route" "PASS  breakglass-oob-binds-no-service" -- acceptance "$ROWS" "${ROW_VARS[@]}"
  step oob-egress-caught 1 "FAIL  breakglass-oob-is-never-the-default-route" "PASS  breakglass-oob-binds-no-service" -- \
    rows_with "ip route add default via 198.51.100.1 dev $OOB metric 10" "ip route del default via 198.51.100.1 dev $OOB metric 10"
  step oob-listener-caught 1 "FAIL  breakglass-oob-binds-no-service" "PASS  breakglass-oob-is-never-the-default-route" -- \
    rows_with "nohup python3 -m http.server --bind $OOB_ADDR 8080 >/dev/null 2>&1 & sleep 2" "pkill -f \"http.server --bind $OOB_ADDR\""
  # The cloud image is not a Pi the builder made: those rows fail, and say why.
  step doctor-live 1 "small-computer $NODE board" "small-computer $NODE root" "is not on an encrypted device" "!UNKNOWN — breakglass requires" -- "${DOCTOR[@]}" --live
  not_run "breakglass-node-is-on-the-mesh: it reads the mesh, which needs phase 10"
  not_run "ADR 1114 section 5's TCP-level health check of the out-of-band path: the site monitor's, not the machine's"
}
