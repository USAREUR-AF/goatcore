package registry_test

import (
	"context"
	"errors"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

func TestValidAlias(t *testing.T) {
	ok := []string{"jane-operator", "acme", "fra-land-bde", "a1", "x", "node-7"}
	for _, a := range ok {
		if err := registry.ValidAlias(a); err != nil {
			t.Errorf("ValidAlias(%q) = %v, want nil", a, err)
		}
	}
	bad := map[string]string{
		"e7a9c75b532e3d434bb09fefd3ce1ec7": "32-hex anchor shape must be rejected (shape-disjoint from UUID)",
		"ops":                              "reserved namespace word",
		"volatile":                         "reserved namespace word",
		"release":                          "reserved namespace word",
		"community":                        "reserved namespace word",
		"Jane-Operator":                    "uppercase not allowed",
		"acme/x":                           "slash not allowed",
		"-acme":                            "leading dash not allowed",
		"acme-":                            "trailing dash not allowed",
		"":                                 "empty not allowed",
		"this-handle-is-far-too-long-to-be-valid": "over 32 chars not allowed",
	}
	for a, why := range bad {
		if err := registry.ValidAlias(a); err == nil {
			t.Errorf("ValidAlias(%q) = nil, want error (%s)", a, why)
		}
	}
}

func TestAllocateSlotAliasUniqueness(t *testing.T) {
	ctx := context.Background()
	r := open(t)
	if _, err := r.InitializeSlotPool(ctx, "galaxy-test", 8); err != nil {
		t.Fatalf("init pool: %v", err)
	}

	first, err := r.AllocateSlot(ctx, "acme")
	if err != nil {
		t.Fatalf("first AllocateSlot(acme): %v", err)
	}
	// Same handle to a second slot must be refused.
	if _, err := r.AllocateSlot(ctx, "acme"); !errors.Is(err, registry.ErrAliasTaken) {
		t.Fatalf("duplicate AllocateSlot(acme) = %v, want ErrAliasTaken", err)
	}
	// A different handle is fine, and gets a distinct anchor.
	second, err := r.AllocateSlot(ctx, "acme-2")
	if err != nil {
		t.Fatalf("AllocateSlot(acme-2): %v", err)
	}
	if second.SlotID == first.SlotID {
		t.Fatalf("distinct handles got the same anchor %q", first.SlotID)
	}
	// Revoking frees the alias for re-use.
	if err := r.RevokeSlot(ctx, first.SlotID); err != nil {
		t.Fatalf("revoke: %v", err)
	}
	if _, err := r.AllocateSlot(ctx, "acme"); err != nil {
		t.Fatalf("re-AllocateSlot(acme) after revoke = %v, want nil (revoked alias is free)", err)
	}
}

func TestListAliases(t *testing.T) {
	ctx := context.Background()
	r := open(t)
	// Per-vendor goatnet (no slots) → empty, non-nil.
	if as, err := r.ListAliases(ctx); err != nil || as == nil || len(as) != 0 {
		t.Fatalf("ListAliases (no pool) = (%v,%v), want ([],nil)", as, err)
	}
	if _, err := r.InitializeSlotPool(ctx, "galaxy-test", 8); err != nil {
		t.Fatalf("init pool: %v", err)
	}
	for _, h := range []string{"bravo", "alpha"} {
		if _, err := r.AllocateSlot(ctx, h); err != nil {
			t.Fatalf("AllocateSlot(%q): %v", h, err)
		}
	}
	as, err := r.ListAliases(ctx)
	if err != nil {
		t.Fatalf("ListAliases: %v", err)
	}
	if len(as) != 2 {
		t.Fatalf("ListAliases len = %d, want 2", len(as))
	}
	// Sorted by alias: alpha before bravo; each slot_id is the 32-hex anchor.
	if as[0].Alias != "alpha" || as[1].Alias != "bravo" {
		t.Fatalf("ListAliases order = [%q,%q], want [alpha,bravo]", as[0].Alias, as[1].Alias)
	}
	for _, a := range as {
		if len(a.SlotID) != 32 {
			t.Errorf("alias %q slot_id %q is not a 32-char anchor", a.Alias, a.SlotID)
		}
	}
}
