package handlers

import (
	"context"
	"encoding/json"
	"net/http"
	"net/url"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// ADR 1071 as amended (option A): the platform tier is invited, vouched
// for, approved, re-queued, reactivated and granted only by a browser
// PLATFORM-ADMIN session. No operator key does any of these. The runner's
// service key still claims a platform-tier proposal and posts its result,
// and a service key still deactivates, removes the platform tier and
// registers a login already minted.
func TestRV7788A_NoOperatorKeyCreatesThePlatformTier(t *testing.T) {
	s, rt := r7Server(t)
	r7OwnKey(t, s, "k-casey", "casey", "iam@example.org")
	s.Cfg.OnboardOperatorAPIKeys["k-svc"] = "operator"
	s.Cfg.OnboardOperatorAPIKeys["k-run"] = "onboard-runner-zagros"
	_ = s.Routes()
	status := func(id string) string {
		got, _ := s.Onboard.GetProposal(context.Background(), id)
		return got.Status
	}
	invite := func(key, tier string) int {
		b, _ := json.Marshal(map[string]any{"env": "zagros", "tier": tier, "ttl_hours": 24})
		return r7API(t, s, key, "/api/onboard/invite", string(b)).Code
	}
	// Invite.
	for _, key := range []string{"k-svc", "k-run", "k-casey"} {
		if c := invite(key, "platform-admin"); c != http.StatusForbidden {
			t.Fatalf("%s invite at platform-admin: %d, want 403", key, c)
		}
	}
	if c := invite("k-svc", "iam-admin"); c != http.StatusCreated {
		t.Fatalf("service invite at iam-admin: %d, want 201", c)
	}
	// Vouch and approve.
	p := r7Proposal(t, s, "pa-a", onboardstore.ProposalProposed, "", []string{"PLATFORM-ADMIN"})
	if w := r7API(t, s, "k-svc", "/api/onboard/proposals/"+p.ID+"/sponsor-approve", `{}`); w.Code != http.StatusForbidden || status(p.ID) != onboardstore.ProposalProposed {
		t.Fatalf("service sponsor-approve of a platform row: %d, status %s", w.Code, status(p.ID))
	}
	q := r7Proposal(t, s, "pa-b", onboardstore.ProposalSponsorApproved, "", []string{"PLATFORM-ADMIN"})
	if w := r7API(t, s, "k-run", "/api/onboard/proposals/"+q.ID+"/approve", `{}`); w.Code != http.StatusForbidden || status(q.ID) != onboardstore.ProposalSponsorApproved {
		t.Fatalf("runner-key approve of a platform row: %d, status %s", w.Code, status(q.ID))
	}
	// A browser PLATFORM-ADMIN approves it; the runner claims and reports.
	if rec := r7Post(t, s, r7AsPA, "/onboarding/"+q.ID+"/approve?idp=tenant", url.Values{"relationship": {"assigned"}}); status(q.ID) != onboardstore.ProposalApproved {
		t.Fatalf("browser PLATFORM-ADMIN approve: %d %s, status %s", rec.Code, rec.Body.String(), status(q.ID))
	}
	if w := r7API(t, s, "k-run", "/api/onboard/proposals/"+q.ID+"/claim", ""); w.Code != http.StatusOK {
		t.Fatalf("runner claim of a console-approved platform row: %d %s", w.Code, w.Body.String())
	}
	if w := r7API(t, s, "k-run", "/api/onboard/proposals/"+q.ID+"/result", `{"status":"failed","error":"x"}`); w.Code != http.StatusOK {
		t.Fatalf("runner result: %d %s", w.Code, w.Body.String())
	}
	// Re-queue (retry) of the platform row by a service key is refused.
	if w := r7API(t, s, "k-run", "/api/onboard/proposals/"+q.ID+"/retry", ""); w.Code != http.StatusForbidden || status(q.ID) != onboardstore.ProposalFailed {
		t.Fatalf("runner-key retry of a platform row: %d %s, status %s", w.Code, w.Body.String(), status(q.ID))
	}
	// Register of an already-minted platform login stays a service-key action.
	body, _ := json.Marshal(map[string]string{"handle": "pa-reg", "env": "zagros", "email": "pa-reg@example.org", "manifest_json": "{}", "tier": "platform-admin"})
	if w := r7API(t, s, "k-svc", "/api/onboard/register", string(body)); w.Code != http.StatusCreated {
		t.Fatalf("service register of a minted platform login: %d %s", w.Code, w.Body.String())
	}
	// Reactivation of a platform login is refused; deactivation is not.
	if w := r7API(t, s, "k-svc", "/api/users/state", `{"id":"`+r7PA2+`","state":"inactive"}`); w.Code != http.StatusOK {
		t.Fatalf("service deactivate of a platform admin: %d %s", w.Code, w.Body.String())
	}
	if w := r7API(t, s, "k-svc", "/api/users/state", `{"id":"`+r7PA2+`","state":"active"}`); w.Code != http.StatusForbidden || rt.ids[r7PA2]["state"] != "inactive" {
		t.Fatalf("service reactivate of a platform admin: %d %s, state %v", w.Code, w.Body.String(), rt.ids[r7PA2]["state"])
	}
	if w := r7API(t, s, "k-svc", "/api/users/state", `{"id":"`+r7U+`","state":"inactive"}`); w.Code != http.StatusOK {
		t.Fatalf("service deactivate of a viewer: %d %s", w.Code, w.Body.String())
	}
	if w := r7API(t, s, "k-svc", "/api/users/state", `{"id":"`+r7U+`","state":"active"}`); w.Code != http.StatusOK {
		t.Fatalf("service reactivate of a viewer: %d %s", w.Code, w.Body.String())
	}
}
