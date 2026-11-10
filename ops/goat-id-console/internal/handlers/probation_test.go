package handlers

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/meshstate"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

func probationServer(t *testing.T, lane string, groups ...string) (*Server, string) {
	t.Helper()
	s := newOnboardTestServer(t)
	s.Cfg.NotifyGoatnet = "zagros"
	dir := t.TempDir()
	s.Cfg.HoldRecordsPath = filepath.Join(dir, "hold-records.jsonl")
	entered := time.Now().UTC().Add(-48 * time.Hour).Truncate(time.Second)
	holdGroup := "ZAGROS-PROBATION-DEVELOPER"
	if lane == meshstate.HoldKindSecurity {
		holdGroup = "ZAGROS-QUARANTINED"
	}
	record := fmt.Sprintf(`{"v":1,"lane":%q,"site":"zagros","peer_id":"peer-1","peer_name":"peer-jane-zagros-laptop","handle":"jane","entered_at":%q,"hold_group":%q,"prior_groups":["ZAGROS-DEVELOPER"],"actor":"sweep","reason":"internal detail"}`+"\n",
		lane, entered.Format(time.RFC3339), holdGroup)
	if err := os.WriteFile(s.Cfg.HoldRecordsPath, []byte(record), 0o600); err != nil {
		t.Fatal(err)
	}
	peerGroups := []meshstate.Group{{ID: "all", Name: "All"}}
	for _, group := range groups {
		peerGroups = append(peerGroups, meshstate.Group{ID: "g-" + group, Name: group})
	}
	s.probationPeerSource = func(context.Context) ([]meshstate.Peer, error) {
		return []meshstate.Peer{{ID: "peer-1", Name: "peer-jane-zagros-laptop", IP: "100.127.0.45", Connected: true, LastSeen: time.Now().UTC().Add(-30 * time.Second), Groups: peerGroups}}, nil
	}
	return s, record
}

func probationRecord(t *testing.T, raw string) meshstate.HoldRecord {
	t.Helper()
	var record meshstate.HoldRecord
	if err := json.Unmarshal([]byte(raw), &record); err != nil {
		t.Fatalf("decode hold record: %v", err)
	}
	return record
}

func probationRequest(method, target, body string) *http.Request {
	req := httptest.NewRequestWithContext(context.Background(), method, target, strings.NewReader(body))
	req.Header.Set("X-Forwarded-For", "100.127.0.45")
	req.Header.Set("X-Forwarded-Method", method)
	req.Header.Set("Accept", "text/html,application/xhtml+xml")
	if body != "" {
		req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	}
	return req
}

func ageProbationPeerSnapshot(t *testing.T, s *Server) {
	t.Helper()
	s.probationMu.Lock()
	defer s.probationMu.Unlock()
	s.probationSnapshot.refreshedAt = time.Now().Add(-probationPeerSnapshotMaxAge - time.Second)
	s.probationRefreshAfter = time.Time{}
}

func TestProbationHoldAge(t *testing.T) {
	now := time.Date(2026, time.October, 20, 12, 0, 0, 0, time.UTC)
	for _, tc := range []struct {
		name    string
		entered time.Time
		want    string
	}{
		{name: "zero", entered: now, want: "today"},
		{name: "under one day", entered: now.Add(-23*time.Hour - 59*time.Minute), want: "today"},
		{name: "exactly one day", entered: now.Add(-24 * time.Hour), want: "1 day ago"},
		{name: "forty seven hours", entered: now.Add(-47 * time.Hour), want: "1 day ago"},
		{name: "fractional forty nine days", entered: now.Add(-49*24*time.Hour - 21*time.Hour - 36*time.Minute), want: "49 days ago"},
		{name: "future", entered: now.Add(time.Hour), want: "today"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			if got := probationHoldAge(tc.entered, now); got != tc.want {
				t.Fatalf("probationHoldAge()=%q, want %q", got, tc.want)
			}
		})
	}
}

func TestProbationAuthShowsOnlyCurrentDormancyHold(t *testing.T) {
	t.Run("dormancy", func(t *testing.T) {
		s, rawRecord := probationServer(t, meshstate.HoldKindDormancy, "ZAGROS-PROBATION-DEVELOPER")
		record := probationRecord(t, rawRecord)
		rec := httptest.NewRecorder()
		s.Routes().ServeHTTP(rec, probationRequest(http.MethodGet, "/internal/probation/auth", ""))
		if rec.Code != http.StatusLocked {
			t.Fatalf("status=%d body=%s", rec.Code, rec.Body.String())
		}
		body := rec.Body.String()
		for _, want := range []string{"Your access is paused", "Paused since " + record.EnteredAt.UTC().Format("2 January 2006"), "2 days ago", "Request reactivation"} {
			if !strings.Contains(body, want) {
				t.Errorf("missing %q", want)
			}
		}
		for _, stale := range []string{"0.0", "2.0 days", "days inactive"} {
			if strings.Contains(body, stale) {
				t.Errorf("explainer contains stale live-last-seen text %q", stale)
			}
		}
		for _, secret := range []string{"ZAGROS-PROBATION-DEVELOPER", "internal detail", "peer-1"} {
			if strings.Contains(body, secret) {
				t.Errorf("explainer leaked %q", secret)
			}
		}
	})

	for name, tc := range map[string]struct {
		lane   string
		groups []string
	}{
		"security":     {meshstate.HoldKindSecurity, []string{"ZAGROS-QUARANTINED"}},
		"inconsistent": {meshstate.HoldKindDormancy, []string{"ZAGROS-PROBATION-DEVELOPER", "ZAGROS-DEVELOPER"}},
		"stale":        {meshstate.HoldKindDormancy, []string{"ZAGROS-DEVELOPER"}},
	} {
		t.Run(name, func(t *testing.T) {
			s, _ := probationServer(t, tc.lane, tc.groups...)
			rec := httptest.NewRecorder()
			s.Routes().ServeHTTP(rec, probationRequest(http.MethodGet, "/internal/probation/auth", ""))
			if rec.Code != http.StatusForbidden || strings.TrimSpace(rec.Body.String()) != "access denied" {
				t.Fatalf("status=%d body=%q, want generic denial", rec.Code, rec.Body.String())
			}
		})
	}
}

func TestProbationExplainerDurationComesFromHoldRecord(t *testing.T) {
	s, _ := probationServer(t, meshstate.HoldKindDormancy, "ZAGROS-PROBATION-DEVELOPER")
	groups := []meshstate.Group{{ID: "all", Name: "All"}, {ID: "g", Name: "ZAGROS-PROBATION-DEVELOPER"}}
	renderDetail := func(lastSeen time.Time) string {
		t.Helper()
		s.probationPeerSource = func(context.Context) ([]meshstate.Peer, error) {
			return []meshstate.Peer{{ID: "peer-1", Name: "peer-jane-zagros-laptop", IP: "100.127.0.45", Connected: true, LastSeen: lastSeen, Groups: groups}}, nil
		}
		s.probationMu.Lock()
		s.probationSnapshot = nil
		s.probationRefreshAfter = time.Time{}
		s.probationRetryAfter = time.Time{}
		s.probationRefreshErr = nil
		s.probationMu.Unlock()
		rec := httptest.NewRecorder()
		s.Routes().ServeHTTP(rec, probationRequest(http.MethodGet, "/internal/probation/auth", ""))
		if rec.Code != http.StatusLocked {
			t.Fatalf("status=%d body=%s", rec.Code, rec.Body.String())
		}
		return regexp.MustCompile(`<strong>[^<]*</strong> \([^<]*\)`).FindString(rec.Body.String())
	}

	now := time.Now().UTC()
	current := renderDetail(now)
	stale := renderDetail(now.Add(-40 * 24 * time.Hour))
	if current == "" || current != stale {
		t.Fatalf("hold detail changed with live last-seen: current=%q stale=%q", current, stale)
	}
}

