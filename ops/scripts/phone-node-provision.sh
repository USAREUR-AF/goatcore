#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# phone-node-provision.sh — turn an Android phone's built-in Linux VM into a
# host a goat node can run on, and refuse unless its integrity checks pass.
# docs/design/phone-node.md, "How the host is made";
# `goatbuilder join-node --class phone` runs it as phase 2.
#
# usage:
#   phone-node-provision.sh --node NAME [--user NAME] [--base BASE]
#                           [--prepare | --check] [--reset]
#                           [--pubkey FILE] [--memory-mib N]
#
#   --node NAME      the node's inventory name; it becomes the VM's hostname
#   --user NAME      the deploy user to create (default: deploy.ssh_user of
#                    ops/goatbuilder/goatbuilder.yaml)
#   --base BASE      where the SSH configuration for the node is kept
#                    (BASE.ssh, BASE.ini, BASE.known_hosts; written by
#                    phone-node-ssh.sh). Default: phone-node-NAME in the
#                    project's state directory
#   --prepare        prepare the VM only; do not run the integrity checks
#   --check          run the integrity checks only
#   --reset          reset the VM to its initial image first
#   --pubkey FILE    the public key the deploy user accepts (default: the
#                    first of ~/.ssh/id_ed25519.pub, id_ecdsa.pub, id_rsa.pub);
#                    read only when the VM is not prepared yet
#   --memory-mib N   set the VM's memory
#
# Run on the operator's computer with the phone attached by USB, USB debugging
# allowed and the phone unlocked. No root, no bootloader unlock and no wipe of
# the phone. With more than one device attached, set ANDROID_SERIAL.
#
# Preparing (each step is safe to repeat; a VM already prepared goes straight
# to the checks):
#   1. the phone: adb reach, support for a VM, the Linux Terminal app, unlocked
#   2. the Terminal app's Debian VM, installed and started
#   3. a first SSH login into the VM, by typing one line into the Terminal app
#   4. Debian's own kernel, the boot settings, Docker Engine, the deploy user
#      (phone-node-guest-prepare.sh, run in the VM)
#   5. the VM restarted on the new kernel
# Checking: phone-node-integrity.sh, run in the VM as the deploy user.
#
# A phone or VM that does not answer over adb is refused with GB-299. A VM
# that cannot be prepared, does not come back on Debian's kernel, or fails an
# integrity check is refused with GB-300.
#
# For the hermetic test only: PHONE_NODE_NAP replaces the length of every wait.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PKG=com.android.virtualization.terminal
VM_USER=droid   # the first user of the Terminal app's Debian image

NODE="" USER_NAME="" BASE="" PUBKEY_FILE="" MEMORY_MIB="" RESET=0 MODE=both
IDENTITY=()   # -i <key>, when --pubkey names one; else ssh's own default keys
while [ $# -gt 0 ]; do
  case "$1" in
    --node) NODE=$2; shift ;;
    --user) USER_NAME=$2; shift ;;
    --base) BASE=$2; shift ;;
    --pubkey) PUBKEY_FILE=$2; IDENTITY=(-i "${2%.pub}"); shift ;;
    --memory-mib) MEMORY_MIB=$2; shift ;;
    --reset) RESET=1 ;;
    --prepare) MODE=prepare ;;
    --check) MODE=check ;;
    -h|--help) sed -n '3,47p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 64 ;;
  esac
  shift
done
[ -n "$NODE" ] || { echo "usage: $0 --node NAME [--user NAME] [--base BASE] [--prepare | --check]" >&2; exit 64; }
[ -n "$USER_NAME" ] || USER_NAME=$("$HERE/project-setting.sh" deploy ssh_user 2>/dev/null || true)
[ -n "$USER_NAME" ] || { echo "no deploy user: pass --user, or set deploy.ssh_user in ops/goatbuilder/goatbuilder.yaml" >&2; exit 64; }
if [ -z "$BASE" ]; then
  state=$("$HERE/project-setting.sh" deploy state_dir 2>/dev/null || true)
  state=${state:-$HOME/.goat/state}
  BASE="${state/#\~/$HOME}/phone-node-$NODE"
fi

say() { printf '>> %s\n' "$*"; }
nap() { sleep "${PHONE_NODE_NAP:-$1}"; }
no_reach() { echo "GB-299: no Linux VM answers over the Android debug bridge: $*" >&2; exit 1; }
no_host() { echo "GB-300: the phone's Linux VM is not a host a node can be built on: $*" >&2; exit 1; }

"$HERE/phone-node-ssh.sh" --write "$BASE" --node "$NODE"

