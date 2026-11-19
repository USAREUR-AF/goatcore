// Package storemigrate copies the portal registry between its SQLite file and
// Postgres, both ways, and proves each copy (ADR 1165 step A,
// docs/design/island-mode-multi-writer.md §5). It is goat-id-console's
// internal/storemigrate with the store table swapped for the registry's and
// REAL columns (envelopes) added to the value types.
//
// Import reads a SQLite file the pre-Postgres portal wrote and fills an empty
// Postgres database with the same rows. Export writes a fresh SQLite file, with
// the original SQLite schema, from Postgres: the rollback path to the old
// image. Both compare, table by table, the row count and a SHA-256 of a
// canonical dump on each side, and refuse to finish on any difference.
//
// The canonical dump is one line per row, columns in name order, values typed
// (\N for NULL, a decimal for an integer, the shortest round-trip form for a
// float, a Go-quoted string for text), lines
// sorted bytewise. It does not depend on column order, so a live SQLite file
// whose later columns were added by ALTER TABLE dumps the same as a fresh one,
// and it does not depend on either engine's collation.
//
// This package is the only place in the module that reads SQLite.
package storemigrate

import (
	"bytes"
	"context"
	"crypto/sha256"
	"database/sql"
	_ "embed"
	"encoding/hex"
	"errors"
	"fmt"
	"os"
	"sort"
	"strconv"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/pgstore"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
	"github.com/jmoiron/sqlx"
	_ "modernc.org/sqlite" // registers the "sqlite" database/sql driver
)

//go:embed sqliteschema/registry.sql
var registrySQLiteSchema string

// registrySQLiteAdditive is what the SQLite build's Open added after its
// schema.sql: the columns it left to ALTER TABLE and the indexes it created
// after them (the F-262 ordering). Every file the old image ran against has
// them, so a file export writes must too.
const registrySQLiteAdditive = `
ALTER TABLE vendors ADD COLUMN sponsor_authorized INTEGER NOT NULL DEFAULT 0;
ALTER TABLE vendors ADD COLUMN sponsor_max_tier TEXT NOT NULL DEFAULT '';
ALTER TABLE vendors ADD COLUMN sponsor_email TEXT NOT NULL DEFAULT '';
ALTER TABLE vendors ADD COLUMN bundle_downloaded_at INTEGER;
ALTER TABLE vendors ADD COLUMN bundle_download_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE vendors ADD COLUMN label_fidelity TEXT NOT NULL DEFAULT '';
ALTER TABLE vendors ADD COLUMN label_context TEXT NOT NULL DEFAULT '';
ALTER TABLE vendors ADD COLUMN label_access TEXT NOT NULL DEFAULT '';
ALTER TABLE vendors ADD COLUMN label_justification TEXT NOT NULL DEFAULT '';
ALTER TABLE vendor_progress ADD COLUMN publish_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE vendor_progress ADD COLUMN last_topic TEXT;
ALTER TABLE slots ADD COLUMN lifecycle_class TEXT NOT NULL DEFAULT '';
CREATE INDEX IF NOT EXISTS vendors_api_key_sha256_idx ON vendors(api_key_sha256);
CREATE INDEX IF NOT EXISTS slots_enrolled_by_idx ON slots(enrolled_by);
CREATE INDEX IF NOT EXISTS slots_identity_idx ON slots(identity_id);
`

// Store describes the store for the copy.
type Store struct {
	// Name is how reports name the store.
	Name string
	// Tables in foreign-key order: a table comes after every table it
	// references, so inserting in this order never trips a foreign key.
	Tables []string
	// SQLiteSchema is the schema the SQLite build created (its schema.sql,
	// verbatim, plus what its Open added). Export applies it to the new file so
	// the old image opens it as its own.
	SQLiteSchema string
	// Identity maps a table to its AUTOINCREMENT / identity column.
	Identity map[string]string
	// ExportOrder is the ORDER BY used when export inserts a table's rows, so
	// rowids in the new file follow creation order.
	ExportOrder map[string]string
	// migrate creates the store's Postgres schema (the store's own Open).
	migrate func(dsn string) error
}

