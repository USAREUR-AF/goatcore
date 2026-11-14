#!/usr/bin/env bash
# hermetic.sh — the deployment engineering, end to end, against hosts that cost nothing.
#
# docs/design/deployment-e2e.md §Hermetic. The REAL drivers (deploy-component.sh,
# rc.sh, rollback.sh, fastlane.sh, pr-land.sh, ansible-preflight.sh, and the
# goatbuilder binary when given) run unmodified against a fresh clone of this
# repository whose declarations name three hermetic goatnets (ops/e2e/overlay.py),
# a local bare git remote standing in for origin, a local TLS registry that the
# runner resolves as ghcr.io, and three containers that look like management
# hosts (ops/e2e/host/). gh, crane and aws-vault are stubs on PATH
# (ops/e2e/stubs/, every call recorded). The only proof of a deploy is the ledger
# row read back from the host.
#
# usage: ops/e2e/hermetic.sh [--work DIR] [--record DIR] [--goatbuilder BIN] [--no-hosts] [--keep]
#                            [--host-distro ubuntu|debian]
#   --no-hosts   no containers, no registry: the dry-run and refusal cases only
#                (what a laptop without Docker can run; Linux + Docker for the rest)
#   --keep       leave the work directory and containers for inspection
#   --host-distro the management hosts' distribution: ubuntu (default, ops/e2e/host/Dockerfile)
#                or debian (ops/e2e/host/Dockerfile.debian); the same cases run on either
#
# Exit 0 when every case passed or failed as expected (xfail); 1 otherwise.
# The record: $RECORD/report.jsonl (ops/e2e/report.schema.json), one log per
# case, ledgers.jsonl, and the stubs' call logs; ops/e2e/report.py renders it.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"
WORK=""; RECORD=""; GB=""; HOSTS=1; KEEP=0; DISTRO=ubuntu
while [ $# -gt 0 ]; do
  case "$1" in
    --work) WORK="$2"; shift 2 ;;
    --record) RECORD="$2"; shift 2 ;;
    --goatbuilder) GB="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"; shift 2 ;;
    --no-hosts) HOSTS=0; shift ;;
    --host-distro) DISTRO="$2"; shift 2 ;;
    --keep) KEEP=1; shift ;;
    -h|--help) sed -n '2,24p' "$0"; exit 0 ;;
    *) echo "hermetic: unknown argument $1" >&2; exit 2 ;;
  esac
done
case "$DISTRO" in
  ubuntu) HOST_DOCKERFILE=Dockerfile ;;
  debian) HOST_DOCKERFILE=Dockerfile.debian ;;
  *) echo "hermetic: --host-distro is ubuntu or debian, not '$DISTRO'" >&2; exit 2 ;;
esac
[ -n "$WORK" ] || WORK="$(mktemp -d "${TMPDIR:-/tmp}/goat-e2e.XXXXXX")"
mkdir -p "$WORK"; WORK="$(cd "$WORK" && pwd)"
[ -n "$RECORD" ] || RECORD="$WORK/report"
mkdir -p "$RECORD"; export E2E_RECORD_DIR="$RECORD"
die() { echo "hermetic: $*" >&2; exit 1; }
say() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }
REC="$HERE/rec.py"
FAILS=0
rec() { python3 "$REC" "$@" || FAILS=$((FAILS + 1)); }

# --- prerequisites: every input this run needs, named before anything runs --------
if [ "$HOSTS" = 1 ]; then "$HERE/prereqs.sh" || die "prerequisites missing (above)"; else "$HERE/prereqs.sh" --no-hosts || die "prerequisites missing (above)"; fi

# --- an isolated HOME: ssh config, the throwaway CA, nothing of the operator's --------
export HOME_REAL="$HOME"
export HOME="$WORK/home"; mkdir -p "$HOME/.ssh"; chmod 700 "$HOME/.ssh"
export GOMODCACHE="${GOMODCACHE:-$HOME_REAL/go/pkg/mod}"
export GOCACHE="${GOCACHE:-$HOME_REAL/.cache/go-build}"; [ "$(uname -s)" = Darwin ] && export GOCACHE="${GOCACHE_DARWIN:-$HOME_REAL/Library/Caches/go-build}"
export GOFLAGS="${GOFLAGS:-}"; export GOPATH="${GOPATH:-$HOME_REAL/go}"
SSH_KEY="$WORK/ssh/id_ed25519"; mkdir -p "$WORK/ssh"
[ -f "$SSH_KEY" ] || ssh-keygen -q -t ed25519 -N '' -f "$SSH_KEY" -C e2e-harness
cat > "$HOME/.ssh/config" <<EOF
Host *
  IdentityFile $SSH_KEY
  IdentitiesOnly yes
  StrictHostKeyChecking no
  UserKnownHostsFile /dev/null
  LogLevel ERROR
EOF
chmod 600 "$HOME/.ssh/config"
# OpenSSH reads ~/.ssh/config from the passwd home directory, not $HOME, so the
# config above is invisible to every ssh the drivers run (rollback.sh, ansible,
# this script). A one-line wrapper first on PATH passes it explicitly; nothing
# else about ssh changes.
mkdir -p "$WORK/bin"
REAL_SSH="$(command -v ssh)"
printf '#!/bin/sh\nexec "%s" -F "%s" "$@"\n' "$REAL_SSH" "$HOME/.ssh/config" > "$WORK/bin/ssh"
chmod +x "$WORK/bin/ssh"
export PATH="$WORK/bin:$PATH"
export ANSIBLE_HOST_KEY_CHECKING=False
# The bare remote carries no LFS objects (the briefs' PDFs); a clone must not try to smudge them.
export GIT_LFS_SKIP_SMUDGE=1