# ---- the VM over ssh -------------------------------------------------------
vm_ssh() {  # vm_ssh <user> <command>; the caller's standard input reaches the command
  ssh -F "$BASE.ssh" -o BatchMode=yes -o LogLevel=ERROR ${IDENTITY[@]+"${IDENTITY[@]}"} "$1@$NODE" "$2"
}
wait_ssh() {  # wait_ssh <user> <seconds>
  local i=0
  while [ "$i" -lt "$2" ]; do
    vm_ssh "$1" true </dev/null 2>/dev/null && return 0
    nap 5; i=$((i + 5))
  done
  return 1
}
vm_ip() {
  adb shell -n ip neigh show dev avf_tap_fixed 2>/dev/null | tr -d '\r' \
    | awk '$1 ~ /^10\./ && /lladdr/ {print $1}' | head -1
}
# adb's output is read whole before it is searched: under pipefail, a search
# that stops at its first match would fail the pipe it reads from.
adb_says() {  # adb_says <pattern> <adb shell arguments...>
  local pattern=$1 out; shift
  out=$(adb shell -n "$@" 2>/dev/null | tr -d '\r') || true
  grep -q -- "$pattern" <<<"$out"
}
vm_running() { adb_says 'name:' /apex/com.android.virt/bin/vm list; }
prepared() { vm_ssh "$USER_NAME" 'bash -s -- --prepared' < "$HERE/phone-node-integrity.sh" >/dev/null 2>&1; }

