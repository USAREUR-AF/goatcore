package handlers

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/kratos"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

func releaseTestServer(t *testing.T, lane string, prior []string, liveGroups ...string) (*Server, time.Time) {
	t.Helper()
	s := newOnboardTestServer(t)
	s.Cfg.MembershipAuthority = authorityUngoverned
	entered := time.Date(2026, time.October, 20, 10, 11, 12, 0, time.UTC)
	holdGroup := "ZAGROS-PROBATION-DEVELOPER"
	if lane == "security" {
		holdGroup = "ZAGROS-QUARANTINED"
	}
	record := map[string]any{
		"v": 1, "lane": lane, "site": "zagros", "peer_id": "peer-1",
		"peer_name": "peer-jane-zagros-laptop", "handle": "jane",
		"entered_at": entered.Format(time.RFC3339), "hold_group": holdGroup,
		"prior_groups": prior, "actor": "sweep", "reason": "inactive",
	}
	raw, _ := json.Marshal(record)
	s.Cfg.HoldRecordsPath = filepath.Join(t.TempDir(), "hold-records.jsonl")
	if err := os.WriteFile(s.Cfg.HoldRecordsPath, append(raw, '\n'), 0o600); err != nil {
		t.Fatal(err)
	}
	groups := []map[string]any{{"id": "all", "name": "All"}, {"id": "hold", "name": holdGroup}}
	for i, name := range prior {
		groups = append(groups, map[string]any{"id": "prior-" + string(rune('a'+i)), "name": name})
	}
	peerGroups := []map[string]any{{"id": "all", "name": "All"}}
	for _, name := range liveGroups {
		peerGroups = append(peerGroups, map[string]any{"id": "live-" + name, "name": name})
	}
	nb := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		switch r.URL.Path {
		case "/api/peers":
			_ = json.NewEncoder(w).Encode([]map[string]any{{"id": "peer-1", "name": "peer-jane-zagros-laptop", "groups": peerGroups}})
		case "/api/groups":
			_ = json.NewEncoder(w).Encode(groups)
		default:
			w.WriteHeader(http.StatusNotFound)
		}
	}))
	t.Cleanup(nb.Close)
	s.Cfg.NotifyMgmtURL = nb.URL
	s.Cfg.EnrollWatchToken = "test-token"
	p := &onboardstore.Proposal{Sponsor: "sponsor", Env: "zagros", Handle: "admin-handle", Email: "dev@bypass.local", Status: onboardstore.ProposalOnboarded, ManifestJSON: `{}`}
	if err := s.Onboard.CreateProposal(context.Background(), p); err != nil {
		t.Fatal(err)
	}
	return s, entered
}

func TestReleaseHoldPreviewAndConfirmUseExactRecord(t *testing.T) {
	s, entered := releaseTestServer(t, "dormancy", []string{"ZAGROS-DEVELOPER", "ZAGROS-P2P"}, "ZAGROS-PROBATION-DEVELOPER")
	target := "/holds/peer-1/release?entered_at=" + url.QueryEscape(stringInt(entered.Unix()))
	preview := httptest.NewRecorder()
	s.Routes().ServeHTTP(preview, httptest.NewRequestWithContext(context.Background(), http.MethodGet, target, nil))
	if preview.Code != http.StatusOK {
		t.Fatalf("preview status=%d body=%s", preview.Code, preview.Body.String())
	}
	for _, want := range []string{"peer-jane-zagros-laptop", "ZAGROS-DEVELOPER", "ZAGROS-P2P", "sweep", "No matching open request"} {
		if !strings.Contains(preview.Body.String(), want) {
			t.Errorf("preview missing %q", want)
		}
	}
	form := url.Values{"csrf": {s.csrfNonce}, "entered_at": {stringInt(entered.Unix())}}
	post := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/holds/peer-1/release", strings.NewReader(form.Encode()))
	post.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	post.Header.Set("X-Forwarded-User", testActorID)
	rec := httptest.NewRecorder()
	s.Routes().ServeHTTP(rec, post)
	if rec.Code != http.StatusSeeOther {
		t.Fatalf("confirm status=%d body=%s", rec.Code, rec.Body.String())
	}
	// The requester is the session's identity, not the admin-handle record
	// that carries the same email (GHSA-3m4r review).
	actions, err := s.Onboard.ListActions(context.Background(), onboardstore.ActionQueued)
	if err != nil || len(actions) != 1 || actions[0].ProposalID != "" || actions[0].Env != "zagros" || actions[0].Handle != "kratos-"+testActorID {
		t.Fatalf("queued actions=%+v err=%v", actions, err)
	}
	for _, want := range []string{`"peer_id":"peer-1"`, `"hold_entered_at":"2026-10-20T10:11:12Z"`, `"prior_groups":["ZAGROS-DEVELOPER","ZAGROS-P2P"]`} {
		if !strings.Contains(actions[0].Params, want) {
			t.Errorf("immutable params missing %s: %s", want, actions[0].Params)
		}
	}
}

