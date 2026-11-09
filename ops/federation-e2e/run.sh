#!/usr/bin/env bash
# run.sh — rung 9 end to end: two sovereigns, generated fresh, federate through the real
# driver on container hosts, and the proof comes from the receiving side's records.
#
# docs/design/deployment-e2e.md §Hermetic is the pattern. Four privileged hosts run
# systemd and their own dockerd:
#
#   <s>-core   the role-named core router, a real spire-server and, from phase 12, the
#              outbound republisher (built here from ops/federation/republisher)
#   <s>-edge   the operator's station and the edge host: the driver runs here, the plays
#              reach it over ssh, its dockerd runs the edge router
#
# Each sovereign has its own CA, its own signing key and its own git repository. The two
# internal networks do not reach each other; the two edges alone share the boundary
# network, and only the tunnel's UDP port crosses it. A peering directory (saltlick, behind
# a TLS front with its own local CA) sits on the boundary network, and phase 5 exchanges
# each side's package, CA certificate and SPIFFE bundle through it, as against a real
# partner. The voice call is scripted as a keyring, and phase 10's review as an
# acl-assemble. Nothing else about the driver is touched.
#
# Each sovereign offers TWO release sets, `release/all/**` and `release/<them>/**`, and
# phase 14 proves each one crosses as `from/<us>/<set>/**`: staged on the sending core,
# lifted by its republisher, read back from the receiving core's store.
#
# Two scenarios, because nothing is asymmetric:
#   1  pasture listens and goes first, both sides through `goatbuilder federate`
#   2  meadow listens and goes first, both sides through peering-standup.sh
#
# Linux + a Docker daemon + the wireguard kernel module + a built goat and goatbuilder.
# Touches no goatnet, no cloud account and no credential store. Synthetic names only.
# Everything a sovereign needs is generated here or read from files every checkout
# carries: the images from ops/pins/fleet-pins.yaml `components:`, each clone's
# goatbuilder config and group_vars from overlay.py. A public checkout runs it as is.
#
# --prepare-only stops before the first container: the image pins, the refusals that need
# no hosts, and both sovereigns generated, each clone's config validated and its
# federation planned. It needs no Docker and no --goat (CI: check-federation-e2e-prepare.sh).
#
# usage: ops/federation-e2e/run.sh --goat BIN --goatbuilder BIN [--work DIR] [--record DIR]
#                                  [--scenario 1|2] [--keep] [--prepare-only]
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"
WORK=""; RECORD=""; GOAT=""; GB=""; KEEP=0; ONLY=""; PREPARE=0
while [ $# -gt 0 ]; do
  case "$1" in
    --goat) GOAT="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"; shift 2 ;;
    --goatbuilder) GB="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"; shift 2 ;;
    --work) WORK="$2"; shift 2 ;;
    --record) RECORD="$2"; shift 2 ;;
    --scenario) ONLY="$2"; shift 2 ;;
    --keep) KEEP=1; shift ;;
    --prepare-only) PREPARE=1; shift ;;
    -h|--help) sed -n '2,34p' "$0"; exit 0 ;;
    *) echo "run.sh: unknown argument $1" >&2; exit 2 ;;
  esac
done
if [ "$PREPARE" = 1 ]; then
  [ -x "$GB" ] || { echo "run.sh: --goatbuilder must name a built binary" >&2; exit 2; }
else
  [ -x "$GOAT" ] && [ -x "$GB" ] || { echo "run.sh: --goat and --goatbuilder must name built binaries" >&2; exit 2; }
fi
[ -n "$WORK" ] || WORK="$(mktemp -d "${TMPDIR:-/tmp}/fed-e2e.XXXXXX")"
mkdir -p "$WORK"; WORK="$(cd "$WORK" && pwd)"
[ -n "$RECORD" ] || RECORD="$WORK/report"
mkdir -p "$RECORD"; RECORD="$(cd "$RECORD" && pwd)"
export E2E_RECORD_DIR="$RECORD"
# The clones carry no LFS objects (the briefs' PDFs); checking one out must not try to smudge them.
export GIT_LFS_SKIP_SMUDGE=1

# The fleet image, by digest, and the SPIRE release the fleet pins: the `spire`
# component's value of record in ops/pins/fleet-pins.yaml, which every checkout carries
# (a public one declares no goatnet there, but keeps every component). Both pull
# anonymously.
ZENOH_IMAGE="ghcr.io/usareur-af/zenoh-pqc@sha256:372de39d2e8b64fe7157194173f1b7ac6eac2700ff04c49cce2ce3c284a6fdb2"
SPIRE_IMAGE="$(python3 -c 'import sys, yaml; print(((yaml.safe_load(open(sys.argv[1])).get("components") or {}).get("spire") or {}).get("fleet") or "")' "$REPO/ops/pins/fleet-pins.yaml" 2>/dev/null || true)"
[[ "$SPIRE_IMAGE" =~ ^ghcr\.io/spiffe/spire-server:[^@[:space:]]+@sha256:[0-9a-f]{64}$ ]] || { echo "run.sh: ops/pins/fleet-pins.yaml declares no digest-pinned SPIRE image (components.spire.fleet)" >&2; exit 2; }
HOST_IMAGE=fed-e2e-host
# The peering directory both edges reach, and the TLS front that issues its certificate
# from a local CA: saltlick refuses a pointer whose package_url is not https.
SALTLICK_IMAGE="ghcr.io/usareur-af/saltlick@sha256:6ec46ca98819e1523f30e85de8210db33f71040f1214171366e459f79bab7971"
DIRECTORY_TLS_IMAGE="caddy@sha256:c3d7ee5d2b11f9dc54f947f68a734c84e9c9666c92c88a7f30b9cba5da182adb"
DIRECTORY_IP=10.77.3.40
# The republisher, built from this checkout and loaded into each core host's dockerd.
# The role takes its content-addressed id (`sha256:<64 hex>`), as immutable as a
# registry digest, until the published image is pinned in ops/pins/fleet-pins.yaml.
RP_TAG=fed-e2e-republisher
RP_IMAGE=""
ACL_ASSEMBLE="$REPO/ops/ansible/roles/goat-zenoh/files/acl-assemble.py"
REC="$REPO/ops/e2e/rec.py"
FAILS=0
rec() { python3 "$REC" "$@" || FAILS=$((FAILS + 1)); }
say() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }
die() { echo "run.sh: $*" >&2; exit 1; }
other() { [ "$1" = pasture ] && echo meadow || echo pasture; }
octet() { [ "$1" = pasture ] && echo 1 || echo 2; }
core_ip() { echo "10.77.$(octet "$1").10"; }
edge_ip() { echo "10.77.$(octet "$1").20"; }
boundary_ip() { [ "$1" = pasture ] && echo 10.77.3.11 || echo 10.77.3.12; }
td() { echo "$1.fed.test"; }

