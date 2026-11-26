#!/usr/bin/env bash
# runner-release.sh: the goat-processors runner, a release component (gate J24),
# deployed from its published releases to its own health check, rolled back and
# verified, by both avenues. docs/design/goatbuilder-build-targets.md section 3.9.
#
# The pattern of ops/e2e/release-file.sh: the real drivers on a fresh clone of
# this commit whose declarations are the hermetic overlay (ops/e2e/overlay.py),
# with the public table's own row of the runner kept, e2e_dev opting in to it,
# and a bare remote as its trunk. The host is a VM from a published Ubuntu cloud image
# (ops/e2e/vm/vm.sh), because the runner is a systemd service; the fabric router
# it connects to is eclipse/zenoh at the digest the e2e suites use, in a
# container on this machine that the VM reaches at QEMU's host address. The
# releases are runner 0.8.0 and 0.8.1, fetched from GitHub and verified against
# the public Sigstore. It needs no sudo and changes nothing on this machine
# outside --work, its VM and its container.
#
# usage: runner-release.sh --goatbuilder BIN [--work DIR] [--record DIR] [--keep]
# Exit 0 when every case passed; 1 otherwise; 2 for a usage error or a missing tool.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"
WORK=""; RECORD=""; GB=""; KEEP=0
while [ $# -gt 0 ]; do
  case "$1" in
    --work) WORK="$2"; shift 2 ;;
    --record) RECORD="$2"; shift 2 ;;
    --goatbuilder) GB="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"; shift 2 ;;
    --keep) KEEP=1; shift ;;
    -h|--help) sed -n '2,18p' "$0"; exit 0 ;;
    *) echo "runner-release: unknown argument $1" >&2; exit 2 ;;
  esac
done
[ -n "$GB" ] && [ -x "$GB" ] || { echo "runner-release: --goatbuilder names a goatbuilder binary" >&2; exit 2; }
for t in docker ssh ssh-keygen git go age-keygen ansible-playbook python3 qemu-img curl sha256sum; do
  command -v "$t" >/dev/null 2>&1 || { echo "runner-release: $t is needed" >&2; exit 2; }
done
command -v cloud-localds >/dev/null 2>&1 || command -v xorriso >/dev/null 2>&1 || { echo "runner-release: cloud-localds or xorriso is needed" >&2; exit 2; }
case "$(uname -m)" in
  x86_64) ARCH=amd64 ;;
  aarch64|arm64) ARCH=arm64 ;;
  *) echo "runner-release: no published package for $(uname -m)" >&2; exit 2 ;;
esac
[ -n "$WORK" ] || WORK="$(mktemp -d "${TMPDIR:-/tmp}/goat-e2e-runner.XXXXXX")"
mkdir -p "$WORK"; WORK="$(cd "$WORK" && pwd)"
[ -n "$RECORD" ] || RECORD="$WORK/report"
mkdir -p "$RECORD"; RECORD="$(cd "$RECORD" && pwd)"
die() { echo "runner-release: $*" >&2; exit 1; }
say() { printf '\n==> %s\n' "$*"; }
FAILS=0; N=0
# case_ <name> <exit> <want|!notwant>... -- <command...>: run it, keep its output, hold it to its words.
case_() {
  local name="$1" want_rc="$2" rc=0 bad=""; shift 2
  local -a words=()
  while [ "$1" != -- ]; do words+=("$1"); shift; done
  shift; N=$((N + 1))
  "$@" > "$RECORD/$name.log" 2>&1 </dev/null || rc=$?
  [ "$rc" = "$want_rc" ] || bad="exit $rc, not $want_rc"
  local w
  for w in ${words[@]+"${words[@]}"}; do
    case "$w" in
      !*) if grep -qF -- "${w#!}" "$RECORD/$name.log"; then bad="$bad; says: ${w#!}"; fi ;;
      *) grep -qF -- "$w" "$RECORD/$name.log" || bad="$bad; does not say: $w" ;;
    esac
  done
  if [ -z "$bad" ]; then echo "PASS $name"; else echo "FAIL $name: $bad"; tail -25 "$RECORD/$name.log" | cut -c1-220 | sed 's/^/     | /'; FAILS=$((FAILS + 1)); fi
}

