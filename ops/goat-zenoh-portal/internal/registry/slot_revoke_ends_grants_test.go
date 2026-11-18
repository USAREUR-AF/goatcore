package registry_test

import (
	"context"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

// Freeing a person's slot ends what they held as that anchor: the machine
// grant, custody of their machines, and their producer data grants, so the
// next holder of the index inherits none of it (GHSA-fghv).
func TestRevokeAssignedSlot_endsGrantsAndCustody(t *testing.T) {
	ctx := context.Background()
	r := open(t)
	if _, err := r.InitializeSlotPool(ctx, "fghv-test", 4); err != nil {
		t.Fatal(err)
	}
	person, err := r.AllocateSlot(ctx, "person-a")
	if err != nil {
		t.Fatal(err)
	}
	if err := r.SetMachineQuota(ctx, person.SlotID, 3, "op"); err != nil {
		t.Fatal(err)
	}
	machine := enrollSlot(t, r, ctx, "person-a-m1", person.SlotID, "SENSOR", "manual", "unit-a")
	g := &registry.Grant{GranteeKind: "org", GranteeID: "reader-org", Producer: person.SlotID, GrantedBy: "person-a", Justification: "test"}
	if err := r.CreateGrant(ctx, g); err != nil {
		t.Fatal(err)
	}

	if err := r.UpsertTopicBudget(ctx, person.SlotID, 500, []string{"video"}, "test", "op"); err != nil {
		t.Fatal(err)
	}

	freed, err := r.RevokeAssignedSlot(ctx, person.SlotID)
	if err != nil || !freed {
		t.Fatalf("RevokeAssignedSlot = (%v, %v), want (true, nil)", freed, err)
	}
	if q, err := r.MachineQuotaFor(ctx, person.SlotID); err == nil && q != nil && q.Quota != 0 {
		t.Fatalf("machine grant survived the revoke: %+v", q)
	}
	var custody *string
	if err := r.DB().GetContext(ctx, &custody, `SELECT enrolled_by FROM slots WHERE slot_id = $1`, machine); err != nil {
		t.Fatal(err)
	}
	if custody != nil {
		t.Fatalf("machine %s still in the freed anchor's custody (%s)", machine, *custody)
	}
	got, err := r.GetGrant(ctx, g.ID)
	if err != nil {
		t.Fatal(err)
	}
	if got.Status != registry.GrantRevoked {
		t.Fatalf("producer grant status = %q after the revoke, want revoked", got.Status)
	}
	budgets, err := r.ListTopicBudgets(ctx)
	if err != nil {
		t.Fatal(err)
	}
	for _, b := range budgets {
		if b.Owner == person.SlotID {
			t.Fatalf("topic budget survived the revoke: %+v", b)
		}
	}
}

// A slot freed without the revoke cleanup (RevokeSlot, or before the fix)
// still hands its next holder no allowance: assignment ends them too.
func TestAllocateSlot_endsResidualAllowances(t *testing.T) {
	ctx := context.Background()
	r := open(t)
	if _, err := r.InitializeSlotPool(ctx, "fghv-alloc", 1); err != nil {
		t.Fatal(err)
	}
	s, err := r.AllocateSlot(ctx, "first")
	if err != nil {
		t.Fatal(err)
	}
	if err := r.SetMachineQuota(ctx, s.SlotID, 3, "op"); err != nil {
		t.Fatal(err)
	}
	if err := r.UpsertTopicBudget(ctx, s.SlotID, 500, nil, "test", "op"); err != nil {
		t.Fatal(err)
	}
	g := &registry.Grant{GranteeKind: "org", GranteeID: "reader-org", Producer: s.SlotID, GrantedBy: "first", Justification: "test"}
	if err := r.CreateGrant(ctx, g); err != nil {
		t.Fatal(err)
	}
	if err := r.RevokeSlot(ctx, s.SlotID); err != nil { // the cleanup-free path
		t.Fatal(err)
	}
	next, err := r.AllocateSlot(ctx, "second")
	if err != nil || next.SlotID != s.SlotID {
		t.Fatalf("re-allocate = (%+v, %v)", next, err)
	}
	if q, err := r.MachineQuotaFor(ctx, s.SlotID); err == nil && q != nil && q.Quota != 0 {
		t.Fatalf("machine grant inherited: %+v", q)
	}
	if bs, _ := r.ListTopicBudgets(ctx); len(bs) != 0 {
		t.Fatalf("topic budget inherited: %+v", bs)
	}
	if got, _ := r.GetGrant(ctx, g.ID); got == nil || got.Status != registry.GrantRevoked {
		t.Fatalf("producer grant inherited: %+v", got)
	}
}
