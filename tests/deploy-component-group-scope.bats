#!/usr/bin/env bats
# Isolated target-scope tests for deploy-component.sh (#1573). Every operational
# command is stubbed; these tests cannot contact a goatnet or managed host.

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
SUBJECT="$REPO_ROOT/ops/scripts/deploy-component.sh"

setup() {
  command -v python3 >/dev/null || skip "python3 not installed"
  python3 -c 'import yaml' 2>/dev/null || skip "PyYAML not installed"

  FIXTURE="$BATS_TEST_TMPDIR/repo"
  BIN="$BATS_TEST_TMPDIR/bin"
  CALLS="$BATS_TEST_TMPDIR/operational-calls"
  mkdir -p "$FIXTURE/ops/scripts" "$FIXTURE/ops/pins" "$FIXTURE/ops/ansible" \
    "$FIXTURE/ops/ansible/group_vars/valid_hosts" \
    "$FIXTURE/ops/ansible/group_vars/absent_hosts" \
    "$FIXTURE/ops/ansible/group_vars/outside_hosts" "$BIN"
  mkdir -p "$FIXTURE/ops/release" && cp "$REPO_ROOT/ops/release/ladder_gate.py" "$FIXTURE/ops/release/"  # the release-ladder gate (GB-373); no ladder declared here, so it passes
  cp "$SUBJECT" "$FIXTURE/ops/scripts/deploy-component.sh"
  chmod +x "$FIXTURE/ops/scripts/deploy-component.sh"

  cat > "$FIXTURE/ops/pins/fleet-pins.yaml" <<'YAML'
goatnets:
  test_site:
    members: [valid_hosts, absent_hosts]
components:
  widget:
    var: widget_image
    fleet: example/widget@sha256:7e577e577e577e577e577e577e577e577e577e577e577e577e577e577e577e57
    deploy_playbook: widget-deploy.yml
    deviations:
      absent_hosts:
        absent: true
        reason: This component does not run there.
  site_absent:
    var: site_absent_image
    fleet: example/site-absent@sha256:7e577e577e577e577e577e577e577e577e577e577e577e577e577e577e577e57
    deploy_playbook: widget-deploy.yml
    deviations:
      test_site:
        absent: true
        reason: This component does not run on this goatnet.
  member_override:
    var: member_override_image
    fleet: example/member-override@sha256:7e577e577e577e577e577e577e577e577e577e577e577e577e577e577e577e57
    deploy_playbook: widget-deploy.yml
    deviations:
      test_site:
        absent: true
        reason: This component normally does not run on this goatnet.
      valid_hosts:
        value: example/member-override@sha256:0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e
        reason: This member is the declared exception.
  map_opt_in:
    var: map_opt_in_image
    opt_in: true
    fleet: example/map@sha256:7e577e577e577e577e577e577e577e577e577e577e577e577e577e577e577e57
    deploy_playbook: widget-deploy.yml
  release_tool:
    var: release_tool_file
    artifact_type: release
    fleet: example/release-tool@v1.0.0@sha256:cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd
    deploy_playbook: widget-deploy.yml
YAML
  printf '%s\n' \
    'widget_image: "example/widget@sha256:7e577e577e577e577e577e577e577e577e577e577e577e577e577e577e577e57"' \
    'release_tool_file: "example/release-tool@v1.0.0@sha256:cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd"' \
    'member_override_image: "example/member-override@sha256:0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e"' \
    > "$FIXTURE/ops/ansible/group_vars/valid_hosts/pins.yml"
  printf '%s\n' 'widget_image: "example/widget@sha256:7e577e577e577e577e577e577e577e577e577e577e577e577e577e577e577e57"' \
    > "$FIXTURE/ops/ansible/group_vars/outside_hosts/pins.yml"
  printf '%s\n' 'hosts: "{{ target }}"' > "$FIXTURE/ops/ansible/widget-deploy.yml"

  cat > "$BIN/netbird" <<'SH'
#!/usr/bin/env bash
printf 'netbird %s\n' "$*" >> "$OPERATIONAL_CALLS"
printf 'Profile: somewhere-else\nManagement: Disconnected\n'
SH
  chmod +x "$BIN/netbird"
  cat > "$FIXTURE/ops/scripts/env-door.sh" <<'SH'
#!/usr/bin/env bash
printf 'env-door %s\n' "$*" >> "$OPERATIONAL_CALLS"
exit 1
SH
  chmod +x "$FIXTURE/ops/scripts/env-door.sh"
  cat > "$BIN/aws-vault" <<'SH'
#!/usr/bin/env bash
printf 'aws-vault %s\n' "$*" >> "$OPERATIONAL_CALLS"
exit 99
SH
  chmod +x "$BIN/aws-vault"
  # A dry run runs the fresh-source check too (it touches no host); here it passes.
  # The intent check (GB-383) is tested by crates/goatbuilder/tests/deploy_dry_run_refusals.rs.
  printf 'print("ok: signature valid")\n' > "$FIXTURE/ops/pins/generate-pins.py"
  printf '#!/usr/bin/env bash\nexit 0\n' > "$FIXTURE/ops/scripts/fresh-source.sh"
  chmod +x "$FIXTURE/ops/scripts/fresh-source.sh"
}

