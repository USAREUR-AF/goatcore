package handlers

import (
	"bytes"
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// TestMintInvite_BlankEnvRefused: a blank env yields a blank-env proposal a scoped
// onboard-runner silently skips forever — so the mint must fail closed, not create
// an un-runnable invite. (Well-oiled-machine strand.)
func TestMintInvite_BlankEnvRefused(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()

	// No env in the body.
	got := postMachine(t, h, "/api/onboard/invite", map[string]any{"tier": "developer"})
	if got.code != http.StatusBadRequest {
		t.Fatalf("blank-env mint: want 400, got %d: %s", got.code, got.body)
	}
	var body struct {
		Error struct {
			Code string `json:"code"`
		} `json:"error"`
	}
	_ = json.Unmarshal([]byte(got.body), &body)
	if body.Error.Code != "env_required" {
		t.Fatalf("blank-env mint: want error.code=env_required, got %q (%s)", body.Error.Code, got.body)
	}
	// A refusal that sets no operator detail must stay byte-identical to the
	// pre-detail envelope (omitempty) — the portal and collector read this
	// exact shape.
	if strings.Contains(got.body, `"detail"`) {
		t.Fatalf("refusal without operator detail must not carry a detail key: %s", got.body)
	}

	// A concrete env still mints.
	ok := postMachine(t, h, "/api/onboard/invite", map[string]any{"env": "zagros", "tier": "developer"})
	if ok.code != http.StatusCreated {
		t.Fatalf("env mint: want 201, got %d: %s", ok.code, ok.body)
	}
}

// TestDecide_IncompleteDraftCannotAdvance: an incomplete draft may be rejected but
// never sponsor-approved/approved — advancing it lets the runner claim it, then the
// native mint hard-fails on the unusable manifest with a misleading error.
func TestDecide_IncompleteDraftCannotAdvance(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()

	mk := func(complete bool, status string) string {
		p := &onboardstore.Proposal{
			Sponsor: "dev@bypass.local", Env: "zagros", Handle: "draftee",
			Email: "person-36@example.org", Status: status, ManifestJSON: "{}", Complete: complete,
		}
		if err := s.Onboard.CreateProposal(context.Background(), p); err != nil {
			t.Fatalf("create proposal: %v", err)
		}
		return p.ID
	}

	// sponsor-approve on an incomplete proposed draft → 400.
	inc := mk(false, onboardstore.ProposalProposed)
	if got := postMachine(t, h, "/api/onboard/proposals/"+inc+"/sponsor-approve", map[string]any{}); got.code != http.StatusBadRequest {
		t.Fatalf("incomplete sponsor-approve: want 400, got %d: %s", got.code, got.body)
	}
	// But reject still works on the incomplete draft.
	if got := postMachine(t, h, "/api/onboard/proposals/"+inc+"/sponsor-reject", map[string]any{}); got.code != http.StatusOK {
		t.Fatalf("incomplete sponsor-reject: want 200, got %d: %s", got.code, got.body)
	}

	// approve on an incomplete sponsor_approved draft → 400 (the misleading-mint gate).
	inc2 := mk(false, onboardstore.ProposalSponsorApproved)
	if got := postMachine(t, h, "/api/onboard/proposals/"+inc2+"/approve", map[string]any{}); got.code != http.StatusBadRequest {
		t.Fatalf("incomplete approve: want 400, got %d: %s", got.code, got.body)
	}
}

// TestSponsorReject_WithdrawsAVouchUntilTheAdminDecides: a sponsor may withdraw
// a vouch while the proposal awaits the admin (ADR 1207), and not after.
func TestSponsorReject_WithdrawsAVouchUntilTheAdminDecides(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	ctx := context.Background()
	for _, tc := range []struct {
		status string
		want   int
	}{
		{onboardstore.ProposalProposed, http.StatusOK},
		{onboardstore.ProposalSponsorApproved, http.StatusOK},
		{onboardstore.ProposalApproved, http.StatusConflict},
		{onboardstore.ProposalRejected, http.StatusConflict},
	} {
		p := &onboardstore.Proposal{
			Sponsor: "dev@bypass.local", Env: "zagros", Handle: "withdrawee",
			Email: tc.status + "@ex.gov", Status: tc.status, ManifestJSON: "{}", Complete: true,
		}
		if err := s.Onboard.CreateProposal(ctx, p); err != nil {
			t.Fatalf("create proposal: %v", err)
		}
		id := p.ID
		got := postMachine(t, h, "/api/onboard/proposals/"+id+"/sponsor-reject", map[string]any{})
		if got.code != tc.want {
			t.Fatalf("sponsor-reject from %s: want %d, got %d: %s", tc.status, tc.want, got.code, got.body)
		}
	}
}

// TestSubmit_DuplicateEmailRefusedBeforeConsumingInvite: a second submission for an
// email already in-flight is refused (409) — and the second invite token is NOT
// burned, so a genuine mistake is recoverable.
func TestSubmit_DuplicateEmailRefusedBeforeConsumingInvite(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()

	tok1 := mintTokenViaStore(t, s)
	sub := map[string]any{"name": "Pat Liaison", "email": "person-106@example.org", "organization": "Duplicate Org", "nationality": "CAN"}
	if pid := submit(t, h, tok1, sub); pid == "" {
		t.Fatal("first submit: empty proposal id")
	}

	// Second, different token, same email → 409 duplicate_email.
	tok2 := mintTokenViaStore(t, s)
	if code := submitRaw(t, h, tok2, sub); code != http.StatusConflict {
		t.Fatalf("duplicate-email submit: want 409, got %d", code)
	}
	// The second token must still be OPEN (not consumed) — a different email
	// completes on it, proving the duplicate guard ran before ConsumeInvite.
	if pid := submit(t, h, tok2, map[string]any{"name": "Other Person", "email": "person-73@example.org", "organization": "Duplicate Org", "nationality": "CAN"}); pid == "" {
		t.Fatal("second token should still be usable for a non-duplicate email")
	}
}

// After a proposal has been fully onboarded and then explicitly offboarded,
// the duplicate-email guard must permit a fresh proposal for the same email
// — the offboarded row is terminal-negative, not an active blocker.
func TestReenrollAfterOffboardedAllowsNewProposal(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()

	// Initial proposal submitted and fully onboarded.
	tok1 := mintTokenViaStore(t, s)
	sub := map[string]any{"name": "Pat Liaison", "email": "person-106@example.org", "organization": "Duplicate Org", "nationality": "CAN"}
	pid := submit(t, h, tok1, sub)
	if pid == "" {
		t.Fatal("first submit: empty proposal id")
	}
	if got := decideMachine(t, h, pid, "sponsor-approve"); got != onboardstore.ProposalSponsorApproved {
		t.Fatalf("sponsor-approve: got %q", got)
	}
	if got := decideMachine(t, h, pid, "approve"); got != onboardstore.ProposalApproved {
		t.Fatalf("approve: got %q", got)
	}
	claim := postMachine(t, h, "/api/onboard/proposals/"+pid+"/claim", nil)
	if claim.code != http.StatusOK {
		t.Fatalf("claim: want 200, got %d: %s", claim.code, claim.body)
	}
	res := postMachine(t, h, "/api/onboard/proposals/"+pid+"/result", map[string]any{"status": "onboarded"})
	if res.code != http.StatusOK {
		t.Fatalf("result: want 200, got %d: %s", res.code, res.body)
	}

	// Offboard the onboarded proposal via the machine realm.
	off := postMachine(t, h, "/api/onboard/proposals/"+pid+"/offboard", map[string]any{})
	if off.code != http.StatusOK {
		t.Fatalf("offboard: want 200, got %d: %s", off.code, off.body)
	}
	var ob struct {
		Status string `json:"status"`
	}
	if err := json.Unmarshal([]byte(off.body), &ob); err != nil {
		t.Fatalf("decode offboard response: %v", err)
	}
	if ob.Status != onboardstore.ProposalOffboarded {
		t.Fatalf("offboard status = %q, want %q", ob.Status, onboardstore.ProposalOffboarded)
	}

	// A fresh proposal for the same email must now be accepted, not rejected
	// as a duplicate.
	tok2 := mintTokenViaStore(t, s)
	if pid2 := submit(t, h, tok2, sub); pid2 == "" {
		t.Fatal("re-enrollment after offboarded: submit returned empty proposal id")
	}
}

// F-279: a bad affiliation.nation (a full country name, not an ISO alpha-3
// code) is refused at submit with a clear error — BEFORE the invite is
// consumed — instead of being stored and only dying at the admin approve gate.
func TestSubmit_InvalidNationRefusedBeforeConsumingInvite(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()

	tok := mintTokenViaStore(t, s)
	bad := map[string]any{"name": "Matt C", "email": "person-57@example.org", "organization": "Nation Org", "nation": "Canada", "nationality": "CAN"}
	if code := submitRaw(t, h, tok, bad); code != http.StatusUnprocessableEntity {
		t.Fatalf("invalid nation submit: want 422, got %d", code)
	}
	// The token must still be OPEN (rejected before ConsumeInvite): a valid
	// ISO 3166-1 alpha-3 code completes on the same token.
	ok := map[string]any{"name": "Matt C", "email": "person-57@example.org", "organization": "Nation Org", "nation": "CAN", "nationality": "CAN"}
	if pid := submit(t, h, tok, ok); pid == "" {
		t.Fatal("valid nation (CAN) should complete on the still-open token")
	}
}

// Offboard must refuse proposals that never reached the provisioning leg
// (pre-onboarding states and other non-eligible statuses). Only onboarded,
// enrolled, and archived proposals are eligible sources.
func TestOffboard_NonEligibleStatesYieldConflict(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	ctx := context.Background()

	for _, status := range []string{
		onboardstore.ProposalProposed,
		onboardstore.ProposalSponsorApproved,
		onboardstore.ProposalApproved,
		onboardstore.ProposalOnboarding,
		onboardstore.ProposalFailed,
		onboardstore.ProposalRejected,
	} {
		t.Run(status, func(t *testing.T) {
			p := &onboardstore.Proposal{
				Sponsor:      "dev@bypass.local",
				Env:          "zagros",
				Handle:       "pat-" + status,
				Email:        status + "@partner.mil",
				Status:       status,
				ManifestJSON: "{}",
			}
			if err := s.Onboard.CreateProposal(ctx, p); err != nil {
				t.Fatalf("create proposal (%s): %v", status, err)
			}

			got := postMachine(t, h, "/api/onboard/proposals/"+p.ID+"/offboard", map[string]any{}) //nolint:contextcheck // test helper builds its own request context; ctx above only seeds the store row
			if got.code != http.StatusConflict {
				t.Fatalf("offboard of %s proposal: want 409, got %d: %s", status, got.code, got.body)
			}
		})
	}
}

// Offboard must accept proposals in the eligible post-provisioning states
// (onboarded, enrolled, archived), record the transition to offboarded, and
// release the duplicate-email guard. Audit emission for the console decide
// path is covered by TestBrowserOffboard_EligibleStates.
func TestOffboard_EligibleStatesTransitionAndReleaseEmailGuard(t *testing.T) {
	ctx := context.Background()

	tests := []struct {
		name string
		body map[string]any // archived → offboarded is one-way and refuses a blank note
		prep func(t *testing.T, s *Server, p *onboardstore.Proposal)
	}{
		{
			name: "onboarded",
			body: map[string]any{},
			prep: func(t *testing.T, s *Server, p *onboardstore.Proposal) {
				if err := s.Onboard.CreateProposal(ctx, p); err != nil {
					t.Fatalf("create proposal (onboarded): %v", err)
				}
			},
		},
		{
			name: "enrolled",
			body: map[string]any{},
			prep: func(t *testing.T, s *Server, p *onboardstore.Proposal) {
				// Start from onboarded and let the store's MarkEnrolled guard
				// perform the transition realistically.
				if err := s.Onboard.CreateProposal(ctx, p); err != nil {
					t.Fatalf("create proposal (onboarded→enrolled): %v", err)
				}
				if ok, err := s.Onboard.MarkEnrolled(ctx, p.ID); err != nil || !ok {
					t.Fatalf("MarkEnrolled: ok=%v err=%v", ok, err)
				}
			},
		},
		{
			name: "archived",
			body: map[string]any{"note": "sweep closed"},
			prep: func(t *testing.T, s *Server, p *onboardstore.Proposal) {
				// ArchiveProposal enforces the onboarded→archived guard; drive
				// through it to mirror the purge lane.
				if err := s.Onboard.CreateProposal(ctx, p); err != nil {
					t.Fatalf("create proposal (onboarded→archived): %v", err)
				}
				if ok, err := s.Onboard.ArchiveProposal(ctx, p.ID, "sweep"); err != nil || !ok {
					t.Fatalf("ArchiveProposal: ok=%v err=%v", ok, err)
				}
			},
		},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			s := newOnboardTestServer(t)
			h := s.Routes()
			ctx := context.Background()

			email := "pat-" + tc.name + "@partner.mil"
			p := &onboardstore.Proposal{
				Sponsor:      "dev@bypass.local",
				Env:          "zagros",
				Handle:       "pat-" + tc.name,
				Email:        email,
				Status:       onboardstore.ProposalOnboarded,
				ManifestJSON: "{}",
			}

			// Prepare the starting state for this case.
			tc.prep(t, s, p)
			// Sanity: before offboard, the proposal must be considered active
			// by the duplicate-email guard.
			if got, ok, err := s.Onboard.ActiveProposalForEmail(ctx, email); err != nil {
				t.Fatalf("ActiveProposalForEmail(before %s): %v", tc.name, err)
			} else if !ok || got.ID != p.ID {
				t.Fatalf("ActiveProposalForEmail(before %s): got ok=%v id=%q (want id=%q)", tc.name, ok, got.ID, p.ID)
			}

			resp := postMachine(t, h, "/api/onboard/proposals/"+p.ID+"/offboard", tc.body)
			if resp.code != http.StatusOK {
				t.Fatalf("offboard from %s: want 200, got %d: %s", tc.name, resp.code, resp.body)
			}
			var body struct {
				Status string `json:"status"`
			}
			if err := json.Unmarshal([]byte(resp.body), &body); err != nil {
				t.Fatalf("decode offboard response (%s): %v", tc.name, err)
			}
			if body.Status != onboardstore.ProposalOffboarded {
				t.Fatalf("offboard status from %s = %q, want %q", tc.name, body.Status, onboardstore.ProposalOffboarded)
			}

			// After offboard, the duplicate-email guard must report no active
			// proposal for this email.
			if got, ok, err := s.Onboard.ActiveProposalForEmail(ctx, email); err != nil {
				t.Fatalf("ActiveProposalForEmail(after %s): %v", tc.name, err)
			} else if ok {
				t.Fatalf("ActiveProposalForEmail(after %s): want no active proposal, got %+v", tc.name, got)
			}
		})
	}
}

