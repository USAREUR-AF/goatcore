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
# Four scenarios:
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
#   4  nothing-hand-written, strict (units/nohand.sh): one sovereign declares only its
#      peering entry and offer, both cores are the goat-zenoh role's and declare only
#      site_id, and admission's values are withheld. The command must go 1 to 14 on both
#      sides with nothing given by hand; the job fails on its first refusal and on any
#      hand-written line outside the reviewed lab-only set. It runs in its own lab (its own
#      host names and subnets) through lib/unit.sh, as ops/federation-e2e/env runs it.
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
#                                  [--scenario 1|2|3|4] [--keep] [--prepare-only] [--src DIR]
#                                  [--images-only DIR | --images DIR]
#
# --images-only DIR builds the host and republisher images, saves them to DIR with the ids
# they are named by (images.env) and stops: the CI build stage. --images DIR loads those
# instead of building them: each scenario stage runs on the images the build stage made.
#
# --src names the goatcore tree under test (default: this checkout): its driver, plays,
# roles and scripts are what run; the harness is this directory. The names, subnets and
# party ids the run creates come from FE2E_* (lib/engine.sh).
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"
WORK=""; RECORD=""; GOAT=""; GB=""; KEEP=0; ONLY=""; PREPARE=0; SRC=""; IMAGES_OUT=""; IMAGES_IN=""
# KEEP is read by lib/engine.sh (final); exported for the units it starts.
export KEEP
while [ $# -gt 0 ]; do
  case "$1" in
    --goat) GOAT="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"; shift 2 ;;
    --goatbuilder) GB="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"; shift 2 ;;
    --work) WORK="$2"; shift 2 ;;
    --record) RECORD="$2"; shift 2 ;;
    --src) SRC="$(cd "$2" && pwd)"; shift 2 ;;
    --scenario) ONLY="$2"; shift 2 ;;
    --keep) KEEP=1; shift ;;
    --prepare-only) PREPARE=1; shift ;;
    --images-only) IMAGES_OUT="$2"; shift 2 ;;
    --images) IMAGES_IN="$2"; shift 2 ;;
    -h|--help) sed -n '2,48p' "$0"; exit 0 ;;
    *) echo "run.sh: unknown argument $1" >&2; exit 2 ;;
  esac
done
if [ -n "$IMAGES_OUT" ] && { [ -n "$IMAGES_IN$ONLY" ] || [ "$PREPARE" = 1 ]; }; then
  echo "run.sh: --images-only takes no --images, --scenario or --prepare-only" >&2; exit 2
fi
if [ -n "$IMAGES_IN" ] && [ "$PREPARE" = 1 ]; then
  echo "run.sh: --prepare-only builds and loads no images; it takes no --images" >&2; exit 2
fi
if [ -n "$IMAGES_OUT" ]; then
  :
elif [ "$PREPARE" = 1 ]; then
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

# The images, the names, and every function the run calls.
# shellcheck source=lib/engine.sh
. "$HERE/lib/engine.sh"
trap final EXIT

# --- what needs no hosts: the driver's own refusals ------------------------------------
# Not in the image build stage (--images-only), which has no binaries: each scenario stage runs them.
if [ -z "$IMAGES_OUT" ]; then
  say "the driver's refusals, no hosts"
  FX="$SRC/ops/goatbuilder/fixtures/goatnet-from-scratch/03-boundary-node"
  SENT="$WORK/mock-ran"; printf '#!/bin/sh\ntouch %s\n' "$SENT" > "$WORK/mock-goat"; chmod +x "$WORK/mock-goat"
  rec mock-executes-nothing --expect-in "MOCK" --expect-in "not executed" -- bash -c \
    "GOAT_BIN='$WORK/mock-goat' '$GB' -C '$SRC' --mock '$FX' federate hello-ally --as zagros --work '$WORK/mock-work' && [ ! -e '$SENT' ] && [ ! -e '$WORK/mock-work' ]"
  rec an-undeclared-peering-is-refused --expect-in "declare it first (C1)" -- bash -c \
    "if '$GB' -C '$SRC' --mock '$FX' federate no-such-peer --dry-run 2>&1; then exit 1; fi"
  mkdir -p "$WORK/quic"; printf '{"transport":{"kind":"quic"}}' > "$WORK/quic/agreement.json"
  rec an-unknown-transport-refuses-and-substitutes-nothing --expect-in "REFUSED, no play for 'quic'" --expect-in "Do NOT substitute" --expect-not-in "ansible-preflight" -- bash -c \
    "'$GB' -C '$SRC' --mock '$FX' federate hello-ally --work '$WORK/quic' --from 11 --to 11 --dry-run 2>&1; FED_WORK='$WORK/quic' '$SRC/ops/scripts/peering-standup.sh' hello-ally --from 11 --to 11 2>&1 && exit 1; exit 0"
fi