func TestProbationAuthRefusesSpoofableOrHeldNonDocumentInputs(t *testing.T) {
	s, _ := probationServer(t, meshstate.HoldKindDormancy, "ZAGROS-PROBATION-DEVELOPER")
	for name, mutate := range map[string]func(*http.Request){
		"forwarded chain": func(r *http.Request) { r.Header.Set("X-Forwarded-For", "203.0.113.5, 100.127.0.45") },
		"missing":         func(r *http.Request) { r.Header.Del("X-Forwarded-For") },
		"asset":           func(r *http.Request) { r.Header.Set("Accept", "text/css,*/*") },
		"JSON API": func(r *http.Request) {
			r.Header.Set("X-Forwarded-Method", http.MethodPost)
			r.Header.Set("Accept", "application/json")
		},
		"forwarded POST document": func(r *http.Request) {
			r.Header.Set("X-Forwarded-Method", http.MethodPost)
		},
		"missing forwarded method": func(r *http.Request) { r.Header.Del("X-Forwarded-Method") },
	} {
		t.Run(name, func(t *testing.T) {
			req := probationRequest(http.MethodGet, "/internal/probation/auth", "")
			mutate(req)
			rec := httptest.NewRecorder()
			s.Routes().ServeHTTP(rec, req)
			if rec.Code != http.StatusForbidden {
				t.Fatalf("status=%d body=%q", rec.Code, rec.Body.String())
			}
		})
	}
	req := probationRequest(http.MethodGet, "/internal/probation/auth", "")
	req.Header.Set("X-Forwarded-For", "172.18.0.4")
	req.Header.Set("Accept", "application/json")
	rec := httptest.NewRecorder()
	s.Routes().ServeHTTP(rec, req)
	if rec.Code != http.StatusNoContent {
		t.Fatalf("off-mesh status=%d body=%q", rec.Code, rec.Body.String())
	}
}

func TestProbationAuthPassesNonMeshSourceBeforeHoldConfiguration(t *testing.T) {
	s := newOnboardTestServer(t)
	req := probationRequest(http.MethodGet, "/internal/probation/auth", "")
	req.Header.Set("X-Forwarded-For", "198.51.100.7")
	rec := httptest.NewRecorder()
	s.Routes().ServeHTTP(rec, req)
	if rec.Code != http.StatusNoContent {
		t.Fatalf("status=%d body=%q, want non-mesh pass", rec.Code, rec.Body.String())
	}
}

func TestReconcileReactivationRequests(t *testing.T) {
	seed := func(t *testing.T, s *Server, entered time.Time) onboardstore.ReactivationRequest {
		t.Helper()
		req, _, err := s.Onboard.RequestReactivation(context.Background(), "peer-1", "peer-jane-zagros-laptop", "jane", entered)
		if err != nil {
			t.Fatal(err)
		}
		return req
	}
	request := func() *http.Request {
		return httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/reactivation-requests", nil)
	}
	markReleasing := func(t *testing.T, s *Server) {
		t.Helper()
		body, err := os.ReadFile(s.Cfg.HoldRecordsPath)
		if err != nil {
			t.Fatal(err)
		}
		body = []byte(strings.Replace(string(body), `"prior_groups":`, `"phase":"releasing","prior_groups":`, 1))
		// #nosec G703 -- releaseTestServer set this to a test-owned TempDir path.
		if err := os.WriteFile(s.Cfg.HoldRecordsPath, body, 0o600); err != nil {
			t.Fatal(err)
		}
	}
	hasReconcileAudit := func(s *Server) bool {
		for _, event := range s.Audit.Recent(50) {
			if event.Action == "reconcile_reactivation_request" {
				return true
			}
		}
		return false
	}

	t.Run("ended hold closes as system and audits", func(t *testing.T) {
		s, entered := releaseTestServer(t, "dormancy", []string{"ZAGROS-DEVELOPER"}, "ZAGROS-DEVELOPER")
		req := seed(t, s, entered)
		if err := os.WriteFile(s.Cfg.HoldRecordsPath, nil, 0o600); err != nil {
			t.Fatal(err)
		}
		kept, err := s.reconcileReactivationRequests(request(), []onboardstore.ReactivationRequest{req})
		if err != nil || len(kept) != 0 {
			t.Fatalf("kept=%+v err=%v", kept, err)
		}
		got, err := s.Onboard.GetReactivationRequest(context.Background(), req.ID)
		if err != nil || got.Status != onboardstore.ReactivationClosed || got.DecidedBy != "system (reconciled: hold ended)" {
			t.Fatalf("request=%+v err=%v", got, err)
		}
		foundAudit := false
		for _, event := range s.Audit.Recent(50) {
			if event.Action == "reconcile_reactivation_request" && event.Actor.Email == "system" {
				foundAudit = true
			}
		}
		if !foundAudit {
			t.Fatal("reconciled close did not emit a system audit event")
		}
	})

	t.Run("releasing hold with a missing prior group stays open", func(t *testing.T) {
		s, entered := releaseTestServer(t, "dormancy", []string{"ZAGROS-DEVELOPER", "ZAGROS-P2P"}, "ZAGROS-DEVELOPER")
		markReleasing(t, s)
		req := seed(t, s, entered)
		kept, err := s.reconcileReactivationRequests(request(), []onboardstore.ReactivationRequest{req})
		if err != nil || len(kept) != 1 {
			t.Fatalf("kept=%+v err=%v", kept, err)
		}
		got, err := s.Onboard.GetReactivationRequest(context.Background(), req.ID)
		if err != nil || got.Status != onboardstore.ReactivationOpen {
			t.Fatalf("request=%+v err=%v", got, err)
		}
		if hasReconcileAudit(s) {
			t.Fatal("open request emitted a reconciliation audit event")
		}
	})

	t.Run("releasing hold with every prior group restored closes", func(t *testing.T) {
		s, entered := releaseTestServer(t, "dormancy", []string{"ZAGROS-DEVELOPER", "ZAGROS-P2P"}, "zagros-developer", "ZAGROS-P2P")
		markReleasing(t, s)
		req := seed(t, s, entered)
		kept, err := s.reconcileReactivationRequests(request(), []onboardstore.ReactivationRequest{req})
		if err != nil || len(kept) != 0 {
			t.Fatalf("kept=%+v err=%v", kept, err)
		}
		got, err := s.Onboard.GetReactivationRequest(context.Background(), req.ID)
		if err != nil || got.Status != onboardstore.ReactivationClosed || got.DecidedBy != "system (reconciled: hold ended)" {
			t.Fatalf("request=%+v err=%v", got, err)
		}
		if !hasReconcileAudit(s) {
			t.Fatal("reconciled close did not emit an audit event")
		}
	})

	t.Run("releasing hold group still present stays open", func(t *testing.T) {
		s, entered := releaseTestServer(t, "dormancy", []string{"ZAGROS-DEVELOPER"}, "ZAGROS-DEVELOPER", "ZAGROS-PROBATION-DEVELOPER")
		markReleasing(t, s)
		req := seed(t, s, entered)
		kept, err := s.reconcileReactivationRequests(request(), []onboardstore.ReactivationRequest{req})
		if err != nil || len(kept) != 1 {
			t.Fatalf("kept=%+v err=%v", kept, err)
		}
		got, err := s.Onboard.GetReactivationRequest(context.Background(), req.ID)
		if err != nil || got.Status != onboardstore.ReactivationOpen {
			t.Fatalf("request=%+v err=%v", got, err)
		}
	})

	t.Run("stale record with a missing prior group closes", func(t *testing.T) {
		s, entered := releaseTestServer(t, "dormancy", []string{"ZAGROS-DEVELOPER", "ZAGROS-P2P"}, "ZAGROS-DEVELOPER")
		req := seed(t, s, entered)
		kept, err := s.reconcileReactivationRequests(request(), []onboardstore.ReactivationRequest{req})
		if err != nil || len(kept) != 0 {
			t.Fatalf("kept=%+v err=%v", kept, err)
		}
		got, err := s.Onboard.GetReactivationRequest(context.Background(), req.ID)
		if err != nil || got.Status != onboardstore.ReactivationClosed || got.DecidedBy != "system (reconciled: hold ended)" {
			t.Fatalf("request=%+v err=%v", got, err)
		}
		if !hasReconcileAudit(s) {
			t.Fatal("reconciled close did not emit an audit event")
		}
	})

	t.Run("existing hold stays open", func(t *testing.T) {
		s, entered := releaseTestServer(t, "dormancy", []string{"ZAGROS-DEVELOPER"}, "ZAGROS-PROBATION-DEVELOPER")
		req := seed(t, s, entered)
		kept, err := s.reconcileReactivationRequests(request(), []onboardstore.ReactivationRequest{req})
		if err != nil || len(kept) != 1 {
			t.Fatalf("kept=%+v err=%v", kept, err)
		}
	})

	t.Run("case-mismatched other goatnet stays open", func(t *testing.T) {
		s, entered := releaseTestServer(t, "dormancy", []string{"ZAGROS-DEVELOPER"}, "ZAGROS-DEVELOPER")
		req := seed(t, s, entered)
		body, err := os.ReadFile(s.Cfg.HoldRecordsPath)
		if err != nil {
			t.Fatal(err)
		}
		// #nosec G703 -- releaseTestServer set this to a test-owned TempDir path.
		if err := os.WriteFile(s.Cfg.HoldRecordsPath, []byte(strings.Replace(string(body), `"site":"zagros"`, `"site":"ZAGROS"`, 1)), 0o600); err != nil {
			t.Fatal(err)
		}
		kept, err := s.reconcileReactivationRequests(request(), []onboardstore.ReactivationRequest{req})
		if err != nil || len(kept) != 1 {
			t.Fatalf("kept=%+v err=%v", kept, err)
		}
	})

	t.Run("close error is surfaced and row retained", func(t *testing.T) {
		s, entered := releaseTestServer(t, "dormancy", []string{"ZAGROS-DEVELOPER"}, "ZAGROS-DEVELOPER")
		req := seed(t, s, entered)
		if err := os.WriteFile(s.Cfg.HoldRecordsPath, nil, 0o600); err != nil {
			t.Fatal(err)
		}
		if err := s.Onboard.Close(); err != nil {
			t.Fatal(err)
		}
		kept, err := s.reconcileReactivationRequests(request(), []onboardstore.ReactivationRequest{req})
		if err == nil || len(kept) != 1 {
			t.Fatalf("kept=%+v err=%v, want retained row and surfaced error", kept, err)
		}
	})
}

