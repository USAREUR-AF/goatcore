package handlers

import (
	"context"
	"net/http"
	"net/url"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// Option A review F2 (vr6w alone too): every console decision that creates,
// re-queues or reactivates the platform tier asks a PLATFORM-ADMIN at aal1
// for a fresh step-up, and nothing changes until then. Review F1 (closed by
// targetGate in this patch): an IAM-ADMIN may not reactivate a platform
// admin at all.
func TestTierCeiling_optA_platformTierDecisionsTakeStepUp(t *testing.T) {
	s, rt := tgKeyServer(t)
	status := func(id string) string {
		got, _ := s.Onboard.GetProposal(context.Background(), id)
		return got.Status
	}
	rt.ids[tcPA]["state"] = "inactive"
	if rec := tcPost(t, s, tcAsIAM, "/users/"+tcPA+"/state?idp=tenant", url.Values{"state": {"active"}}); rec.Code != http.StatusForbidden || rt.ids[tcPA]["state"] != "inactive" {
		t.Fatalf("IAM-ADMIN reactivating a platform admin: %d, state %v; want 403", rec.Code, rt.ids[tcPA]["state"])
	}
	rt.stale = true
	asPA2 := tcActor{"pa2@example.org", "99999999-9999-4999-8999-999999999999", "PLATFORM-ADMIN"}
	wantStepUpRedirect(t, "reactivating a platform admin at aal1",
		tcPost(t, s, asPA2, "/users/"+tcPA+"/state?idp=tenant", url.Values{"state": {"active"}}))
	if rt.ids[tcPA]["state"] != "inactive" {
		t.Fatal("reactivated at aal1")
	}
	for _, c := range []struct{ action, from string }{
		{"sponsor-approve", onboardstore.ProposalProposed},
		{"approve", onboardstore.ProposalSponsorApproved},
		{"retry", onboardstore.ProposalFailed},
		{"restore", onboardstore.ProposalArchived},
	} {
		p := tgProposal(t, s, "pa-su-"+c.action, c.from, "", tgManifest("pa-su-"+c.action, []string{"PLATFORM-ADMIN"}))
		wantStepUpRedirect(t, c.action+" of a platform-tier row at aal1",
			tcPost(t, s, asPA2, "/onboarding/"+p.ID+"/"+c.action+"?idp=tenant", url.Values{"relationship": {"assigned"}}))
		if status(p.ID) != c.from {
			t.Fatalf("%s at aal1 moved the row to %s", c.action, status(p.ID))
		}
	}
	before, _ := s.Onboard.ListInvites(context.Background(), "")
	wantStepUpRedirect(t, "a platform-tier onboarding invite at aal1",
		tcPost(t, s, asPA2, "/onboarding/invite?idp=tenant", url.Values{"tier": {"platform-admin"}, "ttl_hours": {"24"}, "env": {"zagros"}}))
	if after, _ := s.Onboard.ListInvites(context.Background(), ""); len(after) != len(before) {
		t.Fatal("an invite was minted at aal1")
	}
	// Below the platform tier nothing new is asked.
	d := tgProposal(t, s, "dev-su", onboardstore.ProposalSponsorApproved, "developer", tgManifest("dev-su", []string{"DEVELOPER"}))
	if rec := tcPost(t, s, asPA2, "/onboarding/"+d.ID+"/approve?idp=tenant", url.Values{"relationship": {"assigned"}}); strings.HasPrefix(rec.Header().Get("Location"), "/step-up") {
		t.Fatal("a developer approval asked for step-up")
	}
}
