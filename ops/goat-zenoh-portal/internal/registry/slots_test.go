package registry_test

import (
	"context"
	"errors"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

// TestSlotAnchorParity locks the portal's anchor formula to the router's.
// These expected values are the exact anchors the live isolated-router proof
// (ops/ansible/roles/goat-zenoh/tests/isolation-proof.sh, salt "isoproof")
// enforced — slot 0 was admitted, slot 1 was denied. If this test passes, a
// portal-minted cert with CN=SlotAnchor(salt,k) matches the router rule the
// template bakes for that slot.
func TestSlotAnchorParity(t *testing.T) {
	want := map[int]string{
		0: "61dc1e84ca2c7cfea827728a0f4bfa4d",
		1: "b9160d80380bfa367191ec34f3505de0",
	}
	for k, w := range want {
		if got := registry.SlotAnchor("isoproof", k); got != w {
			t.Errorf("SlotAnchor(isoproof,%d) = %q, want %q (router-enforced value)", k, got, w)
		}
	}
}

func TestSlotPoolLifecycle(t *testing.T) {
	ctx := context.Background()
	r := open(t)

	n, err := r.InitializeSlotPool(ctx, "galaxy-test", 4)
	if err != nil || n != 4 {
		t.Fatalf("InitializeSlotPool = (%d,%v), want (4,nil)", n, err)
	}
	// idempotent
	if n2, err := r.InitializeSlotPool(ctx, "galaxy-test", 4); err != nil || n2 != 0 {
		t.Fatalf("re-init = (%d,%v), want (0,nil) — must be idempotent", n2, err)
	}

	// lowest-idx free slot is idx 0 with the deterministic anchor
	free, err := r.GetFreeSlot(ctx)
	if err != nil {
		t.Fatalf("GetFreeSlot: %v", err)
	}
	if free.Idx != 0 || free.SlotID != registry.SlotAnchor("galaxy-test", 0) {
		t.Fatalf("free slot = idx %d id %s, want idx 0 / %s", free.Idx, free.SlotID, registry.SlotAnchor("galaxy-test", 0))
	}

	// allocate in idx order, distinct anchors
	seen := map[string]bool{}
	for i, sys := range []string{"sys-a", "sys-b", "sys-c", "sys-d"} {
		s, err := r.AllocateSlot(ctx, sys)
		if err != nil {
			t.Fatalf("AllocateSlot(%s): %v", sys, err)
		}
		if int(s.Idx) != i {
			t.Errorf("AllocateSlot(%s) idx=%d, want %d (lowest free)", sys, s.Idx, i)
		}
		if s.AssignedTo == nil || *s.AssignedTo != sys {
			t.Errorf("AllocateSlot(%s) assigned_to=%v", sys, s.AssignedTo)
		}
		if seen[s.SlotID] {
			t.Errorf("anchor %s reused across allocations", s.SlotID)
		}
		seen[s.SlotID] = true
	}

	// pool exhausted
	if _, err := r.AllocateSlot(ctx, "sys-e"); !errors.Is(err, registry.ErrSlotPoolExhausted) {
		t.Fatalf("5th allocate err = %v, want ErrSlotPoolExhausted", err)
	}

	// revoke frees the slot back to the pool, lowest-idx first
	if err := r.RevokeSlot(ctx, registry.SlotAnchor("galaxy-test", 1)); err != nil {
		t.Fatalf("RevokeSlot: %v", err)
	}
	free2, err := r.GetFreeSlot(ctx)
	if err != nil || free2.Idx != 1 {
		t.Fatalf("after revoke, GetFreeSlot = (%v,%v), want idx 1 free", free2, err)
	}
	if _, err := r.AllocateSlot(ctx, "sys-e2"); err != nil {
		t.Fatalf("re-allocate freed slot: %v", err)
	}

	// unknown slot id errors
	if err := r.RevokeSlot(ctx, "deadbeef"); err == nil {
		t.Error("RevokeSlot(unknown) = nil, want error")
	}
}