# The published releases: a pin's digest is its SHA256SUMS file's SHA-256.
SRC=USAREUR-AF/goat-processors
# 0.8.1 is the row's pin (the runner's TLS fix release); 0.8.0 is the one before it.
PIN_PREV="$SRC@runner-v0.8.0@sha256:ec07bb0684ba0f20a5309fff24764f0a0befa180e99fc8e505e9af3393a273e9"
PIN_ROW="$SRC@runner-v0.8.1@sha256:1875d7d63631726b8720fb46fc6726b840e4bc3d07c559a544372b9d4cbf4097"
D_PREV="sha256:ec07bb0684ba0f20a5309fff24764f0a0befa180e99fc8e505e9af3393a273e9"
D_ROW="sha256:1875d7d63631726b8720fb46fc6726b840e4bc3d07c559a544372b9d4cbf4097"
C=goat-processors-runner
python3 - "$REPO/ops/pins/fleet-pins.yaml" "$C" "$PIN_ROW" <<'PY' || die "the public table's $C is not pinned at $PIN_ROW"
import sys, yaml
c = yaml.safe_load(open(sys.argv[1]))["components"][sys.argv[2]]
sys.exit(0 if c.get("fleet") == sys.argv[3] and c.get("opt_in") is True else 1)
PY
ZENOH_IMAGE="${E2E_ZENOH_IMAGE:-eclipse/zenoh@sha256:157965d71e0bfd0a044d76a985ff0e5c306ad3968929168fb9678cd2a7fec23f}"

# An isolated HOME, as release-file.sh: an ssh config, the throwaway CA, the kept releases.
HOME_REAL="$HOME"; export HOME="$WORK/home"; mkdir -p "$HOME/.ssh" "$WORK/bin"; chmod 700 "$HOME/.ssh"
export GOMODCACHE="${GOMODCACHE:-$HOME_REAL/go/pkg/mod}" GOCACHE="${GOCACHE:-$HOME_REAL/.cache/go-build}" GOPATH="${GOPATH:-$HOME_REAL/go}"
export ANSIBLE_COLLECTIONS_PATH="${ANSIBLE_COLLECTIONS_PATH:-$HOME_REAL/.ansible/collections:/usr/share/ansible/collections}"
export VM_WORK="$WORK/vm" VM_ARCH="$ARCH" VM_UBUNTU=24.04 VM_IMAGES="${VM_IMAGES:-$HOME_REAL/.cache/goat-vm-images}" VM_MEM="${VM_MEM:-2048}" VM_CPUS="${VM_CPUS:-2}" VM_TAPS=0 VM_NAME=e2e-runner VM_BOOT_WAIT="${VM_BOOT_WAIT:-300}"
VM_PORT="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')"
ZENOH_PORT="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')"
export VM_PORT
mkdir -p "$VM_WORK" "$VM_IMAGES"
# shellcheck source=ops/e2e/vm/vm.sh
. "$HERE/vm/vm.sh"
printf 'Host *\n  IdentityFile %s\n  IdentitiesOnly yes\n  StrictHostKeyChecking no\n  UserKnownHostsFile /dev/null\n  LogLevel ERROR\n' "$VM_WORK/key" > "$HOME/.ssh/config"
chmod 600 "$HOME/.ssh/config"
REAL_SSH="$(command -v ssh)"
printf '#!/bin/sh\nexec "%s" -F "%s" "$@"\n' "$REAL_SSH" "$HOME/.ssh/config" > "$WORK/bin/ssh"; chmod +x "$WORK/bin/ssh"
export PATH="$WORK/bin:$HERE/stubs:$PATH" ANSIBLE_HOST_KEY_CHECKING=False GIT_LFS_SKIP_SMUDGE=1
export ANSIBLE_CACHE_PLUGIN_CONNECTION="$WORK/facts" ANSIBLE_SSH_CONTROL_PATH_DIR="$WORK/cp"
export TMPDIR="$WORK/tmp"; mkdir -p "$TMPDIR"
export E2E_ORIGIN="$WORK/origin.git" E2E_GH_STATE="$WORK/gh-state" GOAT_RC_ROLE_CHECK=0
export E2E_RECORD_DIR="$RECORD" GOAT_MESH_PROBE=0 GOAT_ANSIBLE_TRUNK_REMOTE=origin GOAT_DEPLOY_LOG_DIR="$RECORD" GOAT_NO_FUN=1 GOATBUILDER_BIN="$GB"
unset GOATBUILDER_RELEASE_MIRROR GOATBUILDER_UPDATE_TRUSTED_ROOT