# --- the bare origin and the clone under test ---------------------------------------
say "source of record: a bare remote and a clone of $(git -C "$REPO" rev-parse --short HEAD)"
ORIGIN="$WORK/origin.git"; CLONE="$WORK/goat"
rm -rf "$ORIGIN" "$CLONE"
git clone -q --bare "$REPO" "$ORIGIN"
git -C "$ORIGIN" update-ref refs/heads/main "$(git -C "$REPO" rev-parse HEAD)"
git -C "$ORIGIN" symbolic-ref HEAD refs/heads/main
git clone -q --branch main "$ORIGIN" "$CLONE"
git -C "$CLONE" config user.name "e2e harness"; git -C "$CLONE" config user.email e2e@localhost
# The image tags must name commits the clone holds, exactly as CI's sha-<short>
# and release tags do: affected-flows.py diffs from the fleet tag's commit to
# the candidate's (rc.sh refuses when it cannot). The base canary is the git tag
# e2e-canary/v0.0.0 on the commit under test; the two hotfixes are that commit
# and its parent.
HOT1="$(git -C "$CLONE" rev-parse --short=7 HEAD)"; HOT2="$(git -C "$CLONE" rev-parse --short=7 HEAD~1)"
git -C "$CLONE" tag e2e-canary/v0.0.0 HEAD && git -C "$CLONE" push -q origin e2e-canary/v0.0.0
export E2E_ORIGIN="$ORIGIN"; export E2E_GH_STATE="$WORK/gh-state"; rm -rf "$E2E_GH_STATE"
export PATH="$HERE/stubs:$PATH"
export GOAT_RC_ROLE_CHECK=0 GOAT_MESH_PROBE=0 GOAT_SKIP_GHCR_PARITY=1 GOAT_ANSIBLE_TRUNK_REMOTE=origin
export GOAT_DEPLOY_LOG_DIR="$RECORD"
unset GCP_HEIMDALL_OP_ITEM 2>/dev/null || true

# --- hosts, registry, images --------------------------------------------------------
declare -a HOST_ARGS=()
CANARY_DIGEST=""; HOT_DIGEST=""; RC2_DIGEST=""
export E2E_REGISTRY="ghcr.io" E2E_REGISTRY_URL="https://ghcr.io"
if [ "$HOSTS" = 1 ]; then
  say "registry: a local TLS registry this machine resolves as ghcr.io"
  CERTS="$WORK/certs"; mkdir -p "$CERTS"
  openssl req -x509 -newkey rsa:2048 -nodes -keyout "$CERTS/key.pem" -out "$CERTS/cert.pem" -days 2 -subj '/CN=ghcr.io' -addext 'subjectAltName=DNS:ghcr.io' >/dev/null 2>&1
  export E2E_REGISTRY_CA="$CERTS/cert.pem"
  sudo mkdir -p /etc/docker/certs.d/ghcr.io && sudo cp "$CERTS/cert.pem" /etc/docker/certs.d/ghcr.io/ca.crt
  # Pulled by digest before ghcr.io is re-pointed (the Dockerfiles pin the same digests).
  REGISTRY_IMAGE="registry:2@sha256:a3d8aaa63ed8681a604f1dea0aa03f100d5895b6a58ace528858a7b332415373"
  for img in busybox:1.37.0-musl@sha256:5cec3fc171c87218698e85a52af7087de727372aae264a787b8112901a5b0092 "$REGISTRY_IMAGE" \
             docker:27.3.1-cli@sha256:328eb399a065780c2cebe9224de003aa14084cf69efae882ac27430f921819b7 \
             docker/compose-bin:v2.29.7@sha256:25beb180af2138ac58697b2c229bd7df9992b8d47a8272008c1f1a32d3130253 \
             ubuntu:24.04@sha256:24737cef17f4e22b1977df7d9a05e77c4e28b7da5b8911f8d8ccda136bf3573c; do
    docker pull -q "$img" >/dev/null
  done
  docker rm -f e2e-registry e2e-dev-01 e2e-preprod-01 e2e-prod-01 >/dev/null 2>&1 || true
  docker network inspect e2e-net >/dev/null 2>&1 || docker network create e2e-net >/dev/null
  docker run -d --name e2e-registry -p 443:443 -v "$CERTS:/certs:ro" \
    -e REGISTRY_HTTP_ADDR=0.0.0.0:443 -e REGISTRY_HTTP_TLS_CERTIFICATE=/certs/cert.pem -e REGISTRY_HTTP_TLS_KEY=/certs/key.pem "$REGISTRY_IMAGE" >/dev/null
  grep -q ' ghcr.io$' /etc/hosts || echo "127.0.0.1 ghcr.io" | sudo tee -a /etc/hosts >/dev/null
  for _i in $(seq 1 30); do curl -s --cacert "$CERTS/cert.pem" https://ghcr.io/v2/ >/dev/null 2>&1 && break; sleep 1; done
  curl -s --cacert "$CERTS/cert.pem" https://ghcr.io/v2/ >/dev/null || die "the hermetic registry did not come up as ghcr.io"
  # dockerd is a Go program: its resolver caches /etc/hosts for up to 5 s, so a
  # push right after the re-point can still reach the real ghcr.io. Wait it out.
  sleep 6
  say "images: the canary at three stamps"
  build_push() { # $1 tag, $2 stamp -> the digest the registry holds
    docker build -q --build-arg "CANARY_STAMP=$2" -t "ghcr.io/usareur-af/e2e-canary:$1" "$HERE/canary" >/dev/null
    docker push "ghcr.io/usareur-af/e2e-canary:$1" > "$RECORD/push-$1.log" 2>&1 || die "docker push of e2e-canary:$1 to the hermetic registry failed: $(tail -3 "$RECORD/push-$1.log" | tr '\n' ' ')"
    crane digest "ghcr.io/usareur-af/e2e-canary:$1" || die "the hermetic registry holds no e2e-canary:$1 after the push (see $RECORD/push-$1.log)"
  }
  CANARY_DIGEST="$(build_push v0.0.0 base)"
  RC_DIGEST="$(build_push v0.1.0-rc.1 rc1)"
  RC2_DIGEST="$(build_push v0.1.0-rc.2 rc2)"
  RC3_DIGEST="$(build_push v0.1.0-rc.3 rc3)"
  HOT_DIGEST="$(build_push sha-$HOT1 hotfix)"
  HOT2_DIGEST="$(build_push sha-$HOT2 hotfix2)"
  say "hosts: three containers that look like management hosts"
  docker build -q -f "$HERE/host/$HOST_DOCKERFILE" -t e2e-host "$HERE/host" >/dev/null
  for h in dev preprod prod; do
    docker run -d --name "e2e-$h-01" --hostname "e2e-$h-01" --network e2e-net --add-host ghcr.io:host-gateway \
      -v /var/run/docker.sock:/var/run/docker.sock -v "$SSH_KEY.pub:/pubkey:ro" e2e-host >/dev/null
    ip="$(docker inspect -f '{{ (index .NetworkSettings.Networks "e2e-net").IPAddress }}' "e2e-$h-01")"
    HOST_ARGS+=(--host "e2e_$h=$ip")
    for _i in $(seq 1 30); do ssh -o ConnectTimeout=2 "goat@$ip" true 2>/dev/null && break; sleep 1; done
    ssh -o ConnectTimeout=2 "goat@$ip" "sudo docker version --format '{{.Server.Version}}'" >/dev/null || die "host e2e-$h-01 ($ip) cannot reach the Docker daemon"
  done
