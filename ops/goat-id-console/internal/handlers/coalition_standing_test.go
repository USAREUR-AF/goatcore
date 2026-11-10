package handlers

import (
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/coalition"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/config"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/kratos"
)

func testCoalitionRegistry(t *testing.T) *coalition.Registry {
	t.Helper()
	path := filepath.Join("..", "..", "..", "identity", "coalition-frameworks.yaml")
	reg, err := coalition.Load(path)
	if err != nil {
		t.Skipf("canonical coalition registry unavailable: %v", err)
	}
	return reg
}

func identityWithAff(id, kind, nation string) kratos.Identity {
	aff := map[string]interface{}{}
	if id != "" {
		aff["id"] = id
	}
	if kind != "" {
		aff["kind"] = kind
	}
	if nation != "" {
		aff["nation"] = nation
	}
	return kratos.Identity{
		ID: "identity-1",
		Traits: map[string]interface{}{
			"email": "someone@example.test",
			"aff":   aff,
		},
	}
}

// The case that prompted the surface: a partner-nation org member must read as
// partner on their own page, with the reasoning attached.
func TestResolveAffiliation_partnerNationFromAffClaim(t *testing.T) {
	s := &Server{Cfg: &config.Config{}, Coalition: testCoalitionRegistry(t)}

	av := s.resolveAffiliation(identityWithAff("sky-fortress", "vendor", "UKR"))

	if !av.Known {
		t.Fatal("affiliation must be Known when the aff claim carries values")
	}
	if av.Standing != coalition.StandingPartner {
		t.Errorf("Standing = %q, want partner (basis: %s)", av.Standing, av.Basis)
	}
	if av.Nation != "UKR" {
		t.Errorf("Nation = %q, want UKR", av.Nation)
	}
	if av.Kind != "vendor" {
		t.Errorf("Kind = %q, want vendor — kind and standing are different axes", av.Kind)
	}
	if av.NationSource != "Ory aff claim" || av.KindSource != "Ory aff claim" {
		t.Errorf("sources = (%q, %q), want both from the Ory aff claim", av.KindSource, av.NationSource)
	}
	if av.Basis == "" {
		t.Error("Basis empty — the page would show a bare 'partner' with no reason")
	}
	if av.AsOf == "" {
		t.Error("AsOf empty — a derived standing must say what date it was derived for")
	}
}

func TestResolveAffiliation_memberNation(t *testing.T) {
	s := &Server{Cfg: &config.Config{}, Coalition: testCoalitionRegistry(t)}

	av := s.resolveAffiliation(identityWithAff("org123x", "partner", "SVN"))

	if av.Standing != coalition.StandingMember {
		t.Errorf("Standing = %q, want member (basis: %s)", av.Standing, av.Basis)
	}
	// The two axes are independent: kind partner, standing member. If this ever
	// collapses into one value the page starts lying about one of them.
	if av.Kind != "partner" {
		t.Errorf("Kind = %q, want partner", av.Kind)
	}
}

// No aff trait at all (the default posture — the injection is default-OFF), so
// nation has to come from the org registry. Without that fallback the row would
// read "not recorded" for the entire live population.
func TestResolveAffiliation_missingNationIsNamedNotGuessed(t *testing.T) {
	s := &Server{Cfg: &config.Config{}, Coalition: testCoalitionRegistry(t)}

	id := kratos.Identity{
		ID: "identity-2",
		Traits: map[string]interface{}{
			"email": "someone@example.test",
			"org":   map[string]interface{}{"short_tag": "acme", "display_name": "Acme"},
		},
	}
	av := s.resolveAffiliation(id)

	if !av.Known {
		t.Fatal("an org-shaped identity must render the affiliation block")
	}
	if av.Nation != "" {
		t.Errorf("Nation = %q, want empty — nothing recorded it", av.Nation)
	}
	// The rule still resolves (to partner, per the canonical default), and the
	// basis must say WHY so an operator can tell "no nation on file" from
	// "genuinely a non-member nation".
	if av.Standing != coalition.StandingPartner {
		t.Errorf("Standing = %q, want partner", av.Standing)
	}
	if !strings.Contains(av.Basis, "no nation recorded") {
		t.Errorf("Basis = %q, want it to name the missing nation as the cause", av.Basis)
	}
}

// An operator-shape identity (no org, no aff) has no affiliation to show — the
// block must stay hidden rather than render an empty scaffold.
func TestResolveAffiliation_operatorShapeHidesBlock(t *testing.T) {
	s := &Server{Cfg: &config.Config{}, Coalition: testCoalitionRegistry(t)}

	av := s.resolveAffiliation(kratos.Identity{
		ID:     "identity-3",
		Traits: map[string]interface{}{"email": "operator@example.test"},
	})

	if av.Known {
		t.Error("an operator-shape identity must not render an affiliation block")
	}
}

