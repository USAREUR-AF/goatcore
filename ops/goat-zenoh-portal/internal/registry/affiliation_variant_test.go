package registry_test

// The grant affiliation is stamped onto every machine the custodian enrolls
// and fleet views key on it, so a second spelling of the same unit forks the
// registry. First spelling wins; a later value that folds to it — case,
// spacing, or separator style — is refused, naming the spelling on file.
// Exact matches and genuinely new units pass.

import (
	"errors"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/pgtest"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

func affRegistry(t *testing.T) *registry.Registry {
	t.Helper()
	r, err := registry.Open(pgtest.DSN(t))
	if err != nil {
		t.Fatalf("open: %v", err)
	}
	t.Cleanup(func() { _ = r.Close() })
	return r
}

func TestAffiliationExactRespellingAndNewUnit(t *testing.T) {
	r := affRegistry(t)
	ctx := t.Context()

	if err := r.SetMachineGrant(ctx, "dana-ops", 3, "unit-alpha", "fra", "operator"); err != nil {
		t.Fatalf("seed grant: %v", err)
	}

	// Exact spelling for another custodian: same unit, no fork, accepted.
	if err := r.SetMachineGrant(ctx, "morgan-ops", 2, "unit-alpha", "fra", "operator"); err != nil {
		t.Fatalf("exact respelling refused: %v", err)
	}

	// A genuinely new unit is accepted as written.
	if err := r.SetMachineGrant(ctx, "sam-ops", 1, "unit-bravo", "", "operator"); err != nil {
		t.Fatalf("new unit refused: %v", err)
	}
}

func TestAffiliationVariantRefused(t *testing.T) {
	r := affRegistry(t)
	ctx := t.Context()

	if err := r.SetMachineGrant(ctx, "dana-ops", 3, "unit-alpha", "fra", "operator"); err != nil {
		t.Fatalf("seed grant: %v", err)
	}

	for _, variant := range []string{"UNIT-ALPHA", "Unit-Alpha", "uNIT-alpha"} {
		err := r.SetMachineGrant(ctx, "morgan-ops", 2, variant, "", "operator")
		if !errors.Is(err, registry.ErrAffiliationVariant) {
			t.Fatalf("variant %q: want ErrAffiliationVariant, got %v", variant, err)
		}
	}

	// Inner-whitespace variants of a spaced unit name are the same fork.
	if err := r.SetMachineGrant(ctx, "sam-ops", 1, "1st Unit", "", "operator"); err != nil {
		t.Fatalf("seed spaced unit: %v", err)
	}
	err := r.SetMachineGrant(ctx, "morgan-ops", 2, "1st  Unit", "", "operator")
	if !errors.Is(err, registry.ErrAffiliationVariant) {
		t.Fatalf("inner-whitespace variant: want ErrAffiliationVariant, got %v", err)
	}

	// The variant writes must not have replaced or forked anything: the seeded
	// grant still carries the original spelling, and morgan-ops has no grant.
	g, err := r.MachineGrantFor(ctx, "dana-ops")
	if err != nil {
		t.Fatalf("MachineGrantFor: %v", err)
	}
	if g.Affiliation != "unit-alpha" {
		t.Fatalf("seeded affiliation changed: %q", g.Affiliation)
	}
	if _, err := r.MachineGrantFor(ctx, "morgan-ops"); !errors.Is(err, registry.ErrNotFound) {
		t.Fatalf("refused grant landed anyway: %v", err)
	}
}

// Hyphen-versus-space is the commonest respelling of a unit name, so the fold
// compares on letters and digits only: separator style and trailing
// punctuation fold away too.
func TestAffiliationSeparatorVariantsRefused(t *testing.T) {
	r := affRegistry(t)
	ctx := t.Context()

	if err := r.SetMachineGrant(ctx, "dana-ops", 3, "1st Unit", "", "operator"); err != nil {
		t.Fatalf("seed grant: %v", err)
	}

	for _, variant := range []string{"1st-Unit", "1st_Unit", "1stUnit", "1st Unit.", "1ST.UNIT"} {
		err := r.SetMachineGrant(ctx, "morgan-ops", 2, variant, "", "operator")
		if !errors.Is(err, registry.ErrAffiliationVariant) {
			t.Fatalf("separator variant %q: want ErrAffiliationVariant, got %v", variant, err)
		}
	}
	if _, err := r.MachineGrantFor(ctx, "morgan-ops"); !errors.Is(err, registry.ErrNotFound) {
		t.Fatalf("refused grant landed anyway: %v", err)
	}
}

// A custodian re-sending a case variant of the unit already on their OWN grant
// is refused like anyone else, but the message names their spelling instead of
// sending them to `goat machine ls` to look up what it just told them.
func TestAffiliationVariantOfOwnGrantNamesTheSpelling(t *testing.T) {
	r := affRegistry(t)
	ctx := t.Context()

	if err := r.SetMachineGrant(ctx, "dana-ops", 3, "unit-alpha", "fra", "operator"); err != nil {
		t.Fatalf("seed grant: %v", err)
	}

	err := r.SetMachineGrant(ctx, "dana-ops", 4, "Unit-Alpha", "fra", "operator")
	if !errors.Is(err, registry.ErrAffiliationVariant) {
		t.Fatalf("own-unit variant: want ErrAffiliationVariant, got %v", err)
	}
	if !strings.Contains(err.Error(), `"unit-alpha"`) {
		t.Fatalf("refusal does not name the spelling on file: %v", err)
	}
	if strings.Contains(err.Error(), "goat machine ls") {
		t.Fatalf("refusal sends the custodian to look up their own grant: %v", err)
	}

	// Somebody else's spelling still gets the lookup hint.
	err = r.SetMachineGrant(ctx, "morgan-ops", 2, "Unit-Alpha", "", "operator")
	if !errors.Is(err, registry.ErrAffiliationVariant) {
		t.Fatalf("other-custodian variant: want ErrAffiliationVariant, got %v", err)
	}
	if !strings.Contains(err.Error(), "goat machine ls") {
		t.Fatalf("refusal drops the lookup hint for another custodian's unit: %v", err)
	}
}

// The fold keeps every letter, not just ASCII ones: dropping diacritics would
// make two different accented unit names fold together and refuse the second.
func TestAffiliationFoldKeepsDiacritics(t *testing.T) {
	r := affRegistry(t)
	ctx := t.Context()

	if err := r.SetMachineGrant(ctx, "dana-ops", 3, "Forțele Aeriene Demonstrație", "", "operator"); err != nil {
		t.Fatalf("seed grant: %v", err)
	}

	// A respelling of the same unit is still caught.
	err := r.SetMachineGrant(ctx, "morgan-ops", 2, "forțele-aeriene-demonstrație", "", "operator")
	if !errors.Is(err, registry.ErrAffiliationVariant) {
		t.Fatalf("separator variant of an accented name: want ErrAffiliationVariant, got %v", err)
	}

	// A different unit that shares its unaccented consonants is NOT.
	if err := r.SetMachineGrant(ctx, "sam-ops", 1, "Forțele Navale Demonstrație", "", "operator"); err != nil {
		t.Fatalf("distinct accented unit refused: %v", err)
	}
}

// The custody retro-stamp (PUT /api/slots/{anchor}/custody) writes the same
// column through RecordMachineEnrollment with a caller-stated affiliation, so
// the guard runs there too: a variant spelling is refused and the slot is left
// as it was, the exact spelling on file is accepted, and a later re-stamp that
// respells the slot's own affiliation is refused the same way.
func TestAffiliationVariantRefusedOnMachineStamp(t *testing.T) {
	r := affRegistry(t)
	ctx := t.Context()

	if _, err := r.InitializeSlotPool(ctx, "galaxy-test", 4); err != nil {
		t.Fatalf("init slot pool: %v", err)
	}
	if err := r.SetMachineGrant(ctx, "dana-ops", 3, "unit-alpha", "fra", "operator"); err != nil {
		t.Fatalf("seed grant: %v", err)
	}
	s, err := r.AllocateSlot(ctx, "dana-1")
	if err != nil {
		t.Fatalf("AllocateSlot: %v", err)
	}

	err = r.RecordMachineEnrollment(ctx, s.SlotID, "dana-ops", "SENSOR", "manual", "Unit-Alpha", nil)
	if !errors.Is(err, registry.ErrAffiliationVariant) {
		t.Fatalf("variant stamp: want ErrAffiliationVariant, got %v", err)
	}
	if _, err := r.MachineByAnchor(ctx, s.SlotID); !errors.Is(err, registry.ErrNotFound) {
		t.Fatalf("refused stamp landed anyway: %v", err)
	}

	if err := r.RecordMachineEnrollment(ctx, s.SlotID, "dana-ops", "SENSOR", "manual", "unit-alpha", nil); err != nil {
		t.Fatalf("exact spelling refused: %v", err)
	}

	err = r.RecordMachineEnrollment(ctx, s.SlotID, "dana-ops", "SENSOR", "supervised", "UNIT ALPHA", nil)
	if !errors.Is(err, registry.ErrAffiliationVariant) {
		t.Fatalf("re-stamp variant: want ErrAffiliationVariant, got %v", err)
	}
	m, err := r.MachineByAnchor(ctx, s.SlotID)
	if err != nil {
		t.Fatalf("MachineByAnchor: %v", err)
	}
	if m.Affiliation == nil || *m.Affiliation != "unit-alpha" {
		t.Fatalf("refused re-stamp changed the slot: %v", m.Affiliation)
	}
}

func TestAffiliationTrimmedOnWrite(t *testing.T) {
	r := affRegistry(t)
	ctx := t.Context()

	// Outer whitespace on a NEW unit is dropped before storing, so the first
	// spelling on file is already clean.
	if err := r.SetMachineGrant(ctx, "dana-ops", 3, "  unit-alpha  ", "", "operator"); err != nil {
		t.Fatalf("seed grant: %v", err)
	}
	g, err := r.MachineGrantFor(ctx, "dana-ops")
	if err != nil {
		t.Fatalf("MachineGrantFor: %v", err)
	}
	if g.Affiliation != "unit-alpha" {
		t.Fatalf("affiliation not trimmed on write: %q", g.Affiliation)
	}
}
