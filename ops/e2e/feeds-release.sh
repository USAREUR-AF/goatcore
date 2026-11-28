#!/usr/bin/env bash
# feeds-release.sh: goat-feeds, a release component from goatstuff whose SHA256SUMS is
# keyless-signed (release.bundle: sigstore; gate J24), deployed from its published
# release to its own health check and verified by both avenues, from a tree that layers
# goatstuff over this commit. docs/design/goatbuilder-build-targets.md section 3.9.
#
# The pattern of ops/e2e/pipe-release.sh: the real drivers on a fresh clone of this
# commit whose declarations are the hermetic overlay (ops/e2e/overlay.py), with the
# public table's own row of goat-feeds kept and e2e_dev opting in to it, and a bare
# remote as its trunk. goatstuff's play and its two roles are layered into the clone at
# the commit ops/goatbuilder/deploy-plays.yaml names. The host is a VM from a published
# Ubuntu cloud image (ops/e2e/vm/vm.sh), prepared as a goatnet's core machine is for
# goat-feeds: docker, the site database container postgres-a, a Hydra the fence's client
# is registered with, and the paths and user the service reads with. The images are
# pinned by digest, pulled on this machine and loaded into the VM; the release itself
# is fetched and verified on this machine and copied to the host, which needs no
# network for it. It needs no sudo and changes nothing on this machine outside --work
# and its VM.
#
# goat-feeds has one signed release (0.5.0). A rollback to a file before needs a second;
# until one is pinned (FEEDS_PIN_PREV and its version) the rollback cases check only that
# a host with no earlier file refuses and keeps running the release.
#
# usage: feeds-release.sh --goatbuilder BIN [--work DIR] [--record DIR] [--keep]
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
    -h|--help) sed -n '2,25p' "$0"; exit 0 ;;
    *) echo "feeds-release: unknown argument $1" >&2; exit 2 ;;
  esac
done
[ -n "$GB" ] && [ -x "$GB" ] || { echo "feeds-release: --goatbuilder names a goatbuilder binary" >&2; exit 2; }
for t in docker ssh ssh-keygen git go age-keygen ansible-playbook python3 qemu-img curl sha256sum gzip; do
  command -v "$t" >/dev/null 2>&1 || { echo "feeds-release: $t is needed" >&2; exit 2; }
done
command -v cloud-localds >/dev/null 2>&1 || command -v xorriso >/dev/null 2>&1 || { echo "feeds-release: cloud-localds or xorriso is needed" >&2; exit 2; }
case "$(uname -m)" in
  x86_64) ARCH=amd64 ;;
  aarch64|arm64) ARCH=arm64 ;;
  *) echo "feeds-release: no published release for $(uname -m)" >&2; exit 2 ;;
esac
[ -n "$WORK" ] || WORK="$(mktemp -d "${TMPDIR:-/tmp}/goat-e2e-feeds.XXXXXX")"
mkdir -p "$WORK"; WORK="$(cd "$WORK" && pwd)"
[ -n "$RECORD" ] || RECORD="$WORK/report"
mkdir -p "$RECORD"; RECORD="$(cd "$RECORD" && pwd)"
die() { echo "feeds-release: $*" >&2; exit 1; }
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

# The published release: a pin's digest is its SHA256SUMS file's SHA-256.
SRC=USAREUR-AF/goatstuff
PIN_ROW="$SRC@goat-feeds-v0.5.0@sha256:a7f5d27d53a4bea28aee34137fa1727c88bb20bc0e11cb60f57a9890833b78a8"
D_ROW="sha256:a7f5d27d53a4bea28aee34137fa1727c88bb20bc0e11cb60f57a9890833b78a8"
V_ROW=0.5.0
PIN_PREV="${FEEDS_PIN_PREV:-}"; V_PREV="${FEEDS_VERSION_PREV:-}"
C=goat-feeds
python3 - "$REPO/ops/pins/fleet-pins.yaml" "$C" "$PIN_ROW" <<'PY' || die "the public table's $C is not pinned at $PIN_ROW"
import sys, yaml
c = yaml.safe_load(open(sys.argv[1]))["components"][sys.argv[2]]
sys.exit(0 if c.get("fleet") == sys.argv[3] and c.get("opt_in") is True else 1)
PY
GOATSTUFF_COMMIT="$(python3 -c 'import sys, yaml; print(yaml.safe_load(open(sys.argv[1]))["repositories"]["goatstuff"]["commit"])' "$REPO/ops/goatbuilder/deploy-plays.yaml")"
# The site's own services the play reads, at the digests goatcore runs them at.
PG_IMAGE="postgres:16.4@sha256:e62fbf9d3e2b49816a32c400ed2dba83e3b361e6833e624024309c35d334b412"
HYDRA_IMAGE="oryd/hydra:v2.3.0@sha256:38bbfe332099e9214b8041dffe5e3eaa271140993f148d3c8a5f7af825013097"