func TestReactivationQueueNoticeDoesNotHideOpenRows(t *testing.T) {
	body := renderPage(t, "reactivation_requests", &pageVars{ReactivationRequests: &reactivationRequestsPage{
		Error: "Some ended holds could not be reconciled; their requests remain open.",
		Rows: []reactivationRequestRow{{ReactivationRequest: onboardstore.ReactivationRequest{
			ID: "react-1", PeerID: "peer-1", PeerName: "peer-one", Handle: "one",
		}}},
	}})
	for _, want := range []string{"Some ended holds could not be reconciled", "peer-one", "Decline request"} {
		if !strings.Contains(body, want) {
			t.Fatalf("queue page missing %q while rendering reconciliation notice", want)
		}
	}
}

func TestProbationPetitionBypassRequiresForwardedPost(t *testing.T) {
	s, _ := probationServer(t, meshstate.HoldKindDormancy, "ZAGROS-PROBATION-DEVELOPER")
	req := probationRequest(http.MethodGet, "/internal/probation/auth", "")
	req.Header.Set("X-Forwarded-Uri", probationPetitionPath)
	rec := httptest.NewRecorder()
	s.Routes().ServeHTTP(rec, req)
	if rec.Code != http.StatusLocked || !strings.Contains(rec.Body.String(), "Your access is paused") {
		t.Fatalf("status=%d body=%q, want held-peer explainer", rec.Code, rec.Body.String())
	}
}

func TestProbationPeerSnapshotRejectsDuplicateAddress(t *testing.T) {
	s, _ := probationServer(t, meshstate.HoldKindDormancy, "ZAGROS-PROBATION-DEVELOPER")
	s.probationPeerSource = func(context.Context) ([]meshstate.Peer, error) {
		return []meshstate.Peer{
			{ID: "peer-1", IP: "100.127.0.45"},
			{ID: "peer-2", IP: "100.127.0.45"},
		}, nil
	}
	rec := httptest.NewRecorder()
	s.Routes().ServeHTTP(rec, probationRequest(http.MethodGet, "/internal/probation/auth", ""))
	if rec.Code != http.StatusForbidden || strings.TrimSpace(rec.Body.String()) != "access denied" {
		t.Fatalf("status=%d body=%q, want generic denial", rec.Code, rec.Body.String())
	}
}

func TestProbationPeerSnapshotServesLastKnownGoodDuringRefreshFailure(t *testing.T) {
	s, _ := probationServer(t, meshstate.HoldKindDormancy, "ZAGROS-PROBATION-DEVELOPER")
	req := probationRequest(http.MethodGet, "/internal/probation/auth", "")
	if _, decision := s.classifyProbation(req); decision != probationExplain {
		t.Fatalf("initial decision=%v, want explain", decision)
	}

	started := make(chan struct{})
	release := make(chan struct{})
	s.probationPeerSource = func(context.Context) ([]meshstate.Peer, error) {
		close(started)
		<-release
		return nil, errors.New("management API unavailable")
	}
	s.probationMu.Lock()
	s.probationRefreshAfter = time.Time{}
	s.probationMu.Unlock()

	begin := time.Now()
	if _, decision := s.classifyProbation(req); decision != probationExplain {
		t.Fatalf("decision during refresh=%v, want last-known-good explain", decision)
	}
	if elapsed := time.Since(begin); elapsed > 500*time.Millisecond {
		t.Fatalf("stale snapshot read blocked for %s", elapsed)
	}
	select {
	case <-started:
	case <-time.After(time.Second):
		t.Fatal("background refresh did not start")
	}

	s.probationMu.Lock()
	done := s.probationRefreshDone
	s.probationMu.Unlock()
	close(release)
	select {
	case <-done:
	case <-time.After(time.Second):
		t.Fatal("background refresh did not finish")
	}
	if _, decision := s.classifyProbation(req); decision != probationExplain {
		t.Fatalf("decision after failed refresh=%v, want last-known-good explain", decision)
	}
}

func TestProbationPeerSnapshotKeepsOrdinaryPeerAvailableOnRefreshFailure(t *testing.T) {
	s, _ := probationServer(t, meshstate.HoldKindDormancy)
	if err := os.WriteFile(s.Cfg.HoldRecordsPath, nil, 0o600); err != nil {
		t.Fatal(err)
	}
	req := probationRequest(http.MethodGet, "/internal/probation/auth", "")
	if _, decision := s.classifyProbation(req); decision != probationPass {
		t.Fatalf("initial decision=%v, want pass", decision)
	}

	s.probationPeerSource = func(context.Context) ([]meshstate.Peer, error) {
		return nil, errors.New("management API unavailable")
	}
	s.probationMu.Lock()
	s.probationRefreshAfter = time.Time{}
	s.probationMu.Unlock()
	if _, decision := s.classifyProbation(req); decision != probationPass {
		t.Fatalf("decision during failed refresh=%v, want last-known-good pass", decision)
	}
	s.probationMu.Lock()
	done := s.probationRefreshDone
	s.probationMu.Unlock()
	select {
	case <-done:
	case <-time.After(time.Second):
		t.Fatal("background refresh did not finish")
	}
	if _, decision := s.classifyProbation(req); decision != probationPass {
		t.Fatalf("decision after failed refresh=%v, want last-known-good pass", decision)
	}
}

