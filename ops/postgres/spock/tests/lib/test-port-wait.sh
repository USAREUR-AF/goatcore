#!/usr/bin/env bash
# Tests for ts_run_published (two-site.sh): starting a container whose port is
# published on 127.0.0.1 while that port is briefly held.
#
# Why a port can be held with nothing listening on it: the harness's host
# ports lie in Linux's ephemeral range (net.ipv4.ip_local_port_range,
# 32768-60999 by default), so the kernel can give one of them to any loopback
# client connection as its source port: a psql, a curl health probe, another
# harness run. While that connection is open, and for the 60 s it then spends
# in TIME-WAIT, docker cannot bind the port and `docker run -p` fails with
# "address already in use". The port frees by itself.
#
# Part 1 needs no Docker: stand-in docker and ss commands on PATH.
# Part 2 needs Linux Docker and IMAGE (any image with `sleep`, CI passes the
# postgres-spock image it builds). It reproduces the failure with a real
# TIME-WAIT socket, then shows ts_run_published waiting it out. Without IMAGE
# or Docker, part 2 is reported as skipped.
#
# Usage: ops/postgres/spock/tests/lib/test-port-wait.sh
#        IMAGE=postgres-spock:smoke ops/postgres/spock/tests/lib/test-port-wait.sh

set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"

fails=0
ok()   { echo "  ok    $1"; }
fail() { echo "  FAIL  $1"; fails=$((fails + 1)); }
expect() { # expect <what> <got> <want>
  if [[ "$2" == "$3" ]]; then ok "$1 = $3"; else fail "$1: got '$2', want '$3'"; fi
}
expect_in() { # expect_in <what> <file> <fixed string>
  if grep -qF -- "$3" "$2"; then ok "$1"; else fail "$1: '$3' not in: $(tr '\n' ' ' < "$2")"; fi
}

T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT

# Source the library in a subshell per case, with PATH led by the stand-ins.
# shellcheck disable=SC2016  # $1 is the inner shell's argument
run_case() { # run_case <port> -- runs ts_run_published <port> docker run ... ; echoes rc
  local port="$1"
  set +e
  PATH="$T/bin:$PATH" TS_PORT_STEP="${TS_PORT_STEP:-1}" TS_PORT_WAIT="${TS_PORT_WAIT:-10}" \
    bash -c 'source "$0"; ts_run_published "$1" docker run -d --name harness-x -p "127.0.0.1:$1:80" img sleep 1' \
    "$HERE/two-site.sh" "$port" >"$T/out" 2>"$T/err"
  echo $?
  set -e
}

stub() { # stub <docker-fails-this-many-times> <docker stderr> <ss output>
  rm -rf "${T:?}/bin" "${T:?}/calls"; mkdir -p "$T/bin"; : > "$T/calls"
  printf '%s\n' "$2" > "$T/docker-err"; printf '%s' "$3" > "$T/ss-out"; echo "$1" > "$T/left"
  cat > "$T/bin/docker" <<EOF
#!/usr/bin/env bash
echo "docker \$*" >> "$T/calls"
case "\$1" in
  run)
    left=\$(cat "$T/left")
    if [ "\$left" -gt 0 ]; then echo \$((left - 1)) > "$T/left"; cat "$T/docker-err" >&2; exit 125; fi
    echo cid; exit 0 ;;
  ps) echo "other-slot-kratos"; exit 0 ;;
  *) exit 0 ;;
esac
EOF
  cat > "$T/bin/ss" <<EOF
#!/usr/bin/env bash
cat "$T/ss-out"
EOF
  chmod +x "$T/bin/docker" "$T/bin/ss"
}
calls() { grep -c "^docker $1" "$T/calls" || true; }

INUSE='docker: Error response from daemon: failed to set up container networking: driver failed programming external connectivity on endpoint harness-x (0b66): failed to bind host port 127.0.0.1:57736/tcp: address already in use'

