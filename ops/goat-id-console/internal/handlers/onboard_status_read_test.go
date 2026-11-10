package handlers

import (
	"context"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// TestConsumedInviteIsStatusRead: a consumed invite link is the candidate's
// status page — preview returns a submission block whose phase tracks the
// proposal through the queue, instead of dead-ending at "already used".
func TestConsumedInviteIsStatusRead(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	tok := mintTokenViaStore(t, s)

	pid := submit(t, h, tok, map[string]any{"name": "Pat Liaison", "email": "person-106@example.org", "organization": "Status Org"})

	phase := func(want, detail string) {
		t.Helper()
		pv := preview(t, h, tok)
		if pv.Valid {
			t.Fatalf("consumed invite must not preview valid")
		}
		if pv.Submission == nil {
			t.Fatalf("%s: want submission block on consumed invite, got none (reason=%q)", want, pv.Reason)
		}
		if pv.Submission.Phase != want {
			t.Fatalf("want phase %q, got %q (%s)", want, pv.Submission.Phase, detail)
		}
		if pv.Submission.SubmittedAt == 0 {
			t.Fatalf("submission block missing submitted_at")
		}
	}

	phase("pending-sponsor", "fresh submit")

	if err := s.Onboard.DecideProposal(context.Background(), pid, onboardstore.ProposalSponsorApproved, "sponsor@x", ""); err != nil {
		t.Fatalf("vouch: %v", err)
	}
	phase("pending-admin", "after sponsor vouch")

	if err := s.Onboard.DecideProposal(context.Background(), pid, onboardstore.ProposalApproved, "admin@x", ""); err != nil {
		t.Fatalf("approve: %v", err)
	}
	phase("provisioning", "after admin approve")

	if err := s.Onboard.ClaimProposal(context.Background(), pid); err != nil {
		t.Fatalf("claim: %v", err)
	}
	phase("provisioning", "runner claimed")

	if err := s.Onboard.SetProposalResult(context.Background(), pid, onboardstore.ProposalOnboarded, "", "", "", "", "", false); err != nil {
		t.Fatalf("onboarded: %v", err)
	}
	phase("complete", "after onboarded")
}

// TestConsumedInviteStatusRejected: a rejected proposal is shown to the
// candidate as the rejected phase (the rejection is otherwise invisible to
// them — no email is sent today).
func TestConsumedInviteStatusRejected(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	tok := mintTokenViaStore(t, s)
	pid := submit(t, h, tok, map[string]any{"name": "Pat Liaison", "email": "person-106@example.org", "organization": "Status Org"})
	if err := s.Onboard.DecideProposal(context.Background(), pid, onboardstore.ProposalRejected, "admin@x", "no"); err != nil {
		t.Fatalf("reject: %v", err)
	}
	pv := preview(t, h, tok)
	if pv.Submission == nil || pv.Submission.Phase != "rejected" {
		t.Fatalf("want rejected phase, got %+v", pv.Submission)
	}
}

// TestSubmitEmitAudit: the candidate's submission lands in /audit — the
// forensic surface previously first saw a candidate at the sponsor's decision.
func TestSubmitEmitAudit(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	tok := mintTokenViaStore(t, s)
	pid := submit(t, h, tok, map[string]any{"name": "Pat Liaison", "email": "person-106@example.org", "organization": "Status Org"})

	for _, e := range s.Audit.Recent(50) {
		if e.Action == "onboard_submit" {
			if e.Actor.Email != "person-106@example.org" {
				t.Fatalf("audit actor: want candidate email, got %q", e.Actor.Email)
			}
			if e.After["proposal_id"] != pid {
				t.Fatalf("audit after.proposal_id: want %q, got %v", pid, e.After["proposal_id"])
			}
			return
		}
	}
	t.Fatalf("no onboard_submit audit event emitted")
}

// TestConsumedInviteStatusOffboarded: the offboarded state (added by the
// console's offboard marker) must read as a closed terminal phase — the
// default arm would tell someone whose access was cut that their request
// is still in review.
func TestConsumedInviteStatusOffboarded(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	tok := mintTokenViaStore(t, s)
	pid := submit(t, h, tok, map[string]any{"name": "Pat Liaison", "email": "person-106@example.org", "organization": "Status Org"})
	for _, st := range []string{onboardstore.ProposalSponsorApproved, onboardstore.ProposalApproved} {
		if err := s.Onboard.DecideProposal(context.Background(), pid, st, "admin@x", ""); err != nil {
			t.Fatalf("%s: %v", st, err)
		}
	}
	if err := s.Onboard.ClaimProposal(context.Background(), pid); err != nil {
		t.Fatalf("claim: %v", err)
	}
	if err := s.Onboard.SetProposalResult(context.Background(), pid, onboardstore.ProposalOnboarded, "", "", "", "", "", false); err != nil {
		t.Fatalf("onboarded: %v", err)
	}
	if err := s.Onboard.DecideProposal(context.Background(), pid, onboardstore.ProposalOffboarded, "admin@x", "left the program"); err != nil {
		t.Fatalf("offboard: %v", err)
	}
	pv := preview(t, h, tok)
	if pv.Submission == nil || pv.Submission.Phase != "closed" {
		t.Fatalf("want closed phase for an offboarded proposal, got %+v", pv.Submission)
	}
}
