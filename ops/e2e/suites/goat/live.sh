#!/usr/bin/env bash
# live.sh — the goat CLI suite's live tiers on a real goatnet
# (docs/design/goat-cli-e2e-testing.md §T2 and §T3; docs/design/deployment-e2e.md §Suites).
#
#   ops/e2e/suites/goat/live.sh [--goatnet zagros-dev] [--context <goat context>] [--record <dir>]
#
# T2 — the SERVED binaries work. Download goat (and goatbuilder) from the goatnet's
#      get.<site>, check each against the SHA256SUMS served beside it, require the
#      binary's --version to equal the version.json the site advertises, and run a
#      real round trip on the served goat (`hello --sandbox --json`).
# T3 — live-fire with an operator context on that goatnet: doctor with every step
#      forced (router TCP, mTLS session, portal), the read surfaces' machine output
#      (org ls, topic ls --json, admin queue ls --json), and a real hello round
#      trip through the fabric. Mutations are synthetic and clean up after
#      themselves: the onboarding leg reuses ops/scripts/onboard-intake-smoke.sh,
#      which mints a one-time invite, submits it and rejects the proposal it made.
#      Nothing here touches a real person, and nothing deploys.
#
# Runs from a machine on the goatnet's mesh (the operator laptop, or a runner on
# the mesh); get.<site> and the portal are mesh-only. Exit 0 = every row passed.
set -uo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../../../.." && pwd)"
GOATNET=zagros-dev
CTX=""
record_dir=""
while [ $# -gt 0 ]; do
  case "$1" in
    --goatnet) GOATNET="$2"; shift 2 ;;
    --context) CTX="$2"; shift 2 ;;
    --record) record_dir="$2"; shift 2 ;;
    *) echo "usage: $0 [--goatnet <name>] [--context <goat context>] [--record <dir>]" >&2; exit 2 ;;
  esac
done
CTX="${CTX:-$GOATNET}"
case "$GOATNET" in
  zagros-dev) GET="https://get.zagros-dev.examplebird.zagros-dev.example"; VIA="goat@100.127.0.148" ;;
  *) echo "live.sh: no get.<site> declared here for $GOATNET (add its URL and runner)" >&2; exit 2 ;;
esac
ST="$(mktemp -d "${TMPDIR:-/tmp}/goat-live.XXXXXX")"
trap 'rm -rf "$ST"' EXIT
export E2E_RECORD_DIR="${record_dir:-$ST/record}"
export GOAT_CONTEXT="$CTX" GOAT_NO_FUN=1 NO_COLOR=1
OBJ="python3 $HERE/obj.py"
FAIL=0
rec() { python3 "$REPO/ops/e2e/rec.py" "$@" || FAIL=1; }

OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m | sed 's/x86_64/amd64/;s/aarch64/arm64/')"
SUM() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$@"; else shasum -a 256 "$@"; fi; }

# ---- T2: the served binaries ---------------------------------------------------------
FETCH="$HERE/fetch-served.sh"
rec t2-goat-served-and-checksummed -- "$FETCH" "$GET" "goat-$OS-$ARCH" "$ST/served"
SERVED_GOAT_VERSION="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["version"])' "$ST/served/version.json" 2>/dev/null)"
rec t2-goat-version-is-the-advertised-one --expect-in "goat $SERVED_GOAT_VERSION" -- "$ST/served/goat-$OS-$ARCH" --version
rec t2-goat-served-round-trip -- bash -c "\"$ST/served/goat-$OS-$ARCH\" hello --sandbox --json | $OBJ --stdin 'd[\"mode\"]==\"sandbox\" and d[\"round_trip\"]'"
rec t2-goatbuilder-served-and-checksummed -- "$FETCH" "$GET/goatbuilder" "goatbuilder-$OS-$ARCH" "$ST/served-gb"
SERVED_GB_VERSION="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["version"])' "$ST/served-gb/version.json" 2>/dev/null)"
rec t2-goatbuilder-version-is-the-advertised-one --expect-in "goatbuilder $SERVED_GB_VERSION" -- "$ST/served-gb/goatbuilder-$OS-$ARCH" --version

