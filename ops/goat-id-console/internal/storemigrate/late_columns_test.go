package storemigrate

import (
	"path/filepath"
	"regexp"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/pgtest"
)

// prePatchOnboardSchema is the onboarding SQLite schema as a checkpoint
// written before any late column existed: without the two GHSA-3m4r added
// (offboard_requested_by_identity, offboard_operator_audit) and the three of
// the departure record (ADR 1272).
func prePatchOnboardSchema(t *testing.T) string {
	t.Helper()
	s := Onboard.SQLiteSchema
	for _, col := range []string{"offboard_requested_by_identity", "offboard_operator_audit",
		"offboard_reason_class", "offboard_reason_text", "offboard_effective"} {
		re := regexp.MustCompile(`(?m)^\s*` + col + ` [^\n]*\n`)
		stripped := re.ReplaceAllString(s, "")
		if stripped == s || strings.Contains(stripped, col+" TEXT") {
			t.Fatalf("could not strip %s from the schema", col)
		}
		s = stripped
	}
	for col := range Onboard.LateColumns["onboard_proposals"] {
		if strings.Contains(s, col+" ") {
			t.Fatalf("pre-patch schema still has late column %s", col)
		}
	}
	return s
}

// A SQLite checkpoint written before a late column existed still imports: the
// column takes its Postgres default and every other column compares equal.
// Before, Import refused it ("columns differ ... offboard_requested_by_identity")
// and nothing could add the column to the old file.
func TestImportAcceptsACheckpointMissingALateColumn(t *testing.T) {
	ctx := t.Context()
	src := filepath.Join(t.TempDir(), "onboard.db")
	writeSQLite(ctx, t, src, append([]string{prePatchOnboardSchema(t)}, onboardFixture...)...)
	reports, err := Import(ctx, Onboard, src, pgtest.DSN(t))
	if err != nil {
		t.Fatalf("import of a pre-patch checkpoint: %v", err)
	}
	for _, r := range reports {
		if !r.OK() || r.SQLiteRows != r.PGRows {
			t.Fatalf("import %s: %+v", r.Table, r)
		}
	}
}

// Only the declared late columns are tolerated: any other missing column is
// still the column-drift refusal.
func TestImportStillRefusesAnUndeclaredMissingColumn(t *testing.T) {
	ctx := t.Context()
	re := regexp.MustCompile(`(?m)^\s*restored_by [^\n]*\n`)
	schema := re.ReplaceAllString(Onboard.SQLiteSchema, "")
	var fixture []string
	for _, f := range onboardFixture {
		if !strings.Contains(f, "restored_by") {
			fixture = append(fixture, f)
		}
	}
	src := filepath.Join(t.TempDir(), "onboard.db")
	writeSQLite(ctx, t, src, append([]string{schema}, fixture...)...)
	_, err := Import(ctx, Onboard, src, pgtest.DSN(t))
	if err == nil || !strings.Contains(err.Error(), "columns differ") {
		t.Fatalf("import without restored_by: err = %v, want the column-drift refusal", err)
	}
}
