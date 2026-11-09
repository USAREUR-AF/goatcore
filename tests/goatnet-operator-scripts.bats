#!/usr/bin/env bats
# Behavior tests for the operator scripts that act on one goatnet and read its
# facts from the goatnet's own configuration (ops/scripts/goatnet-facts.py):
# peer-dormancy-report.sh, rosenpass-remediate.sh, rosenpass-revoke-and-reinvite.sh
# and ops/e2e/live-canary.sh.
#
# The tests pin:
#   - goatnet-facts.py reads the management host from doors.yaml (the landed
#     entry of the goatnet's gv_group, else router_role A), the management URL
#     from public_mgmt_fqdn, a group_vars list, and the dev rung of the ladder,
#     and exits 1 with nothing on stdout for a fact that is not declared,
#   - each script refuses without a goatnet and names no deployment of its own,
#   - the dormancy report classes a person's device by the goatnet's peer-name
#     pattern and infrastructure by the function prefixes plus the goatnet's
#     peer_report_infra_prefixes,
#   - the Rosenpass roster attributes owners by the goatnet's name,
#   - the revocation driver hands GOATNET and NB_API to the mgmt-host Python,
#   - live-canary refuses --bump from an assembled tree,
#   - live-canary passes --site to both rollback avenues and, off the mesh,
#     reads the host's ledger through the goatnet's door.

bats_require_minimum_version 1.5.0

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"

setup() {
  T="$BATS_TEST_TMPDIR/checkout"
  mkdir -p "$T/ops/scripts" "$T/ops/e2e" "$T/ops/release" "$T/ops/ansible/inventory" \
           "$T/ops/ansible/group_vars/site_a_hosts" "$T/ops/ansible/group_vars/site_a_dev_hosts" \
           "$T/bin" "$BATS_TEST_TMPDIR/log"
  for f in goatnet-facts.py peer-dormancy-report.sh rosenpass-remediate.sh \
           rosenpass-revoke-and-reinvite.sh rosenpass_revoke_and_reinvite.py; do
    cp "$REPO_ROOT/ops/scripts/$f" "$T/ops/scripts/$f"
  done
  cp "$REPO_ROOT/ops/e2e/live-canary.sh" "$T/ops/e2e/live-canary.sh"
  cat > "$T/ops/ansible/inventory/doors.yaml" <<'YAML'
doors:
  site-a:
    type: ssm
    user: opsuser
    mgmt:
      - {name: mgmt-site-a-02, mesh_ip: 192.0.2.2, router_role: B, gv_group: site_a_hosts}
      - {name: mgmt-site-a-01, mesh_ip: 192.0.2.1, router_role: A, gv_group: site_a_hosts, landed: true}
      - {name: joiner-a, mesh_ip: 192.0.2.9, router_role: A, gv_group: joiner_hosts}
  site-a-dev:
    type: ssm
    user: devuser
    mgmt:
      - {name: mgmt-site-a-dev-01, mesh_ip: 198.51.100.1, router_role: A, gv_group: site_a_dev_hosts, landed: true}
YAML
  cat > "$T/ops/ansible/group_vars/site_a_hosts/vars.yml" <<'YAML'
public_mgmt_fqdn: netbird.site-a.example
peer_report_infra_prefixes:
  - lab-box
  - build-
YAML
  cat > "$T/ops/ansible/group_vars/site_a_dev_hosts/vars.yml" <<'YAML'
public_mgmt_fqdn: "{{ templated }}"
YAML
  cat > "$T/ops/release/ladder.yaml" <<'YAML'
rungs:
  - name: dev
    goatnet: site_a_dev
  - name: prod
    goatnet: site_a
YAML
  # ssh/scp stubs: record every call; the query scripts get the rows in $ROWS.
  cat > "$T/bin/ssh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$LOG/ssh"
[ -n "${ROWS:-}" ] && cat "$ROWS"
exit 0
SH
  cat > "$T/bin/scp" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$LOG/scp"
SH
  chmod +x "$T/bin/ssh" "$T/bin/scp"
  export LOG="$BATS_TEST_TMPDIR/log"
  export PATH="$T/bin:$PATH"
  unset MGMT_HOST NB_API ROWS
}