# An isolated HOME, as pipe-release.sh: an ssh config, the throwaway CA, the kept releases.
HOME_REAL="$HOME"; export HOME="$WORK/home"; mkdir -p "$HOME/.ssh" "$WORK/bin"; chmod 700 "$HOME/.ssh"
export GOMODCACHE="${GOMODCACHE:-$HOME_REAL/go/pkg/mod}" GOCACHE="${GOCACHE:-$HOME_REAL/.cache/go-build}" GOPATH="${GOPATH:-$HOME_REAL/go}"
export ANSIBLE_COLLECTIONS_PATH="${ANSIBLE_COLLECTIONS_PATH:-$HOME_REAL/.ansible/collections:/usr/share/ansible/collections}"
export VM_WORK="$WORK/vm" VM_ARCH="$ARCH" VM_UBUNTU=24.04 VM_IMAGES="${VM_IMAGES:-$HOME_REAL/.cache/goat-vm-images}" VM_MEM="${VM_MEM:-3072}" VM_CPUS="${VM_CPUS:-2}" VM_TAPS=0 VM_NAME=e2e-feeds VM_BOOT_WAIT="${VM_BOOT_WAIT:-300}"
VM_PORT="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')"
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

cleanup() {
  if [ "$KEEP" = 1 ]; then echo "kept: ssh -i $VM_WORK/key -p $VM_PORT goatvm@127.0.0.1"; return; fi
  vm_down
}
trap 'rc=$?; cleanup; cp "$VM_WORK/console.log" "$RECORD/" 2>/dev/null || true; exit $rc' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

on_host() { vm_ssh "$@"; }

say "the site's services, pulled here: $PG_IMAGE, $HYDRA_IMAGE, and the fence's image the role pins"
GS="$WORK/goatstuff"
git init -q "$GS"
git -C "$GS" fetch -q --depth 1 "https://github.com/$SRC" "$GOATSTUFF_COMMIT"
git -C "$GS" checkout -q FETCH_HEAD
[ "$(git -C "$GS" rev-parse HEAD)" = "$GOATSTUFF_COMMIT" ] || die "goatstuff is not at $GOATSTUFF_COMMIT"
FENCE_IMAGE="$(sed -n 's/^goat_feeds_oauth2_proxy_image: "\(.*\)"$/\1/p' "$GS/ops/ansible/roles/goat-feeds/defaults/main.yml")"
case "$FENCE_IMAGE" in *@sha256:*) ;; *) die "the fence's image is not pinned by digest: $FENCE_IMAGE" ;; esac
for img in "$PG_IMAGE" "$HYDRA_IMAGE" "$FENCE_IMAGE"; do
  docker pull -q --platform "linux/$ARCH" "$img" >/dev/null || die "could not pull $img"
done
docker save "$PG_IMAGE" "$HYDRA_IMAGE" "$FENCE_IMAGE" | gzip -1 > "$WORK/images.tar.gz"

say "the host: Ubuntu $VM_UBUNTU $VM_ARCH in a VM"
vm_up || die "the VM did not come up"
echo "up, accel=$VM_ACCEL, image sha256 $VM_IMAGE_SHA256, its checksums signed by Ubuntu's key: $VM_SIGNED"

say "the host as a core machine is for goat-feeds: docker, postgres-a, a Hydra, the service's user and paths"
on_host "sudo DEBIAN_FRONTEND=noninteractive apt-get -qq update && sudo DEBIAN_FRONTEND=noninteractive apt-get -qq install -y docker.io docker-compose-v2 >/dev/null" \
  || die "docker did not install on the host"
vm_ssh "sudo docker load -q" < "$WORK/images.tar.gz" >/dev/null || die "the images did not load on the host"
on_host "sudo docker run -d --name postgres-a --restart unless-stopped -e POSTGRES_PASSWORD=e2e-only -p 127.0.0.1:5432:5432 '$PG_IMAGE' >/dev/null" \
  || die "postgres-a did not start"
