package handlers

import (
	"context"
	"errors"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/permissions"
)

// IAM-ADMIN passes adminGate, so it is admin-bearing like platform-admin,
// and so is any group the permissions matrix grants console admin.
func TestAdminBearingGroups(t *testing.T) {
	kc, _ := fakeKratos(t, abOtherID, "x@example.org", "X", nil, "active")
	srv := newUsersAPITestServer(t, kc)
	srv.Permissions.Groups = append(srv.Permissions.Groups,
		permissions.Group{Name: "CONSOLE-OPS", Grants: []permissions.Grant{{Service: "id-admin-console", Access: "admin"}}},
		permissions.Group{Name: "VIEWER"})
	for _, g := range [][]string{{"PLATFORM-ADMIN"}, {"ADMINS"}, {"IAM-ADMIN"}, {"VIEWER", "IAM-ADMIN"}, {"CONSOLE-OPS"}} {
		if !srv.isAdminBearingGroupList(g) {
			t.Errorf("isAdminBearingGroupList(%v) = false", g)
		}
	}
	if srv.isAdminBearingGroupList([]string{"VIEWER"}) {
		t.Error("VIEWER is not admin-bearing")
	}
	if srv.adminBearingRank([]string{"CONSOLE-OPS"}) != 1 || srv.adminBearingRank([]string{"ADMINS"}) != 2 {
		t.Error("matrix admin group must rank with IAM-ADMIN, ADMINS with PLATFORM-ADMIN")
	}
	if g, r := srv.adminBearingDelta([]string{"VIEWER"}, []string{"VIEWER", "IAM-ADMIN"}); !g || r {
		t.Errorf("adding IAM-ADMIN: granted=%v removed=%v", g, r)
	}
	if g, r := srv.adminBearingDelta([]string{"CONSOLE-OPS"}, nil); g || !r {
		t.Errorf("removing CONSOLE-OPS: granted=%v removed=%v", g, r)
	}
	if g, r := srv.adminBearingDelta([]string{"IAM-ADMIN"}, []string{"IAM-ADMIN", "VIEWER"}); g || r {
		t.Errorf("non-admin change: granted=%v removed=%v", g, r)
	}
}

const (
	abSelfID  = "aaaaaaaa-1111-4111-8111-111111111111"
	abOtherID = "bbbbbbbb-2222-4222-8222-222222222222"
)

// An admin cannot add an admin-bearing group to their own identity through
// the browser path, whatever their second factor says. Self is the session's
// identity id, compared case-insensitively with the id Kratos returns, so an
// upper-cased path id or a changed email does not hide the actor.
func TestApplyGroupChange_refusesSelfGrant(t *testing.T) {
	kc, read := fakeKratos(t, abSelfID, "iam@example.org", "Casey", []string{"IAM-ADMIN"}, "active")
	srv := newUsersAPITestServer(t, kc)
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/users/"+abSelfID+"/groups", nil)
	browser := groupChangeOpts{enforceStepUp: true}
	cases := []struct {
		name  string
		id    string
		actor Actor
	}{
		{"same id", abSelfID, Actor{Email: "iam@example.org", IdentityID: abSelfID, Groups: []string{"IAM-ADMIN"}}},
		{"upper-case path id, changed email", strings.ToUpper(abSelfID), Actor{Email: "old@example.org", IdentityID: abSelfID, Groups: []string{"IAM-ADMIN"}}},
		{"upper-case session id", abSelfID, Actor{Email: "old@example.org", IdentityID: strings.ToUpper(abSelfID), Groups: []string{"IAM-ADMIN"}}},
		{"no identity id", abSelfID, Actor{Email: "someone@example.org", Groups: []string{"IAM-ADMIN"}}},
	}
	for _, c := range cases {
		for _, grant := range [][]string{{"IAM-ADMIN", "PLATFORM-ADMIN"}, {"IAM-ADMIN", "ADMINS"}} {
			_, err := srv.applyGroupChange(httptest.NewRecorder(), req, tenantIdPKey, c.id, grant, c.actor, browser)
			var gce *groupChangeError
			if !errors.As(err, &gce) || gce.status != http.StatusForbidden {
				t.Fatalf("%s: self grant of %v: err=%v, want 403", c.name, grant, err)
			}
		}
	}
	if groups, _ := read(); containsString(groups, "PLATFORM-ADMIN") || containsString(groups, "ADMINS") {
		t.Fatalf("self grant landed in Kratos: %v", groups)
	}
	// A platform admin granting themselves IAM-ADMIN is a self grant too.
	kc, read = fakeKratos(t, abSelfID, "pa@example.org", "Pat", []string{"PLATFORM-ADMIN"}, "active")
	srv = newUsersAPITestServer(t, kc)
	pa := Actor{Email: "pa@example.org", IdentityID: abSelfID, Groups: []string{"PLATFORM-ADMIN"}}
	_, err := srv.applyGroupChange(httptest.NewRecorder(), req, tenantIdPKey, abSelfID,
		[]string{"PLATFORM-ADMIN", "IAM-ADMIN"}, pa, browser)
	var gce *groupChangeError
	if !errors.As(err, &gce) || gce.status != http.StatusForbidden {
		t.Fatalf("self grant of IAM-ADMIN by a platform admin: err=%v, want 403", err)
	}
	if groups, _ := read(); containsString(groups, "IAM-ADMIN") {
		t.Fatalf("self grant landed in Kratos: %v", groups)
	}
}

// Granting IAM-ADMIN to someone else asks for step-up, as platform-admin does.
func TestApplyGroupChange_iamAdminGrantTakesStepUp(t *testing.T) {
	kc, read := fakeKratos(t, "id-2", "new@example.org", "Robin", nil, "active")
	srv := newUsersAPITestServer(t, kc)
	srv.Permissions.StepUpRequired = append(srv.Permissions.StepUpRequired, "grant_ADMINS")
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/users/id-2/groups", nil)
	other := Actor{Email: "admin@example.org", IdentityID: "id-9", Groups: []string{"PLATFORM-ADMIN"}}
	_, err := srv.applyGroupChange(httptest.NewRecorder(), req, tenantIdPKey, "id-2",
		[]string{"IAM-ADMIN"}, other, groupChangeOpts{enforceStepUp: true})
	if !errors.Is(err, errStepUpHandled) {
		t.Fatalf("IAM-ADMIN grant without a fresh second factor: err=%v, want the step-up challenge", err)
	}
	if groups, _ := read(); containsString(groups, "IAM-ADMIN") {
		t.Fatalf("IAM-ADMIN landed without step-up: %v", groups)
	}
}
