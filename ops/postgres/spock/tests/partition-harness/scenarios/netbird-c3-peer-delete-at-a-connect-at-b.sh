# shellcheck shell=bash disable=SC2034  # sourced by ../run.sh, which reads EXPECT
# Row N4. Class 3: revocations, ADR 1193 ledger (netbird.peer.deleted).
# Carries over PR #1538's p-del.
# Pass beyond the common conditions: peer absent at both sites.
#
# Driven by: SQL. A: `DELETE /api/peers/{id}` (admin.rs run_peer,
# `goat admin peer cut`): NetBird deletes the peer's group_peers rows and the
# peer, and bumps accounts.network_serial (inventory §5). B, cut off:
# MarkPeerConnected when the peer's client connects there.
#
# Before NetBird 0.83.0 MarkPeerConnected updated the peers row
# (SavePeerStatus), which reaches A after the heal as an UPDATE of a row A
# deleted. From 0.83.0 it upserts B's site-local peer_connections row instead
# (ADR 1084 D6), the peers row is untouched, and nothing conflicts. B's
# connection row outlives the replicated delete; every reader joins from peers,
# so it is never shown.

P=$(xid); G=$(xid)
if [[ "$NB_PC" == t ]]; then
  EXPECT=""
else
  EXPECT="
a netbird exception peers UPDATE:other 1
"
fi

seed() {
  nb a "INSERT INTO groups (id, account_id, name, issued, resources) VALUES ('$G', '$ACCT', 'HARNESS-PEER-CUT', 'api', '[]');"
  nb_peer a "$P" harness-peer-cut
  nb a "INSERT INTO group_peers (account_id, group_id, peer_id) VALUES ('$ACCT', '$G', '$P');"
}
write_a() {
  local conn=""
  [[ "$NB_PC" == t ]] && conn="DELETE FROM peer_connections WHERE peer_id = '$P';"
  nb a "BEGIN;
        DELETE FROM group_peers WHERE peer_id = '$P';
        $conn
        DELETE FROM peers WHERE id = '$P';
        UPDATE accounts SET network_serial = network_serial + 1 WHERE id = '$ACCT';
        COMMIT;"
}
write_b() {
  if [[ "$NB_PC" == t ]]; then
    nb b "INSERT INTO peer_connections (peer_id, account_id, instance_id, session_started_at, last_seen)
          SELECT id, account_id, 'harness-instance-b', 1, now() FROM peers WHERE id = '$P'
          ON CONFLICT (peer_id) DO UPDATE SET instance_id = excluded.instance_id,
            session_started_at = excluded.session_started_at, last_seen = now()
          WHERE peer_connections.session_started_at < excluded.session_started_at;"
  else
    nb b "UPDATE peers SET peer_status_connected = true, peer_status_login_expired = false,
                           peer_status_last_seen = now() WHERE id = '$P';"
  fi
}

check() {
  for s in a b; do
    expect_eq "$s: deleted peer absent"     "$(nb "$s" "SELECT count(*) FROM peers WHERE id='$P';")" 0
    expect_eq "$s: deleted peer in no group" "$(nb "$s" "SELECT count(*) FROM group_peers WHERE peer_id='$P';")" 0
  done
  if [[ "$NB_PC" == t ]]; then
    expect_eq "a: no connection row for the deleted peer" "$(nb a "SELECT count(*) FROM peer_connections WHERE peer_id='$P';")" 0
  fi
}