on_host "sudo docker run -d --name hydra --restart unless-stopped -e DSN=memory -e SECRETS_SYSTEM=e2e-only-hydra-system-secret-0123456789 -e URLS_SELF_ISSUER=http://127.0.0.1:4444/ -p 127.0.0.1:4444:4444 -p 127.0.0.1:4445:4445 '$HYDRA_IMAGE' serve all --dev >/dev/null" \
  || die "hydra did not start"
on_host "for i in \$(seq 60); do sudo docker exec postgres-a pg_isready -U postgres -q && curl -fsS http://127.0.0.1:4445/health/ready >/dev/null && exit 0; sleep 2; done; exit 1" \
  || die "postgres-a or hydra did not become ready"
# The paths goat-feeds' unit reads read-only, and the user it runs as, as a processors host has them.
on_host "sudo useradd --system --no-create-home --shell /usr/sbin/nologin goat-processors && sudo install -d -m 0755 /etc/goat-xf /etc/goat-processors/tls /opt/goat-xf" \
  || die "the service's user and paths were not made"

say "source of record: a bare remote and a clone of $(git -C "$REPO" rev-parse --short HEAD), goatstuff $GOATSTUFF_COMMIT layered in"
ORIGIN="$WORK/origin.git"; CLONE="$WORK/goat"
git clone -q --bare "$REPO" "$ORIGIN"
git -C "$ORIGIN" update-ref refs/heads/main "$(git -C "$REPO" rev-parse HEAD)"
git -C "$ORIGIN" symbolic-ref HEAD refs/heads/main
git clone -q --branch main "$ORIGIN" "$CLONE"
git -C "$CLONE" config user.name "e2e harness"; git -C "$CLONE" config user.email e2e@localhost
git -C "$CLONE" config commit.gpgsign false

# The goatnet's settings for goat-feeds on e2e_dev, as its group_vars give them.
cat > "$WORK/goat-feeds.yml" <<'EOF'
---
# HERMETIC (ops/e2e/feeds-release.sh): goat-feeds on e2e_dev.
goat_feeds_host: e2e-dev-01
goat_feeds_bind_ip: 127.0.0.1
goat_feeds_oidc_issuer_url: http://127.0.0.1:4444/
goat_feeds_oidc_host_entry: "invalid.invalid:127.0.0.1"
goat_feeds_oauth2_allowed_groups: e2e
goat_feeds_catalog: []
EOF
say "overlay: the hermetic goatnets, e2e_dev opted in to $C"
python3 "$HERE/overlay.py" "$CLONE" --home "$HOME" --ssh-key "$VM_WORK/key" --ssh-user goatvm --host "e2e_dev=127.0.0.1:$VM_PORT" \
  --canary-ref "ghcr.io/{{ ghcr_org }}/e2e-canary:v0.0.0@sha256:$(printf '0%.0s' $(seq 64))" \
  --public-component "$C" --opt-in "e2e_dev=$C" --member-vars "e2e_dev_hosts=$WORK/goat-feeds.yml"
SCR="$CLONE/ops/scripts"; cd "$CLONE"
gb() { "$GB" "$@"; }
deploy_sh() { "$SCR/deploy-component.sh" "$C" --goatnet e2e_dev "$@"; }
deploy_gb() { gb deploy --env e2e_dev --component "$C" --ack "e2e harness" --yes "$@"; }
untouched() { # the host answers, and has nothing of goat-feeds
  local r; r="$(on_host "if sudo test -e /opt/goat/releases/$C -o -e /opt/goat-feeds -o -e /etc/goat-feeds; then echo TOUCHED; else echo UNTOUCHED; fi")" || { echo "the host did not answer"; return 1; }
  if [ "$r" = UNTOUCHED ]; then echo "the host was not touched"; else echo "the host was touched"; return 1; fi
}
# shellcheck disable=SC2016 # expanded on the host
runs() { # <version>: the service runs the verified release file and answers its health check
  on_host "systemctl is-active goat-feeds && sudo readlink -e /opt/goat-feeds/goat-feeds | grep -E '^/opt/goat/releases/$C/[0-9a-f]{64}/goat-feeds-$1-linux-$ARCH\$' && code=\$(curl -s -o /tmp/healthz -w '%{http_code}' http://127.0.0.1:8098/healthz) && echo \"healthz \$code\" && cat /tmp/healthz && case \$code in 200|503) ;; *) exit 1 ;; esac"
}
VERIFIED="release verified: goat-feeds-${V_ROW}-linux-$ARCH ($ARCH) sha256"
SIGNED="signed by https://github.com/$SRC/.github/workflows/goat-feeds-release.yml@refs/tags/goat-feeds-v"
CONVERGED="every host's ledger names the declared ref"