@test "goatnet-facts: the landed management host of the goatnet's gv_group" {
  run python3 "$T/ops/scripts/goatnet-facts.py" mgmt-ssh site-a
  [ "$status" -eq 0 ]
  [ "$output" = "opsuser@192.0.2.1" ]
  run python3 "$T/ops/scripts/goatnet-facts.py" mgmt-ssh site_a
  [ "$output" = "opsuser@192.0.2.1" ]
}

@test "goatnet-facts: a joiner is found in its founder's door, by router_role A" {
  run python3 "$T/ops/scripts/goatnet-facts.py" mgmt-ssh joiner
  [ "$status" -eq 0 ]
  [ "$output" = "opsuser@192.0.2.9" ]
}

@test "goatnet-facts: an undeclared goatnet prints nothing and exits 1" {
  run --separate-stderr python3 "$T/ops/scripts/goatnet-facts.py" mgmt-ssh nowhere
  [ "$status" -eq 1 ]
  [ -z "$output" ]
}

@test "goatnet-facts: mgmt-url, a list variable, a templated value, the dev rung" {
  run python3 "$T/ops/scripts/goatnet-facts.py" mgmt-url site-a
  [ "$output" = "https://netbird.site-a.example" ]
  run python3 "$T/ops/scripts/goatnet-facts.py" var site-a peer_report_infra_prefixes
  [ "$status" -eq 0 ]
  [ "$output" = "lab-box
build-" ]
  run --separate-stderr python3 "$T/ops/scripts/goatnet-facts.py" mgmt-url site-a-dev
  [ "$status" -eq 1 ]
  [ -z "$output" ]
  run python3 "$T/ops/scripts/goatnet-facts.py" dev-goatnet
  [ "$output" = "site_a_dev" ]
}

@test "each script refuses without a goatnet" {
  for s in peer-dormancy-report.sh rosenpass-remediate.sh rosenpass-revoke-and-reinvite.sh; do
    run bash "$T/ops/scripts/$s"
    [ "$status" -eq 2 ]
    [[ "$output" == *"usage:"* ]]
    run bash "$T/ops/scripts/$s" --csv
    [ "$status" -eq 2 ]
  done
  [ ! -e "$LOG/ssh" ]
}

@test "peer-dormancy-report: classes by the goatnet's name and its declared prefixes" {
  ROWS="$BATS_TEST_TMPDIR/rows"; export ROWS
  cat > "$ROWS" <<'ROWS'
peer-alice-site-a-2026-01-02-laptop|down|70|linux||false
peer-bob-other-2026-01-02-laptop|down|40|linux||false
lab-box-7|down|90|linux||false
build-runner|down|90|linux||false
mgmt-site-a-01|down|90|linux||false
peer-carol-site-a-2026-01-02-phone|UP|0|android||false
ROWS
  run bash "$T/ops/scripts/peer-dormancy-report.sh" site-a --csv
  [ "$status" -eq 0 ]
  [[ "$output" == *"peer-alice-site-a-2026-01-02-laptop,person,down,70,linux,false,REVOKE"* ]]
  [[ "$output" == *"peer-bob-other-2026-01-02-laptop,gear,down,40,linux,false,quarantine"* ]]
  [[ "$output" == *"lab-box-7,infra,down,90,linux,false,exempt"* ]]
  [[ "$output" == *"build-runner,infra,down,90,linux,false,exempt"* ]]
  [[ "$output" == *"mgmt-site-a-01,infra,down,90,linux,false,exempt"* ]]
  [[ "$output" == *"peer-carol-site-a-2026-01-02-phone,person,UP,0,android,false,active"* ]]
  grep -q "BatchMode=yes opsuser@192.0.2.1 " "$LOG/ssh"
}

@test "peer-dormancy-report: MGMT_HOST names another host" {
  ROWS="$BATS_TEST_TMPDIR/rows"; export ROWS
  echo 'x|down|1|linux||false' > "$ROWS"
  MGMT_HOST=me@192.0.2.7 run bash "$T/ops/scripts/peer-dormancy-report.sh" site-a --csv
  [ "$status" -eq 0 ]
  grep -q "BatchMode=yes me@192.0.2.7 " "$LOG/ssh"
}

@test "peer-dormancy-report: an empty read is a failure, not an all-clear" {
  run bash "$T/ops/scripts/peer-dormancy-report.sh" site-a
  [ "$status" -eq 2 ]
  [[ "$output" == *"NOT an all-clear"* ]]
}

@test "rosenpass-remediate: owners are the handles of the goatnet's peer names" {
  ROWS="$BATS_TEST_TMPDIR/rows"; export ROWS
  cat > "$ROWS" <<'ROWS'
peer-alice-site-a-2026-01-02-laptop|UP|linux|0.70.0|2026-11-01 10:00
mgmt-site-a-01|down|linux|0.70.0|2026-11-01 10:00
ROWS
  run bash "$T/ops/scripts/rosenpass-remediate.sh" site-a --csv
  [ "$status" -eq 0 ]
  [[ "$output" == *"alice,laptop,UP,linux,0.70.0,2026-11-01 10:00"* ]]
  [[ "$output" == *"(infrastructure — no handle in peer name),mgmt-site-a-01,down"* ]]
}

@test "rosenpass-revoke-and-reinvite: hands GOATNET and NB_API to the mgmt-host run" {
  run bash "$T/ops/scripts/rosenpass-revoke-and-reinvite.sh" site-a --limit=3
  [ "$status" -eq 0 ]
  grep -qF -- "-o BatchMode=yes opsuser@192.0.2.1 cd ~/rp-notice && GOATNET=site-a NB_API=https://netbird.site-a.example python3 rosenpass_revoke_and_reinvite.py --limit=3" "$LOG/ssh"
}

@test "rosenpass-revoke-and-reinvite: refuses an unknown argument and an undeclared API" {
  run bash "$T/ops/scripts/rosenpass-revoke-and-reinvite.sh" site-a '--apply;id'
  [ "$status" -eq 2 ]
  run bash "$T/ops/scripts/rosenpass-revoke-and-reinvite.sh" site-a-dev
  [ "$status" -eq 2 ]
  [[ "$output" == *"no management API for site-a-dev"* ]]
  [ ! -e "$LOG/ssh" ]
}

@test "rosenpass_revoke_and_reinvite.py: refuses without GOATNET or NB_API" {
  run env -u GOATNET -u NB_API python3 "$T/ops/scripts/rosenpass_revoke_and_reinvite.py"
  [ "$status" -eq 2 ]
  [[ "$output" == *"GOATNET names no goatnet"* ]]
  run env -u NB_API GOATNET=site-a python3 "$T/ops/scripts/rosenpass_revoke_and_reinvite.py"
  [ "$status" -eq 2 ]
  [[ "$output" == *"NB_API"* ]]
}

@test "live-canary: refuses --bump from an assembled tree" {
  : > "$T/.goat-assembled"
  : > "$T/gb"; chmod +x "$T/gb"
  run bash "$T/ops/e2e/live-canary.sh" --goatbuilder "$T/gb" --bump
  [ "$status" -eq 2 ]
  [[ "$output" == *"--bump edits the pins"* ]]
}

@test "live-canary: with no dev rung and no --env it asks for the goatnet" {
  printf 'rungs: []\n' > "$T/ops/release/ladder.yaml"
  run bash "$T/ops/e2e/live-canary.sh"
  [ "$status" -eq 2 ]
  [[ "$output" == *"name the goatnet with --env"* ]]
}

# A canary run that reaches the rollback step: an assembled tree (no branch
# check, no --bump), a goatbuilder and a rollback.sh that record their
# arguments, and an ssh that answers the ledger reads with two refs, the newest
# row a deploy until a rollback has run, then a rollback to the older ref. A
# plain ssh to an address in $SSH_DOWN does not answer; one that jumps through
# a ProxyCommand does.
canary_stubs() {
  : > "$T/.goat-assembled"
  cp "$REPO_ROOT/ops/e2e/rec.py" "$T/ops/e2e/rec.py"
  cat > "$T/gb" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$LOG/gb"
echo "e2e-canary: every host's ledger names the declared ref; DRY: would read /var/lib/goat/e2e-canary/deploy-history.jsonl"
SH
  cat > "$T/ops/scripts/rollback.sh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$LOG/rollback.sh"
echo "rolled back"
SH
  cat > "$T/bin/ssh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$LOG/ssh"
case "$*" in *ProxyCommand=*) ;; *"${SSH_DOWN:-no-such-address}"*) exit 255 ;; esac
case "$*" in
  *"wc -l"*) echo 2 ;;
  *"tail -n 1"*)
    if [ -e "$LOG/rollback.sh" ] || grep -q '^rollback e2e-canary 1 ' "$LOG/gb" 2>/dev/null; then
      echo '{"ref": "ghcr.io/example/e2e-canary:sha-a@sha256:1", "action": "rollback"}'
    else
      echo '{"ref": "ghcr.io/example/e2e-canary:sha-b@sha256:2", "action": "deploy"}'
    fi ;;
