package handlers

import (
	"context"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// The Archived card's "Restore and resend key" button (#1037): the browser
// caller of the guarded store write the operator API already used. These
// tests pin the contract the issue names — one primary button first, the
// confirm's facts, the redirect to the row in Approved with the banner, the
// refusal on a row no longer archived, the same audit event as the API with
// the session identity as actor, and no button for a sponsor session.

func postBrowserRestore(t *testing.T, h http.Handler, s *Server, pid string) *httptest.ResponseRecorder {
	t.Helper()
	form := url.Values{}
	form.Set("csrf", s.csrfNonce)
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/onboarding/"+pid+"/restore", strings.NewReader(form.Encode()))
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	req.Header.Set("X-Forwarded-Groups", "PLATFORM-ADMIN")
	req.Header.Set("X-Forwarded-Email", "admin@example.org")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	return rec
}

func adminGet(t *testing.T, h http.Handler, path string) string {
	t.Helper()
	req := httptest.NewRequestWithContext(context.Background(), http.MethodGet, path, nil)
	req.Header.Set("X-Forwarded-Groups", "PLATFORM-ADMIN")
	req.Header.Set("X-Forwarded-Email", "admin@example.org")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("GET %s: want 200, got %d: %s", path, rec.Code, rec.Body.String())
	}
	return rec.Body.String()
}

func TestBrowserRestore_ArchivedCardOrderAndWording(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	pid := onboardedProposalInState(t, s, h, "pat-restore@example.org", onboardstore.ProposalArchived)
	body := adminGet(t, h, "/onboarding?status=archived&id="+pid)

	restore := strings.Index(body, "/onboarding/"+pid+"/restore")
	offboard := strings.Index(body, "/onboarding/"+pid+"/offboard")
	if restore < 0 || offboard < 0 {
		t.Fatalf("archived card must carry both forms (restore at %d, offboard at %d)", restore, offboard)
	}
	if restore > offboard {
		t.Error("Restore must be the FIRST action on the card; Mark offboarded stays below it")
	}
	for _, want := range []string{
		"<h3>Archived · key never used</h3>",
		`<button class="primary" type="submit" aria-label="Restore pat-restore and resend key">Restore and resend key</button>`,
		// The confirm names the goatnet and the destination email, and says the
		// old key is dead and a fresh one is minted + emailed.
		"The old setup key is dead. A fresh one will be minted on zagros and the welcome package emailed to pat-restore@example.org.",
		"Offboarding an archived record is one-way and refuses until the person's access is actually cut.",
	} {
		if !strings.Contains(body, want) {
			t.Errorf("archived card lacks %q", want)
		}
	}
	for _, gone := range []string{"operator API", "/api/onboard/proposals/", "goat admin onboard pat-restore --env zagros --revoke)."} {
		if strings.Contains(body, gone) {
			t.Errorf("archived card still carries %q — the hand-written POST and the CLI command are gone from the note", gone)
		}
	}
}

func TestBrowserRestore_LandsOnApprovedWithBannerAndMark(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	pid := onboardedProposalInState(t, s, h, "pat-back@example.org", onboardstore.ProposalArchived)

	rec := postBrowserRestore(t, h, s, pid)
	if rec.Code != http.StatusSeeOther {
		t.Fatalf("want 303, got %d: %s", rec.Code, rec.Body.String())
	}
	loc, _ := url.Parse(rec.Header().Get("Location"))
	if loc.Path != "/onboarding" || loc.Query().Get("status") != "approved" || loc.Query().Get("id") != pid || loc.Query().Get("flash") != flashRestored {
		t.Fatalf("redirect = %q, want the row in Approved with the restored flash", rec.Header().Get("Location"))
	}
	if got := statusOf(t, s, pid); got != onboardstore.ProposalApproved {
		t.Fatalf("status after restore = %q, want approved", got)
	}
	p, _ := s.Onboard.GetProposal(context.Background(), pid)
	if p.RestoredBy != "admin@example.org" || p.RestoredAt == 0 {
		t.Errorf("restore mark = (%q, %d), want the session identity and a time", p.RestoredBy, p.RestoredAt)
	}

	body := adminGet(t, h, loc.String())
	if !strings.Contains(body, "Restored. A fresh key will be minted and the welcome package sent to pat-back@example.org. Follow this row through Provisioning to Onboarded.") {
		t.Error("landing page lacks the restored banner naming the destination email")
	}
	if !strings.Contains(body, "<b>Restored</b> by admin@example.org at ") {
		t.Error("Approved card lacks the 'Restored by X at T' mark")
	}
	// The mark follows the row through Provisioning and Onboarded.
	ctx := context.Background()
	if err := s.Onboard.ClaimProposal(ctx, pid); err != nil {
		t.Fatalf("claim: %v", err)
	}
	if !strings.Contains(adminGet(t, h, "/onboarding?status=onboarding&id="+pid), "<b>Restored</b> by admin@example.org") {
		t.Error("Provisioning card lacks the restore mark")
	}
	if err := s.Onboard.SetProposalResult(ctx, pid, onboardstore.ProposalOnboarded, "nbs-secret-key-value", "", "", "", "", false); err != nil {
		t.Fatalf("onboarded: %v", err)
	}
	onb := adminGet(t, h, "/onboarding?status=onboarded&id="+pid)
	if !strings.Contains(onb, "<b>Restored</b> by admin@example.org") {
		t.Error("Onboarded card lacks the restore mark")
	}

	// Audit: the SAME event name the operator API emits, actor = the session.
	found := false
	for _, e := range s.Audit.Recent(50) {
		if e.Action == "onboard_restore_proposal" && e.Result == "ok" {
			found = true
			if e.Actor.Email != "admin@example.org" {
				t.Errorf("audit actor = %q, want the session identity", e.Actor.Email)
			}
			if e.Before["status"] != onboardstore.ProposalArchived || e.After["status"] != onboardstore.ProposalApproved || e.After["proposal_id"] != pid {
				t.Errorf("audit before/after = %v / %v", e.Before, e.After)
			}
		}
	}
	if !found {
		t.Error("no onboard_restore_proposal audit event with result ok")
	}
}