echo "== part 1: stand-in docker and ss"

echo "-- a port in TIME-WAIT as a client's source port: wait, then start"
stub 2 "$INUSE" $'TIME-WAIT 0 0 127.0.0.1:57736 127.0.0.1:57733\n'
expect "rc" "$(run_case 57736)" 0
expect "docker run attempts" "$(calls run)" 3
expect "half-made container removed before each retry" "$(calls 'rm -f harness-x')" 2
expect_in "names the holder" "$T/err" "TIME-WAIT 127.0.0.1:57736 127.0.0.1:57733"
expect_in "says why the port is held" "$T/err" "ephemeral"
expect_in "says the wait is bounded" "$T/err" "of 10s"

echo "-- an open client connection on the port: wait, then start"
stub 1 "$INUSE" $'ESTAB 0 0 127.0.0.1:57736 127.0.0.1:57734\n'
expect "rc" "$(run_case 57736)" 0
expect "docker run attempts" "$(calls run)" 2
expect_in "names the holder" "$T/err" "ESTAB 127.0.0.1:57736 127.0.0.1:57734"

echo "-- a listener on the port: no wait, and the container that publishes it is named"
stub 5 "$INUSE" $'LISTEN 0 4096 127.0.0.1:57736 0.0.0.0:*\n'
expect "rc" "$(run_case 57736)" 125
expect "docker run attempts" "$(calls run)" 1
expect_in "names the listening container" "$T/err" "other-slot-kratos"
expect_in "says it will not free by itself" "$T/err" "will not free by itself"

echo "-- docker's own container publishes the port: no wait, and that container is named"
stub 5 'docker: Error response from daemon: failed to set up container networking: driver failed programming external connectivity on endpoint harness-x (f393): Bind for 127.0.0.1:57736 failed: port is already allocated' ''
expect "rc" "$(run_case 57736)" 125
expect "docker run attempts" "$(calls run)" 1
expect "half-made container removed" "$(calls 'rm -f harness-x')" 1
expect_in "names the publishing container" "$T/err" "is published by other-slot-kratos"

echo "-- a listener on another address does not count"
stub 1 "$INUSE" $'LISTEN 0 4096 100.127.0.9%tailscale0:57736 0.0.0.0:*\n'
expect "rc" "$(run_case 57736)" 0
expect "docker run attempts" "$(calls run)" 2

echo "-- still held when the bound runs out: stop with the holder named"
stub 99 "$INUSE" $'TIME-WAIT 0 0 127.0.0.1:57736 127.0.0.1:57733\n'
expect "rc" "$(TS_PORT_WAIT=3 run_case 57736)" 125
expect "docker run attempts (one per second for 3s, then the last)" "$(calls run)" 4
expect_in "says the port is still held" "$T/err" "127.0.0.1:57736 is still held after 3s"
expect_in "names the holder at the end" "$T/err" "TIME-WAIT 127.0.0.1:57736 127.0.0.1:57733"

echo "-- held, but ss sees nothing: wait and say so"
stub 1 "$INUSE" ''
expect "rc" "$(run_case 57736)" 0
expect_in "says no socket was visible" "$T/err" "no socket ss can see"

echo "-- any other docker failure: no retry, docker's own message and rc"
stub 5 'docker: Error response from daemon: pull access denied for img' ''
expect "rc" "$(run_case 57736)" 125
expect "docker run attempts" "$(calls run)" 1
expect_in "passes docker's message through" "$T/err" "pull access denied for img"

echo "-- success first time: no message"
stub 0 '' ''
expect "rc" "$(run_case 57736)" 0
expect "stderr lines" "$(wc -l < "$T/err" | tr -d ' ')" 0
expect "container id on stdout" "$(cat "$T/out")" cid