// A registry that failed to load must produce "unresolved" plus the reason —
// never a standing the console cannot justify.
func TestResolveAffiliation_registryMissingSaysSo(t *testing.T) {
	s := &Server{Cfg: &config.Config{}, Coalition: nil, CoalitionLoadErr: "open /etc/x: no such file"}

	av := s.resolveAffiliation(identityWithAff("sky-fortress", "vendor", "UKR"))

	if av.Standing != "" {
		t.Errorf("Standing = %q, want empty when the registry did not load", av.Standing)
	}
	if !strings.Contains(av.Unresolved, "did not load") || !strings.Contains(av.Unresolved, "no such file") {
		t.Errorf("Unresolved = %q, want it to name the load failure", av.Unresolved)
	}
}

// Standing is resolved against the goatnet's ACTIVE framework, not a hardcoded
// alliance — a goatnet in another AOR must be able to point at its own.
func TestResolveAffiliation_honorsActiveFrameworkConfig(t *testing.T) {
	s := &Server{
		Cfg:       &config.Config{CoalitionFramework: "no-such-framework"},
		Coalition: testCoalitionRegistry(t),
	}

	av := s.resolveAffiliation(identityWithAff("us-unit", "organic", "USA"))

	if av.Framework != "no-such-framework" {
		t.Errorf("Framework = %q, want the configured active framework", av.Framework)
	}
	if av.Standing != coalition.StandingPartner {
		t.Errorf("Standing = %q, want partner — a USA org is not a member of a framework that does not exist",
			av.Standing)
	}
}

// The page itself: standing, its basis, and both halves of the what-it-does /
// what-it-does-not note have to render, because the whole point of the row is
// that "partner" was previously invisible AND easy to over-read.
func TestTemplates_userDetailCoalitionRow(t *testing.T) {
	reg := testCoalitionRegistry(t)
	res := reg.Resolve("UKR", "", time.Now().UTC())

	out := renderPage(t, "user_detail", &pageVars{
		User: &userDetail{
			ID: "x", Email: "anton@example.test",
			OrgShortTag: "sky-fortress", OrgDisplayName: "org4x",
			Aff: affiliationView{
				Known: true, ID: "sky-fortress",
				// ADR 1151: the page shows SECTOR, not the deprecated kind.
				Sector: "commercial",
				Nation: "UKR", NationSource: "org registry",
				Organic: false, OrganicKnown: true,
				OrganicBasis: "UKR is not USA, this goatnet's operating sovereign — foreign",
				Standing:     res.Standing, Basis: res.Basis,
				Framework: res.Framework, FrameworkName: res.FrameworkName,
				AsOf: "2026-08-17",
			},
		},
		CSRFToken: "nonce-123",
	})

	for _, m := range []string{
		"Affiliation &amp; coalition standing",
		"sky-fortress",
		// The org's SECTOR is what the page states now. `vendor` was the
		// deprecated kind and is deliberately absent: ADR 1151 removed that
		// vocabulary from every operator-facing surface.
		"commercial",
		"foreign",
		"UKR",
		"partner",
		res.FrameworkName,
		"as-of 2026-08-17",
		"What standing does",
		"What standing does NOT do",
		"zenoh_partner_read_lanes",
		"coalition framework registry",
	} {
		if !strings.Contains(out, m) {
			t.Errorf("user_detail coalition section missing %q", m)
		}
	}
}

// Unknown affiliation must not render the section at all.
func TestTemplates_userDetailCoalitionRowHiddenWhenUnknown(t *testing.T) {
	out := renderPage(t, "user_detail", &pageVars{
		User: &userDetail{ID: "x", Email: "op@example.test"},
	})
	if strings.Contains(out, "coalition standing") {
		t.Error("the coalition section must stay hidden for an identity with no affiliation")
	}
}

// A missing nation must render as an explicit warning with the remedy, since a
// blank there silently means partner.
func TestTemplates_userDetailMissingNationWarns(t *testing.T) {
	out := renderPage(t, "user_detail", &pageVars{
		User: &userDetail{
			ID: "x", Email: "u@example.test",
			Aff: affiliationView{
				Known: true, ID: "acme", Kind: "vendor", KindSource: "org registry",
				Standing: "partner", Basis: "no nation recorded on the affiliation — the rule defaults to partner",
				Framework: "nato", FrameworkName: "North Atlantic Treaty Organization",
				AsOf: "2026-08-17",
			},
		},
	})
	for _, m := range []string{"not recorded", "goat admin org update"} {
		if !strings.Contains(out, m) {
			t.Errorf("missing-nation row should carry %q", m)
		}
	}
}
