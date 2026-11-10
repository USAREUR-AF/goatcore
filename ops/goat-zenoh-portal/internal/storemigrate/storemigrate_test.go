package storemigrate

import (
	"bytes"
	"context"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/pgstore"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/pgtest"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
	"github.com/jmoiron/sqlx"
)

// registryFixture covers every registry table, with NULLs in nullable INTEGER
// and TEXT columns, 0/1 flags, REAL values that are not exact in decimal, JSON
// text, and text a naive dump would mangle (quotes, tab, newline, non-ASCII).
// The schema is the SQLite build's: schema.sql plus the columns its Open added
// by ALTER TABLE, so vendors, vendor_progress and slots have the column order
// of a long-lived file. Topic ids have a gap and sqlite_sequence is past the
// highest surviving id.
var registryFixture = []string{
	Registry.SQLiteSchema,
	`INSERT INTO vendors (prefix, api_key_hash, api_key_sha256, display_name, approved_at, approver, acl_rendered_at, created_at, bundle_token, bundle_path, bundle_minted_at, synthetic, sponsor_authorized, sponsor_max_tier, sponsor_email, bundle_downloaded_at, bundle_download_count, label_fidelity, label_context, label_access, label_justification)
	 VALUES ('aardwolf', '$2a$10$hash', 'abc123', 'Aardwolf "Labs"	tab', 1800000100, 'ops', 1800000200, 1800000000, 'tok-1', '/opt/goat-zenoh-portal/bundles/aardwolf.tgz', 1800000150, 0, 1, 'operator', 'pat@example.test', 1800000160, 3, 'real', 'exercise', 'org', 'line one
line two')`,
	`INSERT INTO vendors (prefix, api_key_hash, display_name, created_at, synthetic) VALUES ('zoë-ünïcode', 'h2', 'Forțele Terestre', 1800000000, 1)`,
	`INSERT INTO topics (id, vendor_prefix, key_pattern, format, schema_ref, description, publish_to_storage, last_seen_at, publish_count, created_at, retired_at)
	 VALUES (1, 'aardwolf', 'aardwolf/telemetry/**', 'protobuf:goat.Telemetry', 'telemetry.proto', 'all telemetry', 1, 1800000300, 42, 1800000000, NULL)`,
	`INSERT INTO topics (id, vendor_prefix, key_pattern, format, created_at, retired_at) VALUES (4, 'aardwolf', 'aardwolf/old', 'json', 1800000000, 1800000400)`,
	`INSERT INTO topics (id, vendor_prefix, key_pattern, format, created_at) VALUES (9, 'zoë-ünïcode', 'zoë-ünïcode/x', 'text', 1800000000)`,
	`DELETE FROM topics WHERE id = 9`,
	`INSERT INTO audit_events (occurred_at, actor, action, target, details) VALUES (1800000000, 'operator', 'vendor-register', 'aardwolf', '{"note":"first"}')`,
	`INSERT INTO audit_events (occurred_at, actor, action, target) VALUES (1800000001, 'aardwolf', 'topic-create', 'aardwolf/telemetry/**')`,
	`INSERT INTO topic_schemas (topic_id, version, compatibility, format, file_path, size_bytes, sha256, uploaded_at, uploaded_by)
	 VALUES (1, '1.0.0', 'backward', 'protobuf-fds', '/opt/goat-zenoh-portal/schemas/1/1.0.0.fds', 1024, 'deadbeef', 1800000500, 'aardwolf')`,
	`INSERT INTO topic_code_examples (topic_id, language, code, updated_at, updated_by) VALUES (1, 'python', 'print("hi")
	# ünïcode', 1800000600, 'aardwolf')`,
	`INSERT INTO vendor_progress (prefix, l2_at, l3_at, l4_at, last_publish_at, peer_count, updated_at, publish_count, last_topic)
	 VALUES ('aardwolf', 1800000010, 1800000020, NULL, 1800000030, 2, 1800000040, 179000000, 'aardwolf/telemetry/a')`,
	`INSERT INTO vendor_progress (prefix, updated_at) VALUES ('zoë-ünïcode', 1800000040)`,
	`INSERT INTO vendor_topic_activity (prefix, topic, publish_count, first_seen_at, last_seen_at) VALUES ('aardwolf', 'aardwolf/telemetry/a', 40, 1800000000, 1800000100)`,
	`INSERT INTO audit_replay_cursor (subscriber_id, last_key, last_seen_at) VALUES ('site-A', 'audit/2027/01/01/x.ndjson', 1800000000)`,
	`INSERT INTO trashgoat_specs (slug, description, topic_pattern, payload_spec_json, cadence_spec_json, state, notes, created_at, created_by, last_activated_at)
	 VALUES ('noise-1', 'noise', 'trashgoat/noise/**', '{"kind":"random"}', '{"hz":1}', 'active', '', 1800000000, 'operator', 1800000100)`,
	`INSERT INTO slots (slot_id, idx, assigned_to, assigned_at, cert_minted_at, bundle_token, bundle_path, bundle_minted_at, revoked_at, enrolled_by, node_class, autonomy, affiliation, workloads, enrolled_at, identity_id, collected_at, lifecycle_class)
	 VALUES ('0123456789abcdef0123456789abcdef', 0, 'pat-liaison', 1800000000, 1800000001, 'stok', '/b/s.tgz', 1800000002, NULL, NULL, NULL, NULL, NULL, NULL, NULL, 'b8f0b5a4-0000-4000-8000-000000000001', 1800000003, 'real')`,
	`INSERT INTO slots (slot_id, idx, assigned_to, enrolled_by, node_class, autonomy, affiliation, workloads, enrolled_at)
	 VALUES ('fedcba9876543210fedcba9876543210', 1, 'pat-drone', '0123456789abcdef0123456789abcdef', 'SENSOR', 'manual', 'unit-alpha', 'sense,relay', 1800000010)`,
	`INSERT INTO slots (slot_id, idx) VALUES ('00000000000000000000000000000002', 2)`,
	`INSERT INTO machine_grants (custodian, quota, affiliation, site, network_admin, granted_by, granted_at, updated_at)
	 VALUES ('0123456789abcdef0123456789abcdef', 3, 'unit-alpha', 'site-1', 1, 'ops', 1800000000, 1800000001)`,
	`INSERT INTO envelopes (anchor, min_msgs_per_s, mean_msgs_per_s, high_msgs_per_s, max_msgs_per_s, min_bytes_per_s, mean_bytes_per_s, high_bytes_per_s, max_bytes_per_s, source, class_key, created_at, updated_at)
	 VALUES ('aardwolf', 0.1, 0.30000000000000004, 12.5, 1e6, 0, 1234.5678, 1.7976931348623157e308, 5e-324, 'edited', 'SENSOR', 1800000000, 1800000001)`,
	`INSERT INTO grants (id, grantee_kind, grantee_id, producer, action, granted_by, justification, effective_from, effective_to, status, created_at, revoked_at, revoked_by)
	 VALUES ('grant-01', 'org', 'usareur-af', 'aardwolf', 'view', 'ops', 'needed', 1800000000, NULL, 'active', 1800000000, NULL, NULL)`,
	`INSERT INTO grants (id, grantee_kind, grantee_id, producer, action, granted_by, effective_from, effective_to, status, created_at, revoked_at, revoked_by)
	 VALUES ('grant-02', 'principal', 'b8f0b5a4-0000-4000-8000-000000000001', 'multipub/p1/t1', 'write', 'ops', 1800000000, 1800009999, 'revoked', 1800000000, 1800000500, 'ops')`,
	`INSERT INTO govern_admins (id, org, identity_id, nominated_by, justification, status, created_at) VALUES ('gadm-01', 'usareur-af', 'b8f0b5a4-0000-4000-8000-000000000001', 'ops', '', 'active', 1800000000)`,
	`INSERT INTO org_multipub_pools (pool_handle, org, created_by, created_at) VALUES ('p1', 'usareur-af', 'ops', 1800000000)`,
	`INSERT INTO multipub_topics (topic_id, org, pool_handle, display_name, justification, persist, status, created_by, created_at) VALUES ('t1', 'usareur-af', 'p1', 'shared track', 'why', 0, 'active', 'ops', 1800000000)`,
	`INSERT INTO govern_meta (key, value, updated_at) VALUES ('governance_mode', 'fail-closed', 1800000000)`,
	`INSERT INTO data_access_requests (id, requester_id, requester_label, producer, grantee_kind, grantee_id, justification, status, created_at, decided_by, decided_at, grant_id, note)
	 VALUES ('dar-01', 'b8f0b5a4-0000-4000-8000-000000000001', 'pat', 'aardwolf', 'principal', 'b8f0b5a4-0000-4000-8000-000000000001', 'need it', 'approved', 1800000000, 'ops', 1800000100, 'grant-01', '')`,
	`INSERT INTO probation (id, ref, kind, label, reason, detected_at, held_at, held_by, cure_by, disposition, decided_at, decided_by, note)
	 VALUES ('prob-01', 'fedcba9876543210fedcba9876543210', 'identity', 'pat-drone', 'stray:unregistered-key', 0, 1800000000, 'ops', 1800600000, 'probation', NULL, '', '')`,
	`INSERT INTO topic_budgets (owner, granted, prefixes, note, updated_at, updated_by) VALUES ('0123456789abcdef0123456789abcdef', 500, '["a/","b/"]', 'ops 2027-01-01', 1800000000, 'ops')`,
}