NETS=(fed-e2e-pasture fed-e2e-meadow fed-e2e-boundary)
HOSTS=(fed-pasture-core fed-pasture-edge fed-meadow-core fed-meadow-edge)
DIRECTORY=(fed-e2e-directory fed-e2e-directory-tls)
teardown() {
  local net
  docker rm -f -v "${HOSTS[@]}" "${DIRECTORY[@]}" >/dev/null 2>&1 || true
  for net in "${NETS[@]}"; do docker network rm "$net" >/dev/null 2>&1 || true; done
}
final() { [ "$KEEP" = 1 ] || teardown; }
trap final EXIT

host_exec() { local h="$1"; shift; docker exec -i "$h" "$@"; }

# --- the sovereign, generated fresh ---------------------------------------------------
gen_sovereign() { # $1 sovereign: its CA, its signing key, its operator's identity
  local s="$1" d
  d="$WORK/$s"
  rm -rf "$d"; mkdir -p "$d/keys"
  openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes \
    -keyout "$d/keys/ca-key.pem" -out "$d/keys/ca.pem" -days 30 \
    -subj "/O=$s/OU=offline-ca/CN=$s Root CA" 2>/dev/null
  openssl genpkey -algorithm ed25519 -out "$d/keys/signing.pem" 2>/dev/null
  openssl pkey -in "$d/keys/signing.pem" -outform DER 2>/dev/null | tail -c 32 | od -An -tx1 | tr -d ' \n' > "$d/keys/signing.key"
  openssl pkey -in "$d/keys/signing.pem" -pubout -outform DER 2>/dev/null | tail -c 32 | base64 | tr -d '\n' > "$d/keys/signing.pub"
  # The profile: a leaf carrying the SPIFFE URI SAN that `init` reads the trust domain from.
  openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -keyout "$d/keys/profile-key.pem" \
    -out "$d/keys/profile.csr" -subj "/CN=operator-$s" 2>/dev/null
  printf 'subjectAltName=URI:spiffe://%s/operator\nbasicConstraints=CA:FALSE\nextendedKeyUsage=clientAuth\n' "$(td "$s")" > "$d/keys/profile.ext"
  openssl x509 -req -in "$d/keys/profile.csr" -CA "$d/keys/ca.pem" -CAkey "$d/keys/ca-key.pem" \
    -CAcreateserial -out "$d/keys/profile.pem" -days 30 -extfile "$d/keys/profile.ext" 2>/dev/null
  # The core's first certificate. Phase 8 mints the one it runs with after phase 10.
  openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -keyout "$d/keys/core-key.pem" \
    -out "$d/keys/core.csr" -subj "/CN=core-$s" 2>/dev/null
  printf 'subjectAltName=DNS:core-%s,IP:%s\nbasicConstraints=CA:FALSE\nextendedKeyUsage=serverAuth,clientAuth\n' "$s" "$(core_ip "$s")" > "$d/keys/core.ext"
  openssl x509 -req -in "$d/keys/core.csr" -CA "$d/keys/ca.pem" -CAkey "$d/keys/ca-key.pem" \
    -CAcreateserial -out "$d/keys/core.pem" -days 30 -extfile "$d/keys/core.ext" 2>/dev/null
  # A name this sovereign's CA can mint and the agreement never named.
  openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -keyout "$d/keys/forged-key.pem" \
    -out "$d/keys/forged.csr" -subj "/CN=fed.$s.forged.01" 2>/dev/null
  printf 'subjectAltName=DNS:fed.%s.forged.01\nbasicConstraints=CA:FALSE\nextendedKeyUsage=clientAuth\n' "$s" > "$d/keys/forged.ext"
  openssl x509 -req -in "$d/keys/forged.csr" -CA "$d/keys/ca.pem" -CAkey "$d/keys/ca-key.pem" \
    -CAcreateserial -out "$d/keys/forged.pem" -days 30 -extfile "$d/keys/forged.ext" 2>/dev/null
  # The name that matters most: this CA minting the FAR core's common name.
  openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -keyout "$d/keys/impersonate-key.pem" \
    -out "$d/keys/impersonate.csr" -subj "/CN=core-$(other "$s")" 2>/dev/null
  printf 'subjectAltName=DNS:core-%s\nbasicConstraints=CA:FALSE\nextendedKeyUsage=clientAuth\n' "$(other "$s")" > "$d/keys/impersonate.ext"
  openssl x509 -req -in "$d/keys/impersonate.csr" -CA "$d/keys/ca.pem" -CAkey "$d/keys/ca-key.pem" \
    -CAcreateserial -out "$d/keys/impersonate.pem" -days 30 -extfile "$d/keys/impersonate.ext" 2>/dev/null
  ssh-keygen -q -t ed25519 -N '' -f "$d/keys/id_ed25519" -C "fed-e2e-$s"
  chmod 0644 "$d/keys/"*.pem "$d/keys/signing.key" "$d/keys/id_ed25519"
  # Its own repository, as two organisations would hold.
  git clone -q --depth 1 --single-branch "file://$REPO" "$d/clone"
  git -C "$d/clone" checkout -q -B main
  git -C "$d/clone" config user.name "$s operator"; git -C "$d/clone" config user.email "ops@$s.test"
  python3 "$HERE/overlay.py" "$d/clone" "$s" "$(other "$s")" "$(octet "$s")" "$ZENOH_IMAGE" --republisher-image "$RP_IMAGE"
  git -C "$d/clone" add -A
  git -C "$d/clone" commit -q -m "$s: this sovereign's declarations for the peering"
  git clone -q --bare "$d/clone" "$d/origin.git"
  git -C "$d/origin.git" symbolic-ref HEAD refs/heads/main
  git -C "$d/clone" remote set-url origin "$d/origin.git"
  git -C "$d/clone" fetch -q origin
}