TAG="e2e-runner-$$"
cleanup() {
  if [ "$KEEP" = 1 ]; then echo "kept: ssh -i $VM_WORK/key -p $VM_PORT goatvm@127.0.0.1; docker rm -fv $TAG-zenoh"; return; fi
  vm_down
  docker rm -fv "$TAG-zenoh" >/dev/null 2>&1 || true
}
trap 'rc=$?; cleanup; cp "$VM_WORK/console.log" "$RECORD/" 2>/dev/null || true; exit $rc' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

say "the fabric router: $ZENOH_IMAGE on 127.0.0.1:$ZENOH_PORT"
docker run -d --name "$TAG-zenoh" -p "127.0.0.1:$ZENOH_PORT:7447" "$ZENOH_IMAGE" >/dev/null || die "the router did not start"

say "the host: Ubuntu $VM_UBUNTU $VM_ARCH in a VM"
vm_up || die "the VM did not come up"
echo "up, accel=$VM_ACCEL, image sha256 $VM_IMAGE_SHA256, its checksums signed by Ubuntu's key: $VM_SIGNED"

say "source of record: a bare remote and a clone of $(git -C "$REPO" rev-parse --short HEAD)"
ORIGIN="$WORK/origin.git"; CLONE="$WORK/goat"; rm -rf "$ORIGIN" "$CLONE"
git clone -q --bare "$REPO" "$ORIGIN"
git -C "$ORIGIN" update-ref refs/heads/main "$(git -C "$REPO" rev-parse HEAD)"
git -C "$ORIGIN" symbolic-ref HEAD refs/heads/main
git clone -q --branch main "$ORIGIN" "$CLONE"
git -C "$CLONE" config user.name "e2e harness"; git -C "$CLONE" config user.email e2e@localhost
git -C "$CLONE" config commit.gpgsign false

# The runner's configuration, as a goatnet's group_vars give it: a client of the
# router, a local UDP input, the shipped pipeline.
cat > "$WORK/goat-processors-runner.yml" <<EOF
---
# HERMETIC (ops/e2e/runner-release.sh): the runner's configuration on e2e_dev.
goat_processors_runner_config:
  name: e2e-runner
  site: e2e-dev
  tick_ms: 500
  fabric:
    mode: client
    connect: ["tcp/10.0.2.2:$ZENOH_PORT"]
  inputs:
    udp:
      - listen: 127.0.0.1:7600
  pipeline:
    - {function: correlate, pack: correlate-assign-time}
    - {function: quality, pack: quality-feed}
  outputs:
    fabric: {peer: e2e-dev, trace: false}
  api: {audit_max_mb: 64, audit_keep: 9}
EOF
say "overlay: the hermetic goatnets, e2e_dev opted in to $C, then pinned at the release before its row (0.8.0)"
python3 "$HERE/overlay.py" "$CLONE" --home "$HOME" --ssh-key "$VM_WORK/key" --ssh-user goatvm --host "e2e_dev=127.0.0.1:$VM_PORT" \
  --canary-ref "ghcr.io/{{ ghcr_org }}/e2e-canary:v0.0.0@sha256:$(printf '0%.0s' $(seq 64))" \
  --public-component "$C" --opt-in "e2e_dev=$C" --member-vars "e2e_dev_hosts=$WORK/goat-processors-runner.yml"