func writeSQLite(ctx context.Context, t *testing.T, path string, stmts ...string) {
	t.Helper()
	db, err := OpenSQLite(path, false)
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = db.Close() }() //nolint:errcheck // test fixture
	for _, s := range stmts {
		if _, err := db.ExecContext(ctx, s); err != nil {
			t.Fatalf("fixture %q: %v", firstLine(s), err)
		}
	}
}

func firstLine(s string) string {
	l, _, _ := strings.Cut(strings.TrimSpace(s), "\n")
	return l
}

func dumpSQLite(ctx context.Context, t *testing.T, path string, st Store) map[string][]byte {
	t.Helper()
	db, err := OpenSQLite(path, true)
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = db.Close() }() //nolint:errcheck // test read
	out := map[string][]byte{}
	for _, tbl := range append(append([]string{}, st.Tables...), "sqlite_sequence") {
		d, n, err := Dump(ctx, db, SQLite, tbl)
		if err != nil {
			t.Fatalf("dump %s: %v", tbl, err)
		}
		if n == 0 {
			t.Fatalf("fixture table %s is empty; the round trip would prove nothing about it", tbl)
		}
		out[tbl] = d
	}
	return out
}

// A TEXT column that SQLite stored with blob affinity imports as text and
// compares equal (the console's manifest_json case); a blob that is
// not valid UTF-8 is still refused.
func TestImportReadsAValidUTF8BlobAsText(t *testing.T) {
	ctx := t.Context()
	src := filepath.Join(t.TempDir(), "registry.db")
	writeSQLite(ctx, t, src, registryFixture...)
	raw, err := sqlx.Open("sqlite", "file:"+src+"?_pragma=foreign_keys(0)")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := raw.ExecContext(ctx, `UPDATE topics SET description = CAST(description AS BLOB) WHERE id = 1`); err != nil {
		t.Fatalf("seed blob: %v", err)
	}
	var kind string
	if err := raw.GetContext(ctx, &kind, `SELECT typeof(description) FROM topics WHERE id = 1`); err != nil || kind != "blob" {
		t.Fatalf("fixture row is %q, want blob (%v)", kind, err)
	}
	_ = raw.Close() //nolint:errcheck // test
	reports, _, err := Import(ctx, Registry, src, pgtest.DSN(t))
	if err != nil {
		t.Fatalf("import: %v", err)
	}
	for _, r := range reports {
		if !r.OK() {
			t.Fatalf("import %s: %+v", r.Table, r)
		}
	}
	if _, err := canonical([]byte{0xff, 0xfe, 0x00}); err == nil {
		t.Fatal("canonical accepted a blob that is not UTF-8")
	}
}

