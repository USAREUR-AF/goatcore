package onboardstore

import (
	"context"
	"errors"
	"testing"
	"time"
)

func onboardedProposal(t *testing.T, s *Store) *Proposal {
	t.Helper()
	ctx := context.Background()
	p := &Proposal{Sponsor: "jane-admin", Env: "zagros", Handle: "pat-liaison", Email: "person-105@example.org", ManifestJSON: `{"identity":{"handle":"pat-liaison"}}`, Tier: "observer"}
	if err := s.CreateProposal(ctx, p); err != nil {
		t.Fatalf("CreateProposal: %v", err)
	}
	if err := s.SetProposalStatus(ctx, p.ID, ProposalOnboarded); err != nil {
		t.Fatalf("SetProposalStatus: %v", err)
	}
	return p
}

func claimedActionToken(t *testing.T, s *Store, id string) string {
	t.Helper()
	a, err := s.GetAction(context.Background(), id)
	if err != nil || a.ClaimToken == "" {
		t.Fatalf("claimed action token: action=%+v err=%v", a, err)
	}
	return a.ClaimToken
}

func TestReleaseActionNeedsNoProposalAndClosesExactRequest(t *testing.T) {
	s := openTest(t)
	ctx := context.Background()
	entered := time.Date(2026, time.October, 10, 12, 0, 0, 0, time.UTC)
	other := entered.Add(time.Second)
	if _, _, err := s.RequestReactivation(ctx, "peer-1", "peer-one", "one", entered); err != nil {
		t.Fatal(err)
	}
	if _, _, err := s.RequestReactivation(ctx, "peer-1", "peer-one", "one", other); err != nil {
		t.Fatal(err)
	}
	params := `{"site":"zagros","peer_id":"peer-1","hold_entered_at":"2026-10-10T12:00:00Z"}`
	a, err := s.EnqueueHoldAction(ctx, "zagros", "peer-1", entered, "one", "admin@example.invalid", params)
	if err != nil {
		t.Fatalf("enqueue standalone release: %v", err)
	}
	if a.ProposalID != "" || a.Env != "zagros" || a.Handle != "one" || a.RequestedBy != "admin@example.invalid" {
		t.Fatalf("standalone release action = %+v", a)
	}
	if _, err := s.EnqueueHoldAction(ctx, "zagros", "peer-1", entered, "one", "admin@example.invalid", params); !errors.Is(err, ErrActionPending) {
		t.Fatalf("duplicate release: want ErrActionPending, got %v", err)
	}
	if err := s.ClaimAction(ctx, a.ID, "runner"); err != nil {
		t.Fatal(err)
	}
	claimToken := claimedActionToken(t, s, a.ID)
	if err := s.SetReleaseActionResult(ctx, a.ID, claimToken, ActionDone, `{"release":{"outcome":"released"}}`, "", "peer-1", entered, "admin@example.invalid", true); err != nil {
		t.Fatal(err)
	}
	if open, _ := s.HasOpenReactivationRequest(ctx, "peer-1", entered); open {
		t.Fatal("matching request remained open")
	}
	if open, _ := s.HasOpenReactivationRequest(ctx, "peer-1", other); !open {
		t.Fatal("different hold request was closed")
	}
}