SCR="$CLONE/ops/scripts"; cd "$CLONE"
# A pin and its file names move together: as a reviewed change of the table would.
to_version() { # <version> <pin>
  python3 - "$CLONE/ops/pins/fleet-pins.yaml" "$C" "$1" <<'PY'
import sys, yaml
p, c, v = sys.argv[1:4]
d = yaml.safe_load(open(p))
d["components"][c]["release"]["files"] = {a: "goat-processors_%s_%s.deb" % (v, a) for a in ("amd64", "arm64")}
open(p, "w").write(yaml.safe_dump(d, sort_keys=False, width=200))
PY
  python3 "$HERE/overlay.py" "$CLONE" --home "$HOME" --repin "$C=$2" > "$RECORD/repin-$1.log" 2>&1 || die "the re-pin failed ($RECORD/repin-$1.log)"
}
to_version 0.8.0 "$PIN_PREV"
on_host() { vm_ssh "$@"; }
runs() { # <version>: the package, the service and both health answers on the host
  on_host "goat-processors version | grep -qxF 'goat-processors $1' && systemctl is-active goat-processors && sudo goat-processors doctor -o json >/dev/null && curl -fsS http://127.0.0.1:8470/v1/health && echo && goat-processors version"
}
untouched() { if on_host "sudo test -e /opt/goat/releases -o -e /var/lib/goat/$C -o -e /usr/bin/goat-processors"; then echo "the host was touched"; return 1; else echo "the host was not touched"; fi; }
gb() { "$GB" "$@"; }
deploy_sh() { "$SCR/deploy-component.sh" "$C" --goatnet e2e_dev "$@"; }
deploy_gb() { gb deploy --env e2e_dev --component "$C" --ack "e2e harness" --yes "$@"; }
VERIFIED="release verified: goat-processors_0.8.0_$ARCH.deb ($ARCH) sha256"
SIGNED="signed by https://github.com/$SRC/.github/workflows/runner-release.yml@refs/tags/runner-v"
CONVERGED="every host's ledger names the declared ref"

say "cases: a dry run fetches and verifies the Sigstore bundle, and touches no host"
case_ dry-sh 0 "$VERIFIED" "${SIGNED}0.8.0" "goat-processors-runner-deploy.yml" "DRY: nothing was read" -- deploy_sh --dry-run
case_ dry-gb 0 "$VERIFIED" "${SIGNED}0.8.0" "goat-processors-runner-deploy.yml" "DRY: nothing was read" -- deploy_gb --dry-run
case_ dry-touched-nothing 0 "the host was not touched" -- untouched

say "cases: a goatnet that did not opt in does not run it"
NOTOPTED="does not run component"
case_ not-opted-in-sh 1 "$NOTOPTED" "it is opt-in, and the goatnet does not list it under opt_in:" -- "$SCR/deploy-component.sh" "$C" --goatnet e2e_preprod --dry-run
case_ not-opted-in-gb 2 "$NOTOPTED" "it is opt-in, and the goatnet does not list it under opt_in:" -- gb deploy --env e2e_preprod --component "$C" --ack "e2e harness" --yes --dry-run

say "cases: a goatnet that supplies no configuration is refused before anything is installed"
noconfig() {
  (cd "$CLONE/ops/ansible" && ansible-playbook -i inventory/mgmt-hosts.ini goat-processors-runner-deploy.yml -e target=e2e_dev_hosts \
    -e release_file_component="$C" -e '{"goat_processors_runner_config": {}}')
}
case_ no-config 2 "GB-395 the goat-processors runner's configuration is not supplied" "failed=1" -- noconfig
case_ no-config-touched-nothing 0 "the host was not touched" -- untouched

