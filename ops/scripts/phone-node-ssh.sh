#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# phone-node-ssh.sh — reach the Linux VM on an Android phone over the Android
# debug bridge (adb), on a USB cable. docs/design/phone-node.md, "How it is
# reached"; `goatbuilder join-node --via usb` runs it.
#
# The VM sits behind the phone's own address translation: no other machine can
# reach its address, and the address changes between boots. SSH is therefore
# carried by `adb shell nc <VM address> 22`, and the address is looked up on
# every connection.
#
# usage:
#   phone-node-ssh.sh --proxy
#       The SSH proxy command: find the VM's address, connect to its port 22.
#   phone-node-ssh.sh --vm-address
#       Print the VM's address.
#   phone-node-ssh.sh --write <base> --node <node>
#       Write <base>.ssh, an SSH client configuration for <node> that uses the
#       proxy command above, and <base>.ini, an Ansible inventory line that
#       makes Ansible use that configuration for <node>. Host keys go to
#       <base>.known_hosts. `ssh -F <base>.ssh` does not read ~/.ssh/config,
#       so no SSH configuration is written by hand.
#   phone-node-ssh.sh --base <base> <user>@<node> [command...]
#       ssh -F <base>.ssh, for a person.
#
# With more than one device attached, set ANDROID_SERIAL; adb reads it.
#
# Every adb call that does not carry the connection is `adb shell -n`: without
# -n, adb reads the caller's standard input and takes what was meant for ssh.
#
# A phone or VM that does not answer is refused with GB-299.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

refuse() {
  echo "GB-299: no Linux VM answers over the Android debug bridge: $*" >&2
  exit 1
}

vm_address() {
  command -v adb >/dev/null 2>&1 || refuse "adb is not installed on this machine (Android platform tools)"
  [ "$(adb get-state 2>/dev/null | tr -d '\r')" = device ] \
    || refuse "no phone on adb (attach it by USB, allow USB debugging on it, unlock it; \`adb devices\` lists it as device)"
  local ip
  ip=$(adb shell -n ip neigh show dev avf_tap_fixed 2>/dev/null | tr -d '\r' \
    | awk '$1 ~ /^10\./ && /lladdr/ {print $1}' | head -1)
  [ -n "$ip" ] || refuse "the phone's Linux VM has no address (open the Terminal app on the phone; the VM runs while it is open)"
  printf '%s\n' "$ip"
}

case "${1:-}" in
  --proxy)
    ip=$(vm_address)
    exec adb shell nc "$ip" 22
    ;;
  --vm-address)
    vm_address
    ;;
  --write)
    base="${2:-}"
    [ "${3:-}" = --node ] && [ -n "${4:-}" ] && [ -n "$base" ] \
      || { echo "usage: $0 --write <base> --node <node>" >&2; exit 64; }
    node="$4"
    mkdir -p "$(dirname "$base")"
    cat > "$base.ssh" <<EOF
# Written by ops/scripts/phone-node-ssh.sh; do not edit. SSH to $node, the
# Linux VM on a phone, carried over the Android debug bridge.
Host $node
  HostName $node
  HostKeyAlias $node
  ProxyCommand "$HERE/phone-node-ssh.sh" --proxy
  UserKnownHostsFile "$base.known_hosts"
  StrictHostKeyChecking accept-new
  ConnectTimeout 15
EOF
    cat > "$base.ini" <<EOF
# Written by ops/scripts/phone-node-ssh.sh; do not edit. Loaded after the
# goatnet's inventories until $node has a mesh address: Ansible reaches it
# over the Android debug bridge.
$node ansible_host=$node ansible_ssh_common_args='-F "$base.ssh"'
EOF
    ;;
  --base)
    base="${2:-}"
    [ -n "$base" ] && [ $# -ge 3 ] || { echo "usage: $0 --base <base> <user>@<node> [command...]" >&2; exit 64; }
    shift 2
    exec ssh -F "$base.ssh" "$@"
    ;;
  *)
    sed -n '3,32p' "${BASH_SOURCE[0]}" >&2
    exit 64
    ;;
esac
