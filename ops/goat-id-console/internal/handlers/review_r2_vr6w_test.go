package handlers

import (
	"net/http"
	"net/url"
	"strings"
	"testing"
)

// Option A round 2 F1 (vr6w alone): an offboarded platform admin whose
// console session still forwards headers cannot reactivate their own login:
// not by the plain identity id, an upper-cased one, or a stale session that
// carries only their address.
func TestTierCeiling_optA_R2F1_noSelfReactivationOfPlatformTier(t *testing.T) {
	s, rt := tcServer(t)
	rt.ids[tcPA]["state"] = "inactive"
	for _, as := range []tcActor{
		{"pa@example.org", tcPA, "PLATFORM-ADMIN"},
		{"pa@example.org", strings.ToUpper(tcPA), "PLATFORM-ADMIN"},
		{"pa@example.org", "12121212-1212-4212-8212-121212121212", "PLATFORM-ADMIN"},
	} {
		if rec := tcPost(t, s, as, "/users/"+tcPA+"/state?idp=tenant", url.Values{"state": {"active"}}); rec.Code != http.StatusForbidden || rt.ids[tcPA]["state"] != "inactive" {
			t.Fatalf("self reactivation as %q/%q: %d, state %v; want 403 and inactive", as.email, as.id, rec.Code, rt.ids[tcPA]["state"])
		}
	}
	other := tcActor{"pa2@example.org", "99999999-9999-4999-8999-999999999999", "PLATFORM-ADMIN"}
	rt.ids[other.id] = tcIdent(other.id, other.email, "PLATFORM-ADMIN")
	if rec := tcPost(t, s, other, "/users/"+tcPA+"/state?idp=tenant", url.Values{"state": {"active"}}); rec.Code == http.StatusForbidden || rt.ids[tcPA]["state"] != "active" {
		t.Fatalf("another platform admin reactivating: %d, state %v", rec.Code, rt.ids[tcPA]["state"])
	}
}