esac
exit 0
SH
  chmod +x "$T/gb" "$T/ops/scripts/rollback.sh" "$T/bin/ssh"
}

@test "live-canary: the goatbuilder avenue rolls back with --site" {
  canary_stubs
  LIVE_CANARY_AVENUE=cli run bash "$T/ops/e2e/live-canary.sh" --env site_a_dev --goatbuilder "$T/gb" --record "$BATS_TEST_TMPDIR/rec"
  [ "$status" -eq 0 ] || { printf '%s\n' "$output" >&3; return 1; }
  grep -qxF -- "rollback e2e-canary --list --site site-a-dev" "$LOG/gb"
  grep -qxF -- "rollback e2e-canary 1 --site site-a-dev --yes" "$LOG/gb"
  [ ! -e "$LOG/rollback.sh" ]
}

@test "live-canary: the scripts avenue rolls back with --site" {
  canary_stubs
  LIVE_CANARY_AVENUE=scripts run bash "$T/ops/e2e/live-canary.sh" --env site_a_dev --goatbuilder "$T/gb" --record "$BATS_TEST_TMPDIR/rec"
  [ "$status" -eq 0 ] || { printf '%s\n' "$output" >&3; return 1; }
  grep -qxF -- "e2e-canary 1 --site site-a-dev" "$LOG/rollback.sh"
  grep -qxF -- "rollback e2e-canary --list --site site-a-dev" "$LOG/gb"
}

