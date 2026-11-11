#!/usr/bin/env bash
# engine.sh — the federation e2e's images, names and functions, sourced by run.sh.
# Moved unchanged from run.sh; run.sh's header says what the run proves.
# It reads the globals run.sh sets: HERE, REPO, WORK, RECORD, GOAT, GB, KEEP, ONLY.
#
# What it creates is named from the variables below, so that several runs share one Docker
# daemon without touching each other's containers or networks. Set them before sourcing:
#   SRC              the goatcore tree under test: its scripts, roles, driver, and the clone
#                    each sovereign gets (default REPO, the harness's own checkout)
#   FE2E_PARTIES     up to four sovereign ids, in order A B C D (default: ${A} ${B} ${C})
#   FE2E_HOST_PREFIX container hosts are <prefix>-<sovereign>-core|edge (default fed)
#   FE2E_NET_PREFIX  networks <prefix>-<sovereign>, <prefix>-boundary, and the directory's
#                    containers <prefix>-directory[-tls] (default fed-e2e)
#   FE2E_NET_BASE    the first two octets of the run's /24s (default 10.77)
#   FE2E_LAB         the value of label org.goat.fe2e.lab on all of it (default fed-e2e)
#   FE2E_HOST_IMAGE, FE2E_RP_TAG   the host image and the republisher's local tag
#   FE2E_SALTLICK_IMAGE the peering directory's image (default the pinned saltlick release)
#   FE2E_CORE        how each sovereign's core router is deployed (default hand):
#                    hand  `docker run` from core-seed.json5; phase 10 splices the fragment
#                          with acl-assemble.py and restarts the container
#                    role  the ref's own goat-zenoh role (zenoh-deploy.yml), run from the
#                          sovereign's clone on its edge host against the core host over
#                          ssh, as an operator runs it; phase 10 declares the peering in
#                          group_vars, places the fragment where the role splices it, and
#                          redeploys (core_deploy, admit_by_role)
#   FE2E_DRIVER      where each sovereign's operator runs the driver (default edge):
#                    edge       on the edge host, inventory edge at 127.0.0.1
#                    workspace  on a third host <prefix>-<sovereign>-ops, the operator's
#                               machine: on the sovereign's internal network only, holding
#                               the clone, goat, goatbuilder, the goat context, the working
#                               directory and the ssh key; the inventory names the edge and
#                               the core by their addresses. A unit sets it before setup_hosts.
#   FE2E_PARTNER     where each sovereign's federation partner is (default lab):
#                    lab       both parties in this lab, as every unit but solo runs them
#                    external  the partner is outside the lab (units/solo.sh): setup_hosts
#                              creates only the parties that have partners (the lab
#                              sovereign's core and edge), no peering directory and no
#                              isolation checks; the edge offers FE2E_SOLO_PUBLIC_ADDR,
#                              and reaches the partner through the docker bridge's NAT,
#                              through FE2E_SOLO_WAN (a docker network, attached at
#                              FE2E_SOLO_WAN_IP: a stand-in for the internet), or listens
#                              on FE2E_SOLO_PUBLISH, a UDP port the machine publishes
#                              (docker -p on the edge host). See solo_endpoint below.
SRC="${SRC:-$REPO}"
read -r A B C D <<<"${FE2E_PARTIES:-pasture meadow paddock}"
PARTIES="$A $B ${C:-} ${D:-}"
HP="${FE2E_HOST_PREFIX:-fed}"; NP="${FE2E_NET_PREFIX:-fed-e2e}"; NB="${FE2E_NET_BASE:-10.77}"
LAB="${FE2E_LAB:-fed-e2e}"
LABEL=(--label "org.goat.fe2e.lab=$LAB")
FE2E_CORE="${FE2E_CORE:-hand}"
FE2E_DRIVER="${FE2E_DRIVER:-edge}"
FE2E_PARTNER="${FE2E_PARTNER:-lab}"
case "$FE2E_PARTNER" in lab|external) ;; *) echo "engine: FE2E_PARTNER must be lab or external, not '$FE2E_PARTNER'" >&2; exit 2 ;; esac
case "$FE2E_CORE" in hand|role) ;; *) echo "engine: FE2E_CORE must be hand or role, not '$FE2E_CORE'" >&2; exit 2 ;; esac
export SRC A B C D PARTIES HP NP NB LAB FE2E_CORE FE2E_DRIVER FE2E_PARTNER
# The fleet image, by digest, and the SPIRE release the fleet pins: the `spire`
# component's value of record in ops/pins/fleet-pins.yaml, which every checkout carries
# (a public one declares no goatnet there, but keeps every component). Both pull
# anonymously.
ZENOH_IMAGE="ghcr.io/usareur-af/zenoh-pqc@sha256:372de39d2e8b64fe7157194173f1b7ac6eac2700ff04c49cce2ce3c284a6fdb2"
SPIRE_IMAGE="$(python3 -c 'import sys, yaml; print(((yaml.safe_load(open(sys.argv[1])).get("components") or {}).get("spire") or {}).get("fleet") or "")' "$SRC/ops/pins/fleet-pins.yaml" 2>/dev/null || true)"
[[ "$SPIRE_IMAGE" =~ ^ghcr\.io/spiffe/spire-server:[^@[:space:]]+@sha256:[0-9a-f]{64}$ ]] || { echo "run.sh: ops/pins/fleet-pins.yaml declares no digest-pinned SPIRE image (components.spire.fleet)" >&2; exit 2; }
HOST_IMAGE="${FE2E_HOST_IMAGE:-fed-e2e-host}"
# The peering directory both edges reach, and the TLS front that issues its certificate
# from a local CA: saltlick refuses a pointer whose package_url is not https. The pinned
# release by default; FE2E_SALTLICK_IMAGE names another (env --directory-ref builds one from
# a saltlick commit).
SALTLICK_IMAGE="${FE2E_SALTLICK_IMAGE:-ghcr.io/usareur-af/saltlick@sha256:6ec46ca98819e1523f30e85de8210db33f71040f1214171366e459f79bab7971}"
DIRECTORY_TLS_IMAGE="caddy@sha256:c3d7ee5d2b11f9dc54f947f68a734c84e9c9666c92c88a7f30b9cba5da182adb"
DIRECTORY_IP="$NB.3.40"; export DIRECTORY_IP
# The republisher, built from this checkout and loaded into each core host's dockerd.
# The role takes its content-addressed id (`sha256:<64 hex>`), as immutable as a
# registry digest, until the published image is pinned in ops/pins/fleet-pins.yaml.
RP_TAG="${FE2E_RP_TAG:-fed-e2e-republisher}"
RP_IMAGE=""
# The id a host's own dockerd gives an image loaded with `docker save | docker load`: the
# image's config digest. The runner's .Id is that digest only on the classic image store;
# with the containerd image store (Docker 29 defaults) it is the manifest digest instead.
image_config_id() { # $1 image reference -> sha256:<config digest>
  docker save "$1" | tar -xO manifest.json | python3 -c 'import json,sys; c=json.load(sys.stdin)[0]["Config"]; print("sha256:" + c.split("/")[-1].split(".")[0])'
}
ACL_ASSEMBLE="$SRC/ops/ansible/roles/goat-zenoh/files/acl-assemble.py"
REC="$REPO/ops/e2e/rec.py"
FAILS=0
rec() { python3 "$REC" "$@" || FAILS=$((FAILS + 1)); }
# A declared input of the run that could not be reached, or an environment the run cannot
# start in: exit 69 (EX_UNAVAILABLE) with "environment:", never read as a failure of the tree
# under test (docs/design/ci-results-follow-inputs.md, rule 1).
FE2E_EXIT_ENVIRONMENT=69
env_fault() {
  echo "environment: $*" >&2
  [ -z "${RECORD:-}" ] || printf 'environment: %s\n' "$*" >> "$RECORD/stopped.reason"
  exit "$FE2E_EXIT_ENVIRONMENT"
}
say() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }
die() { echo "run.sh: $*" >&2; exit 1; }
other() { [ "$1" = "$A" ] && echo "$B" || echo "$A"; }
# A party's place in the run's party list (PARTIES, set once when the engine loads), so the
# names and addresses below hold even where a unit shadows A and B for one side.
party_pos() { local i=0 p; for p in $PARTIES; do i=$((i + 1)); [ "$p" = "$1" ] && { echo "$i"; return 0; }; done; return 1; }
octet() { case "$(party_pos "$1")" in 1) echo 1 ;; 2) echo 2 ;; 3) echo 4 ;; 4) echo 5 ;; esac; }
core_ip() { echo "$NB.$(octet "$1").10"; }
edge_ip() { echo "$NB.$(octet "$1").20"; }
ops_ip() { echo "$NB.$(octet "$1").30"; }
# The host the sovereign's driver runs on, where its clone, goat context and working
# directory (/var/lib/goat-federation/<them>) are (FE2E_DRIVER).
drv_host() {
  case "$FE2E_DRIVER" in
    edge) echo "$HP-$1-edge" ;;
    workspace) echo "$HP-$1-ops" ;;
    *) echo "engine: FE2E_DRIVER must be edge or workspace, not '$FE2E_DRIVER'" >&2; return 2 ;;
  esac
}
boundary_ip() { case "$(party_pos "$1")" in 1) echo "$NB.3.11" ;; 2) echo "$NB.3.12" ;; 3) echo "$NB.3.13" ;; 4) echo "$NB.3.14" ;; esac; }
td() { echo "$1.fed.test"; }

