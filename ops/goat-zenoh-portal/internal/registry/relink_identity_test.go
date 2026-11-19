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

// GHSA-rr3p reviews r4 F1 / r5 F1: whether a relink is a hand-over is decided
// by the slot's live leaves, not by the identity on file, so a withdrawal
// followed by a new stamp cannot skip it.
func TestRelinkSlotIdentity_handsOverALiveLeaf(t *testing.T) {
	ctx := context.Background()
	r := open(t)
	if _, err := r.InitializeSlotPool(ctx, "relink-test", 2); err != nil {
		t.Fatal(err)
	}
	s, err := r.AllocateSlot(ctx, "holder")
	if err != nil {
		t.Fatal(err)
	}
	// Late linkage: a leaf minted before any identity was on file becomes A's.
	recordLeaf(t, r, s.SlotID)
	if rl, err := r.RelinkSlotIdentity(ctx, s.SlotID, idA); err != nil || rl.HandedOver {
		t.Fatalf("late linkage = (%+v, %v), want no hand-over", rl, err)
	}
	if l := mustLeaves(t, r, s.SlotID); len(l) != 1 || l[0].IssuedForIdentity != idA {
		t.Fatalf("late linkage did not attribute the leaf: %+v", l)
	}
	if rl, _ := r.RelinkSlotIdentity(ctx, s.SlotID, idA); rl.HandedOver {
		t.Fatal("re-stamping the same identity was a hand-over")
	}
	gen := slotGeneration(t, r, s.SlotID)
	if _, err := r.DB().ExecContext(ctx, `UPDATE slots SET collected_at = 5, cert_minted_at = 5 WHERE slot_id = $1`, s.SlotID); err != nil {
		t.Fatal(err)
	}
	// Withdrawing A while A's leaf is live is itself a hand-over.
	rl, err := r.RelinkSlotIdentity(ctx, s.SlotID, "")
	if err != nil || !rl.HandedOver || rl.PreviousHolder != idA || rl.GenerationTo != gen+1 {
		t.Fatalf("withdrawal with A's live leaf = (%+v, %v), want a hand-over from A to generation %d", rl, err, gen+1)
	}
	for _, l := range mustLeaves(t, r, s.SlotID) {
		if l.SupersededAt == nil {
			t.Fatalf("A's leaf %+v not superseded by the hand-over", l)
		}
	}
	// B is stamped next: no live leaf now, so nothing more to cut.
	if rl, err := r.RelinkSlotIdentity(ctx, s.SlotID, idB); err != nil || rl.HandedOver {
		t.Fatalf("stamping B after the hand-over = (%+v, %v)", rl, err)
	}
	sl, err := r.SlotForIdentity(ctx, idB)
	if err != nil || sl.CollectedAt != nil || sl.CertMintedAt != nil {
		t.Fatalf("B's slot = %+v, %v; want collectable (stamps cleared)", sl, err)
	}
}

