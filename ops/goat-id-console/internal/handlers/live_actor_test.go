package handlers

import (
	"context"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/kratos"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

func liveGateGet(t *testing.T, s *Server, sub, groups string) int {
	t.Helper()
	req := httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/users/x?idp=tenant", nil)
	req.Header.Set("X-Forwarded-Email", "a@example.org")
	req.Header.Set("X-Forwarded-User", sub)
	req.Header.Set("X-Forwarded-Groups", groups)
	rec := httptest.NewRecorder()
	s.Routes().ServeHTTP(rec, req)
	return rec.Code
}

// An admin removed while their console session is open is refused on the
// next request: adminGate checks the identity as it is now, not the groups
// the session cookie carried at sign-in.
func TestAdminGate_refusesInactiveIdentity(t *testing.T) {
	const sub = "0b1f0000-0000-4000-8000-000000000001"
	kc, _ := fakeKratosMeta(t, sub, "a@example.org", "A", []string{"PLATFORM-ADMIN"}, "inactive")
	s := newUsersAPITestServer(t, kc)
	s.Cfg.LiveIdentityCheck = true
	if code := liveGateGet(t, s, sub, "PLATFORM-ADMIN"); code != http.StatusForbidden {
		t.Fatalf("inactive identity with an admin cookie: status %d, want 403", code)
	}
}

func TestAdminGate_usesCurrentGroups(t *testing.T) {
	const sub = "0b1f0000-0000-4000-8000-000000000002"
	kc, _ := fakeKratosMeta(t, sub, "a@example.org", "A", []string{"VIEWER"}, "active")
	s := newUsersAPITestServer(t, kc)
	s.Cfg.LiveIdentityCheck = true
	if code := liveGateGet(t, s, sub, "PLATFORM-ADMIN"); code != http.StatusForbidden {
		t.Fatalf("demoted identity whose cookie still says PLATFORM-ADMIN: status %d, want 403", code)
	}
}

func TestAdminGate_noIdentityIDRefused(t *testing.T) {
	kc, _ := fakeKratosMeta(t, "id-x", "a@example.org", "A", []string{"PLATFORM-ADMIN"}, "active")
	s := newUsersAPITestServer(t, kc)
	s.Cfg.LiveIdentityCheck = true
	if code := liveGateGet(t, s, "", "PLATFORM-ADMIN"); code != http.StatusForbidden {
		t.Fatalf("session with no identity id: status %d, want 403", code)
	}
}

func TestAdminGate_activeAdminPasses(t *testing.T) {
	const sub = "0b1f0000-0000-4000-8000-000000000003"
	kc, _ := fakeKratosMeta(t, sub, "a@example.org", "A", []string{"PLATFORM-ADMIN"}, "active")
	s := newUsersAPITestServer(t, kc)
	s.Cfg.LiveIdentityCheck = true
	if code := liveGateGet(t, s, sub, "PLATFORM-ADMIN"); code == http.StatusForbidden || code == http.StatusServiceUnavailable {
		t.Fatalf("active admin refused by the gate: status %d", code)
	}
}

func postState(t *testing.T, s *Server, id, email, state string) *httptest.ResponseRecorder {
	t.Helper()
	form := url.Values{"state": {state}, "csrf": {s.csrfNonce}}
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/users/"+id+"/state?idp=tenant", strings.NewReader(form.Encode()))
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	req.Header.Set("X-Forwarded-Email", email)
	req.Header.Set("X-Forwarded-Groups", "PLATFORM-ADMIN")
	rec := httptest.NewRecorder()
	s.Routes().ServeHTTP(rec, req)
	return rec
}

// Nobody reactivates their own identity.
func TestPatchState_refusesSelfReactivation(t *testing.T) {
	kc, read := fakeKratosMeta(t, "id-1", "gone@example.org", "Gone", []string{"PLATFORM-ADMIN"}, "inactive")
	s := newUsersAPITestServer(t, kc)
	rec := postState(t, s, "id-1", "gone@example.org", "active")
	if rec.Code != http.StatusForbidden {
		t.Fatalf("self reactivation: status %d, want 403; body=%s", rec.Code, rec.Body.String())
	}
	if _, st := read(); st != "inactive" {
		t.Fatalf("identity state = %q after a refused self reactivation", st)
	}
}

// Reactivating someone else takes step-up: the admin (with a real subject, so
// the posture lookup runs) is sent to the step-up interstitial, not refused for
// some other reason, and the identity stays inactive.
func TestPatchState_reactivationTakesStepUp(t *testing.T) {
	kc, read := fakeKratosMeta(t, "id-2", "gone@example.org", "Gone", nil, "inactive")
	s := newUsersAPITestServer(t, kc)
	s.Permissions.StepUpRequired = append(s.Permissions.StepUpRequired, "reactivate_identity")
	form := url.Values{"state": {"active"}, "csrf": {s.csrfNonce}}
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/users/id-2/state?idp=tenant", strings.NewReader(form.Encode()))
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	req.Header.Set("X-Forwarded-Email", "other-admin@example.org")
	req.Header.Set("X-Forwarded-User", testActorID)
	req.Header.Set("X-Forwarded-Groups", "PLATFORM-ADMIN")
	rec := httptest.NewRecorder()
	s.Routes().ServeHTTP(rec, req)
	if rec.Code != http.StatusSeeOther || !strings.HasPrefix(rec.Header().Get("Location"), "/step-up?nonce=") {
		t.Fatalf("reactivation: status %d Location %q, want 303 to /step-up?nonce=", rec.Code, rec.Header().Get("Location"))
	}
	if _, st := read(); st != "inactive" {
		t.Fatalf("reactivated without step-up (status %d)", rec.Code)
	}
	// Control: without the step-up requirement the same request reactivates,
	// so the redirect above is the step-up gate and nothing else.
	s.Permissions.StepUpRequired = nil
	rec = httptest.NewRecorder()
	req = httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/users/id-2/state?idp=tenant", strings.NewReader(form.Encode()))
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	req.Header.Set("X-Forwarded-Email", "other-admin@example.org")
	req.Header.Set("X-Forwarded-User", testActorID)
	req.Header.Set("X-Forwarded-Groups", "PLATFORM-ADMIN")
	s.Routes().ServeHTTP(rec, req)
	if _, st := read(); st != "active" {
		t.Fatalf("control reactivation without step-up: status %d state %q", rec.Code, st)
	}
}

// fakeKratosMeta is fakeKratos with the identity's groups in metadata_public,
// where every mint and the backfill put them (F-572 / ADR 1188) and the only
// place the groups accessor reads once the traits fallback is gone. The
// returned reader reports the current groups and state.
func fakeKratosMeta(t *testing.T, id, email, name string, groups []string, state string) (*kratos.Client, func() (grps []string, st string)) {
	t.Helper()
	gi := make([]interface{}, len(groups))
	for i, g := range groups {
		gi[i] = g
	}
	cur := map[string]interface{}{
		"id": id, "state": state,
		"traits":          map[string]interface{}{"email": email, "name": name},
		"metadata_public": map[string]interface{}{"groups": gi},
	}
	rt := &kratosAdminRoundTripper{cur: cur}
	read := func() ([]string, string) {
		var out []string
		mp, _ := cur["metadata_public"].(map[string]interface{})
		arr, _ := mp["groups"].([]interface{})
		for _, g := range arr {
			out = append(out, g.(string))
		}
		return out, cur["state"].(string)
	}
	cli := kratos.NewClient("https://kratos.example.test")
	cli.HTTP = &http.Client{Transport: rt}
	return cli, read
}

// seedRosterPerson files an onboarded record for handle/email, the roster
// entry an operator-realm operator-key request must name.
func seedRosterPerson(t *testing.T, s *Server, handle, email string) {
	t.Helper()
	if err := s.Onboard.CreateProposal(context.Background(), &onboardstore.Proposal{Sponsor: "sponsor-1", Env: "zagros",
		Handle: handle, Email: email, Status: onboardstore.ProposalOnboarded, ManifestJSON: "{}", Tier: "platform-admin"}); err != nil {
		t.Fatalf("seed roster person %s: %v", handle, err)
	}
}