func TestProbationPeerSnapshotExpiryDeniesReassignedAddressAfterRefreshFailure(t *testing.T) {
	s, _ := probationServer(t, meshstate.HoldKindDormancy, "ZAGROS-PROBATION-DEVELOPER")
	assignedPeer := meshstate.Peer{
		ID:       "peer-1",
		Name:     "peer-jane-zagros-laptop",
		IP:       "100.127.0.45",
		LastSeen: time.Now().UTC(),
		Groups:   []meshstate.Group{{ID: "hold", Name: "ZAGROS-PROBATION-DEVELOPER"}},
	}
	refreshFails := false
	s.probationPeerSource = func(context.Context) ([]meshstate.Peer, error) {
		if refreshFails {
			return nil, errors.New("management API unavailable after address reassignment")
		}
		return []meshstate.Peer{assignedPeer}, nil
	}

	req := probationRequest(http.MethodGet, "/internal/probation/auth", "")
	hold, decision := s.classifyProbation(req)
	if decision != probationExplain {
		t.Fatalf("initial decision=%v, want explain", decision)
	}
	oldToken := s.probationToken(hold)

	// NetBird has reassigned the same address, but the attempted refresh cannot
	// observe the new owner. The previous binding remains usable only inside the
	// bounded outage window.
	assignedPeer = meshstate.Peer{
		ID:       "peer-2",
		Name:     "peer-new-owner",
		IP:       "100.127.0.45",
		LastSeen: time.Now().UTC(),
		Groups:   []meshstate.Group{{ID: "all", Name: "All"}},
	}
	refreshFails = true
	s.probationMu.Lock()
	s.probationRefreshAfter = time.Time{}
	s.probationMu.Unlock()
	if _, decision := s.classifyProbation(req); decision != probationExplain {
		t.Fatalf("decision inside outage window=%v, want explain", decision)
	}
	s.probationMu.Lock()
	done := s.probationRefreshDone
	s.probationMu.Unlock()
	select {
	case <-done:
	case <-time.After(time.Second):
		t.Fatal("background refresh did not finish")
	}

	s.probationMu.Lock()
	s.probationSnapshot.refreshedAt = time.Now().Add(-probationPeerSnapshotMaxAge)
	s.probationMu.Unlock()
	auth := httptest.NewRecorder()
	s.Routes().ServeHTTP(auth, req)
	if auth.Code != http.StatusForbidden || strings.TrimSpace(auth.Body.String()) != "access denied" {
		t.Fatalf("expired auth status=%d body=%q, want generic denial", auth.Code, auth.Body.String())
	}

	form := url.Values{"token": []string{oldToken}}.Encode()
	petition := httptest.NewRecorder()
	s.Routes().ServeHTTP(petition, probationRequest(http.MethodPost, probationPetitionPath, form))
	if petition.Code != http.StatusForbidden || strings.TrimSpace(petition.Body.String()) != "request unavailable" {
		t.Fatalf("expired petition status=%d body=%q, want unavailable", petition.Code, petition.Body.String())
	}
	requests, err := s.Onboard.ListOpenReactivationRequests(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(requests) != 0 {
		t.Fatalf("expired binding created %d reactivation requests, want none", len(requests))
	}

	// A successful refresh establishes the reassigned address's new identity.
	refreshFails = false
	s.probationMu.Lock()
	s.probationRefreshAfter = time.Time{}
	s.probationRetryAfter = time.Time{}
	s.probationMu.Unlock()
	if _, decision := s.classifyProbation(req); decision != probationPass {
		t.Fatalf("decision after synchronous recovery=%v, want pass for new owner", decision)
	}
}

func TestProbationPeerSnapshotIdleGapKeepsOrdinaryPeerAvailable(t *testing.T) {
	s, _ := probationServer(t, meshstate.HoldKindDormancy)
	if err := os.WriteFile(s.Cfg.HoldRecordsPath, nil, 0o600); err != nil {
		t.Fatal(err)
	}
	req := probationRequest(http.MethodGet, "/internal/probation/auth", "")
	if _, decision := s.classifyProbation(req); decision != probationPass {
		t.Fatalf("warm decision=%v, want pass", decision)
	}
	ageProbationPeerSnapshot(t, s)
	if _, decision := s.classifyProbation(req); decision != probationPass {
		t.Fatalf("first decision after idle gap=%v, want pass", decision)
	}
}

func TestProbationPeerSnapshotExpiredLastKnownGoodClassification(t *testing.T) {
	for _, tc := range []struct {
		name             string
		groups           []string
		emptyFile        bool
		writeRecordAfter bool
		want             probationDisposition
	}{
		{name: "ordinary peer", emptyFile: true, want: probationPass},
		{name: "hold group without record", groups: []string{"ZAGROS-PROBATION-DEVELOPER"}, emptyFile: true, want: probationDeny},
		{name: "current dormancy hold", groups: []string{"ZAGROS-PROBATION-DEVELOPER"}, emptyFile: true, writeRecordAfter: true, want: probationDeny},
	} {
		t.Run(tc.name, func(t *testing.T) {
			s, record := probationServer(t, meshstate.HoldKindDormancy, tc.groups...)
			if tc.emptyFile {
				if err := os.WriteFile(s.Cfg.HoldRecordsPath, nil, 0o600); err != nil {
					t.Fatal(err)
				}
			}
			req := probationRequest(http.MethodGet, "/internal/probation/auth", "")
			s.classifyProbation(req)
			if tc.writeRecordAfter {
				if err := os.WriteFile(s.Cfg.HoldRecordsPath, []byte(record), 0o600); err != nil {
					t.Fatal(err)
				}
			}
			var calls atomic.Int32
			s.probationPeerSource = func(context.Context) ([]meshstate.Peer, error) {
				calls.Add(1)
				return nil, errors.New("management API unavailable")
			}
			ageProbationPeerSnapshot(t, s)
			if _, decision := s.classifyProbation(req); decision != tc.want {
				t.Fatalf("decision=%v, want %v", decision, tc.want)
			}
			if tc.name == "current dormancy hold" && calls.Load() != 1 {
				t.Fatalf("management calls=%d, want 1 freshness attempt", calls.Load())
			}
		})
	}
}

func TestProbationPeerSnapshotFreshensExplainerAndPetition(t *testing.T) {
	s, _ := probationServer(t, meshstate.HoldKindDormancy, "ZAGROS-PROBATION-DEVELOPER")
	req := probationRequest(http.MethodGet, "/internal/probation/auth", "")
	if _, decision := s.classifyProbation(req); decision != probationExplain {
		t.Fatalf("warm decision=%v, want explain", decision)
	}
	var calls atomic.Int32
	originalSource := s.probationPeerSource
	s.probationPeerSource = func(ctx context.Context) ([]meshstate.Peer, error) {
		calls.Add(1)
		return originalSource(ctx)
	}
	ageProbationPeerSnapshot(t, s)
	auth := httptest.NewRecorder()
	s.Routes().ServeHTTP(auth, req)
	if auth.Code != http.StatusLocked {
		t.Fatalf("auth status=%d body=%q, want explainer", auth.Code, auth.Body.String())
	}
	if calls.Load() != 1 {
		t.Fatalf("management calls=%d, want 1", calls.Load())
	}
	match := probationTokenRE.FindStringSubmatch(auth.Body.String())
	if len(match) != 2 {
		t.Fatalf("no petition token in %s", auth.Body.String())
	}
	form := url.Values{"token": []string{match[1]}}.Encode()
	petition := httptest.NewRecorder()
	s.Routes().ServeHTTP(petition, probationRequest(http.MethodPost, probationPetitionPath, form))
	if petition.Code != http.StatusOK {
		t.Fatalf("petition status=%d body=%q", petition.Code, petition.Body.String())
	}
	rows, err := s.Onboard.ListOpenReactivationRequests(context.Background())
	if err != nil || len(rows) != 1 {
		t.Fatalf("requests=%+v err=%v, want exactly one", rows, err)
	}
}

func TestProbationPeerSnapshotExpiredExplainerFailureBacksOff(t *testing.T) {
	s, _ := probationServer(t, meshstate.HoldKindDormancy, "ZAGROS-PROBATION-DEVELOPER")
	warm := httptest.NewRecorder()
	s.Routes().ServeHTTP(warm, probationRequest(http.MethodGet, "/internal/probation/auth", ""))
	match := probationTokenRE.FindStringSubmatch(warm.Body.String())
	if len(match) != 2 {
		t.Fatalf("no petition token in %s", warm.Body.String())
	}
	var calls atomic.Int32
	s.probationPeerSource = func(context.Context) ([]meshstate.Peer, error) {
		calls.Add(1)
		return nil, errors.New("management API unavailable")
	}
	ageProbationPeerSnapshot(t, s)
	s.probationMu.Lock()
	s.probationRefreshAfter = time.Now().Add(time.Hour)
	s.probationMu.Unlock()
	auth := httptest.NewRecorder()
	s.Routes().ServeHTTP(auth, probationRequest(http.MethodGet, "/internal/probation/auth", ""))
	if auth.Code != http.StatusForbidden || strings.TrimSpace(auth.Body.String()) != "access denied" {
		t.Fatalf("auth status=%d body=%q, want generic denial", auth.Code, auth.Body.String())
	}
	form := url.Values{"token": []string{match[1]}}.Encode()
	petition := httptest.NewRecorder()
	s.Routes().ServeHTTP(petition, probationRequest(http.MethodPost, probationPetitionPath, form))
	if petition.Code != http.StatusForbidden || strings.TrimSpace(petition.Body.String()) != "request unavailable" {
		t.Fatalf("petition status=%d body=%q, want unavailable", petition.Code, petition.Body.String())
	}
	second := httptest.NewRecorder()
	s.Routes().ServeHTTP(second, probationRequest(http.MethodGet, "/internal/probation/auth", ""))
	if calls.Load() != 1 {
		t.Fatalf("management calls within retry back-off=%d, want 1", calls.Load())
	}
	rows, err := s.Onboard.ListOpenReactivationRequests(context.Background())
	if err != nil || len(rows) != 0 {
		t.Fatalf("requests=%+v err=%v, want none", rows, err)
	}
}

func TestProbationPeerSnapshotExpiredExplainerRefreshIsSingleFlight(t *testing.T) {
	s, _ := probationServer(t, meshstate.HoldKindDormancy, "ZAGROS-PROBATION-DEVELOPER")
	if _, decision := s.classifyProbation(probationRequest(http.MethodGet, "/internal/probation/auth", "")); decision != probationExplain {
		t.Fatalf("warm decision=%v, want explain", decision)
	}
	originalSource := s.probationPeerSource
	started := make(chan struct{})
	release := make(chan struct{})
	var calls atomic.Int32
	s.probationPeerSource = func(ctx context.Context) ([]meshstate.Peer, error) {
		if calls.Add(1) == 1 {
			close(started)
		}
		select {
		case <-release:
			return originalSource(ctx)
		case <-ctx.Done():
			return nil, ctx.Err()
		}
	}
	ageProbationPeerSnapshot(t, s)
	const requestCount = 8
	decisions := make(chan probationDisposition, requestCount)
	var wg sync.WaitGroup
	for i := 0; i < requestCount; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			_, decision := s.classifyProbation(probationRequest(http.MethodGet, "/internal/probation/auth", ""))
			decisions <- decision
		}()
	}
	finished := make(chan struct{})
	go func() {
		wg.Wait()
		close(finished)
	}()
	select {
	case <-started:
	case <-finished:
		t.Fatal("requests completed without refreshing the stale explainer binding")
	}
	close(release)
	<-finished
	close(decisions)
	for decision := range decisions {
		if decision != probationExplain {
			t.Errorf("decision=%v, want explain", decision)
		}
	}
	if calls.Load() != 1 {
		t.Fatalf("management calls=%d, want one single-flight refresh", calls.Load())
	}
}