// The offboard machine API is operator-only; a call without a resolved
// operator principal (no Authorization header) must fail as 401.
func TestOffboard_RequiresOperatorPrincipal(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()

	p := &onboardstore.Proposal{
		Sponsor:      "dev@bypass.local",
		Env:          "zagros",
		Handle:       "pat-liaison",
		Email:        "person-106@example.org",
		Status:       onboardstore.ProposalOnboarded,
		ManifestJSON: "{}",
	}
	if err := s.Onboard.CreateProposal(context.Background(), p); err != nil {
		t.Fatalf("create proposal: %v", err)
	}

	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/api/onboard/proposals/"+p.ID+"/offboard", bytes.NewReader([]byte(`{"note":"x"}`)))
	req.Header.Set("Content-Type", "application/json")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusUnauthorized {
		t.Fatalf("offboard without operator principal: want 401, got %d: %s", rec.Code, rec.Body.String())
	}
}

// The offboard machine API accepts an absent optional note, rejects malformed
// JSON, and truncates oversized notes before they reach the store.
func TestOffboard_DecodesOptionalNoteConsistently(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	ctx := context.Background()

	// Seed three onboarded proposals so each request shape has its own row.
	mk := func(id string) *onboardstore.Proposal {
		p := &onboardstore.Proposal{
			Sponsor:      "dev@bypass.local",
			Env:          "zagros",
			Handle:       id,
			Email:        id + "@partner.mil",
			Status:       onboardstore.ProposalOnboarded,
			ManifestJSON: "{}",
		}
		if err := s.Onboard.CreateProposal(ctx, p); err != nil {
			t.Fatalf("create proposal %s: %v", id, err)
		}
		return p
	}

	pBad := mk("bad-json")
	pEmpty := mk("empty-body")
	pBig := mk("big-note")

	// Bad JSON is a client error and must not change the proposal.
	reqBad := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/api/onboard/proposals/"+pBad.ID+"/offboard", strings.NewReader("{not-json"))
	reqBad.Header.Set("Authorization", "Bearer runner-key")
	reqBad.Header.Set("Content-Type", "application/json")
	recBad := httptest.NewRecorder()
	h.ServeHTTP(recBad, reqBad)
	if recBad.Code != http.StatusBadRequest {
		t.Fatalf("offboard with bad JSON: want 400, got %d: %s", recBad.Code, recBad.Body.String())
	}
	if !strings.Contains(recBad.Body.String(), `"code":"bad_request"`) {
		t.Fatalf("offboard with bad JSON: missing bad_request code: %s", recBad.Body.String())
	}
	if got, err := s.Onboard.GetProposal(ctx, pBad.ID); err != nil {
		t.Fatalf("reload proposal after bad-JSON offboard: %v", err)
	} else if got.Status != onboardstore.ProposalOnboarded {
		t.Fatalf("bad-JSON offboard changed status to %q", got.Status)
	}

	// An empty body is the valid representation of an omitted optional note.
	reqEmpty := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/api/onboard/proposals/"+pEmpty.ID+"/offboard", nil)
	reqEmpty.Header.Set("Authorization", "Bearer runner-key")
	recEmpty := httptest.NewRecorder()
	h.ServeHTTP(recEmpty, reqEmpty)
	if recEmpty.Code != http.StatusOK {
		t.Fatalf("offboard with empty body: want 200, got %d: %s", recEmpty.Code, recEmpty.Body.String())
	}
	if got, err := s.Onboard.GetProposal(ctx, pEmpty.ID); err != nil {
		t.Fatalf("reload proposal after empty-body offboard: %v", err)
	} else if got.Status != onboardstore.ProposalOffboarded {
		t.Fatalf("empty-body offboard: status=%q, want %q", got.Status, onboardstore.ProposalOffboarded)
	}

	// Very large note: offboard still 200s, and the note is truncated to the
	// decision-note cap rather than written to the store whole.
	bigNote := strings.Repeat("x", 1<<15)
	body, _ := json.Marshal(map[string]any{"note": bigNote})
	reqBig := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/api/onboard/proposals/"+pBig.ID+"/offboard", bytes.NewReader(body))
	reqBig.Header.Set("Authorization", "Bearer runner-key")
	reqBig.Header.Set("Content-Type", "application/json")
	recBig := httptest.NewRecorder()
	h.ServeHTTP(recBig, reqBig)
	if recBig.Code != http.StatusOK {
		t.Fatalf("offboard with large note: want 200, got %d: %s", recBig.Code, recBig.Body.String())
	}
	if got, err := s.Onboard.GetProposal(ctx, pBig.ID); err != nil {
		t.Fatalf("reload proposal after large-note offboard: %v", err)
	} else {
		if got.Status != onboardstore.ProposalOffboarded {
			t.Fatalf("large-note offboard: status=%q, want %q", got.Status, onboardstore.ProposalOffboarded)
		}
		if want := strings.Repeat("x", maxDecisionNoteBytes); got.Note != want {
			t.Fatalf("large-note offboard: note bytes=%d, want truncated to %d", len(got.Note), maxDecisionNoteBytes)
		}
	}
}

// handleOnboardingDecide (UI realm) must enforce CSRF on the new offboard
// action just like the existing approve/reject actions.
func TestHandleOnboardingDecide_OffboardRequiresCSRF(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()

	p := &onboardstore.Proposal{
		Sponsor:      "dev@bypass.local",
		Env:          "zagros",
		Handle:       "pat-liaison",
		Email:        "person-106@example.org",
		Status:       onboardstore.ProposalOnboarded,
		ManifestJSON: "{}",
	}
	if err := s.Onboard.CreateProposal(context.Background(), p); err != nil {
		t.Fatalf("create proposal: %v", err)
	}

	// Missing/incorrect csrf field ⇒ 400.
	form := url.Values{}
	form.Set("note", "test")
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/onboarding/"+p.ID+"/offboard", strings.NewReader(form.Encode()))
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusBadRequest {
		t.Fatalf("offboard UI without csrf: want 400, got %d: %s", rec.Code, rec.Body.String())
	}
}

// If another operator has already offboarded the proposal (status no longer
// onboarded), a UI offboard submit must handle the Conflict from the
// state machine gracefully and surface an inline message instead of a
// full-page error.
func TestHandleOnboardingDecide_OffboardRaceReportsConflict(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()

	p := &onboardstore.Proposal{
		Sponsor:      "dev@bypass.local",
		Env:          "zagros",
		Handle:       "pat-liaison",
		Email:        "person-106@example.org",
		Status:       onboardstore.ProposalOffboarded,
		ManifestJSON: "{}",
	}
	if err := s.Onboard.CreateProposal(context.Background(), p); err != nil {
		t.Fatalf("create proposal: %v", err)
	}

	form := url.Values{}
	form.Set("csrf", s.csrfNonce)
	form.Set("note", "test")
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/onboarding/"+p.ID+"/offboard", strings.NewReader(form.Encode()))
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusSeeOther {
		t.Fatalf("offboard UI race: want 303 redirect, got %d: %s", rec.Code, rec.Body.String())
	}
	loc := rec.Header().Get("Location")
	if !strings.HasPrefix(loc, "/onboarding?") || !strings.Contains(loc, "flash=") {
		t.Fatalf("offboard UI race: redirect location must carry flash, got %q", loc)
	}
	// Pin WHICH flash: "carries some flash" also passes when an earlier guard
	// short-circuits for an unrelated reason, which is how a later pre-flight
	// check silently masked this case before it was ordered behind the state
	// guard. The race must report the already-offboarded conflict.
	if !strings.Contains(loc, "flash="+flashOffboardAlready) {
		t.Fatalf("offboard UI race: want flash=%s, got %q", flashOffboardAlready, loc)
	}
}

// The onboarding queue is reachable by URL, so the flash banner must render
// only server-authored text keyed by a known code. A caller-supplied string
// — the shape of a link mailed to an operator — must produce no banner at
// all, so nobody can put their own words inside the trusted console chrome.
func TestHandleOnboarding_FlashRendersOnlyKnownCodes(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()

	const injected = "Your session expired, call 555-0100 to re-verify"
	req := httptest.NewRequestWithContext(context.Background(), http.MethodGet,
		"/onboarding?flash="+url.QueryEscape(injected)+"&flash_level=error", nil)
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("onboarding page: want 200, got %d", rec.Code)
	}
	if strings.Contains(rec.Body.String(), "555-0100") {
		t.Fatal("caller-supplied flash text was rendered into the console page")
	}

	// A known code still renders its own server-side wording.
	req = httptest.NewRequestWithContext(context.Background(), http.MethodGet,
		"/onboarding?flash="+flashOffboardAlready, nil)
	rec = httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if want := onboardFlashes[flashOffboardAlready].Message; !strings.Contains(rec.Body.String(), want) {
		t.Fatalf("known flash code did not render its message %q", want)
	}
}

