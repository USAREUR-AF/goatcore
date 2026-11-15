#!/usr/bin/env bash
# join-node-reach.sh — join-node phase 1: the node answers before anything is done on it.
#
#   join-node-reach.sh ssh <user>@<address>   the node's SSH server answers
#   join-node-reach.sh usb                    one phone answers over the Android debug bridge
#
# ssh: one connection with the operator's own ssh (so an alias, a jump host or a
# door's configuration applies as it does for every later phase), batch mode, a
# 10 s connect timeout and no password prompt. The node has answered when the
# connection reached its SSH server: the login succeeded, or the server refused
# the login. Phase 1 cannot prove a login: before phase 2 seeds this machine's
# key, a node that takes only its first-login password refuses every batch login,
# and phase 2 is the step that turns that password into key auth. A connection
# that never reaches the server (no route, refused, timed out, a name that does
# not resolve, a changed host key) is the stop.
#
# usb: `adb get-state` says `device`: one phone is attached, allows USB debugging
# from this machine and is unlocked (ANDROID_SERIAL picks one of several).
#
# Exit: 0 the node answered; 1 it did not (GB-314, with the step for a person);
#       2 usage.
set -uo pipefail

case "${1:-}" in
  ssh)
    target="${2:?usage: join-node-reach.sh ssh <user>@<address>}"
    err="$(ssh -o BatchMode=yes -o ConnectTimeout=10 -o NumberOfPasswordPrompts=0 \
      -o StrictHostKeyChecking=accept-new "$target" true 2>&1 >/dev/null)"
    rc=$?
    if [ "$rc" -eq 0 ]; then
      echo ">> $target answers over SSH, and logs in"
      exit 0
    fi
    if printf '%s\n' "$err" | grep -qiE 'permission denied|too many authentication failures|no supported authentication'; then
      echo ">> $target answers over SSH; it refuses a key login until phase 2 seeds this machine's key"
      exit 0
    fi
    line="$(printf '%s\n' "$err" | grep -v '^Warning: Permanently added' | head -n 1)"
    echo "GB-314: $target does not answer over SSH (${line:-ssh exited $rc}); nothing was done on it." >&2
    echo "  Reach the box on the site LAN first: ssh $target. Find its real LAN address and pass it as --address." >&2
    echo "  On a Jetson the address handed over may be 192.168.55.1: that is l4tbr0, the USB device-mode bridge" >&2
    echo "  on the box itself, reachable only over a direct USB-C cable." >&2
    exit 1 ;;
  usb)
    state="$(adb get-state 2>&1)"
    if [ "$state" = device ]; then
      echo ">> a phone answers over USB${ANDROID_SERIAL:+ ($ANDROID_SERIAL)}"
      exit 0
    fi
    echo "GB-314: no phone answers over USB (adb get-state: $(printf '%s' "$state" | head -n 1)); nothing was done on it." >&2
    echo "  Attach the phone to this machine by USB, allow USB debugging on it, accept this machine's key on its" >&2
    echo "  screen, and unlock it: adb devices then lists it as device. With more than one device attached, set" >&2
    echo "  ANDROID_SERIAL." >&2
    exit 1 ;;
  *)
    echo "usage: join-node-reach.sh ssh <user>@<address> | usb" >&2
    exit 2 ;;
esac
