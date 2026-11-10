package handlers

import (
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// The Offboard button that does the REAL cut (goat#372): a step-up-gated
// admin click queues the row as offboard-requested for the onboard-runner,
// instead of asserting the cut already happened elsewhere (that assertion —
// the bookkeeping "Mark offboarded" — stays, precheck and all). The step-up
// challenge itself is exercised by the stepup flow tests; DevBypassAuth
// short-circuits it here, so these tests pin the queue behavior and the
// config parity test pins the gate wiring.

func postOffboardCut(t *testing.T, h http.Handler, s *Server, pid string) *httptest.ResponseRecorder {
	t.Helper()
	form := url.Values{}
	form.Set("csrf", s.csrfNonce)
	req := httptest.NewRequestWithContext(t.Context(), http.MethodPost, "/onboarding/"+pid+"/offboard-cut", strings.NewReader(form.Encode()))
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	return rec
}

// A click on an eligible row queues the cut with the acting admin recorded —
// no access changes here; the runner performs the revoke.
func TestOffboardCut_QueuesTheRealCut(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	pid := onboardedProposalInState(t, s, h, "person-87@example.org", onboardstore.ProposalOnboarded)

	rec := postOffboardCut(t, h, s, pid)
	if rec.Code != http.StatusSeeOther {
		t.Fatalf("want 303, got %d: %s", rec.Code, rec.Body.String())
	}
	if loc := rec.Header().Get("Location"); !strings.Contains(loc, "flash=offboard_cut_queued") {
		t.Errorf("Location = %q, want the queued flash", loc)
	}
	if got := statusOf(t, s, pid); got != onboardstore.ProposalOffboardRequested {
		t.Fatalf("status = %q, want offboard-requested", got)
	}
	p := mustGetProposal(t, s, pid)
	if p.OffboardRequestedBy != "dev@bypass.local" {
		t.Errorf("offboard_requested_by = %q, want the acting admin dev@bypass.local", p.OffboardRequestedBy)
	}
}

// A pre-provisioning row has nothing to cut — reject/cancel is the right verb
// there, so the click is refused with a flash and the row does not move.
func TestOffboardCut_RefusesAPreProvisioningRow(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	tok := mintTokenViaStore(t, s)
	pid := submit(t, h, tok, map[string]any{"name": "Pat Start", "email": "person-93@example.org", "organization": "Offboard Org"})

	rec := postOffboardCut(t, h, s, pid)
	if rec.Code != http.StatusSeeOther {
		t.Fatalf("want 303, got %d: %s", rec.Code, rec.Body.String())
	}
	if loc := rec.Header().Get("Location"); !strings.Contains(loc, "flash=offboard_cut_state") {
		t.Errorf("Location = %q, want the state flash", loc)
	}
	if got := statusOf(t, s, pid); got != onboardstore.ProposalProposed {
		t.Errorf("status = %q — a refused cut must not move the row", got)
	}
}

// Two admins clicking at once: the second click is refused loudly, the queue
// entry stays single, and the row does not leave offboard-requested.
func TestOffboardCut_SecondClickIsRefusedLoudly(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	pid := onboardedProposalInState(t, s, h, "person-103@example.org", onboardstore.ProposalEnrolled)

	if rec := postOffboardCut(t, h, s, pid); rec.Code != http.StatusSeeOther {
		t.Fatalf("first click: want 303, got %d", rec.Code)
	}
	rec := postOffboardCut(t, h, s, pid)
	if rec.Code != http.StatusSeeOther {
		t.Fatalf("second click: want 303, got %d: %s", rec.Code, rec.Body.String())
	}
	if loc := rec.Header().Get("Location"); !strings.Contains(loc, "flash=offboard_cut_state") {
		t.Errorf("second click Location = %q, want the state flash (already queued)", loc)
	}
	if got := statusOf(t, s, pid); got != onboardstore.ProposalOffboardRequested {
		t.Errorf("status = %q, want still offboard-requested", got)
	}
}

// The real-cut button renders on eligible rows and nowhere else.
func TestOffboardCut_ButtonRendersOnlyForEligibleRows(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	eligible := onboardedProposalInState(t, s, h, "person-84@example.org", onboardstore.ProposalOnboarded)
	tok := mintTokenViaStore(t, s)
	early := submit(t, h, tok, map[string]any{"name": "Pat Btn", "email": "person-80@example.org", "organization": "Offboard Org"})
	// The real-cut states: failed re-queues (the person still has access);
	// queued and running offer nothing (the cut is already in flight).
	failed := offboardRowTo(t, s, h, "person-81@example.org", onboardstore.ProposalOffboardFailed)
	queued := offboardRowTo(t, s, h, "person-82@example.org", onboardstore.ProposalOffboardRequested)
	running := offboardRowTo(t, s, h, "person-83@example.org", onboardstore.ProposalOffboarding)

	for _, tc := range []struct {
		status, pid string
		want        bool
	}{
		{onboardstore.ProposalOnboarded, eligible, true},
		{onboardstore.ProposalProposed, early, false},
		{onboardstore.ProposalOffboardFailed, failed, true},
		{onboardstore.ProposalOffboardRequested, queued, false},
		{onboardstore.ProposalOffboarding, running, false},
	} {
		req := httptest.NewRequestWithContext(t.Context(), http.MethodGet, "/onboarding?status="+tc.status+"&id="+tc.pid, nil)
		req.Header.Set("X-Forwarded-Groups", "PLATFORM-ADMIN")
		req.Header.Set("X-Forwarded-Email", "admin@example.org")
		rec := httptest.NewRecorder()
		h.ServeHTTP(rec, req)
		if rec.Code != http.StatusOK {
			t.Fatalf("%s: want 200, got %d", tc.status, rec.Code)
		}
		if got := strings.Contains(rec.Body.String(), "/offboard-cut"); got != tc.want {
			t.Errorf("%s row: offboard-cut form rendered = %v, want %v", tc.status, got, tc.want)
		}
	}
}

// On an archived row the cut lands the record at offboarded, and
// RestoreProposal only accepts archived — so that card's confirm has to name
// the one-way record on top of the reversible access, the same disclosure
// #1203 gave the sibling "Mark offboarded" button. Every other card's confirm
// speaks only to access, which is genuinely reversible.
func TestOffboardCut_ArchivedConfirmNamesTheOneWayRecord(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	archived := onboardedProposalInState(t, s, h, "person-77@example.org", onboardstore.ProposalArchived)
	onboarded := onboardedProposalInState(t, s, h, "person-96@example.org", onboardstore.ProposalOnboarded)

	const oneWay = "an offboarded record cannot be restored"
	for _, tc := range []struct {
		status, pid string
		want        bool
	}{
		{onboardstore.ProposalArchived, archived, true},
		{onboardstore.ProposalOnboarded, onboarded, false},
	} {
		req := httptest.NewRequestWithContext(t.Context(), http.MethodGet, "/onboarding?status="+tc.status+"&id="+tc.pid, nil)
		req.Header.Set("X-Forwarded-Groups", "PLATFORM-ADMIN")
		req.Header.Set("X-Forwarded-Email", "admin@example.org")
		rec := httptest.NewRecorder()
		h.ServeHTTP(rec, req)
		if rec.Code != http.StatusOK {
			t.Fatalf("%s: want 200, got %d", tc.status, rec.Code)
		}
		body := rec.Body.String()
		i := strings.Index(body, "/offboard-cut")
		if i < 0 {
			t.Fatalf("%s row: no offboard-cut form rendered", tc.status)
		}
		// Scope the assertion to the cut form's own confirm, so the sibling
		// "Mark offboarded" confirm cannot satisfy it.
		form := body[i:]
		if end := strings.Index(form, "</form>"); end > 0 {
			form = form[:end]
		}
		if got := strings.Contains(form, oneWay); got != tc.want {
			t.Errorf("%s row: cut confirm names the one-way record = %v, want %v", tc.status, got, tc.want)
		}
	}
}