# ---- T3: live-fire with the served goat, on the real context ----------------------
# The operator context lives where ops/validation/goatnet-runners.yaml says goat
# runs for this goatnet (goat_via): on the management host for zagros-dev. The
# served linux binary is copied there to a temporary path, run against the
# host's own context, and removed on exit — the same place the release rows run.
HOSTARCH="$(ssh -o BatchMode=yes -o ConnectTimeout=10 "$VIA" uname -m 2>/dev/null | sed 's/x86_64/amd64/;s/aarch64/arm64/')"
REMOTE="/tmp/goat-e2e-live-$$"
# The host fetches the served binary from its own get.<site> (local, fast) and
# checks it against the SHA256SUMS served beside it before anything runs.
rec t3-served-linux-goat-on-the-host --expect-in "OK" -- ssh -o BatchMode=yes -o ConnectTimeout=10 "$VIA" \
  "set -e; curl -fsS --max-time 300 -o $REMOTE $GET/goat-linux-$HOSTARCH; curl -fsS --max-time 30 $GET/SHA256SUMS | grep ' goat-linux-$HOSTARCH\$' | sed 's# goat-linux-$HOSTARCH\$# $REMOTE#' | sha256sum -c -; chmod +x $REMOTE"
trap 'ssh -o BatchMode=yes "$VIA" rm -f "$REMOTE" 2>/dev/null; rm -rf "$ST"' EXIT
g() { ssh -o BatchMode=yes -o ConnectTimeout=10 "$VIA" "GOAT_CONTEXT=$CTX GOAT_NO_FUN=1 NO_COLOR=1 $REMOTE $*"; }
export -f g; export VIA CTX REMOTE
# The fabric cases need a context holding a slot certificate (mtls.cert.pem). The
# runner's context today is portal-only; without one they are recorded as expected
# failures naming the remedy, never as passes.
FX=()
ssh -o BatchMode=yes -o ConnectTimeout=10 "$VIA" "test -s ~/.goat/contexts/$CTX/mtls.cert.pem" 2>/dev/null \
  || FX=(--xfail "the $CTX context on $VIA has no slot certificate: issue the e2e runner a slot bundle (goat profile init <bundle.cbor>) to run the fabric cases")
rec t3-doctor-portal -- bash -c "g doctor --json --all | $OBJ --ndjson '[x for x in d if x[\"name\"]==\"portal-api-orgs\"][0][\"verdict\"]==\"ok\"'"
rec t3-doctor-mtls-session "${FX[@]}" -- bash -c "g doctor --json --all | $OBJ --ndjson '[x for x in d if x[\"name\"]==\"zenoh-mtls-session\"][0][\"verdict\"]==\"ok\"'"
rec t3-org-ls -- bash -c "g org ls"
# One org's topics, not the whole goatnet's: the served goat 0.5.13 lists the
# whole goatnet with one request per org, which the portal rate-limits here.
# From the release that carries #2088 it reads /api/catalog in one request;
# the whole-goatnet case is in the hermetic tier (run.sh) until then.
FIRST_ORG="$(g org ls 2>/dev/null | awk 'NR>1 && NF {print $1; exit}')"
rec t3-topic-ls-json-one-org -- bash -c "g topic ls $FIRST_ORG --json | $OBJ --ndjson 'all(\"key_pattern\" in x for x in d)'"
rec t3-queue-ls-json -- bash -c "g admin queue ls --env $GOATNET --json | $OBJ --stdin 'isinstance(d, list)'"
rec t3-hello-real-round-trip "${FX[@]}" -- bash -c "g hello --json | $OBJ --stdin 'd[\"mode\"]!=\"sandbox\" and d[\"round_trip\"]'"
# The onboarding leg: a synthetic candidate, invited, submitted in a real browser,
# then rejected so nothing is left in anyone's queue (the existing release row).
rec t3-intake-synthetic-round-trip --expect-in "smoke: PASS" -- "$REPO/ops/scripts/onboard-intake-smoke.sh" --context "$CTX" --env "$GOATNET" --goat-via "$VIA"

python3 "$REPO/ops/e2e/report.py" "$E2E_RECORD_DIR" 2>/dev/null | tail -30 || true
exit "$FAIL"