// Registry is the portal registry (PORTAL_DB / PORTAL_DB_DSN).
var Registry = Store{
	Name:         "registry",
	Tables:       registry.Tables,
	SQLiteSchema: registrySQLiteSchema + registrySQLiteAdditive,
	Identity: map[string]string{
		"topics":              "id",
		"audit_events":        "id",
		"topic_schemas":       "id",
		"topic_code_examples": "id",
		"trashgoat_specs":     "id",
	},
	ExportOrder: map[string]string{
		"vendors":               "created_at, prefix",
		"topics":                "id",
		"audit_events":          "id",
		"topic_schemas":         "id",
		"topic_code_examples":   "id",
		"vendor_progress":       "prefix",
		"vendor_topic_activity": "prefix, topic",
		"audit_replay_cursor":   "subscriber_id",
		"trashgoat_specs":       "id",
		"slots":                 "idx, slot_id",
		"machine_grants":        "granted_at, custodian",
		"envelopes":             "created_at, anchor",
		"grants":                "created_at, id",
		"govern_admins":         "created_at, id",
		"org_multipub_pools":    "created_at, pool_handle",
		"multipub_topics":       "created_at, topic_id",
		"govern_meta":           "key",
		"data_access_requests":  "created_at, id",
		"probation":             "held_at, id",
		"topic_budgets":         "owner",
	},
	// Version 1 only: the copy maps the version 1 tables one to one. Later
	// registry migrations (2: slot generations and slot_leaves, GHSA-rr3p)
	// apply when the portal next opens the database, after the import.
	migrate: func(dsn string) error {
		ctx, cancel := context.WithTimeout(context.Background(), 2*time.Minute)
		defer cancel()
		db, err := pgstore.Open(ctx, dsn)
		if err != nil {
			return err
		}
		defer func() { _ = db.Close() }() //nolint:errcheck // process exit
		if err := pgstore.Migrate(ctx, db, registry.Migrations[:1]); err != nil {
			return err
		}
		return pgstore.RequireTables(ctx, db, registry.Tables...)
	},
}

// TableReport is one table's comparison.
type TableReport struct {
	Table      string
	SQLiteRows int
	PGRows     int
	SQLiteHash string
	PGHash     string
}

// OK reports whether both sides hold the same rows.
func (r TableReport) OK() bool {
	return r.SQLiteRows == r.PGRows && r.SQLiteHash == r.PGHash
}

// ErrMismatch is returned (wrapped) when any table differs after a copy.
var ErrMismatch = errors.New("row count or content hash differs between SQLite and Postgres")

// OpenSQLite opens a SQLite file. readOnly opens it without write access, so
// importing can never change the source.
func OpenSQLite(path string, readOnly bool) (*sqlx.DB, error) {
	mode := ""
	if readOnly {
		mode = "mode=ro&"
	}
	db, err := sqlx.Open("sqlite", fmt.Sprintf("file:%s?%s_pragma=busy_timeout(5000)&_pragma=foreign_keys(1)", path, mode))
	if err != nil {
		return nil, err
	}
	db.SetMaxOpenConns(1)
	return db, nil
}

// Checkpoint folds the write-ahead log of the SQLite file at path into the
// file itself and switches it to rollback-journal mode, so the file alone holds
// every committed row: it can be copied, made read-only (0444) and opened
// read-only later without a -wal or -shm beside it. The cutover runs it after
// the portal has stopped and before the file is archived and imported.
//
// It refuses a missing file (it never creates one) and a file another process
// still has open: the WAL cannot be fully checkpointed, or the journal mode
// cannot change, while a second connection exists.
func Checkpoint(ctx context.Context, path string) error {
	if _, err := os.Stat(path); err != nil {
		return err
	}
	db, err := sqlx.Open("sqlite", fmt.Sprintf("file:%s?mode=rw&_pragma=busy_timeout(5000)", path))
	if err != nil {
		return err
	}
	defer func() { _ = db.Close() }() //nolint:errcheck // closed after the mode change below
	db.SetMaxOpenConns(1)
	var busy, logFrames, checkpointed int
	if err := db.QueryRowxContext(ctx, `PRAGMA wal_checkpoint(TRUNCATE)`).Scan(&busy, &logFrames, &checkpointed); err != nil {
		return fmt.Errorf("%s: wal_checkpoint: %w", path, err)
	}
	if busy != 0 {
		return fmt.Errorf("%s: wal_checkpoint could not finish (busy); another process has the file open — stop the portal first", path)
	}
	var mode string
	if err := db.GetContext(ctx, &mode, `PRAGMA journal_mode=DELETE`); err != nil {
		return fmt.Errorf("%s: journal_mode=DELETE: %w", path, err)
	}
	if !strings.EqualFold(mode, "delete") {
		return fmt.Errorf("%s: journal mode is still %q; another process has the file open — stop the portal first", path, mode)
	}
	return db.Close()
}