// The setup key value never appears on the row, in a flash or in the confirm.
func TestBrowserRestore_KeyNeverInline(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	const secret = "nbs-inline-secret-key" //nolint:gosec // a fake value the test looks for in page output, not a credential
	pid := onboardedProposalInState(t, s, h, "pat-key@example.org", onboardstore.ProposalOnboarded)
	ctx := context.Background()
	if err := s.Onboard.SetProposalResult(ctx, pid, onboardstore.ProposalOnboarded, secret, "", "", "", "", false); err != nil {
		t.Fatalf("set key: %v", err)
	}
	if _, err := s.Onboard.ArchiveProposal(ctx, pid, "dormant"); err != nil {
		t.Fatalf("archive: %v", err)
	}
	// The confirm() text is the one new place a value could leak: pin the
	// whole restore form. (The page's identity-card JSON is the full store
	// row and predates this change; it is not the row, the flash or the
	// confirm.)
	body := adminGet(t, h, "/onboarding?status=archived&id="+pid)
	i := strings.Index(body, `action="/onboarding/`+pid+`/restore"`)
	if i < 0 {
		t.Fatal("restore form not found")
	}
	j := strings.Index(body[i:], "</form>")
	if j < 0 {
		t.Fatal("restore form not closed")
	}
	if form := body[i : i+j]; strings.Contains(form, secret) {
		t.Fatalf("restore form / confirm carries the setup key: %s", form)
	}
	rec := postBrowserRestore(t, h, s, pid)
	loc := rec.Header().Get("Location")
	if strings.Contains(loc, secret) {
		t.Fatal("redirect carries the setup key")
	}
	if strings.Contains(adminGet(t, h, loc), secret) {
		t.Fatal("post-restore page renders the setup key value")
	}
}

func TestBrowserRestore_RefusedWhenNoLongerArchived(t *testing.T) {
	for _, state := range []string{onboardstore.ProposalOnboarded, onboardstore.ProposalEnrolled, onboardstore.ProposalApproved} {
		t.Run(state, func(t *testing.T) {
			s := newOnboardTestServer(t)
			h := s.Routes()
			var pid string
			if state == onboardstore.ProposalApproved {
				// Restored by somebody else meanwhile: archived, then restored via
				// the API, then the browser's stale form arrives.
				pid = onboardedProposalInState(t, s, h, "pat-race@example.org", onboardstore.ProposalArchived)
				if r := postMachine(t, h, "/api/onboard/proposals/"+pid+"/restore", nil); r.code != http.StatusOK {
					t.Fatalf("api restore: %d %s", r.code, r.body)
				}
			} else {
				pid = onboardedProposalInState(t, s, h, "pat-race@example.org", state)
			}
			rec := postBrowserRestore(t, h, s, pid)
			if rec.Code != http.StatusSeeOther {
				t.Fatalf("want 303, got %d: %s", rec.Code, rec.Body.String())
			}
			loc, _ := url.Parse(rec.Header().Get("Location"))
			if loc.Query().Get("flash") != flashRestoreState || loc.Query().Get("status") != state || loc.Query().Get("id") != pid {
				t.Fatalf("redirect = %q, want the row in its current stage with the refusal flash", loc)
			}
			if got := statusOf(t, s, pid); got != state {
				t.Fatalf("row changed: %q, want %q", got, state)
			}
			body := adminGet(t, h, loc.String())
			if !strings.Contains(body, "Not restored: this record is no longer archived — it is in "+onboardStageLabel(state)+" now") {
				t.Errorf("refusal banner does not name the current stage %q", onboardStageLabel(state))
			}
			refused := 0
			for _, e := range s.Audit.Recent(50) {
				if e.Action != "onboard_restore_proposal" || e.Actor.Email != "admin@example.org" {
					continue
				}
				if e.Result != "refused" {
					t.Errorf("browser refusal audited as %q, want refused", e.Result)
					continue
				}
				refused++
			}
			if refused != 1 {
				t.Errorf("want one refused onboard_restore_proposal event from the browser, got %d", refused)
			}
		})
	}
}

