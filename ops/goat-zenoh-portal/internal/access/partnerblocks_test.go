package access

import (
	"os"
	"path/filepath"
	"testing"
)

func writeBlocks(t *testing.T, body string) string {
	t.Helper()
	p := filepath.Join(t.TempDir(), "partner-slot-blocks.yaml")
	if err := os.WriteFile(p, []byte(body), 0o644); err != nil {
		t.Fatal(err)
	}
	return p
}

// A representative deployed shape.
const galaxyProdBlocks = `version: 1
partner_slot_blocks:
  - lanes: ["REL-ALL"]
    from: 900
    to: 929
  - lanes: ["REL-ALL", "REL-PARTNER"]
    from: 930
    to: 949
`

func TestLoadPartnerSlotBlocks_Galaxy(t *testing.T) {
	got, err := LoadPartnerSlotBlocks(writeBlocks(t, galaxyProdBlocks))
	if err != nil {
		t.Fatalf("load: %v", err)
	}
	if len(got) != 2 {
		t.Fatalf("want 2 blocks, got %d", len(got))
	}
	// Lanes are normalized + sorted so set-equality matching is order-insensitive.
	if got[1].Lanes[0] != "REL-ALL" || got[1].Lanes[1] != "REL-PARTNER" {
		t.Fatalf("lanes not normalized/sorted: %v", got[1].Lanes)
	}
}

// A missing file must be an ERROR, never an empty list. An empty list would be
// read by the caller as "this goatnet declares no blocks", routing every partner
// mint to the ordinary lowest-free-slot path — the over-sharing bug.
func TestLoadPartnerSlotBlocks_MissingFileIsAnError(t *testing.T) {
	if _, err := LoadPartnerSlotBlocks(filepath.Join(t.TempDir(), "nope.yaml")); err == nil {
		t.Fatal("a missing block file must fail closed, not return an empty list")
	}
}

func TestLoadPartnerSlotBlocks_RejectsOverlap(t *testing.T) {
	_, err := LoadPartnerSlotBlocks(writeBlocks(t, `version: 1
partner_slot_blocks:
  - lanes: ["REL-ALL"]
    from: 900
    to: 940
  - lanes: ["REL-ALL", "REL-PARTNER"]
    from: 930
    to: 949
`))
	if err == nil {
		t.Fatal("overlapping blocks must be refused: a slot in two blocks has an order-dependent lane-set")
	}
}

func TestLoadPartnerSlotBlocks_RejectsInvertedAndLaneless(t *testing.T) {
	if _, err := LoadPartnerSlotBlocks(writeBlocks(t, `version: 1
partner_slot_blocks:
  - lanes: ["REL-ALL"]
    from: 949
    to: 930
`)); err == nil {
		t.Fatal("inverted range must be refused")
	}
	if _, err := LoadPartnerSlotBlocks(writeBlocks(t, `version: 1
partner_slot_blocks:
  - lanes: []
    from: 900
    to: 929
`)); err == nil {
		t.Fatal("a block with no lanes must be refused — it would strand its occupants")
	}
}

func TestMatchPartnerBlock(t *testing.T) {
	blocks, err := LoadPartnerSlotBlocks(writeBlocks(t, galaxyProdBlocks))
	if err != nil {
		t.Fatal(err)
	}

	// Order-insensitive set equality.
	b, ok := MatchPartnerBlock(blocks, []string{"REL-PARTNER", "rel-all"})
	if !ok {
		t.Fatal("lane-set match must be order-insensitive and case-insensitive")
	}
	if b.From != 930 || b.To != 949 {
		t.Fatalf("matched the wrong block: [%d,%d]", b.From, b.To)
	}

	// A SUBSET must NOT match a wider block. REL-ALL alone is a different
	// standing from REL-ALL+REL-PARTNER; silently upgrading it would hand a partner
	// a bilateral lane nobody granted.
	b, ok = MatchPartnerBlock(blocks, []string{"REL-ALL"})
	if !ok || b.From != 900 || b.To != 929 {
		t.Fatalf("REL-ALL alone must match its own block, got ok=%v [%d,%d]", ok, b.From, b.To)
	}

	// An undeclared lane-set must NOT match anything.
	if _, ok := MatchPartnerBlock(blocks, []string{"REL-ALL", "REL-POL"}); ok {
		t.Fatal("an undeclared lane-set must not match — the caller has to refuse the mint")
	}
	if _, ok := MatchPartnerBlock(blocks, nil); ok {
		t.Fatal("an empty lane-set must not match")
	}
}