start_host() { # $1 name, $2 sovereign, $3 internal ip
  local n="$1" s="$2" ip="$3"
  docker run -d --name "$n" --hostname "$n" --privileged --cgroupns=private \
    --network "fed-e2e-$s" --ip "$ip" \
    --tmpfs /run --tmpfs /run/lock -v /var/lib/docker -v /lib/modules:/lib/modules:ro \
    -v "$WORK/$s/keys:/etc/fed-e2e:ro" -v "$WORK/$s/origin.git:/srv/goat.git:ro" \
    -v "$GOAT:/usr/local/bin/goat:ro" -v "$GB:/usr/local/bin/goatbuilder:ro" \
    -e container=docker "$HOST_IMAGE" >/dev/null
}

wait_ready() { # $1 host: systemd up, dockerd answering
  local h="$1"
  for _ in $(seq 1 90); do
    if docker exec "$h" sh -c 'systemctl is-active --quiet ssh && docker version --format "{{.Server.Version}}"' >/dev/null 2>&1; then return 0; fi
    sleep 2
  done
  docker exec "$h" systemctl --no-pager status docker ssh 2>&1 | tail -20 || true
  die "host $h did not come up"
}

start_core() { # $1 sovereign: seed config, bootstrap cert, the router and the SPIRE server
  local s="$1" h="fed-$1-core" them; them="$(other "$s")"
  sed "s/__THEM__/$them/g; s/__US__/$s/g; s/__CORE_LISTEN__/0.0.0.0/; s/__CORE_REST__/0.0.0.0/" "$HERE/core-seed.json5" > "$WORK/$s/core-seed.json5"
  cp "$WORK/$s/core-seed.json5" "$WORK/$s/core.json5"
  host_exec "$h" sh -c 'install -d -m 0755 /etc/goat-core'
  docker cp -q "$WORK/$s/core.json5" "$h:/etc/goat-core/config.json5"
  docker cp -q "$WORK/$s/keys/ca.pem" "$h:/etc/goat-core/ca.pem"
  docker cp -q "$WORK/$s/keys/core.pem" "$h:/etc/goat-core/core.pem"
  docker cp -q "$WORK/$s/keys/core-key.pem" "$h:/etc/goat-core/core-key.pem"
  host_exec "$h" docker run -d --name "$s-zenoh-router" --label org.goat.component=zenoh-core --restart unless-stopped \
    -e RUST_LOG=zenoh=info,zenoh::net::routing::interceptor::access_control=trace \
    -p 7447:7447 -p 8000:8000 -v /etc/goat-core:/etc/goat-core:ro \
    "$ZENOH_IMAGE" -c /etc/goat-core/config.json5 >/dev/null
  python3 "$HERE/render-spire.py" "$REPO/ops/ansible/roles/spire/templates/server.conf.j2" "$(td "$s")" "$WORK/$s/server.conf"
  host_exec "$h" sh -c 'install -d -m 0755 /opt/spire/config; install -d -o 1000 -g 1000 -m 0750 /opt/spire/data /opt/spire/data/server'
  docker cp -q "$WORK/$s/server.conf" "$h:/opt/spire/config/server.conf"
  host_exec "$h" docker run -d --name spire-server --restart unless-stopped \
    -p 8443:8443 -v /opt/spire/config:/opt/spire/config:ro -v /opt/spire/data:/opt/spire/data \
    "$SPIRE_IMAGE" -config /opt/spire/config/server.conf >/dev/null
  for _ in $(seq 1 30); do
    host_exec "$h" docker exec spire-server /opt/spire/bin/spire-server healthcheck >/dev/null 2>&1 && return 0
    sleep 2
  done
  host_exec "$h" docker logs spire-server 2>&1 | tail -20 || true
  die "spire-server on $h did not become healthy"
}

prepare_edge() { # $1 sovereign, $2 "true" when this side listens
  local s="$1" listener="$2" h="fed-$1-edge" them p; them="$(other "$s")"
  host_exec "$h" git clone -q /srv/goat.git /opt/goat
  host_exec "$h" sh -c "git -C /opt/goat config user.name '$s operator'; git -C /opt/goat config user.email 'ops@$s.test'"
  p=/root/.goat/contexts/default
  host_exec "$h" install -d -m 0700 /root/.goat /root/.goat/contexts "$p"
  host_exec "$h" install -m 0644 /etc/fed-e2e/profile.pem "$p/mtls.cert.pem"
  host_exec "$h" install -m 0644 /etc/fed-e2e/ca.pem "$p/ca-roots.pem"
  cat > "$WORK/$s/federation.toml" <<TOML
sovereign_id = "$s"
term_days    = 30
cut_contact  = "fed-ops@$s.test"

[transport]
interface            = "wg-fed0"
endpoint             = "$(boundary_ip "$s"):51820"
listener             = $listener
proposed_link_subnet = "169.254.77.0/31"
measured_mtu         = 1380

[data]
release                  = ["release/all/**", "release/$them/**"]
accept_hz                = 100
accept_bytes_per_message = 1048576
accept_distinct_keys     = 5000
TOML
  docker cp -q "$WORK/$s/federation.toml" "$h:$p/federation.toml"
  host_exec "$h" install -d -m 0755 /var/lib/goat-federation "/var/lib/goat-federation/$them" "/var/lib/goat-federation/$them/pki"
  host_exec "$h" install -m 0644 /etc/fed-e2e/ca.pem "/var/lib/goat-federation/$them/pki/our-ca.pem"
}