run_deploy() {
  cd "$FIXTURE" || return
  run env PATH="$BIN:$PATH" OPERATIONAL_CALLS="$CALLS" \
    "${BASH_UNDER_TEST:-/bin/bash}" ./ops/scripts/deploy-component.sh "$@"
}

assert_no_operational_calls() {
  [ ! -s "$CALLS" ]
}

@test "deploy-component refuses a release component before operations when nothing here can verify it (GB-381)" {
  export GOATBUILDER_BIN="$BATS_TEST_TMPDIR/no-goatbuilder"
  # A current checkout: the release is checked after fresh-source.
  printf '#!/usr/bin/env bash\necho 1111111111111111111111111111111111111111\n' > "$FIXTURE/ops/scripts/fresh-source.sh"; chmod +x "$FIXTURE/ops/scripts/fresh-source.sh"
  run_deploy release_tool --goatnet test_site --group valid_hosts

  [ "$status" -eq 1 ]
  [[ "$output" == *"GB-381 refused before any host: release_tool's release example/release-tool@v1.0.0@sha256:cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd: $GOATBUILDER_BIN is not here to fetch and verify it"* ]]
  assert_no_operational_calls

  # An older goatbuilder, with no release-file verb.
  printf '#!/usr/bin/env bash\necho "error: unrecognized subcommand" >&2\nexit 2\n' > "$GOATBUILDER_BIN"; chmod +x "$GOATBUILDER_BIN"
  run_deploy release_tool --goatnet test_site --group valid_hosts
  [ "$status" -eq 1 ]
  [[ "$output" == *"a newer goatbuilder is needed (check: goatbuilder)"* ]]
  assert_no_operational_calls
}

@test "deploy-component --dry-run of a release component that cannot be verified prints the plan and exits 1" {
  export GOATBUILDER_BIN="$BATS_TEST_TMPDIR/no-goatbuilder"
  run_deploy release_tool --goatnet test_site --group valid_hosts --dry-run

  [ "$status" -eq 1 ]
  [[ "$output" == *"GB-381 refused before any host"* ]]
  [[ "$output" == *"DRY: would read the file release_tool runs on each host of valid_hosts"* ]]
  [[ "$output" == *"DRY: a real run would refuse before any host (release); nothing was read"* ]]
}

@test "deploy-component's release modes are for a release component" {
  run_deploy widget --goatnet test_site --release-rollback

  [ "$status" -eq 1 ]
  [[ "$output" == *"widget is not a release component"* ]]
  assert_no_operational_calls
}

@test "deploy-component accepts an eligible member selected with --group" {
  run_deploy widget --goatnet test_site --group valid_hosts --dry-run

  [ "$status" -eq 0 ]
  [[ "$output" == *"groups [valid_hosts]"* ]]
  [[ "$output" == *"valid_hosts: deploying example/widget@sha256:7e577e577e577e577e577e577e577e577e577e577e577e577e577e577e577e57"* ]]
  [[ "$output" != *"outside_hosts"* ]]
}

