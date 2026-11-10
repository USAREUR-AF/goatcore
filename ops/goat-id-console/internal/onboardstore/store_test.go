package onboardstore

import (
	"context"
	"encoding/json"
	"errors"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/pgtest"
)

func openTest(t *testing.T) *Store {
	t.Helper()
	s, err := Open(pgtest.DSN(t))
	if err != nil {
		t.Fatalf("Open: %v", err)
	}
	t.Cleanup(func() { _ = s.Close() })
	return s
}

func TestProposalLifecycle(t *testing.T) {
	s := openTest(t)
	ctx := context.Background()

	p := &Proposal{Sponsor: "jane-admin", Env: "zagros", Handle: "pat-liaison", Email: "person-105@example.org", ManifestJSON: `{"identity":{"handle":"pat-liaison"}}`, Tier: "operator"}
	if err := s.CreateProposal(ctx, p); err != nil {
		t.Fatalf("CreateProposal: %v", err)
	}
	if p.ID == "" || p.Status != ProposalProposed {
		t.Fatalf("expected id + proposed status, got %q / %q", p.ID, p.Status)
	}

	got, err := s.GetProposal(ctx, p.ID)
	if err != nil {
		t.Fatalf("GetProposal: %v", err)
	}
	if got.Handle != "pat-liaison" || got.Tier != "operator" || got.Petitions != "[]" {
		t.Fatalf("round-trip mismatch: %+v", got)
	}

	// Sponsor vouch → admin approve → claim (runner) → onboarded.
	if err := s.DecideProposal(ctx, p.ID, ProposalSponsorApproved, "jane-admin", "vouched"); err != nil {
		t.Fatalf("sponsor decide: %v", err)
	}
	if err := s.DecideProposal(ctx, p.ID, ProposalApproved, "maintainer-operator", "ok"); err != nil {
		t.Fatalf("admin decide: %v", err)
	}
	approved, _ := s.ListProposals(ctx, ProposalApproved)
	if len(approved) != 1 {
		t.Fatalf("expected 1 approved, got %d", len(approved))
	}
	if err := s.ClaimProposal(ctx, p.ID); err != nil {
		t.Fatalf("ClaimProposal: %v", err)
	}
	// Second claim must fail — already onboarding, not approved.
	if err := s.ClaimProposal(ctx, p.ID); !errors.Is(err, ErrProposalNotApproved) {
		t.Fatalf("double claim: want ErrProposalNotApproved, got %v", err)
	}
	if err := s.SetProposalStatus(ctx, p.ID, ProposalOnboarded); err != nil {
		t.Fatalf("SetProposalStatus: %v", err)
	}
	final, _ := s.GetProposal(ctx, p.ID)
	if final.Status != ProposalOnboarded {
		t.Fatalf("want onboarded, got %q", final.Status)
	}

	if _, err := s.GetProposal(ctx, "prop-does-not-exist"); !errors.Is(err, ErrNotFound) {
		t.Fatalf("want ErrNotFound, got %v", err)
	}
}

func TestSetProposalResultCollab(t *testing.T) {
	s := openTest(t)
	ctx := context.Background()
	p := &Proposal{Sponsor: "jane-admin", Env: "zagros", Handle: "col-user", Email: "person-21@example.org", ManifestJSON: `{}`, Tier: "developer"}
	if err := s.CreateProposal(ctx, p); err != nil {
		t.Fatalf("CreateProposal: %v", err)
	}

	// Onboarded result carrying a provisioned collab account persists it (read-only
	// mesh awareness); the one-time link is never passed here (not stored).
	if err := s.SetProposalResult(ctx, p.ID, ProposalOnboarded, "KEY", "HKEY", "/bundle/x", "", "col-user", true); err != nil {
		t.Fatalf("SetProposalResult: %v", err)
	}
	got, _ := s.GetProposal(ctx, p.ID)
	if got.CollabAccount != "col-user" || !got.CollabActivated {
		t.Fatalf("collab not persisted: account=%q activated=%v", got.CollabAccount, got.CollabActivated)
	}
	if got.SetupKey != "KEY" || got.HandsetKey != "HKEY" {
		t.Fatalf("keys not persisted: setup=%q handset=%q", got.SetupKey, got.HandsetKey)
	}

	// A later partial writeback (no collab fields) must NOT blank the stored values,
	// and collab_activated only ever latches.
	if err := s.SetProposalResult(ctx, p.ID, ProposalOnboarded, "", "", "", "", "", false); err != nil {
		t.Fatalf("second SetProposalResult: %v", err)
	}
	got2, _ := s.GetProposal(ctx, p.ID)
	if got2.CollabAccount != "col-user" || !got2.CollabActivated {
		t.Fatalf("collab blanked by a partial writeback: %+v", got2)
	}
	if got2.SetupKey != "KEY" || got2.HandsetKey != "HKEY" {
		t.Fatalf("keys blanked by a partial writeback: setup=%q handset=%q", got2.SetupKey, got2.HandsetKey)
	}
}

