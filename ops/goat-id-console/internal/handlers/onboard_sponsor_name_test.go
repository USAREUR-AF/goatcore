package handlers

// onboard_sponsor_name_test.go — the sponsor display-name chain. A participant
// sponsor's machine identity is its UUID org anchor (what the portal whoami
// returns as `vendor`); the review surfaces must show WHO that is, so the
// portal display_name is resolved at mint and carried invite → proposal as a
// display-only sponsor_name alongside the unchanged machine sponsor value.

import (
	"bytes"
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// TestParticipantSponsorNameResolvedAtMint mints an invite through the machine
// realm with a PARTICIPANT key (not in the operator map): the console delegates
// to the portal whoami, records the UUID anchor as the machine sponsor, and the
// portal display_name as the human sponsor_name.
func TestParticipantSponsorNameResolvedAtMint(t *testing.T) {
	const anchor = "0ff404ddade261f5af6631c442b5a299"
	portalURL, portalRT := fakeSponsorPortal(t, map[string]sponsorLookupResponse{
		"some-participant-portal-key": {Org: anchor, Alias: "maintainer-alias", SponsorAuthorized: true, SponsorMaxTier: "developer"},
	}, nil)
	// sponsorLookupResponse is the /sponsorship shape and carries no
	// display_name, so stamp it onto the whoami the stub serves. Alias differs
	// deliberately: asserting "maintainer-operator" below proves display_name wins
	// the firstNonEmpty chain instead of passing through the Alias fallback.
	wh := portalRT.keys["some-participant-portal-key"]
	wh.DisplayName = "maintainer-operator"
	portalRT.keys["some-participant-portal-key"] = wh

	s := newOnboardTestServer(t)
	s.Cfg.PortalInternalURL = portalURL
	h := s.Routes()

	body, _ := json.Marshal(map[string]any{"env": "zagros", "tier": "developer"})
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/api/onboard/invite", bytes.NewReader(body))
	req.Header.Set("Authorization", "Bearer some-participant-portal-key")
	req.Header.Set("Content-Type", "application/json")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusCreated {
		t.Fatalf("participant mint: want 201, got %d: %s", rec.Code, rec.Body.String())
	}
	var resp mintInviteResponse
	if err := json.Unmarshal(rec.Body.Bytes(), &resp); err != nil {
		t.Fatalf("decode mint: %v", err)
	}
	if resp.Sponsor != anchor {
		t.Fatalf("machine sponsor must stay the anchor: want %q, got %q", anchor, resp.Sponsor)
	}
	if resp.SponsorName != "maintainer-operator" {
		t.Fatalf("sponsor_name: want maintainer-operator, got %q", resp.SponsorName)
	}

	// Persisted on the invite row too.
	inv, err := s.Onboard.GetInvite(context.Background(), resp.ID)
	if err != nil {
		t.Fatalf("get invite: %v", err)
	}
	if inv.Sponsor != anchor || inv.SponsorName != "maintainer-operator" {
		t.Fatalf("invite row: want (%q, maintainer-operator), got (%q, %q)", anchor, inv.Sponsor, inv.SponsorName)
	}
}

// TestSponsorNameCarriesToPreviewAndProposal drives the public leg: the
// candidate preview shows the human name (never the anchor), and the consumed
// invite stamps sponsor + sponsor_name onto the proposal.
func TestSponsorNameCarriesToPreviewAndProposal(t *testing.T) {
	const anchor = "0ff404ddade261f5af6631c442b5a299"
	s := newOnboardTestServer(t)
	h := s.Routes()

	pb := onboardstore.Prebound{Tier: "developer", Affiliation: "partner"}
	pbJSON, _ := json.Marshal(pb)
	inv := &onboardstore.Invite{Sponsor: anchor, SponsorName: "maintainer-operator", Env: "zagros", PreboundJSON: string(pbJSON)}
	tok, err := s.Onboard.MintInvite(context.Background(), inv)
	if err != nil {
		t.Fatalf("mint via store: %v", err)
	}

	pv := preview(t, h, tok)
	if !pv.Valid {
		t.Fatalf("preview: want valid, got reason=%q", pv.Reason)
	}
	if pv.Sponsor != "maintainer-operator" {
		t.Fatalf("preview sponsor must be the human label: want maintainer-operator, got %q", pv.Sponsor)
	}

	pid := submit(t, h, tok, map[string]any{"name": "Pat Liaison", "email": "person-106@example.org", "organization": "Sponsor Org"})
	p, err := s.Onboard.GetProposal(context.Background(), pid)
	if err != nil {
		t.Fatalf("get proposal: %v", err)
	}
	if p.Sponsor != anchor {
		t.Fatalf("proposal machine sponsor: want %q, got %q", anchor, p.Sponsor)
	}
	if p.SponsorName != "maintainer-operator" {
		t.Fatalf("proposal sponsor_name: want maintainer-operator, got %q", p.SponsorName)
	}
}