func TestReleaseHoldConfirmRequiresFreshStepUpBeforeEnqueue(t *testing.T) {
	s, entered := releaseTestServer(t, "dormancy", []string{"ZAGROS-DEVELOPER"}, "ZAGROS-PROBATION-DEVELOPER")
	s.Cfg.DevBypassAuth = false
	s.Cfg.StepUpEnforced = true
	s.Cfg.StepUpSSOIdP = "tenant"
	s.Cfg.AAL2FreshWindow = 5 * time.Minute
	p := &onboardstore.Proposal{Sponsor: "sponsor", Env: "zagros", Handle: "real-admin", Email: "admin@example.org", Status: onboardstore.ProposalOnboarded, ManifestJSON: `{}`}
	if err := s.Onboard.CreateProposal(context.Background(), p); err != nil {
		t.Fatal(err)
	}
	post := func() *httptest.ResponseRecorder {
		t.Helper()
		form := url.Values{"csrf": {s.csrfNonce}, "entered_at": {stringInt(entered.Unix())}}
		req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/holds/peer-1/release?idp=tenant", strings.NewReader(form.Encode()))
		req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
		req.Header.Set("X-Forwarded-Groups", "ADMINS")
		req.Header.Set("X-Forwarded-Email", "admin@example.org")
		req.Header.Set("X-Forwarded-User", testActorID)
		rec := httptest.NewRecorder()
		s.Routes().ServeHTTP(rec, req)
		return rec
	}

	passwordOnly, _ := fakeKratos2FA(t, testActorID, "admin@example.org", http.StatusOK, http.StatusNoContent, posturePasswordOnly)
	s.Kratos = map[string]*kratos.Client{"tenant": passwordOnly}
	if rec := post(); rec.Code != http.StatusSeeOther || !strings.HasPrefix(rec.Header().Get("Location"), "/step-up?") {
		t.Fatalf("unverified release status=%d location=%q body=%s", rec.Code, rec.Header().Get("Location"), rec.Body.String())
	}
	if actions, err := s.Onboard.ListActions(context.Background(), onboardstore.ActionQueued); err != nil || len(actions) != 0 {
		t.Fatalf("unverified release queued actions=%+v err=%v", actions, err)
	}

	fresh, _ := fakeKratos2FA(t, testActorID, "admin@example.org", http.StatusOK, http.StatusNoContent, postureFreshAAL2)
	s.Kratos["tenant"] = fresh
	if rec := post(); rec.Code != http.StatusSeeOther || rec.Header().Get("Location") != "/holds?release=queued" {
		t.Fatalf("verified release status=%d location=%q body=%s", rec.Code, rec.Header().Get("Location"), rec.Body.String())
	}
	if actions, err := s.Onboard.ListActions(context.Background(), onboardstore.ActionQueued); err != nil || len(actions) != 1 {
		t.Fatalf("verified release queued actions=%+v err=%v", actions, err)
	}
}

