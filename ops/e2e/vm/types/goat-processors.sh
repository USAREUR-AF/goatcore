# shellcheck shell=bash disable=SC2034 # read by ops/e2e/vm/run.sh
# goat-processors (design section 4.4): a host-class machine for the engines. Phase 13
# installs the runner from the public row of this checkout's pins (goat-processors-runner,
# opt-in, which this goatnet lists), verified against its Sigstore bundle on the
# operator's machine, by the runner's own play. Its fabric router is eclipse/zenoh in a
# container on this machine, which the VM reaches at QEMU's host address.
#
# The acceptance, after the play's own (self test, doctor, one replay): the conformance
# vectors the package carries are byte for byte the repository's at the release's own
# commit, read here from that commit, and the installed runner reproduces every one.
HOST_VARS="platform_family: host-class"
PUBLIC="goat-processors-runner"
OPT_IN="goat-processors-runner"
GP_DEB_amd64="goat-processors_0.8.1_amd64.deb"
GP_DEB_arm64="goat-processors_0.8.1_arm64.deb"
GP_DEB="$(eval echo "\$GP_DEB_$VM_ARCH")"
# runner-v0.8.1 is this commit of the engines' repository.
GP_COMMIT=4a7ed49b6873b31de0145ebbaac421a24bd91b56
GP_ZENOH_IMAGE="eclipse/zenoh@sha256:157965d71e0bfd0a044d76a985ff0e5c306ad3968929168fb9678cd2a7fec23f"
GP_ZENOH_PORT="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')"
GP_ZENOH="goat-vm-zenoh-$$"
GROUP_VARS="goat_processors_runner_config:
  name: vm-runner
  site: zagros
  tick_ms: 500
  fabric:
    mode: client
    connect: [\"tcp/10.0.2.2:$GP_ZENOH_PORT\"]
  inputs:
    udp:
      - listen: 127.0.0.1:7600
  pipeline:
    - {function: correlate, pack: correlate-assign-time}
    - {function: quality, pack: quality-feed}
  outputs:
    fabric: {peer: zagros, trace: false}"

join_13() {
  local rc=0 f
  "${JOIN[@]}" --from 13 --to 13 || rc=$?
  for f in "$(find "$HOME/.goat/state/runs" -maxdepth 1 -mindepth 1 | sort | tail -1)"/logs/*-13.log; do cat "$f"; done
  return "$rc"
}

gp_router() { docker run -d --name "$GP_ZENOH" -p "127.0.0.1:$GP_ZENOH_PORT:7447" "$GP_ZENOH_IMAGE"; }
gp_router_gone() { docker rm -fv "$GP_ZENOH"; }

# The repository's vectors at the release's commit, and their SHA-256 on both sides.
gp_vectors_same() {
  local d="$WORK/gp-src"
  mkdir -p "$d"
  curl -fsSL --retry 3 "https://codeload.github.com/USAREUR-AF/goat-processors/tar.gz/$GP_COMMIT" | tar -xz -C "$d" --strip-components=1 --wildcards '*/packs/*/vectors/*.json'
  (cd "$d" && find packs -path '*/vectors/*.json' -type f | LC_ALL=C sort | xargs sha256sum) > "$WORK/gp-repo.sha256"
  echo "repository vectors: $(wc -l < "$WORK/gp-repo.sha256")"
  vm_ssh 'cd /usr/share/goat-processors && find packs -path "*/vectors/*.json" -type f | LC_ALL=C sort | xargs sha256sum' > "$WORK/gp-host.sha256"
  echo "installed vectors: $(wc -l < "$WORK/gp-host.sha256")"
  [ -s "$WORK/gp-repo.sha256" ] && diff "$WORK/gp-repo.sha256" "$WORK/gp-host.sha256" && echo "the same vectors"
}

# The installed runner replays every vector and matches each.
gp_selftest() {
  vm_ssh 'goat-processors selftest -o json' | tee "$WORK/gp-selftest.json"
  python3 -c 'import json,sys; r=json.load(open(sys.argv[1])); n=int(sys.argv[2]); ok=r["failed"]==[] and r["vectors"]==n and r["passed"]==n; print("selftest: %s of %s vectors, failed %s, the repository has %s" % (r["passed"], r["vectors"], r["failed"], n)); sys.exit(0 if ok else 1)' "$WORK/gp-selftest.json" "$(wc -l < "$WORK/gp-repo.sha256")"
}

type_cases() {
  step router 0 -- gp_router
  step phase-13 0 "release verified: $GP_DEB" "goat-processors-runner-deploy.yml" "goat-processors runner | 8 acceptance replay" "failed=0" "!fatal:" -- join_13
  step running 0 "active" "0.8.1" -- vm_ssh 'systemctl is-active goat-processors; goat-processors version'
  step conformance-vectors 0 "the same vectors" -- gp_vectors_same
  step conformance-replay 0 "failed []" -- gp_selftest
  step phase-13-again 0 "failed=0" "!fatal:" -- join_13
  step router-gone 0 -- gp_router_gone
}