// GHSA-rr3p review r5 F2: a hand-over ends what the previous holder had as
// the anchor, as an offboard does: the portal key, the machine grant, custody
// of its machines, the topic budget and producer data grants.
func TestRelinkSlotIdentity_handOverEndsThePreviousHoldersAllowances(t *testing.T) {
	ctx := context.Background()
	r := open(t)
	if _, err := r.InitializeSlotPool(ctx, "relink-teardown", 3); err != nil {
		t.Fatal(err)
	}
	s, err := r.AllocateSlotAs(ctx, "holder-a", idA)
	if err != nil {
		t.Fatal(err)
	}
	if err := r.StampSlotIdentity(ctx, s.SlotID, idA); err != nil {
		t.Fatal(err)
	}
	now := time.Now().Unix()
	approver := "op"
	if err := r.CreateVendor(ctx, &registry.Vendor{Prefix: s.SlotID, APIKeyHash: "h", APIKeySHA256: "d", DisplayName: "a", ApprovedAt: &now, Approver: &approver}); err != nil {
		t.Fatal(err)
	}
	recordLeaf(t, r, s.SlotID)
	if err := r.SetMachineQuota(ctx, s.SlotID, 3, "op"); err != nil {
		t.Fatal(err)
	}
	machine := enrollSlot(t, r, ctx, "a-m1", s.SlotID, "SENSOR", "manual", "unit-a")
	if err := r.UpsertTopicBudget(ctx, s.SlotID, 100, nil, "t", "op"); err != nil {
		t.Fatal(err)
	}
	g := &registry.Grant{GranteeKind: "org", GranteeID: "a-reader", Producer: s.SlotID, GrantedBy: "a", Justification: "t"}
	if err := r.CreateGrant(ctx, g); err != nil {
		t.Fatal(err)
	}

	rl, err := r.RelinkSlotIdentity(ctx, s.SlotID, idB)
	if err != nil || !rl.HandedOver {
		t.Fatalf("relink = (%+v, %v), want a hand-over", rl, err)
	}
	// The machine's own leaf is not ended: the response must name it.
	if len(rl.MachinesStillLive) != 1 || rl.MachinesStillLive[0] != machine {
		t.Fatalf("MachinesStillLive = %v, want [%s]", rl.MachinesStillLive, machine)
	}
	v, err := r.GetVendor(ctx, s.SlotID)
	if err != nil || v.APIKeyHash != "" || v.APIKeySHA256 != "" {
		t.Fatalf("A's api key survived the hand-over: %+v, %v", v, err)
	}
	if q, err := r.MachineQuotaFor(ctx, s.SlotID); err == nil && q.Quota != 0 {
		t.Fatalf("machine grant inherited by B: %+v", q)
	}
	var custody *string
	if err := r.DB().GetContext(ctx, &custody, `SELECT enrolled_by FROM slots WHERE slot_id = $1`, machine); err != nil || custody != nil {
		t.Fatalf("A's machine still in the anchor's custody: %v, %v", custody, err)
	}
	if bs, _ := r.ListTopicBudgets(ctx); len(bs) != 0 {
		t.Fatalf("topic budget inherited: %+v", bs)
	}
	if got, _ := r.GetGrant(ctx, g.ID); got == nil || got.Status != registry.GrantRevoked {
		t.Fatalf("A's producer grant still %+v", got)
	}
}

// GHSA-rr3p review r5 F3: a leaf minted for A is refused (and recorded as
// superseded) when the slot was relinked to B while the mint ran.
func TestRecordSlotLeaf_refusesALeafForAnotherIdentity(t *testing.T) {
	ctx := context.Background()
	r := open(t)
	if _, err := r.InitializeSlotPool(ctx, "relink-race", 1); err != nil {
		t.Fatal(err)
	}
	s, err := r.AllocateSlotAs(ctx, "holder", idA)
	if err != nil {
		t.Fatal(err)
	}
	if err := r.StampSlotIdentity(ctx, s.SlotID, idA); err != nil {
		t.Fatal(err)
	}
	// No leaf yet, but the link moves away from A: a hand-over all the same (r6 F1/F2).
	if rl, err := r.RelinkSlotIdentity(ctx, s.SlotID, idB); err != nil || !rl.HandedOver || rl.PreviousIdentity != idA {
		t.Fatalf("relink before any leaf = (%+v, %v), want a hand-over from A", rl, err)
	}
	err = r.RecordSlotLeaf(ctx, registry.SlotLeaf{SlotID: s.SlotID, Generation: slotGeneration(t, r, s.SlotID),
		IssuedForIdentity: idA, Serial: "aa", NotAfter: time.Now().Add(time.Hour).Unix()})
	if !errors.Is(err, registry.ErrStaleGeneration) {
		t.Fatalf("RecordSlotLeaf for A on B's slot: err = %v, want refusal", err)
	}
	if l := mustLeaves(t, r, s.SlotID); len(l) != 1 || l[0].SupersededAt == nil {
		t.Fatalf("the refused leaf must be on file and superseded: %+v", l)
	}
}

