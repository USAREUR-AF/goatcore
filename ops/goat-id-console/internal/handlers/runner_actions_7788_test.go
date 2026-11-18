package handlers

import (
	"context"
	"errors"
	"net/http"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// r8: a per-person operator key gets the approve's cap on the runner-side
// lifecycle actions (claim, result, retry, archive), checked at the store
// write. Service keys (the runner) keep claim and result.
func TestRV7788r8_PerPersonKeyLifecycleActionsCapped(t *testing.T) {
	s, _ := r7Server(t)
	r7OwnKey(t, s, "k-casey", "casey", "iam@example.org")
	s.Cfg.OnboardOperatorAPIKeys["k-svc"] = "onboard-runner-zagros"
	_ = s.Routes()
	mk := func(handle, status string, groups []string) *onboardstore.Proposal {
		p := r7Proposal(t, s, handle, onboardstore.ProposalProposed, "", groups)
		r7SetStatus(t, s, p.ID, status)
		return p
	}
	status := func(id string) string {
		got, _ := s.Onboard.GetProposal(context.Background(), id)
		return got.Status
	}
	admin := []string{"PLATFORM-ADMIN"}
	for _, c := range []struct {
		name, from, path, body string
	}{
		{"claim", onboardstore.ProposalApproved, "claim", ""},
		{"result", onboardstore.ProposalOnboarding, "result", `{"status":"onboarded","setup_key":"SK"}`},
		{"retry", onboardstore.ProposalFailed, "retry", ""},
		{"archive", onboardstore.ProposalOnboarded, "archive", ""},
	} {
		p := mk("adm-"+c.name, c.from, admin)
		if w := r7API(t, s, "k-casey", "/api/onboard/proposals/"+p.ID+"/"+c.path, c.body); w.Code != http.StatusForbidden || status(p.ID) != c.from {
			t.Fatalf("per-person %s of a PLATFORM-ADMIN row: %d %s, status %s; want 403 and %s", c.name, w.Code, w.Body.String(), status(p.ID), c.from)
		}
		// The same action on a non-admin row is not refused by the cap.
		d := mk("dev-"+c.name, c.from, []string{"DEVELOPER"})
		if w := r7API(t, s, "k-casey", "/api/onboard/proposals/"+d.ID+"/"+c.path, c.body); w.Code == http.StatusForbidden {
			t.Fatalf("per-person %s of a developer row: 403 %s", c.name, w.Body.String())
		}
	}
	// The runner's service key claims and reports results on admin rows.
	p := mk("svc-claim", onboardstore.ProposalApproved, admin)
	if w := r7API(t, s, "k-svc", "/api/onboard/proposals/"+p.ID+"/claim", ""); w.Code != http.StatusOK {
		t.Fatalf("service claim: %d %s", w.Code, w.Body.String())
	}
	if w := r7API(t, s, "k-svc", "/api/onboard/proposals/"+p.ID+"/result", `{"status":"onboarded","setup_key":"SK"}`); w.Code != http.StatusOK {
		t.Fatalf("service result: %d %s", w.Code, w.Body.String())
	}
}

// rv6-7788 probe as a test: on an admin row a service key approved, a
// per-person key can neither claim it nor post a (failed) result on it, nor
// retry it; the runner's service key runs the whole loop.
func TestRV7788r9_PerPersonKeyRunnerLoopOnServiceApprovedAdminRow(t *testing.T) {
	s, _ := r7Server(t)
	r7OwnKey(t, s, "k-casey", "casey", "iam@example.org")
	s.Cfg.OnboardOperatorAPIKeys["k-svc"] = "onboard-runner-zagros"
	// An IAM-ADMIN row: a service key may approve it (it may not approve the
	// platform tier, ADR 1071 as amended), and a per-person key may not touch it.
	p := r7Proposal(t, s, "iam-loop", onboardstore.ProposalSponsorApproved, "", []string{"IAM-ADMIN"})
	if w := r7API(t, s, "k-svc", "/api/onboard/proposals/"+p.ID+"/approve", `{}`); w.Code != http.StatusOK {
		t.Fatalf("service approve: %d %s", w.Code, w.Body.String())
	}
	status := func() string {
		got, _ := s.Onboard.GetProposal(context.Background(), p.ID)
		return got.Status
	}
	if w := r7API(t, s, "k-casey", "/api/onboard/proposals/"+p.ID+"/claim", ""); w.Code != http.StatusForbidden || status() != onboardstore.ProposalApproved {
		t.Fatalf("per-person claim: %d %s, status %s; want 403 and approved", w.Code, w.Body.String(), status())
	}
	if w := r7API(t, s, "k-svc", "/api/onboard/proposals/"+p.ID+"/claim", ""); w.Code != http.StatusOK {
		t.Fatalf("service claim: %d %s", w.Code, w.Body.String())
	}
	if w := r7API(t, s, "k-casey", "/api/onboard/proposals/"+p.ID+"/result", `{"status":"failed","error":"x"}`); w.Code != http.StatusForbidden || status() != onboardstore.ProposalOnboarding {
		t.Fatalf("per-person failed result: %d %s, status %s; want 403 and onboarding", w.Code, w.Body.String(), status())
	}
	if w := r7API(t, s, "k-svc", "/api/onboard/proposals/"+p.ID+"/result", `{"status":"failed","error":"x"}`); w.Code != http.StatusOK {
		t.Fatalf("service failed result: %d %s", w.Code, w.Body.String())
	}
	if w := r7API(t, s, "k-casey", "/api/onboard/proposals/"+p.ID+"/retry", ""); w.Code != http.StatusForbidden || status() != onboardstore.ProposalFailed {
		t.Fatalf("per-person retry: %d %s, status %s; want 403 and failed", w.Code, w.Body.String(), status())
	}
	if w := r7API(t, s, "k-svc", "/api/onboard/proposals/"+p.ID+"/retry", ""); w.Code != http.StatusOK || status() != onboardstore.ProposalApproved {
		t.Fatalf("service retry: %d %s, status %s", w.Code, w.Body.String(), status())
	}
}

// r9: the decide and status writes into approved check and write the row
// in one transaction (FOR UPDATE). A per-person principal is refused at the
// store; a service key and a no-principal context are not.
func TestRV7788r9_LockedStatusWriteChecksTheRow(t *testing.T) {
	s, _ := r7Server(t)
	_ = s.Routes()
	p := r7Proposal(t, s, "pa-lock", onboardstore.ProposalSponsorApproved, "", []string{"PLATFORM-ADMIN"})
	per := context.WithValue(context.Background(), ctxKeyOnboardPrincipal, onboardPrincipal{Handle: "casey", IsOperator: true})
	svc := context.WithValue(context.Background(), ctxKeyOnboardPrincipal, onboardPrincipal{Handle: "onboard-runner", IsOperator: true})
	if err := s.Onboard.DecideProposal(per, p.ID, onboardstore.ProposalApproved, "casey", ""); err == nil {
		t.Fatal("per-person decide into approved: allowed")
	}
	if err := s.Onboard.SetProposalStatus(per, p.ID, onboardstore.ProposalApproved); err == nil {
		t.Fatal("per-person status write into approved: allowed")
	}
	if got, _ := s.Onboard.GetProposal(context.Background(), p.ID); got.Status != onboardstore.ProposalSponsorApproved {
		t.Fatalf("status %s after refused writes", got.Status)
	}
	if err := s.Onboard.DecideProposal(svc, p.ID, onboardstore.ProposalApproved, "onboard-runner", ""); err == nil {
		t.Fatal("service decide into approved of a PLATFORM-ADMIN row: allowed (ADR 1071 as amended refuses it)")
	}
	if err := s.Onboard.SetProposalStatus(context.Background(), p.ID, onboardstore.ProposalApproved); err != nil {
		t.Fatalf("no-principal status write: %v", err)
	}
	if err := s.Onboard.SetProposalStatus(per, "prop-does-not-exist", onboardstore.ProposalApproved); !errors.Is(err, onboardstore.ErrNotFound) {
		t.Fatalf("unknown id: %v, want ErrNotFound", err)
	}
}

// rv7-7788 F1 and the offboard note: the bookkeeping status writes (mark
// onboarded, bookkeeping offboard, any SetProposalStatus other than into
// approved) carry the per-person cap.
func TestRV7788r10_PerPersonKeyBookkeepingOnAdminRowCapped(t *testing.T) {
	s, _ := r7Server(t)
	r7OwnKey(t, s, "k-casey", "casey", "iam@example.org")
	s.Cfg.OnboardOperatorAPIKeys["k-svc"] = "onboard-runner-zagros"
	_ = s.Routes()
	mk := func(handle, status string, groups []string) *onboardstore.Proposal {
		p := r7Proposal(t, s, handle, onboardstore.ProposalProposed, "", groups)
		r7SetStatus(t, s, p.ID, status)
		return p
	}
	status := func(id string) string {
		got, _ := s.Onboard.GetProposal(context.Background(), id)
		return got.Status
	}
	admin, dev := []string{"PLATFORM-ADMIN"}, []string{"DEVELOPER"}
	for _, c := range []struct{ name, from, path, body string }{
		{"onboarded", onboardstore.ProposalApproved, "onboarded", ""},
		{"offboard", onboardstore.ProposalOnboarded, "offboard", `{"note":"bye"}`},
	} {
		p := mk("adm-"+c.name, c.from, admin)
		if w := r7API(t, s, "k-casey", "/api/onboard/proposals/"+p.ID+"/"+c.path, c.body); w.Code != http.StatusForbidden || status(p.ID) != c.from {
			t.Fatalf("per-person %s of a PLATFORM-ADMIN row: %d %s, status %s; want 403 and %s", c.name, w.Code, w.Body.String(), status(p.ID), c.from)
		}
		if w := r7API(t, s, "k-svc", "/api/onboard/proposals/"+p.ID+"/"+c.path, c.body); w.Code != http.StatusOK {
			t.Fatalf("service %s of a PLATFORM-ADMIN row: %d %s", c.name, w.Code, w.Body.String())
		}
		d := mk("dev-"+c.name, c.from, dev)
		if w := r7API(t, s, "k-casey", "/api/onboard/proposals/"+d.ID+"/"+c.path, c.body); w.Code != http.StatusOK {
			t.Fatalf("per-person %s of a developer row: %d %s", c.name, w.Code, w.Body.String())
		}
	}
	// The store write itself refuses a per-person principal.
	f := mk("adm-store", onboardstore.ProposalOnboarding, admin)
	per := context.WithValue(context.Background(), ctxKeyOnboardPrincipal, onboardPrincipal{Handle: "casey", IsOperator: true})
	if err := s.Onboard.SetProposalStatus(per, f.ID, onboardstore.ProposalFailed); err == nil {
		t.Fatal("per-person status write to failed on an admin row: allowed")
	}
	if ok, err := s.Onboard.MarkOffboardedFrom(per, mk("adm-off2", onboardstore.ProposalOnboarded, admin).ID, "casey", "x"); err == nil || ok {
		t.Fatalf("per-person bookkeeping offboard at the store: ok=%v err=%v", ok, err)
	}
}

// r10 (vr6w rv7 F1, the same rule here): a per-person key may not claim, or
// post the result of, a follow-up action (mesh-key reissue) on an admin row;
// the runner's service key may. Checked at the store write.
func TestRV7788r10_PerPersonKeyFollowUpActionsOnAdminRowCapped(t *testing.T) {
	s, _ := r7Server(t)
	r7OwnKey(t, s, "k-casey", "casey", "iam@example.org")
	s.Cfg.OnboardOperatorAPIKeys["k-svc"] = "onboard-runner-zagros"
	_ = s.Routes()
	enqueue := func(handle string, groups []string) string {
		p := r7Proposal(t, s, handle, onboardstore.ProposalProposed, "", groups)
		r7SetStatus(t, s, p.ID, onboardstore.ProposalOnboarded)
		a, err := s.Onboard.EnqueueAction(context.Background(), p.ID, onboardstore.ActionReissueHandset, "pa@example.org")
		if err != nil {
			t.Fatal(err)
		}
		return a.ID
	}
	adm := enqueue("adm-act", []string{"PLATFORM-ADMIN"})
	if w := r7API(t, s, "k-casey", "/api/onboard/actions/"+adm+"/claim", "{}"); w.Code != http.StatusForbidden {
		t.Fatalf("per-person claim of an admin's reissue: %d %s, want 403", w.Code, w.Body.String())
	}
	if w := r7API(t, s, "k-casey", "/api/onboard/actions/"+adm+"/result", `{"status":"done","handset_key":"HK-X"}`); w.Code != http.StatusForbidden {
		t.Fatalf("per-person result of an admin's reissue: %d %s, want 403", w.Code, w.Body.String())
	}
	per := context.WithValue(context.Background(), ctxKeyOnboardPrincipal, onboardPrincipal{Handle: "casey", IsOperator: true})
	if err := s.Onboard.ClaimAction(per, adm, "casey"); err == nil {
		t.Fatal("per-person ClaimAction at the store: allowed")
	}
	if err := s.Onboard.SetActionResult(per, adm, "", onboardstore.ActionDone, "{}", ""); err == nil {
		t.Fatal("per-person SetActionResult at the store: allowed")
	}
	if got, _ := s.Onboard.GetAction(context.Background(), adm); got.Status != onboardstore.ActionQueued {
		t.Fatalf("action status %s, want queued", got.Status)
	}
	if w := r7API(t, s, "k-svc", "/api/onboard/actions/"+adm+"/claim", "{}"); w.Code != http.StatusOK {
		t.Fatalf("service claim: %d %s", w.Code, w.Body.String())
	}
	dev := enqueue("dev-act", []string{"DEVELOPER"})
	if w := r7API(t, s, "k-casey", "/api/onboard/actions/"+dev+"/claim", "{}"); w.Code != http.StatusOK {
		t.Fatalf("per-person claim of a developer's reissue: %d %s", w.Code, w.Body.String())
	}
}

// r8 (ADR 1071 change): no service key grants the platform tier through
// POST /api/users/groups. It still grants other groups and removes the
// platform tier.
func TestRV7788r8_ServiceKeyCannotGrantPlatformTier(t *testing.T) {
	s, rt := r7Server(t)
	s.Cfg.OnboardOperatorAPIKeys["k-svc"] = "operator"
	for _, g := range []string{`"PLATFORM-ADMIN"`, `"ADMINS"`} {
		w := r7API(t, s, "k-svc", "/api/users/groups", `{"id":"`+r7U+`","groups":["VIEWER",`+g+`]}`)
		if w.Code != http.StatusForbidden {
			t.Fatalf("service key granting %s: %d %s, want 403", g, w.Code, w.Body.String())
		}
		if gs := rt.groups(r7U); containsString(gs, "PLATFORM-ADMIN") || containsString(gs, "ADMINS") {
			t.Fatalf("platform tier landed: %v", gs)
		}
	}
	if w := r7API(t, s, "k-svc", "/api/users/groups", `{"id":"`+r7U+`","groups":["VIEWER","IAM-ADMIN"]}`); w.Code != http.StatusOK {
		t.Fatalf("service key granting IAM-ADMIN: %d %s", w.Code, w.Body.String())
	}
	// Removing the platform tier (offboarding) is unchanged; a platform admin
	// keeping theirs while another group changes is not a grant.
	if w := r7API(t, s, "k-svc", "/api/users/groups", `{"id":"`+r7PA2+`","groups":["PLATFORM-ADMIN","DEVELOPER"]}`); w.Code != http.StatusOK {
		t.Fatalf("service key editing a platform admin's other groups: %d %s", w.Code, w.Body.String())
	}
	if w := r7API(t, s, "k-svc", "/api/users/groups", `{"id":"`+r7PA2+`","groups":[]}`); w.Code != http.StatusOK {
		t.Fatalf("service key removing the platform tier: %d %s", w.Code, w.Body.String())
	}
}
