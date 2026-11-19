package handlers

import (
	"context"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
)

// The live identity check covers every browser gate, not only adminGate: the
// self-service routes and the sponsor branch of onboardGate authorize on the
// same session cookie, so an offboarded person's open tab must be refused there
// too.

const goneSub = "0b1f0000-0000-4000-8000-00000000dead"

func gateReq(t *testing.T, s *Server, method, path, email, sub, groups string, form url.Values) *httptest.ResponseRecorder {
	t.Helper()
	body := strings.NewReader("")
	if form != nil {
		form.Set("csrf", s.csrfNonce)
		body = strings.NewReader(form.Encode())
	}
	req := httptest.NewRequestWithContext(context.Background(), method, path, body)
	if form != nil {
		req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	}
	req.Header.Set("X-Forwarded-Email", email)
	req.Header.Set("X-Forwarded-User", sub)
	req.Header.Set("X-Forwarded-Groups", groups)
	rec := httptest.NewRecorder()
	s.Routes().ServeHTTP(rec, req)
	return rec
}

func TestSelfService_refusesInactiveIdentity(t *testing.T) {
	kc, _ := fakeKratosMeta(t, goneSub, "gone@example.org", "Gone", []string{"PLATFORM-ADMIN"}, "inactive")
	s := newUsersAPITestServer(t, kc)
	s.Cfg.LiveIdentityCheck = true
	for _, p := range []string{"/github", "/operator-key", "/setup-code", "/fabric-credential"} {
		if rec := gateReq(t, s, http.MethodGet, p, "gone@example.org", goneSub, "PLATFORM-ADMIN", nil); rec.Code != http.StatusForbidden {
			t.Errorf("GET %s as an inactive identity: status %d, want 403", p, rec.Code)
		}
	}
}

func TestSelfService_activeMemberPasses(t *testing.T) {
	const sub = "0b1f0000-0000-4000-8000-0000000000c3"
	kc, _ := fakeKratosMeta(t, sub, "member@example.org", "Member", nil, "active")
	s := newUsersAPITestServer(t, kc)
	s.Cfg.LiveIdentityCheck = true
	if rec := gateReq(t, s, http.MethodGet, "/setup-code", "member@example.org", sub, "", nil); rec.Code == http.StatusForbidden || rec.Code == http.StatusServiceUnavailable {
		t.Fatalf("active member refused at /setup-code: %d", rec.Code)
	}
}

func TestOnboardGate_sponsorBranchRefusesInactiveIdentity(t *testing.T) {
	kc, _ := fakeKratosMeta(t, goneSub, "sponsor@example.org", "Gone", nil, "inactive")
	s := newUsersAPITestServer(t, kc)
	s.Cfg.LiveIdentityCheck = true
	s.Cfg.NotifyGoatnet = "zagros"
	portalURL, _ := fakeSponsorPortal(t, nil, map[string]sponsorLookupResponse{
		"sponsor@example.org": {Org: "0ff404dd", Alias: "pat-example", SponsorAuthorized: true, SponsorMaxTier: "developer"},
	})
	s.Cfg.PortalInternalURL = portalURL
	s.Cfg.PortalLookupToken = "svc-token"

	if rec := gateReq(t, s, http.MethodGet, "/onboarding/invite", "sponsor@example.org", goneSub, "", nil); rec.Code != http.StatusForbidden {
		t.Errorf("GET /onboarding/invite as an inactive sponsor: status %d, want 403", rec.Code)
	}
	form := url.Values{"env": {"zagros"}, "tier": {"developer"}, "machine_quota": {"1"}, "ttl_hours": {"48"},
		"candidate_email": {"newperson@example.org"}}
	rec := gateReq(t, s, http.MethodPost, "/onboarding/invite", "sponsor@example.org", goneSub, "", form)
	invs, err := s.Onboard.ListInvites(context.Background(), "")
	if err != nil {
		t.Fatal(err)
	}
	if rec.Code != http.StatusForbidden || len(invs) != 0 {
		t.Errorf("POST /onboarding/invite as an inactive sponsor: status %d, invites minted %d; want 403 and 0", rec.Code, len(invs))
	}
}

func TestOnboardGate_activeSponsorPasses(t *testing.T) {
	const sub = "0b1f0000-0000-4000-8000-0000000000c4"
	kc, _ := fakeKratosMeta(t, sub, "sponsor2@example.org", "Pat", nil, "active")
	s := newUsersAPITestServer(t, kc)
	s.Cfg.LiveIdentityCheck = true
	s.Cfg.NotifyGoatnet = "zagros"
	portalURL, _ := fakeSponsorPortal(t, nil, map[string]sponsorLookupResponse{
		"sponsor2@example.org": {Org: "0ff404de", Alias: "pat-example2", SponsorAuthorized: true, SponsorMaxTier: "developer"},
	})
	s.Cfg.PortalInternalURL = portalURL
	s.Cfg.PortalLookupToken = "svc-token"
	if rec := gateReq(t, s, http.MethodGet, "/onboarding/invite", "sponsor2@example.org", sub, "", nil); rec.Code != http.StatusOK {
		t.Fatalf("active sponsor at /onboarding/invite: status %d, want 200", rec.Code)
	}
}

// The sign-in token's groups claim does not carry an identity's admin groups
// (they live in metadata_public, which every admin page reads through the live
// identity check). The onboarding gate must open its admin branch on the same
// live record, or an admin with no sponsorship row is refused the queue that
// every other admin page lets them reach.
func TestOnboardGate_adminByLiveRecordPassesWithoutGroupsClaim(t *testing.T) {
	const sub = "0b1f0000-0000-4000-8000-0000000000c5"
	kc, _ := fakeKratosMeta(t, sub, "admin@example.org", "Admin", []string{"PLATFORM-ADMIN"}, "active")
	s := newUsersAPITestServer(t, kc)
	s.Cfg.LiveIdentityCheck = true
	s.Cfg.NotifyGoatnet = "zagros"
	portalURL, _ := fakeSponsorPortal(t, nil, map[string]sponsorLookupResponse{})
	s.Cfg.PortalInternalURL = portalURL
	s.Cfg.PortalLookupToken = "svc-token"
	if rec := gateReq(t, s, http.MethodGet, "/onboarding", "admin@example.org", sub, "", nil); rec.Code != http.StatusOK {
		t.Fatalf("admin by live record, no groups claim, at /onboarding: status %d, want 200 (body %q)", rec.Code, rec.Body.String())
	}
}

// The admin branch must not open on the live record for someone who is not an
// admin there: a non-admin with no sponsorship row is still refused.
func TestOnboardGate_nonAdminWithoutSponsorshipStillRefused(t *testing.T) {
	const sub = "0b1f0000-0000-4000-8000-0000000000c6"
	kc, _ := fakeKratosMeta(t, sub, "viewer@example.org", "Viewer", []string{"VIEWER"}, "active")
	s := newUsersAPITestServer(t, kc)
	s.Cfg.LiveIdentityCheck = true
	s.Cfg.NotifyGoatnet = "zagros"
	portalURL, _ := fakeSponsorPortal(t, nil, map[string]sponsorLookupResponse{})
	s.Cfg.PortalInternalURL = portalURL
	s.Cfg.PortalLookupToken = "svc-token"
	if rec := gateReq(t, s, http.MethodGet, "/onboarding", "viewer@example.org", sub, "", nil); rec.Code != http.StatusForbidden {
		t.Fatalf("non-admin without sponsorship at /onboarding: status %d, want 403", rec.Code)
	}
}
