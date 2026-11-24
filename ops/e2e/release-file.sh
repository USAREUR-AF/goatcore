#!/usr/bin/env bash
# release-file.sh: a release component (fleet-pins artifact_type: release, gate
# J23) deployed, verified and rolled back for real, on a container host, by both
# avenues. docs/design/goatbuilder-build-targets.md section 3.9.
#
# The hermetic tier's pattern (ops/e2e/hermetic.sh): the real drivers, on a fresh
# clone of this commit whose declarations are the hermetic overlay
# (ops/e2e/overlay.py), a bare remote as its trunk, and a container that looks
# like a management host. The component is goatbuilder's own published 0.7.2 and
# 0.7.3, fetched from GitHub and verified against the public Sigstore. Unlike
# hermetic.sh it needs no registry: it changes nothing on this machine outside
# --work and its own container and network, and it needs no sudo.
#
# usage: release-file.sh --goatbuilder BIN [--work DIR] [--record DIR] [--keep]
#                        [--host-distro ubuntu|debian]
# Exit 0 when every case passed; 1 otherwise.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"
WORK=""; RECORD=""; GB=""; KEEP=0; DISTRO=ubuntu
while [ $# -gt 0 ]; do
  case "$1" in
    --work) WORK="$2"; shift 2 ;;
    --record) RECORD="$2"; shift 2 ;;
    --goatbuilder) GB="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"; shift 2 ;;
    --keep) KEEP=1; shift ;;
    --host-distro) DISTRO="$2"; shift 2 ;;
    -h|--help) sed -n '2,19p' "$0"; exit 0 ;;
    *) echo "release-file: unknown argument $1" >&2; exit 2 ;;
  esac
done
[ -n "$GB" ] && [ -x "$GB" ] || { echo "release-file: --goatbuilder names a goatbuilder binary" >&2; exit 2; }
case "$DISTRO" in
  ubuntu) HOST_DOCKERFILE=Dockerfile ;;
  debian) HOST_DOCKERFILE=Dockerfile.debian ;;
  *) echo "release-file: --host-distro is ubuntu or debian, not '$DISTRO'" >&2; exit 2 ;;
esac
for t in docker ssh git go age-keygen ansible-playbook jq yq python3; do
  command -v "$t" >/dev/null 2>&1 || { echo "release-file: $t is needed" >&2; exit 2; }
done
[ -n "$WORK" ] || WORK="$(mktemp -d "${TMPDIR:-/tmp}/goat-e2e-rel.XXXXXX")"
mkdir -p "$WORK"; WORK="$(cd "$WORK" && pwd)"
[ -n "$RECORD" ] || RECORD="$WORK/report"
mkdir -p "$RECORD"
die() { echo "release-file: $*" >&2; exit 1; }
say() { printf '\n==> %s\n' "$*"; }
FAILS=0; N=0
# case_ <name> <exit> <want|!notwant>... -- <command...>: run it, keep its output, hold it to its words.
case_() {
  local name="$1" want_rc="$2" rc=0 bad=""; shift 2
  local -a words=()
  while [ "$1" != -- ]; do words+=("$1"); shift; done
  shift; N=$((N + 1))
  "$@" > "$RECORD/$name.log" 2>&1 || rc=$?
  [ "$rc" = "$want_rc" ] || bad="exit $rc, not $want_rc"
  local w
  for w in "${words[@]}"; do
    case "$w" in
      !*) if grep -qF -- "${w#!}" "$RECORD/$name.log"; then bad="$bad; says: ${w#!}"; fi ;;
      *) grep -qF -- "$w" "$RECORD/$name.log" || bad="$bad; does not say: $w" ;;
    esac
  done
  if [ -z "$bad" ]; then echo "PASS $name"; else echo "FAIL $name: $bad"; tail -25 "$RECORD/$name.log" | sed 's/^/     | /'; FAILS=$((FAILS + 1)); fi
}

