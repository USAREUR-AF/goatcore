#!/usr/bin/env bash
# live-host.sh — the host half of ops/federation-e2e/live.sh. Runs as root on ONE
# sovereign's host; live.sh copies it to /opt/fed-live/live-host.sh and calls one verb
# at a time over SSH. Every verb is idempotent.
#
#   prereqs                       packages and the ansible venv
#   login      <user>             docker login ghcr.io, the token on stdin
#   images                        both images, by digest
#   logout                        docker logout ghcr.io and the credential file removed
#   sovereign  <us> <them> <core> this sovereign's CA, signing key and certificates,
#                                 made here; private keys never leave the host
#   clone      <us> <them> <core> the ops tree from /opt/fed-live/ops.tar.gz becomes this
#                                 sovereign's repository, with its declarations
#   core       <us> <them> <core> the core router and spire-server, on <core> and loopback
#   edge       <us> <them> <listener true|false> <endpoint>
#                                 the operator's profile and federation.toml
#   drive      <us> <them> <avenue> <from> <to>
#                                 the real driver, unmodified
#   put        <path> [mode]      stdin to <path>
#   admit      <us> <them>        phase 10, as an operator does it
#   probe      <us> <them> <name> <cert> <key> <rest port> <zid>
#   responder  <us> <them>        the round-trip responder, beside the core
#   ansible    <args...>          one play through ansible-preflight, as the driver runs it
#   collect    <us> <them>        logs and state for the record, on stdout
#   reset                         everything this run made, packages and images kept
#
# Environment: FED_ZENOH_IMAGE, FED_SPIRE_IMAGE, FED_REPUBLISHER_IMAGE (digest-pinned).
set -euo pipefail
VERB="${1:?verb}"; shift
D=/opt/fed-live
K=$D/keys
PY=/opt/goat-ansible/bin/python
die() { echo "live-host: $*" >&2; exit 1; }
need_images() {
  case "${FED_ZENOH_IMAGE:-}${FED_SPIRE_IMAGE:-}" in *@sha256:*@sha256:*) ;; *) die "FED_ZENOH_IMAGE and FED_SPIRE_IMAGE must both be digest-pinned" ;; esac
}
td() { echo "$1.fed.test"; }

case "$VERB" in
prereqs)
  need_images
  [ "$(uname -m)" = x86_64 ] || die "this host is $(uname -m); the goat binaries are x86_64"
  export DEBIAN_FRONTEND=noninteractive
  apt-get update -qq
  apt-get install -y -qq --no-install-recommends docker.io docker-compose-v2 wireguard-tools nftables \
    python3-venv git openssl curl jq iproute2 ca-certificates >/dev/null
  systemctl enable --now docker >/dev/null
  modprobe wireguard || die "no wireguard kernel module on $(uname -r)"
  if [ ! -x /opt/goat-ansible/bin/ansible-playbook ]; then
    python3 -m venv /opt/goat-ansible
    /opt/goat-ansible/bin/pip install -q 'ansible-core>=2.16,<2.21' pyyaml jinja2
  fi
  for b in /opt/goat-ansible/bin/ansible*; do ln -sf "$b" "/usr/local/bin/$(basename "$b")"; done
  for attempt in 1 2 3 4 5; do
    ansible-galaxy collection install -p /usr/share/ansible/collections \
      'ansible.posix:>=1.5.0' 'community.general:>=8.0.0' 'community.docker:>=3.4.0' >/dev/null && break
    [ "$attempt" != 5 ] || die "ansible-galaxy failed five times"
    sleep 20
  done
  goat --version >/dev/null 2>&1 || die "goat does not run here (glibc? the binaries are built on Ubuntu 24.04)"
  goatbuilder --version >/dev/null 2>&1 || die "goatbuilder does not run here"
  echo "prereqs ok: $(. /etc/os-release && echo "$PRETTY_NAME"), kernel $(uname -r), $(docker --version), $(wg --version)"
  ;;

