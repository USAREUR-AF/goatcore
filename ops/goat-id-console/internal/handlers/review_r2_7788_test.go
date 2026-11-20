package handlers

import (
	"context"
	"net/http"
	"net/url"
	"strings"
	"testing"
)

// Option A round 2 F1 (7788 alone): an offboarded platform admin whose
// console session still forwards headers cannot reactivate their own login:
// not by the plain identity id, an upper-cased one, or a stale session that
// carries only their address.
func TestRV7788optA_R2F1_NoSelfReactivationOfPlatformTier(t *testing.T) {
	s, rt := r7Server(t)
	rt.ids[r7PA2]["state"] = "inactive"
	for _, as := range []r7Actor{
		{"pa2@example.org", r7PA2, "PLATFORM-ADMIN"},
		{"pa2@example.org", strings.ToUpper(r7PA2), "PLATFORM-ADMIN"},
		{"pa2@example.org", "12121212-1212-4212-8212-121212121212", "PLATFORM-ADMIN"},
	} {
		if rec := r7Post(t, s, as, "/users/"+r7PA2+"/state?idp=tenant", url.Values{"state": {"active"}}); rec.Code != http.StatusForbidden || rt.ids[r7PA2]["state"] != "inactive" {
			t.Fatalf("self reactivation as %q/%q: %d, state %v; want 403 and inactive", as.email, as.id, rec.Code, rt.ids[r7PA2]["state"])
		}
	}
	// Another platform admin may (fresh session in this fixture).
	if rec := r7Post(t, s, r7AsPA, "/users/"+r7PA2+"/state?idp=tenant", url.Values{"state": {"active"}}); rec.Code == http.StatusForbidden || rt.ids[r7PA2]["state"] != "active" {
		t.Fatalf("another platform admin reactivating: %d, state %v", rec.Code, rt.ids[r7PA2]["state"])
	}
}

// Option A round 2 F2 (7788 alone): a console platform-tier onboarding
// invite asks for a fresh step-up at aal1 whatever the step-up policy lists;
// nothing is minted until then. A lower tier is not challenged.
func TestRV7788optA_R2F2_PlatformInviteTakesStepUp(t *testing.T) {
	s, rt := r7Server(t)
	s.Cfg.NotifyGoatnet = "zagros"
	rt.stale = true
	before, _ := s.Onboard.ListInvites(context.Background(), "")
	wantStepUpRedirect7788(t, "a platform-tier invite at aal1",
		r7Post(t, s, r7AsPA, "/onboarding/invite?idp=tenant", url.Values{"tier": {"platform-admin"}, "ttl_hours": {"24"}, "env": {"zagros"}}))
	if after, _ := s.Onboard.ListInvites(context.Background(), ""); len(after) != len(before) {
		t.Fatal("an invite was minted at aal1")
	}
	rec := r7Post(t, s, r7AsPA, "/onboarding/invite?idp=tenant", url.Values{"tier": {"developer"}, "ttl_hours": {"24"}, "env": {"zagros"}})
	if strings.HasPrefix(rec.Header().Get("Location"), "/step-up") {
		t.Fatal("a developer invite asked for step-up")
	}
}
