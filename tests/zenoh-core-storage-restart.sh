#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# zenoh-core-storage-restart.sh: after the core router restarts, does its own
# store still answer for a value stored before the restart, and does it store a
# fresh value that arrives through an edge router which was not restarted?
#
# Containers only: no peering, no host change. The core runs the router image the
# goat-zenoh role renders, with the RocksDB backend the role bind-mounts, the
# role's storage_manager entries for sovereign ingress (sovereign_from over
# from/** and sovereign_from_<sov> over from/<sov>/**,
# ops/ansible/roles/goat-zenoh/templates/config.json5.j2), the role's pinned
# router id and timestamping, and the storage directory bind-mounted from the
# host as the role mounts it (templates/docker-compose.yml.j2). The edge is a
# router as roles/goat-zenoh-edge renders it (no pinned id, no storage, scouting
# off, dialling the core), less TLS; its REST plugin stands in for the partner
# traffic it relays inward. A third router dials the edge, one hop further out,
# as the partner's edge does. ZENOH_CORE_ZID=random runs the core with no pinned id.
#
#   ZENOH_IMAGE=<router image>  ZENOH_ROCKSDB_SO=<path to libzenoh_backend_rocksdb.so> \
#     bash tests/zenoh-core-storage-restart.sh
#
# ZENOH_IMAGE defaults to the role's zenoh_router_image with ghcr_org set to
# ZENOH_GHCR_ORG. Exit 0 when every check passes; 1 when one does not (each line
# says which); 2 when it cannot run (no docker, no image, no backend).

set -u

die() { echo "zenoh-core-storage-restart: $*" >&2; exit 2; }

command -v docker >/dev/null 2>&1 || die "docker is not on PATH"
command -v curl >/dev/null 2>&1 || die "curl is not on PATH"
here=$(cd "$(dirname "$0")/.." && pwd)
defaults="$here/ops/ansible/roles/goat-zenoh/defaults/main.yml"
if [ -z "${ZENOH_IMAGE:-}" ]; then
  [ -n "${ZENOH_GHCR_ORG:-}" ] || die "set ZENOH_IMAGE, or ZENOH_GHCR_ORG to fill the role's zenoh_router_image"
  ZENOH_IMAGE=$(sed -n 's/^zenoh_router_image: "\(.*\)"$/\1/p' "$defaults" | sed "s/{{ ghcr_org }}/$ZENOH_GHCR_ORG/")
fi
[ -n "$ZENOH_IMAGE" ] || die "could not read zenoh_router_image from $defaults"
if [ -z "${ZENOH_ROCKSDB_SO:-}" ] || [ ! -f "$ZENOH_ROCKSDB_SO" ]; then
  die "set ZENOH_ROCKSDB_SO to the role's libzenoh_backend_rocksdb.so (goat_zenoh_rocksdb_backend_version in $defaults)"
fi
docker image inspect "$ZENOH_IMAGE" >/dev/null 2>&1 || die "image $ZENOH_IMAGE is not present locally"

work=$(mktemp -d "${TMPDIR:-/tmp}/zenoh-storage-restart.XXXXXX")
core="zenoh-storage-restart-core-$$"
edge="zenoh-storage-restart-edge-$$"
far="zenoh-storage-restart-far-$$"
farcore="zenoh-storage-restart-farcore-$$"
client="zenoh-storage-restart-client-$$"
net="zenoh-storage-restart-$$"
cleanup() {
  docker rm -f "$core" "$edge" "$far" "$farcore" "$client" >/dev/null 2>&1
  docker network rm "$net" >/dev/null 2>&1
  # The router writes the store as root: remove it from inside a container.
  docker run --rm -v "$work:/w" --entrypoint /bin/sh "$ZENOH_IMAGE" -c 'rm -rf /w/*' >/dev/null 2>&1
  rm -rf "$work"
}
trap cleanup EXIT

mkdir -p "$work/plugins" "$work/rocksdb"
cp "$ZENOH_ROCKSDB_SO" "$work/plugins/libzenoh_backend_rocksdb.so"
store=/var/lib/goat-zenoh/rocksdb