# EVERY PER-PEERING VALUE IS A FUNCTION OF THE PAIR. A party's partners, in order, are
# P_<party>; its first peering keeps every role default, and each later one gets the
# values two peerings cannot share on one edge host and one core host.
partners() { local v="P_$1"; echo "${!v:-}"; }
set_partners() { printf -v "P_$1" '%s' "$2"; export "P_$1"; } # $1 party, $2 its partners in order
for _p in $PARTIES; do set_partners "$_p" ""; done; NODIR=""
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
  # One /31 per pair, the same from either side.
  local i j t
  i="$(octet "$1")"; j="$(octet "$2")"; [ "$i" -gt "$j" ] && { t="$i"; i="$j"; j="$t"; }
  case "$i:$j" in
    1:2) echo 169.254.77.0/31 ;; 1:4) echo 169.254.78.0/31 ;; 2:4) echo 169.254.79.0/31 ;;
    1:5) echo 169.254.80.0/31 ;; 2:5) echo 169.254.81.0/31 ;; *) echo 169.254.82.0/31 ;;
  esac
}
nodir() { case " $NODIR " in *" $1:$2 "*) return 0 ;; *) return 1 ;; esac; }
partner_list() { # the parties in this scenario
  local s; for s in $PARTIES; do [ -n "$(partners "$s")" ] && echo "$s"; done; return 0
}

NETS=("$NP-boundary"); HOSTS=()
# The workspace host is listed whether or not FE2E_DRIVER starts it, so teardown removes it.
for _p in $PARTIES; do NETS+=("$NP-$_p"); HOSTS+=("$HP-$_p-core" "$HP-$_p-edge" "$HP-$_p-ops"); done
DIRECTORY=("$NP-directory" "$NP-directory-tls")
teardown() {
  local net
  docker rm -f -v "${HOSTS[@]}" "${DIRECTORY[@]}" >/dev/null 2>&1 || true
  for net in "${NETS[@]}"; do docker network rm "$net" >/dev/null 2>&1 || true; done
}
final() { [ "$KEEP" = 1 ] || teardown; }

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
  git clone -q --depth 1 --single-branch "file://$SRC" "$d/clone"
  git -C "$d/clone" checkout -q -B main
  git -C "$d/clone" config user.name "$s operator"; git -C "$d/clone" config user.email "ops@$s.test"
  local also=() first; first="$(partners "$s" | awk '{print $1}')"
  for t in $(partners "$s"); do
    [ "$t" = "$first" ] && continue
    also+=(--also "$t:$(iface "$s" "$t"):$(wgport "$s" "$t"):$(spire_out "$s" "$t"):$(rp_metrics "$s" "$t")")
  done
  # FE2E_OVERLAY_ARGS_<sovereign>: more overlay.py arguments for one sovereign only (a unit
  # sets --no-peering for a sovereign whose peering must come from its operator and the
  # command, units/nohand.sh).
  local more=""; eval "more=\${FE2E_OVERLAY_ARGS_$s:-}"
  # shellcheck disable=SC2086,SC2046 # the optional flags are meant to split into words
  python3 "$HERE/overlay.py" "$d/clone" "$s" "$first" "$(octet "$s")" "$ZENOH_IMAGE" --republisher-image "$RP_IMAGE" \
    --core-addr "$(core_ip "$s")" --spire-listen "$(edge_ip "$s")" ${also[@]+"${also[@]}"} \
    $([ "$FE2E_DRIVER" = workspace ] && echo "--edge-addr $(edge_ip "$s")") $more
  if [ "$FE2E_CORE" = role ]; then core_role_overlay "$s" "$d/clone"; fi
  git -C "$d/clone" add -A
  git -C "$d/clone" commit -q -m "$s: this sovereign's declarations for the peering"
  git clone -q --bare "$d/clone" "$d/origin.git"
  git -C "$d/origin.git" symbolic-ref HEAD refs/heads/main
  git -C "$d/clone" remote set-url origin "$d/origin.git"
  git -C "$d/clone" fetch -q origin
}

start_host() { # $1 name, $2 sovereign, $3 internal ip, [more docker run arguments]
  local n="$1" s="$2" ip="$3"; shift 3
  docker run -d --name "$n" --hostname "$n" "${LABEL[@]}" --privileged --cgroupns=private \
    --network "$NP-$s" --ip "$ip" "$@" \
    --tmpfs /run --tmpfs /run/lock -v /var/lib/docker -v /lib/modules:/lib/modules:ro \
    -v "$WORK/$s/keys:/etc/fed-e2e:ro" -v "$WORK/$s/origin.git:/srv/goat.git:ro" \
    -v "$(goat_of "$s"):/usr/local/bin/goat:ro" -v "$GB:/usr/local/bin/goatbuilder:ro" \
    -e container=docker "$HOST_IMAGE" >/dev/null
}

# host_package <host> <package>: make sure the host has <package> installed in dpkg's records.
# This machine's own copy is repacked only when this machine runs the host image's Ubuntu
# release: a package built for another release (python3-apt for another Python, say) would
# be recorded as installed and fail to import. Otherwise the host installs it with apt.
host_package() {
  local h="$1" pkg="$2" d mine theirs
  docker exec "$h" dpkg -s "$pkg" >/dev/null 2>&1 && return 0
  mine="$(. /etc/os-release 2>/dev/null && echo "${ID:-}-${VERSION_ID:-}")"
  theirs="$(docker exec "$h" sh -c '. /etc/os-release && echo "$ID-$VERSION_ID"' 2>/dev/null || true)"
  # Built once per run under the run's own WORK, and reused for every host.
  d="$WORK/debs/$pkg"
  if [ -n "$mine" ] && [ "$mine" = "$theirs" ] && [ ! -s "$d.deb" ] && dpkg -s "$pkg" >/dev/null 2>&1; then
    mkdir -p "$d/DEBIAN"
    dpkg -L "$pkg" | while read -r f; do [ -f "$f" ] && printf '%s\n' "${f#/}"; done \
      | tar -C / -cf - -T - 2>/dev/null | tar -C "$d" -xf -
    {
      dpkg-query -W -f='Package: ${Package}\nVersion: ${Version}\nArchitecture: ${Architecture}\nMaintainer: ${Maintainer}\n' "$pkg"
      [ -z "$(dpkg-query -W -f='${Depends}' "$pkg")" ] || printf 'Depends: %s\n' "$(dpkg-query -W -f='${Depends}' "$pkg")"
      dpkg-query -W -f='Description: ${Description}\n' "$pkg"
    } > "$d/DEBIAN/control"
    dpkg-deb --root-owner-group --build "$d" "$d.deb" >/dev/null 2>&1 || true
  fi
  # --force-depends only for a copy of the host's own release: the image lacks the package's
  # small dependencies (python-apt-common, distro-info-data), whose files the copy does not
  # need, and the run needs no mirror. A copy of another release is never installed.
  if [ -n "$mine" ] && [ "$mine" = "$theirs" ] && [ -s "$d.deb" ] && docker cp -q "$d.deb" "$h:/root/fe2e-$pkg.deb" \
    && docker exec "$h" dpkg -i --force-depends "/root/fe2e-$pkg.deb" >/dev/null 2>&1; then
    return 0
  fi
  # The Ubuntu mirror, from inside a host, only where a lab asks for it (FE2E_HOST_APT=1): in
  # CI every package a scenario needs is in the host image, and the job reaches no mirror.
  [ "${FE2E_HOST_APT:-0}" = 1 ] || return 1
  for _ in 1 2 3; do
    docker exec "$h" sh -c "apt-get update -qq >/dev/null 2>&1 && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq $pkg >/dev/null 2>&1" && return 0
    sleep 10
  done
  return 1
}

