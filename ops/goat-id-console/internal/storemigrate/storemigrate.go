// Package storemigrate copies the console's stores between their SQLite files
// and Postgres, both ways, and proves each copy (ADR 1165 step A,
// docs/design/island-mode-multi-writer.md §5).
//
// Import reads a SQLite file the pre-Postgres console wrote and fills an empty
// Postgres database with the same rows. Export writes a fresh SQLite file, with
// the original SQLite schema, from Postgres: the rollback path to the old
// image. Both compare, table by table, the row count and a SHA-256 of a
// canonical dump on each side, and refuse to finish on any difference.
//
// The canonical dump is one line per row, columns in name order, values typed
// (\N for NULL, a decimal for an integer, a Go-quoted string for text), lines
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
	"unicode/utf8"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/orgstore"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/pgstore"
	"github.com/jmoiron/sqlx"
	_ "modernc.org/sqlite" // registers the "sqlite" database/sql driver
)

//go:embed sqliteschema/onboard.sql
var onboardSQLiteSchema string

// onboardSQLiteAdditive is the one column the SQLite build's schema.sql left to
// its additive ALTER list rather than CREATE TABLE. Every file the old image
// ran against has it, so a file export writes must too.
const onboardSQLiteAdditive = `
ALTER TABLE onboard_proposals ADD COLUMN handset_key TEXT NOT NULL DEFAULT '';
`

//go:embed sqliteschema/orgs.sql
var orgSQLiteSchema string

// Store describes one console store for the copy.
type Store struct {
	// Name is how the CLI names the store: "onboard" or "org".
	Name string
	// Tables in foreign-key order: a table comes after every table it
	// references, so inserting in this order never trips a foreign key.
	Tables []string
	// SQLiteSchema is the schema the SQLite build created (its schema.sql,
	// verbatim, plus any column only its additive ALTER list added). Export
	// applies it to the new file so the old image opens it as its own.
	SQLiteSchema string
	// Identity maps a table to its AUTOINCREMENT / identity column.
	Identity map[string]string
	// ExportOrder is the ORDER BY used when export inserts a table's rows, so
	// rowids in the new file follow the order the old image sorted ties by.
	ExportOrder map[string]string
	// migrate creates the store's Postgres schema (the store's own Open).
	migrate func(dsn string) error
	// LateColumns names, per table, columns added after a SQLite checkpoint
	// could have been written, with the canonical dump form of their default.
	// No SQLite-opening build exists any more to add them to an old file, so
	// Import accepts a checkpoint that lacks only these: Postgres fills each
	// with its DEFAULT, and the verification checks every imported row holds
	// exactly that default.
	LateColumns map[string]map[string]string
}

// Onboard is the onboarding store (ONBOARD_DB_PATH / ONBOARD_DB_DSN).
var Onboard = Store{
	Name:         "onboard",
	Tables:       onboardstore.Tables,
	SQLiteSchema: onboardSQLiteSchema + onboardSQLiteAdditive,
	ExportOrder: map[string]string{
		"onboard_proposals":               "created_at, id",
		"onboard_invites":                 "created_at, id",
		"ssh_keys":                        "created_at, id",
		"operator_keys":                   "created_at, id",
		"onboard_actions":                 "created_at, id",
		"probation_reactivation_requests": "requested_at, id",
	},
	LateColumns: map[string]map[string]string{
		// GHSA-3m4r: the offboard approver's session identity.
		// ADR 1272: the departure reason, text and effective day.
		"onboard_proposals": {"offboard_requested_by_identity": `""`, "offboard_operator_audit": `""`,
			"offboard_reason_class": `""`, "offboard_reason_text": `""`, "offboard_effective": `""`},
	},
	migrate: func(dsn string) error {
		s, err := onboardstore.Open(dsn)
		if err != nil {
			return err
		}
		return s.Close()
	},
}

// Org is the org registry (ORG_DB_PATH / ORG_DB_DSN).
var Org = Store{
	Name:         "org",
	Tables:       orgstore.Tables,
	SQLiteSchema: orgSQLiteSchema,
	Identity:     map[string]string{"org_parent_edges": "id"},
	ExportOrder: map[string]string{
		"orgs":                "created_at, id",
		"org_parent_edges":    "id",
		"org_aliases":         "recorded_at, alias",
		"org_match_overrides": "recorded_at, name_norm, org_id",
	},
	migrate: func(dsn string) error {
		s, err := orgstore.Open(dsn)
		if err != nil {
			return err
		}
		return s.Close()
	},
}

// Stores lists every store, in the order the CLI handles them.
var Stores = []Store{Onboard, Org}

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
// the console has stopped and before the file is archived and imported.
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
		return fmt.Errorf("%s: wal_checkpoint could not finish (busy); another process has the file open — stop the console first", path)
	}
	var mode string
	if err := db.GetContext(ctx, &mode, `PRAGMA journal_mode=DELETE`); err != nil {
		return fmt.Errorf("%s: journal_mode=DELETE: %w", path, err)
	}
	if !strings.EqualFold(mode, "delete") {
		return fmt.Errorf("%s: journal mode is still %q; another process has the file open — stop the console first", path, mode)
	}
	return db.Close()
}