# The routers' access control, as the run renders it (ZENOH_ACL=off leaves it out).
# Subjects match pinned router ids where the deployment matches certificate names:
# there is no TLS here, so `cert_common_names` and `link_protocols: ["tls"]` have
# nothing to match. The core's rules for the partner are the goat-zenoh role's
# (sovereign-<sov>-*, fed-*-<sov>) plus trial-peer-r-all / trial-peer-deliver-all,
# which reach the edge in a deployment through `link_protocols: ["tls"]`. The edge's
# are roles/goat-zenoh-edge's partner-* and core-* rules with the key expressions
# goat-federation renders for partner xaa offering `all` (render.rs:632-655).
CORE_ZID=a1b2c3d4e5f60718293a4b5c6d7e8f90
EDGE_ZID=e1e2e3e4e5e6e7e8e9eaebecedeeef01
FAR_ZID=f1f2f3f4f5f6f7f8f9fafbfcfdfeff02
FARCORE_ZID=c1c2c3c4c5c6c7c8c9cacbcccdcecf03
CLIENT_ZID=d1d2d3d4d5d6d7d8d9dadbdcdddedf04
acl_rule() { # <id> <flows> <messages> <key_exprs>
  echo "      { id: \"$1\", flows: [$2], messages: [$3], key_exprs: [$4], permission: \"allow\" },"
}
core_acl() {
  [ "${ZENOH_ACL:-on}" = off ] && return
  echo '  access_control: { enabled: true, default_permission: "deny", rules: ['
  acl_rule sovereign-xaa-w '"ingress"' '"put", "delete"' '"from/xaa/**"'
  acl_rule sovereign-xaa-w-interest '"egress"' '"declare_subscriber", "query", "declare_queryable"' '"from/xaa/**"'
  acl_rule fed-w-xaa '"ingress"' '"put", "delete"' '"from/xaa/**"'
  acl_rule fed-w-interest-xaa '"egress"' '"declare_subscriber", "query", "declare_queryable"' '"from/xaa/**"'
  acl_rule fed-r-xaa '"egress"' '"put", "delete", "declare_subscriber", "query", "reply"' '"from/xab/all/**", "from/xab/xaa/**"'
  acl_rule fed-r-interest-xaa '"ingress"' '"declare_subscriber", "query", "declare_queryable"' '"from/xab/**"'
  acl_rule trial-peer-r-all '"egress", "ingress"' '"declare_subscriber", "query", "declare_queryable", "reply", "declare_liveliness_subscriber", "liveliness_query"' '"**", "**/@adv/**"'
  acl_rule trial-peer-deliver-all '"egress"' '"put", "delete", "liveliness_token"' '"**", "**/@adv/**"'
  echo '    ],'
  echo "    subjects: [ { id: \"edge\", zids: [\"$EDGE_ZID\"] } ],"
  echo '    policies: ['
  echo '      { subjects: ["edge"], rules: ["sovereign-xaa-w", "sovereign-xaa-w-interest", "fed-w-xaa", "fed-w-interest-xaa", "fed-r-xaa", "fed-r-interest-xaa", "trial-peer-r-all", "trial-peer-deliver-all"] },'
  echo '    ],'
  echo '  },'
}
farcore_acl() { # the partner's core (xaa): its edge, and the republisher as a client
  [ "${ZENOH_ACL:-on}" = off ] && return
  echo '  access_control: { enabled: true, default_permission: "deny", rules: ['
  acl_rule fed-w-xab '"ingress"' '"put", "delete"' '"from/xab/**"'
  acl_rule fed-w-interest-xab '"egress"' '"declare_subscriber", "query", "declare_queryable"' '"from/xab/**"'
  acl_rule fed-r-xab '"egress"' '"put", "delete", "declare_subscriber", "query", "reply"' '"from/xaa/all/**", "from/xaa/xab/**"'
  acl_rule fed-r-interest-xab '"ingress"' '"declare_subscriber", "query", "declare_queryable"' '"from/xaa/**"'
  acl_rule fed-rp-w-xab '"ingress"' '"put", "delete"' '"from/xaa/all/**", "from/xaa/xab/**"'
  acl_rule fed-rp-w-interest-xab '"egress"' '"declare_subscriber", "query", "declare_queryable"' '"from/xaa/**"'
  acl_rule trial-peer-r-all '"egress", "ingress"' '"declare_subscriber", "query", "declare_queryable", "reply", "declare_liveliness_subscriber", "liveliness_query"' '"**", "**/@adv/**"'
  acl_rule trial-peer-deliver-all '"egress"' '"put", "delete", "liveliness_token"' '"**", "**/@adv/**"'
  echo '    ],'
  echo "    subjects: [ { id: \"edge\", zids: [\"$FAR_ZID\"] }, { id: \"republisher\", zids: [\"$CLIENT_ZID\"] } ],"
  echo '    policies: ['
  echo '      { subjects: ["edge"], rules: ["fed-w-xab", "fed-w-interest-xab", "fed-r-xab", "fed-r-interest-xab", "trial-peer-r-all", "trial-peer-deliver-all"] },'
  echo '      { subjects: ["republisher"], rules: ["fed-rp-w-xab", "fed-rp-w-interest-xab", "trial-peer-r-all", "trial-peer-deliver-all"] },'
  echo '    ],'
  echo '  },'
}
edge_acl() { # <partner zid> <core zid or empty> <ingress> <egress> <egress interest>
  [ "${ZENOH_ACL:-on}" = off ] && return
  echo '  access_control: { enabled: true, default_permission: "deny", rules: ['
  acl_rule partner-land '"ingress"' '"put", "delete", "liveliness_token"' "$3"
  acl_rule partner-land-interest '"egress"' '"declare_subscriber", "query", "declare_queryable", "liveliness_query"' "$3"
  acl_rule partner-receive '"egress"' '"put", "delete", "declare_subscriber", "query", "reply"' "$4"
  acl_rule partner-receive-interest '"ingress"' '"declare_subscriber", "query", "declare_queryable", "liveliness_query"' "$5"
  acl_rule core-forward '"ingress", "egress"' '"put", "delete", "declare_subscriber", "query", "reply", "liveliness_token"' "$3"
  acl_rule core-return '"ingress", "egress"' '"put", "delete", "declare_subscriber", "query", "reply", "liveliness_token"' "$5"
  echo '    ],'
  if [ -n "$2" ]; then
    echo "    subjects: [ { id: \"partner-peer\", zids: [\"$1\"] }, { id: \"our-core\", zids: [\"$2\"] } ],"
    echo '    policies: [ { rules: ["partner-land", "partner-land-interest", "partner-receive", "partner-receive-interest"], subjects: ["partner-peer"] }, { rules: ["core-forward", "core-return"], subjects: ["our-core"] } ],'
  else
    echo "    subjects: [ { id: \"partner-peer\", zids: [\"$1\"] } ],"
    echo '    policies: [ { rules: ["partner-land", "partner-land-interest", "partner-receive", "partner-receive-interest"], subjects: ["partner-peer"] } ],'
  fi
  echo '  },'
}
storage() { # <name> <key_expr>: one storage entry exactly as the role renders it
  printf '        "%s": {\n          key_expr: "%s",\n          volume: { id: "rocksdb", dir: "%s/%s", on_closure: "do_nothing", create_db: true, db_recovery_enabled: false },\n          garbage_collect_period: 86400,\n          time_to_live: 2592000000,\n        },\n' "$1" "$2" "$store" "$1"
}
{
  echo '{'
  echo '  mode: "router",'
  # The role pins the core's id (goat_zenoh_router_zid); ZENOH_CORE_ZID=random leaves it unpinned.
  [ "${ZENOH_CORE_ZID:-pinned}" = random ] || echo "  id: \"$CORE_ZID\","
  core_acl
  echo '  timestamping: { enabled: true, drop_future_timestamp: false },'
  echo '  listen: { endpoints: ["tcp/0.0.0.0:7447"] },'
  echo '  plugins_loading: { enabled: true, search_dirs: ["/", "/opt/zenoh/plugins"] },'
  echo '  plugins: {'
  echo '    rest: { http_port: "0.0.0.0:8000" },'
  echo '    storage_manager: {'
  echo '      backend_search_dirs: ["/opt/zenoh/plugins"],'
  echo '      volumes: { rocksdb: {} },'
  echo '      storages: {'
  storage sovereign_from 'from/**'
  storage sovereign_from_xaa 'from/xaa/**'
  echo '      },'
  echo '    },'
  echo '  },'
  echo '}'
} > "$work/core.json5"
{
  echo '{'
  echo '  mode: "router",'
  echo "  id: \"$EDGE_ZID\","
  echo '  listen: { endpoints: ["tcp/0.0.0.0:7448"] },'
  echo "  connect: { endpoints: [\"tcp/$core:7447\"] },"
  # Our edge (xab) for partner xaa: they land on from/xaa/**, receive the offered set.
  edge_acl "$FAR_ZID" "$CORE_ZID" '"from/xaa/**"' '"from/xab/all/**", "from/xab/xaa/**"' '"from/xab/**"'
  echo '  scouting: { gossip: { enabled: false }, multicast: { enabled: false } },'
  echo '  plugins_loading: { enabled: true, search_dirs: ["/", "/opt/zenoh/plugins"] },'
  echo '  plugins: { rest: { http_port: "0.0.0.0:8000" } },'
  echo '}'
} > "$work/edge.json5"
# The partner side, one more router hop out: it dials our edge, as the partner's
# edge holds the peering session. A value put here crosses two routers to the core.
{
  echo '{'
  echo '  mode: "router",'
  echo "  id: \"$FAR_ZID\","
  echo "  connect: { endpoints: [\"tcp/$edge:7448\", \"tcp/$farcore:7447\"] },"
  # The partner's edge (xaa) for us (xab): the mirror image.
  edge_acl "$EDGE_ZID" "$FARCORE_ZID" '"from/xab/**"' '"from/xaa/all/**", "from/xaa/xab/**"' '"from/xaa/**"'
  echo '  scouting: { gossip: { enabled: false }, multicast: { enabled: false } },'
  echo '  plugins_loading: { enabled: true, search_dirs: ["/", "/opt/zenoh/plugins"] },'
  echo '  plugins: { rest: { http_port: "0.0.0.0:8000" } },'
  echo '}'
} > "$work/far.json5"
# The partner's core, and the outbound republisher's publishing session: a client
# of that core (ops/federation/republisher/src/main.rs:399) whose session.put
# (src/edge.rs:84-99) carries the partner's release onto the wire.
{
  echo '{'
  echo '  mode: "router",'
  echo "  id: \"$FARCORE_ZID\","
  echo '  listen: { endpoints: ["tcp/0.0.0.0:7447"] },'
  farcore_acl
  echo '  plugins_loading: { enabled: true, search_dirs: ["/", "/opt/zenoh/plugins"] },'
  echo '  plugins: { rest: { http_port: "0.0.0.0:8000" } },'
  echo '}'
} > "$work/farcore.json5"
{
  echo '{'
  echo '  mode: "client",'
  echo "  id: \"$CLIENT_ZID\","
  echo "  connect: { endpoints: [\"tcp/$farcore:7447\"] },"
  echo '  plugins_loading: { enabled: true, search_dirs: ["/", "/opt/zenoh/plugins"] },'
  echo '  plugins: { rest: { http_port: "0.0.0.0:8000" } },'
  echo '}'
} > "$work/client.json5"

