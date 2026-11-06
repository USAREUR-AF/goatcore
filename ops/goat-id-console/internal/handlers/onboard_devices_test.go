package handlers

import (
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// #2384: a blocked NetBird must not hold up either a stage list or a selected
// person's page. Increasing the roster must not introduce per-person requests.
func TestOnboardingDevices_PageDoesNotWaitForNetBird(t *testing.T) {
	for _, status := range []string{onboardstore.ProposalOnboarded, onboardstore.ProposalEnrolled, onboardstore.ProposalArchived} {
		for _, count := range []int{1, 12, 48} {
			t.Run(fmt.Sprintf("%s/%d", status, count), func(t *testing.T) {
				s := newOnboardTestServer(t)
				for i := range count {
					seedProposal(t, s, fmt.Sprintf("person-%d", i), "sponsor@example.test", "developer", status)
				}
				requested := make(chan struct{}, 1)
				release := make(chan struct{})
				var once sync.Once
				unblock := func() { once.Do(func() { close(release) }) }
				nb := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
					select {
					case requested <- struct{}{}:
					default:
					}
					select {
					case <-release:
					case <-r.Context().Done():
					}
					_, _ = w.Write([]byte(`[]`))
				}))
				defer nb.Close()
				defer unblock()
				s.Cfg.NotifyMgmtURL, s.Cfg.EnrollWatchToken = nb.URL, "fixture-token"
				h := s.Routes()
				for _, selection := range []string{"", "&id=person-0"} {
					done := make(chan *httptest.ResponseRecorder, 1)
					go func() {
						w := httptest.NewRecorder()
						h.ServeHTTP(w, httptest.NewRequestWithContext(t.Context(), http.MethodGet, "/onboarding?status="+status+selection, nil))
						done <- w
					}()
					select {
					case w := <-done:
						if w.Code != http.StatusOK || !strings.Contains(w.Body.String(), "cand-person-0") {
							t.Fatalf("page: %d %s", w.Code, w.Body.String())
						}
						wantDevices := selection != "" && status != onboardstore.ProposalArchived
						if got := strings.Contains(w.Body.String(), `data-devices-url="/onboarding/person-0/devices"`); got != wantDevices {
							t.Errorf("selected=%q: device placeholder=%v, want %v", selection, got, wantDevices)
						}
					case <-requested:
						unblock()
						<-done
						t.Fatal("page rendering requested NetBird peers before returning HTML")
					case <-time.After(2 * time.Second):
						unblock()
						t.Fatal("page did not finish while NetBird was blocked")
					}
				}
			})
		}
	}
}

func TestOnboardingDevices_Fragment(t *testing.T) {
	for _, status := range []string{onboardstore.ProposalOnboarded, onboardstore.ProposalEnrolled} {
		t.Run(status, func(t *testing.T) {
			s := newOnboardTestServer(t)
			seedProposal(t, s, "person", "sponsor@example.test", "developer", status)
			seedProposal(t, s, "person-extra", "sponsor@example.test", "developer", status)
			var calls atomic.Int32
			nb := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				calls.Add(1)
				if r.URL.Path != "/api/peers" || r.Header.Get("Authorization") != "Token fixture-token" {
					t.Errorf("unexpected NetBird request: %s", r.URL.Path)
				}
				_, _ = w.Write([]byte(`[{"name":"peer-cand-person-zagros-lap","os":"Windows 11 <script>bad()</script>","connected":true},{"name":"peer-cand-person-extra-zagros-lap","os":"Darwin"}]`))
			}))
			defer nb.Close()
			s.Cfg.NotifyMgmtURL, s.Cfg.EnrollWatchToken = nb.URL, "fixture-token"
			w := getOnboarding(t, s, "/onboarding/person/devices", nil)
			if w.Code != http.StatusOK || calls.Load() != 1 {
				t.Fatalf("fragment: status=%d peer requests=%d: %s", w.Code, calls.Load(), w.Body.String())
			}
			body := w.Body.String()
			if !strings.Contains(body, "peer-cand-person-zagros-lap") || !strings.Contains(body, "Windows 11 &lt;script&gt;") || strings.Contains(body, "peer-cand-person-extra") || strings.Contains(body, "<script>") {
				t.Fatalf("fragment escaped or attributed devices incorrectly: %s", body)
			}
			if w.Header().Get("Cache-Control") != "no-store" || !strings.HasPrefix(w.Header().Get("Content-Type"), "text/html") {
				t.Fatalf("fragment headers: %v", w.Header())
			}
		})
	}
}

