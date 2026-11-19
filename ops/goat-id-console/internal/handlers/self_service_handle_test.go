package handlers

import (
	"context"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// The self-service pages bind the signed-in person's handle through their
// Kratos identity and settled onboarding records only. An intake row filed
// later with the victim's email and another handle (any invitee or intake
// key can file one) does not steer the victim's SSH-key or operator-key
// request, or their identity page, to the planted handle.

const victimID = "0b1f0000-0000-4000-8000-0000000002a1"

func victimServer(t *testing.T) *Server {
	t.Helper()
	forgetIdentity(victimID)
	kc, _ := fakeKratosMulti(t,
		fakeIdent{id: victimID, email: "victim@example.test", name: "Vic", state: "active", groups: []string{"PLATFORM-ADMIN"}})
	s := newUsersAPITestServer(t, kc)
	s.Cfg.LiveIdentityCheck = true
	s.Cfg.NotifyGoatnet = "zagros"
	ctx := context.Background()
	if err := s.Onboard.CreateProposal(ctx, &onboardstore.Proposal{Sponsor: "sponsor-1", Env: "zagros", CreatedAt: time.Now().Unix() - 1000,
		Handle: "vic-own", Email: "victim@example.test", Status: onboardstore.ProposalOnboarded, ManifestJSON: "{}", Tier: "platform-admin"}); err != nil {
		t.Fatal(err)
	}
	// The planted intake row: newer, the victim's email, another handle.
	if err := s.Onboard.CreateProposal(ctx, &onboardstore.Proposal{Sponsor: "intake-key", Env: "zagros", CreatedAt: time.Now().Unix() + 1000,
		Handle: "planted", Email: "victim@example.test", Status: onboardstore.ProposalProposed, ManifestJSON: "{}"}); err != nil {
		t.Fatal(err)
	}
	return s
}

func victimReq(t *testing.T, s *Server, method, path string, form url.Values) *httptest.ResponseRecorder {
	t.Helper()
	var body *strings.Reader
	if form != nil {
		form.Set("csrf", s.csrfNonce)
		body = strings.NewReader(form.Encode())
	} else {
		body = strings.NewReader("")
	}
	req := httptest.NewRequestWithContext(context.Background(), method, path, body)
	if form != nil {
		req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	}
	req.Header.Set("X-Forwarded-Email", "victim@example.test")
	req.Header.Set("X-Forwarded-User", victimID)
	req.Header.Set("X-Forwarded-Groups", "PLATFORM-ADMIN")
	rec := httptest.NewRecorder()
	s.Routes().ServeHTTP(rec, req)
	return rec
}

func TestSelfService_plantedIntakeRowDoesNotSteerHandle(t *testing.T) {
	ctx := context.Background()
	t.Run("ssh key", func(t *testing.T) {
		s := victimServer(t) //nolint:contextcheck // behavior kept as reviewed for disclosure; lint cleanup goatcore#1028
		rec := victimReq(t, s, http.MethodPost, "/ssh-key", url.Values{"pubkey": {testPubKeyType + " " + testPubKeyBody + " vic@laptop"}}) //nolint:contextcheck // behavior kept as reviewed for disclosure; lint cleanup goatcore#1028
		if rec.Code >= 400 {
			t.Fatalf("ssh-key submit: %d %s", rec.Code, rec.Body.String())
		}
		ks, err := s.Onboard.ListSSHKeys(ctx, "")
		if err != nil || len(ks) != 1 || ks[0].Handle != "vic-own" {
			t.Fatalf("ssh key rows = %+v %v, want one under vic-own", ks, err)
		}
	})
	t.Run("operator key", func(t *testing.T) {
		s := victimServer(t) //nolint:contextcheck // behavior kept as reviewed for disclosure; lint cleanup goatcore#1028
		rec := victimReq(t, s, http.MethodPost, "/operator-key", url.Values{"reason": {"admin task"}}) //nolint:contextcheck // behavior kept as reviewed for disclosure; lint cleanup goatcore#1028
		if rec.Code >= 400 {
			t.Fatalf("operator-key request: %d %s", rec.Code, rec.Body.String())
		}
		ks, err := s.Onboard.ListOperatorKeys(ctx, "")
		if err != nil || len(ks) != 1 || ks[0].Handle != "vic-own" {
			t.Fatalf("operator key rows = %+v %v, want one under vic-own", ks, err)
		}
	})
	t.Run("identity page", func(t *testing.T) {
		s := victimServer(t)
		rec := victimReq(t, s, http.MethodGet, "/identity", nil)
		if body := rec.Body.String(); rec.Code != http.StatusOK || !strings.Contains(body, "vic-own") || strings.Contains(body, "planted") {
			t.Fatalf("identity page: %d, want vic-own and not planted", rec.Code)
		}
	})
}

// Two settled records under two handles for one person: the self-service
// door refuses rather than pick one.
func TestSelfService_twoSettledHandlesRefused(t *testing.T) {
	s := victimServer(t)
	if err := s.Onboard.CreateProposal(context.Background(), &onboardstore.Proposal{Sponsor: "sponsor-1", Env: "zagros", CreatedAt: time.Now().Unix() + 2000,
		Handle: "vic-second", Email: "victim@example.test", Status: onboardstore.ProposalOnboarded, ManifestJSON: "{}"}); err != nil {
		t.Fatal(err)
	}
	rec := victimReq(t, s, http.MethodPost, "/ssh-key", url.Values{"pubkey": {testPubKeyType + " " + testPubKeyBody + " vic@laptop"}})
	if rec.Code != http.StatusForbidden {
		t.Fatalf("two settled handles: %d, want 403", rec.Code)
	}
	if ks, _ := s.Onboard.ListSSHKeys(context.Background(), ""); len(ks) != 0 {
		t.Fatalf("filed a key despite two handles: %+v", ks)
	}
}
