package handlers

// H2 regression: apiCollectOperatorKey binds the one-time delivery to the
// CALLING principal, not to a free `handle` query parameter. Before the fix the
// store only checked the row's handle matched the handle it was handed, so any
// operator principal could name someone else's handle and walk off with their
// not-yet-delivered op_ key — defeating the F-207 per-person attribution and
// letting one person satisfy both halves of the two-person rule. The browser
// sibling (handleOperatorKeyCollect) already derived the handle from the
// session; this test pins the machine realm to the same rule.

import (
	"context"
	"net/http"
	"testing"
)

func TestApiCollectOperatorKey_boundToCallingPrincipal(t *testing.T) {
	s := newOnboardTestServer(t)
	// Two distinct operator principals. "runner-key" → "onboard-runner" (A) is
	// seeded by newOnboardTestServer; add a second key for principal B.
	s.Cfg.OnboardOperatorAPIKeys["beta-key"] = "beta-op"
	h := s.Routes()

	// B's key is requested and approved by a third party, so it is Active with a
	// live secret waiting to be collected exactly once.
	rowB := seedOperatorKeyRequest(t, s, "beta-op", "person-17@example.org")
	if _, err := s.Onboard.ApproveOperatorKey(context.Background(), rowB.ID, "admin", []string{"admin"}); err != nil {
		t.Fatalf("approve B: %v", err)
	}

	// A (runner-key) attempts to collect B's key by naming B's handle — refused
	// 403 before the store is ever consulted, so B's secret stays undelivered.
	if w := machineReq(t, h, "runner-key", "GET",
		"/api/onboard/operator-keys/"+rowB.ID+"/collect?handle=beta-op", ""); w.Code != http.StatusForbidden {
		t.Fatalf("A collecting B by query handle = %d, want 403; %s", w.Code, w.Body.String())
	}

	// A collecting B's key WITHOUT naming a handle binds to A ("onboard-runner"),
	// which does not own the row — the store refuses (404), never a leak.
	if w := machineReq(t, h, "runner-key", "GET",
		"/api/onboard/operator-keys/"+rowB.ID+"/collect", ""); w.Code != http.StatusNotFound {
		t.Fatalf("A collecting B's row bound to self = %d, want 404; %s", w.Code, w.Body.String())
	}

	// B's key is still deliverable — the refused attempts spent nothing. B
	// collects it with B's own key.
	if w := machineReq(t, h, "beta-key", "GET",
		"/api/onboard/operator-keys/"+rowB.ID+"/collect", ""); w.Code != http.StatusOK {
		t.Fatalf("B collecting own key = %d, want 200; %s", w.Code, w.Body.String())
	}

	// A collecting A's own key, both with and without an explicit matching
	// handle, succeeds.
	rowA := seedOperatorKeyRequest(t, s, "onboard-runner", "person-110@example.org")
	if _, err := s.Onboard.ApproveOperatorKey(context.Background(), rowA.ID, "admin", []string{"admin"}); err != nil {
		t.Fatalf("approve A: %v", err)
	}
	if w := machineReq(t, h, "runner-key", "GET",
		"/api/onboard/operator-keys/"+rowA.ID+"/collect?handle=onboard-runner", ""); w.Code != http.StatusOK {
		t.Fatalf("A collecting own key (explicit handle) = %d, want 200; %s", w.Code, w.Body.String())
	}
}
