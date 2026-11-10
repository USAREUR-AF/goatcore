# shellcheck shell=bash disable=SC2034  # sourced by ../run.sh, which reads EXPECT
# Rows N7, N8, N14, N16, N17. Class 5: declared state, regenerated; creates
# and deletes run at the hub (site A here) after heal.
# Pass beyond the common conditions: equal at both sites after the hub run.
#
# Driven by: SQL.
#   While cut off, both sites: the netbird-admin-reconcile timer (N8,
#     roles/netbird-admin-reconcile/files/netbird-admin-reconcile.sh) runs at
#     every site and promotes the declared admin with
#     `UPDATE users SET role = ...`; same result at each site.
#   After heal, hub only: `POST /api/groups` (N16, standup) and
#     `POST /api/dns/nameservers` (N17), each bumping accounts.network_serial.

U=$(xid); G=$(xid); NS=$(xid)
EXPECT="
b netbird resolution users update_update:keep_local 1
"

seed() {
  nb a "INSERT INTO users (id, account_id, role, is_service_user, non_deletable, auto_groups, blocked,
                           pending_approval, created_at, issued)
        VALUES ('$U', '$ACCT', 'user', false, false, '[]', false, false, now(), 'jwt');"
}
promote() { nb "$1" "UPDATE users SET role = 'admin' WHERE id = '$U' AND role <> 'admin';"; }
write_a() { promote a; }
write_b() { promote b; }
after_heal() {
  nb a "BEGIN;
        INSERT INTO groups (id, account_id, name, issued, resources) VALUES ('$G', '$ACCT', 'HARNESS-DECLARED', 'api', '[]');
        UPDATE accounts SET network_serial = network_serial + 1 WHERE id = '$ACCT';
        COMMIT;"
  nb a "BEGIN;
        INSERT INTO name_server_groups (id, account_id, name, description, name_servers, groups, domains, enabled)
        VALUES ('$NS', '$ACCT', 'harness-mesh-dns', 'declared', '[{\"IP\":\"100.127.0.29\",\"NSType\":1,\"Port\":53}]',
                '[\"$G\"]', '[]', true);
        UPDATE accounts SET network_serial = network_serial + 1 WHERE id = '$ACCT';
        COMMIT;"
}

check() {
  for s in a b; do
    expect_eq "$s: declared admin promoted" "$(nb "$s" "SELECT role FROM users WHERE id='$U';")" admin
    expect_eq "$s: hub-created group"       "$(nb "$s" "SELECT count(*) FROM groups WHERE id='$G';")" 1
    expect_eq "$s: hub-created nameserver"  "$(nb "$s" "SELECT count(*) FROM name_server_groups WHERE id='$NS';")" 1
  done
}