else
  CANARY_DIGEST="sha256:$(printf 'e2e-canary base' | shasum -a 256 | cut -c1-64)"
fi
CANARY_REF="ghcr.io/{{ ghcr_org }}/e2e-canary:v0.0.0@$CANARY_DIGEST"

# --- the overlay -------------------------------------------------------------------
say "overlay: three goatnets, two canaries, a throwaway CA, the ladder pointed at them"
declare -a OVERLAY_ARGS=()
[ "$HOSTS" = 1 ] || OVERLAY_ARGS+=(--no-hosts)
python3 "$HERE/overlay.py" "$CLONE" --home "$HOME" --ssh-key "$SSH_KEY" --canary-ref "$CANARY_REF" "${OVERLAY_ARGS[@]}" "${HOST_ARGS[@]}"
SCR="$CLONE/ops/scripts"
export CLONE   # the ledger probes run in child shells (bash -c) and read the clone's inventory
cleanup() {
  if [ "$KEEP" = 0 ] && [ "$HOSTS" = 1 ]; then docker rm -f e2e-registry e2e-dev-01 e2e-preprod-01 e2e-prod-01 >/dev/null 2>&1 || true; docker ps -aq --filter name=e2e-canary- | xargs -r docker rm -f >/dev/null 2>&1 || true; fi
  [ "$KEEP" = 0 ] && rm -rf "$WORK/goat-wt-rc-"* 2>/dev/null || true
}
trap cleanup EXIT

# --- helpers --------------------------------------------------------------------------
ledger_row() { # $1 goatnet, $2 component -> the newest ledger row on that goatnet's host
  local ip; ip="$(awk -v g="[${1}_hosts]" '$0==g{f=1;next} /^\[/{f=0} f && /ansible_host=/{sub(/.*ansible_host=/,""); sub(/[[:space:]].*/,""); print; exit}' "$CLONE/ops/ansible/inventory/mgmt-hosts.ini")"
  ssh "goat@$ip" "sudo tail -n 1 /var/lib/goat/$2/deploy-history.jsonl 2>/dev/null || echo NO_LEDGER"
}
host_os() { # $1 goatnet -> the ID in its host's /etc/os-release
  local ip; ip="$(awk -v g="[${1}_hosts]" '$0==g{f=1;next} /^\[/{f=0} f && /ansible_host=/{sub(/.*ansible_host=/,""); sub(/[[:space:]].*/,""); print; exit}' "$CLONE/ops/ansible/inventory/mgmt-hosts.ini")"
  # shellcheck disable=SC2016 # $ID is the host's, read there
  ssh "goat@$ip" '. /etc/os-release && echo "$ID"'
}
ledger_rows() { # $1 goatnet, $2 component -> row count
  local ip; ip="$(awk -v g="[${1}_hosts]" '$0==g{f=1;next} /^\[/{f=0} f && /ansible_host=/{sub(/.*ansible_host=/,""); sub(/[[:space:]].*/,""); print; exit}' "$CLONE/ops/ansible/inventory/mgmt-hosts.ini")"
  ssh "goat@$ip" "sudo wc -l < /var/lib/goat/$2/deploy-history.jsonl 2>/dev/null || echo 0"
}
record_ledgers() {
  for g in e2e_dev e2e_preprod e2e_prod; do for c in e2e-canary e2e-canary-secret; do
    printf '{"host":"%s","component":"%s","row":%s}\n' "$g" "$c" "$(ledger_row "$g" "$c" | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read().strip()))')" >> "$RECORD/ledgers.jsonl"
  done; done
}
dev_pin_prs() { python3 -c 'import json,sys,os; p=os.path.join(sys.argv[1],"prs.json"); d=json.load(open(p)) if os.path.exists(p) else {"prs":{}}; print(sum(1 for v in d["prs"].values() if "e2e_dev runs release candidate" in v.get("title","")))' "$E2E_GH_STATE"; }
prs_opened() { python3 -c 'import json,sys,os; p=os.path.join(sys.argv[1],"prs.json"); print(json.load(open(p))["next"]-1 if os.path.exists(p) else 0)' "$E2E_GH_STATE"; }

# =====================================================================================
say "cases: dry runs and refusals (no host touched)"
cd "$CLONE"
rec validate-dry-run --expect-in "groups [e2e_dev_hosts]" --expect-in "DRY: would read /var/lib/goat/e2e-canary/deploy-history.jsonl" \
  -- "$SCR/deploy-component.sh" e2e-canary --goatnet e2e_dev --dry-run
rec refuse-missing-secret --expect-exit 1 --expect-in "NOT SET: E2E_CANARY_TOKEN" --expect-in "refusing before any host is touched" \
  --env E2E_CANARY_TOKEN= -- "$SCR/deploy-component.sh" e2e-canary-secret --goatnet e2e_dev