login)
  # The token arrives on stdin and goes to docker on stdin; it is never an
  # argument and never a file here before docker writes its own config.
  docker login ghcr.io -u "${1:?user}" --password-stdin >/dev/null
  echo "login ok: ghcr.io as $1"
  ;;

images)
  need_images
  docker pull -q "$FED_ZENOH_IMAGE" >/dev/null
  docker pull -q "$FED_SPIRE_IMAGE" >/dev/null
  case "${FED_REPUBLISHER_IMAGE:-}" in *@sha256:*) ;; *) die "FED_REPUBLISHER_IMAGE must be digest-pinned" ;; esac
  docker pull -q "$FED_REPUBLISHER_IMAGE" >/dev/null
  echo "images ok: $FED_ZENOH_IMAGE, $FED_SPIRE_IMAGE, $FED_REPUBLISHER_IMAGE"
  ;;

logout)
  docker logout ghcr.io >/dev/null 2>&1 || true
  rm -f /root/.docker/config.json
  echo "logout ok"
  ;;

sovereign)
  us="$1" them="$2" core="$3"
  install -d -m 0700 "$K"; cd "$K"
  if [ ! -f ca.pem ]; then
    openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes \
      -keyout ca-key.pem -out ca.pem -days 30 -subj "/O=$us/OU=offline-ca/CN=$us Root CA" 2>/dev/null
    openssl genpkey -algorithm ed25519 -out signing.pem 2>/dev/null
    openssl pkey -in signing.pem -outform DER 2>/dev/null | tail -c 32 | od -An -tx1 | tr -d ' \n' > signing.key
    openssl pkey -in signing.pem -pubout -outform DER 2>/dev/null | tail -c 32 | base64 | tr -d '\n' > signing.pub
  fi
  leaf() { # name CN SAN EKU
    [ -f "$1.pem" ] && return 0
    openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -keyout "$1-key.pem" -out "$1.csr" -subj "/CN=$2" 2>/dev/null
    printf 'subjectAltName=%s\nbasicConstraints=CA:FALSE\nextendedKeyUsage=%s\n' "$3" "$4" > "$1.ext"
    openssl x509 -req -in "$1.csr" -CA ca.pem -CAkey ca-key.pem -CAserial ca.srl -CAcreateserial \
      -out "$1.pem" -days 30 -extfile "$1.ext" 2>/dev/null
    rm -f "$1.csr" "$1.ext"
  }
  leaf profile "operator-$us" "URI:spiffe://$(td "$us")/operator" clientAuth
  leaf core "core-$us" "DNS:core-$us,IP:$core" serverAuth,clientAuth
  # A name this CA can mint and the agreement never named, and the far core's name.
  leaf forged "fed.$us.forged.01" "DNS:fed.$us.forged.01" clientAuth
  leaf impersonate "core-$them" "DNS:core-$them" clientAuth
  chmod 0600 ./*key.pem signing.pem signing.key
  cat signing.pub
  ;;

clone)
  us="$1" them="$2" core="$3"; need_images
  [ -f "$D/ops.tar.gz" ] || die "no $D/ops.tar.gz"
  rm -rf /srv/goat.git /opt/goat "$D/src"; install -d "$D/src"
  tar -xzf "$D/ops.tar.gz" -C "$D/src"
  git -C "$D/src" init -q -b main
  git -C "$D/src" config user.name "$us operator"; git -C "$D/src" config user.email "ops@$us.test"
  "$PY" "$D/src/ops/federation-e2e/overlay.py" "$D/src" "$us" "$them" 0 "$FED_ZENOH_IMAGE" \
    --core-addr "$core" --spire-listen "$core" --local --republisher-image "${FED_REPUBLISHER_IMAGE:-}"
  git -C "$D/src" add -A; git -C "$D/src" commit -q -m "$us: this sovereign's tree and its declarations for the peering"
  git clone -q --bare "$D/src" /srv/goat.git
  git clone -q /srv/goat.git /opt/goat
  git -C /opt/goat config user.name "$us operator"; git -C /opt/goat config user.email "ops@$us.test"
  rm -rf "$D/src"
  git -C /opt/goat log -1 --format='clone ok: %h %s'
  ;;

core)
  us="$1" them="$2" core="$3"; need_images
  # The core's address exists on this host alone: a dummy interface, so nothing off the
  # host reaches it, and the edge container reaches it through the host.
  cat > /etc/systemd/system/fed-live-core-addr.service <<UNIT
[Unit]
Description=The federation run's core address ($core), host-local
Before=docker.service
[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart=/bin/sh -c 'ip link show fed-core0 >/dev/null 2>&1 || ip link add fed-core0 type dummy; ip addr replace $core/32 dev fed-core0; ip link set fed-core0 up'
ExecStop=/sbin/ip link del fed-core0
[Install]
WantedBy=multi-user.target
UNIT
  systemctl daemon-reload; systemctl enable --now fed-live-core-addr.service >/dev/null
  install -d -m 0755 /etc/goat-core
  sed "s/__THEM__/$them/g; s/__US__/$us/g; s/__CORE_LISTEN__/$core/; s/__CORE_REST__/127.0.0.1/" \
    /opt/goat/ops/federation-e2e/core-seed.json5 > "$D/core-seed.json5"
  [ -f /etc/goat-core/config.json5 ] || cp "$D/core-seed.json5" /etc/goat-core/config.json5
  install -m 0644 "$K/ca.pem" /etc/goat-core/ca.pem
  [ -f /etc/goat-core/core.pem ] || { install -m 0644 "$K/core.pem" /etc/goat-core/core.pem; install -m 0644 "$K/core-key.pem" /etc/goat-core/core-key.pem; }
  docker rm -f goat-zenoh "$us-zenoh-router" >/dev/null 2>&1 || true
  docker run -d --name "$us-zenoh-router" --label org.goat.component=zenoh-core --restart unless-stopped --network host \
    -e RUST_LOG=zenoh=info,zenoh::net::routing::interceptor::access_control=trace \
    -v /etc/goat-core:/etc/goat-core:ro "$FED_ZENOH_IMAGE" -c /etc/goat-core/config.json5 >/dev/null
  install -d -m 0755 /opt/spire/config; install -d -o 1000 -g 1000 -m 0750 /opt/spire/data /opt/spire/data/server
  "$PY" /opt/goat/ops/federation-e2e/render-spire.py /opt/goat/ops/ansible/roles/spire/templates/server.conf.j2 \
    "$(td "$us")" /opt/spire/config/server.conf "$core"
  docker rm -f spire-server >/dev/null 2>&1 || true
  docker run -d --name spire-server --restart unless-stopped --network host \
    -v /opt/spire/config:/opt/spire/config:ro -v /opt/spire/data:/opt/spire/data \
    "$FED_SPIRE_IMAGE" -config /opt/spire/config/server.conf >/dev/null
  for _ in $(seq 1 30); do
    if docker exec spire-server /opt/spire/bin/spire-server healthcheck >/dev/null 2>&1; then echo "core ok: zenohd on $core:7447, REST on 127.0.0.1:8000, spire-server healthy"; exit 0; fi
    sleep 2
  done
  docker logs spire-server 2>&1 | tail -20; die "spire-server did not become healthy"
  ;;

edge)
  us="$1" them="$2" listener="$3" endpoint="$4"
  p=/root/.goat/contexts/default; w="/var/lib/goat-federation/$them"
  install -d -m 0700 /root/.goat /root/.goat/contexts "$p"
  install -m 0644 "$K/profile.pem" "$p/mtls.cert.pem"
  install -m 0644 "$K/ca.pem" "$p/ca-roots.pem"
  cat > "$p/federation.toml" <<TOML
sovereign_id = "$us"
term_days    = 30
cut_contact  = "fed-ops@$us.test"

[transport]
interface            = "wg-fed0"
endpoint             = "$endpoint"
listener             = $listener
proposed_link_subnet = "169.254.77.0/31"
measured_mtu         = 1380

[data]
release                  = ["release/all/**", "release/$them/**"]
accept_hz                = 100
accept_bytes_per_message = 1048576
accept_distinct_keys     = 5000
TOML
  install -d -m 0755 /var/lib/goat-federation "$w" "$w/pki"
  install -m 0644 "$K/ca.pem" "$w/pki/our-ca.pem"
  echo "edge ok: $us, listener=$listener, endpoint $endpoint"
  ;;

drive)
  us="$1" them="$2" avenue="$3" from="$4" to="$5"
  export FED_WORK="/var/lib/goat-federation/$them" FED_SIGNING_KEY="$K/signing.key"
  export FED_CA_CERT="$K/ca.pem" FED_CA_KEY="$K/ca-key.pem" FED_CORE_CN="core-$us"
  FED_CORE_SAN="$(openssl x509 -in "$K/core.pem" -noout -ext subjectAltName | grep -oE 'IP Address:[0-9.]+' | cut -d: -f2)"
  export FED_CORE_SAN GOAT_AWS_MODE=none GOAT_BIN=/usr/local/bin/goat GOAT_ANSIBLE_TRUNK_REMOTE=origin HOME=/root
  # The harness places each side's package and CA itself; phase 5 runs without a directory.
  export FED_NO_DIRECTORY=1
  cd /opt/goat
  case "$avenue" in
    goatbuilder) exec goatbuilder -C /opt/goat --yes federate "$them" --as "$us" --from "$from" --to "$to" ;;
    script)      exec ops/scripts/peering-standup.sh "$them" --as "$us" --from "$from" --to "$to" ;;
    *) die "unknown avenue $avenue" ;;
  esac
  ;;

put)
  install -d "$(dirname "$1")"; cat > "$1"; chmod "${2:-0644}" "$1"
  ;;

admit)
  us="$1" them="$2"; w="/var/lib/goat-federation/$them"
  want="$(python3 -c 'import json,sys; ag=json.load(open(sys.argv[1])); print([p["ca_fingerprint"] for p in ag["parties"] if p["sovereign_id"]==sys.argv[2]][0])' "$w/agreement.json" "$them")"
  got="sha256:$(openssl x509 -in "$w/theirs-ca.pem" -outform DER | sha256sum | cut -d' ' -f1)"
  [ "$want" = "$got" ] || die "$them's CA does not match the agreement: $got, agreed $want"
  install -m 0644 "$w/theirs-ca.pem" "$w/pki/partner-ca.pem"
  python3 /opt/goat/ops/ansible/roles/goat-zenoh/files/acl-assemble.py assemble --seed "$D/core-seed.json5" \
    --org "$w/applied/ops/federation/acl/$them-fragment.json5" --out /etc/goat-core/config.json5
  install -m 0644 "$w/pki/core.pem" /etc/goat-core/core.pem
  install -m 0644 "$w/pki/core-key.pem" /etc/goat-core/core-key.pem
  docker restart "$us-zenoh-router" >/dev/null
  echo "admit ok: $them's CA matches $want; fragment spliced; core restarted on the minted certificate"
  ;;

probe)
  us="$1" them="$2" name="$3" cert="$4" key="$5" port="$6" zid="$7"; w="/var/lib/goat-federation/$them"; dir="/etc/fed-probe-$name"
  far="$(python3 -c 'import json,sys,ipaddress as i; ag=json.load(open(sys.argv[1])); h=list(i.ip_network(ag["transport"]["link_subnet"])); ps=[p["sovereign_id"] for p in ag["parties"]]; print(h[ps.index(sys.argv[2])])' "$w/agreement.json" "$them")"
  install -d -m 0755 "$dir"
  sed "s/__FAR_LINK_IP__/$far/; s/__REST_ADDR__/127.0.0.1/; s/__REST_PORT__/$port/; s/__ZID__/$zid/" \
    /opt/goat/ops/federation-e2e/probe.json5 > "$dir/config.json5"
  install -m 0644 "$w/pki/partner-ca.pem" "$dir/far-ca.pem"
  install -m 0644 "$cert" "$dir/cert.pem"; install -m 0644 "$key" "$dir/key.pem"
  docker rm -f "fed-probe-$name" >/dev/null 2>&1 || true
  docker run -d --name "fed-probe-$name" --network host --restart on-failure -v "$dir:/probe:ro" \
    -e RUST_LOG=zenoh=info "$FED_ZENOH_IMAGE" -c /probe/config.json5 >/dev/null
  for _ in $(seq 1 30); do curl -s -m 2 -o /dev/null "http://127.0.0.1:$port/@/**" && { echo "probe $name ok on 127.0.0.1:$port"; exit 0; }; sleep 2; done
  docker logs "fed-probe-$name" 2>&1 | tail -20; die "probe $name did not come up"
  ;;

responder)
  us="$1" them="$2"
  install -m 0755 /opt/goat/ops/federation/roundtrip/responder.py /usr/local/bin/fed-responder.py
  systemctl stop fed-live-responder >/dev/null 2>&1 || true
  systemctl reset-failed fed-live-responder >/dev/null 2>&1 || true
  systemd-run --quiet --unit fed-live-responder python3 /usr/local/bin/fed-responder.py "from/$them" "release/all"
  echo "responder ok: answers from/$them under release/all, crossing as from/$us/all"
  ;;

ansible)
  cd /opt/goat/ops/ansible
  exec env GOAT_AWS_MODE=none GOAT_ANSIBLE_TRUNK_REMOTE=origin ../scripts/with-aws none -- ../scripts/ansible-preflight.sh "$@"
  ;;

collect)
  us="$1" them="$2"
  echo "=== wg"; wg show wg-fed0 2>&1 | grep -v 'private key' || true
  echo "=== wg state"; cat /var/log/goat-peering-wg-wg-fed0-state.json 2>&1 || true
  echo "=== edge router"; docker logs "goat-zenoh-edge-$them" 2>&1 | tail -400 || true
  echo "=== core router"; docker logs "$us-zenoh-router" 2>&1 | tail -200 || true
  echo "=== spire-server"; docker logs spire-server 2>&1 | tail -100 || true
  echo "=== host"; . /etc/os-release; echo "$PRETTY_NAME $(uname -r)"
  ;;

reset)
  systemctl stop fed-live-responder goat-peering-bundle-in.socket goat-peering-bundle-out.socket \
    goat-peering-bundle-in.service goat-peering-bundle-out.service 'goat-peering-wg@wg-fed0.service' >/dev/null 2>&1 || true # oss-allow: systemd unit instance, not a mailbox
  systemctl disable goat-peering-bundle-in.socket goat-peering-bundle-out.socket 'goat-peering-wg@wg-fed0.service' >/dev/null 2>&1 || true # oss-allow: systemd unit instance, not a mailbox
  ids="$( { docker ps -aq --filter name=goat-zenoh --filter name=spire-server --filter name=fed-probe-; docker ps -aq --filter label=org.goat.component=zenoh-core; } 2>/dev/null | sort -u || true)"
  [ -z "$ids" ] || docker rm -f $ids >/dev/null
  for c in /opt/goat-zenoh-edge-*; do [ -d "$c" ] && (cd "$c" && docker compose down -v >/dev/null 2>&1 || true); done
  ip link del wg-fed0 >/dev/null 2>&1 || true
  for t in $(nft list tables 2>/dev/null | awk '/goat_peering/{print $3}'); do nft delete table inet "$t"; done
  rm -rf /var/lib/goat-federation /etc/goat-core /opt/spire /etc/goat-zenoh-edge-* /opt/goat-zenoh-edge-* \
    /etc/goat-peering-wg /etc/fed-probe-* /root/.goat /opt/goat /srv/goat.git "$K" \
    /etc/systemd/system/goat-peering-bundle-* /var/log/goat-peering-wg-*
  systemctl daemon-reload
  echo "reset ok"
  ;;

*) die "unknown verb $VERB" ;;
esac