func TestStaleReleaseClaimsRequeueAtTimeoutBoundary(t *testing.T) {
	s := openTest(t)
	ctx := context.Background()
	now := time.Date(2026, time.October, 21, 12, 0, 0, 0, time.UTC)
	makeRelease := func(peer string, entered time.Time) *Action {
		t.Helper()
		params := `{"site":"zagros","peer_id":"` + peer + `","hold_entered_at":"` + entered.Format(time.RFC3339) + `"}`
		a, err := s.EnqueueHoldAction(ctx, "zagros", peer, entered, "handle", "admin@example.invalid", params)
		if err != nil {
			t.Fatal(err)
		}
		if err := s.ClaimAction(ctx, a.ID, "runner"); err != nil {
			t.Fatal(err)
		}
		return a
	}
	stale := makeRelease("peer-stale", now.Add(-time.Hour))
	fresh := makeRelease("peer-fresh", now.Add(-2*time.Hour))
	cutoff := now.Add(-ReleaseHoldClaimRequeueAfter).Unix()
	if _, err := s.db.ExecContext(ctx, `UPDATE onboard_actions SET claimed_at=$1 WHERE id=$2`, cutoff, stale.ID); err != nil {
		t.Fatal(err)
	}
	if _, err := s.db.ExecContext(ctx, `UPDATE onboard_actions SET claimed_at=$1 WHERE id=$2`, cutoff+1, fresh.ID); err != nil {
		t.Fatal(err)
	}

	moved, err := s.requeueStaleReleaseClaimsAt(ctx, ReleaseHoldClaimRequeueAfter, now)
	if err != nil || moved != 1 {
		t.Fatalf("requeue moved=%d err=%v, want one", moved, err)
	}
	gotStale, _ := s.GetAction(ctx, stale.ID)
	gotFresh, _ := s.GetAction(ctx, fresh.ID)
	if gotStale.Status != ActionQueued || gotStale.ClaimedAt != nil || gotStale.ClaimedBy != "" {
		t.Fatalf("boundary claim not requeued: %+v", gotStale)
	}
	if gotFresh.Status != ActionClaimed || gotFresh.ClaimedAt == nil {
		t.Fatalf("claim newer than boundary was requeued: %+v", gotFresh)
	}
}

func TestReclaimedReleaseRejectsPreviousClaimResult(t *testing.T) {
	s := openTest(t)
	ctx := context.Background()
	entered := time.Date(2026, time.October, 21, 12, 0, 0, 0, time.UTC)
	if _, _, err := s.RequestReactivation(ctx, "peer-1", "peer-one", "one", entered); err != nil {
		t.Fatal(err)
	}
	a, err := s.EnqueueHoldAction(ctx, "zagros", "peer-1", entered, "one", "admin@example.invalid",
		`{"site":"zagros","peer_id":"peer-1","hold_entered_at":"2026-10-21T12:00:00Z"}`)
	if err != nil {
		t.Fatal(err)
	}
	if err := s.ClaimAction(ctx, a.ID, "runner-a"); err != nil {
		t.Fatal(err)
	}
	oldToken := claimedActionToken(t, s, a.ID)
	if _, err := s.db.ExecContext(ctx, `UPDATE onboard_actions SET claimed_at=$1 WHERE id=$2`, entered.Add(-ReleaseHoldClaimRequeueAfter).Unix(), a.ID); err != nil {
		t.Fatal(err)
	}
	if moved, err := s.requeueStaleReleaseClaimsAt(ctx, ReleaseHoldClaimRequeueAfter, entered); err != nil || moved != 1 {
		t.Fatalf("requeue moved=%d err=%v", moved, err)
	}
	if err := s.ClaimAction(ctx, a.ID, "runner-b"); err != nil {
		t.Fatal(err)
	}
	newToken := claimedActionToken(t, s, a.ID)
	if oldToken == newToken {
		t.Fatal("reclaim reused the previous claim token")
	}
	if err := s.SetReleaseActionResult(ctx, a.ID, oldToken, ActionFailed, `{}`, "late failure", "peer-1", entered, "admin@example.invalid", false); !errors.Is(err, ErrActionNotClaimed) {
		t.Fatalf("old claimant result: want ErrActionNotClaimed, got %v", err)
	}
	stillClaimed, err := s.GetAction(ctx, a.ID)
	if err != nil || stillClaimed.Status != ActionClaimed || stillClaimed.ClaimedBy != "runner-b" || stillClaimed.ClaimToken != newToken {
		t.Fatalf("new lease changed by stale result: action=%+v err=%v", stillClaimed, err)
	}
	if open, err := s.HasOpenReactivationRequest(ctx, "peer-1", entered); err != nil || !open {
		t.Fatalf("stale result closed request: open=%t err=%v", open, err)
	}
	if err := s.SetReleaseActionResult(ctx, a.ID, newToken, ActionDone, `{"release":{"outcome":"released"}}`, "", "peer-1", entered, "admin@example.invalid", true); err != nil {
		t.Fatalf("current claimant result: %v", err)
	}
}

