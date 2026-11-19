package handlers

// onboard_draft_followups_test.go pins the review follow-ups to #908's draft
// completion: completion runs after the per-action status check and never
// before a refusal; a relationship declared at the admin gate survives the
// completion of a draft, and one the vocabulary refuses is refused BEFORE the
// draft completes; a submission that cannot form a handle is refused at submit
// instead of stored as a draft nothing can advance, and both the candidate and
// the admin are told the one thing that closes the gap; the stored
// submission decodes as tolerantly as the submit path decoded it; every
// completion writes one audit row naming who caused it; and one bad row does
// not stop the boot sweep.

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

// draftCompletionAudit returns the onboard_draft_complete row for proposal id,
// or fails the test when none was written.
func draftCompletionAudit(t *testing.T, s *Server, id string) audit.Event {
	t.Helper()
	for _, e := range s.Audit.Recent(50) {
		if e.Action == "onboard_draft_complete" && e.Target.InviteTokenPrefix == id {
			return e
		}
	}
	t.Fatalf("no onboard_draft_complete audit row for %s", id)
	return audit.Event{}
}

func hasDraftCompletionAudit(s *Server, id string) bool {
	for _, e := range s.Audit.Recent(50) {
		if e.Action == "onboard_draft_complete" && e.Target.InviteTokenPrefix == id {
			return true
		}
	}
	return false
}

// A draft that reaches the admin gate still incomplete is completed BEFORE the
// relationship is recorded, so the relationship the admin declared lands on the
// assembled manifest instead of being rebuilt away with the intake record.
func TestWebApprove_DraftCompletesBeforeRelationshipIsRecorded(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	id := storeDraft(t, s, onboardstore.Prebound{Tier: "developer"},
		map[string]any{"name": "Late Draft", "email": "late@example.test", "organization": "Draft Org", "sector": "commercial"})
	// Sponsor-approved while still a draft: the rules changed between the two
	// gates, or the row predates the completion path.
	if err := s.Onboard.DecideProposal(context.Background(), id, onboardstore.ProposalSponsorApproved, "sp@example.test", ""); err != nil {
		t.Fatalf("stage the draft at the admin gate: %v", err)
	}

	form := url.Values{}
	form.Set("csrf", s.csrfNonce)
	form.Set("relationship", "assigned")
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/onboarding/"+id+"/approve", strings.NewReader(form.Encode()))
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusSeeOther {
		t.Fatalf("web approve: want 303, got %d: %s", rec.Code, rec.Body.String())
	}

	p, m := loadProposal(t, s, id)
	if p.Status != onboardstore.ProposalApproved || !p.Complete {
		t.Fatalf("status=%q complete=%v, want approved and complete", p.Status, p.Complete)
	}
	if rel, _ := affField(m, "relationship"); rel != "assigned" {
		t.Fatalf("affiliation.relationship = %q, want assigned (declared at the gate)", rel)
	}
	if kind, _ := affField(m, "kind"); kind != "vendor" {
		t.Fatalf("affiliation.kind = %q, want vendor (the draft was assembled)", kind)
	}
}