if [ "$PREPARE" = 1 ]; then
  say "the two sovereigns, generated and planned, no hosts"
  set_partners "$A" "$B"; set_partners "$B" "$A"; set_partners "$C" ""
  for s in "$A" "$B"; do
    gen_sovereign "$s"
    rec "prepare-$s-config-validates" -- "$GB" -C "$WORK/$s/clone" validate
    rec "prepare-$s-secrets-posture-is-env" --expect-in "env" --expect-not-in "env-file" -- env -u GOAT_SECRET_STORE python3 "$WORK/$s/clone/ops/secrets/posture.py" store
    rec "prepare-$s-federation-plans" --expect-in "To run it for real" -- "$GB" -C "$WORK/$s/clone" federate "$(other "$s")" --as "$s" --dry-run
    rec "prepare-$s-edge-pins-the-image" --expect-in "$ZENOH_IMAGE" -- cat "$WORK/$s/clone/ops/ansible/group_vars/$(other "$s")_edge/deployment.yml"
  done
  echo "spire image: $SPIRE_IMAGE"
  [ "$FAILS" = 0 ] && exit 0 || exit 1
fi

# An image's config digest: the id a classic image store (each host's dockerd) gives it after
# `docker load`. A containerd image store on the runner reports the manifest digest as .Id
# instead, so ids are always read from `docker save`'s manifest.
config_digest() { echo "sha256:$(docker save "$1" | tar -xO manifest.json | python3 -c 'import json,sys; print(json.load(sys.stdin)[0]["Config"].rsplit("/", 1)[-1].removesuffix(".json").removeprefix("sha256:"))')"; }

if [ -n "$IMAGES_IN" ]; then
  # The images a build stage made (--images-only), loaded as they are: every scenario runs on
  # exactly the image the build job produced.
  say "the host and republisher images, from $IMAGES_IN"
  [ -s "$IMAGES_IN/images.env" ] || die "--images $IMAGES_IN holds no images.env (make it with --images-only)"
  HOST_IMAGE=""; HOST_IMAGE_ID=""; RP_TAG=""; RP_IMAGE=""
  # shellcheck source=/dev/null
  . "$IMAGES_IN/images.env"
  # All four, or the engine's default tags would quietly name whatever local image carries them.
  [ -n "$HOST_IMAGE" ] && [ -n "$RP_TAG" ] || die "$IMAGES_IN/images.env does not name both images (HOST_IMAGE, RP_TAG)"
  [[ "$HOST_IMAGE_ID" =~ ^sha256:[0-9a-f]{64}$ ]] && [[ "$RP_IMAGE" =~ ^sha256:[0-9a-f]{64}$ ]] \
    || die "$IMAGES_IN/images.env does not record both images' digests (HOST_IMAGE_ID, RP_IMAGE)"
  docker load -q -i "$IMAGES_IN/host.tar" >/dev/null || die "the host image in $IMAGES_IN did not load"
  got="$(config_digest "$HOST_IMAGE")"
  [ "$got" = "$HOST_IMAGE_ID" ] || die "the host image loaded from $IMAGES_IN is $got, not the $HOST_IMAGE_ID the build recorded"
  docker load -q -i "$IMAGES_IN/republisher.tar" >/dev/null || die "the republisher image in $IMAGES_IN did not load"
  got="$(config_digest "$RP_TAG")"
  [ "$got" = "$RP_IMAGE" ] || die "the republisher image loaded from $IMAGES_IN is $got, not the $RP_IMAGE the build recorded"
  echo "host image: $HOST_IMAGE ($HOST_IMAGE_ID); republisher image: $RP_IMAGE"
else
say "the host image"
ctx="$WORK/host-context"; rm -rf "$ctx"; cp -R "$HERE/host" "$ctx"
# Not -q: a failed build has to say why in the job log.
# Its packages come from the Ubuntu mirror while it builds (a declared input of the job): a
# build that failed because the mirror did not answer exits as environment.
docker build --progress=plain -t "$HOST_IMAGE" "$ctx" 2>&1 | tee "$WORK/host-image-build.log"
if [ "${PIPESTATUS[0]}" != 0 ]; then
  # Only the failing build step's own lines count: a fetch that apt retried and got, followed
  # by a real error (an unknown package, a failing RUN), is a defect of the Dockerfile.
  why="$(python3 "$HERE/lib/build-failure.py" "$WORK/host-image-build.log")"
  case "$why" in
    network:*) env_fault "the host image did not build: the Ubuntu mirror did not answer (${why#network: })" ;;
    *) die "the host image did not build (host/Dockerfile): ${why#other: }" ;;
  esac
fi

say "the republisher image, from this checkout"
docker build --progress=plain -t "$RP_TAG" "$SRC/ops/federation/republisher"
# The image's config digest: the id a classic image store (each host's dockerd) gives
# it after `docker load`. A containerd image store on the runner reports the manifest
# digest as .Id instead, which the hosts would not know.
RP_IMAGE="$(config_digest "$RP_TAG")"
[[ "$RP_IMAGE" =~ ^sha256:[0-9a-f]{64}$ ]] || die "the republisher image has no content-addressed id: '$RP_IMAGE'"
echo "republisher image: $RP_IMAGE"
fi

