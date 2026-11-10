package handlers

// Reviewer notifications + the queue-badge counts endpoint.
//
// The console under test has no SMTP configured, so every send lands as an
// audited attempt with result "error"/"mailer disabled" rather than an actual
// message. That is the assertion these tests want: WHETHER a notification was
// owed to somebody, and to whom — the delivery leg is the mailer package's
// concern and is exercised there.

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/audit"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// noticeTo returns the recipients recorded for one notification action.
func noticeTo(s *Server, action string) []string {
	var out []string
	for _, e := range s.Audit.Recent(200) {
		if e.Action == action {
			if to, ok := e.After["to"].(string); ok {
				out = append(out, to)
			}
		}
	}
	return out
}

func noticeEvents(s *Server, action string) []audit.Event {
	var out []audit.Event
	for _, e := range s.Audit.Recent(200) {
		if e.Action == action {
			out = append(out, e)
		}
	}
	return out
}

// mintTokenSponsoredBy mints an invite whose recorded sponsor is `sponsor` —
// an email for a console-minted invite, a UUID anchor for a participant mint.
func mintTokenSponsoredBy(t *testing.T, s *Server, sponsor, sponsorName string) string {
	t.Helper()
	pb := onboardstore.Prebound{Tier: "developer", Affiliation: "partner"}
	pbJSON, _ := json.Marshal(pb)
	inv := &onboardstore.Invite{Sponsor: sponsor, SponsorName: sponsorName, Env: "zagros",
		PreboundJSON: string(pbJSON)}
	tok, err := s.Onboard.MintInvite(context.Background(), inv)
	if err != nil {
		t.Fatalf("mint via store: %v", err)
	}
	return tok
}

// TestSubmitNotifiesSponsor: the candidate's submit tells the sponsor a vouch
// is waiting — the D2 dead zone. The link is the console's own base.
func TestSubmitNotifiesSponsor(t *testing.T) {
	s := newOnboardTestServer(t)
	s.Cfg.ConsolePublicURL = "https://admin.auth.zagros.example"
	h := s.Routes()
	tok := mintTokenSponsoredBy(t, s, "sponsor@example.org", "Dana Sponsor")

	submit(t, h, tok, map[string]any{"name": "Pat Liaison", "email": "person-106@example.org", "organization": "Notify Org"})

	got := noticeTo(s, "onboard_email_sponsor_submit")
	if len(got) != 1 || got[0] != "sponsor@example.org" {
		t.Fatalf("sponsor notice recipients = %v, want [sponsor@example.org]", got)
	}
	evs := noticeEvents(s, "onboard_email_sponsor_submit")
	if evs[0].Actor.Email != "console-onboard" {
		t.Fatalf("notice actor = %q, want console-onboard (nobody decided this)", evs[0].Actor.Email)
	}
	// The submit itself still succeeded — a notification is never load-bearing.
	rows, err := s.Onboard.ListProposals(context.Background(), onboardstore.ProposalProposed)
	if err != nil || len(rows) != 1 {
		t.Fatalf("proposal not stored despite mail failure: %v (%v)", rows, err)
	}
}

// TestSubmitSkipsAnchorSponsor: a participant sponsor is recorded as a UUID org
// anchor. Mailing it is impossible, so nothing is attempted — an audit row
// claiming a send to "0ff404dd" would be a fabricated notification.
func TestSubmitSkipsAnchorSponsor(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	tok := mintTokenSponsoredBy(t, s, "0ff404dd", "")

	submit(t, h, tok, map[string]any{"name": "Pat Liaison", "email": "person-106@example.org", "organization": "Notify Org"})

	if got := noticeTo(s, "onboard_email_sponsor_submit"); len(got) != 0 {
		t.Fatalf("anchor sponsor must not be mailed, got %v", got)
	}
}

// TestVouchNotifiesAdmins: the D3 dead zone — a vouched candidate waits on the
// admin gate, and the admin list is told. Both decide doors go through
// decideProposal, so the machine path proves the shared choke point.
func TestVouchNotifiesAdmins(t *testing.T) {
	s := newOnboardTestServer(t)
	s.Cfg.OnboardAdminNotifyEmails = []string{"ops1@example.org", "ops2@example.org"}
	h := s.Routes()
	tok := mintTokenSponsoredBy(t, s, "sponsor@example.org", "Dana Sponsor")
	pid := submit(t, h, tok, map[string]any{"name": "Pat Liaison", "email": "person-106@example.org", "organization": "Notify Org"})

	if got := decideMachine(t, h, pid, "sponsor-approve"); got != onboardstore.ProposalSponsorApproved {
		t.Fatalf("vouch: got %q", got)
	}
	got := noticeTo(s, "onboard_email_admin_vouched")
	if len(got) != 2 || !strings.Contains(strings.Join(got, ","), "ops1@example.org") ||
		!strings.Contains(strings.Join(got, ","), "ops2@example.org") {
		t.Fatalf("admin notice recipients = %v, want both configured addresses", got)
	}
}

