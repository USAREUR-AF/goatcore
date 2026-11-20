package onboardstore

import (
	"context"
	"errors"
	"testing"
)

func TestOperatorKeyHolder_roundTrip(t *testing.T) {
	ctx := context.Background()
	s := opStore(t)
	row, err := s.RequestOperatorKey(ctx, &OperatorKey{Handle: "Op-Alice", Email: "alice@example.test", Env: "zagros"})
	if err != nil {
		t.Fatal(err)
	}
	if h, err := s.OperatorKeyHolderOf(ctx, row.ID); err != nil || h.HolderIdentityID != "" {
		t.Fatalf("new row holder = %+v %v, want empty", h, err)
	}
	if err := s.SetOperatorKeyHolder(ctx, row.ID, " id-1 "); err != nil {
		t.Fatal(err)
	}
	hs, err := s.OperatorKeyHoldersForHandle(ctx, "op-alice")
	if err != nil || len(hs) != 1 || hs[0].HolderIdentityID != "id-1" {
		t.Fatalf("holders for handle = %+v %v", hs, err)
	}
	if err := s.SetOperatorKeyHolder(ctx, "no-such", "x"); !errors.Is(err, ErrNotFound) {
		t.Fatalf("unknown row: %v", err)
	}
	if _, err := s.OperatorKeyHolderOf(ctx, "no-such"); !errors.Is(err, ErrNotFound) {
		t.Fatalf("unknown row read: %v", err)
	}
}

func TestProposalsUnderHandle(t *testing.T) {
	ctx := context.Background()
	s := opStore(t)
	for i, st := range []string{ProposalOnboarded, ProposalProposed, ProposalRejected} {
		if err := s.CreateProposal(ctx, &Proposal{Sponsor: "x", Env: "zagros", Handle: "dup", Email: string(rune('a'+i)) + "@example.test",
			Status: st, ManifestJSON: "{}"}); err != nil {
			t.Fatal(err)
		}
	}
	ps, err := s.ProposalsUnderHandle(ctx, "DUP")
	if err != nil || len(ps) != 3 {
		t.Fatalf("ProposalsUnderHandle = %d %v, want 3", len(ps), err)
	}
	if !ProposalPending(ProposalProposed) || ProposalPending(ProposalOnboarded) || ProposalNonTerminal(ProposalRejected) || !ProposalNonTerminal(ProposalArchived) {
		t.Fatal("status predicates wrong")
	}
}

// UncollectedOperatorKeys reads an explicit column list, so the holder
// column (not on the OperatorKey struct) does not break it.
func TestUncollectedOperatorKeys_withHolderColumn(t *testing.T) {
	ctx := context.Background()
	s := opStore(t)
	row, err := s.RequestOperatorKey(ctx, &OperatorKey{Handle: "uncollected", Email: "u@example.test", Env: "zagros"})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := s.ApproveOperatorKey(ctx, row.ID, "approver@example.test", []string{"approver@example.test", "approver"}); err != nil {
		t.Fatal(err)
	}
	if err := s.SetOperatorKeyHolder(ctx, row.ID, "id-u"); err != nil {
		t.Fatal(err)
	}
	ks, err := s.UncollectedOperatorKeys(ctx, 1<<62)
	if err != nil || len(ks) != 1 || ks[0].ID != row.ID {
		t.Fatalf("UncollectedOperatorKeys = %d %v", len(ks), err)
	}
}
