# shellcheck shell=bash disable=SC2034 # read by ops/e2e/vm/run.sh
# A synthetic overlay's type (gate J26; docs/design/goatbuilder-build-targets.md
# section 3.10): its file is in ops/goatbuilder/overlay/targets/, and its play is
# in a synthetic repository at a full commit, which join-node fetches before any
# phase. Phase 13 runs that play on the machine, and the run reads back what it
# wrote. Then the kept copy is changed, and phase 13 refuses it (GB-420) before
# its play.
HOST_VARS="platform_family: host-class"
OVERLAY_SOURCE=""; OVERLAY_COMMIT=""

type_tree() {
  OVERLAY_SOURCE="$WORK/private/example-overlay"
  mkdir -p "$OVERLAY_SOURCE/deploy" "$OVERLAY_SOURCE/roles/example-overlay-marker/tasks"
  cat > "$OVERLAY_SOURCE/deploy/example-overlay.yml" <<'Y'
- name: the example overlay role
  hosts: "{{ target | mandatory }}"
  become: true
  roles: [example-overlay-marker]
Y
  cat > "$OVERLAY_SOURCE/roles/example-overlay-marker/tasks/main.yml" <<'Y'
- name: the file the run reads back
  ansible.builtin.copy:
    dest: /etc/goat-example-overlay
    content: "built by the example overlay type\n"
    mode: "0644"
Y
  git -C "$OVERLAY_SOURCE" init -q
  git -C "$OVERLAY_SOURCE" add -A
  git -C "$OVERLAY_SOURCE" -c user.name="vm harness" -c user.email=vm@localhost -c commit.gpgsign=false commit -q -m "a synthetic overlay's play"
  OVERLAY_COMMIT="$(git -C "$OVERLAY_SOURCE" rev-parse HEAD)"
  mkdir -p "$CLONE/ops/goatbuilder/overlay/targets"
  cat > "$CLONE/ops/goatbuilder/overlay/targets/example-overlay.yaml" <<Y
schema: goatbuilder-target/v1
type: example-overlay
families: [host-class]
source:
  repository: file://$OVERLAY_SOURCE
  commit: "$OVERLAY_COMMIT"
phases:
  - label: the example overlay role
    plays:
      - {play: deploy/example-overlay.yml, args: "-e target={group}", from: source}
Y
}

# The kept copy, changed after it was fetched: phase 13 refuses it before its play.
changed_copy_join() {
  printf '# changed here\n' >> "$HOME/.goat/state/goat-type-sources/example-overlay/$OVERLAY_COMMIT/deploy/example-overlay.yml"
  "${JOIN[@]}" --from 13 --to 13
}

type_cases() {
  step phase-13 0 "Phase 13" "example-overlay.yml" "!fatal:" -- "${JOIN[@]}" --from 13 --to 13
  step overlay-play-ran 0 "built by the example overlay type" -- vm_ssh cat /etc/goat-example-overlay
  step changed-copy-refused 1 "GB-420" "is not the commit's content" -- changed_copy_join
}