@test "deploy-component rejects a group from another goatnet before operations" {
  run_deploy widget --goatnet test_site --group outside_hosts --dry-run

  [ "$status" -eq 1 ]
  [[ "$output" == *"--group 'outside_hosts' is not a member of goatnet 'test_site'"* ]]
  assert_no_operational_calls
}

@test "deploy-component rejects an Ansible pattern before operations" {
  run_deploy widget --goatnet test_site --group 'valid_hosts:&outside_hosts' --dry-run

  [ "$status" -eq 1 ]
  [[ "$output" == *"--group 'valid_hosts:&outside_hosts' is not a member of goatnet 'test_site'"* ]]
  assert_no_operational_calls
}

@test "deploy-component rejects a member where the component is absent before operations" {
  run_deploy widget --goatnet test_site --group absent_hosts --dry-run

  [ "$status" -eq 1 ]
  [[ "$output" == *"--group 'absent_hosts' does not run component 'widget' (absent: true)"* ]]
  assert_no_operational_calls
}

@test "deploy-component rejects effective goatnet-level component absence before operations" {
  run_deploy site_absent --goatnet test_site --group valid_hosts --dry-run

  [ "$status" -eq 1 ]
  [[ "$output" == *"--group 'valid_hosts' does not run component 'site_absent' (absent: true)"* ]]
  assert_no_operational_calls
}

@test "deploy-component honors a member value over goatnet-level absence" {
  run_deploy member_override --goatnet test_site --group valid_hosts --dry-run

  [ "$status" -eq 0 ]
  [[ "$output" == *"groups [valid_hosts]"* ]]
  [[ "$output" == *"valid_hosts: deploying example/member-override@sha256:0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e"* ]]
}

@test "deploy-component rejects an opt-in component on a goatnet that did not opt in, before operations" {
  for g in "--group valid_hosts" ""; do
    # shellcheck disable=SC2086  # $g is zero or two words on purpose
    run_deploy map_opt_in --goatnet test_site $g --dry-run

    [ "$status" -eq 1 ]
    [[ "$output" == *"goatnet 'test_site' does not run component 'map_opt_in': it is opt-in, and the goatnet does not list it under opt_in:"* ]]
    assert_no_operational_calls
  done
}

@test "deploy-component deploys an opt-in component on a goatnet that lists it" {
  python3 - "$FIXTURE/ops/pins/fleet-pins.yaml" <<'PY'
import sys, yaml
p = sys.argv[1]; t = yaml.safe_load(open(p))
t["goatnets"]["test_site"]["opt_in"] = ["map_opt_in"]
yaml.safe_dump(t, open(p, "w"))
PY
  printf '%s\n' 'map_opt_in_image: "example/map@sha256:7e577e577e577e577e577e577e577e577e577e577e577e577e577e577e577e57"' \
    >> "$FIXTURE/ops/ansible/group_vars/valid_hosts/pins.yml"
  run_deploy map_opt_in --goatnet test_site --group valid_hosts --dry-run

  [ "$status" -eq 0 ]
  [[ "$output" == *"valid_hosts: deploying example/map@sha256:7e577e577e577e577e577e577e577e577e577e577e577e577e577e577e577e57"* ]]
}

@test "deploy-component rejects an unknown component before operations" {
  run_deploy missing --goatnet test_site --group valid_hosts --dry-run

  [ "$status" -eq 1 ]
  [[ "$output" == *"unknown component 'missing'"* ]]
  assert_no_operational_calls
}

@test "deploy-component preserves whole-goatnet selection and absent filtering" {
  run_deploy widget --goatnet test_site --dry-run

  [ "$status" -eq 0 ]
  [[ "$output" == *"skipping (absent: true, topology not skew): absent_hosts"* ]]
  [[ "$output" == *"groups [valid_hosts]"* ]]
  [[ "$output" != *"outside_hosts"* ]]
}