// GHSA-rr3p review r4 F2: both backfills carry the machine custodian, so a
// pre-fix machine's quarantined slot counts against that custodian's quota.
func TestBackfills_carryMachineCustodian(t *testing.T) {
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
	cust := registry.SlotAnchor("cust", 0)
	m0, m1 := registry.SlotAnchor("cust", 1), registry.SlotAnchor("cust", 2)
	recent := time.Now().Add(-24 * time.Hour).Unix()
	for _, q := range []struct {
		sql  string
		args []any
	}{
		{`INSERT INTO slots (slot_id, idx, assigned_to, assigned_at) VALUES ($1, 0, 'custodian', $2)`, []any{cust, recent}},
		// a pre-fix machine, revoked: enrolled_by survives the revoke
		{`INSERT INTO slots (slot_id, idx, enrolled_by, cert_minted_at, revoked_at) VALUES ($1, 1, $2, $3, $3)`, []any{m0, cust, recent}},
		// a pre-fix machine still enrolled, whose tarball is on disk
		{`INSERT INTO slots (slot_id, idx, assigned_to, assigned_at, enrolled_by) VALUES ($1, 2, 'm1', $2, $3)`, []any{m1, recent, cust}},
		{`INSERT INTO machine_grants (custodian, quota, affiliation, granted_by, granted_at, updated_at) VALUES ($1, 2, 'unit', 'op', $2, $2)`, []any{cust, recent}},
	} {
		if _, err := db.ExecContext(ctx, q.sql, q.args...); err != nil {
			t.Fatal(err)
		}
	}
	r, err := registry.Open(dsn)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = r.Close() })
	l0, _ := r.SlotLeaves(ctx, m0)
	if len(l0) != 1 || l0[0].IssuedForCustodian != cust {
		t.Fatalf("migration backfill for a revoked machine = %+v, want custodian %s", l0, cust)
	}
	if n, err := r.RecordBundleLeaves(ctx, []registry.BundleLeaf{{SlotID: m1, Serial: "aa", NotBefore: recent, NotAfter: recent + 365*86400}}); err != nil || n != 1 {
		t.Fatalf("RecordBundleLeaves = %d, %v", n, err)
	}
	for _, l := range mustLeaves(t, r, m1) {
		if l.Serial == "aa" && l.IssuedForCustodian != cust {
			t.Fatalf("bundle backfill custodian = %q, want %s", l.IssuedForCustodian, cust)
		}
	}
	q, err := r.MachineQuotaFor(ctx, cust)
	if err != nil || q.Used != 1 || q.Quarantined != 1 || q.Remaining != 0 {
		t.Fatalf("quota = %+v (%v), want used 1 (m1), quarantined 1 (m0), remaining 0", q, err)
	}
}

func mustLeaves(t *testing.T, r *registry.Registry, slot string) []registry.SlotLeaf {
	t.Helper()
	l, err := r.SlotLeaves(context.Background(), slot)
	if err != nil {
		t.Fatal(err)
	}
	return l
}

// GHSA-rr3p review r6 F3 (the reviewer's collect-tail ordering): a hand-over
// landing after A's leaf is recorded but before collect stamps the slot. The
// conditional stamps collect uses refuse, so B's slot stays collectable.
func TestStampSlotAt_refusesAfterAHandOver(t *testing.T) {
	ctx := context.Background()
	r := open(t)
	if _, err := r.InitializeSlotPool(ctx, "stamp-at", 2); err != nil {
		t.Fatal(err)
	}
	s, _ := r.AllocateSlot(ctx, "holder")
	if err := r.StampSlotIdentity(ctx, s.SlotID, idA); err != nil {
		t.Fatal(err)
	}
	gen, err := r.BumpSlotGeneration(ctx, s.SlotID)
	if err != nil {
		t.Fatal(err)
	}
	now := time.Now()
	if err := r.RecordSlotLeaf(ctx, registry.SlotLeaf{SlotID: s.SlotID, Generation: gen, Serial: "a1", IssuedForIdentity: idA,
		NotBefore: now.Unix(), NotAfter: now.Add(365 * 24 * time.Hour).Unix()}); err != nil {
		t.Fatal(err)
	}
	if rl, err := r.RelinkSlotIdentity(ctx, s.SlotID, idB); err != nil || !rl.HandedOver {
		t.Fatalf("expected a hand-over: %+v %v", rl, err)
	}
	if err := r.StampSlotCertAt(ctx, s.SlotID, "tokA", "/tmp/a.tgz", gen, idA); !errors.Is(err, registry.ErrStaleGeneration) {
		t.Fatalf("StampSlotCertAt after the hand-over: %v, want ErrStaleGeneration", err)
	}
	if err := r.StampSlotCollectedAt(ctx, s.SlotID, gen, idA); !errors.Is(err, registry.ErrStaleGeneration) {
		t.Fatalf("StampSlotCollectedAt after the hand-over: %v, want ErrStaleGeneration", err)
	}
	b, err := r.SlotForIdentity(ctx, idB)
	if err != nil || b.CertMintedAt != nil || b.CollectedAt != nil {
		t.Fatalf("B's slot after the stale stamps = %+v, %v; want collectable", b, err)
	}
	// The same calls for the current generation and identity succeed.
	if err := r.StampSlotCertAt(ctx, s.SlotID, "tokB", "/tmp/b.tgz", b.Generation, idB); err != nil {
		t.Fatalf("StampSlotCertAt for B: %v", err)
	}
}