echo "== part 2: real Docker, a real TIME-WAIT socket"
if [[ -z "${IMAGE:-}" ]] || ! docker info >/dev/null 2>&1 || [[ "$(uname -s)" != Linux ]]; then
  echo "  skip  part 2 needs Linux Docker and IMAGE"
else
  C="port-wait-test-$$"
  cleanup_real() { docker rm -f "$C" >/dev/null 2>&1 || true; }
  # A loopback client takes a port the kernel picks, and closes first: its
  # source port is then in TIME-WAIT for 60 s, with nothing listening on it.
  P="$(python3 - <<'EOF'
import socket, time
srv = socket.socket(); srv.bind(("127.0.0.1", 0)); srv.listen(1)
c = socket.socket(); c.connect(srv.getsockname()); a, _ = srv.accept()
port = c.getsockname()[1]
c.close(); time.sleep(0.2); a.close(); srv.close()
print(port)
EOF
)"
  state="$(ss -Htan "sport = :$P" | awk '{print $1}' | head -n 1)"
  expect "port $P state after the client closed" "$state" TIME-WAIT
  expect "listeners on port $P" "$(ss -Hltn "sport = :$P" | wc -l | tr -d ' ')" 0

  set +e
  docker run -d --name "$C" -p "127.0.0.1:$P:5432" --entrypoint sleep "$IMAGE" 120 >/dev/null 2>"$T/plain"
  rc=$?
  set -e
  cleanup_real
  if [[ "$rc" != 0 ]] && grep -q 'address already in use' "$T/plain"; then
    ok "plain docker run -p on the TIME-WAIT port fails: address already in use (the failure this fixes)"
  else
    fail "plain docker run -p on port $P: rc $rc, want 125 with 'address already in use': $(cat "$T/plain")"
  fi

  set +e
  # shellcheck disable=SC2016  # $0..$3 are the inner shell's arguments
  TS_PORT_STEP=5 TS_PORT_WAIT=90 bash -c 'source "$0"; ts_run_published "$1" docker run -d --name "$2" -p "127.0.0.1:$1:5432" --entrypoint sleep "$3" 120' \
    "$HERE/two-site.sh" "$P" "$C" "$IMAGE" >/dev/null 2>"$T/real"
  rc=$?
  set -e
  running="$(docker inspect -f '{{.State.Running}}' "$C" 2>/dev/null || echo missing)"
  cleanup_real
  expect "ts_run_published on the same port, rc" "$rc" 0
  expect "its container is running" "$running" true
  expect_in "it named the TIME-WAIT holder while waiting" "$T/real" "TIME-WAIT 127.0.0.1:$P"

  # A port another container publishes: stop at once, name it, leave nothing.
  O="port-wait-owner-$$"
  docker run -d --name "$O" -p "127.0.0.1:$P:5432" --entrypoint sleep "$IMAGE" 120 >/dev/null
  set +e
  # shellcheck disable=SC2016  # $0..$3 are the inner shell's arguments
  TS_PORT_STEP=1 TS_PORT_WAIT=30 bash -c 'source "$0"; ts_run_published "$1" docker run -d --name "$2" -p "127.0.0.1:$1:5432" --entrypoint sleep "$3" 120' \
    "$HERE/two-site.sh" "$P" "$C" "$IMAGE" >/dev/null 2>"$T/owned"
  rc=$?
  set -e
  left="$(docker ps -a --filter "name=^$C\$" --format '{{.Names}}' | wc -l | tr -d ' ')"
  docker rm -f "$O" >/dev/null 2>&1 || true
  cleanup_real
  if [[ "$rc" != 0 ]]; then ok "ts_run_published on a port $O publishes fails"; else fail "ts_run_published on a port $O publishes: rc 0"; fi
  expect_in "it names $O" "$T/owned" "is published by $O"
  expect "containers it left behind" "$left" 0
fi

if [[ "$fails" -gt 0 ]]; then echo "== $fails check(s) failed"; exit 1; fi
echo "== all checks passed"