# The transport decision (#2007): the mesh is taken when the goatnet's mesh
# inventory host answers on tcp/22, whatever the netbird profile is called.
mesh_fixture() {
  mkdir -p "$FIXTURE/ops/ansible/inventory"
  printf '%s\n' '[valid_hosts]' 'mgmt-test-01 ansible_host=100.127.0.60 ansible_user=u' \
    > "$FIXTURE/ops/ansible/inventory/mgmt-hosts-mesh.ini"
  cat > "$BIN/nc" <<SH
#!/usr/bin/env bash
printf 'nc %s\n' "\$*" >> "\$OPERATIONAL_CALLS"
exit $1
SH
  chmod +x "$BIN/nc"
}

@test "deploy-component takes the mesh when its host answers on 22, under any profile name" {
  mesh_fixture 0
  run_deploy widget --goatnet test_site --dry-run

  [ "$status" -eq 0 ]
  [[ "$output" == *"via mesh (this laptop reaches 100.127.0.60:22; netbird profile reads 'somewhere-else')"* ]]
  [[ "$output" == *"mgmt-hosts-mesh.ini"* ]]
}

@test "deploy-component falls past the mesh when its host does not answer" {
  mesh_fixture 1
  run_deploy widget --goatnet test_site --dry-run

  [ "$status" -eq 0 ]
  [[ "$output" == *"via direct (mgmt-hosts.ini)"* ]]
  [[ "$output" != *"mgmt-hosts-mesh.ini"* ]]
}

# The secrets preflight: every variable the play templates from the environment
# is named before any host is touched; a dry run notes it,
# prints the whole plan and exits 1; a real run refuses.
secret_fixture() {
  printf '%s\n' 'widget_secret: "{{ lookup('"'"'env'"'"', '"'"'WIDGET_SECRET'"'"') }}"' \
    > "$FIXTURE/ops/ansible/group_vars/valid_hosts/vars.yml"
  printf '%s\n' 'hosts: "{{ target }}"' 'tasks:' '  - debug: var=widget_secret' > "$FIXTURE/ops/ansible/widget-deploy.yml"
  cp "$REPO_ROOT/ops/scripts/deploy-secrets-needed.py" "$FIXTURE/ops/scripts/"
  # How to supply a missing name is the declared secrets posture's (ADR 1204).
  mkdir -p "$FIXTURE/ops/secrets"
  cp "$REPO_ROOT/ops/secrets/posture.py" "$FIXTURE/ops/secrets/"
}

@test "deploy-component names every missing secret before any host is touched" {
  secret_fixture
  # The fixture declares nothing, so it gets the store that needs no account
  # (ADR 1204): the names are still all reported before any host is touched.
  run_deploy widget --goatnet test_site --group valid_hosts --dry-run

  [ "$status" -eq 1 ]
  [[ "$output" == *"DRY: a real run would refuse before any host (secrets)"* ]]
  [[ "$output" == *"DRY: would read /var/lib/goat/widget/deploy-history.jsonl on valid_hosts"* ]]
  [[ "$output" == *"NOT SET: WIDGET_SECRET"* ]]
  [[ "$output" != *"op signin"* ]]
}

@test "deploy-component on the 1Password posture names the sign-in and the wrapper" {
  secret_fixture
  # goat's own posture: references, signed in to 1Password, run through
  # with-secrets. Declared here, since the undeclared default is an env file.
  export GOAT_SECRET_STORE=1password
  run_deploy widget --goatnet test_site --group valid_hosts --dry-run
  unset GOAT_SECRET_STORE

  [ "$status" -eq 1 ]
  [[ "$output" == *"DRY: a real run would refuse before any host (secrets)"* ]]
  [[ "$output" == *"DRY: would read /var/lib/goat/widget/deploy-history.jsonl on valid_hosts"* ]]
  [[ "$output" == *"NOT SET: WIDGET_SECRET"* ]]
  [[ "$output" == *"sign in: eval \$(op signin)"* ]]
  [[ "$output" == *"ops/scripts/with-secrets"* ]]
}

@test "deploy-component's secrets hint follows the declared posture" {
  secret_fixture
  export GOAT_SECRET_STORE=env-file
  run_deploy widget --goatnet test_site --group valid_hosts --dry-run
  unset GOAT_SECRET_STORE

  [ "$status" -eq 1 ]
  [[ "$output" == *"DRY: a real run would refuse before any host (secrets)"* ]]
  [[ "$output" == *"DRY: would read /var/lib/goat/widget/deploy-history.jsonl on valid_hosts"* ]]
  [[ "$output" == *"NOT SET: WIDGET_SECRET"* ]]
  [[ "$output" == *"goatbuilder secrets init --env"* ]]
}

