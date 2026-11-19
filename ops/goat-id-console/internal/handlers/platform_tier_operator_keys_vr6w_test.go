package handlers

import (
	"context"
	"net/http"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// ADR 1071 as amended (option A), the doors not covered elsewhere: no
// service key grants the platform tier through /api/users/groups or
// reactivates a platform-tier login through /api/users/state, while
// deactivation, other grants, and the runner's claim, result and archive of
// a platform-tier row are unchanged.
func TestTierCeiling_serviceKeyPlatformTierDoors(t *testing.T) {
	s, rt := tgKeyServer(t)
	groups := func(body string) int {
		return tgAPI(t, s, http.MethodPost, "k-runner", "/api/users/groups", body).Code
	}
	if c := groups(`{"id":"` + tcU + `","groups":["VIEWER","PLATFORM-ADMIN"]}`); c != http.StatusForbidden || containsString(rt.groups(tcU), "PLATFORM-ADMIN") {
		t.Fatalf("service grant of PLATFORM-ADMIN: %d groups %v", c, rt.groups(tcU))
	}
	if c := groups(`{"id":"` + tcU + `","groups":["VIEWER","IAM-ADMIN"]}`); c != http.StatusOK {
		t.Fatalf("service grant of IAM-ADMIN: %d", c)
	}
	state := func(id, st string) int {
		return tgAPI(t, s, http.MethodPost, "k-runner", "/api/users/state", `{"id":"`+id+`","state":"`+st+`"}`).Code
	}
	if c := state(tcPA, "inactive"); c != http.StatusOK {
		t.Fatalf("service deactivate of a platform admin: %d", c)
	}
	if c := state(tcPA, "active"); c != http.StatusForbidden || rt.ids[tcPA]["state"] != "inactive" {
		t.Fatalf("service reactivate of a platform admin: %d state %v", c, rt.ids[tcPA]["state"])
	}
	if c := state(tcU, "inactive"); c != http.StatusOK {
		t.Fatalf("service deactivate of a viewer: %d", c)
	}
	if c := state(tcU, "active"); c != http.StatusOK {
		t.Fatalf("service reactivate of a viewer: %d", c)
	}
	// The runner keeps claim, result and archive on a platform-tier row.
	p := tgProposal(t, s, "pa-run", onboardstore.ProposalApproved, "", tgManifest("pa-run", []string{"PLATFORM-ADMIN"}))
	for _, step := range []struct{ path, body string }{
		{"claim", ""}, {"result", `{"status":"onboarded","setup_key":"SK"}`}, {"archive", ""},
	} {
		if w := tgAPI(t, s, http.MethodPost, "k-runner", "/api/onboard/proposals/"+p.ID+"/"+step.path, step.body); w.Code != http.StatusOK {
			t.Fatalf("runner %s of a platform-tier row: %d %s", step.path, w.Code, w.Body.String())
		}
	}
	if got, _ := s.Onboard.GetProposal(context.Background(), p.ID); got.Status != onboardstore.ProposalArchived {
		t.Fatalf("status %s, want archived", got.Status)
	}
}