// The round trip the rollback depends on: SQLite fixture -> import -> export
// gives table dumps byte-equal to the fixture's, sqlite_sequence included.
func TestRoundTripSQLitePostgresSQLite(t *testing.T) {
	dir := t.TempDir()
	src := filepath.Join(dir, "registry-src.db")
	back := filepath.Join(dir, "registry-back.db")
	dsn := pgtest.DSN(t)
	ctx := t.Context()
	writeSQLite(ctx, t, src, registryFixture...)

	reports, dropped, err := Import(ctx, Registry, src, dsn)
	if err != nil {
		t.Fatalf("import: %v", err)
	}
	if len(reports) != len(Registry.Tables) {
		t.Fatalf("import reported %d tables, want %d", len(reports), len(Registry.Tables))
	}
	if len(dropped) != 0 {
		t.Fatalf("import of a consistent file dropped %d row(s): %+v", len(dropped), dropped)
	}
	for _, r := range reports {
		if !r.OK() || r.SQLiteRows == 0 {
			t.Fatalf("import %s: %+v", r.Table, r)
		}
	}

	if _, err := Export(ctx, Registry, dsn, back); err != nil {
		t.Fatalf("export: %v", err)
	}
	want, got := dumpSQLite(ctx, t, src, Registry), dumpSQLite(ctx, t, back, Registry)
	for tbl, w := range want {
		if !bytes.Equal(w, got[tbl]) {
			t.Errorf("%s differs after the round trip\nfixture:\n%s\nexported:\n%s", tbl, w, got[tbl])
		}
	}

	// A second import into the same database refuses rather than doubling
	// rows; an export never overwrites a file.
	if _, _, err := Import(ctx, Registry, src, dsn); err == nil || !strings.Contains(err.Error(), "already holds") {
		t.Fatalf("second import: want a refusal, got %v", err)
	}
	if _, err := Export(ctx, Registry, dsn, back); err == nil || !strings.Contains(err.Error(), "already exists") {
		t.Fatalf("export over an existing file: want a refusal, got %v", err)
	}
}