# --- the driver, unmodified -----------------------------------------------------------
# ONE invocation shape per avenue. The environment is what an operator's would be; the
# only thing specific to a hermetic host is GOAT_AWS_MODE=none, the mode ADR 1204
# declares for a deploy needing no cloud credential.
drive() { # $1 sovereign, $2 avenue, $3 from, $4 to
  local s="$1" avenue="$2" from="$3" to="$4" h="fed-$1-edge" them; them="$(other "$s")"
  local env=(
    -e "FED_WORK=/var/lib/goat-federation/$them"
    -e "FED_SIGNING_KEY=/etc/fed-e2e/signing.key"
    -e "FED_CA_CERT=/etc/fed-e2e/ca.pem" -e "FED_CA_KEY=/etc/fed-e2e/ca-key.pem"
    -e "FED_CORE_CN=core-$s" -e "FED_CORE_SAN=$(core_ip "$s")"
    -e "FED_ROUTER_DOCKER_HOST=ssh://goat@$(core_ip "$s")"
    -e GOAT_AWS_MODE=none -e GOAT_BIN=/usr/local/bin/goat -e HOME=/root -e GOAT_ANSIBLE_TRUNK_REMOTE=origin
    # The directory on the boundary network (start_directory), trusted through its TLS
    # front's local CA.
    -e FED_DIRECTORY_URL=https://10.77.3.40:8443 -e "FED_COUNTERPARTY_ID=$them"
    -e SSL_CERT_FILE=/etc/fed-e2e/directory-ca.pem
  )
  case "$avenue" in
    goatbuilder) docker exec "${env[@]}" -w /opt/goat "$h" goatbuilder -C /opt/goat --yes federate "$them" --as "$s" --from "$from" --to "$to" ;;
    script)      docker exec "${env[@]}" -w /opt/goat "$h" ops/scripts/peering-standup.sh "$them" --as "$s" --from "$from" --to "$to" ;;
    *) die "unknown avenue $avenue" ;;
  esac
}

# The peering directory: saltlick on the boundary network, its write budget off, and a TLS
# front in its network namespace issuing a certificate for its address from a local CA.
# Each edge trusts that CA through SSL_CERT_FILE.
start_directory() {
  docker run -d --name fed-e2e-directory --network fed-e2e-boundary --ip "$DIRECTORY_IP" \
    -e SALTLICK_WRITE_LIMIT=0 -e SALTLICK_READ_LIMIT=0 "$SALTLICK_IMAGE" >/dev/null
  docker run -d --name fed-e2e-directory-tls --network container:fed-e2e-directory "$DIRECTORY_TLS_IMAGE" \
    caddy reverse-proxy --from "https://$DIRECTORY_IP:8443" --to 127.0.0.1:8080 >/dev/null
  local s
  for _ in $(seq 1 30); do
    if docker cp -q fed-e2e-directory-tls:/data/caddy/pki/authorities/local/root.crt "$WORK/directory-ca.pem" 2>/dev/null; then
      # /etc/fed-e2e on each edge is its keys folder, mounted read-only.
      for s in pasture meadow; do install -m 0644 "$WORK/directory-ca.pem" "$WORK/$s/keys/directory-ca.pem"; done
      docker exec fed-pasture-edge curl -fsS --cacert /etc/fed-e2e/directory-ca.pem "https://$DIRECTORY_IP:8443/healthz" >/dev/null 2>&1 && return 0
    fi
    sleep 1
  done
  die "the peering directory did not answer over TLS at https://$DIRECTORY_IP:8443: $(docker logs fed-e2e-directory 2>&1 | tail -2)"
}

# Phase 5 goes through the directory. The side that ran it first finds the other's entry
# missing and runs it again. Then the voice call, as two operators do it: each writes down
# both verifying keys, read back independently of the directory.
exchange() { # $1 the side that ran phase 5 first, $2 avenue, $3 scenario number
  local first="$1" avenue="$2" n="$3" s them w
  rec "s$n-$first-phase-5-again" --expect-in "Directory step" -- bash -c "$(declare -f drive host_exec other octet core_ip die); drive '$first' '$avenue' 5 5"
  for s in pasture meadow; do
    them="$(other "$s")"; w="/var/lib/goat-federation/$them"
    rec "s$n-$s-the-directory-placed-their-files" -- docker exec "fed-$s-edge" sh -c \
      "test -s $w/theirs.canonical.json && test -s $w/theirs-ca.pem && test -s $w/theirs.spiffe-bundle"
    printf '%s=%s\n%s=%s\n' "$s" "$(cat "$WORK/$s/keys/signing.pub")" "$them" "$(cat "$WORK/$them/keys/signing.pub")" > "$WORK/$s/keys.txt"
    docker cp -q "$WORK/$s/keys.txt" "fed-$s-edge:/var/lib/goat-federation/$them/keys.txt"
  done
}