@test "deploy-component refuses a real run with a secret unset, and proceeds when it is set" {
  secret_fixture
  # A real run first proves the checkout current with the trunk (contract C3,
  # fresh-source.sh); the fixture is its own trunk, so this tests the secrets gate.
  cp "$REPO_ROOT/ops/scripts/fresh-source.sh" "$FIXTURE/ops/scripts/"
  git -C "$FIXTURE" init -q
  git -C "$FIXTURE" add -A
  git -C "$FIXTURE" -c user.name=t -c user.email=t@t commit -q -m fixture
  cd "$FIXTURE" || return
  run env PATH="$BIN:$PATH" OPERATIONAL_CALLS="$CALLS" GOAT_FRESH_SOURCE_TRUNK="$(git -C "$FIXTURE" rev-parse HEAD)" \
    /bin/bash ./ops/scripts/deploy-component.sh widget --goatnet test_site --group valid_hosts
  [ "$status" -eq 1 ]
  [[ "$output" == *"refusing before any host is touched: widget needs WIDGET_SECRET"* ]]
  assert_no_operational_calls

  run env PATH="$BIN:$PATH" OPERATIONAL_CALLS="$CALLS" WIDGET_SECRET=x GOAT_FRESH_SOURCE_TRUNK="$(git -C "$FIXTURE" rev-parse HEAD)" /bin/bash ./ops/scripts/deploy-component.sh widget --goatnet test_site --group valid_hosts --dry-run
  [ "$status" -eq 0 ]
  [[ "$output" != *"NOT SET"* ]]
}

# A play with LITERAL hosts takes `-l <group>`; it must not end the run silently
# (the grep that extracts a hosts variable finds nothing and returns 1).
@test "deploy-component targets a literal-hosts play with -l and does not die silently" {
  printf '%s\n' 'hosts: valid_hosts' 'tasks: []' > "$FIXTURE/ops/ansible/widget-deploy.yml"
  run_deploy widget --goatnet test_site --group valid_hosts --dry-run

  [ "$status" -eq 0 ]
  [[ "$output" == *"playbook widget-deploy.yml -l <group>"* ]]
  [[ "$output" == *"valid_hosts: deploying example/widget@sha256:7e577e577e577e577e577e577e577e577e577e577e577e577e577e577e577e57"* ]]
}

# #1574: the inventory lookup and ledger tail are read-only and need no AWS
# account on a direct/mesh (SSH) goatnet; door/SSM transport still needs it.
# A single-homed component (`single_homed_role`) is used so both read-only
# with-aws calls run (the inventory lookup only runs when a write role is
# declared).
readonly_verify_fixture() {
  mkdir -p "$FIXTURE/ops/ansible/group_vars/edge_hosts" "$FIXTURE/ops/ansible/inventory"
  cat > "$FIXTURE/ops/pins/fleet-pins.yaml" <<'YAML'
goatnets:
  verify_site:
    members: [edge_hosts]
components:
  probe:
    var: probe_image
    deploy_playbook: probe-deploy.yml
    single_homed_role: edge
YAML
  printf '%s\n' 'probe_image: "example/probe@sha256:abc1230000000000000000000000000000000000000000000000000000000000"' \
    > "$FIXTURE/ops/ansible/group_vars/edge_hosts/pins.yml"
  printf '%s\n' 'hosts: "{{ target }}"' > "$FIXTURE/ops/ansible/probe-deploy.yml"
  cp "$REPO_ROOT/ops/scripts/served-payloads.py" "$FIXTURE/ops/scripts/"

  # The synthetic "no AWS credentials" probe: with-aws refuses outright unless
  # told GOAT_AWS_MODE=none, the one case the real with-aws (ops/scripts/with-aws)
  # execs the command with no AWS touched at all. Logs the mode it saw so a
  # test can tell apart "AWS was bypassed" from "AWS was still wrapped".
  cat > "$FIXTURE/ops/scripts/with-aws" <<'SH'
#!/usr/bin/env bash
printf 'with-aws mode=%s %s\n' "${GOAT_AWS_MODE:-unset}" "$*" >> "$OPERATIONAL_CALLS"
shift 2
if [ "${GOAT_AWS_MODE:-}" = "none" ]; then
  exec "$@"
fi
echo "with-aws: stub refuses -- no AWS credentials available (GOAT_AWS_MODE=${GOAT_AWS_MODE:-unset})" >&2
exit 97
SH
  chmod +x "$FIXTURE/ops/scripts/with-aws"

  cat > "$BIN/ansible-inventory" <<'SH'
#!/usr/bin/env bash
printf '{"edge_hosts": {"hosts": ["edge-1"]}, "_meta": {"hostvars": {"edge-1": {"router_role": "edge"}}}}\n'
SH
  chmod +x "$BIN/ansible-inventory"

  cat > "$BIN/ansible" <<'SH'
#!/usr/bin/env bash
printf '%s\n' 'edge-1 | CHANGED | rc=0 >> {"ref": "example/probe@sha256:abc1230000000000000000000000000000000000000000000000000000000000", "action": "deploy", "ts": "2026-01-01T00:00:00Z"}'
SH
  chmod +x "$BIN/ansible"
}