func TestOnboardingDevices_RefusesIneligibleRowsAndNonAdmins(t *testing.T) {
	s := newOnboardTestServer(t)
	var calls atomic.Int32
	nb := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		_, _ = w.Write([]byte(`[]`))
	}))
	defer nb.Close()
	s.Cfg.NotifyMgmtURL, s.Cfg.EnrollWatchToken = nb.URL, "fixture-token"
	for _, stage := range onboardStageDefs {
		if stage.Key == onboardStageInvited || stage.Key == onboardstore.ProposalOnboarded || stage.Key == onboardstore.ProposalEnrolled {
			continue
		}
		seedProposal(t, s, stage.Key, "sponsor@example.test", "developer", stage.Key)
		if w := getOnboarding(t, s, "/onboarding/"+stage.Key+"/devices", nil); w.Code != http.StatusNotFound {
			t.Errorf("%s: want 404, got %d", stage.Key, w.Code)
		}
	}
	if w := getOnboarding(t, s, "/onboarding/missing/devices", nil); w.Code != http.StatusNotFound {
		t.Errorf("missing: want 404, got %d", w.Code)
	}
	seedProposal(t, s, "person", "sponsor@example.test", "developer", onboardstore.ProposalEnrolled)
	s.Cfg.DevBypassAuth = false
	for _, hdr := range []map[string]string{
		nil,
		{"X-Forwarded-Email": "sponsor@example.test", "X-Forwarded-Groups": "OBSERVERS"},
	} {
		if w := getOnboarding(t, s, "/onboarding/person/devices", hdr); w.Code != http.StatusForbidden {
			t.Errorf("non-admin: want 403, got %d", w.Code)
		}
	}
	if calls.Load() != 0 {
		t.Errorf("refused requests made %d NetBird calls", calls.Load())
	}
}

func TestOnboardingDevices_Unavailable(t *testing.T) {
	for _, failure := range []string{"unconfigured", "upstream"} {
		t.Run(failure, func(t *testing.T) {
			s := newOnboardTestServer(t)
			seedProposal(t, s, "person", "sponsor@example.test", "developer", onboardstore.ProposalEnrolled)
			if failure == "upstream" {
				nb := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
					w.WriteHeader(http.StatusServiceUnavailable)
				}))
				defer nb.Close()
				s.Cfg.NotifyMgmtURL, s.Cfg.EnrollWatchToken = nb.URL, "fixture-token"
			}
			w := getOnboarding(t, s, "/onboarding/person/devices", nil)
			if w.Code != http.StatusOK || !strings.Contains(w.Body.String(), "mesh device list unavailable") {
				t.Fatalf("device failure must remain visible: %d %s", w.Code, w.Body.String())
			}
		})
	}
}

func TestOnboardingDevices_SponsorHasNoDevicePlaceholder(t *testing.T) {
	s := newOnboardTestServer(t)
	s.Cfg.DevBypassAuth = false
	portalURL, _ := fakeSponsorPortal(t, nil, map[string]sponsorLookupResponse{
		"sponsor@example.test": {Org: "fixture-org", Alias: "fixture-sponsor", SponsorAuthorized: true, SponsorMaxTier: "developer"},
	})
	s.Cfg.PortalInternalURL, s.Cfg.PortalLookupToken = portalURL, "fixture-lookup-token"
	hdr := map[string]string{"X-Forwarded-Email": "sponsor@example.test", "X-Forwarded-Groups": "OBSERVER"}
	for _, status := range []string{onboardstore.ProposalOnboarded, onboardstore.ProposalEnrolled} {
		seedProposal(t, s, status, "sponsor@example.test", "developer", status)
		w := getOnboarding(t, s, "/onboarding?status="+status+"&id="+status, hdr)
		if w.Code != http.StatusOK || strings.Contains(w.Body.String(), "data-devices-url") || strings.Contains(w.Body.String(), "/static/onboard-devices.js") {
			t.Fatalf("sponsor must see the candidate without device loading: status=%d", w.Code)
		}
	}
}