// Import copies every table of st from the SQLite file at sqlitePath into the
// Postgres database at dsn. The Postgres tables must be empty. The copy and its
// verification run in one Postgres transaction: on any difference nothing is
// committed.
//
// The SQLite build never enforced its foreign keys, so a purged topic left its
// schema and code-example rows behind. Postgres enforces them. Import
// therefore reads a private copy of the file on which every declared ON DELETE
// CASCADE has been applied (Prune), and returns the rows that removed; the file
// at sqlitePath is never written. A row that breaks a key not declared CASCADE
// stops the import.
func Import(ctx context.Context, st Store, sqlitePath, dsn string) ([]TableReport, []Dropped, error) {
	if _, err := os.Stat(sqlitePath); err != nil {
		return nil, nil, fmt.Errorf("%s: %w", st.Name, err)
	}
	tmp, err := os.MkdirTemp("", "store-migrate-")
	if err != nil {
		return nil, nil, fmt.Errorf("%s: %w", st.Name, err)
	}
	defer func() { _ = os.RemoveAll(tmp) }() //nolint:errcheck // scratch copy
	pruned := tmp + "/pruned.db"
	dropped, err := Prune(ctx, sqlitePath, pruned)
	if err != nil {
		return nil, nil, fmt.Errorf("%s: %w", st.Name, err)
	}
	reports, err := importFile(ctx, st, pruned, dsn)
	return reports, dropped, err
}

// Dropped is one row Prune removed: a declared ON DELETE CASCADE the SQLite
// build never applied.
type Dropped struct {
	Table  string
	RowID  int64
	Parent string // the table whose row is gone
	Row    string // the row, as canonical column=value pairs
}