func TestProbationPeerSnapshotAdmitsNewPeerAfterBackgroundRefresh(t *testing.T) {
	s, _ := probationServer(t, meshstate.HoldKindDormancy)
	if err := os.WriteFile(s.Cfg.HoldRecordsPath, nil, 0o600); err != nil {
		t.Fatal(err)
	}
	if _, decision := s.classifyProbation(probationRequest(http.MethodGet, "/internal/probation/auth", "")); decision != probationPass {
		t.Fatalf("initial decision=%v, want pass", decision)
	}

	s.probationPeerSource = func(context.Context) ([]meshstate.Peer, error) {
		return []meshstate.Peer{{ID: "peer-new", Name: "peer-new", IP: "100.127.0.60", LastSeen: time.Now().UTC()}}, nil
	}
	s.probationMu.Lock()
	s.probationRefreshAfter = time.Time{}
	s.probationMu.Unlock()
	req := probationRequest(http.MethodGet, "/internal/probation/auth", "")
	req.Header.Set("X-Forwarded-For", "100.127.0.60")
	if _, decision := s.classifyProbation(req); decision != probationDeny {
		t.Fatalf("decision from stale snapshot=%v, want fail-closed deny", decision)
	}
	s.probationMu.Lock()
	done := s.probationRefreshDone
	s.probationMu.Unlock()
	select {
	case <-done:
	case <-time.After(time.Second):
		t.Fatal("background refresh did not finish")
	}
	if _, decision := s.classifyProbation(req); decision != probationPass {
		t.Fatalf("decision after refresh=%v, want pass", decision)
	}
}

func TestProbationPeerSnapshotKeepsLastKnownGoodOnEmptyRefresh(t *testing.T) {
	s, _ := probationServer(t, meshstate.HoldKindDormancy)
	if err := os.WriteFile(s.Cfg.HoldRecordsPath, nil, 0o600); err != nil {
		t.Fatal(err)
	}
	req := probationRequest(http.MethodGet, "/internal/probation/auth", "")
	if _, decision := s.classifyProbation(req); decision != probationPass {
		t.Fatalf("initial decision=%v, want pass", decision)
	}

	s.probationPeerSource = func(context.Context) ([]meshstate.Peer, error) { return nil, nil }
	s.probationMu.Lock()
	s.probationRefreshAfter = time.Time{}
	s.probationMu.Unlock()
	if _, decision := s.classifyProbation(req); decision != probationPass {
		t.Fatalf("decision during empty refresh=%v, want last-known-good pass", decision)
	}
	s.probationMu.Lock()
	done := s.probationRefreshDone
	s.probationMu.Unlock()
	select {
	case <-done:
	case <-time.After(time.Second):
		t.Fatal("background refresh did not finish")
	}
	if _, decision := s.classifyProbation(req); decision != probationPass {
		t.Fatalf("decision after empty refresh=%v, want last-known-good pass", decision)
	}
}