say "cases: goatcore alone does not have goat-feeds' play, and deploy refuses before any host"
case_ no-layer-sh 1 "declares deploy_playbook: goat-feeds-deploy.yml, which is not in ops/ansible/" -- deploy_sh --dry-run
case_ no-layer-touched-nothing 0 "the host was not touched" -- untouched

say "layer: goatstuff's play and roles at $GOATSTUFF_COMMIT"
mkdir -p "$CLONE/ops/ansible/roles"
cp "$GS/ops/ansible/goat-feeds-deploy.yml" "$CLONE/ops/ansible/"
cp -R "$GS/ops/ansible/roles/goat-feeds" "$GS/ops/ansible/roles/goat-feeds-oidc-client" "$CLONE/ops/ansible/roles/"
git -C "$CLONE" add -A ops/ansible && git -C "$CLONE" commit -q -m "e2e: layer goatstuff $GOATSTUFF_COMMIT (goat-feeds)" && git -C "$CLONE" push -q origin main

say "cases: a dry run fetches the release and verifies its signature, and touches no host"
case_ dry-sh 0 "$VERIFIED" "${SIGNED}${V_ROW}" "goat-feeds-deploy.yml" "DRY: nothing was read" -- deploy_sh --dry-run
case_ dry-gb 0 "$VERIFIED" "${SIGNED}${V_ROW}" "goat-feeds-deploy.yml" "DRY: nothing was read" -- deploy_gb --dry-run
case_ dry-touched-nothing 0 "the host was not touched" -- untouched

say "cases: a goatnet that did not opt in does not run it"
NOTOPTED="does not run component"
case_ not-opted-in-sh 1 "$NOTOPTED" "it is opt-in, and the goatnet does not list it under opt_in:" -- "$SCR/deploy-component.sh" "$C" --goatnet e2e_preprod --dry-run
case_ not-opted-in-gb 2 "$NOTOPTED" "it is opt-in, and the goatnet does not list it under opt_in:" -- gb deploy --env e2e_preprod --component "$C" --ack "e2e harness" --yes --dry-run

say "cases: a goatnet that supplies no fence settings, or a trial's file beside the release, is refused before anything is installed"
noconfig() {
  (cd "$CLONE/ops/ansible" && ansible-playbook -i inventory/mgmt-hosts.ini goat-feeds-deploy.yml -e target=e2e_dev_hosts \
    -e release_file_component="$C" -e goat_feeds_oidc_issuer_url=)
}
both() {
  (cd "$CLONE/ops/ansible" && ansible-playbook -i inventory/mgmt-hosts.ini goat-feeds-deploy.yml -e target=e2e_dev_hosts \
    -e release_file_component="$C" -e 'goat_feeds_binary_src=/nonexistent/goat-feeds-{arch}')
}
case_ no-config 2 "Nothing was installed" "failed=1" -- noconfig
case_ trial-and-release 2 "not both and not" "failed=1" -- both
case_ refused-touched-nothing 0 "the host was not touched" -- untouched

say "cases: the script avenue deploys $V_ROW to its health check; the binary avenue converges on it too"
case_ deploy-sh 0 "$VERIFIED" "deploying $PIN_ROW" "converged on $D_ROW" "$CONVERGED" -- deploy_sh
case_ host-runs-sh 0 "healthz" "goat-feeds-$V_ROW-linux-$ARCH" -- runs "$V_ROW"
case_ verify-sh 0 "converged on $D_ROW" "$CONVERGED" -- deploy_sh --verify-only
case_ verify-gb 0 "converged on $D_ROW" -- gb verify "$C" --env e2e_dev
case_ again-gb 0 "this run wrote no new ledger row" "converged on $D_ROW" -- deploy_gb
case_ host-runs-gb 0 "healthz" "goat-feeds-$V_ROW-linux-$ARCH" -- runs "$V_ROW"

