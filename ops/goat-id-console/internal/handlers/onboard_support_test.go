package handlers

import (
	"net/http"
	"net/http/httptest"
	"regexp"
	"strings"
	"testing"

	"golang.org/x/text/unicode/norm"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// affiliationIDShape is the manifest schema's affiliation.id pattern
// (operator-manifest.schema.json). Every id the intake derives must match it,
// or the approve-time lint refuses a proposal whose invite is already spent.
var affiliationIDShape = regexp.MustCompile(`^[a-z0-9][a-z0-9-]{0,62}[a-z0-9]$`)

// The invite endpoint checks affiliation.kind at mint. Otherwise an org shortname
// sails through and the lint refuses it at APPROVE — after the candidate has
// filled in the form, and after the field became uneditable under ADR 1137's
// closed set. The value is knowable as wrong the moment it is typed.
func TestIsAffiliationKind(t *testing.T) {
	for _, ok := range []string{"vendor", "organic", "partner", "civil", "Partner", " civil "} {
		if !isAffiliationKind(ok) {
			t.Errorf("%q should be a valid affiliation kind", ok)
		}
	}
	// Org shortnames typed into the field that wants a TYPE.
	for _, bad := range []string{"cis", "cybercom", "demo-nat-unit", "risb", "", "  "} {
		if isAffiliationKind(bad) {
			t.Errorf("%q must NOT pass as an affiliation kind", bad)
		}
	}
}

func TestAffiliationIDFromOrg_SlugASCII(t *testing.T) {
	org := "Demoforge Limited"
	id := affiliationIDFromOrg(org)
	if id == "" {
		t.Fatalf("affiliationIDFromOrg(%q) = empty", org)
	}
	if id != slug(org) {
		t.Fatalf("affiliationIDFromOrg(%q) = %q, want slug %q", org, id, slug(org))
	}
}

func TestAffiliationIDFromOrg_HashNonASCIIStable(t *testing.T) {
	org := "Збройні сили"
	if s := slug(org); s != "" {
		t.Fatalf("slug(%q) = %q, want empty for non-ASCII-only name", org, s)
	}
	id1 := affiliationIDFromOrg(org)
	id2 := affiliationIDFromOrg(org)
	if id1 == "" {
		t.Fatalf("affiliationIDFromOrg(%q) = empty", org)
	}
	if len(id1) != 8 {
		t.Fatalf("affiliationIDFromOrg(%q) length = %d, want 8", org, len(id1))
	}
	if id1 != id2 {
		t.Fatalf("affiliationIDFromOrg(%q) unstable: %q vs %q", org, id1, id2)
	}
}

// Every derived id satisfies the manifest schema shape, whatever the name
// looks like: an over-long slug is cut to the ceiling, a name too short to
// slug hashes, and a name with no letter or digit yields nothing so the
// validator can refuse it by name.
func TestAffiliationIDFromOrg_AlwaysSchemaShaped(t *testing.T) {
	long := "  The Very Long Mixed-Case Organization Name With Spaces And Symbols And More Words!  "
	if got := slug(long); len(got) <= affiliationIDMaxLen {
		t.Fatalf("test name slugs to %d chars, want more than %d", len(got), affiliationIDMaxLen)
	}
	for _, org := range []string{long, "A", "Demoforge Limited", "Збройні сили", "Ц", "x-"} {
		id := affiliationIDFromOrg(org)
		if !affiliationIDShape.MatchString(id) {
			t.Errorf("affiliationIDFromOrg(%q) = %q (len %d), want the schema shape", org, id, len(id))
		}
	}
	if id := affiliationIDFromOrg(long); len(id) != affiliationIDMaxLen || !strings.HasPrefix(id, "the-very-long-") {
		t.Errorf("long name: id = %q (len %d), want the slug cut to %d", id, len(id), affiliationIDMaxLen)
	}
	if id := affiliationIDFromOrg("A"); len(id) != 8 {
		t.Errorf("one-letter name: id = %q, want an 8-hex hash", id)
	}
	for _, org := range []string{"", "   ", "!!! ---", "© ™"} {
		if id := affiliationIDFromOrg(org); id != "" {
			t.Errorf("affiliationIDFromOrg(%q) = %q, want empty for a name with no letter or digit", org, id)
		}
	}
}

// The hash path folds the name the way the registry matcher does, so the
// same organisation typed in another Unicode form, with extra spaces, or in
// another case lands on one id. Two different names still differ.
func TestAffiliationIDFromOrg_HashFoldsLikeTheMatcher(t *testing.T) {
	base := affiliationIDFromOrg("Збройні сили")
	for _, variant := range []string{
		norm.NFD.String("Збройні сили"),
		"Збройні  сили",
		"ЗБРОЙНІ СИЛИ",
		"Збройні\tсили ",
	} {
		if got := affiliationIDFromOrg(variant); got != base {
			t.Errorf("affiliationIDFromOrg(%q) = %q, want %q (same name, other spelling)", variant, got, base)
		}
	}
	if other := affiliationIDFromOrg("Повітряні сили"); other == base {
		t.Errorf("two organisations share the id %q", base)
	}
}

// A refusal that names no field is a programming error, and must never
// become an implicit 200 on a failed submit.
func TestOnboardValidationError_EmptyFieldsIsAServerError(t *testing.T) {
	w := httptest.NewRecorder()
	onboardValidationError(w, nil)
	if w.Code != http.StatusInternalServerError {
		t.Fatalf("empty fields: status %d, want 500; body %s", w.Code, w.Body.String())
	}
	if !strings.Contains(w.Body.String(), `"submit_error"`) {
		t.Fatalf("empty fields: body %s, want the submit_error envelope", w.Body.String())
	}
}

func TestAssembleManifestRejectsInvalidGitHubUsername(t *testing.T) {
	hints, err := loadIntakeHints()
	if err != nil {
		t.Fatalf("loadIntakeHints: %v", err)
	}
	_, err = assembleManifest(hints, effectiveInputs{
		Tier:           "browse",
		AffiliationID:  "test-org",
		Handle:         "test-user",
		Email:          "test@example.test",
		GitHubUsername: "invalid--login",
	})
	if err == nil || !strings.Contains(err.Error(), "not a GitHub login") {
		t.Fatalf("assembleManifest invalid GitHub username: got %v, want validator error", err)
	}
}

func TestNationCodeValidation_SharedByIntakeAndReview(t *testing.T) {
	tests := []struct {
		val   string
		valid bool
	}{
		{"CAN", true},
		{"UKR", true},
		{"NATO", true},
		{"EU", true},
		{"can", false},
		{"Canada", false},
		{"U", false},
		{"123", false},
	}

	for _, tt := range tests {
		t.Run(tt.val, func(t *testing.T) {
			// 1. Check validNationCode (the shared validator)
			got := validNationCode(tt.val)
			if got != tt.valid {
				t.Errorf("validNationCode(%q) = %v, want %v", tt.val, got, tt.valid)
			}

			// 2. Check Review Editor (setNation)
			m := map[string]any{"affiliation": map[string]any{}}
			_, _, err := setNation(m, tt.val)
			reviewValid := (err == nil)
			if reviewValid != tt.valid {
				t.Errorf("setNation(%q) valid = %v, want %v", tt.val, reviewValid, tt.valid)
			}

			// 3. Check Intake (validationErrorsForSubmission)
			inv := &onboardstore.Invite{PreboundJSON: `{"org":"prebound"}`} // Skip org validation
			sub := candidateSubmission{
				Name: "Test", Email: "test@example.com", Handle: "test",
				PrincipalType: "system", // Skip nationality validation
				Nation:        tt.val,   // The field to test
			}
			fields, validationErr := validationErrorsForSubmission(inv, &sub)
			if validationErr != nil {
				t.Fatalf("validationErrorsForSubmission: %v", validationErr)
			}
			_, intakeError := fields["nation"]
			intakeValid := !intakeError
			if intakeValid != tt.valid {
				t.Errorf("validationErrorsForSubmission for nation=%q valid = %v, want %v (fields: %v)", tt.val, intakeValid, tt.valid, fields)
			}
		})
	}
}

// orgIDForIntake derives the affiliation id from either a prebound
// AffiliationID or the typed Organization, through affiliationIDFromOrg.
// These tests lock the precedence and the empty-input behaviour.
func TestOrgIDForIntakePrefersAffiliationID(t *testing.T) {
	in := effectiveInputs{AffiliationID: "prebound-org", Organization: "typed-org"}
	if got := orgIDForIntake(in); got != "prebound-org" {
		t.Fatalf("orgIDForIntake = %q, want prebound-org", got)
	}
}

func TestOrgIDForIntakeFallsBackToOrganizationID(t *testing.T) {
	for _, org := range []string{"New Moon Corp", "Збройні сили"} {
		in := effectiveInputs{Organization: org}
		want := affiliationIDFromOrg(org)
		if got := orgIDForIntake(in); want == "" || got != want {
			t.Fatalf("orgIDForIntake(%q) = %q, want %q", org, got, want)
		}
	}
}

func TestOrgIDForIntakeEmptyInputsYieldEmptyID(t *testing.T) {
	in := effectiveInputs{}
	if got := orgIDForIntake(in); got != "" {
		t.Fatalf("orgIDForIntake = %q, want empty", got)
	}
}

// slug lowercases, trims, and normalizes an arbitrary organisation string
// into a handle-safe slug before registry lookups. These tests focus on
// safety: charset, whitespace collapse, and dropping chaotic characters.
func TestSlugStripsEmojiUnicodeAndKeepsAscii(t *testing.T) {
	name := "New Moon Corp 🌕🚀"
	got := slug(name)
	if got == "" {
		t.Fatalf("slug(%q) empty, want non-empty", name)
	}
	for _, r := range got {
		if (r < 'a' || r > 'z') && (r < '0' || r > '9') && r != '-' {
			t.Fatalf("slug(%q) contains unsafe character %q", name, string(r))
		}
	}
}

func TestSlugSanitizesInjectionShapedCharacters(t *testing.T) {
	name := `Org "Inj'/\Corp---`
	got := slug(name)
	if got == "" {
		t.Fatalf("slug(%q) empty, want non-empty", name)
	}
	if strings.ContainsAny(got, "\"'\\/") {
		t.Fatalf("slug(%q) = %q still contains unsafe punctuation", name, got)
	}
}

func TestSlugWhitespaceOnlyYieldsEmpty(t *testing.T) {
	if got := slug("   \t\n  "); got != "" {
		t.Fatalf("slug on whitespace-only = %q, want empty", got)
	}
}