func TestReleaseHoldDirectPostRefusesSecurityAndEnforcingAuthority(t *testing.T) {
	for _, tc := range []struct {
		name  string
		lane  string
		scope string
		want  string
	}{
		{"security", "security", "", "incident close-out"},
		{"enforcing", "dormancy", "version: 1\nscope: zagros\nconverge: enforce\n", "warrant-governed"},
		{"unknown", "dormancy", "not: [valid", "authority is unknown"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			hold := "ZAGROS-PROBATION-DEVELOPER"
			if tc.lane == "security" {
				hold = "ZAGROS-QUARANTINED"
			}
			s, entered := releaseTestServer(t, tc.lane, []string{"ZAGROS-DEVELOPER"}, hold)
			if tc.scope != "" {
				path := filepath.Join(t.TempDir(), "_scope.yaml")
				if err := os.WriteFile(path, []byte(tc.scope), 0o600); err != nil {
					t.Fatal(err)
				}
				s.Cfg.MembershipScopePath = path
				s.Cfg.MembershipAuthority = "scope"
			}
			form := url.Values{"csrf": {s.csrfNonce}, "entered_at": {stringInt(entered.Unix())}}
			req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/holds/peer-1/release", strings.NewReader(form.Encode()))
			req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
			rec := httptest.NewRecorder()
			s.Routes().ServeHTTP(rec, req)
			if rec.Code != http.StatusConflict || !strings.Contains(rec.Body.String(), tc.want) {
				t.Fatalf("status=%d body=%s, want %q", rec.Code, rec.Body.String(), tc.want)
			}
			actions, _ := s.Onboard.ListActions(context.Background(), onboardstore.ActionQueued)
			if len(actions) != 0 {
				t.Fatalf("refusal queued actions: %+v", actions)
			}
		})
	}
}

func TestReleaseHoldRefusesWhenAuthorityConfigurationIsOmitted(t *testing.T) {
	s, entered := releaseTestServer(t, "dormancy", []string{"ZAGROS-DEVELOPER"}, "ZAGROS-PROBATION-DEVELOPER")
	s.Cfg.MembershipAuthority = ""

	status, body := getHolds(t, s, "/holds", "ADMINS")
	if status != http.StatusOK || strings.Contains(body, ">Release hold</a>") || !strings.Contains(body, "Membership authority is unknown") {
		t.Fatalf("holds page status=%d body=%s", status, body)
	}

	form := url.Values{"csrf": {s.csrfNonce}, "entered_at": {stringInt(entered.Unix())}}
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/holds/peer-1/release", strings.NewReader(form.Encode()))
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	rec := httptest.NewRecorder()
	s.Routes().ServeHTTP(rec, req)
	if rec.Code != http.StatusConflict || !strings.Contains(rec.Body.String(), "authority is unknown") {
		t.Fatalf("status=%d body=%s", rec.Code, rec.Body.String())
	}
	if actions, err := s.Onboard.ListActions(context.Background(), onboardstore.ActionQueued); err != nil || len(actions) != 0 {
		t.Fatalf("omitted authority queued actions=%+v err=%v", actions, err)
	}
}

