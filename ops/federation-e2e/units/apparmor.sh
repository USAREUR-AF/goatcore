#!/usr/bin/env bash
# units/apparmor.sh — the pair under avenue goatbuilder with the machine's own AppArmor
# profile for wg in force (FE2E_WG_APPARMOR=host). Only meaningful on a machine that loads
# that profile, as Ubuntu 26.04 LTS does; elsewhere it measures the same as the pair unit.
# Sourced by lib/unit.sh.
unit_main() {
  export FE2E_WG_APPARMOR=host
  if ! grep -qs '^wg ' /sys/kernel/security/apparmor/profiles 2>/dev/null && ! [ -e /etc/apparmor.d/wg ]; then
    mark ubuntu-2604-apparmor.first-phases-11-to-12 "NOT APPLICABLE" "this machine loads no AppArmor profile for wg"
  fi
  set_partners "$A" "$B"; set_partners "$B" "$A"; set_partners "$C" ""; NODIR=""
  export_helpers
  setup_hosts 1
  peer_up 1 "$A" "$B" goatbuilder
}
