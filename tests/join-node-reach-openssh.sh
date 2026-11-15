#!/usr/bin/env bash
# tests/join-node-reach-openssh.sh: measure join-node's phase 1 check
# (ops/scripts/join-node-reach.sh) against real OpenSSH clients and servers.
#
# The fixture tests in crates/goatbuilder/tests/ use a stand-in ssh. This script
# runs the real script with each client image's own OpenSSH against real sshd
# servers on a disposable docker network:
#
#   answers (exit 0)                        does not answer (exit 1, GB-314)
#   - a password-only server                - a closed port
#   - a publickey-only server that refuses  - a port that speaks something else
#   - a server the key logs in to           - a name that does not resolve
#   - an ssh_config alias (HostName)        - a node behind a jump host that refuses
#   - a node behind a ProxyJump jump host   - a closed port behind a working jump host
#
# "A node behind a jump host that refuses" is the case a check reading the words
# of ssh's error gets wrong: the jump host's "Permission denied" is not the
# node's.
#
# Every container, the image and the network carry REACH_PFX and are removed on
# exit. The client key is made for the run, in a temporary directory, and removed.
#
# Required, with no default (a shared default prefix would collide with another run):
#   REACH_PFX        container, image and network name prefix, e.g. jnr1
# Optional:
#   REACH_CLIENTS    space-separated client images (default: the OpenSSH an operator's
#                    Linux machine is likely to have, oldest first)
#   REACH_SERVER     the base image the sshd servers are built from (default debian:13)
#
# Needs docker. Exit 0 when every case on every client gives the expected result.

set -euo pipefail

: "${REACH_PFX:?set REACH_PFX (a prefix of your own)}"
CLIENTS="${REACH_CLIENTS:-ubuntu:20.04 ubuntu:22.04 debian:12 ubuntu:24.04 alpine:3.20 debian:13 ubuntu:26.04}"
SERVER_BASE="${REACH_SERVER:-debian:13}"

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/.." && pwd)"
REACH="$REPO/ops/scripts/join-node-reach.sh"

P="$REACH_PFX"
NET="$P-net"
IMG="$P-sshd:test"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/join-node-reach-openssh.XXXXXX")"
SERVERS="$P-pw $P-pk $P-login $P-closed $P-notssh"

# shellcheck disable=SC2329 # run by the EXIT trap
cleanup() {
  for c in $SERVERS; do docker rm -f "$c" >/dev/null 2>&1 || true; done
  docker network rm "$NET" >/dev/null 2>&1 || true
  docker rmi "$IMG" >/dev/null 2>&1 || true
  rm -f "$WORK/id" "$WORK/id.pub" "$WORK/Dockerfile" "$WORK/client.sh"
  rmdir "$WORK" 2>/dev/null || true
}
trap cleanup EXIT

cat > "$WORK/Dockerfile" <<EOF
FROM $SERVER_BASE
RUN apt-get update -qq \\
 && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends openssh-server openssh-client socat >/dev/null \\
 && mkdir -p /run/sshd && useradd -m -s /bin/bash goat && echo 'goat:first-login-stand-in' | chpasswd
EOF
docker build -q -t "$IMG" "$WORK" >/dev/null
docker network create "$NET" >/dev/null

# The client key, made for this run; the host user keeps ownership so cleanup can remove it.
docker run --rm -v "$WORK:/k" "$IMG" sh -c "ssh-keygen -q -t ed25519 -N '' -C reach-test -f /k/id && chown $(id -u):$(id -g) /k/id /k/id.pub"

sshd='exec /usr/sbin/sshd -D -e'
docker run -d --name "$P-pw" --network "$NET" "$IMG" \
  sh -c "$sshd -o PubkeyAuthentication=no -o PasswordAuthentication=yes -o KbdInteractiveAuthentication=no" >/dev/null