say "cases: the script avenue deploys 0.8.0 to its health check; the binary avenue moves it to 0.8.1"
case_ deploy-080-sh 0 "$VERIFIED" "deploying $PIN_PREV" "converged on $D_PREV" "$CONVERGED" -- deploy_sh
case_ host-runs-080 0 '"healthy":true' "goat-processors 0.8.0" -- runs 0.8.0
to_version 0.8.1 "$PIN_ROW"
case_ verify-names-the-old-pin-gb 3 "NOT converged" -- gb verify "$C" --env e2e_dev
case_ deploy-081-gb 0 "release verified: goat-processors_0.8.1_$ARCH.deb" "${SIGNED}0.8.1" "converged on $D_ROW" -- deploy_gb
case_ host-runs-081 0 '"healthy":true' "goat-processors 0.8.1" -- runs 0.8.1
case_ verify-081-sh 0 "converged on $D_ROW" "$CONVERGED" -- deploy_sh --verify-only
case_ verify-081-gb 0 "converged on $D_ROW" -- gb verify "$C" --env e2e_dev
case_ again-sh 0 "converged on $D_ROW" -- deploy_sh
case_ again-gb 0 "this run wrote no new ledger row" "converged on $D_ROW" -- deploy_gb

say "cases: rollback installs the package before, and it is healthy"
ROLLED="rolled back: goat-processors@sha256:1875d7d63631 -> goat-processors@sha256:ec07bb068"
case_ rollback-sh 0 "$ROLLED" "every host is on the file before" -- "$SCR/rollback.sh" "$C" 1 --site e2e-dev
case_ rollback-host-runs-080 0 '"healthy":true' "goat-processors 0.8.0" -- runs 0.8.0
case_ verify-names-the-rollback-gb 3 "NOT converged — rolled back from $SRC@runner-v0.8.1@sha256:1875d7d63631 to $SRC@runner-v0.8.0@sha256:ec07bb0684ba" -- gb verify "$C" --env e2e_dev
case_ deploy-after-the-rollback-gb 0 "converged on $D_ROW" "ledger row written by this run" -- deploy_gb
case_ rollback-gb 0 "$ROLLED" "every host is on the file before" -- gb rollback "$C" 1 --site e2e-dev --yes
case_ rollback-gb-host-runs-080 0 '"healthy":true' "goat-processors 0.8.0" -- runs 0.8.0
case_ forward-again-sh 0 "converged on $D_ROW" -- deploy_sh
case_ forward-host-runs-081 0 '"healthy":true' "goat-processors 0.8.1" -- runs 0.8.1

say "cases: a package changed on the host is not installed"
on_host "sudo sh -c 'p=\$(readlink -e /opt/goat/releases/$C/previous); chmod u+w \"\$p\"; echo tampered >> \"\$p\"'"
case_ rollback-to-a-changed-package-gb 3 "NOT rolled back" -- gb rollback "$C" 1 --site e2e-dev --yes
case_ still-081 0 '"healthy":true' "goat-processors 0.8.1" -- runs 0.8.1

python3 - "$RECORD" "$(git -C "$REPO" rev-parse HEAD)" "$("$GB" --version 2>/dev/null | head -1)" "$VM_ACCEL" "$VM_ARCH" "$N" "$FAILS" <<'PY'
import json, os, sys
rec, commit, gb, accel, arch, n, fails = sys.argv[1:8]
json.dump({"schema": "goat-e2e-runner-release/v1", "goatcore": commit, "goatbuilder": gb, "accel": accel, "arch": arch,
           "cases": int(n), "failed": int(fails), "passed": int(fails) == 0}, open(os.path.join(rec, "result.json"), "w"), indent=1)
PY
echo "runner-release: $((N - FAILS)) of $N cases passed ($VM_ARCH, $VM_ACCEL); record $RECORD"
[ "$FAILS" = 0 ]
