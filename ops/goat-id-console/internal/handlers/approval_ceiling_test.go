package handlers

import (
	"context"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// An admin must not approve a tier above their own, including on a second
// proposal they self-submitted under a second mailbox. Approving `browse` and
// approving `platform-admin` must not run identical code: the approve arm reads
// both the proposal's tier and the actor.

// httptestReq wraps a request so each subtest reads as one line.
type httptestReq struct{ r *http.Request }

func TestActorTierIsDerivedFromTheSameTableTheFormOffers(t *testing.T) {
	h, err := loadIntakeHints()
	if err != nil {
		t.Fatalf("loadIntakeHints: %v", err)
	}
	for _, tc := range []struct {
		name   string
		groups []string
		want   string
		ok     bool
	}{
		{"platform admin", []string{"PLATFORM-ADMIN"}, "platform-admin", true},
		{"iam admin", []string{"IAM-ADMIN", "GRAFANA-READONLY"}, "iam-admin", true},
		{"case is not significant", []string{"platform-admin"}, "platform-admin", true},
		{"highest held wins", []string{"IAM-ADMIN", "GRAFANA-READONLY", "PLATFORM-ADMIN"}, "platform-admin", true},
		{"partial groups do not grant the tier", []string{"IAM-ADMIN"}, "", false},
		{"no groups at all", nil, "", false},
		{"unrelated groups", []string{"COMPLIANCE", "OBS-ADMIN"}, "", false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			id, _, ok := h.actorTier(tc.groups)
			if ok != tc.ok || id != tc.want {
				t.Errorf("actorTier(%v) = (%q, %v), want (%q, %v)", tc.groups, id, ok, tc.want, tc.ok)
			}
		})
	}
}

func TestTierRanksOrderTheLadder(t *testing.T) {
	h, err := loadIntakeHints()
	if err != nil {
		t.Fatalf("loadIntakeHints: %v", err)
	}
	// The ceiling is only meaningful if the ladder is ordered as the model says.
	var browse, dev, iam, plat int
	for _, k := range []struct {
		id string
		to *int
	}{{"browse", &browse}, {"developer", &dev}, {"iam-admin", &iam}, {"platform-admin", &plat}} {
		r, ok := h.tierRank(k.id)
		if !ok {
			t.Fatalf("tier %q missing from the table", k.id)
		}
		*k.to = r
	}
	if browse >= dev || dev >= iam || iam >= plat {
		t.Errorf("tier ranks are not ordered browse<developer<iam-admin<platform-admin: %d %d %d %d",
			browse, dev, iam, plat)
	}
}