func TestReleaseHoldRefusesEveryUnsafeLiveShape(t *testing.T) {
	mesh := func(t *testing.T, s *Server, peers, groups []map[string]any) {
		t.Helper()
		nb := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			w.Header().Set("Content-Type", "application/json")
			switch r.URL.Path {
			case "/api/peers":
				_ = json.NewEncoder(w).Encode(peers)
			case "/api/groups":
				_ = json.NewEncoder(w).Encode(groups)
			default:
				w.WriteHeader(http.StatusNotFound)
			}
		}))
		t.Cleanup(nb.Close)
		s.Cfg.NotifyMgmtURL = nb.URL
	}
	for _, tc := range []struct {
		name   string
		want   string
		mutate func(*testing.T, *Server, time.Time) time.Time
	}{
		{"missing exact record", "no matching dormancy hold record", func(_ *testing.T, _ *Server, entered time.Time) time.Time { return entered.Add(time.Second) }},
		{"different goatnet", "different goatnet", func(t *testing.T, s *Server, entered time.Time) time.Time {
			body, err := os.ReadFile(s.Cfg.HoldRecordsPath)
			if err != nil {
				t.Fatal(err)
			}
			// #nosec G703 -- releaseTestServer set this to a test-owned TempDir path.
			if err := os.WriteFile(s.Cfg.HoldRecordsPath, []byte(strings.Replace(string(body), `"site":"zagros"`, `"site":"other"`, 1)), 0o600); err != nil {
				t.Fatal(err)
			}
			return entered
		}},
		{"case-mismatched goatnet", "different goatnet", func(t *testing.T, s *Server, entered time.Time) time.Time {
			body, err := os.ReadFile(s.Cfg.HoldRecordsPath)
			if err != nil {
				t.Fatal(err)
			}
			// Site slugs are canonical and the site runner filters them case-sensitively.
			// #nosec G703 -- releaseTestServer set this to a test-owned TempDir path.
			if err := os.WriteFile(s.Cfg.HoldRecordsPath, []byte(strings.Replace(string(body), `"site":"zagros"`, `"site":"ZAGROS"`, 1)), 0o600); err != nil {
				t.Fatal(err)
			}
			return entered
		}},
		{"duplicate hold records", "duplicate hold records", func(t *testing.T, s *Server, entered time.Time) time.Time {
			body, err := os.ReadFile(s.Cfg.HoldRecordsPath)
			if err != nil {
				t.Fatal(err)
			}
			// #nosec G703 -- releaseTestServer set this to a test-owned TempDir path.
			if err := os.WriteFile(s.Cfg.HoldRecordsPath, append(body, body...), 0o600); err != nil {
				t.Fatal(err)
			}
			return entered
		}},
		{"missing live peer", "live peer is unavailable", func(t *testing.T, s *Server, entered time.Time) time.Time {
			mesh(t, s, []map[string]any{}, []map[string]any{{"id": "hold", "name": "ZAGROS-PROBATION-DEVELOPER"}, {"id": "dest", "name": "ZAGROS-DEVELOPER"}})
			return entered
		}},
		{"ambiguous live peer", "ambiguous in live mesh state", func(t *testing.T, s *Server, entered time.Time) time.Time {
			peer := map[string]any{"id": "peer-1", "name": "peer-jane-zagros-laptop", "groups": []map[string]any{{"id": "hold", "name": "ZAGROS-PROBATION-DEVELOPER"}}}
			mesh(t, s, []map[string]any{peer, peer}, []map[string]any{{"id": "hold", "name": "ZAGROS-PROBATION-DEVELOPER"}, {"id": "dest", "name": "ZAGROS-DEVELOPER"}})
			return entered
		}},
		{"inconsistent live groups", "live state is inconsistent", func(t *testing.T, s *Server, entered time.Time) time.Time {
			mesh(t, s, []map[string]any{{"id": "peer-1", "name": "peer-jane-zagros-laptop", "groups": []map[string]any{{"id": "hold", "name": "ZAGROS-PROBATION-DEVELOPER"}, {"id": "extra", "name": "ZAGROS-OBSERVER"}}}}, []map[string]any{{"id": "hold", "name": "ZAGROS-PROBATION-DEVELOPER"}, {"id": "dest", "name": "ZAGROS-DEVELOPER"}, {"id": "extra", "name": "ZAGROS-OBSERVER"}})
			return entered
		}},
		{"missing destination", "destination group no longer exists", func(t *testing.T, s *Server, entered time.Time) time.Time {
			mesh(t, s, []map[string]any{{"id": "peer-1", "name": "peer-jane-zagros-laptop", "groups": []map[string]any{{"id": "hold", "name": "ZAGROS-PROBATION-DEVELOPER"}}}}, []map[string]any{{"id": "hold", "name": "ZAGROS-PROBATION-DEVELOPER"}})
			return entered
		}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			s, entered := releaseTestServer(t, "dormancy", []string{"ZAGROS-DEVELOPER"}, "ZAGROS-PROBATION-DEVELOPER")
			entered = tc.mutate(t, s, entered)
			form := url.Values{"csrf": {s.csrfNonce}, "entered_at": {stringInt(entered.Unix())}}
			req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/holds/peer-1/release", strings.NewReader(form.Encode()))
			req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
			rec := httptest.NewRecorder()
			s.Routes().ServeHTTP(rec, req)
			if rec.Code != http.StatusConflict || !strings.Contains(rec.Body.String(), tc.want) {
				t.Fatalf("status=%d body=%s, want %q", rec.Code, rec.Body.String(), tc.want)
			}
			actions, _ := s.Onboard.ListActions(context.Background(), onboardstore.ActionQueued)
			if len(actions) != 0 {
				t.Fatalf("unsafe release queued actions: %+v", actions)
			}
		})
	}
}