# The goat binary one sovereign's hosts carry: GOAT_<id> when the environment pinned a
# release for that sovereign (mixed versions), else the ref's own GOAT. A pin that did not
# resolve ("unavailable:<version>") stops the run rather than quietly using the ref's goat.
goat_of() { # $1 sovereign
  local v="GOAT_$1" p
  p="${!v:-}"
  case "$p" in
    "") echo "$GOAT" ;;
    unavailable:*) die "goat ${p#unavailable:} for $1 is not published as a release asset" ;;
    *) echo "$p" ;;
  esac
}

# Replace goat and goatbuilder on a running host with other binaries (a rollback of the
# tools): each is copied into the host and bind-mounted over the path the drivers call,
# /usr/local/bin/goat and /usr/local/bin/goatbuilder. The binaries in the build cache are
# not touched. $1 host, $2 goat, $3 goatbuilder.
replace_tools() {
  local h="$1" t
  docker exec "$h" install -d -m 0755 /usr/local/lib/fe2e-tools
  docker cp -q "$2" "$h:/usr/local/lib/fe2e-tools/goat"
  docker cp -q "$3" "$h:/usr/local/lib/fe2e-tools/goatbuilder"
  for t in goat goatbuilder; do
    docker exec "$h" sh -c "chmod 0755 /usr/local/lib/fe2e-tools/$t && mount --bind /usr/local/lib/fe2e-tools/$t /usr/local/bin/$t"
  done
}

wait_ready() { # $1 host: systemd up, dockerd answering
  local h="$1"
  for _ in $(seq 1 90); do
    if docker exec "$h" sh -c 'systemctl is-active --quiet ssh && docker version --format "{{.Server.Version}}"' >/dev/null 2>&1; then
      # The machine's own AppArmor profile `wg` (Ubuntu 26.04 LTS) attaches by path to
      # /usr/bin/wg in every container and lets it read keys only under /etc/wireguard. A
      # copy on PATH ahead of it runs unconfined, as on a host without that profile.
      # FE2E_WG_APPARMOR=host keeps the machine's profile in force (scenario
      # ubuntu-2604-apparmor).
      [ "${FE2E_WG_APPARMOR:-bypass}" = host ] || docker exec "$h" sh -c 'cp /usr/bin/wg /usr/local/sbin/wg'
      # apt inside a host gives up on an unreachable mirror address in seconds and tries the
      # next, over IPv4: a machine whose IPv6 or one mirror address does not answer otherwise
      # stalls phase 2's apt-get update for many minutes.
      docker exec "$h" sh -c 'printf "%s\n" "Acquire::http::Timeout \"20\";" "Acquire::https::Timeout \"20\";" "Acquire::Retries \"5\";" "Acquire::ForceIPv4 \"true\";" > /etc/apt/apt.conf.d/99fe2e-net'
      # FE2E_DENY_MIRROR=1 (the proof of rule 1, ops/ci/check-federation-e2e-follows-inputs.sh):
      # the host cannot reach a distribution mirror at all, so a scenario that still needs one
      # fails where it reaches for it.
      if [ "${FE2E_DENY_MIRROR:-0}" = 1 ]; then
        docker exec "$h" sh -c 'for n in archive.ubuntu.com security.ubuntu.com ports.ubuntu.com ppa.launchpadcontent.net deb.debian.org pypi.org files.pythonhosted.org; do printf "0.0.0.0 %s\n" "$n"; done >> /etc/hosts'
      fi
      # The roles' apt tasks (update_cache with cache_valid_time 3600; goat-peering-wg installs
      # wireguard-tools and nftables, goat-zenoh installs unzip) and Ansible's apt module, which
      # needs python3-apt, would reach the Ubuntu mirror on every run. So a run needs no mirror:
      # the host image installs FE2E_HOST_PACKAGES at build time (host/Dockerfile), and this
      # step only confirms each is there. A package the image lacks (an older image, or a
      # longer FE2E_HOST_PACKAGES) is installed into the host as a real dpkg package, built
      # from this machine's own installed copy when this machine runs the host image's
      # release, else fetched with apt inside the host. Then the host's apt cache is marked
      # fresh. A host left without one of them stops the unit as an environment fault
      # (BLOCKED), naming the package, never as a FAIL of the tree under test.
      local pkg
      for pkg in ${FE2E_HOST_PACKAGES:-python3-apt unzip}; do
        host_package "$h" "$pkg" \
          || env_fault "host $h lacks package $pkg: the host image (host/Dockerfile) should carry it, and no run-time source provided it (FE2E_HOST_APT=1 lets a lab host fetch it from the Ubuntu mirror)"
      done
      docker exec "$h" sh -c 'mkdir -p /var/lib/apt/periodic && touch /var/lib/apt/periodic/update-success-stamp'
      return 0
    fi
    sleep 2
  done
  docker exec "$h" systemctl --no-pager status docker ssh 2>&1 | tail -20 || true
  die "host $h did not come up"
}

start_core() { # $1 sovereign: seed config, bootstrap cert, the router and the SPIRE server
  local s="$1" them; them="$(partners "$s" | awk '{print $1}')"
  if [ "$FE2E_CORE" = role ]; then start_core_role "$s"; else start_core_hand "$s" "$them"; fi
  start_spire "$s"
}

start_core_hand() { # $1 sovereign, $2 its first partner: the router by `docker run`
  local s="$1" h="$HP-$1-core" them="$2" extra
  sed "s/__THEM__/$them/g; s/__US__/$s/g; s/__CORE_LISTEN__/0.0.0.0/; s/__CORE_REST__/0.0.0.0/" "$HERE/core-seed.json5" > "$WORK/$s/core-seed.json5"
  # A later partner's landing and not-granted prefixes are held the same way.
  read -r -a extra <<< "$(partners "$s" | cut -s -d' ' -f2-)"
  python3 - "$WORK/$s/core-seed.json5" ${extra[@]+"${extra[@]}"} <<'PY'
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
}

