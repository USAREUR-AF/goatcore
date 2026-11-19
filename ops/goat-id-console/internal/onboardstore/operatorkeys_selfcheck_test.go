package onboardstore

import (
	"context"
	"errors"
	"testing"
)

// The two-person rule holds against second spellings of the requester's
// address, not only a different letter case: a plus tag, a lookalike
// character or a zero-width space is still the requester.
func TestOperatorKeyDecisions_refuseSelfDecisionUnderSecondSpelling(t *testing.T) {
	ctx := context.Background()
	s := opStore(t)
	row, err := s.RequestOperatorKey(ctx, &OperatorKey{
		Handle: "owner-handle", Email: "owner@example.test", Env: "zagros"})
	if err != nil {
		t.Fatal(err)
	}
	for _, decider := range []string{
		"owner+approver@example.test",
		"оwner@example.test", // Cyrillic o
		"own\u200ber@Example.Test",
		"оwner-handle", // the handle, with a lookalike
	} {
		if _, err := s.ApproveOperatorKey(ctx, row.ID, decider, []string{decider}); !errors.Is(err, ErrOperatorKeySelfDecision) {
			t.Errorf("approve as %q = %v, want ErrOperatorKeySelfDecision", decider, err)
		}
	}
	approved, err := s.ApproveOperatorKey(ctx, row.ID, "other@example.test", []string{"other@example.test", "other-handle"})
	if err != nil || approved.Status != OperatorKeyActive {
		t.Fatalf("different admin approve = %+v, %v; want active", approved, err)
	}
}
