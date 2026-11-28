package acl

import (
	"os"
	"path/filepath"
	"testing"
)

// The router role's render tests build slot fragments with a Python twin of
// RenderSlotFragment (roles/goat-zenoh/tests/slot-fragment/slotfragment.py).
// Both are pinned to the same golden bytes, so the role is tested against the
// format the portal actually writes.
func TestRenderSlotFragment_matchesRoleGolden(t *testing.T) {
	golden, err := os.ReadFile(filepath.Join("..", "..", "..", "ansible", "roles", "goat-zenoh", "tests", "slot-fragment", "golden.json5"))
	if err != nil {
		t.Fatalf("read role golden: %v", err)
	}
	got, _, err := RenderSlotFragment([]SlotBinding{
		{Anchor: anchorA, CN: anchorA + ".g3", Copub: []CopubSubject{{Key: "1c342d414d73458d", Live: true}, {Key: "09e1cd2ed2bce389"}}},
		{Anchor: anchorB},
		{Anchor: anchorC, CN: anchorC, Carry: []string{anchorA + ".g3", "11112222333344445555666677778888.g1"}},
	})
	if err != nil {
		t.Fatal(err)
	}
	if got != string(golden) {
		t.Fatalf("portal fragment differs from the role golden\nportal:\n%s\ngolden:\n%s", got, golden)
	}
}
