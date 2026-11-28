#!/usr/bin/env bash
# join-node-reach.sh — join-node phase 1: the node answers before anything is done on it.
#
#   join-node-reach.sh ssh <user>@<address> [<node> <inventory file>]
#                                             the node's SSH server answers; with a node and
#                                             its inventory file, through the jump host that
#                                             line declares (-o ProxyJump=...)
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
    target="${2:?usage: join-node-reach.sh ssh <user>@<address> [<node> <inventory file>]}"
    # The jump host the node's inventory line declares (ansible_ssh_common_args:
    # -o ProxyJump=<host>), when the caller names the node and its inventory:
    # every later phase reaches the node through it, so this check does too. A
    # node behind a jump that only the inventory names was refused here as
    # unreachable. Only a ProxyJump of plain host characters is taken.
    jump=""
    if [ -n "${3:-}" ] && [ -f "${4:-}" ]; then
      jump="$(awk -v n="$3" '$1 == n' "$4" | grep -oE 'ProxyJump=[A-Za-z0-9@._:,-]+' | head -1 | cut -d= -f2)"
    fi
    host="$(ssh -G "$target" 2>/dev/null | awk '$1 == "hostname" { print $2; exit }')"
    [ -n "$host" ] || host="${target##*@}"
    case "$host" in
      192.168.55.*)
        echo "GB-314: $target is $host, a Jetson's l4tbr0: the USB device-mode bridge on the box itself, reachable only over a direct USB-C cable, never from the site; nothing was done on it." >&2
        echo "  Find the box's real LAN address (on the box: ip -4 addr, the interface on the site LAN, not l4tbr0) and pass it as --address." >&2
        exit 1 ;;
    esac
    # The run's own known_hosts, in a folder of its own under /tmp: ssh splits
    # UserKnownHostsFile on whitespace and expands % in it, so a TMPDIR with a
    # space or a % would name another file, and a good node would fail.
    dir="$(mktemp -d /tmp/join-node-reach.XXXXXX)" || exit 1
    case "$dir" in *[[:space:]%]*|'') echo "join-node-reach: unsafe folder '$dir'" >&2; exit 1 ;; esac
    kh="$dir/known_hosts"
    errf="$dir/stderr"
    : > "$kh"
    bound="$dir/bound"
    trap 'rm -f "$kh" "$errf" "$bound"; rmdir "$dir" 2>/dev/null' EXIT
    # The whole check is bounded (JOIN_NODE_REACH_LIMIT seconds, default 30). A
    # ProxyJump or ProxyCommand ssh takes none of the options below; measured on
    # OpenSSH 8.2 to 10.2, ConnectTimeout still ends a jump host that drops
    # packets, or one whose key is unknown, at the banner (10 s), and the jump's
    # ssh exits with it. The bound covers what ConnectTimeout does not, such as
    # a server that sends its banner and then stalls the key exchange.
    limit="${JOIN_NODE_REACH_LIMIT:-30}"
    case "$limit" in ''|*[!0-9]*) echo "join-node-reach: JOIN_NODE_REACH_LIMIT is '$limit', not a number of seconds" >&2; exit 2 ;; esac
    opts=(-o BatchMode=yes -o ConnectTimeout=10 -o NumberOfPasswordPrompts=0
      -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile="$kh" -o GlobalKnownHostsFile=/dev/null)
    # A KnownHostsCommand in the operator's ssh_config (OpenSSH 8.5 and later)
    # would vouch for the node and keep its key out of the run's file.
    if ssh -G -o KnownHostsCommand=none "$target" >/dev/null 2>&1; then
      opts+=(-o KnownHostsCommand=none)
    fi
    if [ -n "$jump" ]; then opts+=(-o "ProxyJump=$jump"); fi
    ssh "${opts[@]}" "$target" true </dev/null >/dev/null 2>"$errf" &
    pid=$!
    # The watchdog clears the EXIT trap first, so it can never remove the
    # run's files (the stderr file among them) if a signal reaches it.
    ( trap - EXIT; sleep "$limit"; : > "$bound"; kill -TERM "$pid" ) </dev/null >/dev/null 2>&1 &
    dog=$!
    wait "$pid"; rc=$?
    # ssh's stderr is read before the watchdog is stopped. When ssh fails and
    # says nothing, the message says which case it was: the file was gone, or
    # it was there and empty, so a failure names its cause.
    if [ -e "$errf" ]; then
      err="$(cat "$errf" 2>/dev/null)"; silent="ssh exited $rc and printed nothing on stderr"
    else
      err=""; silent="ssh exited $rc; its stderr file $errf was gone before it was read"
    fi
    kill -TERM "$dog" 2>/dev/null
    if [ -e "$bound" ]; then
      err="no answer within the ${limit} s bound, JOIN_NODE_REACH_LIMIT${err:+; ssh said: $(printf '%s\n' "$err" | grep -v '^Warning: Permanently added' | head -n 1)}"
    fi
    if [ "$rc" -eq 0 ]; then
      echo ">> $target answers over SSH, and logs in"
      exit 0
    fi
    if [ -s "$kh" ]; then
      echo ">> $target answers over SSH (its host key arrived); it refuses a key login until phase 2 seeds this machine's key"
      exit 0
    fi
    line="$(printf '%s\n' "$err" | grep -v '^Warning: Permanently added' | head -n 1)"
    echo "GB-314: $target does not answer over SSH (${line:-$silent}); nothing was done on it." >&2
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
