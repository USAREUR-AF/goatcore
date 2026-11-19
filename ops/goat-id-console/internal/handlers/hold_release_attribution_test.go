package handlers

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

func postReleaseConfirm(t *testing.T, s *Server, entered string, identityID string) *httptest.ResponseRecorder {
	t.Helper()
	form := url.Values{"csrf": {s.csrfNonce}, "entered_at": {entered}}
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/holds/peer-1/release", strings.NewReader(form.Encode()))
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	if identityID != "" {
		req.Header.Set("X-Forwarded-User", identityID)
	}
	rec := httptest.NewRecorder()
	s.Routes().ServeHTTP(rec, req)
	return rec
}

// The release's operator-action actor is the session identity. A key that
// files an intake row carrying the admin's email and another handle can no
// longer have the release recorded under that handle (pre-existing, the same
// email lookup as the offboard cut in GHSA-3m4r).
func TestReleaseHoldActorComesFromTheSession(t *testing.T) {
	s, entered := releaseTestServer(t, "dormancy", []string{"ZAGROS-DEVELOPER"}, "ZAGROS-PROBATION-DEVELOPER")
	res := postMachine(t, s.Routes(), "/api/onboard/intake", map[string]any{"manifest": plantedManifest("dev@bypass.local", "admin-bob"), "env": "zagros"})
	if res.code != http.StatusCreated {
		t.Fatalf("intake: %d %s", res.code, res.body)
	}
	if rec := postReleaseConfirm(t, s, stringInt(entered.Unix()), testActorID); rec.Code != http.StatusSeeOther {
		t.Fatalf("confirm status=%d body=%s", rec.Code, rec.Body.String())
	}
	actions, err := s.Onboard.ListActions(context.Background(), onboardstore.ActionQueued)
	if err != nil || len(actions) != 1 {
		t.Fatalf("queued actions=%+v err=%v", actions, err)
	}
	var p releaseHoldParams
	if err := json.Unmarshal([]byte(actions[0].Params), &p); err != nil {
		t.Fatal(err)
	}
	want := "kratos-" + testActorID
	if actions[0].Handle != want || p.RequestedByHandle != want {
		t.Fatalf("release attributed to handle=%q params=%q, want %q", actions[0].Handle, p.RequestedByHandle, want)
	}
}

// Without a session identity the release is refused before anything is
// queued, as it was when no handle resolved.
func TestReleaseHoldWithoutSessionIdentityIsRefused(t *testing.T) {
	s, entered := releaseTestServer(t, "dormancy", []string{"ZAGROS-DEVELOPER"}, "ZAGROS-PROBATION-DEVELOPER")
	if rec := postReleaseConfirm(t, s, stringInt(entered.Unix()), ""); rec.Code != http.StatusConflict {
		t.Fatalf("confirm status=%d, want 409", rec.Code)
	}
	if actions, err := s.Onboard.ListActions(context.Background(), onboardstore.ActionQueued); err != nil || len(actions) != 0 {
		t.Fatalf("queued actions=%+v err=%v, want none", actions, err)
	}
}
