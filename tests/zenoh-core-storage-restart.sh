#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# zenoh-core-storage-restart.sh: after the receiving core router restarts, with
# everything in front of it left running, does its own store still answer for a
# value stored before the restart, and does it store a fresh value that arrives
# from the partner's side? (Gate row B4, persistent reception.)
#
# Containers only: no peering, no host change. Five processes, all on the router
# image the goat-zenoh role renders:
#
#   client --> partner core --> partner edge --> our edge --> our core (restarted)
#   (xaa)       (core-xaa)       (xaa's edge)     (xab's)      (core-xab, stores)
#
# Our core has the RocksDB backend the role bind-mounts, the role's sovereign
# ingress stores verbatim (sovereign_from over from/**, sovereign_from_xaa over
# from/xaa/**; ops/ansible/roles/goat-zenoh/templates/config.json5.j2), the role's
# pinned router id and timestamping, and the store directory bind-mounted as the
# role mounts it (templates/docker-compose.yml.j2). The edges are routers as
# roles/goat-zenoh-edge renders them: no storage, scouting off, the partner-* and
# core-* access rules with the key expressions goat-federation renders for xaa
# and xab offering `all` (crates/goat-federation/src/render.rs:632-655). The
# client stands where the outbound republisher's publishing session stands: a
# client of the partner's core (ops/federation/republisher/src/main.rs:399).
#
# Modes (environment):
#   ZENOH_TLS=on   every link on mutual TLS, with two CAs minted for the run (ours
#                  issues core-xab and our edge's inward fed.xaa.xab.01 and outward
#                  fed.xab.xaa.01; the partner's issues core-xaa, its edge's inward
#                  fed.xab.xaa.01 and outward fed.xaa.xab.01, and the republisher's
#                  fed.xaa.xab.republisher.01), the edges in `fragment` trust (the
#                  partner's CA global, our CA on the core endpoint through
#                  #root_ca_certificate_file=), close_link_on_expiration on, and
#                  every access-control subject matched by certificate name. Over
#                  tcp the subjects match pinned router ids instead.
#   ZENOH_ACL=off  no access control anywhere (the refusal check is skipped).
#   ZENOH_CORE_ZID=random  our core without a pinned id (needs ZENOH_ACL=off over
#                  tcp, where subjects match ids).
#   ZENOH_CORE_CONFIG=<file>  run our core on that configuration instead of the
#                  one above (ZENOH_TLS=on only): the effective configuration a
#                  deployed xab core logs at start ("Initial conf: {...}"), with
#                  this run's core-xab certificate and our CA mounted at the paths
#                  it names and the store directory at its stores' parent. Its
#                  id must be the core's own; its ACL must name our edge's inward
#                  CN fed.xaa.xab.01. To take it from a router's log:
#                    docker logs <core> 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -m1 'Initial conf:' \
#                      | sed 's/.*Initial conf: //' > core.json
#   ZENOH_REPUBLISHER_IMAGE=<image> ZENOH_AGREEMENT=<agreement.json between xaa
#                  and xab>  also run the real outbound republisher on the
#                  partner's core (ZENOH_TLS=on only): a release put there crosses
#                  as from/xaa/all/<key>, stamped with the partner core's
#                  timestamp (ops/federation/republisher/src/edge.rs:84-99).
#
#   ZENOH_IMAGE=<router image>  ZENOH_ROCKSDB_SO=<libzenoh_backend_rocksdb.so> \
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

proto=tcp
[ "${ZENOH_TLS:-off}" = on ] && proto=tls
acl=on
[ "${ZENOH_ACL:-on}" = off ] && acl=off
rp_image="${ZENOH_REPUBLISHER_IMAGE:-}"
core_config="${ZENOH_CORE_CONFIG:-}"
if [ -n "$core_config" ]; then
  [ "$proto" = tls ] || die "ZENOH_CORE_CONFIG needs ZENOH_TLS=on"
  [ -f "$core_config" ] || die "ZENOH_CORE_CONFIG: $core_config is not a file"
  command -v python3 >/dev/null 2>&1 || die "ZENOH_CORE_CONFIG needs python3 to read the paths it names"
