package registry_test

// Three properties of the spelling guard that the sequential tests in
// affiliation_variant_test.go cannot pin: the check and the write share one
// transaction, so racing first grants cannot both land; the fold compares
// Unicode text, not byte sequences; and a refusal names one deterministic
// spelling when legacy rows already fork.

import (
	"errors"
	"fmt"
	"strings"
	"sync"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

// The guard exists to stop two concurrent first grants for the same new unit
// from both passing the check and both landing. Every spelling below folds to
// the same unit; started together, exactly one may land and the rest must be
// refused as variants of it. The registry runs on one SQLite connection, so
// with the check inside the write's transaction the outcome is deterministic;
// a check issued outside the transaction lets several land.
func TestAffiliationRacingFirstGrantsLandExactlyOne(t *testing.T) {
	r := affRegistry(t)
	ctx := t.Context()

	spellings := []string{
		"Unit-Alpha", "unit-alpha", "UNIT ALPHA", "unit_alpha",
		"UnitAlpha", "unit alpha", "Unit_Alpha", "UNIT-ALPHA",
	}
	custodians := make([]string, len(spellings))
	for i := range spellings {
		custodians[i] = fmt.Sprintf("cust-%d", i)
	}

	start := make(chan struct{})
	errs := make([]error, len(spellings))
	var wg sync.WaitGroup
	for i, spelling := range spellings {
		wg.Add(1)
		go func(i int, spelling string) {
			defer wg.Done()
			<-start
			errs[i] = r.SetMachineGrant(ctx, custodians[i], 1, spelling, "", "operator")
		}(i, spelling)
	}
	close(start)
	wg.Wait()

	landed := 0
	for i, err := range errs {
		switch {
		case err == nil:
			landed++
		case errors.Is(err, registry.ErrAffiliationVariant):
		default:
			t.Fatalf("grant %q: unexpected error %v", spellings[i], err)
		}
	}
	if landed != 1 {
		t.Fatalf("want exactly one racing grant to land, got %d (errs %v)", landed, errs)
	}

	onFile := map[string]bool{}
	for _, custodian := range custodians {
		g, err := r.MachineGrantFor(ctx, custodian)
		if errors.Is(err, registry.ErrNotFound) {
			continue
		}
		if err != nil {
			t.Fatalf("MachineGrantFor(%q): %v", custodian, err)
		}
		onFile[g.Affiliation] = true
	}
	if len(onFile) != 1 {
		t.Fatalf("want one spelling on file, got %v", onFile)
	}
}

// Unicode lets the same accented letter be written as one code point or as a
// base letter plus a combining mark. Both render identically, so they are one
// spelling of one unit: the fold must compare them equal, and the write must
// store one form so a later exact re-send is not refused for a difference the
// operator cannot see.
func TestAffiliationComposedAndDecomposedFormsAreOneSpelling(t *testing.T) {
	r := affRegistry(t)
	ctx := t.Context()

	// Spelled out as escapes so the two forms cannot be confused by an editor
	// that normalises what it saves.
	const composed = "For\u021bele Aeriene Rom\u00e2ne"            // U+021B, U+00E2
	const decomposed = "Fort\u0326ele Aeriene Roma\u0302ne"        // t + U+0326, a + U+0302
	const decomposedVariant = "fort\u0326ele-aeriene-roma\u0302ne" // same unit, respelled

	// Seeded in the decomposed form (what a paste from a macOS file name
	// carries), stored in the composed form.
	if err := r.SetMachineGrant(ctx, "dana-ops", 3, decomposed, "", "operator"); err != nil {
		t.Fatalf("seed grant: %v", err)
	}
	g, err := r.MachineGrantFor(ctx, "dana-ops")
	if err != nil {
		t.Fatalf("MachineGrantFor: %v", err)
	}
	if g.Affiliation != composed {
		t.Fatalf("stored affiliation is not the composed form: %q", g.Affiliation)
	}

	// The composed spelling of the same text is the same spelling: accepted.
	if err := r.SetMachineGrant(ctx, "morgan-ops", 2, composed, "", "operator"); err != nil {
		t.Fatalf("composed form of the on-file spelling refused: %v", err)
	}

	// A respelling in the other form is still a variant of the unit on file.
	err = r.SetMachineGrant(ctx, "sam-ops", 1, decomposedVariant, "", "operator")
	if !errors.Is(err, registry.ErrAffiliationVariant) {
		t.Fatalf("decomposed respelling: want ErrAffiliationVariant, got %v", err)
	}

	// The stamp path stores the same form.
	if _, err := r.InitializeSlotPool(ctx, "galaxy-test", 4); err != nil {
		t.Fatalf("init slot pool: %v", err)
	}
	s, err := r.AllocateSlot(ctx, "dana-1")
	if err != nil {
		t.Fatalf("AllocateSlot: %v", err)
	}
	if err := r.RecordMachineEnrollment(ctx, s.SlotID, "dana-ops", "SENSOR", "manual", decomposed, nil); err != nil {
		t.Fatalf("stamp in the decomposed form refused: %v", err)
	}
	m, err := r.MachineByAnchor(ctx, s.SlotID)
	if err != nil {
		t.Fatalf("MachineByAnchor: %v", err)
	}
	if m.Affiliation == nil || *m.Affiliation != composed {
		t.Fatalf("stamped affiliation is not the composed form: %v", m.Affiliation)
	}
}

// A row written before writes were composed can still hold the decomposed
// form. The fold must compose it too, or the combining mark folds away as a
// non-letter and the composed spelling of the same unit passes as new.
func TestAffiliationLegacyDecomposedRowStillGuards(t *testing.T) {
	r := affRegistry(t)
	ctx := t.Context()

	const composed = "For\u021bele Aeriene Rom\u00e2ne"     // U+021B, U+00E2
	const decomposed = "Fort\u0326ele Aeriene Roma\u0302ne" // t + U+0326, a + U+0302

	if _, err := r.DB().ExecContext(ctx,
		`INSERT INTO machine_grants (custodian, quota, affiliation, site, granted_by, granted_at, updated_at)
		 VALUES ('legacy-ops', 1, $1, '', 'operator', 1, 1)`, decomposed); err != nil {
		t.Fatalf("seed legacy grant: %v", err)
	}

	// Same text, different bytes: refused, naming the row on file, and the
	// composed respelling is a variant of it too. Neither may land.
	for _, spelling := range []string{composed, "for\u021bele-aeriene-rom\u00e2ne"} {
		err := r.SetMachineGrant(ctx, "morgan-ops", 2, spelling, "", "operator")
		if !errors.Is(err, registry.ErrAffiliationVariant) {
			t.Fatalf("%q against a legacy decomposed row: want ErrAffiliationVariant, got %v", spelling, err)
		}
	}
	if _, err := r.MachineGrantFor(ctx, "morgan-ops"); !errors.Is(err, registry.ErrNotFound) {
		t.Fatalf("refused grant landed anyway: %v", err)
	}
}

// Rows written before the guard can already carry two spellings of one unit.
// A refusal then names the spelling that sorts first by byte value, so the
// same input always gets the same answer whichever order the rows were
// written in.
func TestAffiliationRefusalNamesTheFirstSpellingByByteOrder(t *testing.T) {
	r := affRegistry(t)
	ctx := t.Context()

	// Seed the fork directly; the guard would refuse the second row.
	db := r.DB()
	for _, row := range []struct{ custodian, affiliation string }{
		{"legacy-b", "unit-alpha"},
		{"legacy-a", "Unit-Alpha"},
	} {
		if _, err := db.ExecContext(ctx,
			`INSERT INTO machine_grants (custodian, quota, affiliation, site, granted_by, granted_at, updated_at)
			 VALUES ($1, 1, $2, '', 'operator', 1, 1)`, row.custodian, row.affiliation); err != nil {
			t.Fatalf("seed legacy grant %q: %v", row.affiliation, err)
		}
	}

	err := r.SetMachineGrant(ctx, "morgan-ops", 2, "UNIT ALPHA", "", "operator")
	if !errors.Is(err, registry.ErrAffiliationVariant) {
		t.Fatalf("want ErrAffiliationVariant, got %v", err)
	}
	if !strings.Contains(err.Error(), `"Unit-Alpha"`) {
		t.Fatalf("refusal does not name the byte-order-first spelling: %v", err)
	}
}