docker network create "$net" >/dev/null || die "docker network create failed"
docker run -d --name "$core" --network "$net" -p 127.0.0.1::8000 -e RUST_LOG=info \
  -v "$work/core.json5:/etc/zenoh/config.json5:ro" \
  -v "$work/plugins:/opt/zenoh/plugins:ro" \
  -v "$work/rocksdb:$store" \
  "$ZENOH_IMAGE" --config /etc/zenoh/config.json5 >/dev/null || die "the core router did not start"
docker run -d --name "$edge" --network "$net" -p 127.0.0.1::8000 -e RUST_LOG=info \
  -v "$work/edge.json5:/etc/zenoh/config.json5:ro" \
  "$ZENOH_IMAGE" --config /etc/zenoh/config.json5 >/dev/null || die "the edge router did not start"
docker run -d --name "$farcore" --network "$net" -p 127.0.0.1::8000 -e RUST_LOG=info \
  -v "$work/farcore.json5:/etc/zenoh/config.json5:ro" \
  "$ZENOH_IMAGE" --config /etc/zenoh/config.json5 >/dev/null || die "the partner core did not start"
sleep 2
docker run -d --name "$client" --network "$net" -p 127.0.0.1::8000 -e RUST_LOG=info \
  -v "$work/client.json5:/etc/zenoh/config.json5:ro" \
  "$ZENOH_IMAGE" --config /etc/zenoh/config.json5 >/dev/null || die "the client did not start"