// An email whose local part slugs to nothing, with no handle given, cannot
// become a manifest and "missing handle" is not review-editable. Refuse at
// submit with the link still usable, the way a blank tier is refused.
func TestSubmit_UnderivableHandleRefusedAndLinkSurvives(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	tok := mintPrebound(t, s, onboardstore.Prebound{Tier: "browse"})

	body, _ := json.Marshal(map[string]any{"token": tok, "submission": map[string]any{"name": "No Handle", "email": "!!!@example.test", "nationality": "CAN"}})
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/public/onboard/submit", strings.NewReader(string(body)))
	req.Header.Set("Content-Type", "application/json")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusUnprocessableEntity {
		t.Fatalf("underivable handle: want 422, got %d: %s", rec.Code, rec.Body.String())
	}
	var out struct {
		Error struct {
			Code   string            `json:"code"`
			Fields map[string]string `json:"fields"`
		} `json:"error"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &out); err != nil {
		t.Fatalf("decode validation_failed body: %v", err)
	}
	if out.Error.Code != "validation_failed" {
		t.Fatalf("underivable handle: want error.code=validation_failed, got %q", out.Error.Code)
	}
	if got := out.Error.Fields["handle"]; got != errHandleRequired.Error() {
		t.Fatalf("underivable handle: fields.handle = %q, want %q", got, errHandleRequired.Error())
	}
	// The intake page has no handle input, so "choose one" is a dead end; the
	// address is the one thing the candidate can change.
	if body := rec.Body.String(); !strings.Contains(body, "use a different address") || strings.Contains(body, "choose one") {
		t.Fatalf("underivable handle: the message must name the address as the remedy, got %s", body)
	}
	if rows, err := s.Onboard.ListProposals(context.Background(), onboardstore.ProposalProposed); err != nil || len(rows) != 0 {
		t.Fatalf("a refused submit must store nothing, got %d rows (err=%v)", len(rows), err)
	}
	// A handle in the JSON is not honoured (the form renders no handle input,
	// GHSA-mxf4-8rv7-774w), so it does not rescue the address; the remedy the
	// message names, a different address, does. Same link, still unspent.
	body2, _ := json.Marshal(map[string]any{"token": tok, "submission": map[string]any{"name": "No Handle", "email": "!!!@example.test", "organization": "Handle Org", "nationality": "CAN", "handle": "no-handle"}})
	req2 := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/public/onboard/submit", strings.NewReader(string(body2)))
	req2.Header.Set("Content-Type", "application/json")
	rec2 := httptest.NewRecorder()
	h.ServeHTTP(rec2, req2)
	if rec2.Code != http.StatusUnprocessableEntity {
		t.Fatalf("a JSON handle must not rescue an underivable address: want 422, got %d: %s", rec2.Code, rec2.Body.String())
	}
	pid := submit(t, h, tok, map[string]any{"name": "No Handle", "email": "no.handle@example.test", "organization": "Handle Org", "nationality": "CAN"})
	if p, _ := loadProposal(t, s, pid); !p.Complete {
		t.Fatalf("submit with a usable address after refusal must be complete, note=%q", p.Note)
	}
}

// A relationship outside the closed vocabulary is refused BEFORE the draft
// completes: an approve refused for relationship=partner leaves the draft a
// draft, with no completion persisted and no completion audit row, exactly as
// it did when the value was validated ahead of decideProposal.
func TestWebApprove_InvalidRelationshipRefusedBeforeCompletion(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	id := storeDraft(t, s, onboardstore.Prebound{Tier: "developer"},
		map[string]any{"name": "Partner Draft", "email": "partner@example.test", "sector": "commercial"})
	if err := s.Onboard.DecideProposal(context.Background(), id, onboardstore.ProposalSponsorApproved, "sp@example.test", ""); err != nil {
		t.Fatalf("stage the draft at the admin gate: %v", err)
	}

	form := url.Values{}
	form.Set("csrf", s.csrfNonce)
	form.Set("relationship", "partner")
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/onboarding/"+id+"/approve", strings.NewReader(form.Encode()))
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusBadRequest {
		t.Fatalf("approve with relationship=partner: want 400, got %d: %s", rec.Code, rec.Body.String())
	}

	p, _ := loadProposal(t, s, id)
	if p.Status != onboardstore.ProposalSponsorApproved {
		t.Fatalf("status = %q, want sponsor_approved (the approval was refused)", p.Status)
	}
	if p.Complete {
		t.Fatal("a refused relationship must not leave a completed draft behind")
	}
	if hasDraftCompletionAudit(s, id) {
		t.Fatal("a refused relationship must not write a completion audit row")
	}
}

// A stored draft whose email cannot form a handle (rows from before submit
// refused them) is refused at the gate with the remedy an admin can act on.
// Prebound has no handle field, so "set by the sponsor" would be a dead end;
// the candidate has to come back under a different address.
func TestDecide_UnderivableHandleDraftNamesTheRemedy(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	id := storeDraft(t, s, onboardstore.Prebound{Tier: "developer"},
		map[string]any{"name": "No Handle Draft", "email": "!!!@example.test", "sector": "commercial"})

	got := postMachine(t, h, "/api/onboard/proposals/"+id+"/sponsor-approve", map[string]any{})
	if got.code != http.StatusBadRequest {
		t.Fatalf("sponsor-approve on an underivable-handle draft: want 400, got %d: %s", got.code, got.body)
	}
	if !strings.Contains(got.body, "missing handle") || !strings.Contains(got.body, "different address") {
		t.Fatalf("refusal must name the gap and the remedy, got %s", got.body)
	}
	for _, dead := range []string{"choose one", "set by the sponsor"} {
		if strings.Contains(got.body, dead) {
			t.Fatalf("refusal points at a remedy nobody can act on (%q): %s", dead, got.body)
		}
	}
	if p, _ := loadProposal(t, s, id); p.Complete {
		t.Fatal("a blocked draft must stay a draft")
	}
}

// A stored draft with no organisation name (rows that predate the public
// form requiring one) cannot complete under the current rules; the sponsor
// gate refuses it, naming the missing organisation, and the row stays a
// draft that an admin must either reject or correct.
func TestDecide_OrglessDraftStaysBlockedAndNamesTheGap(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	id := storeDraft(t, s, onboardstore.Prebound{Tier: "developer"},
		map[string]any{"name": "Orgless Draft", "email": "orgless@example.test", "sector": "commercial"})

	got := postMachine(t, h, "/api/onboard/proposals/"+id+"/sponsor-approve", map[string]any{})
	if got.code != http.StatusBadRequest {
		t.Fatalf("sponsor-approve on an org-less draft: want 400, got %d: %s", got.code, got.body)
	}
	if !strings.Contains(got.body, "missing organization") {
		t.Fatalf("refusal must name the missing organisation, got %s", got.body)
	}
	if p, _ := loadProposal(t, s, id); p.Complete {
		t.Fatal("a draft blocked for missing organisation must stay incomplete")
	}
}

// The submit path decodes a submission tolerantly (a wrongly typed field is
// skipped, the rest is kept). Completion decodes the stored copy the same way,
// so a submission that was accepted never becomes "unreadable" later.
func TestDecide_DraftWithMistypedFieldStillCompletes(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	id := storeDraft(t, s, onboardstore.Prebound{Tier: "developer"},
		map[string]any{"name": "Quota String", "email": "quota@example.test", "organization": "Draft Org", "sector": "commercial", "machine_quota": "5"})

	if got := decideMachine(t, h, id, "sponsor-approve"); got != onboardstore.ProposalSponsorApproved {
		t.Fatalf("sponsor-approve: got %q", got)
	}
	if p, _ := loadProposal(t, s, id); !p.Complete {
		t.Fatalf("draft with a mistyped field must complete, note=%q", p.Note)
	}
}

// A decision the status check refuses never attempts completion: a rejected
// draft answers 409, not the 500 a zero-row completion update produces.
func TestDecide_RejectedDraftAnswers409(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	id := storeDraft(t, s, onboardstore.Prebound{Tier: "developer"},
		map[string]any{"name": "Gone Draft", "email": "gone@example.test", "organization": "Draft Org", "sector": "commercial"})
	if got := decideMachine(t, h, id, "reject"); got != onboardstore.ProposalRejected {
		t.Fatalf("reject: got %q", got)
	}

	got := postMachine(t, h, "/api/onboard/proposals/"+id+"/sponsor-approve", map[string]any{})
	if got.code != http.StatusConflict {
		t.Fatalf("sponsor-approve on a rejected draft: want 409, got %d: %s", got.code, got.body)
	}
	if p, _ := loadProposal(t, s, id); p.Complete {
		t.Fatal("a refused decision must not persist completion")
	}
}

// An out-of-order approve (sponsor gate skipped) is refused before anything is
// written: the draft stays a draft and no completion audit row exists.
func TestDecide_OutOfOrderApproveLeavesTheDraftUntouched(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	id := storeDraft(t, s, onboardstore.Prebound{Tier: "developer"},
		map[string]any{"name": "Early Draft", "email": "early@example.test", "organization": "Draft Org", "sector": "commercial"})

	got := postMachine(t, h, "/api/onboard/proposals/"+id+"/approve", map[string]any{})
	if got.code != http.StatusConflict {
		t.Fatalf("approve on a proposed draft: want 409, got %d: %s", got.code, got.body)
	}
	if p, _ := loadProposal(t, s, id); p.Complete {
		t.Fatal("a refused decision must not persist completion")
	}
	if hasDraftCompletionAudit(s, id) {
		t.Fatal("a refused decision must not write a completion audit row")
	}
}

// On the machine realm the browser actor is empty; the completion row names
// the resolved operator handle instead of a blank.
func TestDecide_DraftCompletionAuditNamesTheMachineActor(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	id := storeDraft(t, s, onboardstore.Prebound{Tier: "developer"},
		map[string]any{"name": "Machine Draft", "email": "machine@example.test", "organization": "Draft Org", "sector": "commercial"})

	if got := decideMachine(t, h, id, "sponsor-approve"); got != onboardstore.ProposalSponsorApproved {
		t.Fatalf("sponsor-approve: got %q", got)
	}
	e := draftCompletionAudit(t, s, id)
	if e.Actor.Email != "onboard-runner" {
		t.Fatalf("completion audit actor = %q, want the operator handle onboard-runner", e.Actor.Email)
	}
	if e.Result != "ok" {
		t.Fatalf("completion audit result = %q, want ok", e.Result)
	}
}

// A field edit that completes a draft is a state change on its own and writes
// the completion row before the edit row, with the editing admin as actor.
func TestFieldEdit_DraftCompletionAudit(t *testing.T) {
	s := newOnboardTestServer(t)
	id := storeDraft(t, s, onboardstore.Prebound{Tier: "developer"},
		map[string]any{"name": "Edited Draft", "email": "edited@example.test", "organization": "Draft Org", "sector": "commercial"})

	r := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/api/onboard/proposals/"+id+"/fields", nil)
	r = r.WithContext(context.WithValue(r.Context(), ctxKeyActor, Actor{Email: "editor@example.test"}))
	if _, ferr := s.applyFieldEdits(r, id, map[string]string{FieldIdentityGitHub: "edited-person"}); ferr != nil {
		t.Fatalf("field edit on completable draft refused: %v", ferr)
	}
	e := draftCompletionAudit(t, s, id)
	if e.Actor.Email != "editor@example.test" {
		t.Fatalf("completion audit actor = %q, want the editing admin", e.Actor.Email)
	}
}

// The boot sweep audits each completion under the console's own actor label.
func TestCompleteDrafts_SweepAuditsEachCompletion(t *testing.T) {
	s := newOnboardTestServer(t)
	id := storeDraft(t, s, onboardstore.Prebound{Tier: "browse"},
		map[string]any{"name": "Boot Draft", "email": "boot@example.test", "organization": "Draft Org", "sector": "civil-society"})

	if _, _, err := s.CompleteDrafts(context.Background()); err != nil {
		t.Fatalf("CompleteDrafts: %v", err)
	}
	e := draftCompletionAudit(t, s, id)
	if e.Actor.Email != "console-onboard" {
		t.Fatalf("sweep completion audit actor = %q, want console-onboard", e.Actor.Email)
	}
}

// One row that fails hard (here: rejected after the sweep listed it, so the
// guarded completion update matches nothing) is reported, and every later row
// is still completed.
func TestSweepDrafts_OneBadRowDoesNotStopTheRest(t *testing.T) {
	s := newOnboardTestServer(t)
	ctx := context.Background()
	bad := storeDraft(t, s, onboardstore.Prebound{Tier: "developer"},
		map[string]any{"name": "Bad Row", "email": "bad@example.test", "organization": "Draft Org", "sector": "commercial"})
	good := storeDraft(t, s, onboardstore.Prebound{Tier: "browse"},
		map[string]any{"name": "Good Row", "email": "good@example.test", "organization": "Draft Org", "sector": "civil-society"})
	badRow, _ := loadProposal(t, s, bad)
	goodRow, _ := loadProposal(t, s, good)
	if err := s.Onboard.DecideProposal(ctx, bad, onboardstore.ProposalRejected, "x", ""); err != nil {
		t.Fatalf("reject the bad row: %v", err)
	}

	done, left, err := s.sweepDrafts(ctx, []onboardstore.Proposal{*badRow, *goodRow})
	if err == nil || !strings.Contains(err.Error(), bad) {
		t.Fatalf("sweep must report the failed row %s, got err=%v", bad, err)
	}
	if done != 1 || left != 0 {
		t.Fatalf("sweep = (%d done, %d left), want (1, 0): the failed row is in the error, not the counts", done, left)
	}
	if p, _ := loadProposal(t, s, good); !p.Complete {
		t.Fatalf("the row after the failure must still complete, note=%q", p.Note)
	}
}
