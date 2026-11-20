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
# Phase 1's bound for these runs (JOIN_NODE_REACH_LIMIT), and a jump host that
# drops every packet (a documentation address that no route answers).
LIMIT=15
DEAD=192.0.2.1

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/.." && pwd)"
REACH="$REPO/ops/scripts/join-node-reach.sh"

P="$REACH_PFX"
NET="$P-net"
IMG="$P-sshd:test"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/join-node-reach-openssh.XXXXXX")"
SERVERS="$P-pw $P-pk $P-login $P-closed $P-notssh $P-client"

# shellcheck disable=SC2329 # run by the EXIT trap
cleanup() {
  for c in $SERVERS; do docker rm -f "$c" >/dev/null 2>&1 || true; done
  docker network rm "$NET" >/dev/null 2>&1 || true
  docker rmi "$IMG" >/dev/null 2>&1 || true
  rm -f "$WORK/id" "$WORK/id.pub" "$WORK/Dockerfile" "$WORK/client.sh" "$WORK/tty-case.sh" "$WORK/stall.sh"
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

# The terminal case runs inside `script`, and counts the ssh processes still
# alive in the client when the check has ended, before the terminal closes: a jump host's ssh
# left asking on the operator's terminal is a failure.
cat > "$WORK/tty-case.sh" <<'TTY'
bash /r/join-node-reach.sh ssh goat@node-behind-unknown-jump; rc=$?
# Alive: not a zombie (the container's first process reaps no orphans).
echo "left=$(cat /proc/[0-9]*/stat 2>/dev/null | awk '$2 == "(ssh)" && $3 != "Z"' | wc -l | tr -d ' ')"
exit $rc
TTY

# A route that sends an SSH banner and then never answers the key exchange (a
# ProxyCommand): ConnectTimeout does not end that, so only the check's own
# bound does.
cat > "$WORK/stall.sh" <<'STALL'
printf 'SSH-2.0-stall\r\n'
exec sleep 3600
STALL

# Inside each client: install its OpenSSH and bash, write an ssh_config with an
# alias and jump hosts, then run the real script once per case. Each case line
# is `case <label> rc=<exit> secs=<seconds> first=<first stderr line>`; a case the
# client's OpenSSH cannot express is `skip <label> <why>`.
cat > "$WORK/client.sh" <<EOF
set -u
if command -v apt-get >/dev/null; then
  apt-get update -qq >/dev/null && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends openssh-client >/dev/null
else
  apk add -q --no-cache openssh-client bash util-linux-misc >/dev/null
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
Host node-behind-dead-jump
  HostName $P-pw
  ProxyJump goat@$DEAD
Host unknown-jump
  HostName $P-login
  UserKnownHostsFile /root/.ssh/known_hosts_of_nobody
Host node-behind-unknown-jump
  HostName $P-pw
  ProxyJump goat@unknown-jump
Host node-that-stalls
  ProxyCommand /bin/sh /k/stall.sh
Host $P-login $P-pk
  StrictHostKeyChecking accept-new
CFG
# KnownHostsCommand (OpenSSH 8.5 and later): a command that vouches for the node.
vouched=
if ssh -G -o KnownHostsCommand=none x >/dev/null 2>&1; then
  ssh-keyscan $P-pw > /root/vouched 2>/dev/null
  printf 'Host node-vouched\n  HostName $P-pw\n  KnownHostsCommand /bin/cat /root/vouched\n' >> /root/.ssh/config
  vouched=goat@node-vouched
else
  echo "skip goat@node-vouched this OpenSSH has no KnownHostsCommand"
fi
printf 'Host *\n  IdentityFile /root/.ssh/id\n  IdentitiesOnly yes\n' >> /root/.ssh/config
export JOIN_NODE_REACH_LIMIT=$LIMIT
mkdir -p '/tmp/a b%c'
one() { # \$1 label, then the command
  label=\$1; shift
  t0=\$(date +%s)
  err=\$("\$@" 2>&1); rc=\$?
  echo "case \$label rc=\$rc secs=\$((\$(date +%s) - t0)) first=\$(printf '%s\n' "\$err" | grep 'GB-314' | head -n 1)\$(printf '%s\n' "\$err" | grep -v 'GB-314' | head -n 1) \$(printf '%s\n' "\$err" | grep -o 'left=[0-9]*' | tail -n 1)"
}
for t in goat@$P-pw goat@$P-pk goat@$P-login goat@node-alias goat@node-behind-jump \\
         goat@$P-closed goat@$P-notssh goat@no-such-node.invalid goat@node-behind-refusing-jump goat@closed-behind-jump \\
         goat@node-behind-dead-jump goat@node-behind-unknown-jump goat@node-that-stalls \$vouched; do
  one "\$t" bash /r/join-node-reach.sh ssh "\$t"
done
one "tmpdir:goat@$P-pw" env TMPDIR='/tmp/a b%c' bash /r/join-node-reach.sh ssh goat@$P-pw
# A terminal: the jump host's unknown key would be asked about on it.
one "tty:goat@node-behind-unknown-jump" script -qec "sh /k/tty-case.sh" /dev/null
EOF

expect() {
  case "$1" in
    goat@"$P"-pw|goat@"$P"-pk|goat@"$P"-login|goat@node-alias|goat@node-behind-jump|tmpdir:*|goat@node-vouched) echo 0 ;;
    *) echo 1 ;;
  esac
}

CASES=16
fail=0
for img in $CLIENTS; do
  # Every case is bounded, so a client's whole run is too: one that hangs is a
  # failure (its container is removed with the rest), never a stall.
  out="$(docker run --rm --name "$P-client" --network "$NET" -v "$WORK:/k:ro" -v "$REACH:/r/join-node-reach.sh:ro" "$img" \
    timeout "$((CASES * (LIMIT + 5) + 300))" sh /k/client.sh 2>&1)" || true
  printf '%s\n' "$out" | grep '^client ' | sed "s|^|$img |"
  printf '%s\n' "$out" | { grep '^skip ' || true; } | sed "s|^|$img |"
  n="$(printf '%s\n' "$out" | grep -c '^skip ' || true)"
  while read -r _ t rcw secsw rest; do
    [ -n "$t" ] || continue
    n=$((n + 1))
    rc="${rcw#rc=}"
    secs="${secsw#secs=}"
    want="$(expect "$t")"
    if [ "$rc" != "$want" ]; then
      echo "FAIL $img $t: exit $rc, expected $want ($rest)"; fail=1
    elif [ "$rc" = 1 ] && [ "${rest#*GB-314: }" = "$rest" ]; then
      echo "FAIL $img $t: exit 1 without GB-314 ($rest)"; fail=1
    elif [ "${t#tty:}" != "$t" ] && [ "${rest#*left=0}" = "$rest" ]; then
      echo "FAIL $img $t: an ssh was left running on the terminal ($rest)"; fail=1
    elif [ "$secs" -gt $((LIMIT + 5)) ]; then
      echo "FAIL $img $t: took $secs s, over the $LIMIT s bound ($rest)"; fail=1
    else
      echo "ok   $img $t: exit $rc in $secs s"
    fi
  done <<EOF
$(printf '%s\n' "$out" | grep '^case ' || true)
EOF
  if [ "$n" -ne "$CASES" ]; then
    echo "FAIL $img: $n of $CASES cases ran"; printf '%s\n' "$out" | tail -n 5; fail=1
  fi
done
exit "$fail"
