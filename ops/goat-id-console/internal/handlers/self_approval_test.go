package handlers

import (
	"context"
	"errors"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/kratos"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// The rule that nobody approves their own onboarding is decided on the
// person, not on one spelling of their address. These cover the forms a
// second spelling takes (case, plus tag, lookalikes), a changed email on the
// same Kratos identity, a second mailbox under the same handle, and an
// operator key whose handle or onboarding email is the applicant's.

const selfAdminID = "0b1f0000-0000-4000-8000-0000000000a1"

func ceilingReq(actor Actor) *http.Request {
	r := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/onboarding/x/approve", nil)
	return r.WithContext(context.WithValue(r.Context(), ctxKeyActor, actor))
}

func adminActor(email, id string) Actor {
	return Actor{Email: email, IdentityID: id, Groups: []string{"PLATFORM-ADMIN"}}
}

func browseProposal(email, handle string) *onboardstore.Proposal {
	return &onboardstore.Proposal{Email: email, Handle: handle, Tier: "browse"}
}

func TestApprovalCeiling_selfUnderSecondSpelling(t *testing.T) {
	s := &Server{}
	for _, applicant := range []string{
		"ADMIN@Example.Test",
		"admin+second@example.test",
		"аdmin@example.test",  // Cyrillic a
		"admin@еxample.test",  // Cyrillic e in the domain
		"ad\u200bmin@example.test", // zero-width space
		"ａdmin@example.test",  // fullwidth a
	} {
		e := s.approvalCeiling(ceilingReq(adminActor("admin@example.test", "")), browseProposal(applicant, ""), "admin@example.test")
		if e == nil || e.status != http.StatusForbidden {
			t.Errorf("admin@example.test approving %q: got %v, want a 403 self refusal", applicant, e)
		}
	}
}

// The session cookie carries the address the admin had at sign-in. When the
// identity's email has since changed, the proposal under the NEW address is
// still theirs: the identity id says so.
func TestApprovalCeiling_changedEmailOnSameIdentity(t *testing.T) {
	kc, _ := fakeKratosMeta(t, selfAdminID, "new-address@example.test", "Admin", []string{"PLATFORM-ADMIN"}, "active")
	s := newUsersAPITestServer(t, kc)
	r := ceilingReq(adminActor("old-address@example.test", selfAdminID))
	e := s.approvalCeiling(r, browseProposal("New-Address+x@example.test", "someone"), "old-address@example.test")
	if e == nil || e.status != http.StatusForbidden {
		t.Fatalf("approving a proposal under the identity's current address: got %v, want a 403 self refusal", e)
	}
	// And a genuinely different applicant is still decidable by the same admin.
	if e := s.approvalCeiling(ceilingReq(adminActor("old-address@example.test", selfAdminID)),
		browseProposal("someone-else@example.test", "someone-else"), "old-address@example.test"); e != nil {
		t.Fatalf("different applicant refused: %d %s", e.status, e.msg)
	}
}

// A second mailbox with the admin's own onboarding handle is the admin.
func TestApprovalCeiling_secondMailboxSameHandle(t *testing.T) {
	ctx := context.Background()
	kc, _ := fakeKratosMeta(t, selfAdminID, "admin@example.test", "Admin", []string{"PLATFORM-ADMIN"}, "active")
	s := newUsersAPITestServer(t, kc)
	s.Cfg.NotifyGoatnet = "zagros"
	own := &onboardstore.Proposal{Sponsor: "sponsor-1", Env: "zagros", Handle: "admin-handle",
		Email: "admin@example.test", Status: onboardstore.ProposalOnboarded, ManifestJSON: "{}", Tier: "platform-admin"}
	if err := s.Onboard.CreateProposal(ctx, own); err != nil {
		t.Fatalf("CreateProposal: %v", err)
	}
	e := s.approvalCeiling(ceilingReq(adminActor("admin@example.test", selfAdminID)),
		browseProposal("unrelated-mailbox@elsewhere.test", "Admin-Handle"), "admin@example.test")
	if e == nil || e.status != http.StatusForbidden {
		t.Fatalf("approving a second mailbox filed under the admin's own handle: got %v, want 403", e)
	}
}

// When the session names an identity the console cannot read back, the
// decision is refused rather than taken on the cookie's word.
func TestApprovalCeiling_identityUnreadableRefuses(t *testing.T) {
	kc, _ := fakeKratosRevokeSessionsTargetError(t, http.StatusInternalServerError)
	s := newUsersAPITestServer(t, kc)
	e := s.approvalCeiling(ceilingReq(adminActor("admin@example.test", selfAdminID)),
		browseProposal("someone@example.test", "someone"), "admin@example.test")
	if e == nil || e.status != http.StatusServiceUnavailable {
		t.Fatalf("unreadable identity: got %v, want 503", e)
	}
}

func TestApprovalCeiling_noRealmClientWithLiveCheckRefuses(t *testing.T) {
	s := newUsersAPITestServer(t, nil)
	s.Kratos = map[string]*kratos.Client{}
	s.Cfg.LiveIdentityCheck = true
	e := s.approvalCeiling(ceilingReq(adminActor("admin@example.test", selfAdminID)),
		browseProposal("someone@example.test", "someone"), "admin@example.test")
	if e == nil || e.status != http.StatusServiceUnavailable {
		t.Fatalf("no sign-in realm client: got %v, want 503", e)
	}
}

// An operator key is the applicant when its handle is the proposal's handle,
// or when the onboarding email filed under its handle is the proposal's
// address.
func TestApprovalCeiling_operatorPrincipalIsApplicant(t *testing.T) {
	ctx := context.Background()
	s := newUsersAPITestServer(t, nil)
	if err := s.Onboard.CreateProposal(ctx, &onboardstore.Proposal{Sponsor: "sponsor-1", Env: "zagros",
		Handle: "op-alice", Email: "alice@example.test", Status: onboardstore.ProposalOnboarded,
		ManifestJSON: "{}", Tier: "platform-admin"}); err != nil {
		t.Fatalf("CreateProposal: %v", err)
	}
	withOp := func(handle string) *http.Request {
		r := httptest.NewRequestWithContext(ctx, http.MethodPost, "/api/onboard/proposals/x/approve", nil)
		return r.WithContext(context.WithValue(r.Context(), ctxKeyOnboardPrincipal, //nolint:contextcheck // behavior kept as reviewed for disclosure; lint cleanup goatcore#1028
			onboardPrincipal{Handle: handle, IsOperator: true}))
	}
	if e := s.approvalCeiling(withOp("op-alice"), browseProposal("second@elsewhere.test", "OP-Alice"), "op-alice"); e == nil || e.status != http.StatusForbidden {
		t.Errorf("operator key approving a proposal under its own handle: got %v, want 403", e)
	}
	if e := s.approvalCeiling(withOp("op-alice"), browseProposal("Alice+2@example.test", "fresh-handle"), "op-alice"); e == nil || e.status != http.StatusForbidden {
		t.Errorf("operator key approving a proposal under its own onboarding email: got %v, want 403", e)
	}
	if e := s.approvalCeiling(withOp("op-alice"), browseProposal("bob@example.test", "bob"), "op-alice"); e != nil {
		t.Errorf("operator key approving someone else: refused %d %s", e.status, e.msg)
	}
}

// The self-reactivation refusal also recognizes a second spelling of the
// identity's address when the session carries no identity id.
func TestPatchState_refusesSelfReactivationUnderSecondSpelling(t *testing.T) {
	kc, read := fakeKratosMeta(t, "id-3", "gone@example.org", "Gone", []string{"PLATFORM-ADMIN"}, "inactive")
	s := newUsersAPITestServer(t, kc)
	rec := postState(t, s, "id-3", "Gone+back@Example.org", "active")
	if rec.Code != http.StatusForbidden {
		t.Fatalf("self reactivation under a plus tag: status %d, want 403; body=%s", rec.Code, rec.Body.String())
	}
	if _, st := read(); st != "inactive" {
		t.Fatalf("identity state = %q after a refused self reactivation", st)
	}
}

func TestActorIsSelf(t *testing.T) {
	target := &kratos.Identity{ID: selfAdminID, Traits: map[string]interface{}{"email": "new@example.test"},
		VerifiableAddresses: []kratos.VerifiableAddress{{Value: "new@example.test"}}}
	for _, tc := range []struct {
		name  string
		actor Actor
		want  bool
	}{
		{"same identity id, stale cookie email", Actor{Email: "old@example.test", IdentityID: selfAdminID}, true},
		{"same identity id in upper case", Actor{IdentityID: "0B1F0000-0000-4000-8000-0000000000A1"}, true},
		{"no id, lookalike email", Actor{Email: "nеw@example.test"}, true},
		{"different person", Actor{Email: "other@example.test", IdentityID: "0b1f0000-0000-4000-8000-0000000000b2"}, false},
	} {
		if got := actorIsSelf(tc.actor, selfAdminID, target); got != tc.want {
			t.Errorf("%s: actorIsSelf = %v, want %v", tc.name, got, tc.want)
		}
	}
}

// The operator-key queue's decider forms carry the identity's current
// address, so a key requested under the new address cannot be approved from a
// session that signed in under the old one.
func TestOperatorKeyDecider_changedEmailOnSameIdentity(t *testing.T) {
	ctx := context.Background()
	kc, _ := fakeKratosMeta(t, selfAdminID, "new-address@example.test", "Admin", []string{"PLATFORM-ADMIN"}, "active")
	s := newUsersAPITestServer(t, kc)
	row, err := s.Onboard.RequestOperatorKey(ctx, &onboardstore.OperatorKey{
		Handle: "admin-handle", Email: "new-address@example.test", Env: "zagros"})
	if err != nil {
		t.Fatal(err)
	}
	r := ceilingReq(adminActor("old-address@example.test", selfAdminID))
	ids, err := s.operatorKeyDeciderIDs(r, adminActor("old-address@example.test", selfAdminID))
	if err != nil {
		t.Fatalf("operatorKeyDeciderIDs: %v", err)
	}
	if _, err := s.Onboard.ApproveOperatorKey(ctx, row.ID, "old-address@example.test", ids); !errors.Is(err, onboardstore.ErrOperatorKeySelfDecision) {
		t.Fatalf("approve own key from a pre-change session = %v, want ErrOperatorKeySelfDecision", err)
	}
}