start_spire() { # $1 sovereign: the SPIRE server on its core host
  local s="$1" h="$HP-$1-core"
  python3 "$HERE/render-spire.py" "$SRC/ops/ansible/roles/spire/templates/server.conf.j2" "$(td "$s")" "$WORK/$s/server.conf"
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

# Network profile seams. A unit may redefine these after sourcing the engine. The
# defaults are the direct path every other unit runs: the edge holds its own boundary
# address and offers it, with the role's default MTU.
offer_endpoint() { # $1 us, $2 them
  if [ "$FE2E_PARTNER" = external ]; then solo_endpoint "$1" "$2"; else echo "$(boundary_ip "$1"):$(wgport "$1" "$2")"; fi
}
offer_mtu() { echo 1380; }                                           # $1 us, $2 them
attach_boundary() { # $1 sovereign
  if [ "$FE2E_PARTNER" = external ]; then solo_attach "$1"; return 0; fi
  docker network connect --ip "$(boundary_ip "$1")" "$NP-boundary" "$HP-$1-edge"
}

# --- a partner outside the lab (FE2E_PARTNER=external) ------------------------------------
# The lab edge's public side. It dials by default (the partner listens), out through the
# docker bridge's NAT, and offers FE2E_SOLO_PUBLIC_ADDR, the address the partner sees it
# at (the partner's edge drops tunnel packets from any other address). When the partner
# cannot listen, the lab edge listens: on FE2E_SOLO_PUBLISH, a UDP port the machine
# publishes to the edge host's tunnel port, or at FE2E_SOLO_WAN_IP on FE2E_SOLO_WAN.
# FE2E_SOLO_WAN is a docker network that stands in for the internet when two labs play
# both sides; the edge is attached to it, and its address there is the default public one.
solo_public_addr() { echo "${FE2E_SOLO_PUBLIC_ADDR:-${FE2E_SOLO_WAN_IP:-}}"; }
solo_endpoint() { # $1 us, $2 them: host:port as the partner reaches this edge
  local a; a="$(solo_public_addr)"
  [ -n "$a" ] || die "FE2E_PARTNER=external: set FE2E_SOLO_PUBLIC_ADDR, the address the partner's edge sees this edge at"
  echo "$a:${FE2E_SOLO_PUBLISH:-$(wgport "$1" "$2")}"
}
solo_attach() { # $1 sovereign
  [ -n "${FE2E_SOLO_WAN:-}" ] || return 0
  [ -n "${FE2E_SOLO_WAN_IP:-}" ] || die "FE2E_SOLO_WAN is set without FE2E_SOLO_WAN_IP, this edge's address on it"
  docker network connect --ip "$FE2E_SOLO_WAN_IP" "$FE2E_SOLO_WAN" "$HP-$1-edge"
}
solo_edge_args() { # $1 sovereign: docker run arguments of its edge host
  [ "$FE2E_PARTNER" = external ] && [ -n "${FE2E_SOLO_PUBLISH:-}" ] || return 0
  local them; them="$(partners "$1" | awk '{print $1}')"
  echo "-p ${FE2E_SOLO_PUBLISH}:$(wgport "$1" "$them")/udp"
}

# --- the core router by the ref's goat-zenoh role (FE2E_CORE=role) ----------------------
# What a real goatnet's core is: container <site_id>-zenoh-router, label
# org.goat.component=zenoh-core, compose project <site_id>-zenoh from
# /opt/<site_id>-zenoh/docker-compose.yml, config /etc/<site_id>-zenoh/config.json5. The
# role's own templates render it; the harness supplies only a deployment's values.
core_ca() { # $1 sovereign: the trust roots its core router loads
  if [ "$FE2E_CORE" = role ]; then echo "/etc/$1-zenoh/tls/ca-root-cert.pem"; else echo /etc/goat-core/ca.pem; fi
}

# The deployment's declarations for its core router, in its own repository: one inventory
# group naming the core host, and the role's values. The image is the digest-pinned one
# the hand-run core uses. No peering is declared yet: phase 10 does that.
core_role_overlay() { # $1 sovereign, $2 its clone
  local s="$1" c="$2"
  mkdir -p "$c/ops/ansible/inventory" "$c/ops/ansible/group_vars/core_router"
  cat > "$c/ops/ansible/inventory/core-router.ini" <<INI
# This sovereign's core router host, deployed by zenoh-deploy.yml (roles/goat-zenoh).
[core_router]
$s-core ansible_host=$(core_ip "$s") ansible_user=goat
INI
  cat > "$c/ops/ansible/group_vars/core_router/zenoh.yml" <<YAML
# The core router of $s (roles/goat-zenoh, zenoh-deploy.yml -e target=core_router -e site_id=$s).
zenoh_router_image: "$ZENOH_IMAGE"
# No mesh interface on this host: publish the fabric port on every address of the host.
zenoh_listen_host_override: "0.0.0.0"
# No vendor of this goatnet is seeded; the federation fragment is the only org content.
zenoh_seed_vendors: []
YAML
}

# zenoh-deploy.yml from the sovereign's clone on its edge host, through the same wrapper
# phases 11-12 use, against the core host over ssh. REST is published on the core host's
# address because this is a drill whose checks read it from outside (the play pins it to
# loopback; an extra var is the documented override).
core_deploy() { # $1 sovereign, [ansible-playbook arguments]
  local s="$1"; shift
  docker exec -e GOAT_AWS_MODE=none -e GOAT_ANSIBLE_TRUNK_REMOTE=origin -e HOME=/root -w /opt/goat/ops/ansible "$(drv_host "$s")" \
    ../scripts/with-aws none -- ../scripts/ansible-preflight.sh -i inventory/core-router.ini \
    -e target=core_router -e "site_id=$s" -e goat_zenoh_rest_host=0.0.0.0 "$@" zenoh-deploy.yml
}

start_core_role() { # $1 sovereign: the CA root and the bootstrap leaf where the role reads them, then the role
  local s="$1" e log="$RECORD/core-role-deploy-$1.log"; e="$(drv_host "$1")"
  host_exec "$e" sh -c "[ -d /opt/goat/.git ] || git clone -q /srv/goat.git /opt/goat"
  host_exec "$e" sh -c "git -C /opt/goat config user.name '$s operator'; git -C /opt/goat config user.email 'ops@$s.test'"
  # The role's defaults: the CA root under the checkout's ops/enrollment/ca/<site_id>/,
  # the router leaf in the controller's state directory (goat_leaf, ~/.goat/state/<site>-leaves).
  host_exec "$e" sh -c "install -d -m 0755 /opt/goat/ops/enrollment/ca/$s && install -m 0644 /etc/fed-e2e/ca.pem /opt/goat/ops/enrollment/ca/$s/root-cert.pem \
    && install -d -m 0700 /root/.goat/state/$s-leaves && install -m 0644 /etc/fed-e2e/core.pem /root/.goat/state/$s-leaves/router-cert.pem \
    && install -m 0600 /etc/fed-e2e/core-key.pem /root/.goat/state/$s-leaves/router-key.pem"
  if ! core_deploy "$s" > "$log" 2>&1; then
    tail -40 "$log" >&2
    die "the goat-zenoh role did not deploy $s's core router (zenoh-deploy.yml; log $log)"
  fi
  host_exec "$HP-$s-core" docker inspect -f '{{.State.Running}}' "$s-zenoh-router" 2>/dev/null | grep -qx true \
    || die "the goat-zenoh role ran, but no running container $s-zenoh-router is on $HP-$s-core"
}

# A reviewed change to the core's declarations, merged into this sovereign's trunk (its
# origin) and fast-forwarded on the edge's checkout: ansible-preflight.sh refuses to deploy
# from a main that holds commits its trunk does not.
core_declare() { # $1 sovereign, $2 path in the repository, $3 local file, $4 commit message
  local s="$1" rel="$2" src="$3" msg="$4" c="$WORK/$1/clone"
  mkdir -p "$(dirname "$c/$rel")"; cp "$src" "$c/$rel"
  git -C "$c" add "$rel"
  git -C "$c" diff --cached --quiet || git -C "$c" commit -q -m "$msg"
  git -C "$c" push -q origin HEAD:main
  host_exec "$(drv_host "$s")" git -C /opt/goat pull -q --ff-only origin main
}

# Phase 10 on a role-deployed core, as its operator does it: the peering declared in the
# core's group_vars (zenoh_sovereign_peerings, which renders the from/** landing stores;
# its CN is the fragment's subject), the reviewed fragment placed as the role's org
# fragment (the file the role splices into the seed's marker regions with acl-assemble),
# the core certificate phase 8 minted placed where the role reads the router leaf, then a
# dry run and the deploy, which re-renders, re-splices and restarts the router.
admit_by_role() { # $1 sovereign, $2 the peering being admitted, $3 its applied dir, $4 scenario number
  local s="$1" them="$2" d="$3" n="$4" e w="/var/lib/goat-federation/$2"; e="$(drv_host "$1")"
  # shellcheck disable=SC2046 # one argument per partner
  python3 - "$d" $(partners "$s") > "$d/peerings.yml" <<'PY' || die "$s: no peering subject in a rendered fragment under $d"
import os, re, sys
import yaml
d, out = sys.argv[1], []
for t in sys.argv[2:]:
    p = os.path.join(d, f"fragment-{t}.json5")
    if not os.path.exists(p):
        continue
    m = re.search(r'id:\s*"fed-%s",\s*cert_common_names:\s*\["([^"]+)"\]' % re.escape(t), open(p).read())
    if not m:
        sys.exit(f"no subject fed-{t} in {p}")
    out.append({"sovereign": t, "cn": m.group(1)})
print("# Each admitted peering, from the subject of its reviewed fragment (phase 10).")
print(yaml.safe_dump({"zenoh_sovereign_peerings": out}, sort_keys=False), end="")
PY
  core_declare "$s" ops/ansible/group_vars/core_router/peerings.yml "$d/peerings.yml" "$s: admit the peering with $them on the core router"
  docker cp -q "$d/fragment.json5" "$HP-$s-core:/etc/$s-zenoh/acl-org-fragment.json5"
  host_exec "$e" sh -c "install -m 0644 $w/pki/core.pem /root/.goat/state/$s-leaves/router-cert.pem && install -m 0600 $w/pki/core-key.pem /root/.goat/state/$s-leaves/router-key.pem"
  rec "s$n-$s-$them-core-role-dry-run" -- bash -c "core_deploy $s --check --diff"
  rec "s$n-$s-$them-core-role-admits" --expect-in "Restart zenoh router" -- bash -c "core_deploy $s"
}

prepare_edge() { # $1 sovereign, $2 them, $3 "true" when this side listens
  # Each peering's declaration lives in its own goat context: the first in `default`,
  # a later one in a context named for the partner, selected with GOAT_CONTEXT.
  local s="$1" them="$2" listener="$3" h p; h="$(drv_host "$1")"
  host_exec "$h" sh -c "[ -d /opt/goat/.git ] || git clone -q /srv/goat.git /opt/goat"
  host_exec "$h" sh -c "git -C /opt/goat config user.name '$s operator'; git -C /opt/goat config user.email 'ops@$s.test'"
  p="/root/.goat/contexts/$(ctx_of "$s" "$them")"
  host_exec "$h" install -d -m 0700 /root/.goat /root/.goat/contexts "$p"
  host_exec "$h" install -m 0644 /etc/fed-e2e/profile.pem "$p/mtls.cert.pem"
  host_exec "$h" install -m 0644 /etc/fed-e2e/ca.pem "$p/ca-roots.pem"
  # The [data] halves, per side (the agreement-shapes units set them; run.sh does not):
  #   FE2E_RELEASE_<sovereign>  a TOML array for `release`, or `none` to declare no send
  #                             half (default: release/all/** and release/<them>/**)
  #   FE2E_ACCEPT_<sovereign>   `no` to declare no receive half (default: the three accept_*)
  local rel acc release_line accept_lines
  eval "rel=\${FE2E_RELEASE_$s-}"; eval "acc=\${FE2E_ACCEPT_$s-yes}"
  [ -n "$rel" ] || rel="[\"release/all/**\", \"release/$them/**\"]"
  release_line="release                  = $rel"
  [ "$rel" = none ] && release_line="# no send half: this side releases nothing"
  accept_lines="accept_hz                = 100
accept_bytes_per_message = 1048576
accept_distinct_keys     = 5000"
  [ "$acc" = no ] && accept_lines="# no receive half: this side accepts nothing"
  cat > "$WORK/$s/federation.toml" <<TOML
sovereign_id = "$s"
term_days    = 30
cut_contact  = "fed-ops@$s.test"

[transport]
interface            = "$(iface "$s" "$them")"
endpoint             = "$(offer_endpoint "$s" "$them")"
listener             = $listener
proposed_link_subnet = "$(pair_subnet "$s" "$them")"
measured_mtu         = $(offer_mtu "$s" "$them")

[data]
$release_line
$accept_lines
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
  local s="$1" them="$2" avenue="$3" from="$4" to="$5" h; h="$(drv_host "$1")"
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
  if nodir "$s" "$them"; then env+=(-e FED_NO_DIRECTORY=1); else env+=(-e "FED_DIRECTORY_URL=https://$DIRECTORY_IP:8443"); fi
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
  docker run -d --name "$NP-directory" "${LABEL[@]}" --network "$NP-boundary" --ip "$DIRECTORY_IP" \
    -e SALTLICK_WRITE_LIMIT=0 -e SALTLICK_READ_LIMIT=0 "$SALTLICK_IMAGE" >/dev/null
  docker run -d --name "$NP-directory-tls" "${LABEL[@]}" --network "container:$NP-directory" "$DIRECTORY_TLS_IMAGE" \
    caddy reverse-proxy --from "https://$DIRECTORY_IP:8443" --to 127.0.0.1:8080 >/dev/null
  local s
  for _ in $(seq 1 30); do
    if docker cp -q "$NP-directory-tls":/data/caddy/pki/authorities/local/root.crt "$WORK/directory-ca.pem" 2>/dev/null; then
      # /etc/fed-e2e on each edge is its keys folder, mounted read-only.
      for s in $(partner_list); do install -m 0644 "$WORK/directory-ca.pem" "$WORK/$s/keys/directory-ca.pem"; done
      docker exec $HP-${A}-edge curl -fsS --cacert /etc/fed-e2e/directory-ca.pem "https://$DIRECTORY_IP:8443/healthz" >/dev/null 2>&1 && return 0
    fi
    sleep 1
  done
  die "the peering directory did not answer over TLS at https://$DIRECTORY_IP:8443: $(docker logs "$NP-directory" 2>&1 | tail -2)"
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
      docker cp -q "$(drv_host "$s"):/var/lib/goat-federation/$them/ours.canonical.json" "$WORK/$s/handover-$them.json"
      docker cp -q "$WORK/$s/handover-$them.json" "$(drv_host "$them"):/var/lib/goat-federation/$s/theirs.canonical.json"
      docker cp -q "$WORK/$s/keys/ca.pem" "$(drv_host "$them"):/var/lib/goat-federation/$s/theirs-ca.pem"
      docker cp -q "$(drv_host "$s"):/var/lib/goat-federation/$them/ours.spiffe-bundle" "$WORK/$s/handover-$them.bundle"
      docker cp -q "$WORK/$s/handover-$them.bundle" "$(drv_host "$them"):/var/lib/goat-federation/$s/theirs.spiffe-bundle"
    done
  else
    rec "s$n-$first-$second-phase-5-again" --expect-in "Directory step" -- bash -c "drive '$first' '$second' '$avenue' 5 5"
  fi
  for s in "$first" "$second"; do
    them="$([ "$s" = "$first" ] && echo "$second" || echo "$first")"; w="/var/lib/goat-federation/$them"
    rec "s$n-$s-$them-their-files-are-in-place" -- docker exec "$(drv_host "$s")" sh -c \
      "test -s $w/theirs.canonical.json && test -s $w/theirs-ca.pem && test -s $w/theirs.spiffe-bundle"
    printf '%s=%s\n%s=%s\n' "$s" "$(cat "$WORK/$s/keys/signing.pub")" "$them" "$(cat "$WORK/$them/keys/signing.pub")" > "$WORK/$s/keys.txt"
    docker cp -q "$WORK/$s/keys.txt" "$(drv_host "$s"):/var/lib/goat-federation/$them/keys.txt"
  done
}

# Phase 10, as an operator does it: check their CA against the fingerprint in the signed
# agreement, place it, splice the rendered fragment with the real assembler, and restart
# the core on the certificate phase 8 minted.
review_and_admit() { # $1 sovereign, $2 the peering being admitted, [$3 scenario number]
  local s="$1" them="$2" n="${3:-0}" w d t h; w="/var/lib/goat-federation/$them"; d="$WORK/$s/applied-$them"; h="$(drv_host "$s")"
  rm -rf "$d"; mkdir -p "$d"
  # A phase that failed leaves no agreement or partner CA: recorded as a failed step, so the
  # run reports the check, instead of ending the whole run on docker cp's error.
  if ! docker cp -q "$h:$w/agreement.json" "$WORK/$s/agreement-$them.json" 2>/dev/null \
     || ! docker cp -q "$h:$w/theirs-ca.pem" "$d/theirs-ca.pem" 2>/dev/null; then
    rec "s$n-$s-review-and-admit-$them" -- bash -c "echo '$s: no agreement.json or theirs-ca.pem under $w on $h; an earlier phase did not produce them'; exit 1"
    return 0
  fi
  local want got
  want="$(python3 -c 'import json,sys; ag=json.load(open(sys.argv[1])); print([p["ca_fingerprint"] for p in ag["parties"] if p["sovereign_id"]==sys.argv[2]][0])' "$WORK/$s/agreement-$them.json" "$them")"
  got="sha256:$(openssl x509 -in "$d/theirs-ca.pem" -outform DER | sha256sum | cut -d' ' -f1)"
  [ "$want" = "$got" ] || die "$s: $them's CA does not match the agreement: $got, agreed $want"
  host_exec "$h" install -m 0644 "$w/theirs-ca.pem" "$w/pki/partner-ca.pem"
  # The core admits every peering admitted so far: one fragment per partner, reviewed
  # here, joined region by region (acl-assemble.py reads a repeated region header as
  # more of the same region).
  : > "$d/fragment.json5"
  for t in $(partners "$s"); do
    if docker exec "$h" test -s "/var/lib/goat-federation/$t/applied/ops/federation/acl/$t-fragment.json5"; then
      docker cp -q "$h:/var/lib/goat-federation/$t/applied/ops/federation/acl/$t-fragment.json5" "$d/fragment-$t.json5"
      cat "$d/fragment-$t.json5" >> "$d/fragment.json5"
    fi
  done
  if [ "$FE2E_CORE" = role ]; then admit_by_role "$s" "$them" "$d" "$n"; return 0; fi
  python3 "$ACL_ASSEMBLE" assemble --seed "$WORK/$s/core-seed.json5" --org "$d/fragment.json5" --out "$d/core.json5"
  docker cp -q "$d/core.json5" "$HP-$s-core:/etc/goat-core/config.json5"
  docker cp -q "$h:$w/pki/core.pem" "$d/core.pem"
  docker cp -q "$h:$w/pki/core-key.pem" "$d/core-key.pem"
  chmod 0644 "$d/core-key.pem"
  docker cp -q "$d/core.pem" "$HP-$s-core:/etc/goat-core/core.pem"
  docker cp -q "$d/core-key.pem" "$HP-$s-core:/etc/goat-core/core-key.pem"
  host_exec "$HP-$s-core" docker restart "$s-zenoh-router" >/dev/null
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
  host_exec "$HP-$s-edge" install -d -m 0755 "$dir"
  docker cp -q "$WORK/$s/probe-$them-$name.json5" "$HP-$s-edge:$dir/config.json5"
  # The probe's files are read where the driver keeps them (the edge, or the workspace) and
  # placed on the edge, where the probe runs.
  local f src; mkdir -p "$WORK/$s/probe-$them-$name"
  for f in "far-ca.pem:$w/pki/partner-ca.pem" "cert.pem:$cert" "key.pem:$key"; do
    src="${f#*:}"; f="${f%%:*}"
    docker cp -q "$(drv_host "$s"):$src" "$WORK/$s/probe-$them-$name/$f"
    chmod 0644 "$WORK/$s/probe-$them-$name/$f"
    docker cp -q "$WORK/$s/probe-$them-$name/$f" "$HP-$s-edge:$dir/$f"
  done
  host_exec "$HP-$s-edge" docker rm -f "fed-probe-$them-$name" >/dev/null 2>&1 || true
  host_exec "$HP-$s-edge" docker run -d --name "fed-probe-$them-$name" --network host --restart on-failure -v "$dir:/probe:ro" \
    -e RUST_LOG=zenoh=info "$ZENOH_IMAGE" -c /probe/config.json5 >/dev/null
  for _ in $(seq 1 30); do
    curl -s -m 2 -o /dev/null "http://$(edge_ip "$s"):$port/@/**" && return 0
    sleep 2
  done
  host_exec "$HP-$s-edge" docker logs "fed-probe-$them-$name" 2>&1 | tail -20 || true
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
    "FED_EDGE_LOG_OURS=docker exec $HP-$s-edge docker logs goat-zenoh-edge-$them"
    "FED_PROBE_THEIRS_REST=http://$(edge_ip "$them"):$(probe_port "$them" "$s" 8101)"
    "FED_PROBE_FORGED_REST=http://$(edge_ip "$them"):$(probe_port "$them" "$s" 8102)" "FED_PROBE_FORGED_ZID=$(probe_zid "$them-$s" forged)"
    "FED_PROBE_IMPERSONATE_REST=http://$(edge_ip "$them"):$(probe_port "$them" "$s" 8103)" "FED_PROBE_IMPERSONATE_ZID=$(probe_zid "$them-$s" impersonate)"
    "FED_SPIRE_OURS=docker exec $HP-$s-core docker exec spire-server /opt/spire/bin/spire-server"
    "FED_SPIRE_THEIRS=docker exec $HP-$them-core docker exec spire-server /opt/spire/bin/spire-server"
  )
  case "$avenue" in
    goatbuilder) env "${env[@]}" "$GB" -C "$WORK/$s/clone" --yes federate "$them" --as "$s" --work "$w" --from 14 --to 14 ;;
    script)      env "${env[@]}" "$WORK/$s/clone/ops/scripts/peering-standup.sh" "$them" --as "$s" --from 14 --to 14 ;;
  esac
}