func TestApprovalCeiling(t *testing.T) {
	s := &Server{}
	mk := func(email, tier string) *onboardstore.Proposal {
		return &onboardstore.Proposal{Email: email, Tier: tier}
	}
	withActor := func(email string, groups []string) *httptestReq {
		r := httptest.NewRequestWithContext(context.Background(), "POST", "/onboarding/x/approve", nil)
		return &httptestReq{r.WithContext(context.WithValue(r.Context(),
			ctxKeyActor, Actor{Email: email, Groups: groups}))}
	}

	t.Run("an admin may not approve above their own tier", func(t *testing.T) {
		r := withActor("iam@example.test", []string{"IAM-ADMIN", "GRAFANA-READONLY"})
		e := s.approvalCeiling(r.r, mk("someone@example.test", "platform-admin"), "iam@example.test")
		if e == nil {
			t.Fatal("IAM-ADMIN approving PLATFORM-ADMIN must be refused — this is the F-490 case")
		}
		if !strings.Contains(e.msg, "up to their own tier") {
			t.Errorf("refusal should say why: %q", e.msg)
		}
	})

	t.Run("an admin may approve at their own tier", func(t *testing.T) {
		r := withActor("iam@example.test", []string{"IAM-ADMIN", "GRAFANA-READONLY"})
		if e := s.approvalCeiling(r.r, mk("someone@example.test", "iam-admin"), "iam@example.test"); e != nil {
			t.Errorf("same-tier approval must be allowed, got %q", e.msg)
		}
	})

	t.Run("an admin may approve below their own tier", func(t *testing.T) {
		r := withActor("iam@example.test", []string{"IAM-ADMIN", "GRAFANA-READONLY"})
		if e := s.approvalCeiling(r.r, mk("someone@example.test", "browse"), "iam@example.test"); e != nil {
			t.Errorf("lower-tier approval must be allowed, got %q", e.msg)
		}
	})

	t.Run("nobody approves their own application, at any tier", func(t *testing.T) {
		r := withActor("self@example.test", []string{"PLATFORM-ADMIN"})
		e := s.approvalCeiling(r.r, mk("self@example.test", "browse"), "self@example.test")
		if e == nil {
			t.Fatal("self-approval must be refused even for the top tier approving the lowest")
		}
		if !strings.Contains(e.msg, "your own onboarding") {
			t.Errorf("refusal should name the reason: %q", e.msg)
		}
	})

	t.Run("self-approval is matched case- and space-insensitively", func(t *testing.T) {
		r := withActor("  Self@Example.TEST ", []string{"PLATFORM-ADMIN"})
		if e := s.approvalCeiling(r.r, mk("self@example.test", "browse"), "  Self@Example.TEST "); e == nil {
			t.Error("a differently-cased address is the same mailbox and must still be refused")
		}
	})

	t.Run("groups matching no tier cannot approve anything", func(t *testing.T) {
		r := withActor("odd@example.test", []string{"COMPLIANCE"})
		if e := s.approvalCeiling(r.r, mk("someone@example.test", "browse"), "odd@example.test"); e == nil {
			t.Error("an unknown ceiling must refuse, never default to permissive")
		}
	})

	t.Run("a sponsor is capped by their sponsorship, not by groups", func(t *testing.T) {
		// A sponsor is not a console admin and holds no admin groups — deriving
		// their ceiling from groups would refuse every legitimate vouch, which
		// is what the first version of this did.
		withSponsor := func(maxTier string) *http.Request {
			r := httptest.NewRequestWithContext(context.Background(), "POST", "/onboarding/x/sponsor-approve", nil)
			ctx := context.WithValue(r.Context(), ctxKeyActor, Actor{Email: "sp@example.test"})
			ctx = context.WithValue(ctx, ctxKeySponsor, &sponsorSession{
				Email: "sp@example.test", MaxTier: maxTier,
			})
			return r.WithContext(ctx)
		}
		if e := s.approvalCeiling(withSponsor("developer"), mk("cand@example.test", "developer"), "sp@example.test"); e != nil {
			t.Errorf("vouching at their own level must be allowed, got %q", e.msg)
		}
		e := s.approvalCeiling(withSponsor("developer"), mk("cand@example.test", "platform-admin"), "sp@example.test")
		if e == nil {
			t.Fatal("a developer-capped sponsor must not vouch for platform-admin")
		}
		if !strings.Contains(e.msg, "up to their own level") {
			t.Errorf("refusal should say why: %q", e.msg)
		}
		// Empty MaxTier = operator-minted sponsorship = unrestricted, the same
		// reading the invite-mint path already uses. Changing that would silently
		// narrow every existing sponsor.
		if e := s.approvalCeiling(withSponsor(""), mk("cand@example.test", "platform-admin"), "sp@example.test"); e != nil {
			t.Errorf("an uncapped sponsor must keep their existing reach, got %q", e.msg)
		}
		// The self rule still applies to a sponsor.
		if e := s.approvalCeiling(withSponsor(""), mk("sp@example.test", "browse"), "sp@example.test"); e == nil {
			t.Error("a sponsor must not vouch for their own application")
		}
	})

	t.Run("a service operator key keeps the ceiling exemption but not the self rule", func(t *testing.T) {
		// No browser groups and a service operator key (ADR 1071): top of the
		// ladder, so the ceiling is vacuous. Anything else with no groups (a
		// per-person key, no principal at all) fails closed on an admin tier
		// (GHSA-vr6w).
		r := withActor("", nil)
		svc := r.r.WithContext(context.WithValue(r.r.Context(), ctxKeyOnboardPrincipal, onboardPrincipal{Handle: "onboard-api", IsOperator: true}))
		if e := s.approvalCeiling(svc, mk("someone@example.test", "iam-admin"), "onboard-api"); e != nil {
			t.Errorf("service operator key must not be ceiling-checked below the platform tier, got %q", e.msg)
		}
		// ADR 1071 as amended: no operator key approves the platform tier.
		if e := s.approvalCeiling(svc, mk("someone@example.test", "platform-admin"), "onboard-api"); e == nil {
			t.Error("a service operator key must not approve the platform tier")
		}
		if e := s.approvalCeiling(r.r, mk("someone@example.test", "platform-admin"), "onboard-api"); e == nil {
			t.Error("a principal with no groups and no service key must not approve an admin tier")
		}
		r2 := withActor("", nil)
		if e := s.approvalCeiling(r2.r, mk("api@example.test", "browse"), "api@example.test"); e == nil {
			t.Error("even an operator principal must not approve its own application")
		}
	})
}