func TestReleaseHoldAllowsEmptyMultiAndPrivilegedDestinations(t *testing.T) {
	for _, prior := range [][]string{{}, {"ZAGROS-DEVELOPER", "ZAGROS-P2P"}, {"IAM-ADMIN"}} {
		s, entered := releaseTestServer(t, "dormancy", prior, "ZAGROS-PROBATION-DEVELOPER")
		req := httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/holds/peer-1/release?entered_at="+stringInt(entered.Unix()), nil)
		preview, err := s.validateReleaseHold(req, "peer-1", entered)
		if err != nil || !equalGroupSet(preview.Record.PriorGroups, prior) {
			t.Fatalf("prior=%v preview=%+v err=%v", prior, preview, err)
		}
	}
}

func TestReleaseResultKeepsVerifiedOutcomeWhenTerminalOperatorAuditFails(t *testing.T) {
	s, entered := releaseTestServer(t, "dormancy", []string{"ZAGROS-DEVELOPER"}, "ZAGROS-PROBATION-DEVELOPER")
	params, _ := json.Marshal(releaseHoldParams{
		Site: "zagros", PeerID: "peer-1", HoldEnteredAt: entered.Format(time.RFC3339),
		HoldGroup: "ZAGROS-PROBATION-DEVELOPER", PriorGroups: []string{"ZAGROS-DEVELOPER"}, RequestedByHandle: "admin-handle",
	})
	a, err := s.Onboard.EnqueueHoldAction(context.Background(), "zagros", "peer-1", entered, "admin-handle", "dev@bypass.local", string(params))
	if err != nil {
		t.Fatal(err)
	}
	if err := s.Onboard.ClaimAction(context.Background(), a.ID, "onboard-runner"); err != nil {
		t.Fatal(err)
	}
	claimed, err := s.Onboard.GetAction(context.Background(), a.ID)
	if err != nil {
		t.Fatal(err)
	}
	body := `{"status":"done","claim_token":"` + claimed.ClaimToken + `","release":{"outcome":"released","audit_outcome":"failed","audit_error":"audit bus unavailable"}}`
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/api/onboard/actions/"+a.ID+"/result", strings.NewReader(body))
	req.Header.Set("Authorization", "Bearer runner-key")
	req.Header.Set("Content-Type", "application/json")
	rec := httptest.NewRecorder()
	s.Routes().ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("status=%d body=%s, want stored release with visible audit failure", rec.Code, rec.Body.String())
	}
	stored, err := s.Onboard.GetAction(context.Background(), a.ID)
	if err != nil || stored.Status != onboardstore.ActionDone || !strings.Contains(stored.ResultJSON, "audit bus unavailable") {
		t.Fatalf("action=%+v err=%v, want released outcome with audit failure retained", stored, err)
	}
}

func stringInt(v int64) string { return strconv.FormatInt(v, 10) }