func TestActionEnqueueClaimResult(t *testing.T) {
	s := openTest(t)
	ctx := context.Background()
	p := onboardedProposal(t, s)

	a, err := s.EnqueueAction(ctx, p.ID, ActionReissueHandset, "maintainer@example.invalid")
	if err != nil {
		t.Fatalf("EnqueueAction: %v", err)
	}
	if a.Status != ActionQueued || a.Handle != "pat-liaison" || a.Env != "zagros" || a.Kind != ActionReissueHandset {
		t.Fatalf("queued action mismatch: %+v", a)
	}
	queued, err := s.ListActions(ctx, ActionQueued)
	if err != nil || len(queued) != 1 || queued[0].ID != a.ID {
		t.Fatalf("ListActions(queued) = %+v, %v", queued, err)
	}

	if err := s.ClaimAction(ctx, a.ID, "runner-a"); err != nil {
		t.Fatalf("ClaimAction: %v", err)
	}
	if err := s.ClaimAction(ctx, a.ID, "runner-b"); !errors.Is(err, ErrActionNotQueued) {
		t.Fatalf("second claim: want ErrActionNotQueued, got %v", err)
	}
	got, _ := s.GetAction(ctx, a.ID)
	if got.Status != ActionClaimed || got.ClaimedBy != "runner-a" || got.ClaimedAt == nil {
		t.Fatalf("claimed action mismatch: %+v", got)
	}

	claimToken := got.ClaimToken
	if err := s.SetActionResult(ctx, a.ID, claimToken, ActionDone, `{"handset_key_set":true}`, ""); err != nil {
		t.Fatalf("SetActionResult: %v", err)
	}
	got, _ = s.GetAction(ctx, a.ID)
	if got.Status != ActionDone || got.FinishedAt == nil || got.ResultJSON != `{"handset_key_set":true}` {
		t.Fatalf("done action mismatch: %+v", got)
	}
	// A second result for a finished action is refused.
	if err := s.SetActionResult(ctx, a.ID, claimToken, ActionFailed, "", "late"); !errors.Is(err, ErrActionNotClaimed) {
		t.Fatalf("result on done action: want ErrActionNotClaimed, got %v", err)
	}
	if left, _ := s.ListActions(ctx, ActionQueued); len(left) != 0 {
		t.Fatalf("queue should be empty, got %d", len(left))
	}
}

func TestSetActionResultAllowsEmptyTokenButRejectsWrongToken(t *testing.T) {
	s := openTest(t)
	ctx := context.Background()
	p := onboardedProposal(t, s)

	tokenless, err := s.EnqueueAction(ctx, p.ID, ActionReissueHandset, "admin@example.invalid")
	if err != nil {
		t.Fatal(err)
	}
	if err := s.ClaimAction(ctx, tokenless.ID, "old-runner"); err != nil {
		t.Fatal(err)
	}
	if claimedActionToken(t, s, tokenless.ID) == "" {
		t.Fatal("claim did not store a token")
	}
	if err := s.SetActionResult(ctx, tokenless.ID, "", ActionFailed, "{}", "legacy failure"); err != nil {
		t.Fatalf("tokenless result: %v", err)
	}
	if got, err := s.GetAction(ctx, tokenless.ID); err != nil || got.Status != ActionFailed {
		t.Fatalf("tokenless result action=%+v err=%v", got, err)
	}

	wrong, err := s.EnqueueAction(ctx, p.ID, ActionReissueHandset, "admin@example.invalid")
	if err != nil {
		t.Fatal(err)
	}
	if err := s.ClaimAction(ctx, wrong.ID, "new-runner"); err != nil {
		t.Fatal(err)
	}
	if err := s.SetActionResult(ctx, wrong.ID, "wrong-token", ActionFailed, "{}", "failure"); !errors.Is(err, ErrActionNotClaimed) {
		t.Fatalf("wrong token: want ErrActionNotClaimed, got %v", err)
	}
}