rec alias-target-dry-run --expect-in "e2e-canary-secret-deploy.yml -e target=<group>" \
  --env E2E_CANARY_TOKEN=hermetic -- "$SCR/deploy-component.sh" e2e-canary-secret --goatnet e2e_dev --dry-run
rec refuse-feature-branch-checkout --expect-exit 1 --expect-in "run rc.sh from a checkout on main" --cwd "$WORK" \
  -- bash -c "git clone -q '$ORIGIN' '$WORK/feature' && git -C '$WORK/feature' checkout -q -b feature/x && '$WORK/feature/ops/scripts/rc.sh' e2e-canary --version 0.1.0 --ack 'e2e harness' --dry-run"
rec release-dry-run-whole-ladder --expect-in "candidate tag e2e-canary/v0.1.0-rc.1" --expect-in "DRY: python3" --expect-in "preprod rung" \
  -- "$SCR/rc.sh" e2e-canary --version 0.1.0 --ack "e2e harness" --dry-run
# The affected rows are a diff from the fleet commit to the TAG, so the tag has
# to exist for a dry run to see them: create it on origin first (rc.sh then
# validates it as-is), as a re-run after a failure would find it.
rec release-dry-run-names-browser-toolchain --expect-in "tag exists on origin" --expect-in "rows on e2e-dev: intake-form-browser-submit" --expect-in "browser row" --expect-in "npm ci" \
  --env E2E_CANARY_TOKEN=hermetic -- bash -c "git -C '$CLONE' worktree add -q '$WORK/side-secret' origin/main && echo 'a change the secret canary carries' > '$WORK/side-secret/ops/e2e/canary/SECRET-CANDIDATE' && git -C '$WORK/side-secret' add -A && git -C '$WORK/side-secret' -c user.name=e2e -c user.email=e2e@localhost commit -q -m 'e2e: the secret canary candidate touches its mapped path' && git -C '$WORK/side-secret' tag e2e-canary-secret/v0.1.0-rc.1 && git -C '$CLONE' push -q origin e2e-canary-secret/v0.1.0-rc.1 && git -C '$CLONE' worktree remove '$WORK/side-secret' && '$SCR/rc.sh' e2e-canary-secret --version 0.1.0 --rc 1 --ack 'e2e harness' --dry-run"
rec release-dry-run-runs-secrets-check --expect-in "NOT SET: E2E_CANARY_TOKEN" --expect-in "a real run refuses here, before the tag" \
  --env E2E_CANARY_TOKEN= -- "$SCR/rc.sh" e2e-canary-secret --version 0.1.0 --ack "e2e harness" --dry-run
rec release-refuses-before-the-tag-on-a-missing-secret --expect-exit 1 --expect-in "refusing before the tag" --expect-not-in "DRY:" \
  --env E2E_CANARY_TOKEN= -- "$SCR/rc.sh" e2e-canary-secret --version 0.1.0 --ack "e2e harness"
if [ -n "$GB" ]; then
  rec goatbuilder-validate -- "$GB" validate
  rec goatbuilder-deploy-dry-run --expect-in "DRY: would read /var/lib/goat/e2e-canary/deploy-history.jsonl" -- "$GB" deploy --env e2e_dev --component e2e-canary --dry-run --yes
  rec goatbuilder-release-dry-run --expect-in "e2e-canary/v0.1.0-rc.1" -- "$GB" release e2e-canary --version 0.1.0 --ack "e2e harness" --dry-run
fi
rec doc-as-test-first-run-dry --expect-in "first-run page:" \
  -- python3 "$HERE/doc-as-test.py" "$CLONE/docs/operations/goatbuilder-first-run.md" --section "A deploy, with its proof" --only-dry-run \
     --map "<goatnet>=e2e_dev" --map "<component>=e2e-canary" --map "zagros_dev=e2e_dev" --map "netvis=e2e-canary" --map "goatbuilder=${GB:-$HERE/stubs/goatbuilder-absent}" --cwd "$CLONE"

if [ "$HOSTS" = 0 ]; then
  say "no hosts: the deploy, ladder, rollback and fastlane cases need Linux + Docker"
  python3 "$HERE/report.py" "$RECORD" --check --title "deployment e2e (hermetic, no hosts)" || true
  [ "$FAILS" = 0 ] && exit 0 || exit 1
fi

# =====================================================================================
rec fastlane-dry-run --expect-in "new fleet ref" \
  -- "$SCR/fastlane.sh" e2e-canary --ref sha-$HOT1 --reason "hermetic" --ack "e2e harness" --dry-run

say "cases: the hosts are the distribution asked for ($DISTRO)"
# Every host's /etc/os-release names $DISTRO, so a pass below is a pass on it.
rec hosts-run-the-requested-distribution --expect-in "e2e_prod $DISTRO" -- bash -c "$(declare -f host_os); for g in e2e_dev e2e_preprod e2e_prod; do
    id=\$(host_os \$g); echo \"\$g \$id\"; [ \"\$id\" = '$DISTRO' ] || exit 1; done"

say "cases: the push converger on real hosts, proved from the ledger"
rec deploy-canary-dev --expect-in "converged on $CANARY_DIGEST" --expect-in "every host's ledger names the declared ref" \
  -- "$SCR/deploy-component.sh" e2e-canary --goatnet e2e_dev
rec ledger-row-is-deploy -- bash -c "row=\$($(declare -f ledger_row); ledger_row e2e_dev e2e-canary); echo \"\$row\"; echo \"\$row\" | grep -q '\"action\": \"deploy\"' && echo \"\$row\" | grep -q '$CANARY_DIGEST'"
rec ledger-row-says-when-the-play-finished -- bash -c "row=\$($(declare -f ledger_row); ledger_row e2e_dev e2e-canary); echo \"\$row\"; echo \"\$row\" | python3 -c 'import json,sys; r=json.loads(sys.stdin.read()); p=r.get(\"promoted_at\",\"\"); sys.exit(0 if p and p >= r[\"ts\"] else 1)'"
ROWS_BEFORE="$(ledger_rows e2e_dev e2e-canary)"
rec deploy-canary-dev-again-changes-nothing --expect-in "every host's ledger names the declared ref" \
  -- "$SCR/deploy-component.sh" e2e-canary --goatnet e2e_dev
