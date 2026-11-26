# shellcheck shell=bash disable=SC2034  # sourced by ../run.sh, which reads EXPECT
# Row N15. Class 5: warrant ACL policies regenerated from the warrant files.
# Rule (design §3 class 5, inventory §9 D8): while cut off a site may PUT
# existing policy ids only; POST and DELETE run at the hub after heal.
# Pass beyond the common conditions: one policy per warrant rule.
#
# Driven by: SQL, as ops/ansible/netbird-acl-cutover.yml makes the calls:
# `PUT /api/policies/{id}` with the full body for a policy that exists,
# `POST /api/policies` for one that does not (NetBird assigns the id), each
# bumping accounts.network_serial. The warrant gained rule-2 before the cut.
# Each site runs the cutover with cut_off=true while cut off: rule-2 would be
# a create, so the run refuses with GB-295 before any write. After heal the
# hub (A) runs it without cut_off and POSTs rule-2 once.

POL1=$(xid); R1=$(xid)
EXPECT=""

seed() {
  nb a "INSERT INTO policies (id, account_id, name, description, enabled, source_posture_checks)
        VALUES ('$POL1', '$ACCT', 'warrant: harness-rule-1', 'generated', true, '[]');
        INSERT INTO policy_rules (id, policy_id, name, enabled, action, destinations, sources, bidirectional, protocol, ports)
        VALUES ('$R1', '$POL1', 'harness-rule-1', true, 'accept', '[\"g-dst\"]', '[\"g-src\"]', true, 'all', '[]');"
}
put_existing() { # <site>
  nb "$1" "BEGIN;
           UPDATE policies SET name = 'warrant: harness-rule-1', description = 'generated', enabled = true WHERE id = '$POL1';
           UPDATE policy_rules SET name = 'harness-rule-1', enabled = true, action = 'accept',
                  destinations = '[\"g-dst\"]', sources = '[\"g-src\"]', bidirectional = true, protocol = 'all' WHERE id = '$R1';
           UPDATE accounts SET network_serial = network_serial + 1 WHERE id = '$ACCT';
           COMMIT;"
}
rule2_missing() { # <site>
  [[ "$(nb "$1" "SELECT count(*) FROM policies WHERE name = 'warrant: harness-rule-2';")" == 0 ]]
}
post_rule2() { # <site>
  local pol2 r2; pol2=$(xid); r2=$(xid)
  nb "$1" "BEGIN;
           INSERT INTO policies (id, account_id, name, description, enabled, source_posture_checks)
           VALUES ('$pol2', '$ACCT', 'warrant: harness-rule-2', 'generated', true, '[]');
           INSERT INTO policy_rules (id, policy_id, name, enabled, action, destinations, sources, bidirectional, protocol, ports)
           VALUES ('$r2', '$pol2', 'harness-rule-2', true, 'accept', '[\"g-dst2\"]', '[\"g-src\"]', true, 'all', '[]');
           UPDATE accounts SET network_serial = network_serial + 1 WHERE id = '$ACCT';
           COMMIT;"
}
regenerate_cut_off() { # <site>: refuse before any write when a create is pending (GB-295)
  if rule2_missing "$1"; then return 0; fi
  put_existing "$1"
}
write_a() { regenerate_cut_off a; }
write_b() { regenerate_cut_off b; }
after_heal() { # the hub, without cut_off
  put_existing a
  if rule2_missing a; then post_rule2 a; fi
}

check() {
  for s in a b; do
    expect_eq "$s: policies for harness-rule-1" "$(nb "$s" "SELECT count(*) FROM policies WHERE name='warrant: harness-rule-1';")" 1
    expect_eq "$s: policies for harness-rule-2" "$(nb "$s" "SELECT count(*) FROM policies WHERE name='warrant: harness-rule-2';")" 1
  done
}