# The published releases: a pin's digest is its SHA256SUMS file's SHA-256.
SRC=USAREUR-AF/goatcore
PIN72="$SRC@goatbuilder/v0.7.2@sha256:98f4764931eb2049d920fec19499bf019b2d2b1070657a533efe73cfccba2e2f"
PIN73="$SRC@goatbuilder/v0.7.3@sha256:edef489da725921d55eb6687fc5cf6805fd0cc3cdac052e441e5cb9828e5ebfc"
D72="sha256:98f4764931eb2049d920fec19499bf019b2d2b1070657a533efe73cfccba2e2f"
D73="sha256:edef489da725921d55eb6687fc5cf6805fd0cc3cdac052e441e5cb9828e5ebfc"
case "$(uname -m)" in
  x86_64) ARCH=amd64; F72=bbe890c74f8abf38213690bdec50cf70cf6f6d3fbd002787e3ddd42d1e5452f1; F73=ecff2911f3e755d071b62269c39136b1b8f2fc05b371bfc12b9475d6e946a7bf ;;
  aarch64|arm64) ARCH=arm64; F72=6fe2f373d85058723c8e6c9d17f096fe62f0ecc8e570785efbdc47bd0c3c0195; F73=9caf45c98d2aa375dd2e43f5f90078cee7f8a04d3f360be2ec77976dd5888e38 ;;
  *) die "no published file for $(uname -m)" ;;
esac
C=e2e-release

# An isolated HOME, as hermetic.sh: an ssh config, the throwaway CA, the kept releases.
HOME_REAL="$HOME"; export HOME="$WORK/home"; mkdir -p "$HOME/.ssh" "$WORK/ssh" "$WORK/bin"; chmod 700 "$HOME/.ssh"
export GOMODCACHE="${GOMODCACHE:-$HOME_REAL/go/pkg/mod}" GOCACHE="${GOCACHE:-$HOME_REAL/.cache/go-build}" GOPATH="${GOPATH:-$HOME_REAL/go}"
SSH_KEY="$WORK/ssh/id_ed25519"
[ -f "$SSH_KEY" ] || ssh-keygen -q -t ed25519 -N '' -f "$SSH_KEY" -C e2e-harness
# ControlPath is the run's own: the role connects by a container address a later run may reuse.
printf 'Host *\n  IdentityFile %s\n  IdentitiesOnly yes\n  StrictHostKeyChecking no\n  UserKnownHostsFile /dev/null\n  LogLevel ERROR\n' "$SSH_KEY" > "$HOME/.ssh/config"
chmod 600 "$HOME/.ssh/config"
REAL_SSH="$(command -v ssh)"
printf '#!/bin/sh\nexec "%s" -F "%s" "$@"\n' "$REAL_SSH" "$HOME/.ssh/config" > "$WORK/bin/ssh"; chmod +x "$WORK/bin/ssh"
export PATH="$WORK/bin:$HERE/stubs:$PATH" ANSIBLE_HOST_KEY_CHECKING=False GIT_LFS_SKIP_SMUDGE=1
export ANSIBLE_CACHE_PLUGIN_CONNECTION="$WORK/facts" ANSIBLE_SSH_CONTROL_PATH_DIR="$WORK/cp"
# The stubs clone the whole tree to merge a pin: under --work, not a small /tmp.
export TMPDIR="$WORK/tmp"; mkdir -p "$TMPDIR"
export E2E_ORIGIN="$WORK/origin.git" E2E_GH_STATE="$WORK/gh-state" GOAT_RC_ROLE_CHECK=0
export E2E_RECORD_DIR="$RECORD" GOAT_MESH_PROBE=0 GOAT_ANSIBLE_TRUNK_REMOTE=origin GOAT_DEPLOY_LOG_DIR="$RECORD" GOAT_NO_FUN=1 GOATBUILDER_BIN="$GB"
unset GOATBUILDER_RELEASE_MIRROR GOATBUILDER_UPDATE_TRUSTED_ROOT

say "source of record: a bare remote and a clone of $(git -C "$REPO" rev-parse --short HEAD)"
ORIGIN="$WORK/origin.git"; CLONE="$WORK/goat"; rm -rf "$ORIGIN" "$CLONE"
git clone -q --bare "$REPO" "$ORIGIN"
git -C "$ORIGIN" update-ref refs/heads/main "$(git -C "$REPO" rev-parse HEAD)"
git -C "$ORIGIN" symbolic-ref HEAD refs/heads/main
git clone -q --branch main "$ORIGIN" "$CLONE"
git -C "$CLONE" config user.name "e2e harness"; git -C "$CLONE" config user.email e2e@localhost
git -C "$CLONE" config commit.gpgsign false