// After an import the registry serves the copied rows, and the identity
// sequence continues past sqlite_sequence the way AUTOINCREMENT would have:
// topic 9 was issued and deleted, so the next topic is 10. Integer ids are what
// the schema files on disk are keyed by, so they must not move.
func TestImportKeepsIDsAndCarriesIdentitySequence(t *testing.T) {
	src := filepath.Join(t.TempDir(), "registry.db")
	writeSQLite(t.Context(), t, src, registryFixture...)
	dsn := pgtest.DSN(t)
	if _, _, err := Import(t.Context(), Registry, src, dsn); err != nil {
		t.Fatalf("import: %v", err)
	}
	r, err := registry.Open(dsn)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = r.Close() }) //nolint:errcheck // test teardown
	ctx := t.Context()
	tp, err := r.GetTopic(ctx, 1)
	if err != nil || tp.KeyPattern != "aardwolf/telemetry/**" || !tp.PublishToStorage {
		t.Fatalf("topic 1 after import = %+v, %v", tp, err)
	}
	v, err := r.GetVendor(ctx, "aardwolf")
	if err != nil || !v.SponsorAuthorized || v.BundleDownloadCount != 3 {
		t.Fatalf("vendor after import = %+v, %v", v, err)
	}
	id, err := r.CreateTopic(ctx, &registry.Topic{VendorPrefix: "aardwolf", KeyPattern: "aardwolf/sys/new", Format: "json"})
	if err != nil {
		t.Fatalf("CreateTopic: %v", err)
	}
	if id != 10 {
		t.Fatalf("next topic id after import = %d, want 10 (sqlite_sequence was 9)", id)
	}
}

// A content difference with equal row counts is caught by the hash.
func TestCompareCatchesContentDifference(t *testing.T) {
	ctx := t.Context()
	src := filepath.Join(t.TempDir(), "registry.db")
	writeSQLite(ctx, t, src, registryFixture...)
	dsn := pgtest.DSN(t)
	if _, _, err := Import(ctx, Registry, src, dsn); err != nil {
		t.Fatalf("import: %v", err)
	}
	pg, err := pgstore.Open(ctx, dsn)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = pg.Close() }) //nolint:errcheck // test teardown
	if _, err := pg.ExecContext(ctx, `UPDATE envelopes SET mean_msgs_per_s = 0.3 WHERE anchor = 'aardwolf'`); err != nil {
		t.Fatal(err)
	}
	lite, err := OpenSQLite(src, true)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = lite.Close() }) //nolint:errcheck // test teardown
	reports, err := compare(ctx, Registry, lite, pg)
	if !errors.Is(err, ErrMismatch) {
		t.Fatalf("compare: want ErrMismatch, got %v", err)
	}
	for _, r := range reports {
		if r.Table == "envelopes" && (r.OK() || r.SQLiteRows != r.PGRows) {
			t.Fatalf("envelopes: want equal counts and different hashes, got %+v", r)
		}
		if r.Table != "envelopes" && !r.OK() {
			t.Fatalf("%s: untouched table reported different: %+v", r.Table, r)
		}
	}
}

