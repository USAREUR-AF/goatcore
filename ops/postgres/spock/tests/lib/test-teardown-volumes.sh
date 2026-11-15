#!/usr/bin/env bash
# Tests that the Spock harnesses remove their containers' anonymous volumes.
#
# A container gets an anonymous volume for every VOLUME its image declares (the
# Kratos image the partition harness runs declares two) and for every bare
# `-v <path>`. `docker rm -f` leaves those volumes behind; `docker rm -fv`
# removes them with the container (named volumes are kept either way). A
# harness that tears down with `-f` alone leaves them behind on every run.
#
# Part 1 needs no Docker: every `docker rm -f` under ops/postgres/spock/tests
# must also pass -v.
# Part 2 needs Linux Docker and IMAGE (any image with `sleep`; CI passes the
# postgres-spock image it builds): a container started as a site, with an
# anonymous volume, is removed by ts_down (two-site.sh), and the volume must be
# gone too.
# Without IMAGE or Docker, part 2 is reported as skipped.
#
# Usage: ops/postgres/spock/tests/lib/test-teardown-volumes.sh
#        IMAGE=postgres-spock:smoke ops/postgres/spock/tests/lib/test-teardown-volumes.sh

set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
TESTS="$(cd "$HERE/.." && pwd)"

fails=0
ok()   { echo "  ok    $1"; }
fail() { echo "  FAIL  $1"; fails=$((fails + 1)); }

echo "== part 1: every container removal also removes anonymous volumes"
found="$(grep -rnE 'docker( container)? rm -f( |$)' "$TESTS" --include='*.sh' | grep -v "^$HERE/test-teardown-volumes.sh:" || true)"
if [[ -z "$found" ]]; then
  ok "no 'docker rm -f' without -v under ops/postgres/spock/tests"
else
  fail "'docker rm -f' without -v leaves anonymous volumes behind:"
  while IFS= read -r line; do echo "        ${line#"$TESTS"/}"; done <<<"$found"
fi

echo "== part 2: real Docker, ts_down removes the site's anonymous volume"
if [[ -z "${IMAGE:-}" ]] || ! docker info >/dev/null 2>&1 || [[ "$(uname -s)" != Linux ]]; then
  echo "  skip  part 2 needs Linux Docker and IMAGE"
else
  PFX="teardown-volumes-test-$$"; NET="$PFX-net"
  # shellcheck source=two-site.sh
  source "$HERE/two-site.sh"
  docker network create "$NET" >/dev/null
  docker run -d --name "$PFX-a" --network "$NET" -v /teardown-check --entrypoint sleep "$IMAGE" 300 >/dev/null
  vols="$(docker inspect -f '{{range .Mounts}}{{if eq .Type "volume"}}{{.Name}} {{end}}{{end}}' "$PFX-a")"
  if [[ -z "${vols// /}" ]]; then
    fail "the test container has no anonymous volume to check"
    docker rm -fv "$PFX-a" >/dev/null 2>&1 || true
    docker network rm "$NET" >/dev/null 2>&1 || true
  else
    # shellcheck disable=SC2119  # ts_down with no extra containers
    ts_down
    left=""
    for v in $vols; do docker volume inspect "$v" >/dev/null 2>&1 && left="$left $v"; done
    if [[ -z "$left" ]]; then
      ok "ts_down removed the container and its anonymous volume"
    else
      fail "ts_down left anonymous volume(s):$left"
      for v in $left; do docker volume rm "$v" >/dev/null 2>&1 || true; done
    fi
  fi
fi

if (( fails > 0 )); then echo "== $fails failure(s)"; exit 1; fi
echo "== ok"
