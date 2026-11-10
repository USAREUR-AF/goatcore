package config

import "testing"

// Unset lexicon must reproduce the legacy per-vendor wording byte-for-byte so an
// unconfigured portal renders exactly what shipped before the lexicon existed.
func TestLexicon_DefaultsAreLegacyWording(t *testing.T) {
	var l Lexicon
	l.normalize()

	want := map[string]string{
		"__LEX_PRODUCT_NAME__":  "Zenoh Trial",
		"__LEX_PRODUCT_TITLE__": "Zenoh Trial — Vendor Portal",
		"__LEX_ORG__":           "vendor",
		"__LEX_ORGS__":          "vendors",
		"__LEX_ORG_TITLE__":     "Vendor",
		"__LEX_ORGS_TITLE__":    "Vendors",
		"__LEX_ORG_UPPER__":     "VENDOR",
		"__LEX_PANOSCOPE_URL__": "https://panoscope.goat.example/",
	}
	got := l.Tokens()
	for k, v := range want {
		if got[k] != v {
			t.Errorf("token %s = %q, want %q", k, got[k], v)
		}
	}
}

// Setting Org alone derives the plural + title-case forms (regular noun).
func TestLexicon_OrgDerivesForms(t *testing.T) {
	l := Lexicon{Org: "org"}
	l.normalize()
	if l.Orgs != "orgs" || l.OrgTitle != "Org" || l.OrgsTitle != "Orgs" {
		t.Errorf("derive from Org=org: got Orgs=%q OrgTitle=%q OrgsTitle=%q", l.Orgs, l.OrgTitle, l.OrgsTitle)
	}
}

// A slot-pool goatnet override: short product title, org noun. ProductTitle is NOT
// composed from ProductName (it wants "Galaxy Fabric Portal", not
// "Galaxy Fabric — Org Portal"), so an explicit ProductTitle is honored.
func TestLexicon_GoatnetOverride(t *testing.T) {
	l := Lexicon{
		ProductName:  "Galaxy Fabric",
		ProductTitle: "Galaxy Fabric Portal",
		Org:          "org",
	}
	l.normalize()
	tok := l.Tokens()
	if tok["__LEX_PRODUCT_TITLE__"] != "Galaxy Fabric Portal" {
		t.Errorf("ProductTitle = %q, want Galaxy Fabric Portal", tok["__LEX_PRODUCT_TITLE__"])
	}
	if tok["__LEX_ORGS_TITLE__"] != "Orgs" {
		t.Errorf("OrgsTitle = %q, want Orgs", tok["__LEX_ORGS_TITLE__"])
	}
	if tok["__LEX_ORG_UPPER__"] != "ORG" {
		t.Errorf("OrgUpper = %q, want ORG", tok["__LEX_ORG_UPPER__"])
	}
}

// An irregular noun can override the derived forms explicitly.
func TestLexicon_ExplicitFormsWin(t *testing.T) {
	l := Lexicon{Org: "entity", Orgs: "entities"}
	l.normalize()
	if l.Orgs != "entities" {
		t.Errorf("explicit Orgs overridden: got %q", l.Orgs)
	}
}
