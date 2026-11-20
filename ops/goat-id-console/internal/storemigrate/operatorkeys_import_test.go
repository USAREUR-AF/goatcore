package storemigrate

import (
	"errors"
	"path/filepath"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/pgtest"
)

// GHSA-2w7v review r4, finding 1 (fail-to-pass). A checkpoint the pre-Postgres
// console wrote (its schema, no filer recorded anywhere) imports, and a
// request still pending in it cannot be decided by anyone after the import:
// one principal could otherwise approve it alone, revoke it and tombstone an
// arbitrary handle.
func TestImport_pendingOperatorKeyFromSQLiteCannotBeDecided(t *testing.T) {
	ctx := t.Context()
	src := filepath.Join(t.TempDir(), "onboard.db")
	dsn := pgtest.DSN(t)
	writeSQLite(ctx, t, src, Onboard.SQLiteSchema,
		`INSERT INTO operator_keys (id, created_at, handle, email, env, status) VALUES ('opk-pre', 1, 'breakglass', 'x@example.org', 'zagros', 'requested')`)
	if _, err := Import(ctx, Onboard, src, dsn); err != nil {
		t.Fatalf("import of a pre-Postgres checkpoint: %v", err)
	}
	s, err := onboardstore.Open(dsn)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = s.Close() })
	_, err = s.ApproveOperatorKey(ctx, "opk-pre", "onboard-runner", []string{"onboard-runner"})
	if !errors.Is(err, onboardstore.ErrOperatorKeyLegacyRequest) {
		t.Fatalf("approve of an imported pending request: err = %v, want ErrOperatorKeyLegacyRequest", err)
	}
}