docker run -d --name "$far" --network "$net" -p 127.0.0.1::8000 -e RUST_LOG=info \
  -v "$work/far.json5:/etc/zenoh/config.json5:ro" \
  "$ZENOH_IMAGE" --config /etc/zenoh/config.json5 >/dev/null || die "the far router did not start"

url() { echo "http://127.0.0.1:$(docker port "$1" 8000/tcp | head -n 1 | sed 's/.*://')"; }
crest=""
erest=""
frest=""
clrest=""
ready() { # both storages listed in the core's admin space, and the edge answering
  crest=$(url "$core"); erest=$(url "$edge"); frest=$(url "$far"); clrest=$(url "$client")
  i=0
  while [ $i -lt 60 ]; do
    n=$(curl -fs "$crest/@/local/router/status/plugins/storage_manager/storages/**" 2>/dev/null | grep -o 'storages/sovereign_from[a-z_]*' | sort -u | wc -l)
    if [ "$n" -ge 2 ] && curl -fs -o /dev/null "$erest/@/local/router" 2>/dev/null && curl -fs -o /dev/null "$frest/@/local/router" 2>/dev/null && curl -fs -o /dev/null "$clrest/@/local/**" 2>/dev/null; then return 0; fi
    sleep 1; i=$((i + 1))
  done
  docker logs "$core" 2>&1 | tail -n 20 >&2
  die "the routers did not come up within 60 s"
}

