package handlers

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/kratos"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// postBrowserOffboardNote is postBrowserOffboard with a note field on the form.
func postBrowserOffboardNote(t *testing.T, h http.Handler, s *Server, pid, note string) *httptest.ResponseRecorder {
	t.Helper()
	form := url.Values{}
	form.Set("csrf", s.csrfNonce)
	form.Set("note", note)
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/onboarding/"+pid+"/offboard", strings.NewReader(form.Encode()))
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	return rec
}

func noteOf(t *testing.T, s *Server, pid string) string {
	t.Helper()
	p, err := s.Onboard.GetProposal(context.Background(), pid)
	if err != nil {
		t.Fatalf("get proposal: %v", err)
	}
	return p.Note
}

// Offboarding an archived row is one-way, so the gate refuses it without a
// note: the store would otherwise keep the old archive reason as the record's
// last word. Blank and whitespace-only are the same thing; a real note goes
// through and is recorded.
func TestBrowserOffboard_ArchivedRequiresNote(t *testing.T) {
	cases := []struct {
		name       string
		note       string
		wantStatus string
		wantNote   string
		wantFlash  string
	}{
		{"empty note refused", "", onboardstore.ProposalArchived, "dormant", flashOffboardNoteRequired},
		{"whitespace note refused", "   ", onboardstore.ProposalArchived, "dormant", flashOffboardNoteRequired},
		{"real note recorded", "left the unit", onboardstore.ProposalOffboarded, "left the unit", ""},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			s := newOnboardTestServer(t)
			h := s.Routes()
			email := "pat-" + strings.ReplaceAll(tc.name, " ", "-") + "@partner.mil"
			pid := onboardedProposalInState(t, s, h, email, onboardstore.ProposalArchived)
			s.Kratos = map[string]*kratos.Client{"tenant": fakeKratosIdentityList(t, email, "inactive")}

			rec := postBrowserOffboardNote(t, h, s, pid, tc.note)
			if rec.Code != http.StatusSeeOther {
				t.Fatalf("want 303, got %d %s", rec.Code, rec.Body.String())
			}
			loc := rec.Header().Get("Location")
			// The refusal returns the operator to the refused card: the
			// archived filter with this row selected, plus the flash.
			if tc.wantFlash != "" && loc != "/onboarding?flash="+tc.wantFlash+"&id="+pid+"&status=archived" {
				t.Fatalf("want redirect to the archived card with flash %q, got %q", tc.wantFlash, loc)
			}
			if tc.wantFlash == "" && strings.Contains(loc, "flash=") {
				t.Fatalf("accepted offboard must not carry a refusal flash, got %q", loc)
			}
			if got := statusOf(t, s, pid); got != tc.wantStatus {
				t.Fatalf("status = %q, want %q", got, tc.wantStatus)
			}
			if got := noteOf(t, s, pid); got != tc.wantNote {
				t.Fatalf("note = %q, want %q", got, tc.wantNote)
			}
			// The refusal's audit row carries the same sentence the
			// decide-side check writes, so one rule has one shape in the
			// audit stream whichever leg caught it.
			if tc.wantFlash != "" {
				if ev := lastAudit(t, s, "onboard_offboard"); ev.Result != "refused" || ev.ErrorMessage != offboardNoteRequiredMsg {
					t.Fatalf("audit = %s/%q, want refused/%q", ev.Result, ev.ErrorMessage, offboardNoteRequiredMsg)
				}
			}
		})
	}
}

