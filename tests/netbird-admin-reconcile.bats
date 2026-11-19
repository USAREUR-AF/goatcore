#!/usr/bin/env bats
# netbird-admin-reconcile.sh — the Ory group -> NetBird role map.
#
# This is a Linux-host role, outside the operator Bash 3.2 contract. Its
# interpreter stays the host shell even when Bats itself runs under 3.2.
# Runs the real script against fixture Kratos identities and a fixture NetBird
# users table (tests/shims stand in for curl and `docker exec psql`). The
# properties held here are the ones the script's SAFETY header promises:
# promote only up user < auditor < admin, never touch owner or service users,
# report strays instead of demoting, and an unset map behaves exactly as the
# old admin-groups list did (docs/design/mesh-change-control.md §2).

ROLE_DIR="$BATS_TEST_DIRNAME/../ops/ansible/roles/netbird-admin-reconcile"
SCRIPT="$ROLE_DIR/files/netbird-admin-reconcile.sh"

setup() {
  WORK="$(mktemp -d)"
  cp "$ROLE_DIR/tests/fixture-users.txt" "$WORK/users.txt"
  : > "$WORK/updates.txt"
  mkdir -p "$WORK/textfile"
  export NB_USERS="$WORK/users.txt" NB_UPDATES="$WORK/updates.txt"
  export FIXTURE_IDENTITIES="$ROLE_DIR/tests/fixture-identities.json"
  export TEXTFILE_DIR="$WORK/textfile"
  export PATH="$ROLE_DIR/tests/shims:$PATH"
  unset NETBIRD_MGMT_ROLE_MAP NETBIRD_MGMT_ADMIN_GROUPS FIXTURE_KRATOS_DOWN
}

teardown() {
  rm -rf "$WORK"
}

role_of() { awk -v id="$1" '$1 == id {print $2}' "$WORK/users.txt"; }
metric() { awk -v m="$1" '$1 == m {print $2}' "$TEXTFILE_DIR/netbird_admin_reconcile.prom"; }

@test "governed map: NETBIRD-ADMIN -> admin, PLATFORM-ADMIN and IAM-ADMIN -> auditor" {
  export NETBIRD_MGMT_ROLE_MAP="NETBIRD-ADMIN=admin,PLATFORM-ADMIN=auditor,IAM-ADMIN=auditor"
  run "${HOST_BASH_UNDER_TEST:-bash}" "$SCRIPT"
  [ "$status" -eq 0 ]
  [ "$(role_of id-nbadmin)" = admin ]      # highest mapped role wins
  [ "$(role_of id-platform)" = auditor ]
  [ "$(role_of id-iam)" = auditor ]
  [ "$(role_of id-auditor-up)" = admin ]   # auditor -> admin is a promotion
  [ "$(role_of id-plain)" = user ]         # unmapped group, untouched
}

@test "governed map never demotes: an admin the map now makes auditor stays admin and is STRAY" {
  export NETBIRD_MGMT_ROLE_MAP="NETBIRD-ADMIN=admin,PLATFORM-ADMIN=auditor,IAM-ADMIN=auditor"
  run "${HOST_BASH_UNDER_TEST:-bash}" "$SCRIPT"
  [ "$status" -eq 0 ]
  [ "$(role_of id-oldadmin)" = admin ]
  [ "$(role_of id-gone)" = admin ]
  [[ "$output" == *"STRAY netbird admin old-admin@example.test"* ]]
  [[ "$output" == *"STRAY netbird admin gone@example.test"* ]]
  [ "$(metric netbird_admin_reconcile_stray_admins)" = 2 ]
  ! grep -q ' admin auditor$\| admin user$\| auditor user$' "$WORK/updates.txt"
}

@test "migrated identities: metadata_public.groups decides, a null or [] there never reads traits" {
  cat > "$WORK/identities.json" <<'JSON'
[
  {"id": "id-platform", "state": "active", "traits": {"email": "platform@example.test", "groups": ["NETBIRD-ADMIN"]}, "metadata_public": {"groups": null}},
  {"id": "id-iam", "state": "active", "traits": {"email": "iam@example.test", "groups": ["NETBIRD-ADMIN"]}, "metadata_public": {"groups": []}},
  {"id": "id-plain", "state": "active", "traits": {"email": "plain@example.test"}, "metadata_public": {"groups": ["NETBIRD-ADMIN"]}}
]
JSON
  export FIXTURE_IDENTITIES="$WORK/identities.json"
  export NETBIRD_MGMT_ROLE_MAP="NETBIRD-ADMIN=admin"
  run "${HOST_BASH_UNDER_TEST:-bash}" "$SCRIPT"
  [ "$status" -eq 0 ]
  [ "$(role_of id-platform)" = user ]   # null marker: the self-edited trait is ignored
  [ "$(role_of id-iam)" = user ]        # [] marker: the self-edited trait is ignored
  [ "$(role_of id-plain)" = admin ]     # admin-managed metadata promotes
}