// A SQLite file missing a column (never opened by the last SQLite build) is
// refused with the columns named, not imported with defaults filled in.
func TestImportRefusesColumnDrift(t *testing.T) {
	src := filepath.Join(t.TempDir(), "registry.db")
	writeSQLite(t.Context(), t, src, registrySQLiteSchema)
	_, _, err := Import(t.Context(), Registry, src, pgtest.DSN(t))
	if err == nil || !strings.Contains(err.Error(), "columns differ") {
		t.Fatalf("want a column-drift refusal, got %v", err)
	}
}

// TestCheckpointLeavesASelfContainedFile: a WAL-mode file the portal wrote is
// left in rollback-journal mode with every row in the file itself, so a 0444
// copy opens read-only on its own. A missing path is refused, never created.
func TestCheckpointLeavesASelfContainedFile(t *testing.T) {
	ctx := context.Background()
	path := filepath.Join(t.TempDir(), "registry.db")
	db, err := OpenSQLite(path, false)
	if err != nil {
		t.Fatal(err)
	}
	for _, q := range []string{`PRAGMA journal_mode=WAL`, `CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)`, `INSERT INTO t (v) VALUES ('a'), ('b')`} {
		if _, err := db.ExecContext(ctx, q); err != nil {
			t.Fatalf("%s: %v", q, err)
		}
	}
	if err := db.Close(); err != nil {
		t.Fatal(err)
	}

	if err := Checkpoint(ctx, path); err != nil {
		t.Fatalf("Checkpoint: %v", err)
	}
	for _, side := range []string{"-wal", "-shm"} {
		if _, err := os.Stat(path + side); !errors.Is(err, os.ErrNotExist) {
			t.Errorf("%s%s still present after Checkpoint (err=%v)", path, side, err)
		}
	}
	if err := os.Chmod(path, 0o444); err != nil {
		t.Fatal(err)
	}
	ro, err := OpenSQLite(path, true)
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = ro.Close() }() //nolint:errcheck // test handle
	var mode string
	if err := ro.GetContext(ctx, &mode, `PRAGMA journal_mode`); err != nil {
		t.Fatal(err)
	}
	if mode != "delete" {
		t.Errorf("journal_mode = %q, want delete", mode)
	}
	var n int
	if err := ro.GetContext(ctx, &n, `SELECT COUNT(*) FROM t`); err != nil {
		t.Fatal(err)
	}
	if n != 2 {
		t.Errorf("rows = %d, want 2", n)
	}

	missing := filepath.Join(t.TempDir(), "absent.db")
	if err := Checkpoint(ctx, missing); err == nil {
		t.Error("Checkpoint of a missing file succeeded")
	}
	if _, err := os.Stat(missing); !errors.Is(err, os.ErrNotExist) {
		t.Errorf("Checkpoint created %s", missing)
	}
}

