package registry

import (
	"context"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/pgtest"
)

func openProbReg(t *testing.T) *Registry {
	t.Helper()
	r, err := Open(pgtest.DSN(t))
	if err != nil {
		t.Fatalf("open: %v", err)
	}
	t.Cleanup(func() { _ = r.Close() })
	return r
}

func TestProbationHoldListDecide(t *testing.T) {
	r := openProbReg(t)
	ctx := context.Background()

	p := &Probation{Ref: "kid-1", Label: "oliver@mil.example", Reason: "stray:untracked", HeldBy: "jane-operator"}
	if err := r.HoldProbation(ctx, p); err != nil {
		t.Fatalf("hold: %v", err)
	}
	if p.ID == "" || p.Disposition != ProbationOpen || p.Kind != ProbationKindIdentity {
		t.Fatalf("defaults not applied: %+v", p)
	}
	if p.CureBy <= p.HeldAt {
		t.Errorf("cure_by (%d) should be after held_at (%d) via the default window", p.CureBy, p.HeldAt)
	}

	// Double-hold on the same open ref is refused.
	if err := r.HoldProbation(ctx, &Probation{Ref: "kid-1", HeldBy: "op"}); err != ErrProbationOpen {
		t.Errorf("double-hold: want ErrProbationOpen, got %v", err)
	}

	// Open list has the one entry.
	open, err := r.ListProbation(ctx, true)
	if err != nil || len(open) != 1 {
		t.Fatalf("list open: len=%d err=%v", len(open), err)
	}

	// Decide → repaired; it leaves the open list, and the ref can be held again.
	if err := r.DecideProbation(ctx, p.ID, ProbationRepaired, "jane-operator", "onboarded"); err != nil {
		t.Fatalf("decide: %v", err)
	}
	if open, _ := r.ListProbation(ctx, true); len(open) != 0 {
		t.Errorf("repaired entry should leave the open list, got %d", len(open))
	}
	if all, _ := r.ListProbation(ctx, false); len(all) != 1 {
		t.Errorf("history should retain the decided row, got %d", len(all))
	}
	// Re-deciding a decided entry is a no-op (guarded on open disposition).
	if err := r.DecideProbation(ctx, p.ID, ProbationPurged, "op", ""); err != ErrNotFound {
		t.Errorf("re-decide: want ErrNotFound, got %v", err)
	}
	// The ref reconciled, so a fresh hold on the same ref is allowed again.
	if err := r.HoldProbation(ctx, &Probation{Ref: "kid-1", HeldBy: "op"}); err != nil {
		t.Errorf("re-hold after decide should be allowed, got %v", err)
	}
}

func TestProbationExpiryFlags(t *testing.T) {
	r := openProbReg(t)
	ctx := context.Background()

	// cure_by in the past ⇒ Expired=true (flagged), but still OPEN — the
	// deadline never auto-decides (ADR 1068 invariant).
	past := time.Now().Add(-time.Hour).Unix()
	p := &Probation{Ref: "kid-expired", HeldBy: "op", CureBy: past}
	if err := r.HoldProbation(ctx, p); err != nil {
		t.Fatal(err)
	}
	got, err := r.GetProbation(ctx, p.ID)
	if err != nil {
		t.Fatal(err)
	}
	if !got.Expired {
		t.Error("past cure_by should mark Expired")
	}
	if got.Disposition != ProbationOpen {
		t.Error("expired entry must remain OPEN — no auto-decision")
	}
}

func TestProbationRejectsBadInput(t *testing.T) {
	r := openProbReg(t)
	ctx := context.Background()
	if err := r.HoldProbation(ctx, &Probation{HeldBy: "op"}); err == nil {
		t.Error("empty ref should be rejected")
	}
	if err := r.HoldProbation(ctx, &Probation{Ref: "x"}); err == nil {
		t.Error("empty held_by should be rejected")
	}
	if err := r.HoldProbation(ctx, &Probation{Ref: "x", HeldBy: "op", Kind: "bogus"}); err == nil {
		t.Error("invalid kind should be rejected")
	}
	if err := r.DecideProbation(ctx, "prob-x", "sideways", "op", ""); err == nil {
		t.Error("invalid disposition should be rejected")
	}
}
