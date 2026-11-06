package handlers

import (
	"bytes"
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// TestSubmit_RealCallerShape verifies validation using the EXACT full payload structure sent
// by the browser JS (all keys present, some empty, reflecting a real post).
// It also tests AC 1b: A submission with a blank organization, a blank open tier, and a lowercase nationality
// returns ONE 422 whose error.fields names all three.
func TestSubmit_RealCallerShape(t *testing.T) {
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

	// This is the REAL shape from onboard.html's submit event.
	// Empty strings for fields the candidate left blank.
	fullPayload := map[string]any{
		"name":              "Real Caller",
		"email":             "person-109@example.org",
		"principal_type":    "person",
		"organization":      "",    // Blank triggers validation_failed
		"nationality":       "can", // Lowercase nationality triggers bad validation
		"unit_code":         "",
		"unit_code_scheme":  "uic",
		"title":             "",
		"github_username":   "bad!", // Invalid GitHub login joins the same field-error response.
		"alliance_standing": "organic",
		"sector":            "defense",
		"notes":             "",
		"spire_federated":   false,
		"tier":              "", // Blank open tier triggers validation_failed
		"gov_branch":        "",
		// machine_quota is absent because readNumber("mqRow") returns null
	}

	body, _ := json.Marshal(map[string]any{"token": tok, "submission": fullPayload})
	req := httptest.NewRequestWithContext(context.Background(), "POST", "/public/onboard/submit", bytes.NewReader(body))
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	h.ServeHTTP(w, req)

	if w.Code != http.StatusUnprocessableEntity {
		t.Fatalf("real shape validation: want 422, got %d. body: %s", w.Code, w.Body.String())
	}

	var res map[string]onboardAPIError
	if err := json.Unmarshal(w.Body.Bytes(), &res); err != nil {
		t.Fatal(err)
	}

	errObj := res["error"]
	if errObj.Code != "validation_failed" {
		t.Fatalf("want error code validation_failed, got %q", errObj.Code)
	}

	fields := errObj.Fields
	if _, ok := fields["organization"]; !ok {
		t.Errorf("missing organization in error.fields")
	}
	if _, ok := fields["tier"]; !ok {
		t.Errorf("missing tier in error.fields")
	}
	if _, ok := fields["nationality"]; !ok {
		t.Errorf("missing nationality in error.fields")
	}
	if _, ok := fields["github_username"]; !ok {
		t.Errorf("missing github_username in error.fields")
	}
	if len(fields) != 4 {
		t.Errorf("expected exactly 4 fields in error.fields, got %d: %v", len(fields), fields)
	}

	// Verify invite is unconsumed by querying its status.
	// We can check if we can submit it successfully once fields are fixed (AC 1).
	fullPayload["organization"] = "Valid Organization"
	fullPayload["tier"] = "browse"
	fullPayload["nationality"] = "CAN"
	fullPayload["github_username"] = "@valid-user"

	body2, _ := json.Marshal(map[string]any{"token": tok, "submission": fullPayload})
	req2 := httptest.NewRequestWithContext(context.Background(), "POST", "/public/onboard/submit", bytes.NewReader(body2))
	req2.Header.Set("Content-Type", "application/json")
	w2 := httptest.NewRecorder()
	h.ServeHTTP(w2, req2)

	if w2.Code != http.StatusCreated {
		t.Fatalf("subsequent valid submit should succeed, invite should be unconsumed. got %d: %s", w2.Code, w2.Body.String())
	}
}

func TestSubmit_GitHubUsernameValidation(t *testing.T) {
	validSubmission := func(github string) map[string]any {
		return map[string]any{
			"name":            "GitHub Candidate",
			"email":           "github-candidate@example.test",
			"principal_type":  "person",
			"organization":    "Test Organization",
			"nationality":     "CAN",
			"github_username": github,
		}
	}
	post := func(t *testing.T, h http.Handler, token string, sub map[string]any) *httptest.ResponseRecorder {
		t.Helper()
		body, err := json.Marshal(map[string]any{"token": token, "submission": sub})
		if err != nil {
			t.Fatal(err)
		}
		req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/public/onboard/submit", bytes.NewReader(body))
		req.Header.Set("Content-Type", "application/json")
		w := httptest.NewRecorder()
		h.ServeHTTP(w, req)
		return w
	}

	for _, github := range []string{
		"octocat!",
		"-octocat",
		"octocat-",
		"octo--cat",
		strings.Repeat("a", 40),
	} {
		t.Run("reject_"+github, func(t *testing.T) {
			s := newOnboardTestServer(t)
			h := s.Routes()
			inv := &onboardstore.Invite{Sponsor: "sponsor@bypass.local", Env: "zagros", PreboundJSON: `{"tier":"browse"}`}
			token, err := s.Onboard.MintInvite(context.Background(), inv)
			if err != nil {
				t.Fatal(err)
			}

			w := post(t, h, token, validSubmission(github))
			if w.Code != http.StatusUnprocessableEntity {
				t.Fatalf("invalid GitHub login %q: want 422, got %d: %s", github, w.Code, w.Body.String())
			}
			var res map[string]onboardAPIError
			if err := json.Unmarshal(w.Body.Bytes(), &res); err != nil {
				t.Fatal(err)
			}
			if _, ok := res["error"].Fields["github_username"]; !ok {
				t.Fatalf("invalid GitHub login %q: missing github_username field error: %s", github, w.Body.String())
			}

			storedInvite, err := s.Onboard.GetInviteByToken(context.Background(), token)
			if err != nil {
				t.Fatal(err)
			}
			if storedInvite.Status != onboardstore.InviteInvited {
				t.Fatalf("invalid GitHub login %q consumed invite: status=%q", github, storedInvite.Status)
			}
			proposals, err := s.Onboard.ListProposals(context.Background(), "")
			if err != nil {
				t.Fatal(err)
			}
			if len(proposals) != 0 {
				t.Fatalf("invalid GitHub login %q created %d proposals", github, len(proposals))
			}

			w = post(t, h, token, validSubmission("corrected-login"))
			if w.Code != http.StatusCreated {
				t.Fatalf("corrected GitHub login after %q: want 201, got %d: %s", github, w.Code, w.Body.String())
			}
		})
	}

	for _, tc := range []struct {
		name   string
		input  string
		stored string
	}{
		{name: "valid", input: "octocat", stored: "octocat"},
		{name: "leading at", input: "  @Octo-Cat  ", stored: "Octo-Cat"},
		{name: "maximum length", input: strings.Repeat("a", 39), stored: strings.Repeat("a", 39)},
		{name: "empty optional field", input: "", stored: ""},
	} {
		t.Run(tc.name, func(t *testing.T) {
			s := newOnboardTestServer(t)
			h := s.Routes()
			inv := &onboardstore.Invite{Sponsor: "sponsor@bypass.local", Env: "zagros", PreboundJSON: `{"tier":"browse"}`}
			token, err := s.Onboard.MintInvite(context.Background(), inv)
			if err != nil {
				t.Fatal(err)
			}

			w := post(t, h, token, validSubmission(tc.input))
			if w.Code != http.StatusCreated {
				t.Fatalf("valid GitHub login %q: want 201, got %d: %s", tc.input, w.Code, w.Body.String())
			}
			var created onboardSubmitResponse
			if err := json.Unmarshal(w.Body.Bytes(), &created); err != nil {
				t.Fatal(err)
			}
			proposal, err := s.Onboard.GetProposal(context.Background(), created.ProposalID)
			if err != nil {
				t.Fatal(err)
			}
			var manifest map[string]any
			if err := json.Unmarshal([]byte(proposal.ManifestJSON), &manifest); err != nil {
				t.Fatal(err)
			}
			stored, _ := getPath(manifest, "identity.github").(string)
			if stored != tc.stored {
				t.Fatalf("identity.github = %q, want %q", stored, tc.stored)
			}
		})
	}
}

func TestSubmitRejectsMalformedPrebindingWithoutConsumingInvite(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	inv := &onboardstore.Invite{
		Sponsor:      "limited-sponsor@example.com",
		Env:          "zagros",
		PreboundJSON: `{not-json`,
	}
	tok, err := s.Onboard.MintInvite(context.Background(), inv)
	if err != nil {
		t.Fatal(err)
	}

	submission := map[string]any{
		"name": "Privilege Probe", "email": "probe@example.com",
		"principal_type": "system", "organization": "Example Org",
		"tier": "platform-admin",
	}
	body, err := json.Marshal(map[string]any{"token": tok, "submission": submission})
	if err != nil {
		t.Fatal(err)
	}
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/public/onboard/submit", bytes.NewReader(body))
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	h.ServeHTTP(w, req)

	if w.Code != http.StatusInternalServerError {
		t.Fatalf("malformed prebinding submit = %d, want 500; body: %s", w.Code, w.Body.String())
	}
	stored, err := s.Onboard.GetInviteByToken(context.Background(), tok)
	if err != nil {
		t.Fatalf("reload invite: %v", err)
	}
	if stored.Status != onboardstore.InviteInvited {
		t.Fatalf("malformed invite status = %q, want %q", stored.Status, onboardstore.InviteInvited)
	}
	proposals, err := s.Onboard.ListProposals(context.Background(), "")
	if err != nil {
		t.Fatalf("list proposals: %v", err)
	}
	if len(proposals) != 0 {
		t.Fatalf("malformed prebinding created %d proposals", len(proposals))
	}
}

func TestSubmit_NationCodeValidator(t *testing.T) {
	// Tests AC 7: Intake and review editor call the same validator.
	// A full country name upper-cased (e.g. UKRAINE) should be refused by validNationCode.
	if validNationCode("UKRAINE") {
		t.Errorf("UKRAINE should be refused by the shared validator")
	}
	if !validNationCode("UKR") {
		t.Errorf("UKR should be accepted")
	}
	if validNationCode("ukr") {
		t.Errorf("ukr should be refused (must be uppercase)")
	}
	if validNationCode("LONGNAME") {
		t.Errorf("LONGNAME should be refused")
	}
	if validNationCode("") {
		t.Errorf("empty should be refused")
	}
}

func TestSubmit_ValidationEdgeCases(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()

	tests := []struct {
		name       string
		submission map[string]any
		wantStatus int
		wantFields []string
	}{
		{
			name: "missing name and email",
			submission: map[string]any{
				"name":            "   ",
				"email":           "",
				"principal_type":  "person",
				"organization":    "Test Org",
				"nationality":     "CAN",
				"tier":            "browse",
				"github_username": "valid",
			},
			wantStatus: http.StatusUnprocessableEntity,
			wantFields: []string{"name", "email"},
		},
		{
			name: "organization with no letter or digit cannot form an id",
			submission: map[string]any{
				"name":            "Test",
				"email":           "person-118@example.org",
				"principal_type":  "person",
				"organization":    "!!! ---",
				"nationality":     "CAN",
				"tier":            "browse",
				"github_username": "sym",
			},
			wantStatus: http.StatusUnprocessableEntity,
			wantFields: []string{"organization"},
		},
		{
			name: "cannot derive handle from bad email",
			submission: map[string]any{
				"name":            "Test",
				"email":           "no-at-sign",
				"principal_type":  "person",
				"organization":    "Test Org",
				"nationality":     "CAN",
				"tier":            "browse",
				"github_username": "",
			},
			wantStatus: http.StatusUnprocessableEntity,
			wantFields: []string{"handle"},
		},
		{
			name: "system without nationality is accepted",
			submission: map[string]any{
				"name":            "System",
				"email":           "person-119@example.org",
				"principal_type":  "system",
				"organization":    "Test Org",
				"nationality":     "",
				"tier":            "browse",
				"github_username": "sys-handle",
			},
			wantStatus: http.StatusCreated,
			wantFields: nil,
		},
		{
			name: "person without nationality is refused",
			submission: map[string]any{
				"name":            "Person",
				"email":           "person-107@example.org",
				"principal_type":  "person",
				"organization":    "Test Org",
				"nationality":     "",
				"tier":            "browse",
				"github_username": "per-handle",
			},
			wantStatus: http.StatusUnprocessableEntity,
			wantFields: []string{"nationality"},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			inv := &onboardstore.Invite{
				Sponsor:      "sponsor@bypass.local",
				Env:          "zagros",
				PreboundJSON: `{"tier":""}`,
			}
			tok, err := s.Onboard.MintInvite(context.Background(), inv)
			if err != nil {
				t.Fatal(err)
			}

			body, _ := json.Marshal(map[string]any{"token": tok, "submission": tt.submission})
			req := httptest.NewRequestWithContext(context.Background(), "POST", "/public/onboard/submit", bytes.NewReader(body))
			req.Header.Set("Content-Type", "application/json")
			w := httptest.NewRecorder()
			h.ServeHTTP(w, req)

			if w.Code != tt.wantStatus {
				t.Fatalf("want status %d, got %d. body: %s", tt.wantStatus, w.Code, w.Body.String())
			}

			if tt.wantStatus == http.StatusUnprocessableEntity {
				var res map[string]onboardAPIError
				if err := json.Unmarshal(w.Body.Bytes(), &res); err != nil {
					t.Fatal(err)
				}
				fields := res["error"].Fields
				if len(fields) != len(tt.wantFields) {
					t.Errorf("want %d fields %v, got %d: %v", len(tt.wantFields), tt.wantFields, len(fields), fields)
				}
				for _, f := range tt.wantFields {
					if _, ok := fields[f]; !ok {
						t.Errorf("expected field %q in error response, not found", f)
					}
				}
			}
		})
	}
}