func TestProbationPeerSnapshotColdStartEmptyDenies(t *testing.T) {
	s, _ := probationServer(t, meshstate.HoldKindDormancy)
	s.probationPeerSource = func(context.Context) ([]meshstate.Peer, error) { return nil, nil }
	if _, decision := s.classifyProbation(probationRequest(http.MethodGet, "/internal/probation/auth", "")); decision != probationDeny {
		t.Fatalf("decision=%v, want fail-closed deny", decision)
	}
}

func TestProbationAuthSecurityHoldDeniesNonDocumentRequest(t *testing.T) {
	s, _ := probationServer(t, meshstate.HoldKindSecurity, "ZAGROS-QUARANTINED")
	req := probationRequest(http.MethodGet, "/internal/probation/auth", "")
	req.Header.Set("Accept", "application/json")
	rec := httptest.NewRecorder()
	s.Routes().ServeHTTP(rec, req)
	if rec.Code != http.StatusForbidden || strings.TrimSpace(rec.Body.String()) != "access denied" {
		t.Fatalf("status=%d body=%q, want generic denial", rec.Code, rec.Body.String())
	}
}

func TestProbationAuthUnknownHoldDeniesButOrdinaryPeerPasses(t *testing.T) {
	for name, tc := range map[string]struct {
		groups []string
		want   int
	}{
		"hold group without record": {[]string{"ZAGROS-PROBATION-DEVELOPER"}, http.StatusForbidden},
		"ordinary peer":             {nil, http.StatusNoContent},
	} {
		t.Run(name, func(t *testing.T) {
			s, _ := probationServer(t, meshstate.HoldKindDormancy, tc.groups...)
			if err := os.WriteFile(s.Cfg.HoldRecordsPath, nil, 0o600); err != nil {
				t.Fatal(err)
			}
			rec := httptest.NewRecorder()
			s.Routes().ServeHTTP(rec, probationRequest(http.MethodGet, "/internal/probation/auth", ""))
			if rec.Code != tc.want {
				t.Fatalf("status=%d body=%q, want %d", rec.Code, rec.Body.String(), tc.want)
			}
		})
	}
}

func TestProbationAuthUnreadableHoldSourceFailsClosedForOrdinaryPeer(t *testing.T) {
	assertDenied := func(t *testing.T, s *Server) {
		t.Helper()
		rec := httptest.NewRecorder()
		s.Routes().ServeHTTP(rec, probationRequest(http.MethodGet, "/internal/probation/auth", ""))
		if rec.Code != http.StatusForbidden || strings.TrimSpace(rec.Body.String()) != "access denied" {
			t.Fatalf("status=%d body=%q, want generic denial", rec.Code, rec.Body.String())
		}
	}

	t.Run("missing", func(t *testing.T) {
		s, _ := probationServer(t, meshstate.HoldKindDormancy)
		if err := os.Remove(s.Cfg.HoldRecordsPath); err != nil {
			t.Fatal(err)
		}
		assertDenied(t, s)
	})

	t.Run("permission lost after cache warm", func(t *testing.T) {
		s, _ := probationServer(t, meshstate.HoldKindDormancy)
		if err := os.WriteFile(s.Cfg.HoldRecordsPath, nil, 0o600); err != nil {
			t.Fatal(err)
		}
		if _, decision := s.classifyProbation(probationRequest(http.MethodGet, "/internal/probation/auth", "")); decision != probationPass {
			t.Fatalf("initial decision=%v, want pass", decision)
		}
		if err := os.Chmod(s.Cfg.HoldRecordsPath, 0); err != nil {
			t.Fatal(err)
		}
		t.Cleanup(func() { _ = os.Chmod(s.Cfg.HoldRecordsPath, 0o600) })
		f, err := os.Open(s.Cfg.HoldRecordsPath)
		if err == nil {
			_ = f.Close()
			t.Skip("filesystem does not enforce the permission change for this user")
		}
		assertDenied(t, s)
	})
}

func TestProbationAuthDeniesHoldFromEarlierSitePrefix(t *testing.T) {
	for _, group := range []string{"ZAGROS-PROBATION-DEVELOPER", "ZAGROS-QUARANTINED"} {
		for _, fileState := range []string{"empty", "missing"} {
			t.Run(group+"/"+fileState, func(t *testing.T) {
				s, _ := probationServer(t, meshstate.HoldKindDormancy, group)
				s.Cfg.NotifyGoatnet = "zagros-dev"
				if fileState == "missing" {
					if err := os.Remove(s.Cfg.HoldRecordsPath); err != nil {
						t.Fatal(err)
					}
				} else if err := os.WriteFile(s.Cfg.HoldRecordsPath, nil, 0o600); err != nil {
					t.Fatal(err)
				}
				rec := httptest.NewRecorder()
				s.Routes().ServeHTTP(rec, probationRequest(http.MethodGet, "/internal/probation/auth", ""))
				if rec.Code != http.StatusForbidden || strings.TrimSpace(rec.Body.String()) != "access denied" {
					t.Fatalf("status=%d body=%q, want generic denial", rec.Code, rec.Body.String())
				}
			})
		}
	}
}