// GHSA-rr3p review r6 F2 (the reviewer's rotate ordering): a relink between a
// rotation's generation bump and its mint, when no leaf is live, still ends
// the previous holder's allowances.
func TestRelink_betweenRotateBumpAndMint_isAHandOver(t *testing.T) {
	ctx := context.Background()
	r := open(t)
	if _, err := r.InitializeSlotPool(ctx, "relink-rot", 4); err != nil {
		t.Fatal(err)
	}
	p, _ := r.AllocateSlot(ctx, "person-a")
	if err := r.StampSlotIdentity(ctx, p.SlotID, idA); err != nil {
		t.Fatal(err)
	}
	recordLeaf(t, r, p.SlotID)
	if err := r.SetMachineQuota(ctx, p.SlotID, 3, "op"); err != nil {
		t.Fatal(err)
	}
	m := enrollSlot(t, r, ctx, "a-m1", p.SlotID, "SENSOR", "manual", "unit-a")
	g := &registry.Grant{GranteeKind: "org", GranteeID: "a-reader", Producer: p.SlotID, GrantedBy: "person-a", Justification: "t"}
	if err := r.CreateGrant(ctx, g); err != nil {
		t.Fatal(err)
	}
	if _, err := r.BumpSlotGeneration(ctx, p.SlotID); err != nil { // rotate's bump: no leaf live now
		t.Fatal(err)
	}
	rl, err := r.RelinkSlotIdentity(ctx, p.SlotID, idB)
	if err != nil || !rl.HandedOver || rl.PreviousIdentity != idA {
		t.Fatalf("relink mid-rotate = (%+v, %v), want a hand-over from A", rl, err)
	}
	var custody *string
	_ = r.DB().GetContext(ctx, &custody, `SELECT enrolled_by FROM slots WHERE slot_id = $1`, m)
	gg, _ := r.GetGrant(ctx, g.ID)
	q, _ := r.MachineQuotaFor(ctx, p.SlotID)
	if custody != nil || gg.Status != registry.GrantRevoked || q.Quota != 0 {
		t.Fatalf("A's allowances survived: custody=%v grant=%s quota=%+v", custody, gg.Status, q)
	}
}

// GHSA-rr3p review r6 F1: a slot linked before the upgrade gets its backfilled
// leaf attributed to the identity on file, and relinking it away is a
// hand-over (here by the identity on file alone).
func TestPrepatchLinkedSlot_backfillAttributesAndRelinkHandsOver(t *testing.T) {
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
	a := registry.SlotAnchor("prepatch", 0)
	free := registry.SlotAnchor("prepatch", 1)
	ts := time.Now().Add(-24 * time.Hour).Unix()
	if _, err := db.ExecContext(ctx, `INSERT INTO slots (slot_id, idx, assigned_to, assigned_at, identity_id, cert_minted_at, collected_at) VALUES ($1, 0, 'a', $2, $3, $2, $2)`, a, ts, idA); err != nil {
		t.Fatal(err)
	}
	if _, err := db.ExecContext(ctx, `INSERT INTO slots (slot_id, idx, identity_id, revoked_at) VALUES ($1, 1, $2, $3)`, free, idB, ts); err != nil {
		t.Fatal(err)
	}
	r, err := registry.Open(dsn)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = r.Close() })
	if l := mustLeaves(t, r, a); len(l) != 1 || l[0].IssuedForIdentity != idA {
		t.Fatalf("assigned pre-fix slot backfill = %+v, want issued for A", l)
	}
	if l := mustLeaves(t, r, free); len(l) != 1 || l[0].IssuedForIdentity != "" {
		t.Fatalf("a free slot's backfill must not be attributed: %+v", l)
	}
	rl, err := r.RelinkSlotIdentity(ctx, a, idB)
	if err != nil || !rl.HandedOver || rl.PreviousHolder != idA || rl.GenerationTo != 1 {
		t.Fatalf("relink of a pre-fix linked slot = (%+v, %v), want a hand-over from A to generation 1", rl, err)
	}
}