// Import copies every table of st from the SQLite file at sqlitePath into the
// Postgres database at dsn. The Postgres tables must be empty. The copy and its
// verification run in one Postgres transaction: on any difference nothing is
// committed.
func Import(ctx context.Context, st Store, sqlitePath, dsn string) ([]TableReport, error) {
	if _, err := os.Stat(sqlitePath); err != nil {
		return nil, fmt.Errorf("%s: %w", st.Name, err)
	}
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

	shared := map[string][]string{}
	lateByTable := map[string][]string{}
	for _, t := range st.Tables {
		var n int
		if err := ptx.GetContext(ctx, &n, `SELECT COUNT(*) FROM `+t); err != nil {
			return nil, fmt.Errorf("%s.%s: count postgres rows: %w", st.Name, t, err)
		}
		if n != 0 {
			return nil, fmt.Errorf("%s.%s: postgres table already holds %d rows; import only fills an empty database", st.Name, t, n)
		}
		cols, late, err := importColumns(ctx, stx, ptx, t, st.LateColumns[t])
		if err != nil {
			return nil, fmt.Errorf("%s.%s: %w", st.Name, t, err)
		}
		if len(late) > 0 {
			shared[t] = cols
			lateByTable[t] = late
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
	reports, err := compareColumns(ctx, st, stx, ptx, shared)
	if err != nil {
		return reports, err
	}
	if err := checkLateDefaults(ctx, st, ptx, lateByTable); err != nil {
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

// importColumns is matchingColumns for Import, which also accepts a SQLite
// file that lacks some of the table's late columns (Store.LateColumns). It
// returns the columns to copy and the late columns the file lacked.
func importColumns(ctx context.Context, lite, pg sqlx.QueryerContext, table string, lateCols map[string]string) ([]string, []string, error) {
	lc, err := Columns(ctx, lite, SQLite, table)
	if err != nil {
		return nil, nil, fmt.Errorf("sqlite: %w", err)
	}
	pc, err := Columns(ctx, pg, Postgres, table)
	if err != nil {
		return nil, nil, fmt.Errorf("postgres: %w", err)
	}
	inLite := map[string]bool{}
	for _, c := range lc {
		inLite[c] = true
	}
	var shared, late []string
	for _, c := range pc {
		switch {
		case inLite[c]:
			shared = append(shared, c)
			delete(inLite, c)
		case lateCols != nil && lateCols[c] != "":
			late = append(late, c)
		default:
			return nil, nil, fmt.Errorf("columns differ: sqlite has [%s], postgres has [%s]", strings.Join(lc, " "), strings.Join(pc, " "))
		}
	}
	if len(inLite) > 0 {
		return nil, nil, fmt.Errorf("columns differ: sqlite has [%s], postgres has [%s]", strings.Join(lc, " "), strings.Join(pc, " "))
	}
	return shared, late, nil
}

// checkLateDefaults verifies every imported row holds the declared default in
// each late column the SQLite file lacked.
func checkLateDefaults(ctx context.Context, st Store, pg sqlx.QueryerContext, late map[string][]string) error {
	for t, cols := range late {
		d, _, err := dumpColumns(ctx, pg, t, cols)
		if err != nil {
			return fmt.Errorf("%s.%s: late columns: %w", st.Name, t, err)
		}
		lines := strings.Split(strings.TrimSuffix(string(d), "\n"), "\n")[1:]
		want := make([]string, len(cols))
		for i, c := range cols {
			want[i] = st.LateColumns[t][c]
		}
		for _, l := range lines {
			if l != strings.Join(want, "\t") {
				return fmt.Errorf("%s.%s: %w: late column holds %q, want the default %q", st.Name, t, ErrMismatch, l, strings.Join(want, "\t"))
			}
		}
	}
	return nil
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
// orderBy order. Values pass through as the driver returns them (int64, string
// or nil); anything else stops the copy, because version 1 has no other type.
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
	return compareColumns(ctx, st, lite, pg, nil)
}

// compareColumns is compare limited, for the tables named in only, to the
// given columns (the ones an import copied from an older checkpoint).
func compareColumns(ctx context.Context, st Store, lite, pg sqlx.QueryerContext, only map[string][]string) ([]TableReport, error) {
	var reports []TableReport
	bad := []string{}
	for _, t := range st.Tables {
		dump := func(q sqlx.QueryerContext, e Engine) ([]byte, int, error) {
			if cols, ok := only[t]; ok {
				return dumpColumns(ctx, q, t, cols)
			}
			return Dump(ctx, q, e, t)
		}
		ld, ln, err := dump(lite, SQLite)
		if err != nil {
			return reports, fmt.Errorf("%s.%s: sqlite dump: %w", st.Name, t, err)
		}
		pd, pn, err := dump(pg, Postgres)
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
	return dumpColumns(ctx, q, table, cols)
}

// dumpColumns is Dump over the given columns, in the given order.
func dumpColumns(ctx context.Context, q sqlx.QueryerContext, table string, cols []string) ([]byte, int, error) {
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

// canonical renders one value. Version 1 holds only integers, text and NULL.
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
	case string:
		return strconv.Quote(x), nil
	default:
		return "", fmt.Errorf("unexpected value type %T (version 1 holds only integers, text and NULL)", v)
	}
}
