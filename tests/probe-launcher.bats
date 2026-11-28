#!/usr/bin/env bats
# shellcheck shell=bash

# The deployment probe launcher (ops/federation/standup/probe-launcher.sh) meets the §2.4
# contract of docs/design/federation-acceptance.md: start prints HANDLE, REST and ZID and the
# session ends by itself at its deadline; stop ends it; alive exits 0 while it runs and 1
# only once it is gone. It runs on an edge host over ssh (or on this host), installs only
# the identity files it is given, keeps the key private, takes its image from the host's
# pins, and binds the probe's REST port to one address. ssh, sudo, docker and curl are
# stubs here: ssh runs the remote command on this machine and logs the host it was given.

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
PL="$REPO_ROOT/ops/federation/standup/probe-launcher.sh"
DIGEST=sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef

setup() {
  S="$BATS_TEST_TMPDIR"
  mkdir -p "$S/bin" "$S/docker" "$S/ids"
  cat > "$S/bin/ssh" <<'EOF'
#!/usr/bin/env bash
# ssh [-o opt]... host command...: log the host, run the command here.
while [ "${1:-}" = -o ]; do shift 2; done
printf '%s\n' "$1" >> "$STUB/ssh.log"; shift
exec sh -c "$*"
EOF
  cat > "$S/bin/sudo" <<'EOF'
#!/usr/bin/env bash
exec "$@"
EOF
  cat > "$S/bin/docker" <<'EOF'
#!/usr/bin/env bash
# A container is a marker file named after it.
printf '%s\n' "$*" >> "$STUB/docker.log"
case "$1" in
  run) while [ $# -gt 0 ]; do [ "$1" = --name ] && touch "$STUB/docker/$2"; shift; done ;;
  rm) rm -f "$STUB/docker/$3" ;;
  inspect) [ -e "$STUB/docker/$2" ] ;;
esac
EOF
  cat > "$S/bin/curl" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "${*: -1}" >> "$STUB/curl.log"
[ ! -e "$STUB/curl-fails" ]
EOF
  chmod +x "$S/bin/"*
  for k in agreed forged impersonate; do
    printf 'CERT %s\n' "$k" > "$S/ids/$k-cert.pem"; printf 'KEY %s\n' "$k" > "$S/ids/$k-key.pem"
  done
  printf 'FAR CA\n' > "$S/ids/far-ca.pem"
  printf 'portal_image: "ghcr.io/example-org/portal@%s"\nzenoh_router_image: "ghcr.io/example-org/zenoh-pqc@%s"\n' "$DIGEST" "$DIGEST" > "$S/pins.yml"
  export STUB="$S" PATH="$S/bin:$PATH"
  export FED_PL_SSH=deploy@192.0.2.10 FED_PL_REST_IP=192.0.2.10 FED_PL_FAR_IP=169.254.10.1 \
    FED_PL_DIR="$S/ids" FED_PL_PORT=18600 FED_PL_PINS="$S/pins.yml" FED_PL_STATE="$S/state" FED_PL_WAIT_STEP=0
  unset FED_PL_LOCAL FED_PL_FAR_PORT FED_PL_TEMPLATE
}

teardown() {
  # A start leaves its deadline timer behind; end this test's, and only this test's.
  pkill -f "$BATS_TEST_TMPDIR/state" 2>/dev/null || true
}

mode() { stat -c %a "$1" 2>/dev/null || stat -f %Lp "$1"; }
later() { echo $(( $(date +%s) + ${1:-30} )); }

@test "probe-launcher: start prints HANDLE, REST and ZID, one per line, on the kind's port" {
  run "$PL" start forged run-1 "$(later)"
  [ "$status" -eq 0 ]
  [[ "$output" == *"HANDLE=goat-probe-forged-run-1"* ]]
  [[ "$output" == *"REST=http://192.0.2.10:18601"* ]]
  zid=$(printf '%s\n' "$output" | sed -n 's/^ZID=//p')
  [[ "$zid" =~ ^[0-9a-f]{4,32}$ ]]
  grep -qx 'deploy@192.0.2.10' "$STUB/ssh.log"
}

@test "probe-launcher: the key stays private, the certificates and config are readable, the folder is root's" {
  run "$PL" start agreed run-2 "$(later)"
  [ "$status" -eq 0 ]
  d="$FED_PL_STATE/goat-probe-agreed-run-2"
  [ "$(mode "$d")" = 700 ]
  [ "$(mode "$d/key.pem")" = 600 ]
  [ "$(mode "$d/cert.pem")" = 644 ]
  [ "$(mode "$d/far-ca.pem")" = 644 ]
  [ "$(cat "$d/key.pem")" = "KEY agreed" ]
  [ "$(cat "$d/cert.pem")" = "CERT agreed" ]
}