func TestInviteConsumeSingleUse(t *testing.T) {
	s := openTest(t)
	ctx := context.Background()

	inv := &Invite{Sponsor: "jane-admin", Env: "zagros", ExpiresAt: time.Now().Add(time.Hour).Unix(),
		PreboundJSON: `{"tier":"operator","sponsor_max_tier":"iam-admin"}`}
	token, err := s.MintInvite(ctx, inv)
	if err != nil {
		t.Fatalf("MintInvite: %v", err)
	}
	if token == "" {
		t.Fatal("expected plaintext token")
	}
	pb, err := inv.Prebound()
	if err != nil {
		t.Fatalf("parse prebinding: %v", err)
	}
	if pb.Tier != "operator" || pb.SponsorMaxTier != "iam-admin" {
		t.Fatalf("prebound parse mismatch: %+v", pb)
	}

	// Token hash is the lookup key; plaintext is never persisted.
	got, err := s.GetInviteByToken(ctx, token)
	if err != nil {
		t.Fatalf("GetInviteByToken: %v", err)
	}
	if got.ID != inv.ID || got.Status != InviteInvited {
		t.Fatalf("lookup mismatch: %+v", got)
	}

	build := func(iv *Invite, pid string) (*Proposal, error) {
		rec, handle, email, err := AssembleIntakeRecord(iv, json.RawMessage(`{"handle":"pat-liaison","email":"person-105@example.org"}`))
		if err != nil {
			return nil, err
		}
		return &Proposal{Handle: handle, Email: email, ManifestJSON: string(rec)}, nil
	}
	p, err := s.ConsumeInvite(ctx, token, build)
	if err != nil {
		t.Fatalf("ConsumeInvite: %v", err)
	}
	if p.Handle != "pat-liaison" || p.Sponsor != "jane-admin" || p.Env != "zagros" {
		t.Fatalf("assembled proposal mismatch: %+v", p)
	}

	// Single-use: a second consume of the same token fails closed.
	if _, err := s.ConsumeInvite(ctx, token, build); !errors.Is(err, ErrInviteConsumed) {
		t.Fatalf("double consume: want ErrInviteConsumed, got %v", err)
	}

	// The invite now records provenance.
	after, _ := s.GetInvite(ctx, inv.ID)
	if after.Status != InviteSubmitted || after.ProposalID == nil || *after.ProposalID != p.ID {
		t.Fatalf("consumed invite provenance mismatch: %+v", after)
	}
}

func TestInvitePreboundRejectsMalformedNonEmptyMetadata(t *testing.T) {
	legacy, err := (&Invite{}).Prebound()
	if err != nil || legacy != (Prebound{}) {
		t.Fatalf("empty legacy prebinding = %+v, %v; want zero value, nil", legacy, err)
	}

	if _, err := (&Invite{PreboundJSON: `{not-json`}).Prebound(); err == nil {
		t.Fatal("malformed non-empty prebinding was accepted")
	}
}