# ---- the phone's screen ----------------------------------------------------
ui_dump() { adb shell -n 'uiautomator dump /sdcard/.ui.xml >/dev/null 2>&1; cat /sdcard/.ui.xml'; }
ui_has() { local out; out=$(ui_dump) || true; grep -q -- "$1" <<<"$out"; }
# ui_point <attribute> <value>: the center of the first node whose attribute
# is the value (text and content-desc) or ends with it (resource-id).
ui_point() {
  ui_dump | python3 -c '
import re, sys, xml.etree.ElementTree as ET
attr, want = sys.argv[1], sys.argv[2]
for n in ET.fromstring(sys.stdin.read()).iter("node"):
    if attr == "id":
        hit = n.get("resource-id", "").endswith(want)
    else:
        hit = want in (n.get("text"), n.get("content-desc"))
    if hit:
        a = [int(x) for x in re.findall(r"\d+", n.get("bounds"))]
        print((a[0] + a[2]) // 2, (a[1] + a[3]) // 2)
        break
' "$1" "$2"
}
ui_tap() {  # ui_tap <exact text>
  local xy; xy=$(ui_point text "$1"); [ -n "$xy" ] || return 1
  adb shell -n "input tap $xy"
}
ui_tap_id() {  # ui_tap_id <resource-id suffix>
  local xy; xy=$(ui_point id "$1"); [ -n "$xy" ] || return 1
  adb shell -n "input tap $xy"
}
focus_is() { adb_says "mCurrentFocus=.*$1" dumpsys window; }
# Type one line into the Terminal app and press Enter. The line holds no single quote.
vm_type() { local t=${1// /%s}; adb shell -n "input text '$t'; input keyevent 66"; }

vm_restart() {
  # Stop: ask the VM to power off, wait until Android lists no VM, stop the app.
  vm_ssh "$VM_USER" 'sync; (sleep 1; sudo poweroff) >/dev/null 2>&1 &' </dev/null || true
  local i=0
  while vm_running && [ "$i" -lt 60 ]; do nap 3; i=$((i + 3)); done
  adb shell -n "am force-stop $PKG"; nap 3
  # Start: after an unexpected VM stop the app can open on its error screen;
  # stop it and start it again.
  local try=0
  while [ "$try" -lt 4 ]; do
    adb shell -n "am start -n $PKG/.LauncherActivity" >/dev/null 2>&1; nap 15
    if focus_is ErrorActivity; then
      adb shell -n "am force-stop $PKG"; nap 3
    elif vm_running; then
      return 0
    fi
    try=$((try + 1))
  done
  return 1
}

# ---- 1. the phone ----------------------------------------------------------
check_phone() {
  command -v adb >/dev/null 2>&1 || no_reach "adb is not installed on this machine (Android platform tools)"
  [ "$(adb get-state 2>/dev/null | tr -d '\r')" = device ] \
    || no_reach "no phone on adb (attach it by USB, allow USB debugging on it, unlock it; \`adb devices\` lists it as device)"
}

prepare() {
  local model release
  model=$(adb shell -n getprop ro.product.model | tr -d '\r')
  release=$(adb shell -n getprop ro.build.version.release | tr -d '\r')
  say "1. the phone: $model, Android $release"
  [ "$(adb shell -n getprop ro.boot.hypervisor.vm.supported | tr -d '\r')" = 1 ] \
    || no_host "$model does not support a VM (ro.boot.hypervisor.vm.supported)"
  adb_says "$PKG" pm list packages || no_host "$model has no Linux Terminal app ($PKG)"
  adb_says 'isKeyguardShowing=false' dumpsys window || no_reach "the phone is locked; unlock it and run again"
  adb shell -n settings put global stay_on_while_plugged_in 7

  if [ "$RESET" = 0 ] && prepared; then
    say "the VM is already prepared: Debian's kernel, the boot settings, Docker Engine and $USER_NAME are in place"
    return 0
  fi

  if [ -z "$PUBKEY_FILE" ]; then
    for f in "$HOME/.ssh/id_ed25519.pub" "$HOME/.ssh/id_ecdsa.pub" "$HOME/.ssh/id_rsa.pub"; do
      if [ -f "$f" ]; then PUBKEY_FILE=$f; break; fi
    done
    [ -n "$PUBKEY_FILE" ] || { echo "no public key under ~/.ssh: pass --pubkey FILE" >&2; exit 64; }
  fi
  local pubkey
  pubkey=$(cut -d' ' -f1,2 "$PUBKEY_FILE") || { echo "cannot read $PUBKEY_FILE" >&2; exit 64; }

  say "2. the Terminal app's VM"
  adb shell -n pm enable "$PKG" >/dev/null
  adb shell -n pm grant "$PKG" android.permission.POST_NOTIFICATIONS 2>/dev/null || true
  adb shell -n "am start -n $PKG/.LauncherActivity" >/dev/null; nap 4
  if [ "$RESET" = 1 ] && ! focus_is InstallerActivity; then
    say "   resetting the VM to its initial image"
    ui_tap_id ":id/settings_button" || no_host "the Terminal app shows no settings button"; nap 2
    ui_tap "Recovery" || true; nap 2
    ui_tap "Reset to initial version" || true; nap 2
    ui_tap "Reset" || no_host "the Terminal app shows no Reset confirmation"; nap 5
    rm -f "$BASE.known_hosts"
  fi
  if focus_is InstallerActivity; then
    say "   installing the Debian image (a download of about 760 MB)"
    ui_tap "Install" || no_host "the Terminal app shows no Install button"
    rm -f "$BASE.known_hosts"
  fi
  local i=0
  until [ -n "$(vm_ip)" ] || ui_has "$VM_USER@debian"; do
    nap 8; i=$((i + 8))
    [ "$i" -lt 600 ] || no_host "the VM did not reach a prompt in 10 minutes"
  done
  if [ "$RESET" = 0 ] && prepared; then
    say "the VM is already prepared: Debian's kernel, the boot settings, Docker Engine and $USER_NAME are in place"
    return 0
  fi

  say "3. first SSH login"
  if ! wait_ssh "$VM_USER" 10; then
    i=0
    until ui_has "$VM_USER@debian"; do
      nap 5; i=$((i + 5))
      [ "$i" -lt 300 ] || no_host "the Terminal app shows no shell prompt"
    done
    ui_tap_id ":id/webview" || true; nap 1
    vm_type "sudo apt-get update -qq && sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq openssh-server >/dev/null && mkdir -p ~/.ssh && echo $pubkey > ~/.ssh/authorized_keys && chmod 600 ~/.ssh/authorized_keys && sudo systemctl enable --now ssh"
    wait_ssh "$VM_USER" 300 || no_host "SSH into the VM did not come up"
  fi

  say "4. Debian's kernel, the boot settings, Docker Engine, the deploy user $USER_NAME"
  vm_ssh "$VM_USER" "USER_NAME='$USER_NAME' HOST_NAME='$NODE' PUBKEY='$pubkey' MEMORY_MIB='$MEMORY_MIB' bash -s" \
    < "$HERE/phone-node-guest-prepare.sh" || no_host "the VM could not be prepared (the step above names why)"

  say "5. restarting the VM on the new kernel"
  vm_restart || no_host "the Terminal app did not start the VM again (its settings, Recovery, reset it)"
  wait_ssh "$USER_NAME" 300 || no_host "the VM did not come back after the kernel change (the Terminal app's settings, Recovery, reset it)"
}

check() {
  say "integrity checks"
  wait_ssh "$USER_NAME" 30 || no_reach "$USER_NAME@$NODE does not answer over SSH (is the VM prepared? run this script without --check)"
  vm_ssh "$USER_NAME" 'bash -s' < "$HERE/phone-node-integrity.sh" \
    || no_host "an integrity check failed (the line above names it); nothing is built on this VM"
}

check_phone
case "$MODE" in
  prepare) prepare ;;
  check) check ;;
  both) prepare; check ;;
esac
say "done: $USER_NAME@$NODE"