rec ledger-unchanged-on-rerun -- bash -c "[ \"\$($(declare -f ledger_rows); ledger_rows e2e_dev e2e-canary)\" = '$ROWS_BEFORE' ]"
rec deploy-secret-canary-through-alias --expect-in "every host's ledger names the declared ref" \
  --env E2E_CANARY_TOKEN=hermetic -- "$SCR/deploy-component.sh" e2e-canary-secret --goatnet e2e_dev
rec verify-only-reads-the-ledger --expect-in "converged on $CANARY_DIGEST" \
  -- "$SCR/deploy-component.sh" e2e-canary --goatnet e2e_dev --verify-only
# Served payloads: what a host serves can change with no image change (goat-get
# serves the goat CLI release a role var names). The row records the payload read
# back from the host; verify judges it against main on both avenues; a payload
# bumped on main and not deployed is drift, and the next deploy records it.
rec served-payload-recorded --expect-in "serves as declared: e2e-payload e2e-payload-v1" \
  -- "$SCR/deploy-component.sh" e2e-canary --goatnet e2e_dev --verify-only
rec served-payload-bumped-on-main-is-drift --expect-exit 1 --expect-in "served payload DRIFT — e2e-payload: serves e2e-payload-v1, main declares e2e-payload-v2" \
  -- bash -c "git clone -q '$ORIGIN' '$WORK/payload' && sed -i.bak 's/^e2e_canary_payload_tag: .*/e2e_canary_payload_tag: \"e2e-payload-v2\"/' '$WORK/payload/ops/ansible/roles/e2e-canary/defaults/main.yml' && rm -f '$WORK/payload/ops/ansible/roles/e2e-canary/defaults/main.yml.bak' && git -C '$WORK/payload' -c user.name=e2e -c user.email=e2e@localhost commit -qam 'e2e: bump the canary payload' && git -C '$WORK/payload' push -q origin main && git -C '$CLONE' fetch -q origin && git -C '$CLONE' merge -q --ff-only origin/main && '$SCR/deploy-component.sh' e2e-canary --goatnet e2e_dev --verify-only"
if [ -n "$GB" ]; then
  rec goatbuilder-served-payload-drift --expect-exit 3 --expect-in "served payload DRIFT" -- "$GB" verify e2e-canary --env e2e_dev
fi
rec served-payload-deploy-records-it --expect-in "every host's ledger names the declared ref" --expect-in "serves as declared: e2e-payload e2e-payload-v2" \
  -- "$SCR/deploy-component.sh" e2e-canary --goatnet e2e_dev
if [ -n "$GB" ]; then
  rec goatbuilder-served-payload-as-declared --expect-in "serves as declared: e2e-payload e2e-payload-v2" -- "$GB" verify e2e-canary --env e2e_dev
  rec goatbuilder-deploy-preprod --expect-in "converged on $CANARY_DIGEST" -- "$GB" deploy --env e2e_preprod --component e2e-canary --ack "e2e harness" --yes
  rec goatbuilder-verify-only --expect-in "converged on $CANARY_DIGEST" -- "$GB" deploy --env e2e_preprod --component e2e-canary --verify-only --yes
fi

say "cases: the checkout rules"
rec behind-origin-refused --expect-exit 1 --expect-in "behind" \
  -- bash -c "git clone -q '$ORIGIN' '$WORK/side' && git -C '$WORK/side' -c user.name=e2e -c user.email=e2e@localhost commit -q --allow-empty -m 'e2e: a commit the deployer has not pulled' && git -C '$WORK/side' push -q origin main && git -C '$CLONE' fetch -q origin && '$SCR/deploy-component.sh' e2e-canary --goatnet e2e_dev"
# Contract C3 on a branch (ops/scripts/fresh-source.sh): a branch is judged on
# what a deploy reads. Behind the trunk in ops/ansible: refused before any host,
# on both avenues. Rebased, carrying its own commit: runs, naming the commit. A
# trunk it cannot fetch: refused, unless --offline.
FB="$WORK/fb"; G="-c user.name=e2e -c user.email=e2e@localhost"
rec branch-behind-in-ansible-refused --expect-exit 1 --expect-in "GB-064" --expect-in "refusing before any host is touched" --expect-not-in "deploying" \
  -- bash -c "git clone -q '$ORIGIN' '$FB' && git -C '$FB' checkout -q -b feature/c3 && echo c3 > '$FB/docs/e2e-c3-branch-note' && git -C '$FB' add -A && git -C '$FB' $G commit -q -m 'e2e: a branch with its own commit' && echo c3 > '$WORK/side/ops/ansible/.e2e-c3-probe' && git -C '$WORK/side' add -A && git -C '$WORK/side' $G commit -q -m 'e2e: a role change the branch has not pulled' && git -C '$WORK/side' push -q origin main && '$FB/ops/scripts/deploy-component.sh' e2e-canary --goatnet e2e_dev"
if [ -n "$GB" ]; then
  rec goatbuilder-branch-behind-in-ansible-refused --expect-exit 1 --expect-in "GB-064" --expect-not-in "deploying" --cwd "$FB" \
    -- "$GB" deploy --env e2e_dev --component e2e-canary --ack "e2e harness" --yes
fi
rec branch-ahead-warned-and-runs --expect-in "carries 1 commit(s) not on origin/main" --expect-in "every host's ledger names the declared ref" \
  -- bash -c "git -C '$FB' fetch -q origin && git -C '$FB' $G rebase -q origin/main && '$FB/ops/scripts/deploy-component.sh' e2e-canary --goatnet e2e_dev"
rec trunk-unreachable-refused --expect-exit 1 --expect-in "GB-065" --expect-in "On an islanded site" --expect-not-in "deploying" \
  -- bash -c "git -C '$FB' remote set-url origin '$WORK/no-such-trunk.git' && '$FB/ops/scripts/deploy-component.sh' e2e-canary --goatnet e2e_dev"
