package handlers

import (
	"context"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// A missing destination list should leave the peer and its current access
// visible while the administrator is choosing a move.
func TestUserDetailMoveGroup_DestinationGroupsUnavailable(t *testing.T) {
	for _, tc := range []struct {
		name   string
		status int
	}{
		{name: "empty group list", status: http.StatusOK},
		{name: "group lookup fails", status: http.StatusBadGateway},
	} {
		t.Run(tc.name, func(t *testing.T) {
			ctx := context.Background()
			const email = "move-target@example.test"
			const peerName = "peer-move-target-acme-zagros-20260604-laptop"
			s := newAccessTestServer(t, email, "Move Target", []string{"PLATFORM-ADMIN"})
			s.Cfg.MembershipAuthority = "ungoverned"
			s.Cfg.HoldRecordsPath = filepath.Clean(holdRecordsFixturePath)
			s.Cfg.EnrollWatchToken = "NETBIRD-TOKEN-SECRET"
			if err := s.Onboard.CreateProposal(ctx, &onboardstore.Proposal{
				Sponsor:      "sponsor-1",
				Env:          "zagros",
				Handle:       "move-target",
				Email:        email,
				Status:       onboardstore.ProposalOnboarded,
				ManifestJSON: "{}",
				Tier:         "developer",
			}); err != nil {
				t.Fatalf("CreateProposal: %v", err)
			}

			var groupLookups atomic.Int32
			nb := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				w.Header().Set("Content-Type", "application/json")
				switch r.URL.Path {
				case "/api/peers":
					_, _ = w.Write([]byte(`[{"id":"p-move","name":"` + peerName + `","groups":[{"id":"g-dev","name":"ZAGROS-DEVELOPER"}]}]`))
				case "/api/setup-keys":
					_, _ = w.Write([]byte(`[]`))
				case "/api/groups":
					groupLookups.Add(1)
					w.WriteHeader(tc.status)
					_, _ = w.Write([]byte(`[]`))
				default:
					http.NotFound(w, r)
				}
			}))
			t.Cleanup(nb.Close)
			s.Cfg.NotifyMgmtURL = nb.URL

			req := httptest.NewRequestWithContext(ctx, http.MethodGet, "/users/id-1?idp=tenant", nil)
			req.Header.Set("X-Forwarded-Email", "admin@example.test")
			req.Header.Set("X-Forwarded-Groups", "PLATFORM-ADMIN")
			rec := httptest.NewRecorder()
			s.Routes().ServeHTTP(rec, req)

			if rec.Code != http.StatusOK {
				t.Fatalf("status = %d, want 200; body: %s", rec.Code, rec.Body.String())
			}
			if got := groupLookups.Load(); got != 1 {
				t.Fatalf("group lookups = %d, want 1", got)
			}
			body := rec.Body.String()
			for _, want := range []string{peerName, "ZAGROS-DEVELOPER", "No destination groups are available.", `data-move-preview disabled`} {
				if !strings.Contains(body, want) {
					t.Errorf("user detail is missing %q", want)
				}
			}
			if strings.Contains(body, `aria-autocomplete="list"`) {
				t.Error("user detail offers destination selection without a group list")
			}
		})
	}
}
