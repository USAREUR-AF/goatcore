# shellcheck shell=bash disable=SC2034 # read by ops/e2e/vm/run.sh
# network-jump (ADR 1131): the network-gear jump box. Its play and role are
# goatstuff's, brought into the tree at a pinned commit as an assembled tree has
# them. herd-connector has no public image, so a public image that only stays
# running (the pause image, pinned by its index digest) stands in for it, and
# stand-in files stand in for the SPIFFE material the role requires: the run
# proves the play and the rows on a real machine, not the connector.
GOATSTUFF_REPO="${GOATSTUFF_REPO:-https://github.com/usareur-af/goatstuff}"
GOATSTUFF_REF=f878cab5cb20d541d1c13baf462ee632980fb1e4
STAND_IN_IMAGE="registry.k8s.io/pause:3.10@sha256:ee6521f290b2168b6e0935a181d4cff9be1ac3f505666ef0e3c98fae8199917a"
# A small computer on arm64; on amd64 the family a Thor is, host-class.
if [ "$VM_ARCH" = arm64 ]; then HOST_VARS="platform_family: small-computer"; else HOST_VARS="platform_family: host-class"; fi
GROUP_VARS="herd_connector_enabled: true
herd_connector_image: \"$STAND_IN_IMAGE\"
herd_connector_site: zagros
herd_connector_console_ws_url: wss://herd.zagros.example.org/ws/gear-connector"
ROWS=network-jump-accepts-only-ssh,network-jump-connector-runs
ROW_VARS=(network_jump_host="$NODE" has_network_jump_node=true)

type_tree() {
  local clone="$1" src="$WORK/goatstuff"
  git init -q "$src"
  git -C "$src" fetch -q --depth 1 "$GOATSTUFF_REPO" "$GOATSTUFF_REF"
  git -C "$src" -c advice.detachedHead=false checkout -q FETCH_HEAD
  [ "$(git -C "$src" rev-parse HEAD)" = "$GOATSTUFF_REF" ] || return 1
  cp "$src/ops/ansible/herd-connector-deploy.yml" "$clone/ops/ansible/"
  cp -R "$src/ops/ansible/roles/herd-connector" "$clone/ops/ansible/roles/"
}

spiffe_stand_in() {
  vm_ssh "sudo sh -eu" <<'VM'
mkdir -p /run/spiffe
for f in cert.pem key.pem spire-bundle.pem; do printf 'stand-in, not a certificate\n' > /run/spiffe/$f; done
chmod 600 /run/spiffe/key.pem
VM
}

type_cases() {
  step spiffe-stand-in 0 -- spiffe_stand_in
  step phase-13 0 "Phase 13" "!fatal:" -- "${JOIN[@]}" --from 13 --to 13
  step acceptance 0 "PASS  network-jump-accepts-only-ssh" "PASS  network-jump-connector-runs" -- acceptance "$ROWS" "${ROW_VARS[@]}"
  # Each rule broken by hand, on the machine: the row that asks it must fail.
  step listener-caught 1 "FAIL  network-jump-accepts-only-ssh" "PASS  network-jump-connector-runs" -- \
    rows_with "nohup python3 -m http.server 8080 >/dev/null 2>&1 & sleep 2" "pkill -f \"http.server 8080\""
  step connector-stopped-caught 1 "FAIL  network-jump-connector-runs" "PASS  network-jump-accepts-only-ssh" -- \
    rows_with "docker stop herd-connector >/dev/null" "docker start herd-connector >/dev/null"
  step doctor-live 0 "network-jump at $SITE" -- "${DOCTOR[@]}" --live
  not_run "network-jump-node-is-on-the-mesh: it reads the mesh, which needs phase 10"
  not_run "the connector itself: herd-connector has no public image; the pause image stands in, so its registration with the console is not run"
}

# rows_with <how to break it> <how to put it back>: the rows, with one rule broken by hand.
rows_with() {
  local rc=0
  vm_ssh "sudo sh -c '$1'"
  acceptance "$ROWS" "${ROW_VARS[@]}" || rc=$?
  vm_ssh "sudo sh -c '$2'" || true
  return "$rc"
}