@test "probe-launcher: the config binds REST to the one address and dials the far edge's link" {
  run "$PL" start impersonate run-3 "$(later)"
  [ "$status" -eq 0 ]
  c="$FED_PL_STATE/goat-probe-impersonate-run-3/config.json5"
  grep -q '"tls/169.254.10.1:7448"' "$c"
  grep -q 'http_port: "192.0.2.10:18602"' "$c"
  if grep -q '0\.0\.0\.0' "$c"; then false; fi
  if grep -q '__' "$c"; then false; fi
}

@test "probe-launcher: FED_PL_FAR_PORT changes the far edge's port" {
  FED_PL_FAR_PORT=7449 run "$PL" start agreed run-4 "$(later)"
  [ "$status" -eq 0 ]
  grep -q '"tls/169.254.10.1:7449"' "$FED_PL_STATE/goat-probe-agreed-run-4/config.json5"
}

@test "probe-launcher: the image is the pins file's zenoh_router_image" {
  run "$PL" start agreed run-5 "$(later)"
  [ "$status" -eq 0 ]
  grep -q "^run -d --name goat-probe-agreed-run-5 .* ghcr.io/example-org/zenoh-pqc@$DIGEST -c /probe/config.json5$" "$STUB/docker.log"
}

@test "probe-launcher: a pins file without a digest-pinned zenoh_router_image is refused (X-PROBE-002)" {
  printf 'zenoh_router_image: "ghcr.io/example-org/zenoh-pqc:latest"\n' > "$STUB/pins.yml"
  run "$PL" start agreed run-6 "$(later)"
  [ "$status" -eq 2 ]
  [[ "$output" == *"X-PROBE-002"* ]]
  printf 'zenoh_router_image: "{{ zenoh_image }}"\n' > "$STUB/pins.yml"
  run "$PL" start agreed run-6 "$(later)"
  [ "$status" -eq 2 ]
  [[ "$output" == *"X-PROBE-002"* ]]
  [ ! -e "$STUB/docker.log" ]
}

@test "probe-launcher: stop ends the session, and alive says 0 while it runs and 1 once it is gone" {
  run "$PL" start agreed run-7 "$(later)"
  [ "$status" -eq 0 ]
  run "$PL" alive goat-probe-agreed-run-7
  [ "$status" -eq 0 ]
  run "$PL" stop goat-probe-agreed-run-7
  [ "$status" -eq 0 ]
  run "$PL" alive goat-probe-agreed-run-7
  [ "$status" -eq 1 ]
  [ ! -e "$FED_PL_STATE/goat-probe-agreed-run-7" ]
}

@test "probe-launcher: the session ends by itself at its deadline, with no stop" {
  run "$PL" start agreed run-8 "$(later 1)"
  [ "$status" -eq 0 ]
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    [ -e "$STUB/docker/goat-probe-agreed-run-8" ] || break
    sleep 1
  done
  run "$PL" alive goat-probe-agreed-run-8
  [ "$status" -eq 1 ]
  [ ! -e "$FED_PL_STATE/goat-probe-agreed-run-8" ]
}

@test "probe-launcher: an unreachable edge host is not gone: alive exits 2, never 1" {
  printf '#!/usr/bin/env bash\nexit 255\n' > "$STUB/bin/ssh"
  run "$PL" alive goat-probe-agreed-run-9
  [ "$status" -eq 2 ]
}

@test "probe-launcher: a probe that never answers is removed and the start fails, after naming its handle" {
  touch "$STUB/curl-fails"
  run "$PL" start agreed run-10 "$(later)"
  [ "$status" -eq 1 ]
  [[ "$output" == *"HANDLE=goat-probe-agreed-run-10"* ]]
  [[ "$output" != *"REST="* ]]
  [[ "$output" == *"X-PROBE-003"* ]]
  [ ! -e "$STUB/docker/goat-probe-agreed-run-10" ]
  [ ! -e "$FED_PL_STATE/goat-probe-agreed-run-10" ]
}

@test "probe-launcher: a missing setting is refused by name (X-PROBE-001), and nothing reaches the host" {
  unset FED_PL_PINS
  run "$PL" start agreed run-11 "$(later)"
  [ "$status" -eq 2 ]
  [[ "$output" == *"FED_PL_PINS is not set"*"X-PROBE-001"* ]]
  unset FED_PL_SSH
  run "$PL" start agreed run-11 "$(later)"
  [ "$status" -eq 2 ]
  [[ "$output" == *"FED_PL_SSH"*"FED_PL_LOCAL=1"*"X-PROBE-001"* ]]
  [ ! -e "$STUB/ssh.log" ]
}