func TestProbationHoldSnapshotDetectsAppendAndAtomicReplacement(t *testing.T) {
	s, record := probationServer(t, meshstate.HoldKindDormancy)
	if err := os.WriteFile(s.Cfg.HoldRecordsPath, nil, 0o600); err != nil {
		t.Fatal(err)
	}
	req := probationRequest(http.MethodGet, "/internal/probation/auth", "")
	if _, decision := s.classifyProbation(req); decision != probationPass {
		t.Fatalf("empty file decision=%v, want pass", decision)
	}

	f, err := os.OpenFile(s.Cfg.HoldRecordsPath, os.O_APPEND|os.O_WRONLY, 0)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := f.WriteString(record); err != nil {
		_ = f.Close()
		t.Fatal(err)
	}
	if err := f.Close(); err != nil {
		t.Fatal(err)
	}
	if _, decision := s.classifyProbation(req); decision != probationDeny {
		t.Fatalf("appended record decision=%v, want deny for record/mesh mismatch", decision)
	}

	replacement := filepath.Join(filepath.Dir(s.Cfg.HoldRecordsPath), "replacement.jsonl")
	if err := os.WriteFile(replacement, nil, 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.Rename(replacement, s.Cfg.HoldRecordsPath); err != nil {
		t.Fatal(err)
	}
	if _, decision := s.classifyProbation(req); decision != probationPass {
		t.Fatalf("atomic replacement decision=%v, want pass", decision)
	}
}

func TestProbationReadyChecksRunningDependencies(t *testing.T) {
	t.Run("ready", func(t *testing.T) {
		s, _ := probationServer(t, meshstate.HoldKindDormancy)
		s.Cfg.NotifyMgmtURL = "https://management.example.test"
		s.Cfg.EnrollWatchToken = "runtime-token"
		rec := httptest.NewRecorder()
		s.Routes().ServeHTTP(rec, httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/internal/probation/ready", nil))
		if rec.Code != http.StatusNoContent {
			t.Fatalf("status=%d body=%q", rec.Code, rec.Body.String())
		}
	})

	t.Run("stale runtime token", func(t *testing.T) {
		s, _ := probationServer(t, meshstate.HoldKindDormancy)
		s.Cfg.NotifyMgmtURL = "https://management.example.test"
		s.Cfg.EnrollWatchToken = "stale-token"
		s.probationPeerSource = func(context.Context) ([]meshstate.Peer, error) {
			return nil, errors.New("management API rejected runtime token")
		}
		rec := httptest.NewRecorder()
		s.Routes().ServeHTTP(rec, httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/internal/probation/ready", nil))
		if rec.Code != http.StatusServiceUnavailable {
			t.Fatalf("status=%d body=%q", rec.Code, rec.Body.String())
		}
	})

	t.Run("missing runtime configuration", func(t *testing.T) {
		s, _ := probationServer(t, meshstate.HoldKindDormancy)
		s.Cfg.EnrollWatchToken = ""
		rec := httptest.NewRecorder()
		s.Routes().ServeHTTP(rec, httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/internal/probation/ready", nil))
		if rec.Code != http.StatusServiceUnavailable {
			t.Fatalf("status=%d body=%q", rec.Code, rec.Body.String())
		}
	})

	t.Run("empty peer list", func(t *testing.T) {
		s, _ := probationServer(t, meshstate.HoldKindDormancy)
		s.Cfg.NotifyMgmtURL = "https://management.example.test"
		s.Cfg.EnrollWatchToken = "runtime-token"
		s.probationPeerSource = func(context.Context) ([]meshstate.Peer, error) { return nil, nil }
		rec := httptest.NewRecorder()
		s.Routes().ServeHTTP(rec, httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/internal/probation/ready", nil))
		if rec.Code != http.StatusServiceUnavailable {
			t.Fatalf("status=%d body=%q", rec.Code, rec.Body.String())
		}
	})
}

var probationTokenRE = regexp.MustCompile(`name="token" value="([^"]+)"`)

func TestProbationPetitionIsIdempotentAndBoundToCurrentHold(t *testing.T) {
	s, _ := probationServer(t, meshstate.HoldKindDormancy, "ZAGROS-PROBATION-DEVELOPER")
	auth := httptest.NewRecorder()
	s.Routes().ServeHTTP(auth, probationRequest(http.MethodGet, "/internal/probation/auth", ""))
	m := probationTokenRE.FindStringSubmatch(auth.Body.String())
	if len(m) != 2 {
		t.Fatalf("no petition token in %s", auth.Body.String())
	}
	post := func(token string) *httptest.ResponseRecorder {
		rec := httptest.NewRecorder()
		form := url.Values{"token": []string{token}}.Encode()
		s.Routes().ServeHTTP(rec, probationRequest(http.MethodPost, probationPetitionPath, form))
		return rec
	}
	for i := 0; i < 2; i++ {
		if rec := post(m[1]); rec.Code != http.StatusOK || !strings.Contains(rec.Body.String(), "has been recorded") {
			t.Fatalf("post %d: status=%d body=%s", i, rec.Code, rec.Body.String())
		}
	}
	revisit := httptest.NewRecorder()
	s.Routes().ServeHTTP(revisit, probationRequest(http.MethodGet, "/internal/probation/auth", ""))
	if revisit.Code != http.StatusLocked || !strings.Contains(revisit.Body.String(), "has been recorded") || strings.Contains(revisit.Body.String(), "<form") {
		t.Fatalf("revisit status=%d body=%s, want recorded state without form", revisit.Code, revisit.Body.String())
	}
	rows, err := s.Onboard.ListOpenReactivationRequests(context.Background())
	if err != nil || len(rows) != 1 {
		t.Fatalf("requests=%+v err=%v, want exactly one", rows, err)
	}

	// The token for one hold cannot be replayed after a new hold is recorded.
	newEntered := time.Now().UTC().Add(-time.Hour).Truncate(time.Second)
	replacement := fmt.Sprintf(`{"v":1,"lane":"dormancy","site":"zagros","peer_id":"peer-1","peer_name":"peer-jane-zagros-laptop","handle":"jane","entered_at":%q,"hold_group":"ZAGROS-PROBATION-DEVELOPER","prior_groups":["ZAGROS-DEVELOPER"],"actor":"sweep"}`+"\n", newEntered.Format(time.RFC3339))
	if err := os.WriteFile(s.Cfg.HoldRecordsPath, []byte(replacement), 0o600); err != nil {
		t.Fatal(err)
	}
	if rec := post(m[1]); rec.Code != http.StatusForbidden {
		t.Fatalf("stale token status=%d body=%s", rec.Code, rec.Body.String())
	}

	queue := httptest.NewRecorder()
	s.Routes().ServeHTTP(queue, httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/reactivation-requests", nil))
	if queue.Code != http.StatusOK {
		t.Fatalf("queue status=%d body=%s", queue.Code, queue.Body.String())
	}
	for _, want := range []string{"jane", "peer-jane-zagros-laptop", "View mesh hold", "/holds?filter=peer-1"} {
		if !strings.Contains(queue.Body.String(), want) {
			t.Errorf("queue missing %q", want)
		}
	}
	if strings.Contains(strings.ToLower(queue.Body.String()), "<form") {
		t.Error("read-only queue rendered a form")
	}
}

func TestReactivationQueueNamesHoldRecordSourceAndLinksByPeerID(t *testing.T) {
	s := newOnboardTestServer(t)
	now := time.Now().UTC().Truncate(time.Second)
	for _, req := range []struct {
		peerID   string
		peerName string
		handle   string
	}{
		{peerID: "peer-with-handle", peerName: "device-alpha", handle: "jane-doe"},
		{peerID: "peer-without-handle", peerName: "device-bravo"},
	} {
		if _, _, err := s.Onboard.RequestReactivation(context.Background(), req.peerID, req.peerName, req.handle, now); err != nil {
			t.Fatal(err)
		}
	}
	rec := httptest.NewRecorder()
	s.Routes().ServeHTTP(rec, httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/reactivation-requests", nil))
	if rec.Code != http.StatusOK {
		t.Fatalf("status=%d body=%s", rec.Code, rec.Body.String())
	}
	body := rec.Body.String()
	if strings.Count(body, "named in the hold record") != 1 {
		t.Fatalf("named-in-record labels=%d, want 1", strings.Count(body, "named in the hold record"))
	}
	for _, want := range []string{
		"<strong>jane-doe</strong><br><span class=\"muted\">named in the hold record</span>",
		"<span class=\"muted\">not attributed</span>",
		"The member name comes from the hold record; Mesh holds may not attribute the device if the onboarding roster does not.",
		`href="/holds?filter=peer-with-handle"`,
		`href="/holds?filter=peer-without-handle"`,
	} {
		if !strings.Contains(body, want) {
			t.Errorf("queue missing %q", want)
		}
	}
}

func TestProbationExplainerRendersRequestFormWhenStoreCheckFails(t *testing.T) {
	s, _ := probationServer(t, meshstate.HoldKindDormancy, "ZAGROS-PROBATION-DEVELOPER")
	if err := s.Onboard.Close(); err != nil {
		t.Fatalf("close store: %v", err)
	}
	rec := httptest.NewRecorder()
	s.Routes().ServeHTTP(rec, probationRequest(http.MethodGet, "/internal/probation/auth", ""))
	if rec.Code != http.StatusLocked || !strings.Contains(rec.Body.String(), "<form") || !strings.Contains(rec.Body.String(), "Request reactivation") {
		t.Fatalf("status=%d body=%s, want explainer with request form", rec.Code, rec.Body.String())
	}
}

func TestReactivationQueueAdminOnlyAndEmptyState(t *testing.T) {
	s := newOnboardTestServer(t)
	s.Cfg.DevBypassAuth = false

	unauthorized := httptest.NewRecorder()
	s.Routes().ServeHTTP(unauthorized, httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/reactivation-requests", nil))
	if unauthorized.Code != http.StatusForbidden {
		t.Fatalf("unauthorized status=%d", unauthorized.Code)
	}

	adminReq := httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/reactivation-requests", nil)
	adminReq.Header.Set("X-Forwarded-Email", "admin@example.test")
	adminReq.Header.Set("X-Forwarded-User", "admin-id")
	adminReq.Header.Set("X-Forwarded-Groups", "ADMINS")
	empty := httptest.NewRecorder()
	s.Routes().ServeHTTP(empty, adminReq)
	if empty.Code != http.StatusOK || !strings.Contains(empty.Body.String(), "No reactivation requests") {
		t.Fatalf("empty queue status=%d body=%s", empty.Code, empty.Body.String())
	}
}

func TestReactivationRequestDeclineFlow(t *testing.T) {
	s := newOnboardTestServer(t)
	entered := time.Date(2026, time.October, 20, 10, 0, 0, 0, time.UTC)
	req, _, err := s.Onboard.RequestReactivation(context.Background(), "peer-1", "peer-jane", "jane", entered)
	if err != nil {
		t.Fatal(err)
	}
	preview := httptest.NewRecorder()
	s.Routes().ServeHTTP(preview, httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/reactivation-requests/"+req.ID+"/decline", nil))
	if preview.Code != http.StatusOK || !strings.Contains(preview.Body.String(), "Decline reactivation request") || !strings.Contains(preview.Body.String(), "peer-1") || !strings.Contains(preview.Body.String(), "jane") {
		t.Fatalf("preview status=%d body=%s", preview.Code, preview.Body.String())
	}
	form := url.Values{"csrf": {s.csrfNonce}, "reason": {"Identity could not be confirmed"}}
	post := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/reactivation-requests/"+req.ID+"/decline", strings.NewReader(form.Encode()))
	post.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	rec := httptest.NewRecorder()
	s.Routes().ServeHTTP(rec, post)
	if rec.Code != http.StatusSeeOther {
		t.Fatalf("decline status=%d body=%s", rec.Code, rec.Body.String())
	}
	location, err := url.Parse(rec.Header().Get("Location"))
	if err != nil || location.Path != "/reactivation-requests" || location.Query().Get("declined") != req.ID {
		t.Fatalf("decline redirect=%q err=%v, want request-specific confirmation", rec.Header().Get("Location"), err)
	}
	got, err := s.Onboard.GetReactivationRequest(context.Background(), req.ID)
	if err != nil || got.Status != "declined" || got.DecidedBy != "dev@bypass.local" || got.DecisionReason != "Identity could not be confirmed" || got.DecidedAt == 0 {
		t.Fatalf("declined request=%+v err=%v", got, err)
	}
	second := httptest.NewRecorder()
	post = httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/reactivation-requests/"+req.ID+"/decline", strings.NewReader(form.Encode()))
	post.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	s.Routes().ServeHTTP(second, post)
	if second.Code != http.StatusConflict {
		t.Fatalf("second decline status=%d body=%s", second.Code, second.Body.String())
	}

	confirmed := httptest.NewRecorder()
	s.Routes().ServeHTTP(confirmed, httptest.NewRequestWithContext(context.Background(), http.MethodGet, location.String(), nil))
	if confirmed.Code != http.StatusOK || !strings.Contains(confirmed.Body.String(), "Reactivation request declined.") {
		t.Fatalf("confirmed queue status=%d body=%s", confirmed.Code, confirmed.Body.String())
	}
	forged := httptest.NewRecorder()
	s.Routes().ServeHTTP(forged, httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/reactivation-requests?declined=not-a-request", nil))
	if forged.Code != http.StatusOK || strings.Contains(forged.Body.String(), "Reactivation request declined.") {
		t.Fatalf("forged confirmation status=%d body=%s", forged.Code, forged.Body.String())
	}
	otherOperatorReq := httptest.NewRequestWithContext(context.Background(), http.MethodGet, location.String(), nil)
	otherOperatorReq.Header.Set("X-Forwarded-Email", "other-admin@example.test")
	otherOperatorReq.Header.Set("X-Forwarded-Groups", "ADMINS")
	otherOperator := httptest.NewRecorder()
	s.Routes().ServeHTTP(otherOperator, otherOperatorReq)
	if otherOperator.Code != http.StatusOK || strings.Contains(otherOperator.Body.String(), "Reactivation request declined.") {
		t.Fatalf("other operator confirmation status=%d body=%s", otherOperator.Code, otherOperator.Body.String())
	}
	closedReq, _, err := s.Onboard.RequestReactivation(context.Background(), "peer-2", "peer-alex", "alex", entered.Add(time.Second))
	if err != nil {
		t.Fatal(err)
	}
	closed, err := s.Onboard.CloseReactivationRequest(context.Background(), closedReq.PeerID, time.Unix(closedReq.HoldEnteredAt, 0), "dev@bypass.local")
	if err != nil || !closed {
		t.Fatalf("close request=%v err=%v", closed, err)
	}
	closedMarker := httptest.NewRecorder()
	s.Routes().ServeHTTP(closedMarker, httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/reactivation-requests?declined="+url.QueryEscape(closedReq.ID), nil))
	if closedMarker.Code != http.StatusOK || strings.Contains(closedMarker.Body.String(), "Reactivation request declined.") {
		t.Fatalf("closed request confirmation status=%d body=%s", closedMarker.Code, closedMarker.Body.String())
	}
}

func TestReactivationDeclineReasonCountsCharacters(t *testing.T) {
	for _, tc := range []struct {
		name       string
		characters int
		wantStatus int
		wantState  string
	}{
		{"500 multibyte characters accepted", 500, http.StatusSeeOther, onboardstore.ReactivationDeclined},
		{"501 multibyte characters refused", 501, http.StatusBadRequest, onboardstore.ReactivationOpen},
	} {
		t.Run(tc.name, func(t *testing.T) {
			s := newOnboardTestServer(t)
			entered := time.Date(2026, time.October, 20, 10, 0, 0, 0, time.UTC)
			req, _, err := s.Onboard.RequestReactivation(context.Background(), "peer-1", "peer-jane", "jane", entered)
			if err != nil {
				t.Fatal(err)
			}
			form := url.Values{"csrf": {s.csrfNonce}, "reason": {strings.Repeat("é", tc.characters)}}
			post := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/reactivation-requests/"+req.ID+"/decline", strings.NewReader(form.Encode()))
			post.Header.Set("Content-Type", "application/x-www-form-urlencoded")
			rec := httptest.NewRecorder()
			s.Routes().ServeHTTP(rec, post)
			if rec.Code != tc.wantStatus {
				t.Fatalf("status=%d body=%s, want %d", rec.Code, rec.Body.String(), tc.wantStatus)
			}
			got, err := s.Onboard.GetReactivationRequest(context.Background(), req.ID)
			if err != nil || got.Status != tc.wantState {
				t.Fatalf("request=%+v err=%v, want status %q", got, err, tc.wantState)
			}
		})
	}
}

func TestReactivationDeclineDirectPostIsAdminOnly(t *testing.T) {
	s := newOnboardTestServer(t)
	s.Cfg.DevBypassAuth = false
	entered := time.Date(2026, time.October, 20, 10, 0, 0, 0, time.UTC)
	req, _, err := s.Onboard.RequestReactivation(context.Background(), "peer-1", "peer-jane", "jane", entered)
	if err != nil {
		t.Fatal(err)
	}
	form := url.Values{"csrf": {s.csrfNonce}}
	post := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/reactivation-requests/"+req.ID+"/decline", strings.NewReader(form.Encode()))
	post.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	post.Header.Set("X-Forwarded-Email", "member@example.invalid")
	post.Header.Set("X-Forwarded-Groups", "MEMBERS")
	rec := httptest.NewRecorder()
	s.Routes().ServeHTTP(rec, post)
	if rec.Code != http.StatusForbidden {
		t.Fatalf("non-admin decline status=%d body=%s", rec.Code, rec.Body.String())
	}
	got, _ := s.Onboard.GetReactivationRequest(context.Background(), req.ID)
	if got.Status != "open" {
		t.Fatalf("non-admin changed request: %+v", got)
	}
}