func TestInviteExpiryAndRevoke(t *testing.T) {
	s := openTest(t)
	ctx := context.Background()

	// Already-past TTL → lazily expired on read, and consume fails closed.
	expired := &Invite{Sponsor: "jane-admin", Env: "zagros", ExpiresAt: time.Now().Add(-time.Minute).Unix()}
	tokE, _ := s.MintInvite(ctx, expired)
	got, _ := s.GetInviteByToken(ctx, tokE)
	if got.Status != InviteExpired {
		t.Fatalf("want lazily expired, got %q", got.Status)
	}
	if _, err := s.ConsumeInvite(ctx, tokE, func(*Invite, string) (*Proposal, error) { return &Proposal{}, nil }); !errors.Is(err, ErrInviteExpired) {
		t.Fatalf("expired consume: want ErrInviteExpired, got %v", err)
	}

	// Revoke an open invite → consume fails closed.
	open := &Invite{Sponsor: "jane-admin", Env: "zagros", ExpiresAt: time.Now().Add(time.Hour).Unix()}
	tokR, _ := s.MintInvite(ctx, open)
	if err := s.RevokeInvite(ctx, open.ID); err != nil {
		t.Fatalf("RevokeInvite: %v", err)
	}
	if _, err := s.ConsumeInvite(ctx, tokR, func(*Invite, string) (*Proposal, error) { return &Proposal{}, nil }); !errors.Is(err, ErrInviteConsumed) {
		t.Fatalf("revoked consume: want ErrInviteConsumed, got %v", err)
	}
	// Revoking a non-open invite is a no-op error (already consumed/revoked).
	if err := s.RevokeInvite(ctx, open.ID); !errors.Is(err, ErrInviteConsumed) {
		t.Fatalf("double revoke: want ErrInviteConsumed, got %v", err)
	}
	if err := s.RevokeInvite(ctx, "inv-nope"); !errors.Is(err, ErrNotFound) {
		t.Fatalf("revoke unknown: want ErrNotFound, got %v", err)
	}
}

func TestRecordSponsorApprovalAndRetroactiveColumns(t *testing.T) {
	s := openTest(t)
	ctx := context.Background()
	p := &Proposal{ID: "prop-x", CreatedAt: 1, Sponsor: "spon@example.org", Env: "zagros",
		Handle: "h", Email: "h@example.org", Status: ProposalProposed, ManifestJSON: "{}"}
	if err := s.CreateProposal(ctx, p); err != nil {
		t.Fatal(err)
	}
	if err := s.RecordSponsorApproval(ctx, "prop-x", "admin@example.org"); err != nil {
		t.Fatal(err)
	}
	got, err := s.GetProposal(ctx, "prop-x")
	if err != nil {
		t.Fatal(err)
	}
	if got.SponsorApprovedBy != "admin@example.org" || got.SponsorApprovedAt == 0 {
		t.Errorf("sponsor approval provenance not recorded: %+v", got)
	}
	if got.Retroactive {
		t.Error("fresh rows must not be retroactive")
	}
	if err := s.RecordSponsorApproval(ctx, "nope", "x"); err == nil {
		t.Error("unknown id must error")
	}
}

// The delivery record is written at mint (addressed) and stamped on a send —
// and an address, once recorded, is never erased by a later blank.
func TestInviteDeliveryRecord(t *testing.T) {
	s := openTest(t)
	ctx := context.Background()

	inv := &Invite{Sponsor: "op@example.org", Env: "zagros", ExpiresAt: time.Now().Add(72 * time.Hour).Unix(),
		CandidateEmail: "  person-49@example.org  "}
	if _, err := s.MintInvite(ctx, inv); err != nil {
		t.Fatalf("MintInvite: %v", err)
	}
	got, err := s.GetInvite(ctx, inv.ID)
	if err != nil {
		t.Fatalf("GetInvite: %v", err)
	}
	if got.CandidateEmail != "person-49@example.org" {
		t.Fatalf("addressed-at-mint email = %q, want the trimmed address", got.CandidateEmail)
	}
	if got.SentAt != nil || got.SentVia != "" {
		t.Fatalf("mint must not claim a send: sent_at=%v via=%q", got.SentAt, got.SentVia)
	}

	// A blank address is a no-op, not an erase.
	if err := s.AddressInvite(ctx, inv.ID, "  "); err != nil {
		t.Fatalf("AddressInvite(blank): %v", err)
	}
	if got, _ = s.GetInvite(ctx, inv.ID); got.CandidateEmail != "person-49@example.org" {
		t.Fatalf("blank AddressInvite erased the recipient: %q", got.CandidateEmail)
	}

	at := time.Now().Unix()
	if err := s.RecordInviteSend(ctx, inv.ID, "person-73@example.org", SentViaEmail, at); err != nil {
		t.Fatalf("RecordInviteSend: %v", err)
	}
	got, _ = s.GetInvite(ctx, inv.ID)
	if got.CandidateEmail != "person-73@example.org" || got.SentVia != SentViaEmail || got.SentAt == nil || *got.SentAt != at {
		t.Fatalf("send record = %q/%q/%v, want the re-addressed send", got.CandidateEmail, got.SentVia, got.SentAt)
	}
}