# Phase 10, as an operator does it: check their CA against the fingerprint in the signed
# agreement, place it, splice the rendered fragment with the real assembler, and restart
# the core on the certificate phase 8 minted.
review_and_admit() { # $1 sovereign
  local s="$1" them w d; them="$(other "$s")"; w="/var/lib/goat-federation/$them"; d="$WORK/$s/applied"
  rm -rf "$d"; mkdir -p "$d"
  docker cp -q "fed-$s-edge:$w/agreement.json" "$WORK/$s/agreement.json"
  docker cp -q "fed-$s-edge:$w/theirs-ca.pem" "$d/theirs-ca.pem"
  local want got
  want="$(python3 -c 'import json,sys; ag=json.load(open(sys.argv[1])); print([p["ca_fingerprint"] for p in ag["parties"] if p["sovereign_id"]==sys.argv[2]][0])' "$WORK/$s/agreement.json" "$them")"
  got="sha256:$(openssl x509 -in "$d/theirs-ca.pem" -outform DER | sha256sum | cut -d' ' -f1)"
  [ "$want" = "$got" ] || die "$s: $them's CA does not match the agreement: $got, agreed $want"
  host_exec "fed-$s-edge" install -m 0644 "$w/theirs-ca.pem" "$w/pki/partner-ca.pem"
  docker cp -q "fed-$s-edge:$w/applied/ops/federation/acl/$them-fragment.json5" "$d/fragment.json5"
  python3 "$ACL_ASSEMBLE" assemble --seed "$WORK/$s/core-seed.json5" --org "$d/fragment.json5" --out "$d/core.json5"
  docker cp -q "$d/core.json5" "fed-$s-core:/etc/goat-core/config.json5"
  docker cp -q "fed-$s-edge:$w/pki/core.pem" "$d/core.pem"
  docker cp -q "fed-$s-edge:$w/pki/core-key.pem" "$d/core-key.pem"
  chmod 0644 "$d/core-key.pem"
  docker cp -q "$d/core.pem" "fed-$s-core:/etc/goat-core/core.pem"
  docker cp -q "$d/core-key.pem" "fed-$s-core:/etc/goat-core/core-key.pem"
  host_exec "fed-$s-core" docker restart "$s-zenoh-router" >/dev/null
}

# A fixed id, because a peer matching no ACL subject is logged by its id alone.
probe_zid() { printf 'f%s' "$(printf '%s-%s' "$1" "$2" | sha256sum | cut -c1-31)"; }

# A client session holding this sovereign's identity, on its edge host, dialling the far
# edge across the tunnel. What the far edge's ACL does with it is the tier-3 evidence.
start_probe() { # $1 sovereign, $2 name, $3 cert, $4 key, $5 rest port
  local s="$1" name="$2" cert="$3" key="$4" port="$5" them w far_ip dir
  them="$(other "$s")"; w="/var/lib/goat-federation/$them"; dir="/etc/fed-probe-$name"
  far_ip="$(python3 -c 'import json,sys,ipaddress as i; ag=json.load(open(sys.argv[1])); h=list(i.ip_network(ag["transport"]["link_subnet"])); ps=[p["sovereign_id"] for p in ag["parties"]]; print(h[ps.index(sys.argv[2])])' "$WORK/$s/agreement.json" "$them")"
  sed "s/__FAR_LINK_IP__/$far_ip/; s/__REST_ADDR__/0.0.0.0/; s/__REST_PORT__/$port/; s/__ZID__/$(probe_zid "$s" "$name")/" "$HERE/probe.json5" > "$WORK/$s/probe-$name.json5"
  host_exec "fed-$s-edge" install -d -m 0755 "$dir"
  docker cp -q "$WORK/$s/probe-$name.json5" "fed-$s-edge:$dir/config.json5"
  host_exec "fed-$s-edge" sh -c "install -m 0644 '$w/pki/partner-ca.pem' '$dir/far-ca.pem'; install -m 0644 '$cert' '$dir/cert.pem'; install -m 0644 '$key' '$dir/key.pem'"
  host_exec "fed-$s-edge" docker rm -f "fed-probe-$name" >/dev/null 2>&1 || true
  host_exec "fed-$s-edge" docker run -d --name "fed-probe-$name" --network host --restart on-failure -v "$dir:/probe:ro" \
    -e RUST_LOG=zenoh=info "$ZENOH_IMAGE" -c /probe/config.json5 >/dev/null
  for _ in $(seq 1 30); do
    curl -s -m 2 -o /dev/null "http://$(edge_ip "$s"):$port/@/**" && return 0
    sleep 2
  done
  host_exec "fed-$s-edge" docker logs "fed-probe-$name" 2>&1 | tail -20 || true
  die "probe $name did not come up on $s"
}

# Phase 14, run by the one party that owns both sides: only a drill has that. The driver's
# own acceptance.sh, unmodified, FED_ACCEPTANCE_STRICT so that no check may be skipped.
accept_as() { # $1 sovereign, $2 avenue
  local s="$1" avenue="$2" them w; them="$(other "$s")"; w="$WORK/$s/fedwork"
  mkdir -p "$w"; cp "$WORK/$s/agreement.json" "$w/agreement.json"
  local env=(
    "FED_WORK=$w" "GOAT_AWS_MODE=none" "FED_ACCEPTANCE_STRICT=1"
    "FED_OURS_REST=http://$(core_ip "$s"):8000" "FED_THEIRS_REST=http://$(core_ip "$them"):8000"
    "FED_EDGE_LOG_OURS=docker exec fed-$s-edge docker logs goat-zenoh-edge-$them"
    "FED_PROBE_THEIRS_REST=http://$(edge_ip "$them"):8101"
    "FED_PROBE_FORGED_REST=http://$(edge_ip "$them"):8102" "FED_PROBE_FORGED_ZID=$(probe_zid "$them" forged)"
    "FED_PROBE_IMPERSONATE_REST=http://$(edge_ip "$them"):8103" "FED_PROBE_IMPERSONATE_ZID=$(probe_zid "$them" impersonate)"
    "FED_SPIRE_OURS=docker exec fed-$s-core docker exec spire-server /opt/spire/bin/spire-server"
    "FED_SPIRE_THEIRS=docker exec fed-$them-core docker exec spire-server /opt/spire/bin/spire-server"
  )
  case "$avenue" in
    goatbuilder) env "${env[@]}" "$GB" -C "$WORK/$s/clone" --yes federate "$them" --as "$s" --work "$w" --from 14 --to 14 ;;
    script)      env "${env[@]}" "$WORK/$s/clone/ops/scripts/peering-standup.sh" "$them" --as "$s" --from 14 --to 14 ;;
  esac
}