// F-593: the SQLite build never enforced its foreign keys, so a purged topic
// left its schema and code-example rows behind. Import applies the declared
// ON DELETE CASCADE on a private copy, lists what it removed, leaves the source
// file untouched, and the rest round-trips byte for byte.
func TestImportAppliesDeclaredCascadeToRowsOfPurgedTopics(t *testing.T) {
	ctx := t.Context()
	dir := t.TempDir()
	src := filepath.Join(dir, "registry.db")
	writeSQLite(ctx, t, src, registryFixture...)
	consistent := dumpSQLite(ctx, t, src, Registry)

	// The purged topic 9's rows, as the SQLite build left them.
	raw, err := sqlx.Open("sqlite", "file:"+src+"?_pragma=foreign_keys(0)")
	if err != nil {
		t.Fatal(err)
	}
	for _, q := range []string{
		`INSERT INTO topic_schemas (topic_id, version, compatibility, format, file_path, size_bytes, sha256, uploaded_at, uploaded_by)
		 VALUES (9, 'v1', 'none', 'protobuf-fds', '/opt/goat-zenoh-portal/schemas/9/v1.fds', 10, 'cafe', 1800000700, 'zoë-ünïcode')`,
		`INSERT INTO topic_code_examples (topic_id, language, code, updated_at, updated_by) VALUES (9, 'go', 'x', 1800000701, 'zoë-ünïcode')`,
	} {
		if _, err := raw.ExecContext(ctx, q); err != nil {
			t.Fatalf("seed orphan: %v", err)
		}
	}
	_ = raw.Close() //nolint:errcheck // test
	before, err := os.ReadFile(src)
	if err != nil {
		t.Fatal(err)
	}

	dsn := pgtest.DSN(t)
	reports, dropped, err := Import(ctx, Registry, src, dsn)
	if err != nil {
		t.Fatalf("import: %v", err)
	}
	for _, r := range reports {
		if !r.OK() {
			t.Fatalf("import %s: %+v", r.Table, r)
		}
	}
	got := map[string]int{}
	for _, d := range dropped {
		if d.Parent != "topics" || !strings.Contains(d.Row, "topic_id=9") {
			t.Fatalf("dropped %+v, want only topic 9's rows", d)
		}
		got[d.Table]++
	}
	if got["topic_schemas"] != 1 || got["topic_code_examples"] != 1 || len(dropped) != 2 {
		t.Fatalf("dropped = %+v, want topic 9's one schema and one code example", dropped)
	}
	after, err := os.ReadFile(src)
	if err != nil || !bytes.Equal(before, after) {
		t.Fatalf("import changed the source file (err %v)", err)
	}

	back := filepath.Join(dir, "registry-back.db")
	if _, err := Export(ctx, Registry, dsn, back); err != nil {
		t.Fatalf("export: %v", err)
	}
	// Every table round-trips to the fixture as it was before the orphans,
	// except sqlite_sequence: the removed rows' ids stay issued, so they are
	// never handed out again.
	gotDump := dumpSQLite(ctx, t, back, Registry)
	for tbl, w := range consistent {
		if tbl == "sqlite_sequence" {
			continue
		}
		if !bytes.Equal(w, gotDump[tbl]) {
			t.Errorf("%s differs from the consistent fixture after the round trip\nwant:\n%s\ngot:\n%s", tbl, w, gotDump[tbl])
		}
	}
	seq := string(gotDump["sqlite_sequence"])
	if !strings.Contains(seq, "\"topic_schemas\"\t2") || !strings.Contains(seq, "\"topic_code_examples\"\t2") {
		t.Errorf("sqlite_sequence after the round trip does not keep the removed ids issued:\n%s", seq)
	}
}

// A missing reference under a key that is not ON DELETE CASCADE is not
// guessed at: Prune refuses.
func TestPruneRefusesAMissingReferenceThatIsNotCascade(t *testing.T) {
	ctx := t.Context()
	src := filepath.Join(t.TempDir(), "x.db")
	db, err := sqlx.Open("sqlite", "file:"+src+"?_pragma=foreign_keys(0)")
	if err != nil {
		t.Fatal(err)
	}
	for _, q := range []string{
		`CREATE TABLE p (id INTEGER PRIMARY KEY)`,
		`CREATE TABLE c (id INTEGER PRIMARY KEY, p_id INTEGER REFERENCES p(id) ON DELETE RESTRICT)`,
		`INSERT INTO c (id, p_id) VALUES (1, 7)`,
	} {
		if _, err := db.ExecContext(ctx, q); err != nil {
			t.Fatal(err)
		}
	}
	_ = db.Close() //nolint:errcheck // test
	if _, err := Prune(ctx, src, filepath.Join(t.TempDir(), "out.db")); err == nil || !strings.Contains(err.Error(), "not CASCADE") {
		t.Fatalf("prune: want a refusal naming the key, got %v", err)
	}
}