// Prune writes a copy of the SQLite file at src to dst (which must not exist)
// and applies, on the copy, every ON DELETE CASCADE its schema declares: each
// row whose referenced row is missing is deleted, then the check runs again,
// since removing a row can orphan rows that reference it. src is opened
// read-only. A missing reference under a key that is not ON DELETE CASCADE is
// an error; nothing is guessed.
func Prune(ctx context.Context, src, dst string) ([]Dropped, error) {
	if _, err := os.Stat(dst); err == nil {
		return nil, fmt.Errorf("prune: %s already exists", dst)
	}
	in, err := OpenSQLite(src, true)
	if err != nil {
		return nil, fmt.Errorf("prune: open %s: %w", src, err)
	}
	if _, err := in.ExecContext(ctx, `VACUUM INTO `+sqliteQuote(dst)); err != nil {
		_ = in.Close() //nolint:errcheck // read-only handle
		return nil, fmt.Errorf("prune: copy %s: %w", src, err)
	}
	_ = in.Close() //nolint:errcheck // read-only handle
	// Foreign keys off on the copy, so each deletion is seen and recorded by
	// the check below rather than cascaded silently by SQLite.
	db, err := sqlx.Open("sqlite", fmt.Sprintf("file:%s?_pragma=busy_timeout(5000)&_pragma=foreign_keys(0)", dst))
	if err != nil {
		return nil, err
	}
	defer func() { _ = db.Close() }() //nolint:errcheck // scratch copy
	db.SetMaxOpenConns(1)
	var dropped []Dropped
	for pass := 0; pass < 16; pass++ {
		var bad []struct {
			Table  string        `db:"table"`
			RowID  sql.NullInt64 `db:"rowid"`
			Parent string        `db:"parent"`
			FKID   int           `db:"fkid"`
		}
		if err := db.SelectContext(ctx, &bad, `PRAGMA foreign_key_check`); err != nil {
			return dropped, fmt.Errorf("prune: foreign_key_check: %w", err)
		}
		if len(bad) == 0 {
			return dropped, nil
		}
		for _, b := range bad {
			if !b.RowID.Valid {
				return dropped, fmt.Errorf("prune: %s has a row without a rowid referencing missing %s", b.Table, b.Parent)
			}
			action, err := onDelete(ctx, db, b.Table, b.FKID)
			if err != nil {
				return dropped, err
			}
			if action != "CASCADE" {
				return dropped, fmt.Errorf("prune: %s rowid %d references a missing %s row, and that key is ON DELETE %s, not CASCADE; resolve it by hand",
					b.Table, b.RowID.Int64, b.Parent, action)
			}
			row, err := rowText(ctx, db, b.Table, b.RowID.Int64)
			if err != nil {
				return dropped, err
			}
			if _, err := db.ExecContext(ctx, `DELETE FROM `+b.Table+` WHERE rowid = ?`, b.RowID.Int64); err != nil {
				return dropped, fmt.Errorf("prune: delete %s rowid %d: %w", b.Table, b.RowID.Int64, err)
			}
			dropped = append(dropped, Dropped{Table: b.Table, RowID: b.RowID.Int64, Parent: b.Parent, Row: row})
		}
	}
	return dropped, fmt.Errorf("prune: references still missing after 16 passes")
}

func onDelete(ctx context.Context, db *sqlx.DB, table string, fkid int) (string, error) {
	rows, err := db.QueryxContext(ctx, `PRAGMA foreign_key_list(`+table+`)`)
	if err != nil {
		return "", fmt.Errorf("prune: foreign_key_list(%s): %w", table, err)
	}
	defer func() { _ = rows.Close() }() //nolint:errcheck // read
	for rows.Next() {
		m := map[string]any{}
		if err := rows.MapScan(m); err != nil {
			return "", err
		}
		if id, ok := m["id"].(int64); ok && int(id) == fkid {
			return strings.ToUpper(fmt.Sprint(m["on_delete"])), nil
		}
	}
	return "", fmt.Errorf("prune: %s has no foreign key %d", table, fkid)
}