fi
if [ -n "$rp_image" ]; then
  [ "$proto" = tls ] || die "ZENOH_REPUBLISHER_IMAGE needs ZENOH_TLS=on"
  [ -n "${ZENOH_AGREEMENT:-}" ] && [ -f "$ZENOH_AGREEMENT" ] || die "ZENOH_REPUBLISHER_IMAGE needs ZENOH_AGREEMENT=<agreement.json between xaa and xab>"
  docker image inspect "$rp_image" >/dev/null 2>&1 || die "image $rp_image is not present locally"
fi

work=$(mktemp -d "${TMPDIR:-/tmp}/zenoh-storage-restart.XXXXXX")
tag="zenoh-storage-restart-$$"
core="$tag-core"; edge="$tag-edge"; far="$tag-far"; farcore="$tag-farcore"; client="$tag-client"; rp="$tag-republisher"
net="$tag"
cleanup() {
  docker rm -f "$core" "$edge" "$far" "$farcore" "$client" "$rp" >/dev/null 2>&1
  docker network rm "$net" >/dev/null 2>&1
  # The router writes the store as root: remove it from inside a container.
  docker run --rm -v "$work:/w" --entrypoint /bin/sh "$ZENOH_IMAGE" -c 'rm -rf /w/*' >/dev/null 2>&1
  rm -rf "$work"
}
trap cleanup EXIT
mkdir -p "$work/plugins" "$work/rocksdb" "$work/tls"
cp "$ZENOH_ROCKSDB_SO" "$work/plugins/libzenoh_backend_rocksdb.so"
store=/var/lib/goat-zenoh/rocksdb

CORE_ZID=a1b2c3d4e5f60718293a4b5c6d7e8f90
EDGE_ZID=e1e2e3e4e5e6e7e8e9eaebecedeeef01
FAR_ZID=f1f2f3f4f5f6f7f8f9fafbfcfdfeff02
FARCORE_ZID=c1c2c3c4c5c6c7c8c9cacbcccdcecf03
CLIENT_ZID=d1d2d3d4d5d6d7d8d9dadbdcdddedf04

# ---- certificates (ZENOH_TLS=on) ---------------------------------------------
mkca() { # <label>
  openssl req -x509 -newkey rsa:2048 -nodes -subj "/CN=zenoh-storage-restart $1 CA" -days 2 \
    -addext basicConstraints=critical,CA:TRUE -addext keyUsage=critical,keyCertSign,cRLSign \
    -keyout "$work/tls/$1-key.pem" -out "$work/tls/$1.pem" >/dev/null 2>&1 || die "openssl could not make the $1 CA"
}
mint() { # <label> <common name> <ca label>
  openssl req -newkey rsa:2048 -nodes -subj "/CN=$2" -keyout "$work/tls/$1-key.pem" -out "$work/tls/$1.csr" >/dev/null 2>&1 &&
    printf 'subjectAltName=DNS:%s\nextendedKeyUsage=serverAuth,clientAuth\nbasicConstraints=CA:FALSE\n' "$2" > "$work/tls/$1.ext" &&
    openssl x509 -req -in "$work/tls/$1.csr" -CA "$work/tls/$3.pem" -CAkey "$work/tls/$3-key.pem" -CAcreateserial \
      -days 2 -extfile "$work/tls/$1.ext" -out "$work/tls/$1.pem" >/dev/null 2>&1 || die "openssl could not mint $2"
  # The containers read the keys as their own users; the keys are this run's alone.
  chmod 0644 "$work/tls/$1-key.pem"
}
if [ "$proto" = tls ]; then
  command -v openssl >/dev/null 2>&1 || die "ZENOH_TLS=on needs openssl"
  mkca ca-xab; mkca ca-xaa
  mint core-xab core-xab ca-xab
  mint xab-inward fed.xaa.xab.01 ca-xab          # our edge, presented to our core
  mint xab-outward fed.xab.xaa.01 ca-xab         # our edge, presented to the partner
  mint core-xaa core-xaa ca-xaa
  mint xaa-inward fed.xab.xaa.01 ca-xaa          # their edge, presented to their core
  mint xaa-outward fed.xaa.xab.01 ca-xaa         # their edge, presented to us
  mint republisher fed.xaa.xab.republisher.01 ca-xaa
  chmod 0755 "$work/tls"