# The helpers every `rec ... bash -c` runs in a child shell.
export_helpers() {
  export -f drive host_exec other party_pos octet core_ip edge_ip boundary_ip die partners pidx iface wgport \
    spire_out rp_metrics probe_port ctx_of pair_subnet nodir partner_list accept_as probe_zid td round_trip round_trip_eventually \
    core_deploy core_ca drv_host ops_ip
  for _p in $PARTIES; do export "P_$_p"; done
  export NODIR WORK GB REPO HERE SRC RECORD DIRECTORY_IP FE2E_DRIVER
}

# The hosts of every party in this scenario, the directory, and the isolation checks.
setup_hosts() { # $1 scenario number
  local n="$1" s a b
  teardown
  for s in $(partner_list); do docker network create "${LABEL[@]}" --subnet "$NB.$(octet "$s").0/24" "$NP-$s" >/dev/null; done
  docker network create "${LABEL[@]}" --internal --subnet "$NB.3.0/24" "$NP-boundary" >/dev/null
  for s in $(partner_list); do gen_sovereign "$s"; done
  for s in $(partner_list); do
    start_host "$HP-$s-core" "$s" "$(core_ip "$s")"
    # shellcheck disable=SC2046
    start_host "$HP-$s-edge" "$s" "$(edge_ip "$s")" $(solo_edge_args "$s")
    attach_boundary "$s"
    # The operator's workspace: the internal network only, never the boundary.
    [ "$FE2E_DRIVER" != workspace ] || start_host "$HP-$s-ops" "$s" "$(ops_ip "$s")"
  done
  for s in $(partner_list); do
    wait_ready "$HP-$s-core"; wait_ready "$HP-$s-edge"
    [ "$FE2E_DRIVER" != workspace ] || wait_ready "$HP-$s-ops"
  done
  for s in $(partner_list); do start_core "$s"; done
  for s in $(partner_list); do docker save "$RP_TAG" | docker exec -i "$HP-$s-core" docker load -q >/dev/null; done
  for s in $(partner_list); do host_exec "$(drv_host "$s")" sh -c "[ -d /opt/goat/.git ] || git clone -q /srv/goat.git /opt/goat"; done
  # A partner outside the lab: no directory (phase 5 by hand), and nothing to isolate from.
  [ "$FE2E_PARTNER" = external ] && return 0
  start_directory

  say "scenario $n: the isolation the rest of the run depends on"
  for a in $(partner_list); do
    for b in $(partner_list); do
      [ "$a" = "$b" ] && continue
      rec "s$n-$a-core-cannot-reach-$b-boundary" --expect-exit 1 -- docker exec "$HP-$a-core" ping -c1 -W2 "$(boundary_ip "$b")"
      rec "s$n-$a-core-cannot-reach-$b-core" --expect-exit 1 -- docker exec "$HP-$a-core" ping -c1 -W2 "$(core_ip "$b")"
      rec "s$n-$a-edge-cannot-reach-$b-core" --expect-exit 1 -- docker exec "$HP-$a-edge" ping -c1 -W2 "$(core_ip "$b")"
      rec "s$n-$a-edge-shares-the-boundary-with-$b" -- docker exec "$HP-$a-edge" ping -c1 -W2 "$(boundary_ip "$b")"
    done
  done
}