scenario() { # $1 number, $2 first side (it also listens), $3 avenue
  local n="$1" first="$2" avenue="$3" second s them; second="$(other "$first")"
  say "scenario $n: $first listens and goes first; avenue $avenue"
  teardown
  docker network create --subnet 10.77.1.0/24 fed-e2e-pasture >/dev/null
  docker network create --subnet 10.77.2.0/24 fed-e2e-meadow >/dev/null
  docker network create --internal --subnet 10.77.3.0/24 fed-e2e-boundary >/dev/null
  for s in pasture meadow; do gen_sovereign "$s"; done
  for s in pasture meadow; do
    start_host "fed-$s-core" "$s" "$(core_ip "$s")"
    start_host "fed-$s-edge" "$s" "$(edge_ip "$s")"
    docker network connect --ip "$(boundary_ip "$s")" fed-e2e-boundary "fed-$s-edge"
  done
  for s in pasture meadow; do wait_ready "fed-$s-core"; wait_ready "fed-$s-edge"; done
  for s in pasture meadow; do start_core "$s"; done
  for s in pasture meadow; do docker save "$RP_TAG" | docker exec -i "fed-$s-core" docker load -q >/dev/null; done
  prepare_edge "$first" true
  prepare_edge "$second" false
  start_directory

  say "scenario $n: the isolation the rest of the run depends on"
  rec "s$n-a-core-cannot-reach-the-boundary" --expect-exit 1 -- docker exec "fed-$first-core" ping -c1 -W2 "$(boundary_ip "$second")"
  rec "s$n-a-core-cannot-reach-the-other-core" --expect-exit 1 -- docker exec "fed-$first-core" ping -c1 -W2 "$(core_ip "$second")"
  rec "s$n-an-edge-cannot-reach-the-other-core" --expect-exit 1 -- docker exec "fed-$first-edge" ping -c1 -W2 "$(core_ip "$second")"
  rec "s$n-the-edges-share-the-boundary" -- docker exec "fed-$first-edge" ping -c1 -W2 "$(boundary_ip "$second")"

  say "scenario $n: phases 1-4, each side, up to the exchange"
  for s in "$first" "$second"; do
    rec "s$n-$s-phases-1-to-5" --expect-in "exchange through the directory" -- bash -c "$(declare -f drive host_exec other octet core_ip die); drive '$s' '$avenue' 1 5"
  done
  exchange "$first" "$avenue" "$n"

  say "scenario $n: phases 6-9, each side, up to the review"
  for s in "$first" "$second"; do
    rec "s$n-$s-phases-6-to-10" --expect-in "review what apply rendered" -- bash -c "$(declare -f drive host_exec other octet core_ip die); drive '$s' '$avenue' 6 10"
  done
  rec "s$n-both-sides-hold-one-agreement" -- bash -c \
    "a=\$(docker exec fed-pasture-edge sha256sum /var/lib/goat-federation/meadow/agreement.json | cut -d' ' -f1); b=\$(docker exec fed-meadow-edge sha256sum /var/lib/goat-federation/pasture/agreement.json | cut -d' ' -f1); echo \"\$a \$b\"; [ -n \"\$a\" ] && [ \"\$a\" = \"\$b\" ]"
  rec "s$n-the-agreement-selected-wireguard" --expect-in '"kind":"wireguard"' -- docker exec fed-pasture-edge cat /var/lib/goat-federation/meadow/agreement.json
  for s in "$first" "$second"; do review_and_admit "$s"; done

  say "scenario $n: phases 11-12, each side: the transport, the edge, the republisher"
  for s in "$first" "$second"; do
    rec "s$n-$s-phases-11-to-12" --expect-in "open the wireguard transport" --expect-in "goat-fed-republisher-$(other "$s") is running" -- bash -c "$(declare -f drive host_exec other octet core_ip die); drive '$s' '$avenue' 11 12"
  done
  # The republisher on each core maps BOTH offered sets onto that side's wire name.
  for s in "$first" "$second"; do
    them="$(other "$s")"
    rec "s$n-$s-republisher-maps-both-sets" --expect-in "release release/all/** -> wire from/$s/all/**" --expect-in "release release/$them/** -> wire from/$s/$them/**" \
      -- docker exec "fed-$s-core" docker logs "goat-fed-republisher-$them"
  done
  rec "s$n-a-wireguard-handshake" -- bash -c \
    "for _ in \$(seq 1 30); do t=\$(docker exec fed-pasture-edge wg show wg-fed0 latest-handshakes | cut -f2); [ \"\${t:-0}\" != 0 ] && break; sleep 2; done; docker exec fed-pasture-edge wg show wg-fed0 latest-handshakes | cut -f2; [ \"\${t:-0}\" != 0 ]"
  rec "s$n-fabric-port-closed-on-the-boundary" --expect-exit 1 -- docker exec "fed-$first-edge" bash -c "exec 3<>/dev/tcp/$(boundary_ip "$second")/7448"

  say "scenario $n: phase 13, each side: the SPIRE trust domains"
  for s in "$first" "$second"; do
    rec "s$n-$s-phase-13" --expect-in "federate the SPIRE trust domains" -- bash -c "$(declare -f drive host_exec other octet core_ip die); drive '$s' '$avenue' 13 13"
  done

  say "scenario $n: phase 14 from both sides, positives and refusals together"
  for s in "$first" "$second"; do
    them="$(other "$s")"
    host_exec "fed-$s-core" sh -c 'cat > /usr/local/bin/fed-responder.py' < "$REPO/ops/federation/roundtrip/responder.py"
    docker exec -d "fed-$s-core" python3 /usr/local/bin/fed-responder.py "from/$them" "release/all"
    start_probe "$s" agreed "/var/lib/goat-federation/$them/pki/peering-outward.pem" "/var/lib/goat-federation/$them/pki/peering-outward-key.pem" 8101
    start_probe "$s" forged /etc/fed-e2e/forged.pem /etc/fed-e2e/forged-key.pem 8102
    start_probe "$s" impersonate /etc/fed-e2e/impersonate.pem /etc/fed-e2e/impersonate-key.pem 8103
  done
  for s in "$first" "$second"; do
    rec "s$n-acceptance-as-$s" --expect-in "acceptance-counts:" --expect-in "skip=0" --expect-in "fail=0" --expect-not-in "ACCEPTANCE FAILED" \
      -- bash -c "$(declare -f accept_as probe_zid other octet core_ip edge_ip die); WORK='$WORK' GB='$GB'; accept_as '$s' '$avenue'"
  done

  say "scenario $n: the failures that make the passes mean something"
  # F-424: a partner authority on a core router IS an authorization grant.
  # Both subjects are printed by the runner's openssl: the host image's openssl
  # may format names differently (3.5 drops the spaces around '=').
  for s in pasture meadow; do
    rec "s$n-$s-core-trusts-only-its-own-ca" -- bash -c \
      "docker exec fed-$s-core sh -c 'grep -c BEGIN /etc/goat-core/ca.pem' | grep -qx 1 && [ \"\$(docker exec fed-$s-core cat /etc/goat-core/ca.pem | openssl x509 -noout -subject)\" = \"\$(openssl x509 -in '$WORK/$s/keys/ca.pem' -noout -subject)\" ]"
    rec "s$n-$s-tunnel-carried-it" -- bash -c \
      "docker exec fed-$s-edge wg show wg-fed0 transfer | awk '{print; if (\$2 < 1 || \$3 < 1) exit 1}'"
  done
  # A round trip that cannot fail is not a check: cut on one side, and it fails.
  local rt=(env "BASE=http://$(core_ip "$first"):8000" "$REPO/ops/federation/roundtrip/fedcheck.sh" "release/all" "from/$second" 15)
  # The republisher is the only writer of our wire name: stop it, and nothing crosses.
  rec "s$n-stop-the-republisher" -- docker exec "fed-$first-core" docker stop "goat-fed-republisher-$second"
  rec "s$n-round-trip-fails-without-the-republisher" --expect-exit 1 --expect-in "FAIL" -- "${rt[@]}"
  rec "s$n-start-the-republisher" -- docker exec "fed-$first-core" docker start "goat-fed-republisher-$second"
  rec "s$n-round-trip-passes-with-the-republisher" --expect-in "PASS" -- bash -c \
    "for _ in 1 2 3 4 5 6; do out=\$(BASE=http://$(core_ip "$first"):8000 '$REPO/ops/federation/roundtrip/fedcheck.sh' release/all from/$second 20) && { echo \"\$out\"; exit 0; }; sleep 10; done; echo \"\$out\"; exit 1"
  rec "s$n-cut-the-link" -- docker exec "fed-$second-edge" systemctl stop goat-peering-wg@wg-fed0
  rec "s$n-round-trip-fails-when-cut" --expect-exit 1 --expect-in "FAIL" -- "${rt[@]}"
  rec "s$n-restore-the-link" -- docker exec "fed-$second-edge" systemctl start goat-peering-wg@wg-fed0
  rec "s$n-round-trip-passes-again" --expect-in "PASS" -- bash -c \
    "for _ in 1 2 3 4 5 6; do out=\$(BASE=http://$(core_ip "$first"):8000 '$REPO/ops/federation/roundtrip/fedcheck.sh' release/all from/$second 20) && { echo \"\$out\"; exit 0; }; sleep 10; done; echo \"\$out\"; exit 1"
  # F-495: the role refuses without a recorded reason, and refuses any other transport.
  for s in pasture meadow; do
    docker exec "fed-$s-edge" cat /var/log/goat-peering-wg-wg-fed0-state.json > "$RECORD/s$n-$s-wg-state.json" 2>&1 || true
  done
  local ans="cd /opt/goat/ops/ansible && GOAT_AWS_MODE=none GOAT_ANSIBLE_TRUNK_REMOTE=origin ../scripts/with-aws none -- ../scripts/ansible-preflight.sh -i inventory/peering-edges.ini --limit ${first}_edge -e fed_edge_group=${first}_edge -e fed_work=/var/lib/goat-federation/$first -e fed_counterparty=/var/lib/goat-federation/$first/theirs.canonical.json"
  rec "s$n-wireguard-refused-without-a-waiver" --expect-exit 2 --expect-in "goat-peering-wg refuses" -- docker exec "fed-$second-edge" bash -c "$ans -e peering_wg_fips_waived_reason= peering-wg.yml"
  rec "s$n-wireguard-refused-under-another-transport" --expect-exit 2 --expect-in "goat-peering-wg refuses" -- docker exec "fed-$second-edge" bash -c "$ans -e goat_peering_transport_kind=ipsec-fips -e peering_wg_kind=ipsec-fips peering-wg.yml"
  # The guards run on what this sovereign generated, in its own repository.
  for g in check-spire-federated-trust-files.sh check-federated-trust-retirement.sh; do
    rec "s$n-guard-${g%.sh}" -- bash -c "docker cp -q fed-pasture-edge:/var/lib/goat-federation/meadow/applied/ops/ansible/group_vars/meadow_edge/. '$WORK/pasture/clone/ops/ansible/group_vars/meadow_edge/' && git -C '$WORK/pasture/clone' add -A && git -C '$WORK/pasture/clone' -c user.name=e2e -c user.email=e2e@localhost commit -q --allow-empty -m generated && '$WORK/pasture/clone/ops/ci/$g'"
  done
  for s in pasture meadow; do
    docker exec "fed-$s-edge" sh -c "docker logs goat-zenoh-edge-$(other "$s") 2>&1 | tail -400" > "$RECORD/s$n-$s-edge-router.log" 2>&1 || true
    docker exec "fed-$s-core" sh -c "docker logs $s-zenoh-router 2>&1 | tail -200" > "$RECORD/s$n-$s-core-router.log" 2>&1 || true
    docker exec "fed-$s-core" sh -c "docker logs spire-server 2>&1 | tail -100" > "$RECORD/s$n-$s-spire.log" 2>&1 || true
    docker exec "fed-$s-core" sh -c "docker logs goat-fed-republisher-$(other "$s") 2>&1 | tail -200" > "$RECORD/s$n-$s-republisher.log" 2>&1 || true
    docker exec "fed-$s-edge" wg show wg-fed0 > "$RECORD/s$n-$s-wg.txt" 2>&1 || true
  done
}

