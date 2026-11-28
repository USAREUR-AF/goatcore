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
# The role arms the boot-time clock step only with the HTTP time source, as at a
# site that blocks NTP; the clock-recovery row asks for it on every break-glass node.
HOST_VARS="platform_family: small-computer
breakglass_oob_interface: $OOB
breakglass_http_time_enabled: true"
# The values generate-values reads from a site's checkout, given here for one
# machine: the clock step is armed by the host vars above (the HTTP time source
# on, the step left at its default), so its row applies.
ROW_VARS=(breakglass_host="$NODE" breakglass_oob_interface="$OOB" breakglass_has_oob=true has_breakglass_node=true breakglass_clock_step_armed=true)

ROUTE_ROWS=breakglass-clock-recovery-armed,breakglass-oob-is-never-the-default-route
BIND_ROW=breakglass-oob-binds-no-service

# oob_up <metric>: the out-of-band link as a bearer would give it, an address
# and a default route of its own at that metric, above the primary's. The role
# raises a DHCP or RA route's metric; a static one it leaves, and checks.
oob_up() {
  vm_ssh "sudo sh -eu" <<VM
printf 'network:\n  version: 2\n  ethernets:\n    $OOB:\n      addresses: [$OOB_ADDR/24]\n      routes: [{to: default, via: 198.51.100.1, metric: $1}]\n' > /etc/netplan/60-oob.yaml
chmod 600 /etc/netplan/60-oob.yaml
netplan apply
sleep 3
ip -4 route show default
VM
}

# rows_with <rows> <how to break it> <how to put it back>: the rows, with one rule broken by hand.
rows_with() {
  local rows="$1" rc=0
  vm_ssh "sudo sh -c '$2'"
  acceptance "$rows" "${ROW_VARS[@]}" || rc=$?
  vm_ssh "sudo sh -c '$3'" || true
  return "$rc"
}

type_cases() {
  step oob-up 0 "dev $OOB proto static metric 2000" -- oob_up 2000
  step phase-13 0 "Phase 13" "!fatal:" -- "${JOIN[@]}" --from 13 --to 13
  step acceptance 0 "PASS  breakglass-clock-recovery-armed" "PASS  breakglass-oob-is-never-the-default-route" -- acceptance "$ROUTE_ROWS" "${ROW_VARS[@]}"
  step oob-egress-caught 1 "FAIL  breakglass-oob-is-never-the-default-route" "PASS  breakglass-clock-recovery-armed" -- \
    rows_with "$ROUTE_ROWS" "ip route add default via 198.51.100.1 dev $OOB metric 10" "ip route del default via 198.51.100.1 dev $OOB metric 10"
  step oob-binds-nothing 0 "PASS  $BIND_ROW" -- acceptance "$BIND_ROW" "${ROW_VARS[@]}"
  step oob-listener-caught 1 "FAIL  $BIND_ROW" -- \
    rows_with "$BIND_ROW" "nohup python3 -m http.server --bind $OOB_ADDR 8080 >/dev/null 2>&1 & sleep 2" "pkill -f \"http.server --bind $OOB_ADDR\""
  # The cloud image is not a Pi the builder made: those rows fail, and say why.
  step doctor-live 1 "small-computer $NODE board" "small-computer $NODE root" "is not on an encrypted device" "!UNKNOWN — breakglass requires" -- "${DOCTOR[@]}" --live
  not_run "breakglass-node-is-on-the-mesh: it reads the mesh, which needs phase 10"
  not_run "the role's own refusal of an out-of-band default route below the primary's: such a route takes the egress, so the play stops first at its clock step, which can reach no time source (measured); the route row's egress case above is the check on the machine"
  not_run "ADR 1114 section 5's TCP-level health check of the out-of-band path: the site monitor's, not the machine's"
}
