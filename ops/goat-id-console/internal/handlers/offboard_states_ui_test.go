package handlers

import (
	"context"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// The console surface of the three real-cut states (goat#372 review): a cut
// that is queued, running, or failed must be visible on the page an admin
// uses — a tab to find the row, a panel that says what is happening, the
// runner's failure reason rendered, and a re-queue action on offboard-failed.
// "Loud in the queue" at the store layer only (the API tests) is not loud.

// offboardRowTo drives a fresh onboarded proposal into one of the real-cut
// states through the same doors production uses: the admin's offboard-cut
// click, then the machine claim/result contract.
func offboardRowTo(t *testing.T, s *Server, h http.Handler, email, target string) string {
	t.Helper()
	pid := onboardedProposalInState(t, s, h, email, onboardstore.ProposalOnboarded)
	if rec := postOffboardCut(t, h, s, pid); rec.Code != http.StatusSeeOther {
		t.Fatalf("queue cut: want 303, got %d: %s", rec.Code, rec.Body.String())
	}
	if target == onboardstore.ProposalOffboardRequested {
		return pid
	}
	if c := postMachine(t, h, "/api/onboard/proposals/"+pid+"/offboard-claim", nil); c.code != http.StatusOK {
		t.Fatalf("offboard-claim: %d %s", c.code, c.body)
	}
	if target == onboardstore.ProposalOffboarding {
		return pid
	}
	body := map[string]any{"status": target}
	if target == onboardstore.ProposalOffboardFailed {
		body["error"] = "revoke: peer cut refused"
	}
	if c := postMachine(t, h, "/api/onboard/proposals/"+pid+"/result", body); c.code != http.StatusOK {
		t.Fatalf("result %s: %d %s", target, c.code, c.body)
	}
	return pid
}

func getQueuePage(t *testing.T, h http.Handler, path string) *httptest.ResponseRecorder {
	t.Helper()
	req := httptest.NewRequestWithContext(t.Context(), http.MethodGet, path, nil)
	req.Header.Set("X-Forwarded-Groups", "PLATFORM-ADMIN")
	req.Header.Set("X-Forwarded-Email", "admin@example.org")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	return rec
}

// Every real-cut state has a tab in the stage strip — without one, a row in
// these states exists on no page an admin can reach by clicking.
func TestOffboardStates_HaveQueueTabs(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	rec := getQueuePage(t, h, "/onboarding")
	if rec.Code != http.StatusOK {
		t.Fatalf("want 200, got %d", rec.Code)
	}
	for _, key := range []string{"offboard-requested", "offboarding", "offboard-failed"} {
		if !strings.Contains(rec.Body.String(), `data-stage="`+key+`"`) {
			t.Errorf("stage strip has no tab for %s", key)
		}
	}
}

// The offboard-failed panel is the loud one: the person still has access. It
// must render the runner's failure reason (persisted in note), name who
// queued the cut, and offer the re-queue — the store supports the retry
// (RequestOffboard from offboard-failed); the page must expose it.
func TestOffboardFailedDetail_ShowsReasonRequesterAndRequeue(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	pid := offboardRowTo(t, s, h, "person-89@example.org", onboardstore.ProposalOffboardFailed)

	rec := getQueuePage(t, h, "/onboarding?status=offboard-failed&id="+pid)
	if rec.Code != http.StatusOK {
		t.Fatalf("want 200, got %d", rec.Code)
	}
	body := rec.Body.String()
	if !strings.Contains(body, "revoke: peer cut refused") {
		t.Error("panel does not render the runner's failure reason (note)")
	}
	if !strings.Contains(body, "dev@bypass.local") {
		t.Error("panel does not name who queued the cut (offboard_requested_by)")
	}
	if !strings.Contains(body, "/offboard-cut") {
		t.Error("panel offers no re-queue — the failed cut is a dead end on the page")
	}
}

// Queued and running cuts each get a panel that says what is happening and
// who asked — and deliberately no button: there is nothing for a second
// admin to do to a cut already in flight.
func TestOffboardInFlightDetails_NameRequesterWithoutButton(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	for _, tc := range []struct{ email, state string }{
		{"person-90@example.org", onboardstore.ProposalOffboardRequested},
		{"person-91@example.org", onboardstore.ProposalOffboarding},
	} {
		pid := offboardRowTo(t, s, h, tc.email, tc.state)
		rec := getQueuePage(t, h, "/onboarding?status="+tc.state+"&id="+pid)
		if rec.Code != http.StatusOK {
			t.Fatalf("%s: want 200, got %d", tc.state, rec.Code)
		}
		body := rec.Body.String()
		if !strings.Contains(body, "dev@bypass.local") {
			t.Errorf("%s: panel does not name who queued the cut", tc.state)
		}
		if strings.Contains(body, "no action available") {
			t.Errorf("%s: row falls through to the generic no-action branch", tc.state)
		}
		if strings.Contains(body, "/offboard-cut") {
			t.Errorf("%s: a cut already in flight must not offer the button", tc.state)
		}
	}
}

// A runner-cut row must not read as a hand-marked one: decided_by is the
// FINAL ONBOARDING approver (untouched by the cut, correctly), so the panel
// names offboard_requested_by and says the runner executed it — not
// "marked offboarded by <approver>" with access left an open question.
func TestOffboardedDetail_RunnerCutSaysSo(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	pid := offboardRowTo(t, s, h, "person-88@example.org", onboardstore.ProposalOffboarded)

	rec := getQueuePage(t, h, "/onboarding?status=offboarded&id="+pid)
	if rec.Code != http.StatusOK {
		t.Fatalf("want 200, got %d", rec.Code)
	}
	body := rec.Body.String()
	if !strings.Contains(body, "dev@bypass.local") {
		t.Error("runner-cut panel does not name who queued the cut")
	}
	if !strings.Contains(body, "runner") {
		t.Error("runner-cut panel does not say the runner executed the cut")
	}
	if strings.Contains(body, "separate question") {
		t.Error("runner-cut panel hedges on whether access was cut — the ledgered revoke answered it")
	}
}

// The failed (onboarding) panel promises the note above its retry button, so
// proposalView must carry Note: a missing field is a template exec error on the
// first failed row an admin opens, and only a render test sees it.
func TestFailedDetail_ShowsRunnerNote(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	tok := mintTokenViaStore(t, s)
	pid := submit(t, h, tok, map[string]any{"name": "Pat Mintfail", "email": "person-97@example.org", "organization": "Offboard Org"})
	for _, st := range []string{onboardstore.ProposalSponsorApproved, onboardstore.ProposalApproved} {
		if err := s.Onboard.DecideProposal(context.Background(), pid, st, "admin@x", ""); err != nil {
			t.Fatalf("decide %s: %v", st, err)
		}
	}
	if c := postMachine(t, h, "/api/onboard/proposals/"+pid+"/claim", nil); c.code != http.StatusOK {
		t.Fatalf("claim: %d %s", c.code, c.body)
	}
	if c := postMachine(t, h, "/api/onboard/proposals/"+pid+"/result",
		map[string]any{"status": onboardstore.ProposalFailed, "error": "mint: goatnet missing from native config tables"}); c.code != http.StatusOK {
		t.Fatalf("result failed: %d %s", c.code, c.body)
	}

	rec := getQueuePage(t, h, "/onboarding?status=failed&id="+pid)
	if rec.Code != http.StatusOK {
		t.Fatalf("want 200, got %d", rec.Code)
	}
	if !strings.Contains(rec.Body.String(), "goatnet missing from native config tables") {
		t.Error("failed panel does not render the runner's note")
	}
}
