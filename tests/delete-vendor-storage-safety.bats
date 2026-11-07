#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
  FIXTURE="$BATS_TEST_TMPDIR/vendor fixture"
  mkdir -p "$FIXTURE/bin" "$FIXTURE/storage/bin"
  FIXTURE="$(cd "$FIXTURE" && pwd -P)"
  printf 'seed\n' > "$FIXTURE/seed"
  printf 'unchanged\n' > "$FIXTURE/org"
  printf 'unchanged\n' > "$FIXTURE/live"
  export COMMAND_LOG="$FIXTURE/commands"
  : > "$COMMAND_LOG"
  # Never call Docker or remove any directory in this test. Capture the actual
  # argv so the original unsafe /bin deletion can be reproduced harmlessly.
  cat > "$FIXTURE/bin/docker" <<'SH'
#!/usr/bin/env bash
printf 'docker\n' >> "$COMMAND_LOG"
SH
  cat > "$FIXTURE/bin/rm" <<'SH'
#!/usr/bin/env bash
printf 'rm\n' >> "$COMMAND_LOG"
printf '<%s>\n' "$@" >> "$COMMAND_LOG"
SH
  cat > "$FIXTURE/bin/assemble" <<'SH'
#!/usr/bin/env bash
printf 'assemble\n' >> "$COMMAND_LOG"
SH
  chmod +x "$FIXTURE/bin/"*
  export PATH="$FIXTURE/bin:$PATH"
  ln -s / "$FIXTURE/root-alias"
}

# The roles that carry delete-vendor.sh in this tree (a goatnet's own role may
# not be present); goat-zenoh's always must.
vendor_roles() {
  local role
  for role in goat-zenoh nova-lan-zenoh; do
    [ -f "$REPO_ROOT/ops/ansible/roles/$role/files/delete-vendor.sh" ] && echo "$role"
  done
  return 0
}

invoke_delete() {
  local role="$1" parent="$2"
  shift 2
  if [[ "$role" == goat-zenoh ]]; then
    run "${BASH_UNDER_TEST:-/bin/bash}" "$REPO_ROOT/ops/ansible/roles/$role/files/delete-vendor.sh" \
      --vendor bin --storage-parent "$parent" --seed "$FIXTURE/seed" \
      --org "$FIXTURE/org" --config "$FIXTURE/live" --assembler "$FIXTURE/bin/assemble" "$@"
  else
    run "${BASH_UNDER_TEST:-/bin/bash}" "$REPO_ROOT/ops/ansible/roles/$role/files/delete-vendor.sh" \
      --vendor bin --storage-parent "$parent" --config "$FIXTURE/org" "$@"
  fi
}

@test "vendor deletion refuses empty and root parents before any side effect" {
  [ -f "$REPO_ROOT/ops/ansible/roles/goat-zenoh/files/delete-vendor.sh" ]
  for role in $(vendor_roles); do
    for parent in "" / // /../ "$FIXTURE/root-alias"; do
      invoke_delete "$role" "$parent"
      [ "$status" -eq 4 ]
      [[ "$output" == *'"action":"validate","ok":false'* ]]
      [ ! -s "$COMMAND_LOG" ]
      [ "$(cat "$FIXTURE/org")" = unchanged ]
      [ "$(cat "$FIXTURE/live")" = unchanged ]
    done
  done
}

@test "vendor deletion keeps cleanup scoped to a valid parent with spaces" {
  [ -f "$REPO_ROOT/ops/ansible/roles/goat-zenoh/files/delete-vendor.sh" ]
  for role in $(vendor_roles); do
    : > "$COMMAND_LOG"
    invoke_delete "$role" "$FIXTURE/storage"
    [ "$status" -eq 0 ]
    grep -qx 'docker' "$COMMAND_LOG"
    grep -qx '<-rf>' "$COMMAND_LOG"
    grep -qx '<-->' "$COMMAND_LOG"
    grep -qx "<$FIXTURE/storage/bin>" "$COMMAND_LOG"
  done
}

@test "vendor deletion accepts a relative storage parent and canonicalizes it" {
  cd "$FIXTURE"
  [ -f "$REPO_ROOT/ops/ansible/roles/goat-zenoh/files/delete-vendor.sh" ]
  for role in $(vendor_roles); do
    : > "$COMMAND_LOG"
    invoke_delete "$role" storage
    [ "$status" -eq 0 ]
    grep -qx "<$FIXTURE/storage/bin>" "$COMMAND_LOG"
  done
}

@test "vendor deletion cleans the same directory after editing existing stanzas" {
  [ -f "$REPO_ROOT/ops/ansible/roles/goat-zenoh/files/delete-vendor.sh" ]
  for role in $(vendor_roles); do
    printf '      {\n        id: "trial-peer-w-bin",\n        key_exprs: ["bin/**"],\n      },\n' > "$FIXTURE/org"
    : > "$COMMAND_LOG"
    invoke_delete "$role" "$FIXTURE/storage"
    [ "$status" -eq 0 ]
    ! grep -qF 'trial-peer-w-bin' "$FIXTURE/org"
    grep -qx "<$FIXTURE/storage/bin>" "$COMMAND_LOG"
    grep -qx 'docker' "$COMMAND_LOG"
  done
}
