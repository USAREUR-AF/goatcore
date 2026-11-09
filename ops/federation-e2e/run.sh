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
# Three scenarios:
#   1  pasture listens and goes first, both sides through `goatbuilder federate`
#   2  meadow listens and goes first, both sides through peering-standup.sh
#   3  THREE parties. pasture federates with meadow and with a third sovereign, paddock,
#      at the same time, and its two peerings share ONE edge host and ONE core host.
#      pasture <-> meadow goes through `goatbuilder federate` and the directory;
#      pasture <-> paddock is added afterwards through peering-standup.sh with the
#      packages exchanged by hand, because the directory holds one package per party.
#      Proved: each peering carries and refuses per its own agreement, neither partner
#      receives the other's bilateral set, adding or re-running one peering leaves the
#      other untouched, and the refusals that keep two peerings on one host apart fire.
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
#                                  [--scenario 1|2|3] [--keep] [--prepare-only]
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
    -h|--help) sed -n '2,43p' "$0"; exit 0 ;;
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
octet() { case "$1" in pasture) echo 1 ;; meadow) echo 2 ;; paddock) echo 4 ;; esac; }
core_ip() { echo "10.77.$(octet "$1").10"; }
edge_ip() { echo "10.77.$(octet "$1").20"; }
boundary_ip() { case "$1" in pasture) echo 10.77.3.11 ;; meadow) echo 10.77.3.12 ;; paddock) echo 10.77.3.13 ;; esac; }
td() { echo "$1.fed.test"; }

# EVERY PER-PEERING VALUE IS A FUNCTION OF THE PAIR. A party's partners, in order, are
# P_<party>; its first peering keeps every role default, and each later one gets the
# values two peerings cannot share on one edge host and one core host.
P_pasture=""; P_meadow=""; P_paddock=""; NODIR=""
partners() { local v="P_$1"; echo "${!v}"; }
pidx() { # $1 us, $2 them: the position of the peering among ours
  local i=0 t
  for t in $(partners "$1"); do [ "$t" = "$2" ] && { echo "$i"; return 0; }; i=$((i + 1)); done
  echo "run.sh: $2 is not a partner of $1" >&2; return 1
}
iface() { echo "wg-fed$(pidx "$1" "$2")"; }
wgport() { echo $((51820 + $(pidx "$1" "$2"))); }
spire_out() { echo $((18443 + $(pidx "$1" "$2"))); }
rp_metrics() { echo $((9310 + $(pidx "$1" "$2"))); }
probe_port() { echo $(($3 + 10 * $(pidx "$1" "$2"))); } # $3 the base: 8101 agreed, 8102 forged, 8103 impersonate
ctx_of() { [ "$(pidx "$1" "$2")" = 0 ] && echo default || echo "$2"; }
pair_subnet() {
  case "$(printf '%s\n' "$1" "$2" | sort | tr '\n' ' ')" in
    "meadow pasture ") echo 169.254.77.0/31 ;;
    "paddock pasture ") echo 169.254.78.0/31 ;;
    *) echo 169.254.79.0/31 ;;
  esac
}
nodir() { case " $NODIR " in *" $1:$2 "*) return 0 ;; *) return 1 ;; esac; }
partner_list() { # the parties in this scenario
  local s; for s in pasture meadow paddock; do [ -n "$(partners "$s")" ] && echo "$s"; done; return 0
}

NETS=(fed-e2e-pasture fed-e2e-meadow fed-e2e-paddock fed-e2e-boundary)
HOSTS=(fed-pasture-core fed-pasture-edge fed-meadow-core fed-meadow-edge fed-paddock-core fed-paddock-edge)
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
  # The name that matters most: this CA minting the FAR core's common name, one per partner.
  local t
  for t in $(partners "$s"); do
    openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -keyout "$d/keys/impersonate-$t-key.pem" \
      -out "$d/keys/impersonate-$t.csr" -subj "/CN=core-$t" 2>/dev/null
    printf 'subjectAltName=DNS:core-%s\nbasicConstraints=CA:FALSE\nextendedKeyUsage=clientAuth\n' "$t" > "$d/keys/impersonate-$t.ext"
    openssl x509 -req -in "$d/keys/impersonate-$t.csr" -CA "$d/keys/ca.pem" -CAkey "$d/keys/ca-key.pem" \
      -CAcreateserial -out "$d/keys/impersonate-$t.pem" -days 30 -extfile "$d/keys/impersonate-$t.ext" 2>/dev/null
  done
  ssh-keygen -q -t ed25519 -N '' -f "$d/keys/id_ed25519" -C "fed-e2e-$s"
  chmod 0644 "$d/keys/"*.pem "$d/keys/signing.key" "$d/keys/id_ed25519"
  # Its own repository, as two organisations would hold.
  git clone -q --depth 1 --single-branch "file://$REPO" "$d/clone"
  git -C "$d/clone" checkout -q -B main
  git -C "$d/clone" config user.name "$s operator"; git -C "$d/clone" config user.email "ops@$s.test"
  local also=() first; first="$(partners "$s" | awk '{print $1}')"
  for t in $(partners "$s"); do
    [ "$t" = "$first" ] && continue
    also+=(--also "$t:$(iface "$s" "$t"):$(wgport "$s" "$t"):$(spire_out "$s" "$t"):$(rp_metrics "$s" "$t")")
  done
  python3 "$HERE/overlay.py" "$d/clone" "$s" "$first" "$(octet "$s")" "$ZENOH_IMAGE" --republisher-image "$RP_IMAGE" "${also[@]}"
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
  local s="$1" h="fed-$1-core" them; them="$(partners "$s" | awk '{print $1}')"
  sed "s/__THEM__/$them/g; s/__US__/$s/g; s/__CORE_LISTEN__/0.0.0.0/; s/__CORE_REST__/0.0.0.0/" "$HERE/core-seed.json5" > "$WORK/$s/core-seed.json5"
  # A later partner's landing and not-granted prefixes are held the same way.
  python3 - "$WORK/$s/core-seed.json5" $(partners "$s" | cut -s -d' ' -f2-) <<'PY'