@test "live-canary: off the mesh, the ledger is read by jumping through the door to the host" {
  canary_stubs
  printf '#!/bin/sh\necho "$*" >> "$LOG/door-open"\n' > "$T/ops/scripts/door-open.sh"
  printf '#!/bin/sh\n[ "$2" = --ssh-target ] && echo doorkeeper@127.0.0.1:12203\n' > "$T/ops/scripts/env-door.sh"
  SSH_DOWN=devuser@198.51.100.1 LIVE_CANARY_AVENUE=scripts run bash "$T/ops/e2e/live-canary.sh" --env site_a_dev --goatbuilder "$T/gb" --record "$BATS_TEST_TMPDIR/rec"
  [ "$status" -eq 0 ] || { printf '%s\n' "$output" >&3; return 1; }
  [[ "$output" == *"by jumping through the site-a-dev door (doorkeeper@127.0.0.1:12203)"* ]]
  grep -qxF -- "site-a-dev" "$LOG/door-open"
  # Every read after the first probe goes to the host's own address, its key
  # checked, through the door's tunnel hop; the door is never the destination.
  [ "$(grep -c 'ProxyCommand=ssh .* -p 12203 -W %h:%p doorkeeper@127.0.0.1 devuser@198.51.100.1 ' "$LOG/ssh")" -ge 3 ]
  grep 'ProxyCommand' "$LOG/ssh" | grep -qF "StrictHostKeyChecking=accept-new"
  [ "$(grep -vc ProxyCommand "$LOG/ssh")" -eq 1 ]
  ! grep -qE '(^| )(ssh://)?[a-z]+@127\.0\.0\.1' "$LOG/ssh"
  grep -qxF -- "e2e-canary 1 --site site-a-dev" "$LOG/rollback.sh"
}

@test "live-canary: a named --host is used as given, with no door" {
  canary_stubs
  LIVE_CANARY_AVENUE=cli run bash "$T/ops/e2e/live-canary.sh" --env site_a_dev --host ops@192.0.2.50 --goatbuilder "$T/gb" --record "$BATS_TEST_TMPDIR/rec"
  [ "$status" -eq 0 ] || { printf '%s\n' "$output" >&3; return 1; }
  grep -qF "ops@192.0.2.50" "$LOG/ssh"
  ! grep -qF "127.0.0.1" "$LOG/ssh"
}
