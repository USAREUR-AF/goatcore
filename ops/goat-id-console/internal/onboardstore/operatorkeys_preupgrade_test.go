package onboardstore

import (
	"context"
	"errors"
	"testing"
)

// GHSA-2w7v review r3/r4, finding 1 (fail-to-pass). A pending request with no
// recorded filer (filed before the console recorded filers, imported from a
// SQLite checkpoint, or left by an interrupted write) could be approved by the
// principal that filed it, which could then revoke it and tombstone an
// arbitrary handle. Nobody may decide such a row; it can only be withdrawn.
func TestOperatorKeyDecision_pendingRowWithoutFilerCannotBeDecided(t *testing.T) {
	ctx := context.Background()
	s := opStore(t)
	// The row as the previous release, or an import, leaves it: no filer.
	if _, err := s.db.ExecContext(ctx, `INSERT INTO operator_keys (id, created_at, handle, email, env, status)
		VALUES ('opk-pre', 1, 'breakglass', 'x@example.org', 'zagros', 'requested')`); err != nil {
		t.Fatal(err)
	}
	pre, err := s.OperatorKey(ctx, "opk-pre")
	if err != nil {
		t.Fatal(err)
	}
	if pre.RequestedBy != "" {
		t.Fatalf("RequestedBy = %q, want empty", pre.RequestedBy)
	}
	for _, decider := range []string{"onboard-runner", "approver"} {
		if _, err := s.ApproveOperatorKey(ctx, "opk-pre", decider, []string{decider}); !errors.Is(err, ErrOperatorKeyLegacyRequest) ||
			!errors.Is(err, ErrOperatorKeyUnverifiedDecider) {
			t.Fatalf("approve by %s of a row with no filer: err = %v, want ErrOperatorKeyLegacyRequest", decider, err)
		}
	}
	if _, err := s.RevokeOperatorKey(ctx, "opk-pre", "onboard-runner", "withdrawn"); err != nil {
		t.Fatalf("a row with no filer must still be withdrawable: %v", err)
	}
	// Filed through a door: the filer is recorded and the ordinary rule applies.
	row, err := s.RequestOperatorKey(WithOperatorKeyRequester(ctx, "onboard-runner"), &OperatorKey{Handle: "ada", Email: "ada@example.org", Env: "zagros"})
	if err != nil {
		t.Fatal(err)
	}
	if got, _ := s.OperatorKey(ctx, row.ID); got.RequestedBy != "onboard-runner" {
		t.Fatalf("RequestedBy = %q, want onboard-runner", got.RequestedBy)
	}
	if _, err := s.ApproveOperatorKey(ctx, row.ID, "onboard-runner", []string{"onboard-runner"}); !errors.Is(err, ErrOperatorKeySelfDecision) {
		t.Fatalf("self-approve: err = %v, want ErrOperatorKeySelfDecision", err)
	}
	if _, err := s.ApproveOperatorKey(ctx, row.ID, "approver", []string{"approver"}); err != nil {
		t.Fatalf("second principal approve: %v", err)
	}
}
