#!/usr/bin/env bash
# join-node-reach.sh — join-node phase 1: the node answers before anything is done on it.
#
#   join-node-reach.sh ssh <user>@<address>   the node's SSH server answers
#   join-node-reach.sh usb                    one phone answers over the Android debug bridge
#
# ssh: one connection with the operator's own ssh, so an alias, a jump host or a
# door's configuration applies as it does for every later phase. It uses batch
# mode, a 10 s connect timeout and no password prompt. The node has answered when
# the login succeeded, or when the node's own host key arrived: the connection
# reached the node's SSH server and finished the key exchange. The key exchange
# comes before any login. The run gives ssh an empty known_hosts file of its own
# (UserKnownHostsFile, with StrictHostKeyChecking=accept-new), and ssh writes the
# node's key into it only then. A jump host's key never goes there: a ProxyJump
# or ProxyCommand ssh does not take these options. So the answer does not depend
# on the words of ssh's error, which differ between versions and cannot tell the
# node's "Permission denied" from a jump host's.
#
# Phase 1 cannot prove a login: before phase 2 seeds this machine's key, a node
# that takes only its first-login password refuses every batch login, and phase 2
# is the step that turns that password into key auth. A connection that never
# finishes a key exchange with the node is the stop: no route, refused, timed
# out, a name that does not resolve, a port that does not speak SSH, or a jump
# host that refuses. The operator's own known_hosts is not read or written here.
# Phase 2's ssh RECORDS a new node's key there (accept-new); it checks it only
# for a node this machine already knows. So phase 1 trusts any SSH server at the
# address on first use: the address is the operator's word for the node.
#
# One address is refused before any connection: 192.168.55.0/24, a Jetson's
# l4tbr0, the USB device-mode bridge on the box itself. Its sshd answers, so the
# check above would pass it, but it is reachable only over a direct USB-C cable,
# never from the site. The address is the one ssh would dial (`ssh -G`, so an
# alias's HostName counts), else the part after the @.
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
    host="$(ssh -G "$target" 2>/dev/null | awk '$1 == "hostname" { print $2; exit }')"
    [ -n "$host" ] || host="${target##*@}"
    case "$host" in
      192.168.55.*)
        echo "GB-314: $target is $host, a Jetson's l4tbr0: the USB device-mode bridge on the box itself, reachable only over a direct USB-C cable, never from the site; nothing was done on it." >&2
        echo "  Find the box's real LAN address (on the box: ip -4 addr, the interface on the site LAN, not l4tbr0) and pass it as --address." >&2
        exit 1 ;;
    esac
    kh="$(mktemp "${TMPDIR:-/tmp}/join-node-reach.XXXXXX")" || exit 1
    trap 'rm -f "$kh"' EXIT
    err="$(ssh -o BatchMode=yes -o ConnectTimeout=10 -o NumberOfPasswordPrompts=0 \
      -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile="$kh" -o GlobalKnownHostsFile=/dev/null \
      "$target" true 2>&1 >/dev/null)"
    rc=$?
    if [ "$rc" -eq 0 ]; then
      echo ">> $target answers over SSH, and logs in"
      exit 0
    fi
    if [ -s "$kh" ]; then
      echo ">> $target answers over SSH (its host key arrived); it refuses a key login until phase 2 seeds this machine's key"
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
