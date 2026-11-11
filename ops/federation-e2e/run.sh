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
#                                  [--scenario 1|2|3] [--keep] [--prepare-only] [--src DIR]
#
# --src names the goatcore tree under test (default: this checkout): its driver, plays,
# roles and scripts are what run; the harness is this directory. The names, subnets and
# party ids the run creates come from FE2E_* (lib/engine.sh).
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"
WORK=""; RECORD=""; GOAT=""; GB=""; KEEP=0; ONLY=""; PREPARE=0; SRC=""
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
    -h|--help) sed -n '2,48p' "$0"; exit 0 ;;
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

# The images, the names, and every function the run calls.
# shellcheck source=lib/engine.sh
. "$HERE/lib/engine.sh"
trap final EXIT

# --- what needs no hosts: the driver's own refusals ------------------------------------
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

say "the host image"
ctx="$WORK/host-context"; rm -rf "$ctx"; cp -R "$HERE/host" "$ctx"
# Not -q: a failed build has to say why in the job log.
docker build --progress=plain -t "$HOST_IMAGE" "$ctx"

say "the republisher image, from this checkout"
docker build --progress=plain -t "$RP_TAG" "$SRC/ops/federation/republisher"
# The image's config digest: the id a classic image store (each host's dockerd) gives
# it after `docker load`. A containerd image store on the runner reports the manifest
# digest as .Id instead, which the hosts would not know.
RP_IMAGE="sha256:$(docker save "$RP_TAG" | tar -xO manifest.json | python3 -c 'import json,sys; print(json.load(sys.stdin)[0]["Config"].rsplit("/", 1)[-1].removesuffix(".json").removeprefix("sha256:"))')"
[[ "$RP_IMAGE" =~ ^sha256:[0-9a-f]{64}$ ]] || die "the republisher image has no content-addressed id: '$RP_IMAGE'"
echo "republisher image: $RP_IMAGE"

if [ -z "$ONLY" ] || [ "$ONLY" = 1 ]; then scenario 1 "$A" goatbuilder; fi
if [ -z "$ONLY" ] || [ "$ONLY" = 2 ]; then scenario 2 "$B" script; fi
if [ -z "$ONLY" ] || [ "$ONLY" = 3 ]; then scenario3; fi

say "the record"
{
  grep -h "acceptance-counts:" "$RECORD"/*acceptance-as-*.log || true
} | tee "$RECORD/acceptance-counts.txt"
# The record step is a step: a report that cannot be written fails the run.
python3 "$REPO/ops/e2e/report.py" "$RECORD" --check --title "cross-sovereign federation e2e (hermetic)" || { echo "run.sh: the run record did not validate (above)" >&2; FAILS=$((FAILS + 1)); }
echo "record: $RECORD"
[ "$FAILS" = 0 ] && exit 0 || exit 1