mesh_fixture_edge() {
  printf '%s\n' '[edge_hosts]' 'mgmt-edge-01 ansible_host=100.127.0.60 ansible_user=u' \
    > "$FIXTURE/ops/ansible/inventory/mgmt-hosts-mesh.ini"
  cat > "$BIN/nc" <<'SH'
#!/usr/bin/env bash
exit 0
SH
  chmod +x "$BIN/nc"
}

@test "deploy-component's read-only ledger verification works over direct SSH with no AWS available" {
  readonly_verify_fixture
  run_deploy probe --goatnet verify_site --group edge_hosts --verify-only

  [ "$status" -eq 0 ]
  [[ "$output" == *"via direct (mgmt-hosts.ini)"* ]]
  [[ "$output" == *"edge-1: converged on sha256:abc123"* ]]
  grep -q 'with-aws mode=none .*ansible-inventory' "$CALLS"
  grep -q 'with-aws mode=none .*ansible ' "$CALLS"
}

@test "deploy-component's read-only ledger verification works over the mesh with no AWS available" {
  readonly_verify_fixture
  mesh_fixture_edge
  run_deploy probe --goatnet verify_site --group edge_hosts --verify-only

  [ "$status" -eq 0 ]
  [[ "$output" == *"via mesh (this laptop reaches 100.127.0.60:22"* ]]
  [[ "$output" == *"edge-1: converged on sha256:abc123"* ]]
  grep -q 'with-aws mode=none .*ansible-inventory' "$CALLS"
  grep -q 'with-aws mode=none .*ansible ' "$CALLS"
}

# Mixed inventory: an SSM/door-reachable goatnet is NOT given the AWS bypass,
# only direct/mesh transport is. When with-aws itself fails, the inventory read
# fails and verification refuses, rather than reading as "nothing to report" (#1545).
@test "deploy-component keeps the AWS wrapper for a door-transport (SSM-style) goatnet" {
  readonly_verify_fixture
  cat > "$FIXTURE/ops/scripts/env-door.sh" <<'SH'
#!/usr/bin/env bash
exit 0
SH
  chmod +x "$FIXTURE/ops/scripts/env-door.sh"
  cat > "$FIXTURE/ops/scripts/door-open.sh" <<'SH'
#!/usr/bin/env bash
exit 0
SH
  chmod +x "$FIXTURE/ops/scripts/door-open.sh"
  run_deploy probe --goatnet verify_site --group edge_hosts --verify-only

  [ "$status" -ne 0 ]
  [[ "$output" == *"via door"* ]]
  grep -q 'with-aws mode=unset .*ansible-inventory' "$CALLS"
  [[ "$output" == *"edge_hosts: inventory read FAILED"* ]]
  [[ "$output" != *"edge-1: converged"* ]]
}
