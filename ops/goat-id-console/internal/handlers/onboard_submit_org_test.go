package handlers

import (
	"bytes"
	"context"
	"encoding/json"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

// TestSubmit_LongMixedCaseOrg tests the realistic caller shape for organization
// using a mixed-case free text name to ensure the generated affiliation.id
// is valid in the store/manifest.
func TestSubmit_LongMixedCaseOrg(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()

	inv := &onboardstore.Invite{
		Sponsor:      "sponsor@bypass.local",
		Env:          "zagros",
		PreboundJSON: "{\"tier\":\"\"}",
	}
	tok, err := s.Onboard.MintInvite(context.Background(), inv)
	if err != nil {
		t.Fatal(err)
	}

	orgName := "  The Very Long Mixed-Case Organization Name With Spaces And Symbols!  "
	fullPayload := map[string]any{
		"name":              "Real Caller",
		"email":             "person-109@example.org",
		"principal_type":    "person",
		"organization":      orgName,
		"nationality":       "CAN",
		"unit_code":         "",
		"unit_code_scheme":  "uic",
		"title":             "",
		"github_username":   "real",
		"alliance_standing": "organic",
		"sector":            "defense",
		"notes":             "",
		"spire_federated":   false,
		"tier":              "browse",
		"gov_branch":        "",
	}

	body, _ := json.Marshal(map[string]any{"token": tok, "submission": fullPayload})
	req := httptest.NewRequestWithContext(context.Background(), "POST", "/public/onboard/submit", bytes.NewReader(body))
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	h.ServeHTTP(w, req)

	if w.Code != http.StatusCreated {
		t.Fatalf("submit long org name: want 201, got %d. body: %s", w.Code, w.Body.String())
	}

	var res onboardSubmitResponse
	if err := json.Unmarshal(w.Body.Bytes(), &res); err != nil {
		t.Fatal(err)
	}

	prop, err := s.Onboard.GetProposal(context.Background(), res.ProposalID)
	if err != nil {
		t.Fatal(err)
	}
	var m map[string]any
	if err := json.Unmarshal([]byte(prop.ManifestJSON), &m); err != nil {
		t.Fatal(err)
	}

	aff := m["affiliation"].(map[string]any)
	affID := aff["id"].(string)
	if !affiliationIDShape.MatchString(affID) {
		t.Fatalf("affiliation.id %q (len %d) does not satisfy the manifest schema shape", affID, len(affID))
	}
}

// TestSubmit_NonLatinOrg tests AC 5: A submission whose organization is "Збройні сили"
// is accepted and its affiliation.id is non-empty and equal across two submissions of the same name.
func TestSubmit_NonLatinOrg(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()

	submitWithOrg := func(orgName string, handle string, email string) string {
		inv := &onboardstore.Invite{
			Sponsor:      "sponsor@bypass.local",
			Env:          "zagros",
			PreboundJSON: "{\"tier\":\"\"}",
		}
		tok, err := s.Onboard.MintInvite(context.Background(), inv)
		if err != nil {
			t.Fatal(err)
		}

		fullPayload := map[string]any{
			"name":              "Real Caller",
			"email":             email,
			"principal_type":    "person",
			"organization":      orgName,
			"nationality":       "UKR",
			"unit_code":         "",
			"unit_code_scheme":  "uic",
			"title":             "",
			"github_username":   handle,
			"alliance_standing": "organic",
			"sector":            "defense",
			"notes":             "",
			"spire_federated":   false,
			"tier":              "browse",
			"gov_branch":        "",
		}
		body, _ := json.Marshal(map[string]any{"token": tok, "submission": fullPayload})
		req := httptest.NewRequestWithContext(context.Background(), "POST", "/public/onboard/submit", bytes.NewReader(body))
		req.Header.Set("Content-Type", "application/json")
		w := httptest.NewRecorder()
		h.ServeHTTP(w, req)

		if w.Code != http.StatusCreated {
			t.Fatalf("submit %s: want 201, got %d. body: %s", orgName, w.Code, w.Body.String())
		}
		var res onboardSubmitResponse
		if err := json.Unmarshal(w.Body.Bytes(), &res); err != nil {
			t.Fatal(err)
		}

		prop, err := s.Onboard.GetProposal(context.Background(), res.ProposalID)
		if err != nil {
			t.Fatal(err)
		}
		var m map[string]any
		if err := json.Unmarshal([]byte(prop.ManifestJSON), &m); err != nil {
			t.Fatal(err)
		}
		aff := m["affiliation"].(map[string]any)
		return aff["id"].(string)
	}

	affID1 := submitWithOrg("Збройні сили", "caller1", "1@example.test")
	if strings.TrimSpace(affID1) == "" {
		t.Fatalf("affiliation.id is empty for non-Latin org")
	}
	if affID1 == "caller1" {
		t.Fatalf("affiliation.id fell back to handle")
	}

	affID2 := submitWithOrg("Збройні сили", "caller2", "2@example.test")
	if affID1 != affID2 {
		t.Fatalf("affiliation.id is not deterministic: %q != %q", affID1, affID2)
	}
}