say "cases: the release file changed on the host is put back by the next deploy"
# The program runs, so its file is replaced, not written to.
on_host "sudo sh -c 'p=\$(readlink -e /opt/goat-feeds/goat-feeds); cp -p \"\$p\" /tmp/changed && echo tampered >> /tmp/changed && mv -f /tmp/changed \"\$p\"'"
case_ changed-file-redeployed-gb 0 "converged on $D_ROW" -- deploy_gb
# shellcheck disable=SC2016 # expanded on the host
put_back() { on_host 'p=$(sudo readlink -e /opt/goat-feeds/goat-feeds); if sudo tail -c 9 "$p" | grep -q tampered; then echo still changed; else echo the program is as released; fi'; }
case_ changed-file-put-back 0 "the program is as released" -- put_back
case_ changed-file-runs 0 "healthz" "goat-feeds-$V_ROW-linux-$ARCH" -- runs "$V_ROW"

if [ -z "$PIN_PREV" ]; then
  say "cases: rollback with no earlier release on the host is refused, and the release keeps running"
  case_ rollback-none-gb 3 "NOT rolled back" -- gb rollback "$C" 1 --site e2e-dev --yes
  case_ rollback-none-runs 0 "healthz" "goat-feeds-$V_ROW-linux-$ARCH" -- runs "$V_ROW"
else
  say "cases: rollback runs the release before, and it is healthy"
  to_prev() {
    python3 - "$CLONE/ops/pins/fleet-pins.yaml" "$C" "$V_PREV" <<'PY'
import sys, yaml
p, c, v = sys.argv[1:4]
d = yaml.safe_load(open(p))
d["components"][c]["release"]["files"] = {a: "goat-feeds-%s-linux-%s" % (v, a) for a in ("amd64", "arm64")}
open(p, "w").write(yaml.safe_dump(d, sort_keys=False, width=200))
PY
    python3 "$HERE/overlay.py" "$CLONE" --home "$HOME" --repin "$C=$PIN_PREV" > "$RECORD/repin-prev.log" 2>&1
  }
  to_row() {
    python3 - "$CLONE/ops/pins/fleet-pins.yaml" "$C" "$V_ROW" <<'PY'
import sys, yaml
p, c, v = sys.argv[1:4]
d = yaml.safe_load(open(p))
d["components"][c]["release"]["files"] = {a: "goat-feeds-%s-linux-%s" % (v, a) for a in ("amd64", "arm64")}
open(p, "w").write(yaml.safe_dump(d, sort_keys=False, width=200))
PY
    python3 "$HERE/overlay.py" "$CLONE" --home "$HOME" --repin "$C=$PIN_ROW" > "$RECORD/repin-row.log" 2>&1
  }
  to_prev || die "the re-pin to $PIN_PREV failed"
  case_ deploy-prev-sh 0 "deploying $PIN_PREV" "$CONVERGED" -- deploy_sh
  to_row || die "the re-pin to $PIN_ROW failed"
  case_ deploy-row-gb 0 "converged on $D_ROW" -- deploy_gb
  case_ rollback-sh 0 "every host is on the file before" -- "$SCR/rollback.sh" "$C" 1 --site e2e-dev
  case_ rollback-runs-prev 0 "healthz" "goat-feeds-$V_PREV-linux-$ARCH" -- runs "$V_PREV"
  case_ verify-names-the-rollback-gb 3 "NOT converged — rolled back" -- gb verify "$C" --env e2e_dev
  case_ forward-again-gb 0 "converged on $D_ROW" -- deploy_gb
  case_ forward-runs-row 0 "healthz" "goat-feeds-$V_ROW-linux-$ARCH" -- runs "$V_ROW"
fi

python3 - "$RECORD" "$(git -C "$REPO" rev-parse HEAD)" "$("$GB" --version 2>/dev/null | head -1)" "$VM_ACCEL" "$VM_ARCH" "$N" "$FAILS" "$GOATSTUFF_COMMIT" <<'PY'
import json, os, sys
rec, commit, gb, accel, arch, n, fails, gs = sys.argv[1:9]
json.dump({"schema": "goat-e2e-feeds-release/v1", "goatcore": commit, "goatstuff": gs, "goatbuilder": gb, "accel": accel,
           "arch": arch, "cases": int(n), "failed": int(fails), "passed": int(fails) == 0}, open(os.path.join(rec, "result.json"), "w"), indent=1)
PY
echo "feeds-release: $((N - FAILS)) of $N cases passed ($VM_ARCH, $VM_ACCEL); record $RECORD"
[ "$FAILS" = 0 ]
