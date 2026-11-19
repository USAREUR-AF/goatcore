#!/usr/bin/env bash
# GHSA-x44g: the Docker API proxy passes on exactly what Vector's docker_logs
# source needs, refuses everything else, and is reachable by nobody but the
# collector. Brings up the RENDERED compose's proxy service (docker compose,
# the role's pinned nginx image) against this machine's Docker daemon, and
# probes its unix socket as the collector would (its gid, the socket dir
# mounted read-only), then as a host user and as another container, which
# must both fail. Its own test container is the only thing a refused write
# could reach (it would be killed); nothing else is touched.
#
# Opt-in: needs docker and the pinned nginx image already present locally.
# Every run and compose call uses --pull never. Prints SKIP and exits 0
# otherwise, or when a goat-logship-docker-proxy container already exists.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
image="$(sed -n 's/^logship_docker_proxy_image: "\(.*\)"$/\1/p' "$SCRIPT_DIR/../defaults/main.yml")"
sock=/var/run/docker.sock
if ! docker info >/dev/null 2>&1 || ! docker image inspect "$image" >/dev/null 2>&1; then
  echo "[goat-logship/docker-proxy] SKIP (no docker, or $image not present locally)"; exit 0
fi
if docker inspect goat-logship-docker-proxy >/dev/null 2>&1; then
  echo "[goat-logship/docker-proxy] SKIP (a goat-logship-docker-proxy container already exists here)"; exit 0
