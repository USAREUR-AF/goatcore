package registry_test

import (
	"context"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

// Migration step 3 finds the generation-0 slots whose index had an earlier
// holder, advances them once, and routes each holder to a new leaf.
func TestReissue_findsAndAdvancesReissuedSlots(t *testing.T) {
	ctx := context.Background()
	r := open(t)
	if _, err := r.InitializeSlotPool(ctx, "reissue-test", 4); err != nil {
		t.Fatal(err)
	}
	a := func(k int) string { return registry.SlotAnchor("reissue-test", k) }
	past := time.Now().Add(-48 * time.Hour).Unix()
	now := time.Now().Unix()
	db := r.DB()
	// Pre-fix state, written directly: every slot at generation 0.
	exec := func(q string, args ...any) {
		t.Helper()
		if _, err := db.ExecContext(ctx, q, args...); err != nil {
			t.Fatal(err)
		}
	}
	// slot 0: a person, offboarded predecessor (vendor-delete before assigned_at), identity linked.
	exec(`UPDATE slots SET assigned_to='p0', assigned_at=$1, identity_id=$2, cert_minted_at=$1, collected_at=$1 WHERE slot_id=$3`, now, idA, a(0))
	exec(`INSERT INTO audit_events (occurred_at, actor, action, target, details) VALUES ($1,'op','vendor-delete',$2,'{}')`, past, a(0))
	// slot 1: a machine whose index was enrolled before (anchor in details).
	exec(`UPDATE slots SET assigned_to='m1', assigned_at=$1, enrolled_by='cust' WHERE slot_id=$2`, now, a(1))
	exec(`INSERT INTO audit_events (occurred_at, actor, action, target, details) VALUES ($1,'op','machine-enroll','old-m',$2)`, past, `{"anchor":"`+a(1)+`","custodian":"x"}`)
	// slot 2: assigned, no earlier holder on record.
	exec(`UPDATE slots SET assigned_to='p2', assigned_at=$1 WHERE slot_id=$2`, now, a(2))
	exec(`INSERT INTO audit_events (occurred_at, actor, action, target, details) VALUES ($1,'op','slot-onboard','p2',$2)`, now+1, `{"anchor":"`+a(2)+`"}`)
	// slot 3: free, with history: not a candidate (nobody to reissue to).
	exec(`INSERT INTO audit_events (occurred_at, actor, action, target, details) VALUES ($1,'op','vendor-delete',$2,'{}')`, past, a(3))

	cands, err := r.ReissueCandidates(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if len(cands) != 2 || cands[0].SlotID != a(0) || cands[1].SlotID != a(1) {
		t.Fatalf("candidates = %+v, want slots 0 and 1", cands)
	}
	if cands[0].Path() != registry.ReissueRecollect || cands[1].Path() != registry.ReissueRotate {
		t.Fatalf("paths = %s, %s; want recollect, rotate", cands[0].Path(), cands[1].Path())
	}
	for _, c := range cands {
		ok, err := r.ApplyReissue(ctx, "test", c)
		if err != nil || !ok {
			t.Fatalf("ApplyReissue(%s) = %v, %v", c.SlotID, ok, err)
		}
	}
	if g := slotGeneration(t, r, a(0)); g != 1 {
		t.Fatalf("slot 0 generation = %d, want 1", g)
	}
	var collected *int64
	if err := db.GetContext(ctx, &collected, `SELECT collected_at FROM slots WHERE slot_id=$1`, a(0)); err != nil || collected != nil {
		t.Fatalf("slot 0 must be collectable again: collected_at=%v err=%v", collected, err)
	}
	var n int
	if err := db.GetContext(ctx, &n, `SELECT COUNT(*) FROM audit_events WHERE action='rr3p-reissue'`); err != nil || n != 2 {
		t.Fatalf("rr3p-reissue audit rows = %d (%v), want 2", n, err)
	}
	// Idempotent: a second run finds nothing, and re-applying a stale candidate is a no-op.
	if again, _ := r.ReissueCandidates(ctx); len(again) != 0 {
		t.Fatalf("second listing = %+v, want none", again)
	}
	if ok, err := r.ApplyReissue(ctx, "test", cands[0]); err != nil || ok {
		t.Fatalf("re-apply = %v, %v; want skipped", ok, err)
	}
	if g := slotGeneration(t, r, a(0)); g != 1 {
		t.Fatalf("re-apply moved generation to %d", g)
	}
	if bs, _ := r.SlotBindings(ctx); bs[0].CN != a(0)+".g1" {
		t.Fatalf("binding after reissue = %+v", bs[0])
	}
}

// A rotation during the current assignment, before generations, re-minted the
// bare anchor and left the earlier leaf valid: the slot is a candidate too.
// A rotation before the current assignment says nothing about this holder.
func TestReissue_preFixRotationIsACandidate(t *testing.T) {
	ctx := context.Background()
	r := open(t)
	if _, err := r.InitializeSlotPool(ctx, "reissue-rot", 2); err != nil {
		t.Fatal(err)
	}
	a := func(k int) string { return registry.SlotAnchor("reissue-rot", k) }
	assigned := time.Now().Add(-24 * time.Hour).Unix()
	for _, q := range []struct {
		sql  string
		args []any
	}{
		{`UPDATE slots SET assigned_to='p0', assigned_at=$1, identity_id=$2 WHERE slot_id=$3`, []any{assigned, idA, a(0)}},
		{`UPDATE slots SET assigned_to='p1', assigned_at=$1 WHERE slot_id=$2`, []any{assigned, a(1)}},
		{`INSERT INTO audit_events (occurred_at, actor, action, target, details) VALUES ($1,'op','vendor-rotate',$2,'{}')`, []any{assigned + 60, a(0)}},
		{`INSERT INTO audit_events (occurred_at, actor, action, target, details) VALUES ($1,'op','vendor-rotate',$2,'{}')`, []any{assigned - 60, a(1)}},
	} {
		if _, err := r.DB().ExecContext(ctx, q.sql, q.args...); err != nil {
			t.Fatal(err)
		}
	}
	cands, err := r.ReissueCandidates(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if len(cands) != 1 || cands[0].SlotID != a(0) || cands[0].Path() != registry.ReissueRecollect {
		t.Fatalf("candidates = %+v, want only slot 0 (rotated during its assignment), path recollect", cands)
	}
}