fi

# ---- config pieces -----------------------------------------------------------
tls_conf() { # <root ca label> <listen label or ""> <connect label or ""> [edge]: transport block, empty over tcp
  [ "$proto" = tls ] || return 0
  echo '  transport: { link: { tls: {'
  echo "    root_ca_certificate: \"/tls/$1.pem\", enable_mtls: true, verify_name_on_connect: false,"
  [ -n "$2" ] && echo "    listen_certificate: \"/tls/$2.pem\", listen_private_key: \"/tls/$2-key.pem\","
  [ -n "$3" ] && echo "    connect_certificate: \"/tls/$3.pem\", connect_private_key: \"/tls/$3-key.pem\","
  [ "${4:-}" = edge ] && echo '    close_link_on_expiration: true,'
  echo '  } } },'
}
subj() { # <id> <zid> <common name>: an ACL subject by name over TLS, by router id over tcp
  if [ "$proto" = tls ]; then echo "{ id: \"$1\", cert_common_names: [\"$3\"] }"; else echo "{ id: \"$1\", zids: [\"$2\"] }"; fi
}
acl_rule() { # <id> <flows> <messages> <key_exprs>
  echo "      { id: \"$1\", flows: [$2], messages: [$3], key_exprs: [$4], permission: \"allow\" },"
}
core_acl() { # our core (xab): the role's rules for partner xaa and its edge
  [ "$acl" = on ] || return 0
  echo '  access_control: { enabled: true, default_permission: "deny", rules: ['
  acl_rule sovereign-xaa-w '"ingress"' '"put", "delete"' '"from/xaa/**"'
  acl_rule sovereign-xaa-w-interest '"egress"' '"declare_subscriber", "query", "declare_queryable"' '"from/xaa/**"'
  acl_rule fed-w-xaa '"ingress"' '"put", "delete"' '"from/xaa/**"'
  acl_rule fed-w-interest-xaa '"egress"' '"declare_subscriber", "query", "declare_queryable"' '"from/xaa/**"'
  acl_rule fed-r-xaa '"egress"' '"put", "delete", "declare_subscriber", "query", "reply"' '"from/xab/all/**", "from/xab/xaa/**"'
  acl_rule fed-r-interest-xaa '"ingress"' '"declare_subscriber", "query", "declare_queryable"' '"from/xab/**"'
  # trial-peer-read: the role grants these to every TLS session (link_protocols: ["tls"]).
  acl_rule trial-peer-r-all '"egress", "ingress"' '"declare_subscriber", "query", "declare_queryable", "reply", "declare_liveliness_subscriber", "liveliness_query"' '"**", "**/@adv/**"'
  acl_rule trial-peer-deliver-all '"egress"' '"put", "delete", "liveliness_token"' '"**", "**/@adv/**"'
  echo '    ],'
  echo "    subjects: [ $(subj edge "$EDGE_ZID" fed.xaa.xab.01) ],"
  echo '    policies: [ { subjects: ["edge"], rules: ["sovereign-xaa-w", "sovereign-xaa-w-interest", "fed-w-xaa", "fed-w-interest-xaa", "fed-r-xaa", "fed-r-interest-xaa", "trial-peer-r-all", "trial-peer-deliver-all"] } ],'
  echo '  },'
}
farcore_acl() { # the partner's core (xaa): its edge, and the republisher's sessions
  [ "$acl" = on ] || return 0
  echo '  access_control: { enabled: true, default_permission: "deny", rules: ['
  acl_rule fed-w-xab '"ingress"' '"put", "delete"' '"from/xab/**"'
  acl_rule fed-w-interest-xab '"egress"' '"declare_subscriber", "query", "declare_queryable"' '"from/xab/**"'
  acl_rule fed-r-xab '"egress"' '"put", "delete", "declare_subscriber", "query", "reply"' '"from/xaa/all/**", "from/xaa/xab/**"'
  acl_rule fed-r-interest-xab '"ingress"' '"declare_subscriber", "query", "declare_queryable"' '"from/xaa/**"'
  acl_rule fed-rp-r-xab '"egress"' '"put", "delete", "declare_subscriber", "query", "reply"' '"release/all/**", "release/xab/**"'
  acl_rule fed-rp-r-interest-xab '"ingress"' '"declare_subscriber", "query", "declare_queryable"' '"release/all/**", "release/xab/**"'
  acl_rule fed-rp-w-xab '"ingress"' '"put", "delete"' '"from/xaa/all/**", "from/xaa/xab/**"'
  acl_rule fed-rp-w-interest-xab '"egress"' '"declare_subscriber", "query", "declare_queryable"' '"from/xaa/**"'
  acl_rule trial-peer-r-all '"egress", "ingress"' '"declare_subscriber", "query", "declare_queryable", "reply", "declare_liveliness_subscriber", "liveliness_query"' '"**", "**/@adv/**"'
  acl_rule trial-peer-deliver-all '"egress"' '"put", "delete", "liveliness_token"' '"**", "**/@adv/**"'
  echo '    ],'
  echo "    subjects: [ $(subj edge "$FAR_ZID" fed.xab.xaa.01), $(subj republisher "$CLIENT_ZID" fed.xaa.xab.republisher.01) ],"
  echo '    policies: ['
  echo '      { subjects: ["edge"], rules: ["fed-w-xab", "fed-w-interest-xab", "fed-r-xab", "fed-r-interest-xab", "trial-peer-r-all", "trial-peer-deliver-all"] },'
  echo '      { subjects: ["republisher"], rules: ["fed-rp-r-xab", "fed-rp-r-interest-xab", "fed-rp-w-xab", "fed-rp-w-interest-xab", "trial-peer-r-all", "trial-peer-deliver-all"] },'
  echo '    ],'
  echo '  },'
}
edge_acl() { # <partner zid> <partner name> <core zid> <core name> <ingress> <egress> <egress interest>
  [ "$acl" = on ] || return 0
  echo '  access_control: { enabled: true, default_permission: "deny", rules: ['
  acl_rule partner-land '"ingress"' '"put", "delete", "liveliness_token"' "$5"
  acl_rule partner-land-interest '"egress"' '"declare_subscriber", "query", "declare_queryable", "liveliness_query"' "$5"
  acl_rule partner-receive '"egress"' '"put", "delete", "declare_subscriber", "query", "reply"' "$6"
  acl_rule partner-receive-interest '"ingress"' '"declare_subscriber", "query", "declare_queryable", "liveliness_query"' "$7"
  acl_rule core-forward '"ingress", "egress"' '"put", "delete", "declare_subscriber", "query", "reply", "liveliness_token"' "$5"
  acl_rule core-return '"ingress", "egress"' '"put", "delete", "declare_subscriber", "query", "reply", "liveliness_token"' "$7"
  echo '    ],'
  echo "    subjects: [ $(subj partner-peer "$1" "$2"), $(subj our-core "$3" "$4") ],"
  echo '    policies: [ { rules: ["partner-land", "partner-land-interest", "partner-receive", "partner-receive-interest"], subjects: ["partner-peer"] }, { rules: ["core-forward", "core-return"], subjects: ["our-core"] } ],'
  echo '  },'
}
storage() { # <name> <key_expr>: one storage entry exactly as the role renders it
  printf '        "%s": {\n          key_expr: "%s",\n          volume: { id: "rocksdb", dir: "%s/%s", on_closure: "do_nothing", create_db: true, db_recovery_enabled: false },\n          garbage_collect_period: 86400,\n          time_to_live: 2592000000,\n        },\n' "$1" "$2" "$store" "$1"
}
plugins_rest() {
  echo '  plugins_loading: { enabled: true, search_dirs: ["/", "/opt/zenoh/plugins"] },'
  echo '  plugins: { rest: { http_port: "0.0.0.0:8000" } },'
}
# A dial to <host:port>, over TLS with the endpoint's own CA and, optionally, its own certificate.
dial() { # <host:port> <root ca label or ""> <connect label or "">
  if [ "$proto" = tcp ]; then echo "tcp/$1"; return; fi
  opts=""
  [ -n "$2" ] && opts="root_ca_certificate_file=/tls/$2.pem"
  [ -n "$3" ] && opts="${opts:+$opts;}connect_certificate_file=/tls/$3.pem;connect_private_key_file=/tls/$3-key.pem"
  echo "tls/$1${opts:+#$opts}"
}

