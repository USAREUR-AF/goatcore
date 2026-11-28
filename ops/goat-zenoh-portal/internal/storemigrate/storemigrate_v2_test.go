package storemigrate

import (
	"path/filepath"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/pgtest"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

// The copy is version 1 only. Import leaves the database at version 1 so the
// round trip still holds; once the portal opens it, registry migration 2 (slot
// generations and slot_leaves, GHSA-rr3p) applies, and an export to the
// SQLite build is refused rather than silently dropping slot_leaves and
// slots.generation (and, from migration 3, the carriage tables).
func TestImportStopsAtVersion1AndExportRefusesVersion2(t *testing.T) {
	ctx := t.Context()
	dir := t.TempDir()
	src := filepath.Join(dir, "registry.db")
	writeSQLite(ctx, t, src, registryFixture...)
	dsn := pgtest.DSN(t)
	if _, _, err := Import(ctx, Registry, src, dsn); err != nil {
		t.Fatalf("import: %v", err)
	}
	r, err := registry.Open(dsn)
	if err != nil {
		t.Fatalf("open after import: %v", err)
	}
	_ = r.Close()
	_, err = Export(ctx, Registry, dsn, filepath.Join(dir, "back.db"))
	if err == nil || !strings.Contains(err.Error(), "copies version 1 only") {
		t.Fatalf("export of a migrated registry: err = %v, want a version refusal", err)
	}
}