say "host: a container that looks like a management host ($DISTRO)"
TAG="e2e-rel-$$"
cleanup() {
  [ "$KEEP" = 1 ] && return
  docker rm -fv "$TAG-host" >/dev/null 2>&1 || true
  docker network rm "$TAG-net" >/dev/null 2>&1 || true
  docker rmi "$TAG-image" >/dev/null 2>&1 || true
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
# A build handed the runner's mirror retries once without it if the mirror dies mid-install.
# shellcheck source=ops/ci/docker-build-mirror-fallback.sh
. "$REPO/ops/ci/docker-build-mirror-fallback.sh"
build_with_mirror_fallback -q -f "$HERE/host/$HOST_DOCKERFILE" -t "$TAG-image" "$HERE/host" >/dev/null
docker network create "$TAG-net" >/dev/null
docker run -d --name "$TAG-host" --hostname "$TAG-host" --network "$TAG-net" -v "$SSH_KEY.pub:/pubkey:ro" "$TAG-image" >/dev/null
IP="$(docker inspect -f "{{ (index .NetworkSettings.Networks \"$TAG-net\").IPAddress }}" "$TAG-host")"
for _i in $(seq 1 30); do ssh -o ConnectTimeout=2 "goat@$IP" true 2>/dev/null && break; sleep 1; done
ssh -o ConnectTimeout=2 "goat@$IP" true || die "the host did not come up"

say "overlay: the hermetic goatnets, and $C pinned at goatbuilder 0.7.2"
python3 "$HERE/overlay.py" "$CLONE" --home "$HOME" --ssh-key "$SSH_KEY" --host "e2e_dev=$IP" \
  --canary-ref "ghcr.io/{{ ghcr_org }}/e2e-canary:v0.0.0@sha256:$(printf '0%.0s' $(seq 64))" --release-ref "$PIN72"
SCR="$CLONE/ops/scripts"; cd "$CLONE"
on_host() { ssh "goat@$IP" "$@"; }
repin() { python3 "$HERE/overlay.py" "$CLONE" --home "$HOME" --repin "$C=$1" > "$RECORD/repin-$N.log" 2>&1 || die "the re-pin failed ($RECORD/repin-$N.log)"; }
host_is() { # <file sha256> <ref>: the file current names, and the version it answers
  [ "$(on_host "sha256sum \$(readlink -e /opt/goat/releases/$C/current) | cut -d' ' -f1")" = "$1" ] || return 1
  on_host "cat \$(dirname \$(readlink -e /opt/goat/releases/$C/current))/.ref" | grep -qxF "$2" || return 1
  echo "current is $1, $(on_host "/opt/goat/releases/$C/current --version")"
}
untouched() { if on_host "sudo test -e /opt/goat/releases -o -e /var/lib/goat/$C"; then echo "the host was touched"; return 1; else echo "the host was not touched"; fi; }
gb() { "$GB" "$@"; }
deploy_sh() { "$SCR/deploy-component.sh" "$C" --goatnet e2e_dev "$@"; }
deploy_gb() { gb deploy --env e2e_dev --component "$C" --ack "e2e harness" --yes "$@"; }
VERIFIED="release verified: goatbuilder-linux-amd64 (amd64) sha256"
SIGNED="signed by https://github.com/USAREUR-AF/goatcore/.github/workflows/_release-binary.yml@refs/tags/goatbuilder/"
CONVERGED="every host's ledger names the declared ref"

say "cases: a dry run fetches and verifies, and touches no host"
case_ dry-sh 0 "$VERIFIED" "${SIGNED}v0.7.2" "release-file-deploy.yml" "DRY: would read the file $C runs" "DRY: nothing was read" -- deploy_sh --dry-run
case_ dry-gb 0 "$VERIFIED" "${SIGNED}v0.7.2" "release-file-deploy.yml" "DRY: would read the file $C runs" "DRY: nothing was read" -- deploy_gb --dry-run
case_ dry-touched-nothing 0 "the host was not touched" -- untouched

say "cases: a release that does not verify is refused before any host, in the same words"
repin "${PIN72%sha256:*}sha256:$(printf 'a%.0s' $(seq 64))"
WRONG="GB-381 refused before any host: $C's release"
for a in sh gb; do
  case_ "wrong-digest-dry-$a" 1 "$WRONG" "(check: digest)" "DRY: would read /var/lib/goat" "DRY: a real run would refuse before any host (release)" -- "deploy_$a" --dry-run
  case_ "wrong-digest-$a" 1 "$WRONG" "(check: digest)" "!deploying" -- "deploy_$a"
done
case_ refused-touched-nothing 0 "the host was not touched" -- untouched
words_of() { grep -oE 'GB-381 refused before any host: [^"]*\(check: [a-z]+\)' "$1" | head -1; }
case_ refusal-words-agree 0 "(check: digest)" -- bash -c "$(declare -f words_of); a=\$(words_of '$RECORD/wrong-digest-sh.log'); b=\$(words_of '$RECORD/wrong-digest-gb.log'); echo \"\$a\"; [ -n \"\$a\" ] && [ \"\$a\" = \"\$b\" ]"
repin "$PIN72"

say "cases: script avenue deploys 0.7.2, binary avenue moves it to 0.7.3"
case_ verify-before-any-deploy-sh 1 "NO_LEDGER" "NOT converged" -- deploy_sh --verify-only
case_ deploy-072-sh 0 "$VERIFIED" "deploying $PIN72" "installed file goatbuilder-linux-$ARCH is the pin's (sha256 $F72)" "converged on $D72" "$CONVERGED" -- deploy_sh
case_ host-runs-072 0 "0.7.2" -- host_is "$F72" "$PIN72"
case_ verify-072-gb 0 "installed file goatbuilder-linux-$ARCH is the pin's (sha256 $F72)" "converged on $D72" -- gb verify "$C" --env e2e_dev
repin "$PIN73"
case_ verify-names-the-old-pin-sh 1 "NOT converged" -- deploy_sh --verify-only
case_ verify-names-the-old-pin-gb 3 "NOT converged" -- gb verify "$C" --env e2e_dev
case_ deploy-073-gb 0 "$VERIFIED" "installed file goatbuilder-linux-$ARCH is the pin's (sha256 $F73)" "converged on $D73" "ledger row written by this run" -- deploy_gb
case_ host-runs-073 0 "0.7.3" -- host_is "$F73" "$PIN73"
case_ verify-073-sh 0 "installed file goatbuilder-linux-$ARCH is the pin's (sha256 $F73)" "converged on $D73" "$CONVERGED" -- deploy_sh --verify-only
case_ again-moves-nothing-gb 0 "this run wrote no new ledger row" "converged on $D73" -- deploy_gb
case_ again-moves-nothing-sh 0 "converged on $D73" -- deploy_sh

say "cases: verify compares the file, not only the ledger"
on_host "sudo cp -p /opt/goat/releases/$C/$F73/goatbuilder-linux-$ARCH /tmp/kept && sudo chmod u+w /opt/goat/releases/$C/$F73/goatbuilder-linux-$ARCH && echo tampered | sudo tee -a /opt/goat/releases/$C/$F73/goatbuilder-linux-$ARCH >/dev/null"
OFF="NOT converged — ledger on $D73, but the installed file is sha256"
case_ changed-file-sh 1 "$OFF" "!every host's ledger" -- deploy_sh --verify-only
case_ changed-file-gb 3 "$OFF" -- gb verify "$C" --env e2e_dev
case_ deploy-repairs-it-sh 0 "installed file goatbuilder-linux-$ARCH is the pin's (sha256 $F73)" "$CONVERGED" -- deploy_sh
on_host "sudo mv /opt/goat/releases/$C/current /opt/goat/releases/$C/current.away"
case_ missing-file-sh 1 "ledger on $D73, but no file is installed" -- deploy_sh --verify-only
case_ missing-file-gb 3 "ledger on $D73, but no file is installed" -- gb verify "$C" --env e2e_dev
on_host "sudo mv /opt/goat/releases/$C/current.away /opt/goat/releases/$C/current"

say "cases: rollback switches to previous with no fetch, on both avenues"
ROLLED="rolled back: goatcore@sha256:edef489da725 -> goatcore@sha256:98f476493"
case_ rollback-dry-sh 0 "release_file_rollback=true" "DRY: nothing was read or changed" -- "$SCR/rollback.sh" "$C" --dry-run --site e2e-dev
case_ rollback-dry-gb 0 "release_file_rollback=true" "DRY: nothing was read or changed" -- gb rollback "$C" 1 --site e2e-dev --dry-run
case_ rollback-dry-moved-nothing 0 "0.7.3" -- host_is "$F73" "$PIN73"
case_ rollback-two-steps-sh 1 "GB-379 $C is a release component" "rolls back 1 step, not 2" -- "$SCR/rollback.sh" "$C" 2 --site e2e-dev
case_ rollback-two-steps-gb 2 "$C is a release component" "rolls back 1 step, not 2" -- gb rollback "$C" 2 --site e2e-dev --yes
NOSITE="'$C' is deployed on 3 goatnets (e2e-dev, e2e-preprod, e2e-prod); a rollback with no --site would flip every one"
case_ rollback-no-site-sh 1 "$NOSITE" -- "$SCR/rollback.sh" "$C" 1
case_ rollback-no-site-gb 1 "$NOSITE" -- gb rollback "$C" 1 --yes
case_ rollback-sh 0 "$ROLLED" "every host is on the file before" -- "$SCR/rollback.sh" "$C" 1 --site e2e-dev
case_ rollback-sh-host 0 "0.7.2" -- host_is "$F72" "$PIN72"
for a in sh gb; do
  if [ "$a" = sh ]; then case_ "verify-names-the-rollback-sh" 1 "NOT converged — rolled back from $SRC@goatbuilder/v0.7.3@sha256:edef489da725 to $SRC@goatbuilder/v0.7.2@sha256:98f476493" "!the play did not finish" -- deploy_sh --verify-only
  else case_ "verify-names-the-rollback-gb" 3 "NOT converged — rolled back from $SRC@goatbuilder/v0.7.3@sha256:edef489da725 to $SRC@goatbuilder/v0.7.2@sha256:98f476493" "!the play did not finish" -- gb verify "$C" --env e2e_dev; fi
done
case_ status-sh 0 "artifact_type=release" "$PIN72" -- "$SCR/rollback.sh" "$C" --status --site e2e-dev
case_ status-gb 0 "artifact_type=release" "$PIN72" -- gb rollback "$C" --status --site e2e-dev
case_ list-sh 0 "[release]  $PIN73" "action=rollback" -- "$SCR/rollback.sh" "$C" --list --site e2e-dev
case_ list-gb 0 "[release]  $PIN73" "action=rollback" -- gb rollback "$C" --list --site e2e-dev
words() { sed -E 's/^(ok  |FAIL)  rollback  //; s/^ +//' "$1" | grep -E '\[release\]|on-host @|^deploy history|artifact_type=|^e2e-release: '; }
case_ list-and-status-agree 0 "agree" -- bash -c "$(declare -f words); diff <(words '$RECORD/list-sh.log') <(words '$RECORD/list-gb.log') && diff <(words '$RECORD/status-sh.log') <(words '$RECORD/status-gb.log') && [ \$(words '$RECORD/list-sh.log' | wc -l) -ge 4 ] && echo agree"
case_ deploy-after-the-rollback-gb 0 "converged on $D73" "ledger row written by this run" -- deploy_gb
case_ rollback-gb 0 "$ROLLED" "every host is on the file before" -- gb rollback "$C" 1 --site e2e-dev --yes
case_ rollback-gb-host 0 "0.7.2" -- host_is "$F72" "$PIN72"
on_host "sudo mv /opt/goat/releases/$C/previous /opt/goat/releases/$C/previous.away"
STALE="NOT rolled back — this run wrote no ledger row; its newest is an earlier rollback"
case_ no-previous-sh 1 "has no earlier file of $C" "$STALE" -- "$SCR/rollback.sh" "$C" 1 --site e2e-dev
case_ no-previous-gb 3 "has no earlier file of $C" "$STALE" -- gb rollback "$C" 1 --site e2e-dev --yes
on_host "sudo mv /opt/goat/releases/$C/previous.away /opt/goat/releases/$C/previous"
repin "$PIN72"
case_ pin-follows-the-rollback-sh 0 "converged on $D72" "$CONVERGED" -- deploy_sh
case_ verify-after-sh 0 "converged on $D72" "$CONVERGED" -- deploy_sh --verify-only
case_ verify-after-gb 0 "converged on $D72" -- gb verify "$C" --env e2e_dev

say "cases: fastlane verifies the release, moves the pin, signs, lands it; deploy follows"
LANE=(--reason "e2e" --ack "e2e harness")
NOTPIN="$C is a release component (artifact_type: release) and v0.7.3 is not a release pin"
case_ fastlane-not-a-pin-sh 1 "$NOTPIN" -- "$SCR/fastlane.sh" "$C" --ref v0.7.3 "${LANE[@]}" --dry-run
case_ fastlane-not-a-pin-gb 1 "$NOTPIN" -- gb fastlane "$C" --ref v0.7.3 "${LANE[@]}" --dry-run
case_ fastlane-parity-dry 0 "parity ok: fastlane $C" -- "$CLONE/ops/goatbuilder/parity-fastlane.sh" "$GB" "$C" "$PIN73"
case_ fastlane-parity-refused 0 "parity ok: fastlane $C" -- "$CLONE/ops/goatbuilder/parity-fastlane.sh" "$GB" "$C" "${PIN72%sha256:*}sha256:$(printf 'a%.0s' $(seq 64))"
case_ fastlane-sh 0 "$VERIFIED" "fleet: -> $PIN73" "hotfix record written" "the pin is in main" -- "$SCR/fastlane.sh" "$C" --ref "$PIN73" "${LANE[@]}" --no-deploy
case_ fastlane-sh-deploys-gb 0 "converged on $D73" -- deploy_gb
case_ fastlane-gb 0 "$VERIFIED" "fleet: -> $PIN72" "hotfix record written" "the pin is in main" -- gb fastlane "$C" --ref "$PIN72" "${LANE[@]}" --no-deploy --yes
case_ fastlane-gb-deploys-sh 0 "converged on $D72" "$CONVERGED" -- deploy_sh
case_ fastlane-host 0 "0.7.2" -- host_is "$F72" "$PIN72"
case_ fastlane-check 0 "check" -- python3 "$CLONE/ops/release/graduate.py" check

say "cases: a previous file changed on the host is not switched to"
on_host "sudo chmod u+w /opt/goat/releases/$C/$F73/goatbuilder-linux-$ARCH && echo tampered | sudo tee -a /opt/goat/releases/$C/$F73/goatbuilder-linux-$ARCH >/dev/null"
case_ rollback-to-a-changed-file-sh 1 "NOT rolled back" -- "$SCR/rollback.sh" "$C" 1 --site e2e-dev
case_ rollback-to-a-changed-file-gb 3 "NOT rolled back" -- gb rollback "$C" 1 --site e2e-dev --yes
case_ still-072 0 "0.7.2" -- host_is "$F72" "$PIN72"

say "cases: kept files verify with no network; the host is not reached"
OFFNET=(env https_proxy=http://127.0.0.1:9 GOATBUILDER_RELEASE_MIRROR="$WORK/no-such-mirror")
case_ kept-sh 0 "$VERIFIED" -- "${OFFNET[@]}" "$SCR/deploy-component.sh" "$C" --goatnet e2e_dev --dry-run
mv "$HOME/.goat/state/releases" "$WORK/releases-away" 2>/dev/null || mv "$(dirname "$(dirname "$(find "$HOME" -path "*releases/$C" -type d | head -1)")")/releases" "$WORK/releases-away"
for a in sh gb; do
  if [ "$a" = sh ]; then case_ no-mirror-verify-sh 1 "unknown — ledger on $D72, but its file was not compared: GB-381" -- "${OFFNET[@]}" "$SCR/deploy-component.sh" "$C" --goatnet e2e_dev --verify-only
  else case_ no-mirror-verify-gb 3 "unknown — ledger on $D72, but its file was not compared: GB-381" -- "${OFFNET[@]}" "$GB" verify "$C" --env e2e_dev; fi
done
docker stop -t 1 "$TAG-host" >/dev/null
case_ host-down-sh 1 "UNREACHABLE!" "NOT converged everywhere" -- deploy_sh --verify-only
case_ host-down-gb 3 "unknown — host unreachable" -- gb verify "$C" --env e2e_dev
case_ rollback-host-down-gb 3 "NOT rolled back everywhere" -- gb rollback "$C" 1 --site e2e-dev --yes
case_ rollback-host-down-sh 1 "NOT rolled back everywhere" -- "$SCR/rollback.sh" "$C" 1 --site e2e-dev

say "parity: deploy --dry-run, every component on e2e_dev"
case_ parity-deploy 0 "parity ok: 3 component/goatnet pairs" -- "$CLONE/ops/goatbuilder/parity-deploy.sh" "$GB" e2e_dev
repin "${PIN72%sha256:*}sha256:$(printf 'a%.0s' $(seq 64))"
case_ parity-deploy-refused 0 "parity ok: 3 component/goatnet pairs" -- "$CLONE/ops/goatbuilder/parity-deploy.sh" "$GB" e2e_dev

echo "record: $RECORD"
echo "release-file: $((N - FAILS)) of $N cases passed ($DISTRO host, $ARCH)"
[ "$FAILS" = 0 ]