if [ -n "$IMAGES_OUT" ]; then
  # The build stage: both images saved for the scenario stages, with the ids they are named by.
  mkdir -p "$IMAGES_OUT"
  docker save -o "$IMAGES_OUT/host.tar" "$HOST_IMAGE"
  docker save -o "$IMAGES_OUT/republisher.tar" "$RP_TAG"
  HOST_IMAGE_ID="$(config_digest "$HOST_IMAGE")"
  [[ "$HOST_IMAGE_ID" =~ ^sha256:[0-9a-f]{64}$ ]] || die "the host image has no content-addressed id: '$HOST_IMAGE_ID'"
  printf 'HOST_IMAGE=%q\nHOST_IMAGE_ID=%q\nRP_TAG=%q\nRP_IMAGE=%q\n' "$HOST_IMAGE" "$HOST_IMAGE_ID" "$RP_TAG" "$RP_IMAGE" > "$IMAGES_OUT/images.env"
  echo "images saved to $IMAGES_OUT: host $HOST_IMAGE_ID, republisher $RP_IMAGE"
  exit 0
fi

if [ -z "$ONLY" ] || [ "$ONLY" = 1 ]; then scenario 1 "$A" goatbuilder; fi
if [ -z "$ONLY" ] || [ "$ONLY" = 2 ]; then scenario 2 "$B" script; fi
if [ -z "$ONLY" ] || [ "$ONLY" = 3 ]; then scenario3; fi
if [ -z "$ONLY" ] || [ "$ONLY" = 4 ]; then
  say "scenario 4: nothing-hand-written, strict, in its own lab"
  # The record directory is named for the unit, as env lays it out (<avenue>/<unit>):
  # lib/verdict.py reads a unit's steps only from a directory of that name.
  NH_WORK="$WORK/nohand"; NH_REC="$RECORD/nohand"
  # Its own names and subnets, apart from scenarios 1-3; lib/unit.sh removes what it made.
  env FE2E_LAB=fed-e2e-nh FE2E_HOST_PREFIX=fed-nh FE2E_NET_PREFIX=fed-e2e-nh FE2E_NET_BASE=10.78 \
    FE2E_PARTIES="xaa xab" FE2E_HOST_IMAGE="$HOST_IMAGE" FE2E_RP_TAG="$RP_TAG" RP_IMAGE="$RP_IMAGE" \
    FE2E_NH_MODE=strict FE2E_UNIT_TIMEOUT="${FE2E_NH_TIMEOUT:-2700}" \
    SRC="$SRC" GOAT="$GOAT" GB="$GB" AVENUE=goatbuilder WORK="$NH_WORK" RECORD="$NH_REC" KEEP="$KEEP" \
    bash "$HERE/lib/unit.sh" nohand > "$RECORD/nothing-hand-written.log" 2>&1 || true
  python3 "$HERE/lib/verdict.py" --catalog "$HERE/scenarios/catalog.yaml" --record "$NH_REC" \
    --ref-name "$(git -C "$SRC" rev-parse --abbrev-ref HEAD 2>/dev/null || echo tree)" --ref-sha "$(git -C "$SRC" rev-parse HEAD)" \
    --avenue goatbuilder --harness-sha "$(git -C "$REPO" rev-parse HEAD)" --started "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --only nothing-hand-written --harness-fp run.sh \
    --out "$RECORD/nothing-hand-written.json" --markdown "$RECORD/nothing-hand-written.md" || true
  nh_outcome="$(python3 -c 'import json,sys; print([s["outcome"] for s in json.load(open(sys.argv[1]))["scenarios"] if s["id"]=="nothing-hand-written"][0])' "$RECORD/nothing-hand-written.json" 2>/dev/null || echo "no result")"
  echo "nothing-hand-written: $nh_outcome"
  echo "written by a person besides the operator input:"; cat "$NH_REC/hand-written.txt" 2>/dev/null || echo none
  [ -s "$NH_REC/loop.md" ] && { echo "refusals:"; cat "$NH_REC/loop.md"; }
  [ "$nh_outcome" = PASS ] || FAILS=$((FAILS + 1))
fi

say "the record"
{
  grep -h "acceptance-counts:" "$RECORD"/*acceptance-as-*.log "$RECORD"/nohand/*acceptance-as-*.log 2>/dev/null || true
} | tee "$RECORD/acceptance-counts.txt"
# The record step is a step: a report that cannot be written fails the run.
python3 "$REPO/ops/e2e/report.py" "$RECORD" --check --title "cross-sovereign federation e2e (hermetic)" || { echo "run.sh: the run record did not validate (above)" >&2; FAILS=$((FAILS + 1)); }
echo "record: $RECORD"
[ "$FAILS" = 0 ] && exit 0 || exit 1