# The avenue one side drives: the first side (and both, by default) drives $3; the other
# side drives $4 when given. $1 the side, $2 the first side.
side_avenue() { if [ "$1" != "$2" ] && [ -n "${4:-}" ]; then echo "$4"; else echo "$3"; fi; }

# One peering, phases 1-14 from both sides. $1 scenario, $2 the side that listens and goes
# first, $3 the other side, $4 avenue, [$5 the other side's avenue, default $4].
peer_up() {
  local n="$1" first="$2" second="$3" avenue="$4" avenue2="${5:-}" s them i
  i="$(iface "$first" "$second")"
  prepare_edge "$first" "$second" true
  prepare_edge "$second" "$first" false
  local want5="exchange through the directory"; nodir "$first" "$second" && want5="exchange packages by hand"

  say "scenario $n: $first <-> $second phases 1-4, each side, up to the exchange ($avenue${avenue2:+ and $avenue2})"
  for s in "$first" "$second"; do
    them="$([ "$s" = "$first" ] && echo "$second" || echo "$first")"
    rec "s$n-$s-$them-phases-1-to-5" --expect-in "$want5" -- bash -c "drive '$s' '$them' '$(side_avenue "$s" "$first" "$avenue" "$avenue2")' 1 5"
  done
  exchange "$first" "$second" "$avenue" "$n"

  say "scenario $n: $first <-> $second phases 6-9, each side, up to the review"
  for s in "$first" "$second"; do
    them="$([ "$s" = "$first" ] && echo "$second" || echo "$first")"
    rec "s$n-$s-$them-phases-6-to-10" --expect-in "review what apply rendered" -- bash -c "drive '$s' '$them' '$(side_avenue "$s" "$first" "$avenue" "$avenue2")' 6 10"
  done
  rec "s$n-$first-$second-both-sides-hold-one-agreement" -- bash -c \
    "a=\$(docker exec $(drv_host "$first") sha256sum /var/lib/goat-federation/$second/agreement.json | cut -d' ' -f1); b=\$(docker exec $(drv_host "$second") sha256sum /var/lib/goat-federation/$first/agreement.json | cut -d' ' -f1); echo \"\$a \$b\"; [ -n \"\$a\" ] && [ \"\$a\" = \"\$b\" ]"
  rec "s$n-$first-$second-the-agreement-selected-wireguard" --expect-in '"kind":"wireguard"' -- docker exec "$(drv_host "$first")" cat "/var/lib/goat-federation/$second/agreement.json"
  review_and_admit "$first" "$second" "$n"
  review_and_admit "$second" "$first" "$n"

  say "scenario $n: $first <-> $second phases 11-12, each side: the transport, the edge, the republisher"
  for s in "$first" "$second"; do
    them="$([ "$s" = "$first" ] && echo "$second" || echo "$first")"
    rec "s$n-$s-$them-phases-11-to-12" --expect-in "open the wireguard transport" --expect-in "goat-fed-republisher-$them is running" -- bash -c "drive '$s' '$them' '$(side_avenue "$s" "$first" "$avenue" "$avenue2")' 11 12"
    rec "s$n-$s-$them-republisher-maps-both-sets" --expect-in "release release/all/** -> wire from/$s/all/**" --expect-in "release release/$them/** -> wire from/$s/$them/**" \
      -- docker exec "$HP-$s-core" docker logs "goat-fed-republisher-$them"
  done
  rec "s$n-$first-$second-a-wireguard-handshake" -- bash -c \
    "for _ in \$(seq 1 30); do t=\$(docker exec $HP-$first-edge wg show $i latest-handshakes | cut -f2); [ \"\${t:-0}\" != 0 ] && break; sleep 2; done; docker exec $HP-$first-edge wg show $i latest-handshakes | cut -f2; [ \"\${t:-0}\" != 0 ]"
  rec "s$n-$first-$second-fabric-port-closed-on-the-boundary" --expect-exit 1 -- docker exec "$HP-$first-edge" bash -c "exec 3<>/dev/tcp/$(boundary_ip "$second")/7448"

  say "scenario $n: $first <-> $second phase 13, each side: the SPIRE trust domains"
  for s in "$first" "$second"; do
    them="$([ "$s" = "$first" ] && echo "$second" || echo "$first")"
    rec "s$n-$s-$them-phase-13" --expect-in "federate the SPIRE trust domains" -- bash -c "drive '$s' '$them' '$(side_avenue "$s" "$first" "$avenue" "$avenue2")' 13 13"
  done

  say "scenario $n: $first <-> $second phase 14 from both sides, positives and refusals together"
  for s in "$first" "$second"; do
    them="$([ "$s" = "$first" ] && echo "$second" || echo "$first")"
    host_exec "$HP-$s-core" sh -c 'cat > /usr/local/bin/fed-responder.py' < "$SRC/ops/federation/roundtrip/responder.py"
    docker exec -d "$HP-$s-core" python3 /usr/local/bin/fed-responder.py "from/$them" "release/all"
    start_probe "$s" "$them" agreed "/var/lib/goat-federation/$them/pki/peering-outward.pem" "/var/lib/goat-federation/$them/pki/peering-outward-key.pem" "$(probe_port "$s" "$them" 8101)"
    start_probe "$s" "$them" forged /etc/fed-e2e/forged.pem /etc/fed-e2e/forged-key.pem "$(probe_port "$s" "$them" 8102)"
    start_probe "$s" "$them" impersonate "/etc/fed-e2e/impersonate-$them.pem" "/etc/fed-e2e/impersonate-$them-key.pem" "$(probe_port "$s" "$them" 8103)"
  done
  accept_pair "$n" "$first" "$second" "$avenue" "" "$avenue2"
}