put() { # <base url> <key> <value>
  curl -fs -X PUT -H 'content-type: text/plain' --data "$3" "$1/$2" >/dev/null || die "PUT $2 failed"
}
holds() { # <key> <value>: a GET on the core returns the value (plain or base64)
  curl -fs "$crest/$1" 2>/dev/null | grep -q "\"$2\"\|$(printf '%s' "$2" | base64)"
}

fails=0
pass() { echo "  PASS  $1"; }
fail() { echo "  FAIL  $1"; fails=$((fails + 1)); }
read_back() { # <key> <value> <label>
  if holds "$1" "$2"; then pass "$3: GET $1 at the core returns $2"; else fail "$3: GET $1 at the core returned $(curl -fs "$crest/$1" 2>/dev/null || echo nothing), expected $2"; fi
}
arrives() { # <base url> <where> <key> <value> <label>: PUT there, stored at the core within 20 s
  put "$1" "$3" "$4"
  i=0
  while [ $i -lt 20 ]; do
    holds "$3" "$4" && { pass "$5: PUT $3 at the $2 is stored at the core"; return; }
    sleep 1; i=$((i + 1))
  done
  fail "$5: PUT $3 at the $2 is not stored at the core within 20 s"
}
via_edge() { arrives "$erest" edge "$@"; }
via_far() { arrives "$frest" "far router" "$@"; }
via_client() { arrives "$clrest" "partner core's client (the republisher's place)" "$@"; }

ready
echo "before the core restarts"
put "$crest" from/xaa/core-before c-before
sleep 1
read_back from/xaa/core-before c-before "put at the core"
via_edge from/xaa/edge-before e-before "put through the edge"
via_far from/xaa/all/far-before f-before "put two hops out"
via_client from/xaa/all/client-before r-before "put by a client of the partner core"

docker restart "$core" >/dev/null || die "docker restart failed"
ready
echo "after the core restarts (everything else keeps running)"
read_back from/xaa/core-before c-before "put at the core before the restart"
read_back from/xaa/edge-before e-before "put through the edge before the restart"
read_back 'from/xaa/**' c-before "a selector over from/xaa/**"
put "$crest" from/xaa/core-after c-after
sleep 1
read_back from/xaa/core-after c-after "put at the core after the restart"
via_edge from/xaa/edge-after e-after "put through the edge after the restart"
via_far from/xaa/all/far-after f-after "put two hops out after the restart"
via_client from/xaa/all/client-after r-after "put by a client of the partner core after the restart"

warns=$(docker logs "$core" 2>&1 | grep -c 'no data_info in database')
echo "core log: $warns line(s) 'no data_info in database, however payload data exists'"
if [ "$fails" -gt 0 ]; then
  echo "core log, sessions and storages:"
  docker logs "$core" 2>&1 | grep -i 'rocksdb\|data_info\|storage\|session\|transport' | tail -n 15
  echo "edge log, last lines:"
  docker logs "$edge" 2>&1 | tail -n 15
  echo "far router log, last lines:"
  docker logs "$far" 2>&1 | tail -n 10
  echo "zenoh-core-storage-restart: FAIL ($fails of 11)"
  exit 1
fi
echo "zenoh-core-storage-restart: PASS"