// TestVouchWithNoAdminListIsSilent: empty ONBOARD_ADMIN_NOTIFY_EMAILS = feature
// off, not a broken send.
func TestVouchWithNoAdminListIsSilent(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	tok := mintTokenSponsoredBy(t, s, "sponsor@example.org", "")
	pid := submit(t, h, tok, map[string]any{"name": "Pat Liaison", "email": "person-106@example.org", "organization": "Notify Org"})
	decideMachine(t, h, pid, "sponsor-approve")

	if got := noticeTo(s, "onboard_email_admin_vouched"); len(got) != 0 {
		t.Fatalf("unconfigured admin list must send nothing, got %v", got)
	}
}

// TestRejectNotifiesCandidate: the rejection body in notify.go was unreachable
// after the extraction; both reject actions now reach the candidate.
func TestRejectNotifiesCandidate(t *testing.T) {
	for _, action := range []string{"sponsor-reject", "reject"} {
		t.Run(action, func(t *testing.T) {
			s := newOnboardTestServer(t)
			h := s.Routes()
			tok := mintTokenSponsoredBy(t, s, "sponsor@example.org", "")
			pid := submit(t, h, tok, map[string]any{"name": "Pat Liaison", "email": "person-106@example.org", "organization": "Notify Org"})
			if action == "reject" {
				decideMachine(t, h, pid, "sponsor-approve")
			}
			if got := decideMachine(t, h, pid, action); got != onboardstore.ProposalRejected {
				t.Fatalf("%s: got %q", action, got)
			}
			if got := noticeTo(s, "onboard_email_rejected"); len(got) != 1 || got[0] != "person-106@example.org" {
				t.Fatalf("rejection notice recipients = %v, want [person-106@example.org]", got)
			}
		})
	}
}

// TestRejectionEmailCarriesReviewerNote: the candidate is told WHY when a note
// was left — "not approved" with no reason is what sends them to their sponsor
// to ask a question the email could have answered.
func TestRejectionEmailCarriesReviewerNote(t *testing.T) {
	s := newOnboardTestServer(t)
	_, body, ok := s.candidateEmail(notifyRequest{Event: "rejected", Email: "person-106@example.org",
		Name: "Pat Liaison", Env: "zagros", Reason: "no sponsor relationship on record"})
	if !ok {
		t.Fatal("rejected event must render")
	}
	if !strings.Contains(body, "no sponsor relationship on record") {
		t.Fatalf("rejection body dropped the reviewer note:\n%s", body)
	}
}

// TestSponsorSubmitEmailBody: the sponsor is told who, what, and what vouching
// means — the mail has to stand alone in an inbox.
func TestSponsorSubmitEmailBody(t *testing.T) {
	subject, body := sponsorSubmitEmail("zagros", "Dana Sponsor", "Pat Liaison", "pat-liaison",
		"person-106@example.org", true, "https://admin.auth.zagros.example/onboarding?status=proposed")
	if !strings.Contains(subject, "Pat Liaison") {
		t.Fatalf("subject must name the candidate: %q", subject)
	}
	for _, want := range []string{"Hello Dana Sponsor,", "pat-liaison", "person-106@example.org",
		"ESCALATED", "https://admin.auth.zagros.example/onboarding?status=proposed"} {
		if !strings.Contains(body, want) {
			t.Fatalf("sponsor body missing %q:\n%s", want, body)
		}
	}
	// No console base configured → words, never a link to nowhere.
	_, noLink := sponsorSubmitEmail("zagros", "", "Pat Liaison", "pat-liaison", "person-106@example.org", false, "")
	if strings.Contains(noLink, "http") {
		t.Fatalf("unconfigured console base must not fabricate a link:\n%s", noLink)
	}
}

func TestAdminVouchedEmailBody(t *testing.T) {
	subject, body := adminVouchedEmail("zagros", "Pat Liaison", "pat-liaison", "person-106@example.org",
		"sponsor@example.org", true, "https://admin.auth.zagros.example/onboarding?status=sponsor_approved")
	if !strings.Contains(subject, "ESCALATED") {
		t.Fatalf("escalated proposal must be flagged in the subject: %q", subject)
	}
	for _, want := range []string{"pat-liaison", "sponsor@example.org", "?status=sponsor_approved"} {
		if !strings.Contains(body, want) {
			t.Fatalf("admin body missing %q:\n%s", want, body)
		}
	}
}

