package handlers

import (
	"context"
	"encoding/json"
	"net/http"
	"net/url"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// r5: every way into the runner's mint queue (status approved) passes the
// same check, not only the approve verbs: register -> archive -> restore,
// retry, and a direct status write, in both realms.

func r7SetStatus(t *testing.T, s *Server, id, status string) {
	t.Helper()
	if err := s.Onboard.SetProposalStatus(context.Background(), id, status); err != nil {
		t.Fatal(err)
	}
}

func TestRV7788r5_PerPersonKeyCannotQueueAdminMintBySideDoor(t *testing.T) {
	s, _ := r7Server(t)
	r7OwnKey(t, s, "k-casey", "casey", "iam@example.org")
	s.Cfg.OnboardOperatorAPIKeys["k-svc"] = "onboard-runner-zagros"
	_ = s.Routes() // installs the store check, as the server does at start

	var m map[string]any
	_ = json.Unmarshal([]byte(gateCleanManifest), &m)
	m["identity"].(map[string]any)["handle"] = "puppet"
	m["identity"].(map[string]any)["email"] = "puppet@evil.example"
	m["ory_site"].(map[string]any)["groups"] = []string{"PLATFORM-ADMIN"}
	mj, _ := json.Marshal(m)
	body, _ := json.Marshal(map[string]string{"handle": "puppet", "env": "zagros", "email": "puppet@evil.example",
		"manifest_json": string(mj), "tier": "platform-admin"})
	// r7: a per-person key may not register an admin row (bookkeeping
	// records no admin the person was never approved for).
	w := r7API(t, s, "k-casey", "/api/onboard/register", string(body))
	if w.Code != http.StatusForbidden {
		t.Fatalf("per-person register of a PLATFORM-ADMIN row: %d %s, want 403", w.Code, w.Body.String())
	}
	if ps, _ := s.Onboard.ListProposalsFiltered(context.Background(), "", "puppet@evil.example"); len(ps) != 0 {
		t.Fatalf("refused register left %d rows", len(ps))
	}
	// The row exists when a service key registers it (ADR 1071); the
	// per-person key then archives it and tries to restore it.
	w = r7API(t, s, "k-svc", "/api/onboard/register", string(body))
	if w.Code != http.StatusCreated {
		t.Fatalf("service register: %d %s", w.Code, w.Body.String())
	}
	var reg struct {
		ID string `json:"id"`
	}
	_ = json.Unmarshal(w.Body.Bytes(), &reg)
	// Revision 8: the per-person key may not archive it either; the service
	// key does.
	if w := r7API(t, s, "k-casey", "/api/onboard/proposals/"+reg.ID+"/archive", ""); w.Code != http.StatusForbidden {
		t.Fatalf("per-person archive of a PLATFORM-ADMIN row: %d %s, want 403", w.Code, w.Body.String())
	}
	if w := r7API(t, s, "k-svc", "/api/onboard/proposals/"+reg.ID+"/archive", ""); w.Code != http.StatusOK {
		t.Fatalf("service archive: %d %s", w.Code, w.Body.String())
	}
	if w := r7API(t, s, "k-casey", "/api/onboard/proposals/"+reg.ID+"/restore", ""); w.Code != http.StatusForbidden {
		t.Fatalf("per-person restore of a PLATFORM-ADMIN row: %d %s, want 403", w.Code, w.Body.String())
	}
	if got, _ := s.Onboard.GetProposal(context.Background(), reg.ID); got.Status != onboardstore.ProposalArchived {
		t.Fatalf("status %s, want archived", got.Status)
	}

	// Retry of a failed admin row, by API and by the decide route.
	f := r7Proposal(t, s, "pa-fail", onboardstore.ProposalProposed, "", []string{"IAM-ADMIN"})
	r7SetStatus(t, s, f.ID, onboardstore.ProposalFailed)
	if w := r7API(t, s, "k-casey", "/api/onboard/proposals/"+f.ID+"/retry", ""); w.Code != http.StatusForbidden {
		t.Fatalf("per-person retry of an IAM-ADMIN row: %d %s, want 403", w.Code, w.Body.String())
	}
	if got, _ := s.Onboard.GetProposal(context.Background(), f.ID); got.Status != onboardstore.ProposalFailed {
		t.Fatalf("status %s, want failed", got.Status)
	}

	// The store refuses the write itself for a per-person principal.
	ctx := context.WithValue(context.Background(), ctxKeyOnboardPrincipal, onboardPrincipal{Handle: "casey", IsOperator: true})
	if err := s.Onboard.SetProposalStatus(ctx, f.ID, onboardstore.ProposalApproved); err == nil {
		t.Fatal("store status write into approved by a per-person key: allowed")
	}

	// A service key may not re-queue the platform tier either (ADR 1071 as
	// amended); a per-person key keeps non-admin rows.
	if w := r7API(t, s, "k-svc", "/api/onboard/proposals/"+reg.ID+"/restore", ""); w.Code != http.StatusForbidden {
		t.Fatalf("service key restore of a PLATFORM-ADMIN row: %d %s, want 403", w.Code, w.Body.String())
	}
	// A per-person key still registers a non-admin row.
	devBody, _ := json.Marshal(map[string]string{"handle": "devreg", "env": "zagros", "email": "devreg@example.org",
		"manifest_json": "{}", "tier": "developer"})
	if w := r7API(t, s, "k-casey", "/api/onboard/register", string(devBody)); w.Code != http.StatusCreated {
		t.Fatalf("per-person register of a developer row: %d %s", w.Code, w.Body.String())
	}
	d := r7Proposal(t, s, "dev-fail", onboardstore.ProposalProposed, "developer", []string{"DEVELOPER"})
	r7SetStatus(t, s, d.ID, onboardstore.ProposalFailed)
	// (The fixture manifest is not a full provisioning manifest, so the
	// store's own manifest check answers 409; the admission check must not.)
	if w := r7API(t, s, "k-casey", "/api/onboard/proposals/"+d.ID+"/retry", ""); w.Code == http.StatusForbidden {
		t.Fatalf("per-person retry of a developer row: %d %s", w.Code, w.Body.String())
	}
}

func TestRV7788r5_BrowserRestoreAndRetryRankTheRow(t *testing.T) {
	s, _ := r7Server(t)
	asIAM := r7Actor{"iam@example.org", r7IAM, "IAM-ADMIN,GRAFANA-READONLY"}
	p := r7Proposal(t, s, "pa-arch", onboardstore.ProposalProposed, "platform-admin", []string{"PLATFORM-ADMIN"})
	r7SetStatus(t, s, p.ID, onboardstore.ProposalArchived)
	if rec := r7Post(t, s, asIAM, "/onboarding/"+p.ID+"/restore?idp=tenant", url.Values{}); rec.Code != http.StatusForbidden {
		t.Fatalf("IAM-ADMIN restore of a PLATFORM-ADMIN row: %d, want 403", rec.Code)
	}
	if got, _ := s.Onboard.GetProposal(context.Background(), p.ID); got.Status != onboardstore.ProposalArchived {
		t.Fatalf("status %s, want archived", got.Status)
	}
	f := r7Proposal(t, s, "pa-f2", onboardstore.ProposalProposed, "", []string{"PLATFORM-ADMIN"})
	r7SetStatus(t, s, f.ID, onboardstore.ProposalFailed)
	r7Post(t, s, asIAM, "/onboarding/"+f.ID+"/retry?idp=tenant", url.Values{})
	if got, _ := s.Onboard.GetProposal(context.Background(), f.ID); got.Status != onboardstore.ProposalFailed {
		t.Fatalf("IAM-ADMIN retry of a PLATFORM-ADMIN row: status %s, want failed", got.Status)
	}
	// A platform admin restores it.
	if rec := r7Post(t, s, r7AsPA, "/onboarding/"+p.ID+"/restore?idp=tenant", url.Values{}); rec.Code == http.StatusForbidden {
		t.Fatalf("platform admin restore: 403 %s", rec.Body.String())
	}
	if got, _ := s.Onboard.GetProposal(context.Background(), p.ID); got.Status != onboardstore.ProposalApproved {
		t.Fatalf("platform admin restore: status %s, want approved", got.Status)
	}
}