rec trunk-unreachable-offline-runs --expect-in "OFFLINE" --expect-in "every host's ledger names the declared ref" \
  -- "$FB/ops/scripts/deploy-component.sh" e2e-canary --goatnet e2e_dev --offline
rec behind-origin-fast-forward-then-ok --expect-in "every host's ledger names the declared ref" \
  -- bash -c "git -C '$CLONE' fetch -q origin && git -C '$CLONE' merge -q --ff-only origin/main && '$SCR/deploy-component.sh' e2e-canary --goatnet e2e_dev"

say "cases: the release ladder, tag to prod, through rc.sh with the stubbed gh"
rec release-tag-dev-preprod --expect-in "deploy e2e-canary/v0.1.0-rc.1 to e2e_dev" --expect-in "converged on $RC_DIGEST" --expect-in "is the preprod candidate" \
  -- "$SCR/rc.sh" e2e-canary --version 0.1.0 --ack "e2e harness"
rec preprod-ledger-names-rc1 -- bash -c "$(declare -f ledger_row); ledger_row e2e_preprod e2e-canary | grep -q '$RC_DIGEST'"
# A re-run of a step that already landed lands nothing (#2046, and the deviation
# edit that keeps a re-declared deviation where it stands): the dev rung already
# runs rc.1, so re-running from dev must open no dev-pin PR. `--from dev` goes on
# to re-do the preprod step, which re-opens a candidate record — a separate
# question this case does not judge.
DEV_PRS_BEFORE="$(dev_pin_prs)"
rec release-rerun-dev-step-opens-no-pr --expect-in "pins already current on main; nothing to land" --expect-in "no dev-pin PR opened" \
  -- bash -c "'$SCR/rc.sh' e2e-canary --tag e2e-canary/v0.1.0-rc.1 --from dev --ack 'e2e harness' 2>&1 | grep -E 'nothing to land|reusing|==> '; [ \"\$($(declare -f dev_pin_prs); dev_pin_prs)\" = '$DEV_PRS_BEFORE' ] && echo 'no dev-pin PR opened'"
# ...and the preprod step reuses the record preprod already runs instead of
# opening a second one for the same ref (#2086).
rec release-rerun-reuses-the-preprod-record --expect-in "open records for the rc.1 digest: 1" \
  -- bash -c "cd '$CLONE' && git fetch -q origin && git merge -q --ff-only origin/main && echo \"open records for the rc.1 digest: \$(grep -l 'state: candidate' ops/release/records/e2e-canary/*-${RC_DIGEST:7:12}.yaml 2>/dev/null | wc -l | tr -d ' ')\""
rec soak-elapses --note "the soak gate is the real one (min_days 1); the candidate record's opened date is moved one day earlier than the day it opened and committed, so a day has passed whatever day the case runs on" \
  -- bash -c "cd '$CLONE' && git fetch -q origin && git merge -q --ff-only origin/main && python3 - <<'PY'
