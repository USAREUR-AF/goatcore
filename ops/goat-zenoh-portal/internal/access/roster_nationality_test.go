package access

import (
	"os"
	"path/filepath"
	"testing"
)

// TestLoadRoster_NationalityIsNotInheritedFromTheOrg pins ADR 1171's central
// claim at the one place the projection could quietly undo it: a member's own
// citizenship comes from the member row, the org's nation comes from the org
// row, and an undeclared nationality stays empty rather than borrowing the
// org's.
func TestLoadRoster_NationalityIsNotInheritedFromTheOrg(t *testing.T) {
	path := filepath.Join(t.TempDir(), "roster.json")
	body := `{
      "orgs": [
        {"id": "usareur-af", "kind": "organic", "nation": "USA"},
        {"id": "example-hq", "kind": "partner", "nation": "NATO"}
      ],
      "members": [
        {"handle": "a-canadian-officer", "type": "person", "org": "usareur-af",
         "status": "active", "nationality": "CAN"},
        {"handle": "a-us-officer", "type": "person", "org": "usareur-af",
         "status": "active", "nationality": "USA"},
        {"handle": "undeclared", "type": "person", "org": "usareur-af",
         "status": "active"},
        {"handle": "a-belgian-at-example-hq", "type": "person", "org": "example-hq",
         "status": "active", "nationality": "BEL"}
      ]
    }`
	if err := os.WriteFile(path, []byte(body), 0o600); err != nil {
		t.Fatalf("write roster: %v", err)
	}

	entries, err := LoadRoster(path)
	if err != nil {
		t.Fatalf("LoadRoster: %v", err)
	}
	byHandle := map[string]RosterEntry{}
	for _, e := range entries {
		byHandle[e.Handle] = e
	}

	for _, tc := range []struct{ handle, wantNation, wantNationality string }{
		// The case the model exists for: person and org disagree, both are kept.
		{"a-canadian-officer", "USA", "CAN"},
		// Agreeing is not the same as inheriting — it is still the member's row.
		{"a-us-officer", "USA", "USA"},
		// Undeclared stays undeclared. Copying "USA" here is the whole bug.
		{"undeclared", "USA", ""},
		{"a-belgian-at-example-hq", "NATO", "BEL"},
	} {
		e, ok := byHandle[tc.handle]
		if !ok {
			t.Fatalf("%s missing from the loaded roster", tc.handle)
		}
		if e.Nation != tc.wantNation {
			t.Errorf("%s: Nation = %q, want %q (the org's)", tc.handle, e.Nation, tc.wantNation)
		}
		if e.Nationality != tc.wantNationality {
			t.Errorf("%s: Nationality = %q, want %q (the person's own)",
				tc.handle, e.Nationality, tc.wantNationality)
		}
	}
}
