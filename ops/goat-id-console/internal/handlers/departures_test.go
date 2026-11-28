package handlers

import (
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// The departure the Offboard button records (ADR 1272): the cut is refused
// without the reason the person left, the reason and the day are written with
// the queue entry, and /departures lists every cut with them.

func postOffboardCutForm(t *testing.T, h http.Handler, s *Server, pid string, fields map[string]string) *httptest.ResponseRecorder {
	t.Helper()
	form := url.Values{}
	form.Set("csrf", s.csrfNonce)
	for k, v := range fields {
		form.Set(k, v)
	}
	req := httptest.NewRequestWithContext(t.Context(), http.MethodPost, "/onboarding/"+pid+"/offboard-cut", strings.NewReader(form.Encode()))
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	return rec
}

func TestOffboardCut_RefusesWithoutAValidDeparture(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	pid := onboardedProposalInState(t, s, h, "person-41@example.org", onboardstore.ProposalOnboarded)
	for name, fields := range map[string]map[string]string{
		"no reason":          {},
		"unknown reason":     {"reason_class": "retired"},
		"other without text": {"reason_class": "other", "reason_text": "  "},
		"malformed day":      {"reason_class": "left-service", "effective": "27 Nov"},
		"two-line text":      {"reason_class": "other", "reason_text": "one\ntwo"},
	} {
		rec := postOffboardCutForm(t, h, s, pid, fields)
		if rec.Code != http.StatusBadRequest {
			t.Errorf("%s: want 400, got %d", name, rec.Code)
		}
		if !strings.Contains(rec.Body.String(), "needs the reason the person left") {
			t.Errorf("%s: the refusal does not say what is missing: %s", name, rec.Body.String())
		}
		if got := statusOf(t, s, pid); got != onboardstore.ProposalOnboarded {
			t.Fatalf("%s: status = %q; a refused cut must not move the row", name, got)
		}
	}
}

func TestOffboardCut_RecordsTheDeparture(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	pid := onboardedProposalInState(t, s, h, "person-42@example.org", onboardstore.ProposalOnboarded)
	rec := postOffboardCutForm(t, h, s, pid, map[string]string{
		"reason_class": "other", "reason_text": " contract ended ", "effective": "2031-02-03"})
	if rec.Code != http.StatusSeeOther {
		t.Fatalf("want 303, got %d: %s", rec.Code, rec.Body.String())
	}
	p := mustGetProposal(t, s, pid)
	if p.Status != onboardstore.ProposalOffboardRequested {
		t.Fatalf("status = %q, want offboard-requested", p.Status)
	}
	if p.OffboardReasonClass != "other" || p.OffboardReasonText != "contract ended" || p.OffboardEffective != "2031-02-03" {
		t.Errorf("departure = %q / %q / %q", p.OffboardReasonClass, p.OffboardReasonText, p.OffboardEffective)
	}
}

func TestOffboardCut_EffectiveDefaultsToToday(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	pid := onboardedProposalInState(t, s, h, "person-43@example.org", onboardstore.ProposalOnboarded)
	before := time.Now().UTC().Format("2006-01-02")
	if rec := postOffboardCutForm(t, h, s, pid, map[string]string{"reason_class": "new-position"}); rec.Code != http.StatusSeeOther {
		t.Fatalf("want 303, got %d: %s", rec.Code, rec.Body.String())
	}
	after := time.Now().UTC().Format("2006-01-02")
	if got := mustGetProposal(t, s, pid).OffboardEffective; got != before && got != after {
		t.Errorf("effective = %q, want today (%s)", got, before)
	}
}

// A re-queue after a failed cut is the same departure: the form on that card
// carries no reason, and the recorded one stands.
func TestOffboardCut_RequeueKeepsTheRecordedDeparture(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	pid := onboardedProposalInState(t, s, h, "person-44@example.org", onboardstore.ProposalOnboarded)
	if rec := postOffboardCutForm(t, h, s, pid, map[string]string{
		"reason_class": "left-service", "effective": "2031-02-03"}); rec.Code != http.StatusSeeOther {
		t.Fatalf("queue: want 303, got %d", rec.Code)
	}
	ctx := t.Context()
	if err := s.Onboard.ClaimOffboard(ctx, pid); err != nil {
		t.Fatal(err)
	}
	if err := s.Onboard.SetOffboardResult(ctx, pid, onboardstore.ProposalOffboardFailed, "mesh leg refused", ""); err != nil {
		t.Fatal(err)
	}
	if rec := postOffboardCutForm(t, h, s, pid, nil); rec.Code != http.StatusSeeOther {
		t.Fatalf("re-queue: want 303, got %d: %s", rec.Code, rec.Body.String())
	}
	p := mustGetProposal(t, s, pid)
	if p.Status != onboardstore.ProposalOffboardRequested || p.OffboardReasonClass != "left-service" || p.OffboardEffective != "2031-02-03" {
		t.Errorf("after re-queue: %q / %q / %q", p.Status, p.OffboardReasonClass, p.OffboardEffective)
	}
}

func TestDepartures_ListsEveryCutWithItsReasonAndState(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	ctx := t.Context()
	done := onboardedProposalInState(t, s, h, "person-45@example.org", onboardstore.ProposalOnboarded)
	open := onboardedProposalInState(t, s, h, "person-46@example.org", onboardstore.ProposalOnboarded)
	onboardedProposalInState(t, s, h, "person-47@example.org", onboardstore.ProposalOnboarded) // never cut
	for pid, class := range map[string]string{done: "left-service", open: "new-position"} {
		if rec := postOffboardCutForm(t, h, s, pid, map[string]string{"reason_class": class, "effective": "2031-02-03"}); rec.Code != http.StatusSeeOther {
			t.Fatalf("queue %s: %d", class, rec.Code)
		}
	}
	if err := s.Onboard.ClaimOffboard(ctx, done); err != nil {
		t.Fatal(err)
	}
	if err := s.Onboard.SetOffboardResult(ctx, done, onboardstore.ProposalOffboarded, "", "recorded"); err != nil {
		t.Fatal(err)
	}

	req := httptest.NewRequestWithContext(ctx, http.MethodGet, "/departures", nil)
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("GET /departures = %d: %s", rec.Code, rec.Body.String())
	}
	body := rec.Body.String()
	doneHandle, openHandle := mustGetProposal(t, s, done).Handle, mustGetProposal(t, s, open).Handle
	for _, want := range []string{
		`data-departures-open="1"`,
		`data-departure="` + doneHandle + `" data-state="cut" data-status="offboarded" data-reason="left-service"`,
		`data-departure="` + openHandle + `" data-state="open" data-status="offboard-requested" data-reason="new-position"`,
		`/offboard-verify?person=` + openHandle,
		"2031-02-03",
	} {
		if !strings.Contains(body, want) {
			t.Errorf("page lacks %q", want)
		}
	}
	if n := strings.Count(body, "data-departure="); n != 2 {
		t.Errorf("%d rows listed, want 2: a person never cut is not a departure", n)
	}
	if strings.Contains(body, "example.org") {
		t.Error("the register shows a login email; it names people by handle")
	}
}

func TestDeparture_ValidateHoldsToThreeClasses(t *testing.T) {
	for _, tc := range []struct {
		d  onboardstore.Departure
		ok bool
	}{
		{onboardstore.Departure{Class: "new-position", Effective: "2031-02-03"}, true},
		{onboardstore.Departure{Class: "left-service", Effective: "2031-02-03"}, true},
		{onboardstore.Departure{Class: "other", Text: "why", Effective: "2031-02-03"}, true},
		{onboardstore.Departure{Class: "other", Effective: "2031-02-03"}, false},
		{onboardstore.Departure{Class: "", Effective: "2031-02-03"}, false},
		{onboardstore.Departure{Class: "left-service", Effective: "2031-02-31"}, false},
		{onboardstore.Departure{Class: "left-service", Effective: ""}, false},
		{onboardstore.Departure{Class: "other", Text: strings.Repeat("x", 301), Effective: "2031-02-03"}, false},
	} {
		d := tc.d
		if err := d.Validate(); (err == nil) != tc.ok {
			t.Errorf("%+v: err = %v, want ok=%v", tc.d, err, tc.ok)
		}
	}
}