// Excluding offboarded rows from ActiveProposalForEmail has a second,
// non-obvious consequence: sshKeyHandleFor resolves a signed-in email to a
// handle through that same query, so an offboarded person's handle stops
// resolving and self-service key requests refuse rather than attaching a key
// to a handle whose access was already cut. That is the intended direction
// (refuse, never guess), and it is behavior this change introduced, so it is
// pinned here rather than left to be rediscovered.
func TestSSHKeyHandleFor_OffboardedNoLongerResolves(t *testing.T) {
	s := newOnboardTestServer(t)
	ctx := context.Background()

	p := &onboardstore.Proposal{
		Sponsor:      "dev@bypass.local",
		Env:          "zagros",
		Handle:       "dana-signals",
		Email:        "person-27@example.org",
		Status:       onboardstore.ProposalOnboarded,
		ManifestJSON: "{}",
	}
	if err := s.Onboard.CreateProposal(ctx, p); err != nil {
		t.Fatalf("create proposal: %v", err)
	}

	req := httptest.NewRequestWithContext(ctx, http.MethodGet, "/ssh-keys", nil)
	handle, err := s.sshKeyHandleFor(req, "person-27@example.org")
	if err != nil || handle != "dana-signals" {
		t.Fatalf("while onboarded: want handle dana-signals, got %q (err %v)", handle, err)
	}

	if err := s.Onboard.DecideProposal(ctx, p.ID, onboardstore.ProposalOffboarded, "op@bypass.local", ""); err != nil {
		t.Fatalf("offboard: %v", err)
	}

	handle, err = s.sshKeyHandleFor(req, "person-27@example.org")
	if err == nil {
		t.Fatalf("after offboard: want a refusal, got handle %q", handle)
	}
	if handle != "" {
		t.Fatalf("after offboard: a refusal must return no handle, got %q", handle)
	}
}

