package handlers

import (
	"net/http"
	"net/url"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/kratos"
)

// fakeKratosMigrated7788 is fakeKratos for a migrated identity: its groups
// are in admin-managed metadata_public (ADR 1188), not only in traits.
func fakeKratosMigrated7788(t *testing.T, id, email, name string, groups []string, state string) (*kratos.Client, func() ([]string, string)) {
	t.Helper()
	gi := make([]interface{}, len(groups))
	for i, g := range groups {
		gi[i] = g
	}
	cur := map[string]interface{}{
		"id": id, "state": state,
		"traits":          map[string]interface{}{"email": email, "name": name, "groups": gi},
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

// An identity with no metadata_public.groups (not migrated) ranks unknown:
// whatever its self-editable traits say, it is protected as the platform
// tier. A per-person key may not act on it, an IAM-ADMIN may not recover or
// take it down, and a PLATFORM-ADMIN may. Once its metadata holds groups, it
// ranks by them.
func TestRV7788u_UnmigratedIdentityRanksUnknown(t *testing.T) {
	s, rt := r7Server(t)
	r7OwnKey(t, s, "k-casey", "casey", "iam@example.org")
	// A viewer whose groups live only in traits (as the person may set them).
	rt.ids[r7U] = map[string]interface{}{
		"id": r7U, "state": "active",
		"traits": map[string]interface{}{"email": "u@example.org", "name": "u", "groups": []interface{}{"VIEWER"}},
	}
	if w := r7API(t, s, "k-casey", "/api/users/state", `{"id":"`+r7U+`","state":"inactive"}`); w.Code != http.StatusForbidden {
		t.Fatalf("per-person key on an unmigrated identity: %d %s, want 403", w.Code, w.Body.String())
	}
	if w := r7API(t, s, "k-casey", "/api/users/groups", `{"id":"`+r7U+`","groups":["VIEWER","DEVELOPER"]}`); w.Code != http.StatusForbidden {
		t.Fatalf("per-person key changing an unmigrated identity's groups: %d %s, want 403", w.Code, w.Body.String())
	}
	if rec := r7Post(t, s, r7AsIAM, "/users/"+r7U+"/revoke-sessions?idp=tenant", url.Values{}); rec.Code != http.StatusForbidden {
		t.Fatalf("IAM-ADMIN revoke-sessions of an unmigrated identity: %d, want 403", rec.Code)
	}
	if rec := r7Post(t, s, r7AsPA, "/users/"+r7U+"/revoke-sessions?idp=tenant", url.Values{}); rec.Code == http.StatusForbidden {
		t.Fatalf("PLATFORM-ADMIN revoke-sessions of an unmigrated identity: 403 %s", rec.Body.String())
	}
	// Traits claiming nothing do not lower protection either: an unmigrated
	// platform admin whose traits were emptied is still protected.
	rt.ids[r7PA2] = map[string]interface{}{
		"id": r7PA2, "state": "active",
		"traits": map[string]interface{}{"email": "pa2@example.org", "name": "pa2", "groups": []interface{}{}},
	}
	if rec := r7Post(t, s, r7AsIAM, "/users/"+r7PA2+"/recovery-link?idp=tenant", url.Values{}); rec.Code != http.StatusForbidden {
		t.Fatalf("IAM-ADMIN recovery link for an unmigrated identity with empty traits: %d, want 403", rec.Code)
	}
	// Migrated: ranked by metadata.
	rt.ids[r7U] = r7Ident(r7U, "u@example.org", "VIEWER")
	if w := r7API(t, s, "k-casey", "/api/users/state", `{"id":"`+r7U+`","state":"inactive"}`); w.Code != http.StatusOK {
		t.Fatalf("per-person key on a migrated viewer: %d %s, want 200", w.Code, w.Body.String())
	}
}
