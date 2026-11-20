package handlers

import (
	"context"
	"net/http"
	"net/url"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// Option A review F1: reactivation in the console is gated exactly like
// deactivation: an IAM-ADMIN may not reactivate a deactivated platform admin
// or an unmigrated identity; a platform admin may, and anyone may reactivate
// a viewer within their tier.
func TestRV7788optA_F1_BrowserReactivationGated(t *testing.T) {
	s, rt := r7Server(t)
	rt.ids[r7PA2]["state"] = "inactive"
	rt.ids[r7U] = map[string]interface{}{"id": r7U, "state": "inactive",
		"traits": map[string]interface{}{"email": "u@example.org", "name": "u", "groups": []interface{}{"VIEWER"}}}
	for _, id := range []string{r7PA2, r7U} {
		if rec := r7Post(t, s, r7AsIAM, "/users/"+id+"/state?idp=tenant", url.Values{"state": {"active"}}); rec.Code != http.StatusForbidden || rt.ids[id]["state"] != "inactive" {
			t.Fatalf("IAM-ADMIN reactivating %s: %d, state %v; want 403 and inactive", id, rec.Code, rt.ids[id]["state"])
		}
	}
	if rec := r7Post(t, s, r7AsPA, "/users/"+r7PA2+"/state?idp=tenant", url.Values{"state": {"active"}}); rec.Code == http.StatusForbidden || rt.ids[r7PA2]["state"] != "active" {
		t.Fatalf("PLATFORM-ADMIN reactivating a platform admin: %d, state %v", rec.Code, rt.ids[r7PA2]["state"])
	}
	rt.ids[r7U] = r7Ident(r7U, "u@example.org", "VIEWER")
	rt.ids[r7U]["state"] = "inactive"
	if rec := r7Post(t, s, r7AsIAM, "/users/"+r7U+"/state?idp=tenant", url.Values{"state": {"active"}}); rec.Code == http.StatusForbidden || rt.ids[r7U]["state"] != "active" {
		t.Fatalf("IAM-ADMIN reactivating a migrated viewer: %d, state %v", rec.Code, rt.ids[r7U]["state"])
	}
}

// Option A review F2: every console decision that creates, re-queues or
// reactivates the platform tier asks a PLATFORM-ADMIN at aal1 for a fresh
// step-up: sponsor-approve, approve, retry and restore of a platform-tier
// row, and reactivating a platform-tier login. Nothing changes until then.
func TestRV7788optA_F2_PlatformTierDecisionsTakeStepUp(t *testing.T) {
	s, rt := r7Server(t)
	rt.stale = true
	status := func(id string) string {
		got, _ := s.Onboard.GetProposal(context.Background(), id)
		return got.Status
	}
	mk := func(handle, st string) *onboardstore.Proposal {
		p := r7Proposal(t, s, handle, onboardstore.ProposalProposed, "", []string{"PLATFORM-ADMIN"})
		r7SetStatus(t, s, p.ID, st)
		return p
	}
	for _, c := range []struct{ action, from string }{
		{"sponsor-approve", onboardstore.ProposalProposed},
		{"approve", onboardstore.ProposalSponsorApproved},
		{"retry", onboardstore.ProposalFailed},
		{"restore", onboardstore.ProposalArchived},
	} {
		p := mk("pa-su-"+c.action, c.from)
		wantStepUpRedirect7788(t, c.action+" of a platform-tier row at aal1",
			r7Post(t, s, r7AsPA, "/onboarding/"+p.ID+"/"+c.action+"?idp=tenant", url.Values{"relationship": {"assigned"}}))
		if status(p.ID) != c.from {
			t.Fatalf("%s at aal1 moved the row to %s", c.action, status(p.ID))
		}
	}
	rt.ids[r7PA2]["state"] = "inactive"
	wantStepUpRedirect7788(t, "reactivating a platform admin at aal1",
		r7Post(t, s, r7AsPA, "/users/"+r7PA2+"/state?idp=tenant", url.Values{"state": {"active"}}))
	if rt.ids[r7PA2]["state"] != "inactive" {
		t.Fatalf("reactivated at aal1")
	}
	// Below the platform tier nothing new is asked.
	d := r7Proposal(t, s, "dev-su", onboardstore.ProposalSponsorApproved, "developer", []string{"DEVELOPER"})
	if rec := r7Post(t, s, r7AsPA, "/onboarding/"+d.ID+"/approve?idp=tenant", url.Values{"relationship": {"assigned"}}); rec.Code == http.StatusSeeOther && rec.Header().Get("Location") != "" && len(rec.Header().Get("Location")) > 8 && rec.Header().Get("Location")[:8] == "/step-up" {
		t.Fatalf("a developer approval asked for step-up")
	}
}

// Option A review F3: the per-person bookkeeping caps rank the person's login
// as well as the row: a row filed as developer for a platform admin (or for
// an unmigrated person) is not the per-person key's to mark offboarded.
func TestRV7788optA_F3_BookkeepingRanksThePersonsLogin(t *testing.T) {
	s, rt := r7Server(t)
	r7OwnKey(t, s, "k-casey", "casey", "iam@example.org")
	_ = s.Routes()
	mk := func(handle, email string) *onboardstore.Proposal {
		p := &onboardstore.Proposal{Sponsor: "x@example.org", Env: "zagros", Handle: handle, Email: email,
			Status: onboardstore.ProposalOnboarded, Tier: "developer", ManifestJSON: "{}", Complete: true}
		if err := s.Onboard.CreateProposal(context.Background(), p); err != nil {
			t.Fatal(err)
		}
		return p
	}
	// pa2@example.org holds PLATFORM-ADMIN; the unmigrated identity has no metadata.
	unmID := "88888888-8888-4888-8888-888888888888"
	rt.ids[unmID] = map[string]interface{}{"id": unmID, "state": "active",
		"traits": map[string]interface{}{"email": "unm@example.org", "name": "unm", "groups": []interface{}{"DEVELOPER"}}}
	for _, email := range []string{"pa2@example.org", "unm@example.org"} {
		p := mk("row-"+email[:3], email)
		if w := r7API(t, s, "k-casey", "/api/onboard/proposals/"+p.ID+"/offboard", `{"note":"bye"}`); w.Code != http.StatusForbidden {
			t.Fatalf("per-person offboard of a developer row for %s: %d %s, want 403", email, w.Code, w.Body.String())
		}
	}
	// A developer row for a migrated viewer is the key's to mark.
	p := mk("row-u", "u@example.org")
	if w := r7API(t, s, "k-casey", "/api/onboard/proposals/"+p.ID+"/offboard", `{"note":"bye"}`); w.Code != http.StatusOK {
		t.Fatalf("per-person offboard of a viewer's row: %d %s, want 200", w.Code, w.Body.String())
	}
}