// TestSubmit_DuplicateEmail409NamesBlockingProposal: the duplicate-email 409
// must name the proposal that blocks the email — id + status in an
// operator-facing `detail` — while the candidate-facing `message` stays
// unchanged. Before this, the refusal named nothing: the only way to find the
// one blocking row was SSH + sqlite3 against onboard.db on the management
// host (#334; the worked case was a months-old `archived` row that survived
// identity deletion, invite revocation, and peer checks).
func TestSubmit_DuplicateEmail409NamesBlockingProposal(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()

	// The blocker: an archived proposal — active to the guard, invisible to
	// every surface the operator would reasonably check first.
	p := &onboardstore.Proposal{
		Sponsor: "dev@bypass.local", Env: "zagros", Handle: "pat",
		Email: "person-106@example.org", Status: onboardstore.ProposalArchived,
		ManifestJSON: "{}", Complete: true,
	}
	if err := s.Onboard.CreateProposal(context.Background(), p); err != nil {
		t.Fatalf("create blocking proposal: %v", err)
	}

	tok := mintTokenViaStore(t, s)
	body, _ := json.Marshal(map[string]any{
		"token":      tok,
		"submission": map[string]any{"name": "Pat Liaison", "email": "person-106@example.org", "organization": "Duplicate Org", "nationality": "CAN"},
	})
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/public/onboard/submit", bytes.NewReader(body))
	req.Header.Set("Content-Type", "application/json")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusConflict {
		t.Fatalf("want 409, got %d: %s", rec.Code, rec.Body.String())
	}

	var out struct {
		Error struct {
			Code    string `json:"code"`
			Message string `json:"message"`
			Detail  string `json:"detail"`
		} `json:"error"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &out); err != nil {
		t.Fatalf("decode 409 body: %v", err)
	}

	// Candidate-facing copy is untouched (onboard.html renders error.message).
	if out.Error.Code != "duplicate_email" {
		t.Fatalf("want error.code=duplicate_email, got %q", out.Error.Code)
	}
	if want := "an onboarding for this email is already in progress or complete — contact your operator if that's not you"; out.Error.Message != want {
		t.Fatalf("candidate-facing message changed:\n want %q\n got  %q", want, out.Error.Message)
	}

	// Operator-facing detail names the blocker.
	if want := "blocked by proposal " + p.ID + " (status: archived)"; out.Error.Detail != want {
		t.Fatalf("detail: want %q, got %q", want, out.Error.Detail)
	}
}

// A PERSON's nationality is required at submit: shared data lanes
// are narrowed by the nationality of the reader, so a request without one
// cannot be reviewed. Refused BEFORE the one-time invite is consumed, so the
// candidate can reopen the same link and add it. A system has no citizenship
// and submits without one.
func TestSubmit_BlankNationalityRefusedBeforeConsumingInvite(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	tok := mintTokenViaStore(t, s)

	person := map[string]any{"name": "Pat Liaison", "email": "person-106@example.org", "organization": "BlankNat Org"}
	if code := submitRaw(t, h, tok, person); code != http.StatusUnprocessableEntity {
		t.Fatalf("person without nationality: want 422, got %d", code)
	}
	// Same link, now with the nationality: the invite was not burned.
	person["nationality"] = "CAN"
	if pid := submit(t, h, tok, person); pid == "" {
		t.Fatal("submit with nationality after the refusal returned no proposal id")
	}

	tok2 := mintTokenViaStore(t, s)
	system := map[string]any{"name": "Radar 21", "email": "person-108@example.org", "organization": "Radar Org", "principal_type": "system"}
	if pid := submit(t, h, tok2, system); pid == "" {
		t.Fatal("system without nationality should be accepted")
	}
}
func TestSubmit_BlankOrganizationRefusedBeforeConsumingInvite(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	tok := mintTokenViaStore(t, s)

	person := map[string]any{"name": "Pat Liaison", "email": "person-104@example.org", "nationality": "CAN"}
	code := submitRaw(t, h, tok, person)
	if code != http.StatusUnprocessableEntity {
		t.Fatalf("person without organization: want 422, got %d", code)
	}

	w := httptest.NewRecorder()
	reqBody, _ := json.Marshal(map[string]any{"token": tok, "submission": person})
	req := httptest.NewRequestWithContext(context.Background(), "POST", "/public/onboard/submit", bytes.NewReader(reqBody))
	req.Header.Set("Content-Type", "application/json")
	h.ServeHTTP(w, req)

	var res map[string]onboardAPIError
	if err := json.Unmarshal(w.Body.Bytes(), &res); err != nil {
		t.Fatalf("unmarshal submit response: %v", err)
	}
	if _, ok := res["error"].Fields["organization"]; !ok {
		t.Errorf("expected error.fields.organization, got %v", res["error"].Fields)
	}

	person["organization"] = "Pat Org"
	if pid := submit(t, h, tok, person); pid == "" {
		t.Fatal("person with organization should complete on the still-open token")
	}
}

// TestOnboardCapabilities_AdvertisesSponsorWithdraw: the intake smoke reads this
// before vouching, so a console carrying ADR 1207 must say so without a key.
func TestOnboardCapabilities_AdvertisesSponsorWithdraw(t *testing.T) {
	s := newOnboardTestServer(t)
	req := httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/api/onboard/capabilities", nil)
	rec := httptest.NewRecorder()
	s.Routes().ServeHTTP(rec, req)
	if rec.Code != http.StatusOK || !strings.Contains(rec.Body.String(), `"sponsor_withdraw":true`) {
		t.Fatalf("capabilities: want 200 with sponsor_withdraw true, got %d: %s", rec.Code, rec.Body.String())
	}
}