@test "GHSA-9v3v: a not-yet-backfilled identity's self-editable traits.groups never grants a role" {
  cat > "$WORK/identities.json" <<'JSON'
[
  {"id": "id-platform", "state": "active", "traits": {"email": "platform@example.test", "groups": ["NETBIRD-ADMIN"]}},
  {"id": "id-iam", "state": "active", "traits": {"email": "iam@example.test", "groups": ["NETBIRD-ADMIN"]}, "metadata_public": {"org": {}}}
]
JSON
  export FIXTURE_IDENTITIES="$WORK/identities.json"
  export NETBIRD_MGMT_ROLE_MAP="NETBIRD-ADMIN=admin"
  run "${HOST_BASH_UNDER_TEST:-bash}" "$SCRIPT"
  [ "$status" -eq 0 ]
  [ "$(role_of id-platform)" = user ]   # no metadata_public: traits ignored
  [ "$(role_of id-iam)" = user ]        # metadata_public without groups: traits ignored
}

@test "owner and service users are never touched" {
  export NETBIRD_MGMT_ROLE_MAP="NETBIRD-ADMIN=admin,PLATFORM-ADMIN=auditor"
  run "${HOST_BASH_UNDER_TEST:-bash}" "$SCRIPT"
  [ "$status" -eq 0 ]
  [ "$(role_of id-owner)" = owner ]
  [ "$(role_of svc-warrant)" = admin ]
  ! grep -q '^id-owner \|^svc-warrant ' "$WORK/updates.txt"
  [[ "$output" != *"STRAY netbird admin"*"svc-warrant"* ]]
}

@test "metrics: entitled, auditors, pending, promoted" {
  export NETBIRD_MGMT_ROLE_MAP="NETBIRD-ADMIN=admin,PLATFORM-ADMIN=auditor,IAM-ADMIN=auditor"
  run "${HOST_BASH_UNDER_TEST:-bash}" "$SCRIPT"
  [ "$status" -eq 0 ]
  [ "$(metric netbird_admin_reconcile_entitled)" = 3 ]    # nbadmin, owner, auditor-up
  [ "$(metric netbird_admin_reconcile_auditors)" = 4 ]    # platform, iam, oldadmin, pending
  [ "$(metric netbird_admin_reconcile_pending_first_login)" = 1 ]
  [ "$(metric netbird_admin_reconcile_promoted_last_run)" = 4 ]
}

@test "no map: derived from NETBIRD_MGMT_ADMIN_GROUPS, every group -> admin (pre-map behaviour)" {
  export NETBIRD_MGMT_ADMIN_GROUPS="PLATFORM-ADMIN,IAM-ADMIN"
  run "${HOST_BASH_UNDER_TEST:-bash}" "$SCRIPT"
  [ "$status" -eq 0 ]
  [ "$(role_of id-platform)" = admin ]
  [ "$(role_of id-iam)" = admin ]
  [ "$(role_of id-nbadmin)" = admin ]      # carries PLATFORM-ADMIN too
  [ "$(role_of id-auditor-up)" = auditor ] # NETBIRD-ADMIN is not in the old list
  [ "$(metric netbird_admin_reconcile_auditors)" = 0 ]
  [[ "$output" != *"STRAY netbird admin old-admin@example.test"* ]]
}

@test "no map and no admin groups: default is PLATFORM-ADMIN -> admin" {
  run "${HOST_BASH_UNDER_TEST:-bash}" "$SCRIPT"
  [ "$status" -eq 0 ]
  [ "$(role_of id-platform)" = admin ]
  [ "$(role_of id-iam)" = user ]
}

@test "a map that does not parse promotes nobody and fails the tick" {
  export NETBIRD_MGMT_ROLE_MAP="NETBIRD-ADMIN=superuser"
  run "${HOST_BASH_UNDER_TEST:-bash}" "$SCRIPT"
  [ "$status" -ne 0 ]
  [ ! -s "$WORK/updates.txt" ]
  [[ "$output" == *"does not parse"* ]]
}

@test "Kratos unreachable: nothing changes, tick exits 0" {
  export FIXTURE_KRATOS_DOWN=1
  run "${HOST_BASH_UNDER_TEST:-bash}" "$SCRIPT"
  [ "$status" -eq 0 ]
  [ ! -s "$WORK/updates.txt" ]
}
