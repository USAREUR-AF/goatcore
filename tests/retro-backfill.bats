#!/usr/bin/env bats

# Behavior tests for ops/goat-id-console/scripts/retro-backfill.py (#1244).
#
# The zagros backfill derived handles from email local-parts, wrote
# them unverified, and the enrollment watcher promoted 15 of those rows to
# `enrolled` under handles the fabric never knew. These pin the refusal
# (a handle is never written unverified), the check that fails on a
# retroactive enrolled row without an identity, and the guarded repair —
# red, then green, on one fixture database.

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
SCRIPT="$REPO_ROOT/ops/goat-id-console/scripts/retro-backfill.py"

setup() {
  DB="$BATS_TEST_TMPDIR/onboard.db"
  ROSTER="$BATS_TEST_TMPDIR/roster.txt"
  MAP="$BATS_TEST_TMPDIR/map.csv"
  # The zagros shape in miniature: one retro row with a fabricated handle
  # (jane vs the real jane-doe), one retro row whose derivation happened to
  # be right (the machine), one normal row whose person has left.
  python3 - "$DB" <<'EOF'
import sqlite3, sys
con = sqlite3.connect(sys.argv[1])
con.execute("""CREATE TABLE onboard_proposals (
  id TEXT PRIMARY KEY, created_at INTEGER, sponsor TEXT, sponsor_name TEXT,
  env TEXT, handle TEXT, email TEXT, status TEXT, manifest_json TEXT,
  lint_json TEXT, decided_by TEXT, decided_at INTEGER, note TEXT,
  escalated INTEGER, complete INTEGER, petitions TEXT, tier TEXT,
  retroactive INTEGER)""")
rows = [
  ("prop-retro-aaaaaaaaaaaa", "jane",        "person-46@example.org", "enrolled", 1),
  ("prop-retro-bbbbbbbbbbbb", "g1x-gcs",     "person-38@example.org",  "enrolled", 1),
  ("prop-1111",               "gone-person", "person-40@example.org",     "enrolled", 0),
]
for id_, handle, email, status, retro in rows:
    con.execute(
        "INSERT INTO onboard_proposals (id, created_at, sponsor, sponsor_name,"
        " env, handle, email, status, manifest_json, lint_json, decided_by,"
        " decided_at, note, escalated, complete, petitions, tier, retroactive)"
        " VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        (id_, 0, "", "", "zagros", handle, email, status, "{}", "{}",
         None, None, "", 0, 1, "[]", "", retro))
con.commit()
EOF
  printf 'g1x-gcs\njane-doe\n# a comment line\n' > "$ROSTER"
  printf 'person-46@example.org,jane-doe\n' > "$MAP"
}

@test "retro-backfill: the self-test proves the pure logic red then green" {
  run python3 "$SCRIPT" --self-test
  [ "$status" -eq 0 ]
  [[ "$output" != *"FAIL"* ]]
}

@test "retro-backfill: every mode but self-test refuses to run without a roster" {
  run python3 "$SCRIPT" --db "$DB" --check
  [ "$status" -eq 2 ]
  [[ "$output" == *"--roster is required"* ]]
}

@test "retro-backfill: the check fails on a retroactive enrolled row the roster does not know" {
  run python3 "$SCRIPT" --db "$DB" --roster "$ROSTER" --check
  [ "$status" -eq 1 ]
  [[ "$output" == *"prop-retro-aaaaaaaaaaaa"* ]]
  [[ "$output" == *"FAIL"* ]]
}

@test "retro-backfill: the check reports a non-retroactive miss without failing on it" {
  # Repair the retro row first, so only the departed normal row remains.
  python3 "$SCRIPT" --db "$DB" --roster "$ROSTER" --handle-map "$MAP" --fix-handles --apply
  run python3 "$SCRIPT" --db "$DB" --roster "$ROSTER" --check
  [ "$status" -eq 0 ]
  [[ "$output" == *"prop-1111"* ]]
  [[ "$output" == *"may have left"* ]]
}

@test "retro-backfill: fix-handles dry-run plans the update and writes nothing" {
  run python3 "$SCRIPT" --db "$DB" --roster "$ROSTER" --handle-map "$MAP" --fix-handles
  [ "$status" -eq 0 ]
  [[ "$output" == *"'jane' -> 'jane-doe'"* ]]
  [[ "$output" == *"dry-run"* ]]
  got=$(python3 -c "import sqlite3,sys; print(sqlite3.connect(sys.argv[1]).execute(\"SELECT handle FROM onboard_proposals WHERE id='prop-retro-aaaaaaaaaaaa'\").fetchone()[0])" "$DB")
  [ "$got" = "jane" ]
}

@test "retro-backfill: fix-handles --apply repairs the row, and the check goes green" {
  run python3 "$SCRIPT" --db "$DB" --roster "$ROSTER" --handle-map "$MAP" --fix-handles --apply
  [ "$status" -eq 0 ]
  [[ "$output" == *"read back and confirmed"* ]]
  got=$(python3 -c "import sqlite3,sys; print(sqlite3.connect(sys.argv[1]).execute(\"SELECT handle FROM onboard_proposals WHERE id='prop-retro-aaaaaaaaaaaa'\").fetchone()[0])" "$DB")
  [ "$got" = "jane-doe" ]
  run python3 "$SCRIPT" --db "$DB" --roster "$ROSTER" --check
  [ "$status" -eq 0 ]
}

@test "retro-backfill: a second --apply run finds nothing to fix" {
  python3 "$SCRIPT" --db "$DB" --roster "$ROSTER" --handle-map "$MAP" --fix-handles --apply
  run python3 "$SCRIPT" --db "$DB" --roster "$ROSTER" --handle-map "$MAP" --fix-handles --apply
  [ "$status" -eq 0 ]
  [[ "$output" == *"wrote 0 handle fix(es)"* ]]
}

@test "retro-backfill: a retro row with no mapping stays unfixable, never guessed" {
  printf '# empty on purpose\n' > "$MAP"
  run python3 "$SCRIPT" --db "$DB" --roster "$ROSTER" --handle-map "$MAP" --fix-handles
  [ "$status" -eq 1 ]
  [[ "$output" == *"UNFIXABLE prop-retro-aaaaaaaaaaaa"* ]]
  [[ "$output" == *"no mapping"* ]]
}

@test "retro-backfill: an empty roster is refused as an authority" {
  printf '# nothing\n' > "$ROSTER"
  run python3 "$SCRIPT" --db "$DB" --roster "$ROSTER" --check
  [ "$status" -eq 1 ]
  [[ "$output" == *"empty roster"* ]]
}