func rowText(ctx context.Context, db *sqlx.DB, table string, rowid int64) (string, error) {
	m := map[string]any{}
	if err := db.QueryRowxContext(ctx, `SELECT * FROM `+table+` WHERE rowid = ?`, rowid).MapScan(m); err != nil {
		return "", fmt.Errorf("prune: read %s rowid %d: %w", table, rowid, err)
	}
	keys := make([]string, 0, len(m))
	for k := range m {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	parts := make([]string, 0, len(keys))
	for _, k := range keys {
		v, err := canonical(m[k])
		if err != nil {
			return "", err
		}
		parts = append(parts, k+"="+v)
	}
	return strings.Join(parts, " "), nil
}

func sqliteQuote(s string) string { return "'" + strings.ReplaceAll(s, "'", "''") + "'" }

// importFile is Import on a file already pruned.
func importFile(ctx context.Context, st Store, sqlitePath, dsn string) ([]TableReport, error) {
	src, err := OpenSQLite(sqlitePath, true)
	if err != nil {
		return nil, fmt.Errorf("%s: open sqlite: %w", st.Name, err)
	}
	defer func() { _ = src.Close() }() //nolint:errcheck // read-only handle
	stx, err := src.BeginTxx(ctx, &sql.TxOptions{ReadOnly: true})
	if err != nil {
		return nil, fmt.Errorf("%s: sqlite read transaction: %w", st.Name, err)
	}
	defer func() { _ = stx.Rollback() }() //nolint:errcheck // read-only transaction

	if err := st.migrate(dsn); err != nil {
		return nil, fmt.Errorf("%s: prepare postgres schema: %w", st.Name, err)
	}
	pg, err := pgstore.Open(ctx, dsn)
	if err != nil {
		return nil, fmt.Errorf("%s: %w", st.Name, err)
	}
	defer func() { _ = pg.Close() }() //nolint:errcheck // process exit
	ptx, err := pg.BeginTxx(ctx, nil)
	if err != nil {
		return nil, fmt.Errorf("%s: postgres transaction: %w", st.Name, err)
	}
	defer func() { _ = ptx.Rollback() }() //nolint:errcheck // no-op after Commit
	if err := requireVersion1(ctx, ptx); err != nil {
		return nil, fmt.Errorf("%s: %w", st.Name, err)
	}

	for _, t := range st.Tables {
		var n int
		if err := ptx.GetContext(ctx, &n, `SELECT COUNT(*) FROM `+t); err != nil {
			return nil, fmt.Errorf("%s.%s: count postgres rows: %w", st.Name, t, err)
		}
		if n != 0 {
			return nil, fmt.Errorf("%s.%s: postgres table already holds %d rows; import only fills an empty database", st.Name, t, n)
		}
		cols, err := matchingColumns(ctx, stx, ptx, t)
		if err != nil {
			return nil, fmt.Errorf("%s.%s: %w", st.Name, t, err)
		}
		if err := copyRows(ctx, stx, ptx, t, cols, "rowid", pgPlaceholders); err != nil {
			return nil, fmt.Errorf("%s.%s: %w", st.Name, t, err)
		}
		if col, ok := st.Identity[t]; ok {
			if err := carrySequenceToPG(ctx, stx, ptx, t, col); err != nil {
				return nil, fmt.Errorf("%s.%s: identity: %w", st.Name, t, err)
			}
		}
	}
	reports, err := compare(ctx, st, stx, ptx)
	if err != nil {
		return reports, err
	}
	if err := ptx.Commit(); err != nil {
		return reports, fmt.Errorf("%s: commit: %w", st.Name, err)
	}
	return reports, nil
}

// Export writes the tables of st from the Postgres database at dsn into a new
// SQLite file at sqlitePath, created with the original SQLite schema. The file
// must not exist. It is written beside the target as <path>.partial and renamed
// into place only after every table compares equal.
func Export(ctx context.Context, st Store, dsn, sqlitePath string) ([]TableReport, error) {
	if _, err := os.Stat(sqlitePath); err == nil {
		return nil, fmt.Errorf("%s: %s already exists; export writes a new file only", st.Name, sqlitePath)
	} else if !errors.Is(err, os.ErrNotExist) {
		return nil, fmt.Errorf("%s: %w", st.Name, err)
	}
	pg, err := pgstore.Open(ctx, dsn)
	if err != nil {
		return nil, fmt.Errorf("%s: %w", st.Name, err)
	}
	defer func() { _ = pg.Close() }() //nolint:errcheck // process exit
	ptx, err := pg.BeginTxx(ctx, &sql.TxOptions{Isolation: sql.LevelRepeatableRead, ReadOnly: true})
	if err != nil {
		return nil, fmt.Errorf("%s: postgres transaction: %w", st.Name, err)
	}
	defer func() { _ = ptx.Rollback() }() //nolint:errcheck // read-only transaction
	if err := requireVersion1(ctx, ptx); err != nil {
		return nil, fmt.Errorf("%s: %w", st.Name, err)
	}
	if err := pgstore.RequireTables(ctx, ptx, st.Tables...); err != nil {
		return nil, fmt.Errorf("%s: %w", st.Name, err)
	}

	partial := sqlitePath + ".partial"
	for _, p := range []string{partial, partial + "-journal", partial + "-wal", partial + "-shm"} {
		_ = os.Remove(p) //nolint:errcheck // leftovers of an earlier failed export; absent is fine
	}
	ok := false
	defer func() {
		if !ok {
			_ = os.Remove(partial) //nolint:errcheck // a failed export leaves no file behind
		}
	}()
	dst, err := OpenSQLite(partial, false)
	if err != nil {
		return nil, fmt.Errorf("%s: create sqlite: %w", st.Name, err)
	}
	defer func() { _ = dst.Close() }() //nolint:errcheck // closed explicitly below on success
	if _, err := dst.ExecContext(ctx, st.SQLiteSchema); err != nil {
		return nil, fmt.Errorf("%s: apply sqlite schema: %w", st.Name, err)
	}
	stx, err := dst.BeginTxx(ctx, nil)
	if err != nil {
		return nil, fmt.Errorf("%s: sqlite transaction: %w", st.Name, err)
	}
	defer func() { _ = stx.Rollback() }() //nolint:errcheck // no-op after Commit
	for _, t := range st.Tables {
		cols, err := matchingColumns(ctx, stx, ptx, t)
		if err != nil {
			return nil, fmt.Errorf("%s.%s: %w", st.Name, t, err)
		}
		order := st.ExportOrder[t]
		if order == "" {
			return nil, fmt.Errorf("%s.%s: no export order declared", st.Name, t)
		}
		if err := copyRows(ctx, ptx, stx, t, cols, order, sqlitePlaceholders); err != nil {
			return nil, fmt.Errorf("%s.%s: %w", st.Name, t, err)
		}
		if col, ok := st.Identity[t]; ok {
			if err := carrySequenceToSQLite(ctx, ptx, stx, t, col); err != nil {
				return nil, fmt.Errorf("%s.%s: identity: %w", st.Name, t, err)
			}
		}
	}
	reports, err := compare(ctx, st, stx, ptx)
	if err != nil {
		return reports, err
	}
	if err := stx.Commit(); err != nil {
		return reports, fmt.Errorf("%s: commit sqlite: %w", st.Name, err)
	}
	if err := dst.Close(); err != nil {
		return reports, fmt.Errorf("%s: close sqlite: %w", st.Name, err)
	}
	if err := os.Rename(partial, sqlitePath); err != nil {
		return reports, fmt.Errorf("%s: %w", st.Name, err)
	}
	ok = true
	return reports, nil
}

// requireVersion1 refuses a Postgres database whose schema is not exactly the
// step A schema: the copy maps version 1 tables one to one and nothing else.
func requireVersion1(ctx context.Context, q sqlx.QueryerContext) error {
	v, err := pgstore.SchemaVersion(ctx, q)
	if err != nil {
		return fmt.Errorf("read schema version: %w", err)
	}
	if v != 1 {
		return fmt.Errorf("postgres schema is at version %d; this tool copies version 1 only", v)
	}
	return nil
}

// Engine says which side a handle is on, which decides how columns are listed.
type Engine int

// The two engines.
const (
	SQLite Engine = iota
	Postgres
)

// Columns lists a table's columns, sorted by name.
func Columns(ctx context.Context, q sqlx.QueryerContext, e Engine, table string) ([]string, error) {
	var cols []string
	var err error
	if e == SQLite {
		err = sqlx.SelectContext(ctx, q, &cols, `SELECT name FROM pragma_table_info(?)`, table)
	} else {
		err = sqlx.SelectContext(ctx, q, &cols, `SELECT column_name FROM information_schema.columns
			WHERE table_schema = current_schema() AND table_name = $1`, table)
	}
	if err != nil {
		return nil, fmt.Errorf("list columns: %w", err)
	}
	if len(cols) == 0 {
		return nil, fmt.Errorf("table %s not found", table)
	}
	sort.Strings(cols)
	return cols, nil
}

// matchingColumns returns the table's columns, sorted by name, after checking
// both sides have the same set. A SQLite file missing a column is one the
// current SQLite build never opened (its Open adds every column); open it with
// that build once, then import.
func matchingColumns(ctx context.Context, lite, pg sqlx.QueryerContext, table string) ([]string, error) {
	lc, err := Columns(ctx, lite, SQLite, table)
	if err != nil {
		return nil, fmt.Errorf("sqlite: %w", err)
	}
	pc, err := Columns(ctx, pg, Postgres, table)
	if err != nil {
		return nil, fmt.Errorf("postgres: %w", err)
	}
	if strings.Join(lc, ",") != strings.Join(pc, ",") {
		return nil, fmt.Errorf("columns differ: sqlite has [%s], postgres has [%s]", strings.Join(lc, " "), strings.Join(pc, " "))
	}
	return lc, nil
}

func pgPlaceholders(n int) string {
	ps := make([]string, n)
	for i := range ps {
		ps[i] = "$" + strconv.Itoa(i+1)
	}
	return strings.Join(ps, ", ")
}

func sqlitePlaceholders(n int) string {
	return strings.TrimSuffix(strings.Repeat("?, ", n), ", ")
}

// copyRows inserts every row of table from one side into the other, reading in
// orderBy order. Values pass through as the driver returns them (int64,
// float64, string or nil); anything else stops the copy, because version 1 has
// no other type.
func copyRows(ctx context.Context, from sqlx.QueryerContext, to sqlx.ExecerContext, table string, cols []string, orderBy string, placeholders func(int) string) error {
	list := strings.Join(cols, ", ")
	rows, err := from.QueryxContext(ctx, `SELECT `+list+` FROM `+table+` ORDER BY `+orderBy)
	if err != nil {
		return fmt.Errorf("read: %w", err)
	}
	defer func() { _ = rows.Close() }() //nolint:errcheck // read path; rows.Err is checked below
	insert := `INSERT INTO ` + table + ` (` + list + `) VALUES (` + placeholders(len(cols)) + `)`
	for rows.Next() {
		vals, err := rows.SliceScan()
		if err != nil {
			return fmt.Errorf("scan: %w", err)
		}
		for i, v := range vals {
			if b, ok := v.([]byte); ok && utf8.Valid(b) {
				vals[i] = string(b) // text stored with blob affinity: written as text
				v = vals[i]
			}
			if _, err := canonical(v); err != nil {
				return fmt.Errorf("column %s: %w", cols[i], err)
			}
		}
		if _, err := to.ExecContext(ctx, insert, vals...); err != nil {
			return fmt.Errorf("write: %w", err)
		}
	}
	return rows.Err()
}

// carrySequenceToPG sets the identity sequence so the next generated id is the
// one SQLite would have issued: past both the highest id and sqlite_sequence
// (AUTOINCREMENT never reuses an id, even a deleted one).
func carrySequenceToPG(ctx context.Context, lite sqlx.QueryerContext, pg sqlx.ExtContext, table, col string) error {
	var next int64
	var hasSeqTable int
	if err := sqlx.GetContext(ctx, lite, &hasSeqTable, `SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'sqlite_sequence'`); err != nil {
		return err
	}
	if hasSeqTable > 0 {
		var seq sql.NullInt64
		err := sqlx.GetContext(ctx, lite, &seq, `SELECT seq FROM sqlite_sequence WHERE name = ?`, table)
		if err != nil && !errors.Is(err, sql.ErrNoRows) {
			return err
		}
		next = seq.Int64
	}
	var maxID int64
	if err := sqlx.GetContext(ctx, pg, &maxID, `SELECT COALESCE(MAX(`+col+`), 0) FROM `+table); err != nil {
		return err
	}
	if maxID > next {
		next = maxID
	}
	if next == 0 {
		return nil
	}
	_, err := pg.ExecContext(ctx, `SELECT setval(pg_get_serial_sequence($1, $2), $3, true)`, table, col, next)
	return err
}

// carrySequenceToSQLite records the identity sequence's last value in
// sqlite_sequence, so the old image's AUTOINCREMENT continues past it.
func carrySequenceToSQLite(ctx context.Context, pg sqlx.QueryerContext, lite sqlx.ExtContext, table, col string) error {
	var last sql.NullInt64
	if err := sqlx.GetContext(ctx, pg, &last,
		`SELECT pg_sequence_last_value(pg_get_serial_sequence($1, $2)::regclass)`, table, col); err != nil {
		return err
	}
	if !last.Valid {
		return nil // never used: the inserted rows (none) set nothing either
	}
	res, err := lite.ExecContext(ctx, `UPDATE sqlite_sequence SET seq = MAX(seq, ?) WHERE name = ?`, last.Int64, table)
	if err != nil {
		return err
	}
	if n, err := res.RowsAffected(); err != nil {
		return err
	} else if n == 0 {
		_, err = lite.ExecContext(ctx, `INSERT INTO sqlite_sequence (name, seq) VALUES (?, ?)`, table, last.Int64)
		return err
	}
	return nil
}

// compare dumps every table on both sides and reports counts and hashes.
func compare(ctx context.Context, st Store, lite, pg sqlx.QueryerContext) ([]TableReport, error) {
	var reports []TableReport
	bad := []string{}
	for _, t := range st.Tables {
		ld, ln, err := Dump(ctx, lite, SQLite, t)
		if err != nil {
			return reports, fmt.Errorf("%s.%s: sqlite dump: %w", st.Name, t, err)
		}
		pd, pn, err := Dump(ctx, pg, Postgres, t)
		if err != nil {
			return reports, fmt.Errorf("%s.%s: postgres dump: %w", st.Name, t, err)
		}
		r := TableReport{Table: t, SQLiteRows: ln, PGRows: pn, SQLiteHash: Hash(ld), PGHash: Hash(pd)}
		reports = append(reports, r)
		if !r.OK() {
			bad = append(bad, t)
		}
	}
	if len(bad) > 0 {
		return reports, fmt.Errorf("%s: %w: %s", st.Name, ErrMismatch, strings.Join(bad, ", "))
	}
	return reports, nil
}

// Dump returns the canonical dump of one table and its row count: a header
// line naming the columns, then one line per row, rows sorted bytewise.
func Dump(ctx context.Context, q sqlx.QueryerContext, e Engine, table string) ([]byte, int, error) {
	cols, err := Columns(ctx, q, e, table)
	if err != nil {
		return nil, 0, err
	}
	rows, err := q.QueryxContext(ctx, `SELECT `+strings.Join(cols, ", ")+` FROM `+table)
	if err != nil {
		return nil, 0, err
	}
	defer func() { _ = rows.Close() }() //nolint:errcheck // read path; rows.Err is checked below
	var lines [][]byte
	for rows.Next() {
		vals, err := rows.SliceScan()
		if err != nil {
			return nil, 0, err
		}
		var b bytes.Buffer
		for i, v := range vals {
			c, err := canonical(v)
			if err != nil {
				return nil, 0, fmt.Errorf("column %s: %w", cols[i], err)
			}
			if i > 0 {
				b.WriteByte('\t')
			}
			b.WriteString(c)
		}
		lines = append(lines, b.Bytes())
	}
	if err := rows.Err(); err != nil {
		return nil, 0, err
	}
	sort.Slice(lines, func(i, j int) bool { return bytes.Compare(lines[i], lines[j]) < 0 })
	var out bytes.Buffer
	out.WriteString("# " + table + ": " + strings.Join(cols, "\t") + "\n")
	for _, l := range lines {
		out.Write(l)
		out.WriteByte('\n')
	}
	return out.Bytes(), len(lines), nil
}

// Hash is the hex SHA-256 of a dump.
func Hash(dump []byte) string {
	sum := sha256.Sum256(dump)
	return hex.EncodeToString(sum[:])
}

// canonical renders one value. Version 1 holds only integers, floats (the
// envelopes table's REAL columns), text and NULL.
func canonical(v any) (string, error) {
	switch x := v.(type) {
	case nil:
		return `\N`, nil
	case []byte:
		// A TEXT column SQLite stored with blob affinity (a []byte write of
		// JSON, which onboard_proposals.manifest_json can hold).
		// The bytes are text, so they are read as text; anything that is not
		// valid UTF-8 is still refused.
		if !utf8.Valid(x) {
			return "", fmt.Errorf("a blob that is not valid UTF-8 text (%d bytes)", len(x))
		}
		return strconv.Quote(string(x)), nil
	case int64:
		return strconv.FormatInt(x, 10), nil
	case float64:
		return "f" + strconv.FormatFloat(x, 'g', -1, 64), nil
	case string:
		return strconv.Quote(x), nil
	default:
		return "", fmt.Errorf("unexpected value type %T (version 1 holds only integers, floats, text and NULL)", v)
	}
}