@test "probe-launcher: REST on every address is refused" {
  FED_PL_REST_IP=0.0.0.0 run "$PL" start agreed run-12 "$(later)"
  [ "$status" -eq 2 ]
  [[ "$output" == *"X-PROBE-001"* ]]
}

@test "probe-launcher: a handle it did not give is refused, and nothing runs on the host" {
  run "$PL" stop 'goat-probe-agreed-x; rm -rf /'
  [ "$status" -eq 2 ]
  run "$PL" alive '../../etc'
  [ "$status" -eq 2 ]
  run "$PL" start agreed 'run-13' 'soon; reboot'
  [ "$status" -eq 2 ]
  run "$PL" start everything run-13 "$(later)"
  [ "$status" -eq 2 ]
  FED_PL_FAR_IP='169.254.10.1;id' run "$PL" start agreed run-13 "$(later)"
  [ "$status" -eq 2 ]
  [ ! -e "$STUB/ssh.log" ]
}

@test "probe-launcher: on the edge host itself (FED_PL_LOCAL=1) it needs no ssh" {
  unset FED_PL_SSH
  export FED_PL_LOCAL=1
  run "$PL" start agreed run-14 "$(later)"
  [ "$status" -eq 0 ]
  [ ! -e "$STUB/ssh.log" ]
  run "$PL" alive goat-probe-agreed-run-14
  [ "$status" -eq 0 ]
}

@test "probe-launcher: it carries no identity and mints nothing" {
  run grep -nE 'openssl|-CAkey|BEGIN (EC |RSA )?PRIVATE KEY' "$PL" "$REPO_ROOT/ops/federation/standup/probe.json5"
  [ "$status" -eq 1 ]
}

@test "probe-launcher: stop deletes the key on the edge host and says the deletion was verified" {
  run "$PL" start agreed run-15 "$(later)"
  [ "$status" -eq 0 ]
  [ -e "$FED_PL_STATE/goat-probe-agreed-run-15/key.pem" ]
  run "$PL" stop goat-probe-agreed-run-15
  [ "$status" -eq 0 ]
  [[ "$output" == *"KEY_DELETED=verified"* ]]
  [ ! -e "$FED_PL_STATE/goat-probe-agreed-run-15/key.pem" ]
}

@test "probe-launcher: a key that is still there after stop is reported, and stop fails" {
  run "$PL" start agreed run-16 "$(later)"
  [ "$status" -eq 0 ]
  mkdir -p "$STUB/norm"
  printf '#!/bin/sh\nexit 0\n' > "$STUB/norm/rm"; chmod +x "$STUB/norm/rm"
  PATH="$STUB/norm:$PATH" run "$PL" stop goat-probe-agreed-run-16
  [ "$status" -eq 1 ]
  [[ "$output" == *"KEY_DELETED=failed"* ]]
  [[ "$output" == *"X-PROBE-003"* ]]
  [ -e "$FED_PL_STATE/goat-probe-agreed-run-16/key.pem" ]
}

@test "probe-launcher: a start interrupted after the key is on the host removes it, on any exit" {
  # The probe never answers: the start keeps waiting, and is terminated mid-run.
  printf '#!/usr/bin/env bash\nsleep 1\nexit 7\n' > "$STUB/bin/curl"
  FED_PL_WAIT_STEP=1 "$PL" start agreed run-17 "$(later)" > "$STUB/start.out" 2> "$STUB/start.err" &
  pid=$!
  for _ in $(seq 1 50); do [ -e "$FED_PL_STATE/goat-probe-agreed-run-17/key.pem" ] && break; sleep 0.1; done
  [ -e "$FED_PL_STATE/goat-probe-agreed-run-17/key.pem" ]
  sleep 0.5
  kill -TERM "$pid"
  rc=0; wait "$pid" || rc=$?
  [ "$rc" -eq 143 ]
  [ ! -e "$FED_PL_STATE/goat-probe-agreed-run-17" ]
  [ ! -e "$STUB/docker/goat-probe-agreed-run-17" ]
  grep -q 'KEY_DELETED=verified' "$STUB/start.out"
  grep -q 'interrupted' "$STUB/start.err"
}

@test "probe-launcher: a start that fails after the key is on the host removes it" {
  cat > "$STUB/bin/docker" <<'STUBEOF'
#!/usr/bin/env bash
# docker run fails; every other call succeeds.
printf '%s\n' "$*" >> "$STUB/docker.log"
[ "$1" != run ] || exit 125
STUBEOF
  run "$PL" start agreed run-18 "$(later)"
  [ "$status" -ne 0 ]
  [ ! -e "$FED_PL_STATE/goat-probe-agreed-run-18" ]
  [[ "$output" == *"KEY_DELETED=verified"* ]]
}