# --- what needs no hosts: the driver's own refusals ------------------------------------
say "the driver's refusals, no hosts"
FX="$REPO/ops/goatbuilder/fixtures/goatnet-from-scratch/03-boundary-node"
SENT="$WORK/mock-ran"; printf '#!/bin/sh\ntouch %s\n' "$SENT" > "$WORK/mock-goat"; chmod +x "$WORK/mock-goat"
rec mock-executes-nothing --expect-in "MOCK" --expect-in "not executed" -- bash -c \
  "GOAT_BIN='$WORK/mock-goat' '$GB' -C '$REPO' --mock '$FX' federate hello-ally --as zagros --work '$WORK/mock-work' && [ ! -e '$SENT' ] && [ ! -e '$WORK/mock-work' ]"
rec an-undeclared-peering-is-refused --expect-in "declare it first (C1)" -- bash -c \
  "if '$GB' -C '$REPO' --mock '$FX' federate no-such-peer --dry-run 2>&1; then exit 1; fi"
mkdir -p "$WORK/quic"; printf '{"transport":{"kind":"quic"}}' > "$WORK/quic/agreement.json"
rec an-unknown-transport-refuses-and-substitutes-nothing --expect-in "REFUSED, no play for 'quic'" --expect-in "Do NOT substitute" --expect-not-in "ansible-preflight" -- bash -c \
  "'$GB' -C '$REPO' --mock '$FX' federate hello-ally --work '$WORK/quic' --from 11 --to 11 --dry-run 2>&1; FED_WORK='$WORK/quic' '$REPO/ops/scripts/peering-standup.sh' hello-ally --from 11 --to 11 2>&1 && exit 1; exit 0"

