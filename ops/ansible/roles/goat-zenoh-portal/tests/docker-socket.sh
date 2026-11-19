#!/usr/bin/env bash
# GHSA-6828: with the role defaults (portal_acl_docker_socket empty) the
# docker-socket tasks must not fail on the empty path, must leave a socket
# they did not regroup alone, and must give back one an earlier deploy gave
# to the portal's gid, idempotently. Runs as the invoking user on scratch
# files; never touches /var/run/docker.sock.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
fails=0
fail() { echo "  FAIL $1"; fails=$((fails + 1)); }

# A group the invoker belongs to other than their primary one stands in for
# the daemon's group, so the regroup is observable without root.
my_gid="$(id -g)"
target_gid="$(id -G | tr ' ' '\n' | grep -vx "$my_gid" | head -1 || true)"
target_gid="${target_gid:-$my_gid}"
target_group="$(getent group "$target_gid" | cut -d: -f1)"

play() {  # play <extra args...>; prints the recap line, returns ansible's rc
  ansible-playbook -i localhost, --connection local "$SCRIPT_DIR/docker-socket.yml" "$@" 2>&1
}
recap() { printf '%s\n' "$1" | grep -E '^localhost +:' || true; }

# 1. Role defaults, nothing at the socket path: the play passes.
out="$(play -e portal_docker_host_socket="$work/absent.sock")" || { fail "defaults, no socket: play failed"; printf '%s\n' "$out" | tail -5; }
[ "$fails" -eq 0 ] && echo "  ok   defaults with an empty portal_acl_docker_socket do not fail ($(recap "$out" | tr -s ' '))"

# 2. Residue from the old role: a socket with the portal's gid is given back.
f="$work/residue.sock"; : > "$f"; chmod 0644 "$f"
args=(-e portal_docker_host_socket="$f" -e portal_docker_socket_residue_gid="$my_gid" -e portal_docker_socket_group="$target_group")
if out="$(play "${args[@]}")"; then
  got="$(stat -c '%g %a' "$f")"
  if [ "$got" = "$target_gid 660" ] && recap "$out" | grep -q 'changed=1'; then echo "  ok   residue regrouped to $target_group, mode 0660"
  else fail "residue: got '$got', want '$target_gid 660'; $(recap "$out")"; fi
else fail "residue: play failed"; printf '%s\n' "$out" | tail -5; fi

# 3. Second run on the same file changes nothing.
if out="$(play "${args[@]}")" && recap "$out" | grep -q 'changed=0'; then echo "  ok   second run is idempotent"
else fail "second run: $(recap "$out")"; fi

# 4. A socket whose group is not the portal's is left as it is.
g="$work/other.sock"; : > "$g"; chmod 0644 "$g"
if out="$(play -e portal_docker_host_socket="$g" -e portal_docker_socket_residue_gid=65532)" \
   && [ "$(stat -c '%a' "$g")" = 644 ]; then echo "  ok   a socket not regrouped by the portal is left alone"
else fail "other socket touched or play failed: $(stat -c '%g %a' "$g"); $(recap "$out")"; fi

# 5. Opt-in (variable set): the grant task runs. Check mode: no root needed.
h="$work/optin.sock"; : > "$h"
if out="$(play --check -e portal_acl_docker_socket="$h" -e portal_docker_host_socket="$work/absent.sock")" \
   && printf '%s\n' "$out" | grep -A3 'Ensure docker socket is group-accessible' | grep -q '^changed: \[localhost\]'; then echo "  ok   opt-in still grants the socket to the portal (check mode)"
else fail "opt-in: $(recap "$out")"; printf '%s\n' "$out" | tail -8; fi

[ "$fails" -eq 0 ] || { echo "goat-zenoh-portal: docker-socket FAILED ($fails)"; exit 1; }
echo "goat-zenoh-portal: docker-socket OK"