import sys
path, extra = sys.argv[1], sys.argv[2:]
out = []
for ln in open(path).read().split("\n"):
    out.append(ln)
    if ln.strip().startswith("refused_prefix:"):
        pad = ln[: len(ln) - len(ln.lstrip())]
        for t in extra:
            out.append(f'{pad}landing_{t}: {{ key_expr: "from/{t}/**", volume: "memory" }},')
            out.append(f'{pad}refused_prefix_{t}: {{ key_expr: "from/{t}-not-granted/**", volume: "memory" }},')
open(path, "w").write("\n".join(out))
PY
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

prepare_edge() { # $1 sovereign, $2 them, $3 "true" when this side listens
  # Each peering's declaration lives in its own goat context: the first in `default`,
  # a later one in a context named for the partner, selected with GOAT_CONTEXT.
  local s="$1" them="$2" listener="$3" h="fed-$1-edge" p
  host_exec "$h" sh -c "[ -d /opt/goat/.git ] || git clone -q /srv/goat.git /opt/goat"
  host_exec "$h" sh -c "git -C /opt/goat config user.name '$s operator'; git -C /opt/goat config user.email 'ops@$s.test'"
  p="/root/.goat/contexts/$(ctx_of "$s" "$them")"
  host_exec "$h" install -d -m 0700 /root/.goat /root/.goat/contexts "$p"
  host_exec "$h" install -m 0644 /etc/fed-e2e/profile.pem "$p/mtls.cert.pem"
  host_exec "$h" install -m 0644 /etc/fed-e2e/ca.pem "$p/ca-roots.pem"
  cat > "$WORK/$s/federation.toml" <<TOML
sovereign_id = "$s"
term_days    = 30
cut_contact  = "fed-ops@$s.test"

[transport]
interface            = "$(iface "$s" "$them")"
endpoint             = "$(boundary_ip "$s"):$(wgport "$s" "$them")"
listener             = $listener
proposed_link_subnet = "$(pair_subnet "$s" "$them")"
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
drive() { # $1 sovereign, $2 them, $3 avenue, $4 from, $5 to
  local s="$1" them="$2" avenue="$3" from="$4" to="$5" h="fed-$1-edge"
  local env=(
    -e "GOAT_CONTEXT=$(ctx_of "$s" "$them")"
    -e "FED_WORK=/var/lib/goat-federation/$them"
    -e "FED_SIGNING_KEY=/etc/fed-e2e/signing.key"
    -e "FED_CA_CERT=/etc/fed-e2e/ca.pem" -e "FED_CA_KEY=/etc/fed-e2e/ca-key.pem"
    -e "FED_CORE_CN=core-$s" -e "FED_CORE_SAN=$(core_ip "$s")"
    -e "FED_ROUTER_DOCKER_HOST=ssh://goat@$(core_ip "$s")"
    -e GOAT_AWS_MODE=none -e GOAT_BIN=/usr/local/bin/goat -e HOME=/root -e GOAT_ANSIBLE_TRUNK_REMOTE=origin
    # The directory on the boundary network (start_directory), trusted through its TLS
    # front's local CA.
    -e "FED_COUNTERPARTY_ID=$them" -e SSL_CERT_FILE=/etc/fed-e2e/directory-ca.pem
  )
  # A party has one entry in the directory. A peering added beside one composed through
  # it exchanges its packages by hand, so the entry the first partner composed against
  # stays the one it names.
  if nodir "$s" "$them"; then env+=(-e FED_NO_DIRECTORY=1); else env+=(-e FED_DIRECTORY_URL=https://10.77.3.40:8443); fi
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
      for s in $(partner_list); do install -m 0644 "$WORK/directory-ca.pem" "$WORK/$s/keys/directory-ca.pem"; done
      docker exec fed-pasture-edge curl -fsS --cacert /etc/fed-e2e/directory-ca.pem "https://$DIRECTORY_IP:8443/healthz" >/dev/null 2>&1 && return 0
    fi
    sleep 1
  done
  die "the peering directory did not answer over TLS at https://$DIRECTORY_IP:8443: $(docker logs fed-e2e-directory 2>&1 | tail -2)"
}

# Phase 5 goes through the directory. The side that ran it first finds the other's entry
# missing and runs it again. Then the voice call, as two operators do it: each writes down
# both verifying keys, read back independently of the directory.
exchange() { # $1 the side that ran phase 5 first, $2 the other side, $3 avenue, $4 scenario number
  local first="$1" second="$2" avenue="$3" n="$4" s them w
  if nodir "$first" "$second"; then
    # By hand, as FED_NO_DIRECTORY=1 tells each operator: our signed package, our CA
    # certificate and our SPIFFE bundle to them, theirs to us. Compose still checks
    # their package against the key read back on the call.
    for s in "$first" "$second"; do
      them="$([ "$s" = "$first" ] && echo "$second" || echo "$first")"
      docker cp -q "fed-$s-edge:/var/lib/goat-federation/$them/ours.canonical.json" "$WORK/$s/handover-$them.json"
      docker cp -q "$WORK/$s/handover-$them.json" "fed-$them-edge:/var/lib/goat-federation/$s/theirs.canonical.json"
      docker cp -q "$WORK/$s/keys/ca.pem" "fed-$them-edge:/var/lib/goat-federation/$s/theirs-ca.pem"
      docker cp -q "fed-$s-edge:/var/lib/goat-federation/$them/ours.spiffe-bundle" "$WORK/$s/handover-$them.bundle"
      docker cp -q "$WORK/$s/handover-$them.bundle" "fed-$them-edge:/var/lib/goat-federation/$s/theirs.spiffe-bundle"
    done
  else
    rec "s$n-$first-$second-phase-5-again" --expect-in "Directory step" -- bash -c "drive '$first' '$second' '$avenue' 5 5"
  fi
  for s in "$first" "$second"; do
    them="$([ "$s" = "$first" ] && echo "$second" || echo "$first")"; w="/var/lib/goat-federation/$them"
    rec "s$n-$s-$them-their-files-are-in-place" -- docker exec "fed-$s-edge" sh -c \
      "test -s $w/theirs.canonical.json && test -s $w/theirs-ca.pem && test -s $w/theirs.spiffe-bundle"
    printf '%s=%s\n%s=%s\n' "$s" "$(cat "$WORK/$s/keys/signing.pub")" "$them" "$(cat "$WORK/$them/keys/signing.pub")" > "$WORK/$s/keys.txt"
    docker cp -q "$WORK/$s/keys.txt" "fed-$s-edge:/var/lib/goat-federation/$them/keys.txt"
  done
}

# Phase 10, as an operator does it: check their CA against the fingerprint in the signed
# agreement, place it, splice the rendered fragment with the real assembler, and restart
# the core on the certificate phase 8 minted.
review_and_admit() { # $1 sovereign, $2 the peering being admitted
  local s="$1" them="$2" w d t; w="/var/lib/goat-federation/$them"; d="$WORK/$s/applied-$them"
  rm -rf "$d"; mkdir -p "$d"
  docker cp -q "fed-$s-edge:$w/agreement.json" "$WORK/$s/agreement-$them.json"
  docker cp -q "fed-$s-edge:$w/theirs-ca.pem" "$d/theirs-ca.pem"
  local want got
  want="$(python3 -c 'import json,sys; ag=json.load(open(sys.argv[1])); print([p["ca_fingerprint"] for p in ag["parties"] if p["sovereign_id"]==sys.argv[2]][0])' "$WORK/$s/agreement-$them.json" "$them")"
  got="sha256:$(openssl x509 -in "$d/theirs-ca.pem" -outform DER | sha256sum | cut -d' ' -f1)"
  [ "$want" = "$got" ] || die "$s: $them's CA does not match the agreement: $got, agreed $want"
  host_exec "fed-$s-edge" install -m 0644 "$w/theirs-ca.pem" "$w/pki/partner-ca.pem"
  # The core admits every peering admitted so far: one fragment per partner, reviewed
  # here, joined region by region (acl-assemble.py reads a repeated region header as
  # more of the same region).
  : > "$d/fragment.json5"
  for t in $(partners "$s"); do
    if docker exec "fed-$s-edge" test -s "/var/lib/goat-federation/$t/applied/ops/federation/acl/$t-fragment.json5"; then
      docker cp -q "fed-$s-edge:/var/lib/goat-federation/$t/applied/ops/federation/acl/$t-fragment.json5" "$d/fragment-$t.json5"
      cat "$d/fragment-$t.json5" >> "$d/fragment.json5"
    fi
  done
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
start_probe() { # $1 sovereign, $2 them, $3 name, $4 cert, $5 key, $6 rest port
  local s="$1" them="$2" name="$3" cert="$4" key="$5" port="$6" w far_ip dir
  w="/var/lib/goat-federation/$them"; dir="/etc/fed-probe-$them-$name"
  far_ip="$(python3 -c 'import json,sys,ipaddress as i; ag=json.load(open(sys.argv[1])); h=list(i.ip_network(ag["transport"]["link_subnet"])); ps=[p["sovereign_id"] for p in ag["parties"]]; print(h[ps.index(sys.argv[2])])' "$WORK/$s/agreement-$them.json" "$them")"
  sed "s/__FAR_LINK_IP__/$far_ip/; s/__REST_ADDR__/0.0.0.0/; s/__REST_PORT__/$port/; s/__ZID__/$(probe_zid "$s-$them" "$name")/" "$HERE/probe.json5" > "$WORK/$s/probe-$them-$name.json5"
  host_exec "fed-$s-edge" install -d -m 0755 "$dir"
  docker cp -q "$WORK/$s/probe-$them-$name.json5" "fed-$s-edge:$dir/config.json5"
  host_exec "fed-$s-edge" sh -c "install -m 0644 '$w/pki/partner-ca.pem' '$dir/far-ca.pem'; install -m 0644 '$cert' '$dir/cert.pem'; install -m 0644 '$key' '$dir/key.pem'"
  host_exec "fed-$s-edge" docker rm -f "fed-probe-$them-$name" >/dev/null 2>&1 || true
  host_exec "fed-$s-edge" docker run -d --name "fed-probe-$them-$name" --network host --restart on-failure -v "$dir:/probe:ro" \
    -e RUST_LOG=zenoh=info "$ZENOH_IMAGE" -c /probe/config.json5 >/dev/null
  for _ in $(seq 1 30); do
    curl -s -m 2 -o /dev/null "http://$(edge_ip "$s"):$port/@/**" && return 0
    sleep 2
  done
  host_exec "fed-$s-edge" docker logs "fed-probe-$them-$name" 2>&1 | tail -20 || true
  die "probe $name toward $them did not come up on $s"
}

# Phase 14, run by the one party that owns both sides: only a drill has that. The driver's
# own acceptance.sh, unmodified, FED_ACCEPTANCE_STRICT so that no check may be skipped.
accept_as() { # $1 sovereign, $2 them, $3 avenue
  local s="$1" them="$2" avenue="$3" w; w="$WORK/$s/fedwork-$them"
  mkdir -p "$w"; cp "$WORK/$s/agreement-$them.json" "$w/agreement.json"
  local env=(
    "FED_WORK=$w" "GOAT_AWS_MODE=none" "FED_ACCEPTANCE_STRICT=1"
    "FED_OURS_REST=http://$(core_ip "$s"):8000" "FED_THEIRS_REST=http://$(core_ip "$them"):8000"
    "FED_EDGE_LOG_OURS=docker exec fed-$s-edge docker logs goat-zenoh-edge-$them"
    "FED_PROBE_THEIRS_REST=http://$(edge_ip "$them"):$(probe_port "$them" "$s" 8101)"
    "FED_PROBE_FORGED_REST=http://$(edge_ip "$them"):$(probe_port "$them" "$s" 8102)" "FED_PROBE_FORGED_ZID=$(probe_zid "$them-$s" forged)"
    "FED_PROBE_IMPERSONATE_REST=http://$(edge_ip "$them"):$(probe_port "$them" "$s" 8103)" "FED_PROBE_IMPERSONATE_ZID=$(probe_zid "$them-$s" impersonate)"
    "FED_SPIRE_OURS=docker exec fed-$s-core docker exec spire-server /opt/spire/bin/spire-server"
    "FED_SPIRE_THEIRS=docker exec fed-$them-core docker exec spire-server /opt/spire/bin/spire-server"
  )
  case "$avenue" in
    goatbuilder) env "${env[@]}" "$GB" -C "$WORK/$s/clone" --yes federate "$them" --as "$s" --work "$w" --from 14 --to 14 ;;
    script)      env "${env[@]}" "$WORK/$s/clone/ops/scripts/peering-standup.sh" "$them" --as "$s" --from 14 --to 14 ;;
  esac
}

# The helpers every `rec ... bash -c` runs in a child shell.
export_helpers() {
  export -f drive host_exec other octet core_ip edge_ip boundary_ip die partners pidx iface wgport \
    spire_out rp_metrics probe_port ctx_of pair_subnet nodir partner_list accept_as probe_zid td round_trip round_trip_eventually
  export P_pasture P_meadow P_paddock NODIR WORK GB REPO HERE
}

# The hosts of every party in this scenario, the directory, and the isolation checks.
setup_hosts() { # $1 scenario number
  local n="$1" s a b
  teardown
  docker network create --subnet 10.77.1.0/24 fed-e2e-pasture >/dev/null
  docker network create --subnet 10.77.2.0/24 fed-e2e-meadow >/dev/null
  docker network create --subnet 10.77.4.0/24 fed-e2e-paddock >/dev/null
  docker network create --internal --subnet 10.77.3.0/24 fed-e2e-boundary >/dev/null
  for s in $(partner_list); do gen_sovereign "$s"; done
  for s in $(partner_list); do
    start_host "fed-$s-core" "$s" "$(core_ip "$s")"
    start_host "fed-$s-edge" "$s" "$(edge_ip "$s")"
    docker network connect --ip "$(boundary_ip "$s")" fed-e2e-boundary "fed-$s-edge"
  done
  for s in $(partner_list); do wait_ready "fed-$s-core"; wait_ready "fed-$s-edge"; done
  for s in $(partner_list); do start_core "$s"; done
  for s in $(partner_list); do docker save "$RP_TAG" | docker exec -i "fed-$s-core" docker load -q >/dev/null; done
  for s in $(partner_list); do host_exec "fed-$s-edge" sh -c "[ -d /opt/goat/.git ] || git clone -q /srv/goat.git /opt/goat"; done
  start_directory

  say "scenario $n: the isolation the rest of the run depends on"
  for a in $(partner_list); do
    for b in $(partner_list); do
      [ "$a" = "$b" ] && continue
      rec "s$n-$a-core-cannot-reach-$b-boundary" --expect-exit 1 -- docker exec "fed-$a-core" ping -c1 -W2 "$(boundary_ip "$b")"
      rec "s$n-$a-core-cannot-reach-$b-core" --expect-exit 1 -- docker exec "fed-$a-core" ping -c1 -W2 "$(core_ip "$b")"
      rec "s$n-$a-edge-cannot-reach-$b-core" --expect-exit 1 -- docker exec "fed-$a-edge" ping -c1 -W2 "$(core_ip "$b")"
      rec "s$n-$a-edge-shares-the-boundary-with-$b" -- docker exec "fed-$a-edge" ping -c1 -W2 "$(boundary_ip "$b")"
    done
  done
}

# One peering, phases 1-14 from both sides. $1 scenario, $2 the side that listens and goes
# first, $3 the other side, $4 avenue.
peer_up() {
  local n="$1" first="$2" second="$3" avenue="$4" s them i probes
  i="$(iface "$first" "$second")"
  prepare_edge "$first" "$second" true
  prepare_edge "$second" "$first" false
  local want5="exchange through the directory"; nodir "$first" "$second" && want5="exchange packages by hand"

  say "scenario $n: $first <-> $second phases 1-4, each side, up to the exchange ($avenue)"
  for s in "$first" "$second"; do
    them="$([ "$s" = "$first" ] && echo "$second" || echo "$first")"
    rec "s$n-$s-$them-phases-1-to-5" --expect-in "$want5" -- bash -c "drive '$s' '$them' '$avenue' 1 5"
  done
  exchange "$first" "$second" "$avenue" "$n"

  say "scenario $n: $first <-> $second phases 6-9, each side, up to the review"
  for s in "$first" "$second"; do
    them="$([ "$s" = "$first" ] && echo "$second" || echo "$first")"
    rec "s$n-$s-$them-phases-6-to-10" --expect-in "review what apply rendered" -- bash -c "drive '$s' '$them' '$avenue' 6 10"
  done
  rec "s$n-$first-$second-both-sides-hold-one-agreement" -- bash -c \
    "a=\$(docker exec fed-$first-edge sha256sum /var/lib/goat-federation/$second/agreement.json | cut -d' ' -f1); b=\$(docker exec fed-$second-edge sha256sum /var/lib/goat-federation/$first/agreement.json | cut -d' ' -f1); echo \"\$a \$b\"; [ -n \"\$a\" ] && [ \"\$a\" = \"\$b\" ]"
  rec "s$n-$first-$second-the-agreement-selected-wireguard" --expect-in '"kind":"wireguard"' -- docker exec "fed-$first-edge" cat "/var/lib/goat-federation/$second/agreement.json"
  review_and_admit "$first" "$second"
  review_and_admit "$second" "$first"

  say "scenario $n: $first <-> $second phases 11-12, each side: the transport, the edge, the republisher"
  for s in "$first" "$second"; do
    them="$([ "$s" = "$first" ] && echo "$second" || echo "$first")"
    rec "s$n-$s-$them-phases-11-to-12" --expect-in "open the wireguard transport" --expect-in "goat-fed-republisher-$them is running" -- bash -c "drive '$s' '$them' '$avenue' 11 12"
    rec "s$n-$s-$them-republisher-maps-both-sets" --expect-in "release release/all/** -> wire from/$s/all/**" --expect-in "release release/$them/** -> wire from/$s/$them/**" \
      -- docker exec "fed-$s-core" docker logs "goat-fed-republisher-$them"
  done
  rec "s$n-$first-$second-a-wireguard-handshake" -- bash -c \
    "for _ in \$(seq 1 30); do t=\$(docker exec fed-$first-edge wg show $i latest-handshakes | cut -f2); [ \"\${t:-0}\" != 0 ] && break; sleep 2; done; docker exec fed-$first-edge wg show $i latest-handshakes | cut -f2; [ \"\${t:-0}\" != 0 ]"
  rec "s$n-$first-$second-fabric-port-closed-on-the-boundary" --expect-exit 1 -- docker exec "fed-$first-edge" bash -c "exec 3<>/dev/tcp/$(boundary_ip "$second")/7448"

  say "scenario $n: $first <-> $second phase 13, each side: the SPIRE trust domains"
  for s in "$first" "$second"; do
    them="$([ "$s" = "$first" ] && echo "$second" || echo "$first")"
    rec "s$n-$s-$them-phase-13" --expect-in "federate the SPIRE trust domains" -- bash -c "drive '$s' '$them' '$avenue' 13 13"
  done

  say "scenario $n: $first <-> $second phase 14 from both sides, positives and refusals together"
  for s in "$first" "$second"; do
    them="$([ "$s" = "$first" ] && echo "$second" || echo "$first")"
    host_exec "fed-$s-core" sh -c 'cat > /usr/local/bin/fed-responder.py' < "$REPO/ops/federation/roundtrip/responder.py"
    docker exec -d "fed-$s-core" python3 /usr/local/bin/fed-responder.py "from/$them" "release/all"
    start_probe "$s" "$them" agreed "/var/lib/goat-federation/$them/pki/peering-outward.pem" "/var/lib/goat-federation/$them/pki/peering-outward-key.pem" "$(probe_port "$s" "$them" 8101)"
    start_probe "$s" "$them" forged /etc/fed-e2e/forged.pem /etc/fed-e2e/forged-key.pem "$(probe_port "$s" "$them" 8102)"
    start_probe "$s" "$them" impersonate "/etc/fed-e2e/impersonate-$them.pem" "/etc/fed-e2e/impersonate-$them-key.pem" "$(probe_port "$s" "$them" 8103)"
  done
  accept_pair "$n" "$first" "$second" "$avenue" ""
}

accept_pair() { # $1 scenario, $2 a, $3 b, $4 avenue, $5 label suffix
  local n="$1" a="$2" b="$3" avenue="$4" sfx="$5" s them
  for s in "$a" "$b"; do
    them="$([ "$s" = "$a" ] && echo "$b" || echo "$a")"
    rec "s$n-acceptance-as-$s-with-$them$sfx" --expect-in "acceptance-counts:" --expect-in "skip=0" --expect-in "fail=0" --expect-not-in "ACCEPTANCE FAILED" \
      -- bash -c "accept_as '$s' '$them' '$avenue'"
  done
}

round_trip() { # $1 sender, $2 receiver, $3 seconds: one try
  BASE="http://$(core_ip "$1"):8000" "$REPO/ops/federation/roundtrip/fedcheck.sh" release/all "from/$2" "$3"
}
round_trip_eventually() { # $1 sender, $2 receiver
  local out
  for _ in 1 2 3 4 5 6; do out="$(round_trip "$1" "$2" 20)" && { echo "$out"; return 0; }; sleep 10; done
  echo "$out"; return 1
}

# The failures that make the passes mean something, for a two-party scenario.
failures() { # $1 scenario, $2 first, $3 second
  local n="$1" first="$2" second="$3" s them i
  i="$(iface "$first" "$second")"
  # F-424: a partner authority on a core router IS an authorization grant.
  # Both subjects are printed by the runner's openssl: the host image's openssl
  # may format names differently (3.5 drops the spaces around '=').
  for s in "$first" "$second"; do
    them="$([ "$s" = "$first" ] && echo "$second" || echo "$first")"
    rec "s$n-$s-core-trusts-only-its-own-ca" -- bash -c \
      "docker exec fed-$s-core sh -c 'grep -c BEGIN /etc/goat-core/ca.pem' | grep -qx 1 && [ \"\$(docker exec fed-$s-core cat /etc/goat-core/ca.pem | openssl x509 -noout -subject)\" = \"\$(openssl x509 -in '$WORK/$s/keys/ca.pem' -noout -subject)\" ]"
    rec "s$n-$s-tunnel-carried-it" -- bash -c \
      "docker exec fed-$s-edge wg show $(iface "$s" "$them") transfer | awk '{print; if (\$2 < 1 || \$3 < 1) exit 1}'"
  done
  # A round trip that cannot fail is not a check: cut on one side, and it fails.
  # The republisher is the only writer of our wire name: stop it, and nothing crosses.
  rec "s$n-stop-the-republisher" -- docker exec "fed-$first-core" docker stop "goat-fed-republisher-$second"
  rec "s$n-round-trip-fails-without-the-republisher" --expect-exit 1 --expect-in "FAIL" -- bash -c "round_trip '$first' '$second' 15"
  rec "s$n-start-the-republisher" -- docker exec "fed-$first-core" docker start "goat-fed-republisher-$second"
  rec "s$n-round-trip-passes-with-the-republisher" --expect-in "PASS" -- bash -c "round_trip_eventually '$first' '$second'"
  rec "s$n-cut-the-link" -- docker exec "fed-$second-edge" systemctl stop "goat-peering-wg@$(iface "$second" "$first")"
  rec "s$n-round-trip-fails-when-cut" --expect-exit 1 --expect-in "FAIL" -- bash -c "round_trip '$first' '$second' 15"
  rec "s$n-restore-the-link" -- docker exec "fed-$second-edge" systemctl start "goat-peering-wg@$(iface "$second" "$first")"
  rec "s$n-round-trip-passes-again" --expect-in "PASS" -- bash -c "round_trip_eventually '$first' '$second'"
  # F-495: the role refuses without a recorded reason, and refuses any other transport.
  for s in "$first" "$second"; do
    them="$([ "$s" = "$first" ] && echo "$second" || echo "$first")"
    docker exec "fed-$s-edge" cat "/var/log/goat-peering-wg-$(iface "$s" "$them")-state.json" > "$RECORD/s$n-$s-$them-wg-state.json" 2>&1 || true
  done
  local ans="cd /opt/goat/ops/ansible && GOAT_AWS_MODE=none GOAT_ANSIBLE_TRUNK_REMOTE=origin ../scripts/with-aws none -- ../scripts/ansible-preflight.sh -i inventory/peering-edges.ini --limit ${first}_edge -e fed_edge_group=${first}_edge -e fed_work=/var/lib/goat-federation/$first -e fed_counterparty=/var/lib/goat-federation/$first/theirs.canonical.json"
  rec "s$n-wireguard-refused-without-a-waiver" --expect-exit 2 --expect-in "goat-peering-wg refuses" -- docker exec "fed-$second-edge" bash -c "$ans -e peering_wg_fips_waived_reason= peering-wg.yml"
  rec "s$n-wireguard-refused-under-another-transport" --expect-exit 2 --expect-in "goat-peering-wg refuses" -- docker exec "fed-$second-edge" bash -c "$ans -e goat_peering_transport_kind=ipsec-fips -e peering_wg_kind=ipsec-fips peering-wg.yml"
  # The guards run on what this sovereign generated, in its own repository.
  for g in check-spire-federated-trust-files.sh check-federated-trust-retirement.sh; do
    rec "s$n-guard-${g%.sh}" -- bash -c "docker cp -q fed-pasture-edge:/var/lib/goat-federation/meadow/applied/ops/ansible/group_vars/meadow_edge/. '$WORK/pasture/clone/ops/ansible/group_vars/meadow_edge/' && git -C '$WORK/pasture/clone' add -A && git -C '$WORK/pasture/clone' -c user.name=e2e -c user.email=e2e@localhost commit -q --allow-empty -m generated && '$WORK/pasture/clone/ops/ci/$g'"
  done
}

collect() { # $1 scenario
  local n="$1" s them
  for s in $(partner_list); do
    docker exec "fed-$s-core" sh -c "docker logs $s-zenoh-router 2>&1 | tail -200" > "$RECORD/s$n-$s-core-router.log" 2>&1 || true
    docker exec "fed-$s-core" sh -c "docker logs spire-server 2>&1 | tail -100" > "$RECORD/s$n-$s-spire.log" 2>&1 || true
    for them in $(partners "$s"); do
      docker exec "fed-$s-edge" sh -c "docker logs goat-zenoh-edge-$them 2>&1 | tail -400" > "$RECORD/s$n-$s-$them-edge-router.log" 2>&1 || true
      docker exec "fed-$s-core" sh -c "docker logs goat-fed-republisher-$them 2>&1 | tail -200" > "$RECORD/s$n-$s-$them-republisher.log" 2>&1 || true
      docker exec "fed-$s-edge" wg show "$(iface "$s" "$them")" > "$RECORD/s$n-$s-$them-wg.txt" 2>&1 || true
    done
  done
}

scenario() { # $1 number, $2 first side (it also listens), $3 avenue
  local n="$1" first="$2" avenue="$3" second; second="$(other "$first")"
  P_pasture=meadow; P_meadow=pasture; P_paddock=""; NODIR=""
  export_helpers
  say "scenario $n: $first listens and goes first; avenue $avenue"
  setup_hosts "$n"
  peer_up "$n" "$first" "$second" "$avenue"
  say "scenario $n: the failures that make the passes mean something"
  failures "$n" "$first" "$second"
  collect "$n"
}

# --- scenario 3: one party, two peerings, one edge host and one core host ---------------

# What a peering owns on the shared hosts, printed so that two prints compare: its
# interface (key, port, peer, address), its firewall table without counters, its files,
# its containers' start times and images, and every SPIRE forward unit that is not named
# for the peering being changed. $1 us, $2 the peering to print, $3 the peering being changed.
snap() {
  local s="$1" them="$2" changing="$3" i t
  i="$(iface "$s" "$them")"; t="goat_peering_$(echo "$i" | tr -c 'a-zA-Z0-9_\n' _)"
  docker exec "fed-$s-edge" bash -c "
    wg show $i dump | awk 'NR==1{print \"iface\", \$2, \$3} NR>1{print \"peer\", \$1, \$3, \$4, \$8}'
    ip -o -4 addr show dev $i | awk '{print \"addr\", \$4}'
    nft list table inet $t | sed -E 's/counter packets [0-9]+ bytes [0-9]+/counter/'
    sha256sum /etc/goat-peering-wg/$i.env /etc/goat-peering-wg/$i.nft /etc/goat-zenoh-edge-$them/config.json5 /opt/goat-zenoh-edge-$them/docker-compose.yml
    docker inspect -f '{{.Name}} {{.State.StartedAt}} {{.Image}}' goat-zenoh-edge-$them
    for u in /etc/systemd/system/goat-peering-bundle-*; do
      case \"\$u\" in *-$changing.*) continue ;; esac
      [ -e \"\$u\" ] || continue
      sha256sum \"\$u\"; systemctl show -p ActiveEnterTimestamp -p ActiveState \"\$(basename \"\$u\")\"
    done" 2>&1
  docker exec "fed-$s-core" bash -c "
    docker inspect -f '{{.Name}} {{.State.StartedAt}} {{.Image}}' goat-fed-republisher-$them
    sha256sum /etc/goat-fed-republisher-$them/* /opt/goat-fed-republisher-$them/docker-compose.yml" 2>&1
}

# A staged sample crosses to the partner whose agreement offers its set, and only there.
set_isolation() { # $1 scenario, $2 us, $3 label
  local n="$1" s="$2" label="$3" nonce t u
  nonce="iso$(date +%s)$$"
  for t in $(partners "$s"); do
    curl -s -m 8 -X PUT -d "$t-$nonce" "http://$(core_ip "$s"):8000/release/$t/$nonce" -o /dev/null
  done
  for t in $(partners "$s"); do
    rec "s$n-$label-$t-receives-its-own-set" -- bash -c \
      "for _ in \$(seq 1 30); do r=\$(curl -s -m 5 'http://$(core_ip "$t"):8000/from/$s/$t/$nonce'); case \"\$r\" in *value*) echo \"\$r\"; exit 0 ;; esac; sleep 1; done; echo 'not received'; exit 1"
  done
  sleep 5
  for t in $(partners "$s"); do
    for u in $(partners "$s"); do
      [ "$t" = "$u" ] && continue
      rec "s$n-$label-$t-never-receives-$u-set" -- bash -c \
        "r=\$(curl -s -m 5 'http://$(core_ip "$t"):8000/from/$s/$u/**'); echo \"\$r\"; case \"\$r\" in *value*) exit 1 ;; esac; exit 0"
    done
  done
}

guard_refusals() { # $1 scenario: the refusals that keep two peerings on one host apart
  local n="$1" a="cd /opt/goat/ops/ansible && GOAT_AWS_MODE=none GOAT_ANSIBLE_TRUNK_REMOTE=origin ../scripts/with-aws none -- ../scripts/ansible-preflight.sh -i inventory/peering-edges.ini"
  local pd="-e fed_work=/var/lib/goat-federation/paddock -e fed_counterparty=/var/lib/goat-federation/paddock/theirs.canonical.json -e fed_edge_group=paddock_edge"
  rec "s$n-a-second-peering-may-not-take-the-first-ones-interface" --expect-exit 2 --expect-in "carries the peering with meadow" -- \
    docker exec fed-pasture-edge bash -c "$a --limit paddock_edge $pd -e peering_wg_interface=wg-fed0 peering-wg.yml"
  rec "s$n-a-second-peering-may-not-take-the-first-ones-port" --expect-exit 2 --expect-in "listen port 51820" -- \
    docker exec fed-pasture-edge bash -c "$a --limit paddock_edge $pd -e peering_wg_listen_port=51820 peering-wg.yml"
  rec "s$n-a-second-republisher-may-not-take-the-first-ones-metrics-port" --expect-exit 2 --expect-in "goat-fed-republisher-meadow" -- \
    docker exec fed-pasture-edge bash -c "$a --limit paddock_core $pd -e goat_fed_republisher_metrics_port=9310 peering-republisher.yml"
  # One inventory name in two peering groups: the merge the per-peering names prevent.
  rec "s$n-preflight-refuses-one-inventory-name-in-two-peerings" --expect-exit 1 --expect-in "pasture-edge-meadow is in" -- bash -c \
    "rm -rf '$WORK/merged' && cp -R '$WORK/pasture/clone' '$WORK/merged' && sed -i 's/^pasture-edge-paddock /pasture-edge-meadow /' '$WORK/merged/ops/ansible/inventory/peering-edges.ini' && python3 '$WORK/merged/ops/federation/standup/preflight.py' paddock --work '$WORK/merged-work' --edge-group paddock_edge --repo '$WORK/merged'"
}

scenario3() {
  local n=3
  P_pasture="meadow paddock"; P_meadow=pasture; P_paddock=pasture; NODIR="pasture:paddock paddock:pasture"
  export_helpers
  say "scenario 3: pasture federates with meadow and with paddock on one edge host and one core host"
  setup_hosts "$n"

  # The first peering, through the driver's goatbuilder avenue and the directory.
  peer_up "$n" pasture meadow goatbuilder
  snap pasture meadow paddock > "$RECORD/s3-meadow-before-paddock.txt"
  docker exec fed-pasture-edge sha256sum /var/lib/goat-federation/meadow/ours.canonical.json | cut -d' ' -f1 > "$WORK/pasture-meadow-package.sha"

  # The second peering, added beside it: peering-standup.sh, packages by hand.
  peer_up "$n" pasture paddock script
  snap pasture meadow paddock > "$RECORD/s3-meadow-after-paddock.txt"
  # Phase 10 for paddock restarts pasture's core on the joined ACL, so the meadow
  # peering reconnects; nothing of its own may have changed.
  rec "s3-adding-paddock-left-meadow-untouched" -- diff "$RECORD/s3-meadow-before-paddock.txt" "$RECORD/s3-meadow-after-paddock.txt"
  accept_pair "$n" pasture meadow goatbuilder "-after-paddock"

  say "scenario 3: each partner receives the sets its own agreement offers, and no other"
  set_isolation "$n" pasture both-up
  # Each offered set is lifted once per peering that offers it: two writers of a set
  # both agreements offer. A measurement, not a check.
  for p in 9310 9311; do
    docker exec fed-pasture-core curl -s "http://127.0.0.1:$p/metrics" 2>&1 | grep -E 'published_total' || true
  done > "$RECORD/s3-shared-set-writers.txt"

  say "scenario 3: re-running one peering's phases leaves the other untouched"
  snap pasture paddock meadow > "$RECORD/s3-paddock-before-meadow-rerun.txt"
  rec "s3-rerun-meadow-phase-2" -- bash -c "drive pasture meadow goatbuilder 2 2"
  rec "s3-rerun-meadow-phases-7-to-9" -- bash -c "drive pasture meadow goatbuilder 7 9"
  rec "s3-rerun-meadow-phases-11-to-13" -- bash -c "drive pasture meadow goatbuilder 11 13"
  snap pasture paddock meadow > "$RECORD/s3-paddock-after-meadow-rerun.txt"
  rec "s3-meadow-rerun-left-paddock-untouched" -- diff "$RECORD/s3-paddock-before-meadow-rerun.txt" "$RECORD/s3-paddock-after-meadow-rerun.txt"
  rec "s3-paddock-round-trip-after-meadow-rerun" --expect-in "PASS" -- bash -c "round_trip_eventually pasture paddock"

  snap pasture meadow paddock > "$RECORD/s3-meadow-before-paddock-rerun.txt"
  rec "s3-rerun-paddock-phase-2" -- bash -c "drive pasture paddock script 2 2"
  rec "s3-rerun-paddock-phases-7-to-9" -- bash -c "drive pasture paddock script 7 9"
  rec "s3-rerun-paddock-phases-11-to-13" -- bash -c "drive pasture paddock script 11 13"
  snap pasture meadow paddock > "$RECORD/s3-meadow-after-paddock-rerun.txt"
  rec "s3-paddock-rerun-left-meadow-untouched" -- diff "$RECORD/s3-meadow-before-paddock-rerun.txt" "$RECORD/s3-meadow-after-paddock-rerun.txt"
  rec "s3-meadow-round-trip-after-paddock-rerun" --expect-in "PASS" -- bash -c "round_trip_eventually pasture meadow"
  accept_pair "$n" pasture meadow goatbuilder "-after-reruns"
  accept_pair "$n" pasture paddock script "-after-reruns"
  set_isolation "$n" pasture after-reruns

  say "scenario 3: the refusals that keep two peerings on one host apart"
  guard_refusals "$n"

  say "scenario 3: the directory still names the package meadow composed against"
  local dir_ptr="curl -fsS --cacert /etc/fed-e2e/directory-ca.pem https://$DIRECTORY_IP:8443/v1/parties/pasture"
  rec "s3-directory-names-the-meadow-package" --expect-in "$(cat "$WORK/pasture-meadow-package.sha")" -- docker exec fed-pasture-edge bash -c "$dir_ptr"
  # What meadow would see if pasture published its paddock package as its one entry:
  # the entry and its transport offer change; meadow's running peering does not.
  rec "s3-what-meadow-sees-if-pasture-republishes" --expect-in "$(boundary_ip pasture):$(wgport pasture paddock)" -- docker exec fed-pasture-edge bash -c \
    "SSL_CERT_FILE=/etc/fed-e2e/directory-ca.pem python3 /opt/goat/ops/federation/standup/directory.py --url https://$DIRECTORY_IP:8443 --ours /var/lib/goat-federation/paddock/ours.canonical.json --key /etc/fed-e2e/signing.key --ca /etc/fed-e2e/ca.pem --their-id meadow --work /tmp/republish-demo >/dev/null; $dir_ptr; echo; curl -fsS --cacert /etc/fed-e2e/directory-ca.pem https://$DIRECTORY_IP:8443/v1/parties/pasture/package | python3 -c 'import json,sys; print(json.load(sys.stdin)[\"transport_offer\"])'"
  rec "s3-meadow-still-carries-after-the-republish" --expect-in "PASS" -- bash -c "round_trip_eventually pasture meadow"
  collect "$n"
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
  P_pasture=meadow; P_meadow=pasture; P_paddock=""
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
if [ -z "$ONLY" ] || [ "$ONLY" = 3 ]; then scenario3; fi

say "the record"
{
  grep -h "acceptance-counts:" "$RECORD"/*acceptance-as-*.log || true
} | tee "$RECORD/acceptance-counts.txt"
# The record step is a step: a report that cannot be written fails the run.
python3 "$REPO/ops/e2e/report.py" "$RECORD" --check --title "cross-sovereign federation e2e (hermetic)" || { echo "run.sh: the run record did not validate (above)" >&2; FAILS=$((FAILS + 1)); }
echo "record: $RECORD"
[ "$FAILS" = 0 ] && exit 0 || exit 1