fi
# Stand-ins for the host users tasks/access.yml creates.
P_UID=64101 C_UID=64199 C_GID=64102 O_ID=64200
tag="x44g-proxytest-$$"
project="x44gproxytest$$"
work="$(mktemp -d)"
compose_dir="$work/compose"; sockdir="$compose_dir/docker-proxy-socket"
run() { docker run --rm --pull never "$@"; }
cleanup() {
  docker compose -p "$project" -f "$compose_dir/docker-compose.yml" down -t 1 >/dev/null 2>&1 || true
  docker rm -f "$tag-src" >/dev/null 2>&1 || true
  # The socket dir belongs to the stand-in proxy uid; remove it as root, in a container.
  run -v "$work:/w" --entrypoint rm "$image" -rf /w/compose >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT
fails=0
fail() { echo "  FAIL $1"; fails=$((fails + 1)); }

mkdir -p "$compose_dir"
ansible-playbook -i localhost, --connection local "$SCRIPT_DIR/render.yml" -e out_dir="$compose_dir" \
  -e logship_compose_dir="$compose_dir" -e _logship_dproxy_uid=$P_UID -e _logship_gid=$C_GID \
  -e _logship_docker_gid="$(stat -c %g "$sock")" >/dev/null 2>&1
chmod 0644 "$compose_dir/docker-proxy.nginx.conf"
# The socket dir as tasks/datadir.yml makes it: proxy uid, collector gid, 0710.
mkdir -p "$sockdir"
run -v "$sockdir:/s" --entrypoint sh "$image" -c "chown $P_UID:$C_GID /s && chmod 0710 /s"

# A container whose stdout the proxy must serve (the pinned image; no pull).
run -d --name "$tag-src" --network none --entrypoint sh "$image" \
  -c 'echo x44g-log-line; while sleep 1; do :; done' >/dev/null
# The proxy service exactly as rendered.
docker compose -p "$project" -f "$compose_dir/docker-compose.yml" up -d --pull never --no-deps \
  goat-logship-docker-proxy >/dev/null 2>&1 \
  || { fail "compose could not start the rendered proxy service"; docker compose -p "$project" -f "$compose_dir/docker-compose.yml" logs 2>&1 | tail -5; exit 1; }

# vcurl <curl args...>: curl the proxy's socket as the collector would.
vcurl() {
  run --network none --user "$C_UID:$C_GID" -v "$sockdir:/run/goat-logship-docker-proxy:ro" \
    --entrypoint curl "$image" --unix-socket /run/goat-logship-docker-proxy/api.sock "$@"
}
# probe <method> <path> -> HTTP status, as the collector.
probe() {
  vcurl -s -o /dev/null -w '%{http_code}' --path-as-is -X "$1" --max-time 5 "http://docker$2" || true
}
for _ in $(seq 1 20); do [ "$(probe GET /_ping)" = 200 ] && break; sleep 0.5; done
uid="$(docker exec goat-logship-docker-proxy sh -c 'id -u' 2>/dev/null || echo '?')"
[ "$uid" = $P_UID ] && echo "  ok   proxy runs as its own uid ($P_UID), not root" || fail "proxy uid is $uid"

# Nobody but the collector reaches it.
mode="$(run -v "$sockdir:/s:ro" --entrypoint stat "$image" -c '%a %u:%g' /s/api.sock 2>/dev/null || echo '?')"
[ "$mode" = "660 $P_UID:$C_GID" ] && echo "  ok   socket is 0660, proxy uid : collector gid" || fail "socket mode/owner: $mode"
[ "$(stat -c '%a' "$sockdir")" = 710 ] && echo "  ok   socket dir is 0710" || fail "socket dir mode $(stat -c %a "$sockdir")"
nets="$(docker inspect -f '{{.HostConfig.NetworkMode}} {{range $k, $v := .NetworkSettings.Networks}}{{$k}};{{end}} {{len .NetworkSettings.Ports}}' goat-logship-docker-proxy)"
[ "$nets" = "none none; 0" ] && echo "  ok   proxy has no network address and no ports (no TCP path)" || fail "proxy networking: $nets"
if [ "$(id -u)" != 0 ] && ! id -G | tr ' ' '\n' | grep -qx "$C_GID"; then
  if curl -s --max-time 3 --unix-socket "$sockdir/api.sock" http://docker/_ping >/dev/null 2>&1; then fail "host user $(id -un) reached the proxy"
  else echo "  ok   a host user outside the collector's group cannot connect"; fi
fi
if run --network none --user "$O_ID:$O_ID" -v "$sockdir:/run/goat-logship-docker-proxy:ro" --entrypoint curl "$image" \
     -s --max-time 3 --unix-socket /run/goat-logship-docker-proxy/api.sock http://docker/_ping >/dev/null 2>&1; then
  fail "another container (uid/gid $O_ID) with the dir mounted reached the proxy"
else echo "  ok   another container, even with the socket dir mounted, cannot connect"; fi
if run --network none --user "$C_UID:$O_ID" -v "$sockdir:/run/goat-logship-docker-proxy:ro" --entrypoint curl "$image" \
     -s --max-time 3 --unix-socket /run/goat-logship-docker-proxy/api.sock http://docker/_ping >/dev/null 2>&1; then
  fail "a uid without the collector's group reached the proxy"
else echo "  ok   without the collector's group, no connection"; fi

expect() {  # expect <status> <method> <path> <what>
  got="$(probe "$2" "$3")"
  [ "$got" = "$1" ] && echo "  ok   $1 $2 $3  ($4)" || fail "$2 $3: got $got, want $1 ($4)"
}
expect 200 GET  /_ping                                   "ping"
expect 200 GET  /version                                 "version"
expect 200 GET  "/v1.41/containers/json?all=1"           "list, versioned"
expect 200 GET  "/containers/json"                       "list"
expect 200 GET  "/containers/$tag-src/json"              "inspect"
expect 200 HEAD /_ping                                   "HEAD is a read"
expect 200 GET  "/v1.41/containers/$tag-src/logs?stdout=1&stderr=1&tail=5" "logs"
expect 403 POST "/containers/$tag-src/kill"              "kill refused"
expect 403 POST "/containers/create"                     "create refused"
expect 403 POST "/containers/$tag-src/exec"              "exec refused"
expect 403 DELETE "/containers/$tag-src"                 "remove refused"
expect 403 PUT  "/containers/$tag-src/archive?path=/"    "archive write refused"
expect 403 GET  "/containers/$tag-src/archive?path=/"    "filesystem read refused"
expect 403 GET  "/containers/$tag-src/export"            "export refused"
expect 403 GET  /images/json                             "images refused"
expect 403 GET  /info                                    "info refused"
expect 403 GET  /secrets                                 "secrets refused"
expect 403 GET  "/containers/$tag-src/json/../../../images/json" "dot segments resolved, then refused"
expect 403 GET  "/containers/..%2Fimages/json"           "encoded slash refused"
expect 403 GET  "/containers/$tag-src%2Fexport/json"     "encoded slash in a name refused"
expect 403 GET  "//containers/json"                      "raw path must match too"

# A restart replaces the stale socket and makes the new one 0660 again.
docker restart -t 2 goat-logship-docker-proxy >/dev/null
for _ in $(seq 1 20); do [ "$(probe GET /_ping)" = 200 ] && break; sleep 0.5; done
mode="$(run -v "$sockdir:/s:ro" --entrypoint stat "$image" -c '%a' /s/api.sock 2>/dev/null || echo '?')"
[ "$(probe GET /_ping)" = 200 ] && [ "$mode" = 660 ] && echo "  ok   after a restart the socket answers and is 0660 again" \
  || fail "after restart: ping $(probe GET /_ping), mode $mode"

# The logs actually stream through, and the refused kill did nothing.
body="$(vcurl -s --max-time 5 "http://docker/containers/$tag-src/logs?stdout=1&tail=5" | tr -d '\000-\010' || true)"
printf '%s' "$body" | grep -q x44g-log-line && echo "  ok   a container's stdout is served through the proxy" \
  || fail "log line not served (got: $(printf '%s' "$body" | head -c 80))"
[ "$(docker inspect -f '{{.State.Running}}' "$tag-src")" = true ] && echo "  ok   the test container is still running (no write got through)" \
  || fail "test container is not running"
# A follow stream stays open (the long-lived path docker_logs uses).
# curl's 28 is "timed out": the stream was open, and still open at 3 s.
rc=0; vcurl -sN --max-time 3 -o /dev/null http://docker/events >/dev/null 2>&1 || rc=$?
[ "$rc" = 28 ] && echo "  ok   /events streams (held open until the client gave up)" \
  || fail "/events: curl exit $rc, want 28 (open stream)"

[ "$fails" -eq 0 ] || { echo "goat-logship: docker-proxy FAILED ($fails)"; exit 1; }
echo "goat-logship: docker-proxy OK"