func TestBrowserRestore_CSRFAndUnknownRow(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	pid := onboardedProposalInState(t, s, h, "pat-csrf@example.org", onboardstore.ProposalArchived)

	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/onboarding/"+pid+"/restore", strings.NewReader("csrf=wrong"))
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	req.Header.Set("X-Forwarded-Groups", "PLATFORM-ADMIN")
	req.Header.Set("X-Forwarded-Email", "admin@example.org")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusBadRequest {
		t.Fatalf("bad csrf: want 400, got %d", rec.Code)
	}
	if got := statusOf(t, s, pid); got != onboardstore.ProposalArchived {
		t.Fatalf("bad csrf changed the row to %q", got)
	}
	if rec := postBrowserRestore(t, h, s, "prop-does-not-exist"); rec.Code != http.StatusNotFound {
		t.Fatalf("unknown row: want 404, got %d", rec.Code)
	}
}

// Sponsor sessions: no button on the card, and the route refuses them even
// with a valid CSRF token. Restore is an admin decision (approve weight).
func TestBrowserRestore_SponsorSeesNoButtonAndIsRefused(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	pid := onboardedProposalInState(t, s, h, "pat-sp@example.org", onboardstore.ProposalArchived)
	sp := &sponsorSession{Email: "dev@bypass.local", Prefix: "dev@bypass.local"}
	withSponsor := func(r *http.Request) *http.Request {
		ctx := context.WithValue(r.Context(), ctxKeyActor, Actor{Email: sp.Email})
		ctx = context.WithValue(ctx, ctxKeySponsor, sp)
		return r.WithContext(ctx)
	}

	// The invitee IS this sponsor's (Sponsor = dev@bypass.local via the store
	// mint), so the row renders for them — without the button.
	rec := httptest.NewRecorder()
	s.handleOnboarding(rec, withSponsor(httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/onboarding?status=archived&id="+pid, nil)))
	if rec.Code != http.StatusOK {
		t.Fatalf("sponsor GET: want 200, got %d: %s", rec.Code, rec.Body.String())
	}
	if strings.Contains(rec.Body.String(), "/onboarding/"+pid+"/restore") {
		t.Error("sponsor session sees the Restore button")
	}

	form := url.Values{}
	form.Set("csrf", s.csrfNonce)
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/onboarding/"+pid+"/restore", strings.NewReader(form.Encode()))
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	req.SetPathValue("id", pid)
	rec = httptest.NewRecorder()
	s.handleOnboardingRestore(rec, withSponsor(req))
	if rec.Code != http.StatusForbidden {
		t.Fatalf("sponsor POST: want 403, got %d: %s", rec.Code, rec.Body.String())
	}
	if got := statusOf(t, s, pid); got != onboardstore.ProposalArchived {
		t.Fatalf("sponsor POST changed the row to %q", got)
	}
}

// The Invited tab's folded "Archived" section is dead links, not people; the
// note says so and points at the stage where people are restored.
func TestBrowserRestore_InvitedArchiveNoteDisambiguates(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	body := adminGet(t, h, "/onboarding?status=invited")
	if !strings.Contains(body, "These are dead invite links, not people: a link here cannot be restored") {
		t.Error("Invited tab's archived section lacks the not-restorable note")
	}
	if !strings.Contains(body, `<a href="/onboarding?status=archived">Archived</a> stage tab instead, and can be restored from there`) {
		t.Error("Invited tab's archived note does not point at the Archived stage")
	}
}