docker run -d --name "$P-pk" --network "$NET" "$IMG" \
  sh -c "$sshd -o PasswordAuthentication=no -o KbdInteractiveAuthentication=no" >/dev/null
docker run -d --name "$P-login" --network "$NET" -v "$WORK:/k:ro" "$IMG" \
  sh -c "mkdir -p /home/goat/.ssh && cp /k/id.pub /home/goat/.ssh/authorized_keys && chown -R goat /home/goat/.ssh && chmod 700 /home/goat/.ssh && $sshd -o PasswordAuthentication=no -o KbdInteractiveAuthentication=no" >/dev/null
docker run -d --name "$P-closed" --network "$NET" "$IMG" sleep 3600 >/dev/null
docker run -d --name "$P-notssh" --network "$NET" "$IMG" \
  socat TCP-LISTEN:22,fork,reuseaddr SYSTEM:"echo HTTP/1.0 400 Bad Request" >/dev/null

# Inside each client: install its OpenSSH and bash, write an ssh_config with an
# alias and jump hosts, then run the real script once per case.
cat > "$WORK/client.sh" <<EOF
set -u
if command -v apt-get >/dev/null; then
  apt-get update -qq >/dev/null && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends openssh-client >/dev/null
else
  apk add -q --no-cache openssh-client bash >/dev/null
fi
echo "client \$(ssh -V 2>&1)"
mkdir -p /root/.ssh && cp /k/id /root/.ssh/id && chmod 600 /root/.ssh/id
cat > /root/.ssh/config <<CFG
Host node-alias
  HostName $P-pw
Host node-behind-jump
  HostName $P-pw
  ProxyJump goat@$P-login
Host node-behind-refusing-jump
  HostName $P-pw
  ProxyJump goat@$P-pk
Host closed-behind-jump
  HostName $P-closed
  ProxyJump goat@$P-login
Host $P-login $P-pk
  StrictHostKeyChecking accept-new
Host *
  IdentityFile /root/.ssh/id
  IdentitiesOnly yes
CFG
for t in goat@$P-pw goat@$P-pk goat@$P-login goat@node-alias goat@node-behind-jump \\
         goat@$P-closed goat@$P-notssh goat@no-such-node.invalid goat@node-behind-refusing-jump goat@closed-behind-jump; do
  err=\$(bash /r/join-node-reach.sh ssh "\$t" 2>&1 >/dev/null); rc=\$?
  echo "case \$t rc=\$rc first=\$(printf '%s' "\$err" | head -n 1)"
done
EOF

expect() {
  case "$1" in
    goat@"$P"-pw|goat@"$P"-pk|goat@"$P"-login|goat@node-alias|goat@node-behind-jump) echo 0 ;;
    *) echo 1 ;;
  esac
}

fail=0
for img in $CLIENTS; do
  out="$(docker run --rm --network "$NET" -v "$WORK:/k:ro" -v "$REACH:/r/join-node-reach.sh:ro" "$img" sh /k/client.sh 2>&1)" || true
  printf '%s\n' "$out" | grep '^client ' | sed "s|^|$img |"
  n=0
  while read -r _ t rcw rest; do
    [ -n "$t" ] || continue
    n=$((n + 1))
    rc="${rcw#rc=}"
    want="$(expect "$t")"
    if [ "$rc" != "$want" ]; then
      echo "FAIL $img $t: exit $rc, expected $want ($rest)"; fail=1
    elif [ "$rc" = 1 ] && [ "${rest#first=GB-314: }" = "$rest" ]; then
      echo "FAIL $img $t: exit 1 without GB-314 ($rest)"; fail=1
    else
      echo "ok   $img $t: exit $rc"
    fi
  done <<EOF
$(printf '%s\n' "$out" | grep '^case ')
EOF
  if [ "$n" -ne 10 ]; then
    echo "FAIL $img: $n of 10 cases ran"; printf '%s\n' "$out" | tail -n 5; fail=1
  fi
done
exit "$fail"