accept_pair() { # $1 scenario, $2 a, $3 b, $4 avenue, $5 label suffix, [$6 b's avenue, default $4]
  local n="$1" a="$2" b="$3" avenue="$4" sfx="$5" avenue2="${6:-}" s them
  for s in "$a" "$b"; do
    them="$([ "$s" = "$a" ] && echo "$b" || echo "$a")"
    rec "s$n-acceptance-as-$s-with-$them$sfx" --expect-in "acceptance-counts:" --expect-in "skip=0" --expect-in "fail=0" --expect-not-in "ACCEPTANCE FAILED" \
      -- bash -c "accept_as '$s' '$them' '$(side_avenue "$s" "$a" "$avenue" "$avenue2")'"
  done
}

round_trip() { # $1 sender, $2 receiver, $3 seconds: one try
  BASE="http://$(core_ip "$1"):8000" "$SRC/ops/federation/roundtrip/fedcheck.sh" release/all "from/$2" "$3"
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
      "docker exec $HP-$s-core sh -c 'grep -c BEGIN $(core_ca "$s")' | grep -qx 1 && [ \"\$(docker exec $HP-$s-core cat $(core_ca "$s") | openssl x509 -noout -subject)\" = \"\$(openssl x509 -in '$WORK/$s/keys/ca.pem' -noout -subject)\" ]"
    rec "s$n-$s-tunnel-carried-it" -- bash -c \
      "docker exec $HP-$s-edge wg show $(iface "$s" "$them") transfer | awk '{print; if (\$2 < 1 || \$3 < 1) exit 1}'"
  done
  # A round trip that cannot fail is not a check: cut on one side, and it fails.
  # The republisher is the only writer of our wire name: stop it, and nothing crosses.
  rec "s$n-stop-the-republisher" -- docker exec "$HP-$first-core" docker stop "goat-fed-republisher-$second"
  rec "s$n-round-trip-fails-without-the-republisher" --expect-exit 1 --expect-in "FAIL" -- bash -c "round_trip '$first' '$second' 15"
  rec "s$n-start-the-republisher" -- docker exec "$HP-$first-core" docker start "goat-fed-republisher-$second"
  rec "s$n-round-trip-passes-with-the-republisher" --expect-in "PASS" -- bash -c "round_trip_eventually '$first' '$second'"
  rec "s$n-cut-the-link" -- docker exec "$HP-$second-edge" systemctl stop "goat-peering-wg@$(iface "$second" "$first")"
  rec "s$n-round-trip-fails-when-cut" --expect-exit 1 --expect-in "FAIL" -- bash -c "round_trip '$first' '$second' 15"
  rec "s$n-restore-the-link" -- docker exec "$HP-$second-edge" systemctl start "goat-peering-wg@$(iface "$second" "$first")"
  rec "s$n-round-trip-passes-again" --expect-in "PASS" -- bash -c "round_trip_eventually '$first' '$second'"
  # F-495: the role refuses without a recorded reason, and refuses any other transport.
  for s in "$first" "$second"; do
    them="$([ "$s" = "$first" ] && echo "$second" || echo "$first")"
    docker exec "$HP-$s-edge" cat "/var/log/goat-peering-wg-$(iface "$s" "$them")-state.json" > "$RECORD/s$n-$s-$them-wg-state.json" 2>&1 || true
  done
  local ans="cd /opt/goat/ops/ansible && GOAT_AWS_MODE=none GOAT_ANSIBLE_TRUNK_REMOTE=origin ../scripts/with-aws none -- ../scripts/ansible-preflight.sh -i inventory/peering-edges.ini --limit ${first}_edge -e fed_edge_group=${first}_edge -e fed_work=/var/lib/goat-federation/$first -e fed_counterparty=/var/lib/goat-federation/$first/theirs.canonical.json"
  rec "s$n-wireguard-refused-without-a-waiver" --expect-exit 2 --expect-in "goat-peering-wg refuses" -- docker exec "$HP-$second-edge" bash -c "$ans -e peering_wg_fips_waived_reason= peering-wg.yml"
  rec "s$n-wireguard-refused-under-another-transport" --expect-exit 2 --expect-in "goat-peering-wg refuses" -- docker exec "$HP-$second-edge" bash -c "$ans -e goat_peering_transport_kind=ipsec-fips -e peering_wg_kind=ipsec-fips peering-wg.yml"
  # The guards run on what this sovereign generated, in its own repository.
  for g in check-spire-federated-trust-files.sh check-federated-trust-retirement.sh; do
    rec "s$n-guard-${g%.sh}" -- bash -c "docker cp -q $HP-${A}-edge:/var/lib/goat-federation/${B}/applied/ops/ansible/group_vars/${B}_edge/. '$WORK/${A}/clone/ops/ansible/group_vars/${B}_edge/' && git -C '$WORK/${A}/clone' add -A && git -C '$WORK/${A}/clone' -c user.name=e2e -c user.email=e2e@localhost commit -q --allow-empty -m generated && '$WORK/${A}/clone/ops/ci/$g'"
  done
}

collect() { # $1 scenario
  local n="$1" s them
  for s in $(partner_list); do
    docker exec "$HP-$s-core" sh -c "docker logs $s-zenoh-router 2>&1 | tail -200" > "$RECORD/s$n-$s-core-router.log" 2>&1 || true
    docker exec "$HP-$s-core" sh -c "docker logs spire-server 2>&1 | tail -100" > "$RECORD/s$n-$s-spire.log" 2>&1 || true
    for them in $(partners "$s"); do
      docker exec "$HP-$s-edge" sh -c "docker logs goat-zenoh-edge-$them 2>&1 | tail -400" > "$RECORD/s$n-$s-$them-edge-router.log" 2>&1 || true
      docker exec "$HP-$s-core" sh -c "docker logs goat-fed-republisher-$them 2>&1 | tail -200" > "$RECORD/s$n-$s-$them-republisher.log" 2>&1 || true
      docker exec "$HP-$s-edge" wg show "$(iface "$s" "$them")" > "$RECORD/s$n-$s-$them-wg.txt" 2>&1 || true
    done
  done
}