# ---- configs -----------------------------------------------------------------
{
  echo '{'
  echo '  mode: "router",'
  # The role pins the core's id (goat_zenoh_router_zid).
  [ "${ZENOH_CORE_ZID:-pinned}" = random ] || echo "  id: \"$CORE_ZID\","
  echo '  timestamping: { enabled: true, drop_future_timestamp: false },'
  echo "  listen: { endpoints: [\"$proto/0.0.0.0:7447\"] },"
  tls_conf ca-xab core-xab ""
  core_acl
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
{ # our edge (xab) for partner xaa: fragment trust, the partner's CA global
  echo '{'
  echo '  mode: "router",'
  echo "  id: \"$EDGE_ZID\","
  echo "  listen: { endpoints: [\"$proto/0.0.0.0:7448\"] },"
  echo "  connect: { endpoints: [\"$(dial "$core:7447" ca-xab "")\"] },"
  echo '  scouting: { gossip: { enabled: false }, multicast: { enabled: false } },'
  tls_conf ca-xaa xab-outward xab-inward edge
  edge_acl "$FAR_ZID" fed.xaa.xab.01 "$CORE_ZID" core-xab '"from/xaa/**"' '"from/xab/all/**", "from/xab/xaa/**"' '"from/xab/**"'
  plugins_rest
  echo '}'
} > "$work/edge.json5"
{ # the partner's edge (xaa): dials us with its outward identity, its core with the inward one
  echo '{'
  echo '  mode: "router",'
  echo "  id: \"$FAR_ZID\","
  echo "  connect: { endpoints: [\"$(dial "$edge:7448" "" xaa-outward)\", \"$(dial "$farcore:7447" ca-xaa "")\"] },"
  echo '  scouting: { gossip: { enabled: false }, multicast: { enabled: false } },'
  tls_conf ca-xab "" xaa-inward edge
  edge_acl "$EDGE_ZID" fed.xab.xaa.01 "$FARCORE_ZID" core-xaa '"from/xab/**"' '"from/xaa/all/**", "from/xaa/xab/**"' '"from/xaa/**"'
  plugins_rest
  echo '}'
} > "$work/far.json5"
{
  echo '{'
  echo '  mode: "router",'
  echo "  id: \"$FARCORE_ZID\","
  echo '  timestamping: { enabled: true, drop_future_timestamp: false },'
  echo "  listen: { endpoints: [\"$proto/0.0.0.0:7447\"] },"
  tls_conf ca-xaa core-xaa ""
  farcore_acl
  plugins_rest
  echo '}'
} > "$work/farcore.json5"
{
  echo '{'
  echo '  mode: "client",'
  echo "  id: \"$CLIENT_ZID\","
  echo "  connect: { endpoints: [\"$proto/$farcore:7447\"] },"
  tls_conf ca-xaa "" republisher
  plugins_rest
  echo '}'
} > "$work/client.json5"

# ---- run ---------------------------------------------------------------------
docker network create "$net" >/dev/null || die "docker network create failed"
run() { # <name> <config> [extra docker args...]
  n="$1"; c="$2"; shift 2
  docker run -d --name "$n" --network "$net" -p 127.0.0.1::8000 -e RUST_LOG=info \
    -v "$work/tls:/tls:ro" -v "$work/$c:/etc/zenoh/config.json5:ro" "$@" \
    "$ZENOH_IMAGE" --config /etc/zenoh/config.json5 >/dev/null || die "$n did not start"
}
if [ -n "$core_config" ]; then
  cp "$core_config" "$work/core.json5"
  # The paths the configuration names: its certificate, key and CA, its stores' parent.
  paths=$(python3 -I -c '
import json, os, sys
c = json.load(open(sys.argv[1]))
t = c["transport"]["link"]["tls"]
dirs = [s["volume"]["dir"] for s in c["plugins"]["storage_manager"]["storages"].values()]
print(t["listen_certificate"], t["listen_private_key"], t["root_ca_certificate"], os.path.commonpath(dirs) if len(dirs) > 1 else os.path.dirname(dirs[0]), c["plugins"]["storage_manager"]["backend_search_dirs"][0], c.get("id") or "")
' "$work/core.json5") || die "ZENOH_CORE_CONFIG: cannot read its tls paths, stores and backend dir"
  set -- $paths
  [ "${6:-}" = "" ] || CORE_ZID="$6"
  run "$core" core.json5 -v "$work/tls/core-xab.pem:$1:ro" -v "$work/tls/core-xab-key.pem:$2:ro" -v "$work/tls/ca-xab.pem:$3:ro" \
    -v "$work/rocksdb:$4" -v "$work/plugins:$5:ro"
  set --
else
  run "$core" core.json5 -v "$work/plugins:/opt/zenoh/plugins:ro" -v "$work/rocksdb:$store"
fi
run "$farcore" farcore.json5
sleep 2
run "$edge" edge.json5
run "$far" far.json5
run "$client" client.json5
if [ -n "$rp_image" ]; then
  cp "$ZENOH_AGREEMENT" "$work/agreement.json" && chmod 0644 "$work/agreement.json"
  # Deployed as roles/goat-fed-republisher runs it: both sessions dial its own core.
  docker run -d --name "$rp" --network "$net" -v "$work/tls:/tls:ro" -v "$work/agreement.json:/agreement.json:ro" \
    -e RUST_LOG=info \
    -e FED_REPUBLISHER_AGREEMENT=/agreement.json -e FED_REPUBLISHER_SOVEREIGN_ID=xaa \
    -e FED_REPUBLISHER_LOCAL_ENDPOINT="tls/$farcore:7447" -e FED_REPUBLISHER_EDGE_ENDPOINT="tls/$farcore:7447" \
    -e FED_REPUBLISHER_LOCAL_CERT=/tls/republisher.pem -e FED_REPUBLISHER_LOCAL_KEY=/tls/republisher-key.pem -e FED_REPUBLISHER_LOCAL_ROOT_CA=/tls/ca-xaa.pem \
    -e FED_REPUBLISHER_EDGE_CERT=/tls/republisher.pem -e FED_REPUBLISHER_EDGE_KEY=/tls/republisher-key.pem -e FED_REPUBLISHER_EDGE_ROOT_CA=/tls/ca-xaa.pem \
    "$rp_image" >/dev/null || die "the republisher did not start"
fi

url() { echo "http://127.0.0.1:$(docker port "$1" 8000/tcp | head -n 1 | sed 's/.*://')"; }
crest=""; erest=""; frest=""; fcrest=""; clrest=""
ready() { # both stores listed at our core, every router and the client answering
  crest=$(url "$core"); erest=$(url "$edge"); frest=$(url "$far"); fcrest=$(url "$farcore"); clrest=$(url "$client")
  i=0
  while [ $i -lt 60 ]; do
    n=$(curl -fs "$crest/@/local/router/status/plugins/storage_manager/storages/**" 2>/dev/null | grep -o 'storages/sovereign_from[a-z_]*' | sort -u | wc -l)
    if [ "$n" -ge 2 ] && curl -fs -o /dev/null "$erest/@/local/router" 2>/dev/null && curl -fs -o /dev/null "$frest/@/local/router" 2>/dev/null &&
       curl -fs -o /dev/null "$fcrest/@/local/router" 2>/dev/null && curl -fs -o /dev/null "$clrest/@/local/**" 2>/dev/null; then
      return 0
    fi
    sleep 1; i=$((i + 1))
  done
  docker logs "$core" 2>&1 | tail -n 20 >&2
  die "the routers did not come up within 60 s"
}
put() { # <base url> <key> <value>
  curl -fs -X PUT -H 'content-type: text/plain' --data "$3" "$1/$2" >/dev/null || die "PUT $2 failed"
}
holds() { # <key> <value>: a GET on our core returns the value (plain or base64)
  curl -fs "$crest/$1" 2>/dev/null | grep -q "\"$2\"\|$(printf '%s' "$2" | base64)"
}

fails=0
pass() { echo "  PASS  $1"; }
fail() { echo "  FAIL  $1"; fails=$((fails + 1)); }
read_back() { # <key> <value> <label>
  if holds "$1" "$2"; then pass "$3: GET $1 at our core returns $2"; else fail "$3: GET $1 at our core returned $(curl -fs "$crest/$1" 2>/dev/null || echo nothing), expected $2"; fi
}
arrives() { # <base url> <where> <put key> <stored key> <value> <label>: stored at our core within 20 s
  put "$1" "$3" "$5"
  i=0
  while [ $i -lt 20 ]; do
    holds "$4" "$5" && { pass "$6: PUT $3 at $2 is stored at our core as $4"; return; }
    sleep 1; i=$((i + 1))
  done
  fail "$6: PUT $3 at $2 is not stored at our core as $4 within 20 s"
}
via_edge() { arrives "$erest" "our edge" "$1" "$1" "$2" "$3"; }
via_far() { arrives "$frest" "the partner's edge" "$1" "$1" "$2" "$3"; }
via_client() { arrives "$clrest" "a client of the partner's core" "$1" "$1" "$2" "$3"; }
via_republisher() { # <release key under release/all/> <value> <label>
  [ -n "$rp_image" ] || return 0
  arrives "$fcrest" "the partner's core (crossing by its republisher)" "release/all/$1" "from/xaa/all/$1" "$2" "$3"
}
refused() { # <key> <value> <label>: never stored at our core (the access control holds)
  [ "$acl" = on ] || return 0
  put "$clrest" "$1" "$2"
  sleep 5
  if holds "$1" "$2"; then fail "$3: PUT $1 by the partner core's client reached our core"; else pass "$3: PUT $1 by the partner core's client is not stored at our core"; fi
}

ready
if [ -n "$rp_image" ]; then
  i=0; until docker logs "$rp" 2>&1 | grep -q 'republishing' || [ $i -ge 30 ]; do sleep 1; i=$((i + 1)); done
fi
echo "before our core restarts ($proto, access control $acl)"
put "$crest" from/xaa/core-before c-before
sleep 1
read_back from/xaa/core-before c-before "put at our core"
via_edge from/xaa/edge-before e-before "put at our edge"
via_far from/xaa/all/far-before f-before "put at the partner's edge"
via_client from/xaa/all/client-before r-before "put by a client of the partner's core"
via_republisher rp-before p-before "a release republished by the partner"

docker restart "$core" >/dev/null || die "docker restart failed"
ready
echo "after our core restarts (everything else keeps running)"
read_back from/xaa/core-before c-before "put at our core before the restart"
read_back from/xaa/edge-before e-before "put at our edge before the restart"
read_back 'from/xaa/**' c-before "a selector over from/xaa/**"
put "$crest" from/xaa/core-after c-after
sleep 1
read_back from/xaa/core-after c-after "put at our core after the restart"
via_edge from/xaa/edge-after e-after "put at our edge after the restart"
via_far from/xaa/all/far-after f-after "put at the partner's edge after the restart"
via_client from/xaa/all/client-after r-after "put by a client of the partner's core after the restart"
via_republisher rp-after p-after "a release republished by the partner after the restart"
refused from/xaa/not-offered/x n-after "a set the partner does not offer, after the restart"

warns=$(docker logs "$core" 2>&1 | grep -c 'no data_info in database')
echo "our core's log: $warns line(s) 'no data_info in database, however payload data exists' (printed for any key a store does not hold)"
if [ "$fails" -gt 0 ]; then
  for n in "$core" "$edge" "$far"; do
    echo "--- $n, last lines:"
    docker logs "$n" 2>&1 | tail -n 12
  done
  [ -n "$rp_image" ] && { echo "--- republisher, last lines:"; docker logs "$rp" 2>&1 | tail -n 8; }
  echo "zenoh-core-storage-restart: FAIL ($fails)"
  exit 1
fi
echo "zenoh-core-storage-restart: PASS"