// The reason check runs before the login-plane read. With Kratos unreachable
// a precheck-first order would answer "unverified"; the operator must instead
// hear "reason required" at once, and the login plane must not be consulted
// at all (#1283).
func TestBrowserOffboard_ArchivedBlankNoteRefusedBeforeKratos(t *testing.T) {
	for _, note := range []string{"", " "} {
		t.Run(fmt.Sprintf("note=%q", note), func(t *testing.T) {
			s := newOnboardTestServer(t)
			h := s.Routes()
			email := "person-99@example.org"
			pid := onboardedProposalInState(t, s, h, email, onboardstore.ProposalArchived)
			calls := 0
			cli := kratos.NewClient("https://kratos.example.test")
			cli.HTTP = &http.Client{Transport: countingRoundTripper{calls: &calls}}
			s.Kratos = map[string]*kratos.Client{"tenant": cli}

			rec := postBrowserOffboardNote(t, h, s, pid, note)
			loc := rec.Header().Get("Location")
			if rec.Code != http.StatusSeeOther || loc != "/onboarding?flash="+flashOffboardNoteRequired+"&id="+pid+"&status=archived" {
				t.Fatalf("want 303 to the archived card with the note-required flash, got %d %q", rec.Code, loc)
			}
			if calls != 0 {
				t.Fatalf("login plane consulted %d time(s) before the reason check", calls)
			}
			if got := statusOf(t, s, pid); got != onboardstore.ProposalArchived {
				t.Fatalf("status = %q, want archived", got)
			}
			if got := noteOf(t, s, pid); got != "dormant" {
				t.Fatalf("note = %q, want the archive reason kept", got)
			}
		})
	}
}

// countingRoundTripper counts requests and fails each one, so any call made
// before the reason check is both counted and unverifiable.
type countingRoundTripper struct{ calls *int }

func (rt countingRoundTripper) RoundTrip(*http.Request) (*http.Response, error) {
	*rt.calls++
	return nil, errors.New("login plane unreachable")
}

// The requirement is archived-only: from onboarded or enrolled an empty note
// is still accepted, exactly as before.
func TestBrowserOffboard_EmptyNoteStillAcceptedFromReversibleStates(t *testing.T) {
	for _, state := range []string{onboardstore.ProposalOnboarded, onboardstore.ProposalEnrolled} {
		t.Run(state, func(t *testing.T) {
			s := newOnboardTestServer(t)
			h := s.Routes()
			email := "pat-" + state + "@partner.mil"
			pid := onboardedProposalInState(t, s, h, email, state)
			s.Kratos = map[string]*kratos.Client{"tenant": fakeKratosIdentityList(t, email, "inactive")}

			rec := postBrowserOffboardNote(t, h, s, pid, "")
			if loc := rec.Header().Get("Location"); rec.Code != http.StatusSeeOther || strings.Contains(loc, "flash=") {
				t.Fatalf("want 303 without a refusal flash, got %d %q", rec.Code, loc)
			}
			if got := statusOf(t, s, pid); got != onboardstore.ProposalOffboarded {
				t.Fatalf("status = %q, want offboarded", got)
			}
		})
	}
}

// Same gate in the machine realm: {} on an archived row is a 400 bad_request
// carrying the sentence; a body with a note is a 200 offboarded.
func TestMachineOffboard_ArchivedRequiresNote(t *testing.T) {
	cases := []struct {
		name       string
		body       map[string]any
		wantCode   int
		wantStatus string
	}{
		{"empty body refused", map[string]any{}, http.StatusBadRequest, onboardstore.ProposalArchived},
		{"whitespace note refused", map[string]any{"note": " \t"}, http.StatusBadRequest, onboardstore.ProposalArchived},
		{"note accepted", map[string]any{"note": "closing"}, http.StatusOK, onboardstore.ProposalOffboarded},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			s := newOnboardTestServer(t)
			h := s.Routes()
			p := &onboardstore.Proposal{
				Sponsor:      "dev@bypass.local",
				Env:          "zagros",
				Handle:       "pat-" + strings.ReplaceAll(tc.name, " ", "-"),
				Email:        strings.ReplaceAll(tc.name, " ", "-") + "@partner.mil",
				Status:       onboardstore.ProposalArchived,
				ManifestJSON: "{}",
			}
			if err := s.Onboard.CreateProposal(context.Background(), p); err != nil {
				t.Fatalf("create proposal: %v", err)
			}

			got := postMachine(t, h, "/api/onboard/proposals/"+p.ID+"/offboard", tc.body)
			if got.code != tc.wantCode {
				t.Fatalf("want %d, got %d: %s", tc.wantCode, got.code, got.body)
			}
			if tc.wantCode == http.StatusBadRequest {
				if !strings.Contains(got.body, `"code":"bad_request"`) || !strings.Contains(got.body, "closed for good") {
					t.Fatalf("400 body must name bad_request and the reason, got %s", got.body)
				}
			}
			if st := statusOf(t, s, p.ID); st != tc.wantStatus {
				t.Fatalf("status = %q, want %q", st, tc.wantStatus)
			}
		})
	}
}