if [ "$PREPARE" = 1 ]; then
  say "the two sovereigns, generated and planned, no hosts"
  for s in pasture meadow; do
    gen_sovereign "$s"
    rec "prepare-$s-config-validates" -- "$GB" -C "$WORK/$s/clone" validate
    rec "prepare-$s-secrets-posture-is-env" --expect-in "env" --expect-not-in "env-file" -- env -u GOAT_SECRET_STORE python3 "$WORK/$s/clone/ops/secrets/posture.py" store
    rec "prepare-$s-federation-plans" --expect-in "To run it for real" -- "$GB" -C "$WORK/$s/clone" federate "$(other "$s")" --as "$s" --dry-run
    rec "prepare-$s-edge-pins-the-image" --expect-in "$ZENOH_IMAGE" -- cat "$WORK/$s/clone/ops/ansible/group_vars/$(other "$s")_edge/deployment.yml"
  done
  echo "spire image: $SPIRE_IMAGE"
  [ "$FAILS" = 0 ] && exit 0 || exit 1
fi

say "the host image"
ctx="$WORK/host-context"; rm -rf "$ctx"; cp -R "$HERE/host" "$ctx"
# Not -q: a failed build has to say why in the job log.
docker build --progress=plain -t "$HOST_IMAGE" "$ctx"

say "the republisher image, from this checkout"
docker build --progress=plain -t "$RP_TAG" "$REPO/ops/federation/republisher"
RP_IMAGE="$(docker image inspect -f '{{.Id}}' "$RP_TAG")"
[[ "$RP_IMAGE" =~ ^sha256:[0-9a-f]{64}$ ]] || die "the republisher image has no content-addressed id: '$RP_IMAGE'"
echo "republisher image: $RP_IMAGE"

if [ -z "$ONLY" ] || [ "$ONLY" = 1 ]; then scenario 1 pasture goatbuilder; fi
if [ -z "$ONLY" ] || [ "$ONLY" = 2 ]; then scenario 2 meadow script; fi

say "the record"
{
  grep -h "acceptance-counts:" "$RECORD"/*acceptance-as-*.log || true
} | tee "$RECORD/acceptance-counts.txt"
# The record step is a step: a report that cannot be written fails the run.
python3 "$REPO/ops/e2e/report.py" "$RECORD" --check --title "cross-sovereign federation e2e (hermetic)" || { echo "run.sh: the run record did not validate (above)" >&2; FAILS=$((FAILS + 1)); }
echo "record: $RECORD"
[ "$FAILS" = 0 ] && exit 0 || exit 1
