package handlers

import (
	"net/http"
	"net/url"
	"testing"
)

// An identity with no metadata_public.groups (not migrated) ranks unknown,
// counted as the platform tier, whatever its self-editable traits say: an
// IAM-ADMIN may not recover it, reset its second factor, change its groups
// or take it down, and a per-person key may not act on it; a PLATFORM-ADMIN
// may. Once its metadata holds groups, it ranks by them.
func TestTierCeiling_unmigratedIdentityRanksUnknown(t *testing.T) {
	s, rt := tgKeyServer(t)
	unmigrated := func(id, email string, groups ...interface{}) {
		rt.ids[id] = map[string]interface{}{
			"id": id, "state": "active",
			"traits": map[string]interface{}{"email": email, "name": email, "groups": groups},
		}
	}
	// A viewer by its traits, an unmigrated platform admin with emptied traits.
	unmigrated(tcU, "u@example.org", "VIEWER")
	unmigrated(tcPA, "pa@example.org")
	for _, target := range []string{tcU, tcPA} {
		for _, c := range []struct {
			path string
			form url.Values
		}{
			{"/users/" + target + "/recovery-link?idp=tenant", url.Values{}},
			{"/users/" + target + "/reset-2fa?idp=tenant", url.Values{}},
			{"/users/" + target + "/groups?idp=tenant", url.Values{"groups": {"VIEWER"}}},
			{"/users/" + target + "/revoke-sessions?idp=tenant", url.Values{}},
		} {
			if rec := tcPost(t, s, tcAsIAM, c.path, c.form); rec.Code != http.StatusForbidden {
				t.Fatalf("IAM-ADMIN %s on an unmigrated identity: %d, want 403", c.path, rec.Code)
			}
		}
	}
	if w := tgAPI(t, s, http.MethodPost, "k-casey", "/api/users/state", `{"id":"`+tcU+`","state":"inactive"}`); w.Code != http.StatusForbidden {
		t.Fatalf("per-person key on an unmigrated identity: %d %s, want 403", w.Code, w.Body.String())
	}
	if rec := tcPost(t, s, tcAsPA, "/users/"+tcU+"/revoke-sessions?idp=tenant", url.Values{}); rec.Code == http.StatusForbidden {
		t.Fatalf("PLATFORM-ADMIN on an unmigrated identity: 403 %s", rec.Body.String())
	}
	// Migrated: ranked by metadata, so an IAM-ADMIN administers a viewer.
	rt.ids[tcU] = tcIdent(tcU, "u@example.org", "VIEWER")
	if rec := tcPost(t, s, tcAsIAM, "/users/"+tcU+"/revoke-sessions?idp=tenant", url.Values{}); rec.Code == http.StatusForbidden {
		t.Fatalf("IAM-ADMIN on a migrated viewer: 403 %s", rec.Body.String())
	}
}
