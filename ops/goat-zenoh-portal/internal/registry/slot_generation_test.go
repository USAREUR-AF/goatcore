package registry_test

import (
	"context"
	"errors"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/pgstore"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/pgtest"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

const (
	idA = "11111111-1111-4111-8111-111111111111"
	idB = "22222222-2222-4222-8222-222222222222"
)

func slotGeneration(t *testing.T, r *registry.Registry, slotID string) int64 {
	t.Helper()
	var g int64
	if err := r.DB().GetContext(context.Background(), &g, `SELECT generation FROM slots WHERE slot_id = $1`, slotID); err != nil {
		t.Fatal(err)
	}
	return g
}

// recordLeaf records a leaf valid for a year from now at the slot's current
// generation, as a mint would.
func recordLeaf(t *testing.T, r *registry.Registry, slotID string) {
	t.Helper()
	now := time.Now()
	if err := r.RecordSlotLeaf(context.Background(), registry.SlotLeaf{
		SlotID: slotID, Generation: slotGeneration(t, r, slotID), Serial: "AB01",
		NotBefore: now.Unix(), NotAfter: now.Add(365 * 24 * time.Hour).Unix(),
	}); err != nil {
		t.Fatalf("RecordSlotLeaf: %v", err)
	}
}

func TestLeafCN(t *testing.T) {
	a := registry.SlotAnchor("cn-test", 0)
	if got := registry.LeafCN(a, 0); got != a {
		t.Errorf("generation 0 CN = %q, want the bare anchor", got)
	}
	if got := registry.LeafCN(a, 7); got != a+".g7" {
		t.Errorf("generation 7 CN = %q, want %s.g7", got, a)
	}
	for cn, want := range map[string]struct {
		anchor string
		gen    int64
		ok     bool
	}{
		a:                            {a, 0, true},
		a + ".g12":                   {a, 12, true},
		a + ".g0":                    {"", 0, false},
		a + ".g":                     {"", 0, false},
		a + ".g01":                   {"", 0, false},
		a + "-trial-peer":            {"", 0, false},
		"operator-trial-peer":        {"", 0, false},
		"ABCDEF" + a[6:] + ".g1":     {"", 0, false},
		a + ".g1/" + "other":         {"", 0, false},
		" " + a + ".g3 ":             {a, 3, true},
		a + ".g99999999999999999999": {"", 0, false},
	} {
		anchor, gen, ok := registry.AnchorOfCN(cn)
		if anchor != want.anchor || gen != want.gen || ok != want.ok {
			t.Errorf("AnchorOfCN(%q) = (%q, %d, %v), want (%q, %d, %v)", cn, anchor, gen, ok, want.anchor, want.gen, want.ok)
		}
	}
}

// The generation advances in the same transaction as every assign, rotate
// (BumpSlotGeneration), collect (BumpSlotGeneration) and revoke, and each
// advance supersedes the slot's earlier leaves.
func TestSlotGeneration_bumpsOnEveryChange(t *testing.T) {
	ctx := context.Background()
	r := open(t)
	if _, err := r.InitializeSlotPool(ctx, "gen-test", 3); err != nil {
		t.Fatal(err)
	}
	s, err := r.AllocateSlot(ctx, "holder-a")
	if err != nil {
		t.Fatal(err)
	}
	if s.Generation != 1 || slotGeneration(t, r, s.SlotID) != 1 {
		t.Fatalf("after assign: generation %d (row %d), want 1", s.Generation, slotGeneration(t, r, s.SlotID))
	}
	recordLeaf(t, r, s.SlotID)

	g, err := r.BumpSlotGeneration(ctx, s.SlotID) // rotate / collect
	if err != nil || g != 2 {
		t.Fatalf("BumpSlotGeneration = (%d, %v), want (2, nil)", g, err)
	}
	leaves, err := r.SlotLeaves(ctx, s.SlotID)
	if err != nil || len(leaves) != 1 || leaves[0].SupersededAt == nil {
		t.Fatalf("after the bump the generation-1 leaf must be superseded: %+v, %v", leaves, err)
	}
	recordLeaf(t, r, s.SlotID)

	freed, err := r.RevokeAssignedSlot(ctx, s.SlotID)
	if err != nil || !freed {
		t.Fatalf("RevokeAssignedSlot = (%v, %v)", freed, err)
	}
	if g := slotGeneration(t, r, s.SlotID); g != 3 {
		t.Fatalf("after revoke: generation %d, want 3", g)
	}
	leaves, _ = r.SlotLeaves(ctx, s.SlotID)
	for _, l := range leaves {
		if l.SupersededAt == nil {
			t.Fatalf("leaf %+v still current after the revoke", l)
		}
	}
	// A free slot has no holder: nothing to rotate or collect for.
	if _, err := r.BumpSlotGeneration(ctx, s.SlotID); !errors.Is(err, registry.ErrNotFound) {
		t.Fatalf("BumpSlotGeneration on a free slot: err = %v, want ErrNotFound", err)
	}

	// RevokeSlot (the rollback path) advances it too.
	s2, err := r.AllocateSlot(ctx, "holder-b")
	if err != nil {
		t.Fatal(err)
	}
	before := slotGeneration(t, r, s2.SlotID)
	if err := r.RevokeSlot(ctx, s2.SlotID); err != nil {
		t.Fatal(err)
	}
	if after := slotGeneration(t, r, s2.SlotID); after != before+1 {
		t.Fatalf("RevokeSlot: generation %d -> %d, want +1", before, after)
	}
}

// A leaf minted against a generation the slot has since left is recorded (it
// exists, and keeps the slot in quarantine) but refused.
func TestRecordSlotLeaf_staleGeneration(t *testing.T) {
	ctx := context.Background()
	r := open(t)
	if _, err := r.InitializeSlotPool(ctx, "stale-test", 2); err != nil {
		t.Fatal(err)
	}
	s, err := r.AllocateSlot(ctx, "holder")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := r.BumpSlotGeneration(ctx, s.SlotID); err != nil {
		t.Fatal(err)
	}
	err = r.RecordSlotLeaf(ctx, registry.SlotLeaf{SlotID: s.SlotID, Generation: s.Generation, Serial: "01", NotAfter: time.Now().Add(time.Hour).Unix()})
	if !errors.Is(err, registry.ErrStaleGeneration) {
		t.Fatalf("RecordSlotLeaf at a left generation: err = %v, want ErrStaleGeneration", err)
	}
	leaves, _ := r.SlotLeaves(ctx, s.SlotID)
	if len(leaves) != 1 || leaves[0].SupersededAt == nil {
		t.Fatalf("the stale leaf must still be on file, superseded: %+v", leaves)
	}
}

// Allocation skips a slot while any leaf recorded for it is valid, unless the
// claimant is the identity those leaves were issued for. Once every leaf has
// expired the slot is free to anyone.
func TestAllocation_quarantinedByLiveLeaves(t *testing.T) {
	ctx := context.Background()
	r := open(t)
	if _, err := r.InitializeSlotPool(ctx, "quar-test", 3); err != nil {
		t.Fatal(err)
	}
	a, err := r.AllocateSlotAs(ctx, "person-a", idA)
	if err != nil {
		t.Fatal(err)
	}
	if err := r.StampSlotIdentity(ctx, a.SlotID, idA); err != nil {
		t.Fatal(err)
	}
	recordLeaf(t, r, a.SlotID)
	if freed, err := r.RevokeAssignedSlot(ctx, a.SlotID); err != nil || !freed {
		t.Fatalf("revoke: %v %v", freed, err)
	}

	if f, err := r.GetFreeSlot(ctx); err != nil || f.SlotID == a.SlotID {
		t.Fatalf("GetFreeSlot offers the quarantined slot: %+v %v", f, err)
	}
	b, err := r.AllocateSlotAs(ctx, "person-b", idB)
	if err != nil {
		t.Fatal(err)
	}
	if b.SlotID == a.SlotID {
		t.Fatal("a slot with a live leaf was handed to a different identity")
	}
	if _, err := r.RevokeAssignedSlot(ctx, b.SlotID); err != nil {
		t.Fatal(err)
	}
	// An unidentified claim never matches a leaf's identity.
	c, err := r.AllocateSlot(ctx, "anon")
	if err != nil {
		t.Fatal(err)
	}
	if c.SlotID == a.SlotID {
		t.Fatal("an unidentified claimant got a slot with a live leaf")
	}
	// The person the leaf was issued for gets their own slot back.
	again, err := r.AllocateSlotAs(ctx, "person-a-again", idA)
	if err != nil {
		t.Fatal(err)
	}
	if again.SlotID != a.SlotID {
		t.Fatalf("re-onboarded person got %s, want their own slot %s", again.SlotID, a.SlotID)
	}
	if again.Generation <= a.Generation {
		t.Fatalf("re-assignment kept generation %d (was %d): the old leaf's CN would match again", again.Generation, a.Generation)
	}
	if _, err := r.RevokeAssignedSlot(ctx, again.SlotID); err != nil {
		t.Fatal(err)
	}
	recordLeaf(t, r, again.SlotID) // a leaf minted around the free

	// The pool's other slots are b's (free, no leaf) and c's (held). Slot a
	// is quarantined, so a partner block of just slot a is exhausted.
	if _, err := r.AllocateSlotIn(ctx, "partner", a.Idx, a.Idx); !errors.Is(err, registry.ErrPartnerBlockExhausted) {
		t.Fatalf("block of one quarantined slot: err = %v, want ErrPartnerBlockExhausted", err)
	}

	saved := registry.SlotLeafClock
	registry.SlotLeafClock = func() time.Time { return time.Now().Add(400 * 24 * time.Hour) }
	t.Cleanup(func() { registry.SlotLeafClock = saved })
	if p, err := r.AllocateSlotIn(ctx, "partner", a.Idx, a.Idx); err != nil || p.SlotID != a.SlotID {
		t.Fatalf("after every leaf expired the slot is free: (%+v, %v)", p, err)
	}
}

// SlotBindings gives an assigned slot its current-generation CN and a free one
// none.
func TestSlotBindings(t *testing.T) {
	ctx := context.Background()
	r := open(t)
	if _, err := r.InitializeSlotPool(ctx, "bind-test", 3); err != nil {
		t.Fatal(err)
	}
	s, err := r.AllocateSlot(ctx, "holder")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := r.BumpSlotGeneration(ctx, s.SlotID); err != nil {
		t.Fatal(err)
	}
	bs, err := r.SlotBindings(ctx)
	if err != nil || len(bs) != 3 {
		t.Fatalf("SlotBindings = %d bindings, %v", len(bs), err)
	}
	for _, b := range bs {
		switch b.Anchor {
		case s.SlotID:
			if b.CN != s.SlotID+".g2" {
				t.Errorf("assigned slot CN = %q, want %s.g2", b.CN, s.SlotID)
			}
		default:
			if b.CN != "" {
				t.Errorf("free slot %s renders CN %q", b.Anchor, b.CN)
			}
		}
	}
	if _, err := r.RevokeAssignedSlot(ctx, s.SlotID); err != nil {
		t.Fatal(err)
	}
	bs, _ = r.SlotBindings(ctx)
	for _, b := range bs {
		if b.CN != "" {
			t.Errorf("after revoke slot %s renders CN %q", b.Anchor, b.CN)
		}
	}
}

// Migration 2 carries the history the slots row holds into slot_leaves, so a
// slot freed or minted before the table existed stays quarantined for a year
// and a day past the latest stamp, keyed to no identity.
func TestMigration2_backfillsUnknownLeaves(t *testing.T) {
	ctx := context.Background()
	dsn := pgtest.DSN(t)
	db, err := pgstore.Open(ctx, dsn)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = db.Close() })
	if err := pgstore.Migrate(ctx, db, registry.Migrations[:1]); err != nil {
		t.Fatal(err)
	}
	minted, freed := time.Now().Add(-30*24*time.Hour).Unix(), time.Now().Add(-10*24*time.Hour).Unix()
	old := time.Now().Add(-800 * 24 * time.Hour).Unix()
	for _, q := range []struct {
		sql  string
		args []any
	}{
		{`INSERT INTO slots (slot_id, idx, cert_minted_at, revoked_at) VALUES ($1, 0, $2, $3)`, []any{registry.SlotAnchor("m", 0), minted, freed}},
		{`INSERT INTO slots (slot_id, idx, assigned_to, assigned_at) VALUES ($1, 1, 'holder', $2)`, []any{registry.SlotAnchor("m", 1), minted}},
		{`INSERT INTO slots (slot_id, idx) VALUES ($1, 2)`, []any{registry.SlotAnchor("m", 2)}},
		{`INSERT INTO slots (slot_id, idx, cert_minted_at, revoked_at) VALUES ($1, 3, $2, $2)`, []any{registry.SlotAnchor("m", 3), old}},
	} {
		if _, err := db.ExecContext(ctx, q.sql, q.args...); err != nil {
			t.Fatal(err)
		}
	}
	r, err := registry.Open(dsn)
	if err != nil {
		t.Fatalf("open (applies migration 2): %v", err)
	}
	t.Cleanup(func() { _ = r.Close() })

	l0, _ := r.SlotLeaves(ctx, registry.SlotAnchor("m", 0))
	if len(l0) != 1 || l0[0].NotAfter != freed+366*86400 || l0[0].IssuedForIdentity != "" || l0[0].SupersededAt == nil {
		t.Fatalf("freed slot backfill = %+v, want one superseded unknown leaf to revoked_at+366d", l0)
	}
	l1, _ := r.SlotLeaves(ctx, registry.SlotAnchor("m", 1))
	if len(l1) != 1 || l1[0].NotAfter != minted+366*86400 || l1[0].SupersededAt != nil {
		t.Fatalf("assigned slot backfill = %+v, want one current unknown leaf to assigned_at+366d", l1)
	}
	if l2, _ := r.SlotLeaves(ctx, registry.SlotAnchor("m", 2)); len(l2) != 0 {
		t.Fatalf("a never-used slot got backfilled rows: %+v", l2)
	}
	if g := slotGeneration(t, r, registry.SlotAnchor("m", 1)); g != 0 {
		t.Fatalf("existing slots start at generation %d, want 0 (their live leaves have CN = anchor)", g)
	}
	// Slot 0 is quarantined; slot 3's history is long expired, slot 2 never used.
	s, err := r.AllocateSlot(ctx, "next")
	if err != nil || s.SlotID != registry.SlotAnchor("m", 2) {
		t.Fatalf("first allocation after migration = (%+v, %v), want slot 2", s, err)
	}
	s, err = r.AllocateSlot(ctx, "next2")
	if err != nil || s.SlotID != registry.SlotAnchor("m", 3) {
		t.Fatalf("second allocation after migration = (%+v, %v), want slot 3", s, err)
	}
	if _, err := r.AllocateSlot(ctx, "next3"); !errors.Is(err, registry.ErrSlotPoolExhausted) {
		t.Fatalf("slot 0 must stay quarantined: err = %v", err)
	}
}