func TestActionEnqueueRefusals(t *testing.T) {
	s := openTest(t)
	ctx := context.Background()

	// Unknown proposal.
	if _, err := s.EnqueueAction(ctx, "prop-nope", ActionReissueHandset, "x"); !errors.Is(err, ErrNotFound) {
		t.Fatalf("unknown proposal: want ErrNotFound, got %v", err)
	}
	// Not onboarded yet.
	p := &Proposal{Sponsor: "jane-admin", Env: "zagros", Handle: "sam", Email: "person-111@example.org", ManifestJSON: `{}`}
	if err := s.CreateProposal(ctx, p); err != nil {
		t.Fatal(err)
	}
	if _, err := s.EnqueueAction(ctx, p.ID, ActionReissueHandset, "x"); !errors.Is(err, ErrActionNotOnboarded) {
		t.Fatalf("proposed proposal: want ErrActionNotOnboarded, got %v", err)
	}
	// Double enqueue while one is pending (queued, then claimed).
	o := onboardedProposal(t, s)
	a, err := s.EnqueueAction(ctx, o.ID, ActionReissueHandset, "x")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := s.EnqueueAction(ctx, o.ID, ActionReissueHandset, "x"); !errors.Is(err, ErrActionPending) {
		t.Fatalf("double enqueue (queued): want ErrActionPending, got %v", err)
	}
	if err := s.ClaimAction(ctx, a.ID, "runner-a"); err != nil {
		t.Fatal(err)
	}
	claimToken := claimedActionToken(t, s, a.ID)
	if _, err := s.EnqueueAction(ctx, o.ID, ActionReissueHandset, "x"); !errors.Is(err, ErrActionPending) {
		t.Fatalf("double enqueue (claimed): want ErrActionPending, got %v", err)
	}
	// Once finished, a new one may be queued.
	if err := s.SetActionResult(ctx, a.ID, claimToken, ActionFailed, "", "mint said no"); err != nil {
		t.Fatal(err)
	}
	// A real runner takes seconds between enqueue and result, so the next
	// action is created in a later second. Model that: within one second the
	// history order falls back to the random action id.
	if _, err := s.db.ExecContext(ctx, `UPDATE onboard_actions SET created_at = created_at - 1 WHERE id = $1`, a.ID); err != nil {
		t.Fatal(err)
	}
	if _, err := s.EnqueueAction(ctx, o.ID, ActionReissueHandset, "x"); err != nil {
		t.Fatalf("enqueue after failed: %v", err)
	}
	hist, _ := s.ListActionsForProposal(ctx, o.ID)
	if len(hist) != 2 || hist[0].Status != ActionQueued || hist[1].Status != ActionFailed || hist[1].Error != "mint said no" {
		t.Fatalf("history mismatch: %+v", hist)
	}
}

func TestActionDeviceReissueCarriesParamsAndIsOfferedToEnrolled(t *testing.T) {
	s := openTest(t)
	ctx := context.Background()
	p := onboardedProposal(t, s)
	// An expired key is usually met after the person enrolled a first device.
	if err := s.SetProposalStatus(ctx, p.ID, ProposalEnrolled); err != nil {
		t.Fatalf("SetProposalStatus: %v", err)
	}
	a, err := s.EnqueueActionWithParams(ctx, p.ID, ActionReissueDevice, "maintainer@example.invalid", `{"uses":5}`)
	if err != nil {
		t.Fatalf("enqueue for an enrolled person: %v", err)
	}
	if a.Kind != ActionReissueDevice || a.Params != `{"uses":5}` || a.RequestedBy != "maintainer@example.invalid" {
		t.Fatalf("queued device action mismatch: %+v", a)
	}
	// A pending device reissue does not block a handset one: pending is per kind.
	if _, err := s.EnqueueAction(ctx, p.ID, ActionReissueHandset, "x"); err != nil {
		t.Fatalf("handset beside a pending device reissue: %v", err)
	}
	if _, err := s.EnqueueActionWithParams(ctx, p.ID, ActionReissueDevice, "x", `{"uses":2}`); !errors.Is(err, ErrActionPending) {
		t.Fatalf("second device reissue: want ErrActionPending, got %v", err)
	}
	// Blank params are stored as an empty object, not "".
	b, _ := s.EnqueueAction(ctx, onboardedProposal(t, s).ID, ActionReissueDevice, "x")
	if b.Params != "{}" {
		t.Fatalf("blank params stored as %q", b.Params)
	}
	// Cut access is not reissued: archived and offboarded are refused.
	for _, st := range []string{ProposalArchived, ProposalOffboarded} {
		q := onboardedProposal(t, s)
		if err := s.SetProposalStatus(ctx, q.ID, st); err != nil {
			t.Fatalf("SetProposalStatus(%s): %v", st, err)
		}
		if _, err := s.EnqueueAction(ctx, q.ID, ActionReissueDevice, "x"); !errors.Is(err, ErrActionNotOnboarded) {
			t.Fatalf("%s: want ErrActionNotOnboarded, got %v", st, err)
		}
	}
}