scenario() { # $1 number, $2 first side (it also listens), $3 avenue
  local n="$1" first="$2" avenue="$3" second; second="$(other "$first")"
  set_partners "$A" "$B"; set_partners "$B" "$A"; set_partners "$C" ""; NODIR=""
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
  docker exec "$HP-$s-edge" bash -c "
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
  docker exec "$HP-$s-core" bash -c "
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
  local pd="-e fed_work=/var/lib/goat-federation/${C} -e fed_counterparty=/var/lib/goat-federation/${C}/theirs.canonical.json -e fed_edge_group=${C}_edge"
  rec "s$n-a-second-peering-may-not-take-the-first-ones-interface" --expect-exit 2 --expect-in "carries the peering with ${B}" -- \
    docker exec $HP-${A}-edge bash -c "$a --limit ${C}_edge $pd -e peering_wg_interface=wg-fed0 peering-wg.yml"
  rec "s$n-a-second-peering-may-not-take-the-first-ones-port" --expect-exit 2 --expect-in "listen port 51820" -- \
    docker exec $HP-${A}-edge bash -c "$a --limit ${C}_edge $pd -e peering_wg_listen_port=51820 peering-wg.yml"
  rec "s$n-a-second-republisher-may-not-take-the-first-ones-metrics-port" --expect-exit 2 --expect-in "goat-fed-republisher-${B}" -- \
    docker exec $HP-${A}-edge bash -c "$a --limit ${C}_core $pd -e goat_fed_republisher_metrics_port=9310 peering-republisher.yml"
  # One inventory name in two peering groups: the merge the per-peering names prevent.
  rec "s$n-preflight-refuses-one-inventory-name-in-two-peerings" --expect-exit 1 --expect-in "${A}-edge-${B} is in" -- bash -c \
    "rm -rf '$WORK/merged' && cp -R '$WORK/${A}/clone' '$WORK/merged' && sed -i 's/^${A}-edge-${C} /${A}-edge-${B} /' '$WORK/merged/ops/ansible/inventory/peering-edges.ini' && python3 '$WORK/merged/ops/federation/standup/preflight.py' ${C} --work '$WORK/merged-work' --edge-group ${C}_edge --repo '$WORK/merged'"
}

scenario3() { # [$1 avenue of the first peering, default goatbuilder] [$2 of the second, default script]
  local n=3 av1="${1:-goatbuilder}" av2="${2:-script}"
  set_partners "$A" "$B $C"; set_partners "$B" "$A"; set_partners "$C" "$A"; NODIR="$A:$C $C:$A"
  export_helpers
  say "scenario 3: ${A} federates with ${B} and with ${C} on one edge host and one core host"
  setup_hosts "$n"

  # The first peering, through the driver's $av1 avenue and the directory.
  peer_up "$n" ${A} ${B} $av1
  snap ${A} ${B} ${C} > "$RECORD/s3-${B}-before-${C}.txt"
  docker exec $HP-${A}-edge sha256sum /var/lib/goat-federation/${B}/agreement.json | cut -d' ' -f1 > "$WORK/${A}-${B}-agreement.sha"
  docker exec $HP-${A}-edge sha256sum /var/lib/goat-federation/${B}/ours.canonical.json | cut -d' ' -f1 > "$WORK/${A}-${B}-package.sha"

  # The second peering, added beside it: peering-standup.sh, packages by hand.
  peer_up "$n" ${A} ${C} $av2
  snap ${A} ${B} ${C} > "$RECORD/s3-${B}-after-${C}.txt"
  # Phase 10 for ${C} restarts ${A}'s core on the joined ACL, so the ${B}
  # peering reconnects; nothing of its own may have changed.
  rec "s3-adding-${C}-left-${B}-untouched" -- diff "$RECORD/s3-${B}-before-${C}.txt" "$RECORD/s3-${B}-after-${C}.txt"
  rec "s3-adding-${C}-left-${B}-agreement-digest" --expect-in "$(cat "$WORK/${A}-${B}-agreement.sha")" -- docker exec $HP-${A}-edge sha256sum /var/lib/goat-federation/${B}/agreement.json
  accept_pair "$n" ${A} ${B} $av1 "-after-${C}"

  say "scenario 3: each partner receives the sets its own agreement offers, and no other"
  set_isolation "$n" ${A} both-up
  # Each offered set is lifted once per peering that offers it: two writers of a set
  # both agreements offer. A measurement, not a check.
  for p in 9310 9311; do
    docker exec $HP-${A}-core curl -s "http://127.0.0.1:$p/metrics" 2>&1 | grep -E 'published_total' || true
  done > "$RECORD/s3-shared-set-writers.txt"

  say "scenario 3: re-running one peering's phases leaves the other untouched"
  snap ${A} ${C} ${B} > "$RECORD/s3-${C}-before-${B}-rerun.txt"
  rec "s3-rerun-${B}-phase-2" -- bash -c "drive ${A} ${B} $av1 2 2"
  rec "s3-rerun-${B}-phases-7-to-9" -- bash -c "drive ${A} ${B} $av1 7 9"
  rec "s3-rerun-${B}-phases-11-to-13" -- bash -c "drive ${A} ${B} $av1 11 13"
  snap ${A} ${C} ${B} > "$RECORD/s3-${C}-after-${B}-rerun.txt"
  rec "s3-${B}-rerun-left-${C}-untouched" -- diff "$RECORD/s3-${C}-before-${B}-rerun.txt" "$RECORD/s3-${C}-after-${B}-rerun.txt"
  rec "s3-${C}-round-trip-after-${B}-rerun" --expect-in "PASS" -- bash -c "round_trip_eventually ${A} ${C}"

  snap ${A} ${B} ${C} > "$RECORD/s3-${B}-before-${C}-rerun.txt"
  rec "s3-rerun-${C}-phase-2" -- bash -c "drive ${A} ${C} $av2 2 2"
  rec "s3-rerun-${C}-phases-7-to-9" -- bash -c "drive ${A} ${C} $av2 7 9"
  rec "s3-rerun-${C}-phases-11-to-13" -- bash -c "drive ${A} ${C} $av2 11 13"
  snap ${A} ${B} ${C} > "$RECORD/s3-${B}-after-${C}-rerun.txt"
  rec "s3-${C}-rerun-left-${B}-untouched" -- diff "$RECORD/s3-${B}-before-${C}-rerun.txt" "$RECORD/s3-${B}-after-${C}-rerun.txt"
  rec "s3-${B}-round-trip-after-${C}-rerun" --expect-in "PASS" -- bash -c "round_trip_eventually ${A} ${B}"
  accept_pair "$n" ${A} ${B} $av1 "-after-reruns"
  accept_pair "$n" ${A} ${C} $av2 "-after-reruns"
  set_isolation "$n" ${A} after-reruns

  say "scenario 3: the refusals that keep two peerings on one host apart"
  guard_refusals "$n"

  say "scenario 3: the directory still names the package ${B} composed against"
  local dir_ptr="curl -fsS --cacert /etc/fed-e2e/directory-ca.pem https://$DIRECTORY_IP:8443/v1/parties/${A}"
  rec "s3-directory-names-the-${B}-package" --expect-in "$(cat "$WORK/${A}-${B}-package.sha")" -- docker exec $HP-${A}-edge bash -c "$dir_ptr"
  # What ${B} would see if ${A} published its ${C} package as its one entry:
  # the entry and its transport offer change; ${B}'s running peering does not.
  rec "s3-what-${B}-sees-if-${A}-republishes" --expect-in "$(boundary_ip ${A}):$(wgport ${A} ${C})" -- docker exec $HP-${A}-edge bash -c \
    "SSL_CERT_FILE=/etc/fed-e2e/directory-ca.pem python3 /opt/goat/ops/federation/standup/directory.py --url https://$DIRECTORY_IP:8443 --ours /var/lib/goat-federation/${C}/ours.canonical.json --key /etc/fed-e2e/signing.key --ca /etc/fed-e2e/ca.pem --their-id ${B} --work /tmp/republish-demo >/dev/null; $dir_ptr; echo; curl -fsS --cacert /etc/fed-e2e/directory-ca.pem https://$DIRECTORY_IP:8443/v1/parties/${A}/package | python3 -c 'import json,sys; print(json.load(sys.stdin)[\"transport_offer\"])'"
  # What the first partner composes when it reruns phases 5 to 7 against that entry.
  rec "s3-${B}-rerun-5-to-7-composes-the-same-agreement" --expect-in "$(cat "$WORK/${A}-${B}-agreement.sha")" -- bash -c \
    "drive ${B} ${A} $av1 5 7 >/dev/null 2>&1; docker exec $HP-${B}-edge sha256sum /var/lib/goat-federation/${A}/agreement.json"
  rec "s3-${B}-still-carries-after-the-republish" --expect-in "PASS" -- bash -c "round_trip_eventually ${A} ${B}"
  collect "$n"
}
