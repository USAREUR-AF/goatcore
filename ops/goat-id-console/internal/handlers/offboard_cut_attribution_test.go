package handlers

import (
	"bytes"
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

func postOffboardCutAs(t *testing.T, h http.Handler, s *Server, pid, email, identityID string) *httptest.ResponseRecorder {
	t.Helper()
	form := url.Values{}
	form.Set("csrf", s.csrfNonce)
	form.Set("reason_class", "left-service")
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/onboarding/"+pid+"/offboard-cut", strings.NewReader(form.Encode()))
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	req.Header.Set("X-Forwarded-Groups", "PLATFORM-ADMIN")
	req.Header.Set("X-Forwarded-Email", email)
	if identityID != "" {
		req.Header.Set("X-Forwarded-User", identityID)
	}
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	return rec
}

// plantedManifest is an intake manifest that carries the admin's email under
// a handle of the caller's choosing: what a sponsor or operator key could file
// to steer an email-keyed lookup.
func plantedManifest(email, handle string) map[string]any {
	return map[string]any{
		"identity": map[string]any{"handle": handle, "email": email, "name": "Planted", "status": "active",
			"added": "2026-06-01", "added_by": "admin@example.test"},
		"netbird":  map[string]any{"enabled": true, "sites": []string{"zagros"}, "role": "peer", "auto_groups": []string{"OBSERVER"}, "handset_limit": 1},
		"ory_site": map[string]any{"enabled": false},
		"ory_idp":  map[string]any{"enabled": false},
		"zenoh":    map[string]any{"enabled": false},
	}
}

func seedAdminRow(t *testing.T, s *Server) {
	t.Helper()
	admin := &onboardstore.Proposal{Sponsor: "root@example.org", Env: "zagros", Handle: "admin-alex",
		Email: "admin@example.org", ManifestJSON: "{}"}
	if err := s.Onboard.CreateProposal(context.Background(), admin); err != nil {
		t.Fatal(err)
	}
	if err := s.Onboard.SetProposalStatus(context.Background(), admin.ID, onboardstore.ProposalEnrolled); err != nil {
		t.Fatal(err)
	}
}

// The approver recorded on a queued cut is the authenticated session's
// identity id, whatever onboarding records exist for the admin's email. The
// planted cases are the GHSA-3m4r review exploits: an operator key or a
// sponsor key files an intake row with the admin's email and another handle
// (forging the approver), or under another goatnet (suppressing it).
func TestOffboardCutApproverComesFromTheSession(t *testing.T) {
	cases := []struct {
		name  string
		plant func(t *testing.T, s *Server, h http.Handler)
	}{
		{"no plant", func(*testing.T, *Server, http.Handler) {}},
		{"operator key plants another handle", func(t *testing.T, _ *Server, h http.Handler) {
			res := postMachine(t, h, "/api/onboard/intake", map[string]any{"manifest": plantedManifest("admin@example.org", "admin-bob"), "env": "zagros"})
			if res.code != http.StatusCreated {
				t.Fatalf("intake: %d %s", res.code, res.body)
			}
		}},
		{"sponsor key plants another handle", func(t *testing.T, s *Server, _ http.Handler) {
			body, _ := json.Marshal(map[string]any{"manifest": plantedManifest("admin@example.org", "admin-bob"), "env": "zagros"})
			req := httptest.NewRequest(http.MethodPost, "/api/onboard/intake", bytes.NewReader(body)) //nolint:noctx // behavior kept as reviewed for disclosure; lint cleanup goatcore#1028
			req.Header.Set("Content-Type", "application/json")
			req = req.WithContext(context.WithValue(req.Context(), ctxKeyOnboardPrincipal, onboardPrincipal{Handle: "partner-sponsor", IsOperator: false}))
			rec := httptest.NewRecorder()
			s.apiOnboardIntake(rec, req)
			if rec.Code != http.StatusCreated {
				t.Fatalf("intake as sponsor: %d %s", rec.Code, rec.Body.String())
			}
		}},
		{"operator key plants another goatnet", func(t *testing.T, _ *Server, h http.Handler) {
			res := postMachine(t, h, "/api/onboard/intake", map[string]any{"manifest": plantedManifest("admin@example.org", "whatever"), "env": "othernet"})
			if res.code != http.StatusCreated {
				t.Fatalf("intake: %d %s", res.code, res.body)
			}
		}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			s := newOnboardTestServer(t)
			h := s.Routes()
			seedAdminRow(t, s)
			tc.plant(t, s, h)
			id := seedOffboardRow(t, s, onboardstore.ProposalOnboarded)
			if rec := postOffboardCutAs(t, h, s, id, "admin@example.org", strings.ToUpper(testActorID)); rec.Code != http.StatusSeeOther {
				t.Fatalf("offboard-cut: %d %s", rec.Code, rec.Body.String())
			}
			// What the runner sees on its poll.
			list := getMachine(t, h, "/api/onboard/proposals?status="+onboardstore.ProposalOffboardRequested)
			var rows []onboardstore.Proposal
			if err := json.Unmarshal([]byte(list.body), &rows); err != nil {
				t.Fatalf("decode list: %v (%s)", err, list.body)
			}
			if len(rows) != 1 || rows[0].OffboardRequestedBy != "admin@example.org" || rows[0].OffboardRequestedByIdentity != testActorID {
				t.Fatalf("runner sees %+v, want requested_by admin@example.org with the session identity %s", rows, testActorID)
			}
		})
	}
}

// A session with no identity id still cuts access: the identity is
// attribution, never a gate on removing someone.
func TestOffboardCutWithoutSessionIdentityStillQueues(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	id := seedOffboardRow(t, s, onboardstore.ProposalOnboarded)
	if rec := postOffboardCutAs(t, h, s, id, "admin@example.org", "not-a-uuid"); rec.Code != http.StatusSeeOther {
		t.Fatalf("offboard-cut: %d %s", rec.Code, rec.Body.String())
	}
	p := mustGetProposal(t, s, id)
	if p.Status != onboardstore.ProposalOffboardRequested || p.OffboardRequestedBy != "admin@example.org" || p.OffboardRequestedByIdentity != "" {
		t.Fatalf("row = status %q by %q identity %q, want queued by the email with no identity", p.Status, p.OffboardRequestedBy, p.OffboardRequestedByIdentity)
	}
}

func TestSessionOperatorActor(t *testing.T) {
	for in, want := range map[string]string{
		testActorID:                  "kratos-" + testActorID,
		strings.ToUpper(testActorID): "kratos-" + testActorID,
		"":                           "",
		"admin@example.org":          "",
		"admin-alex":                 "",
	} {
		if got := sessionOperatorActor(Actor{IdentityID: in, Email: "admin@example.org"}); got != want {
			t.Errorf("sessionOperatorActor(%q) = %q, want %q", in, got, want)
		}
	}
}
