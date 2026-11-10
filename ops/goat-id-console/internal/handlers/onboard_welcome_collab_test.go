package handlers

import (
	"context"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// The console's welcome download is what an admin reaches for when the mailed
// package is lost, so it has to carry what the mail carried, including the
// collaboration account the mail named. The one-time set-up code is deliberately
// not stored and must not appear in either package.
func TestWelcomeDownloadNamesTheCollabAccount(t *testing.T) {
	s := newOnboardTestServer(t)
	s.Cfg.NotifyPortalURL = "https://portal.zagros.netbird.zagros.example"
	s.Cfg.NotifyMgmtURL = "https://netbird.zagros.example"
	h := s.Routes()
	ctx := context.Background()

	p := &onboardstore.Proposal{Sponsor: "jane-admin", Env: "zagros", Handle: "pat-liaison",
		Email:        "person-105@example.org",
		ManifestJSON: `{"identity":{"handle":"pat-liaison","email":"person-105@example.org","name":"Pat Liaison","status":"active","added":"2026-06-01","added_by":"admin@example.test"},"netbird":{"enabled":true,"sites":["zagros"],"role":"peer","auto_groups":["OBSERVER"],"handset_limit":1},"ory_site":{"enabled":false},"ory_idp":{"enabled":false},"zenoh":{"enabled":false}}`, Tier: "observer"}
	if err := s.Onboard.CreateProposal(ctx, p); err != nil {
		t.Fatal(err)
	}
	if err := s.Onboard.SetProposalResult(ctx, p.ID, onboardstore.ProposalOnboarded,
		"DEVICE-KEY", "", "", "", "pat.liaison", true); err != nil {
		t.Fatal(err)
	}

	req := httptest.NewRequestWithContext(ctx, http.MethodGet, "/onboarding/"+p.ID+"/welcome", nil)
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("welcome download = %d: %s", rec.Code, rec.Body.String())
	}
	body := rec.Body.String()

	for _, want := range []string{
		"COLLABORATION ENVIRONMENT",
		"pat.liaison",
		"/setup-code",
	} {
		if !strings.Contains(body, want) {
			t.Errorf("welcome download missing %q:\n%s", want, body)
		}
	}
	for _, unwanted := range []string{"Paste this code", "valid 24 hours"} {
		if strings.Contains(body, unwanted) {
			t.Errorf("welcome download must not imply a set-up credential (%q):\n%s", unwanted, body)
		}
	}
}