import datetime, glob, yaml
for f in glob.glob('ops/release/records/e2e-canary/*.yaml'):
    r = yaml.safe_load(open(f))
    if r.get('state') == 'candidate' and r.get('class') != 'hotfix':
        s = open(f).read().replace(\"opened: '%s'\" % r['opened'], \"opened: '%s'\" % (datetime.date.fromisoformat(str(r['opened'])) - datetime.timedelta(days=1)).isoformat())
        open(f, 'w').write(s); print('dated back:', f)
PY
git add -A && git commit -q -m 'e2e: the soak elapses (the record opened a day earlier)' && git push -q origin main"

rec release-prod-graduate --expect-in "graduated:" --expect-in "converged on $RC_DIGEST" --expect-in "fleet ghcr.io" \
  -- "$SCR/rc.sh" e2e-canary --tag e2e-canary/v0.1.0-rc.1 --from prod --ack "e2e harness"
rec prod-ledger-names-rc1 -- bash -c "$(declare -f ledger_row); ledger_row e2e_prod e2e-canary | grep -q '$RC_DIGEST'"
rec every-rung-reads-rc1 -- bash -c "$(declare -f ledger_row); for g in e2e_dev e2e_preprod e2e_prod; do ledger_row \$g e2e-canary | grep -q '$RC_DIGEST' || exit 1; done"

say "cases: fastlane to every goatnet, then rollback from the on-host ledger"
rec fastlane-everywhere --expect-in "is sha-$HOT1 on every goatnet, ledger-verified" \
  -- "$SCR/fastlane.sh" e2e-canary --ref sha-$HOT1 --reason "hermetic: the exception, exercised" --ack "e2e harness"
rec every-rung-reads-the-hotfix -- bash -c "$(declare -f ledger_row); for g in e2e_dev e2e_preprod e2e_prod; do ledger_row \$g e2e-canary | grep -q '$HOT_DIGEST' || exit 1; done"
rec rollback-list --expect-in "$HOT_DIGEST" -- "$SCR/rollback.sh" e2e-canary --list
rec rollback-status --expect-in "e2e-canary" -- "$SCR/rollback.sh" e2e-canary --status
# e2e-canary spans three goatnets (dev/preprod/prod); a rollback with no --site
# would flip every one, prod included, so it is refused and the operator names
# the one goatnet (GB-055). The roll itself is then scoped to e2e-dev.
rec rollback-site-less-refused --expect-exit 1 --expect-in "with no --site would flip every one" \
  -- "$SCR/rollback.sh" e2e-canary 1
rec rollback-one-step --expect-in "rolled back" -- "$SCR/rollback.sh" e2e-canary 1 --site e2e-dev
rec dev-ledger-names-the-rollback -- bash -c "$(declare -f ledger_row); ledger_row e2e_dev e2e-canary | grep -q '\"action\":\"rollback\"'"
# A rolled-back host is not "a play that did not finish": the declared digest
# sits in the row's prior_ref, and both avenues say the host was moved away.
rec verify-names-the-rollback --expect-exit 1 --expect-in "rolled back from" --expect-not-in "the play did not finish" \
  -- "$SCR/deploy-component.sh" e2e-canary --goatnet e2e_dev --verify-only
if [ -n "$GB" ]; then
  rec goatbuilder-rollback-list --expect-in "$HOT_DIGEST" -- "$GB" rollback e2e-canary --list
  rec goatbuilder-verify-names-the-rollback --expect-exit 3 --expect-in "rolled back from" --expect-not-in "the play did not finish" \
    -- "$GB" verify e2e-canary --env e2e_dev
  # Both avenues read the host ledger line for line (#2066): CI's parity gate
  # reaches no host, so the host path is compared here, on real hosts, after a
  # row whose note spans two lines is added to the ledger rollback.sh reads first.
  rec rollback-host-path-parity --expect-in "on the host path" --expect-not-in " 0 on the host path" \
    -- bash -c "h=\$(yq -r '.hosts[0].host' '$CLONE/ops/ansible/roles/e2e-canary/deploy-history.yml'); u=\$(yq -r '.hosts[0].ssh_user' '$CLONE/ops/ansible/roles/e2e-canary/deploy-history.yml'); ssh \"\$u@\$h\" 'sudo tail -n 1 /var/lib/goat/e2e-canary/deploy-history.jsonl' | python3 -c 'import json,sys; r=json.loads(sys.stdin.read()); r[\"note\"]=\"first line\\nsecond line\"; print(json.dumps(r))' | ssh \"\$u@\$h\" 'sudo tee -a /var/lib/goat/e2e-canary/deploy-history.jsonl >/dev/null' && '$CLONE/ops/goatbuilder/parity-rollback.sh' '$GB' e2e-canary"
fi

say "cases: an open candidate is abandoned when the next one reaches preprod (C6, #2039)"
rec release-rc2-to-preprod --expect-in "converged on $RC2_DIGEST" --expect-in "is the preprod candidate" \
  -- "$SCR/rc.sh" e2e-canary --version 0.1.0 --rc 2 --ack "e2e harness"

# The CLI avenue, live, on the same ladder (ADR 1201: both avenues stay
# functional; a failure on either is a failure). A third candidate through
# `goatbuilder release`, a second hotfix through `goatbuilder fastlane`, and
# `goatbuilder rollback` one step.
if [ -n "$GB" ]; then
  say "cases: the same ladder through the goatbuilder avenue"
  rec goatbuilder-release-tag-dev-preprod --expect-in "converged on $RC3_DIGEST" --expect-in "is the preprod candidate" \
    -- "$GB" release e2e-canary --version 0.1.0 --rc 3 --ack "e2e harness" --yes
  rec goatbuilder-abandoned-the-rc2-candidate --note "the CLI avenue of #2039: rc.3 reaching preprod closes rc.2's open record" \
    -- bash -c "cd '$CLONE' && git fetch -q origin && git merge -q --ff-only origin/main && grep -l 'state: abandoned' ops/release/records/e2e-canary/*.yaml | xargs grep -l 'v0.1.0-rc.2' | grep -q ."
  rec goatbuilder-preprod-ledger-names-rc3 -- bash -c "$(declare -f ledger_row); ledger_row e2e_preprod e2e-canary | grep -q '$RC3_DIGEST'"
  rec goatbuilder-release-rerun-reuses-the-preprod-record --expect-in "reusing it" \
    -- "$GB" release e2e-canary --tag e2e-canary/v0.1.0-rc.3 --from dev --ack "e2e harness" --yes
  rec goatbuilder-one-open-record-for-rc3 --expect-in "open records for the rc.3 digest: 1" \
    -- bash -c "cd '$CLONE' && git fetch -q origin && git merge -q --ff-only origin/main && echo \"open records for the rc.3 digest: \$(grep -l 'state: candidate' ops/release/records/e2e-canary/*-${RC3_DIGEST:7:12}.yaml 2>/dev/null | wc -l | tr -d ' ')\""
  rec goatbuilder-soak-elapses --note "the record opened one day earlier than it did" \
    -- bash -c "cd '$CLONE' && git fetch -q origin && git merge -q --ff-only origin/main && python3 - <<'PY'
import datetime, glob, yaml
for f in glob.glob('ops/release/records/e2e-canary/*.yaml'):
    r = yaml.safe_load(open(f))
    if r.get('state') == 'candidate' and r.get('class') != 'hotfix':
        s = open(f).read().replace(\"opened: '%s'\" % r['opened'], \"opened: '%s'\" % (datetime.date.fromisoformat(str(r['opened'])) - datetime.timedelta(days=1)).isoformat())
        open(f, 'w').write(s); print('dated back:', f)
PY
git add -A && git commit -q -m 'e2e: the soak elapses (the record opened a day earlier)' && git push -q origin main"
  rec goatbuilder-release-prod-graduate --expect-in "converged on $RC3_DIGEST" \
    -- "$GB" release e2e-canary --tag e2e-canary/v0.1.0-rc.3 --from prod --ack "e2e harness" --yes
  rec goatbuilder-every-rung-reads-rc3 -- bash -c "$(declare -f ledger_row); for g in e2e_dev e2e_preprod e2e_prod; do ledger_row \$g e2e-canary | grep -q '$RC3_DIGEST' || exit 1; done"
  rec goatbuilder-fastlane-everywhere --expect-in "sha-$HOT2" \
    -- "$GB" fastlane e2e-canary --ref sha-$HOT2 --reason "hermetic: the CLI avenue" --ack "e2e harness" --yes
  rec goatbuilder-every-rung-reads-hotfix2 -- bash -c "$(declare -f ledger_row); for g in e2e_dev e2e_preprod e2e_prod; do ledger_row \$g e2e-canary | grep -q '$HOT2_DIGEST' || exit 1; done"
  # survey reads every rung in parallel: each host's ledger row and its
  # containers, in one call per member group. With every rung on the same
  # digest, e2e-canary must read same on every host and carry no finding. (Other
  # components may: e2e-canary-secret is declared on every rung and deployed on
  # dev only, which survey reports. The container hosts share the runner's
  # docker daemon, so each sees every rung's containers: the case sits where
  # they all run one digest.)
  rec goatbuilder-survey-every-rung-consistent --expect-in "e2e-canary: same on 3 host(s)" \
    -- bash -c "'$GB' -o json survey --env e2e_dev --env e2e_preprod --env e2e_prod --intra | python3 -c '
import json, sys
d = json.load(sys.stdin)
rows = [r for g in d[\"goatnets\"] for r in g[\"rows\"] if r[\"component\"] == \"e2e-canary\"]
bad = [f[\"detail\"] for f in d[\"findings\"] if f.get(\"component\") == \"e2e-canary\"]
ok = rows and all(r[\"state\"] == \"same\" and \"$HOT2_DIGEST\" in r[\"declared\"] for r in rows) and not bad
print(\"e2e-canary: same on %d host(s)\" % len(rows) if ok else \"e2e-canary not consistent: %s %s\" % (rows, bad))
sys.exit(0 if ok else 1)'"
  rec goatbuilder-rollback-site-less-refused --expect-exit 1 --expect-in "GB-055" --expect-in "with no --site would flip every one" \
    -- "$GB" rollback e2e-canary 1 --yes
  rec goatbuilder-rollback-one-step -- "$GB" rollback e2e-canary 1 --site e2e-dev --yes
  rec goatbuilder-dev-ledger-names-the-rollback -- bash -c "$(declare -f ledger_row); ledger_row e2e_dev e2e-canary | grep -q '\"action\":\"rollback\"'"
fi

say "cases: the first-run page's commands, run as written"
rec doc-as-test-first-run-live --expect-in "first-run page:" \
  -- python3 "$HERE/doc-as-test.py" "$CLONE/docs/operations/goatbuilder-first-run.md" --section "A deploy, with its proof" \
     --map "<goatnet>=e2e_dev" --map "<component>=e2e-canary" --map "zagros_dev=e2e_dev" --map "netvis=e2e-canary" --map "goatbuilder=${GB:-$HERE/stubs/goatbuilder-absent}" --cwd "$CLONE"

# The teardown plays (ops/goatbuilder/teardown.yaml, goatbuilder-speed.md
# "Teardown"): a service stopped by name inside its compose project, then the
# project brought down, each proved from the host, a teardown.jsonl row written
# and the deploy ledger left as it was. Last, because the canary stops.
say "cases: the teardown plays against the dev canary"
dev_ip="$(awk '$0=="[e2e_dev_hosts]"{f=1;next} /^\[/{f=0} f && /ansible_host=/{sub(/.*ansible_host=/,""); sub(/[[:space:]].*/,""); print; exit}' "$CLONE/ops/ansible/inventory/mgmt-hosts.ini")"
canary_svc="$(ssh "goat@$dev_ip" "sudo sed -n 's/^ *container_name: *//p' /opt/e2e-canary/docker-compose.yml" | head -1)"
ledger_before="$(ledger_rows e2e_dev e2e-canary | tr -d ' ')"
rec teardown-service-stop-stops-the-named-service --expect-in "failed=0" \
  -- bash -c "cd '$CLONE/ops/ansible' && ansible-playbook -i inventory/mgmt-hosts.ini teardown-service-stop.yml -e target=e2e_dev_hosts -e teardown_component=e2e-canary -e teardown_project=/opt/e2e-canary -e 'teardown_services=[\"$canary_svc\"]'"
rec teardown-compose-down-proves-nothing-runs --expect-in "failed=0" \
  -- bash -c "cd '$CLONE/ops/ansible' && ansible-playbook -i inventory/mgmt-hosts.ini teardown-compose.yml -e target=e2e_dev_hosts -e teardown_component=e2e-canary -e 'teardown_dirs=[{\"var\":\"e2e_canary_dir\",\"default\":\"/opt/e2e-canary\"}]'"
rec teardown-records-two-rows --expect-in "teardown rows: 2" \
  -- bash -c "echo \"teardown rows: \$(ssh 'goat@$dev_ip' 'sudo wc -l < /var/lib/goat/e2e-canary/teardown.jsonl' | tr -d ' ')\""
rec teardown-leaves-the-deploy-ledger --expect-in "ledger rows: $ledger_before" \
  -- bash -c "echo \"ledger rows: \$(ssh 'goat@$dev_ip' 'sudo wc -l < /var/lib/goat/e2e-canary/deploy-history.jsonl' | tr -d ' ')\""

# The site-cache client (goatbuilder-speed.md sink 6): apt's proxy probe names
# the cache when it answers and DIRECT when it does not, so a cache that is down
# never stops an apt run. The host's own sshd stands in for an answering cache.
say "cases: the site-cache client falls back when the cache does not answer"
rec site-cache-client-applies \
  -- bash -c "cd '$CLONE/ops/ansible' && ansible e2e_dev_hosts -i inventory/mgmt-hosts.ini -b -m include_role -a name=site-cache-client -e site_cache_host=127.0.0.1 -e site_cache_apt_port=22"
rec site-cache-probe-names-an-answering-cache --expect-in "http://127.0.0.1:22" \
  -- ssh "goat@$dev_ip" /usr/local/bin/goat-apt-proxy-detect
rec site-cache-client-falls-back-direct --expect-in "DIRECT" \
  -- bash -c "cd '$CLONE/ops/ansible' && ansible e2e_dev_hosts -i inventory/mgmt-hosts.ini -b -m include_role -a name=site-cache-client -e site_cache_host=127.0.0.1 -e site_cache_apt_port=1 >/dev/null && ssh 'goat@$dev_ip' /usr/local/bin/goat-apt-proxy-detect"
rec site-cache-client-apt-reads-the-probe --expect-in "goat-apt-proxy-detect" \
  -- ssh "goat@$dev_ip" "apt-config dump | grep -i proxy-auto-detect"

record_ledgers
python3 "$HERE/report.py" "$RECORD" --check --title "deployment e2e (hermetic)" || true
echo "record: $RECORD"
[ "$FAILS" = 0 ] && exit 0 || exit 1