// TestOnboardCounts: the badge poll target returns the two gate depths.
func TestOnboardCounts(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	seedProposal(t, s, "p1", "sponsor@example.org", "developer", onboardstore.ProposalProposed)
	seedProposal(t, s, "p2", "sponsor@example.org", "developer", onboardstore.ProposalProposed)
	seedProposal(t, s, "p3", "other@example.org", "developer", onboardstore.ProposalSponsorApproved)
	seedProposal(t, s, "p4", "other@example.org", "developer", onboardstore.ProposalOnboarded)

	counts := getCounts(t, h, nil)
	if counts["proposed"] != 2 || counts["sponsor_approved"] != 1 {
		t.Fatalf("admin counts = %v, want proposed 2 / sponsor_approved 1", counts)
	}
	if _, present := counts["onboarded"]; present {
		t.Fatalf("counts must carry the two gates only, got %v", counts)
	}
}

// TestOnboardCountsSponsorScoped: a sponsor session's badge must show its own
// queue depth. A count is a leak like any other read — telling a sponsor there
// are 40 candidates pending when they may see 2 discloses the other 38.
func TestOnboardCountsSponsorScoped(t *testing.T) {
	s := newOnboardTestServer(t)
	s.Cfg.DevBypassAuth = false
	portalURL, _ := fakeSponsorPortal(t, nil, map[string]sponsorLookupResponse{
		"counts-sponsor@example.org": {Org: "0ff404dd", Alias: "counts-sponsor", SponsorAuthorized: true, SponsorMaxTier: "developer"},
	})
	s.Cfg.PortalInternalURL = portalURL
	s.Cfg.PortalLookupToken = "svc-token"
	h := s.Routes()

	seedProposal(t, s, "own1", "counts-sponsor@example.org", "developer", onboardstore.ProposalProposed)
	seedProposal(t, s, "own2", "0ff404dd", "developer", onboardstore.ProposalSponsorApproved)
	seedProposal(t, s, "theirs1", "someone-else@example.org", "developer", onboardstore.ProposalProposed)
	seedProposal(t, s, "theirs2", "someone-else@example.org", "developer", onboardstore.ProposalProposed)

	counts := getCounts(t, h, map[string]string{
		"X-Forwarded-Email":  "counts-sponsor@example.org",
		"X-Forwarded-Groups": "DEVELOPER",
	})
	if counts["proposed"] != 1 || counts["sponsor_approved"] != 1 {
		t.Fatalf("sponsor-scoped counts = %v, want 1/1 (their own rows only)", counts)
	}
}

func getCounts(t *testing.T, h http.Handler, headers map[string]string) map[string]int {
	t.Helper()
	r := httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/api/onboard/proposals/counts", nil)
	for k, v := range headers {
		r.Header.Set(k, v)
	}
	w := httptest.NewRecorder()
	h.ServeHTTP(w, r)
	if w.Code != http.StatusOK {
		t.Fatalf("counts = %d: %s", w.Code, w.Body.String())
	}
	var out map[string]int
	if err := json.Unmarshal(w.Body.Bytes(), &out); err != nil {
		t.Fatalf("decode counts: %v", err)
	}
	return out
}

// TestQueueRowCarriesAge: the row renders the data the age indicator + the
// stale-runner marker are computed from. Without created_at on the row, a
// proposal that has waited a week is indistinguishable from a fresh one.
func TestQueueRowCarriesAge(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	seedProposal(t, s, "aged", "sponsor@example.org", "developer", onboardstore.ProposalApproved)

	w := httptest.NewRecorder()
	h.ServeHTTP(w, httptest.NewRequestWithContext(context.Background(), http.MethodGet,
		"/onboarding?status="+url.QueryEscape(onboardstore.ProposalApproved), nil))
	if w.Code != http.StatusOK {
		t.Fatalf("queue = %d", w.Code)
	}
	body := w.Body.String()
	if !strings.Contains(body, `class="ob-age" data-created="`) {
		t.Fatalf("row carries no created_at for the age indicator")
	}
	if !strings.Contains(body, `data-status="approved"`) {
		t.Fatalf("row carries no status — the stale-runner marker keys on it")
	}
	if !strings.Contains(body, "/static/onboard-queue.js") {
		t.Fatalf("queue page does not load the badge/age script")
	}
}
