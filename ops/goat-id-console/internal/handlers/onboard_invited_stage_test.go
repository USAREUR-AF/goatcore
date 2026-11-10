package handlers

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// seedInvite plants one invite row via the real store paths (mint, then revoke
// or consume for the non-open states), returning its id.
func seedInvite(t *testing.T, s *Server, sponsor, note string, expiresAt int64, status string) string {
	t.Helper()
	ctx := context.Background()
	inv := &onboardstore.Invite{
		Sponsor: sponsor, SponsorName: sponsor, Env: "zagros",
		ExpiresAt: expiresAt, Note: note,
	}
	token, err := s.Onboard.MintInvite(ctx, inv)
	if err != nil {
		t.Fatalf("seed invite: %v", err)
	}
	switch status {
	case onboardstore.InviteRevoked:
		if err := s.Onboard.RevokeInvite(ctx, inv.ID); err != nil {
			t.Fatalf("seed revoke: %v", err)
		}
	case onboardstore.InviteSubmitted:
		_, err := s.Onboard.ConsumeInvite(ctx, token, func(i *onboardstore.Invite, pid string) (*onboardstore.Proposal, error) {
			return &onboardstore.Proposal{Handle: "cand-" + i.ID, Email: i.ID + "@example.org", ManifestJSON: "{}", Complete: true}, nil
		})
		if err != nil {
			t.Fatalf("seed consume: %v", err)
		}
	}
	return inv.ID
}

// The Invited tab lists open invites (newest state first), folds a past-TTL
// 'invited' row to expired for display, excludes submitted invites (they live
// on as proposals), and counts only OPEN invites on the tab badge.
func TestOnboardingInvitedTab(t *testing.T) {
	s := newOnboardTestServer(t)
	s.Cfg.InviteArchiveAfter = 7 * 24 * time.Hour
	h := s.Routes()
	future := time.Now().Add(72 * time.Hour).Unix()

	openID := seedInvite(t, s, "op@example.org", "romania cohort", future, "")
	expiredID := seedInvite(t, s, "op@example.org", "lapsed one", time.Now().Add(-time.Hour).Unix(), "")
	revokedID := seedInvite(t, s, "op@example.org", "killed one", future, onboardstore.InviteRevoked)
	submittedID := seedInvite(t, s, "op@example.org", "already submitted", future, onboardstore.InviteSubmitted)

	req := httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/onboarding?status=invited", nil)
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("invited tab = %d: %s", rec.Code, rec.Body.String())
	}
	body := rec.Body.String()
	for _, want := range []string{openID, expiredID, revokedID, "romania cohort", "expired", "revoked"} {
		if !strings.Contains(body, want) {
			t.Fatalf("invited tab missing %q", want)
		}
	}
	revStart := strings.Index(body, "killed one")
	if revStart == -1 {
		t.Fatalf("revoked invite row not found in invited tab body:\n%s", body)
	}
	revEnd := strings.Index(body[revStart:], "</a>")
	if revEnd == -1 {
		t.Fatalf("revoked invite row closing tag not found in invited tab body:\n%s", body)
	}
	row := body[revStart : revStart+revEnd]
	if strings.Contains(row, "revoked in") {
		t.Fatalf("revoked badge must not render future tense 'revoked in ...':\n%s", row)
	}
	// The invite row itself is excluded. Its id may still appear on the page:
	// the proposal it became is in the cross-stage find index (#1306), and this
	// seed builds that proposal's handle and email from the invite id.
	if strings.Contains(body, "&id="+submittedID+`"`) {
		t.Fatalf("invited tab must exclude a submitted invite (it lives on as a proposal)")
	}
	// Tab badge counts only the one OPEN invite.
	if !strings.Contains(body, `Invited<span class="c">1</span>`) {
		t.Fatalf("invited tab count != 1:\n%s", body)
	}

	// Detail pane: the open invite offers Revoke; the revoked one does not.
	rec = httptest.NewRecorder()
	h.ServeHTTP(rec, httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/onboarding?status=invited&id="+openID, nil))
	if rec.Code != http.StatusOK || !strings.Contains(rec.Body.String(), "/onboarding/invite/"+openID+"/revoke") {
		t.Fatalf("open invite detail lacks revoke action (%d)", rec.Code)
	}
	rec = httptest.NewRecorder()
	h.ServeHTTP(rec, httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/onboarding?status=invited&id="+revokedID, nil))
	if rec.Code != http.StatusOK || strings.Contains(rec.Body.String(), "/onboarding/invite/"+revokedID+"/revoke") {
		t.Fatalf("revoked invite detail must not offer revoke (%d)", rec.Code)
	}

	// The proposal tabs still carry the invited count so the funnel head is
	// visible from anywhere in the queue.
	rec = httptest.NewRecorder()
	h.ServeHTTP(rec, httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/onboarding", nil))
	if rec.Code != http.StatusOK || !strings.Contains(rec.Body.String(), `Invited<span class="c">1</span>`) {
		t.Fatalf("default tab lost the invited count (%d)", rec.Code)
	}
}

func TestOnboardingInvitedTab_MalformedMetadataDegradesOnlyThatRow(t *testing.T) {
	s := newOnboardTestServer(t)
	s.Cfg.InviteArchiveAfter = 7 * 24 * time.Hour
	h := s.Routes()
	future := time.Now().Add(72 * time.Hour).Unix()

	validID := seedInvite(t, s, "op@example.org", "valid invite", future, "")
	invalidOpenID := seedInvite(t, s, "op@example.org", "invalid open invite", future, "")
	invalidRevokedID := seedInvite(t, s, "op@example.org", "invalid revoked invite", future, onboardstore.InviteRevoked)
	invalidArchivedID := seedInvite(t, s, "op@example.org", "invalid archived invite", future, onboardstore.InviteRevoked)
	if _, err := s.Onboard.DB().ExecContext(context.Background(),
		`UPDATE onboard_invites SET prebound_json = $1 WHERE id = $2`,
		`{"tier":"developer"}`, validID); err != nil {
		t.Fatalf("seed valid metadata: %v", err)
	}
	for _, id := range []string{invalidOpenID, invalidRevokedID, invalidArchivedID} {
		if _, err := s.Onboard.DB().ExecContext(context.Background(),
			`UPDATE onboard_invites SET prebound_json = $1 WHERE id = $2`,
			`{"tier":"iam-admin"`, id); err != nil {
			t.Fatalf("seed malformed metadata for %s: %v", id, err)
		}
	}
	if _, err := s.Onboard.DB().ExecContext(context.Background(),
		`UPDATE onboard_invites SET status = $1, archived_at = $2 WHERE id = $3`,
		onboardstore.InviteArchived, time.Now().Unix(), invalidArchivedID); err != nil {
		t.Fatalf("archive malformed invite: %v", err)
	}

	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, httptest.NewRequestWithContext(context.Background(), http.MethodGet,
		"/onboarding?status=invited", nil))
	if rec.Code != http.StatusOK {
		t.Fatalf("invited tab = %d, want 200: %s", rec.Code, rec.Body.String())
	}
	body := rec.Body.String()
	for _, id := range []string{validID, invalidOpenID, invalidRevokedID, invalidArchivedID} {
		if !strings.Contains(body, id) {
			t.Fatalf("invited tab must retain invite %s", id)
		}
	}
	rowFor := func(note string) string {
		t.Helper()
		start := strings.Index(body, note)
		if start < 0 {
			t.Fatalf("invite row %q not found: %s", note, body)
		}
		end := strings.Index(body[start:], "</a>")
		if end < 0 {
			t.Fatalf("invite row %q closing tag not found: %s", note, body)
		}
		return body[start : start+end]
	}
	openRow := rowFor("invalid open invite")
	for _, want := range []string{"expires", "access metadata unreadable — revoke and re-invite"} {
		if !strings.Contains(openRow, want) {
			t.Fatalf("malformed open row missing %q: %s", want, openRow)
		}
	}
	if strings.Contains(openRow, "ESCALATED") || strings.Contains(openRow, "iam-admin") {
		t.Fatalf("malformed open row must not infer a tier: %s", openRow)
	}
	revokedRow := rowFor("invalid revoked invite")
	if !strings.Contains(revokedRow, "revoked") || !strings.Contains(revokedRow, "mint a fresh invite to re-nominate") {
		t.Fatalf("malformed revoked row lost lifecycle or remediation: %s", revokedRow)
	}
	if strings.Contains(revokedRow, "revoke and re-invite") {
		t.Fatalf("malformed revoked row must not advise another revoke: %s", revokedRow)
	}
	archiveStart := strings.Index(body, `<details class="ob-archive"`)
	archivedRow := strings.Index(body, "invalid archived invite")
	if archiveStart < 0 || archivedRow < archiveStart {
		t.Fatalf("malformed archived row must render inside the Archived fold")
	}
	if row := rowFor("invalid archived invite"); !strings.Contains(row, "archived") || !strings.Contains(row, "mint a fresh invite to re-nominate") {
		t.Fatalf("malformed archived row lost lifecycle or remediation: %s", row)
	}

	apiReq := httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/api/onboard/invites", nil)
	apiReq.Header.Set("Authorization", "Bearer runner-key")
	apiRec := httptest.NewRecorder()
	h.ServeHTTP(apiRec, apiReq)
	if apiRec.Code != http.StatusOK {
		t.Fatalf("invite API = %d, want 200: %s", apiRec.Code, apiRec.Body.String())
	}
	var invites []onboardstore.Invite
	if err := json.Unmarshal(apiRec.Body.Bytes(), &invites); err != nil {
		t.Fatalf("decode invite API: %v", err)
	}
	if len(invites) != 4 {
		t.Fatalf("invite API rows = %d, want 4: %+v", len(invites), invites)
	}
	seenValid := false
	seenInvalid := map[string]bool{invalidOpenID: false, invalidRevokedID: false, invalidArchivedID: false}
	for _, inv := range invites {
		switch inv.ID {
		case validID:
			seenValid = true
			if inv.MetadataInvalid {
				t.Errorf("valid invite marked invalid")
			}
		case invalidOpenID, invalidRevokedID, invalidArchivedID:
			seenInvalid[inv.ID] = true
			if !inv.MetadataInvalid || inv.Escalated {
				t.Errorf("invalid invite flags = metadata_invalid:%v escalated:%v", inv.MetadataInvalid, inv.Escalated)
			}
		}
	}
	if !seenValid || !seenInvalid[invalidOpenID] || !seenInvalid[invalidRevokedID] || !seenInvalid[invalidArchivedID] {
		t.Fatalf("invite API omitted a row: valid=%v invalid=%v", seenValid, seenInvalid)
	}

	detailRec := httptest.NewRecorder()
	h.ServeHTTP(detailRec, httptest.NewRequestWithContext(context.Background(), http.MethodGet,
		"/onboarding?status=invited&id="+invalidOpenID, nil))
	if detailRec.Code != http.StatusOK {
		t.Fatalf("malformed open detail = %d, want 200: %s", detailRec.Code, detailRec.Body.String())
	}
	detail := detailRec.Body.String()
	for _, want := range []string{"access metadata unreadable — revoke and re-invite", "/onboarding/invite/" + invalidOpenID + "/revoke"} {
		if !strings.Contains(detail, want) {
			t.Fatalf("malformed open detail missing %q: %s", want, detail)
		}
	}
	for _, absent := range []string{"Pre-bound fields", "unrestricted", "ESCALATED", "iam-admin"} {
		if strings.Contains(detail, absent) {
			t.Fatalf("malformed open detail must not contain %q: %s", absent, detail)
		}
	}

	form := url.Values{"csrf": {s.csrfNonce}}
	revokeReq := httptest.NewRequestWithContext(context.Background(), http.MethodPost,
		"/onboarding/invite/"+invalidOpenID+"/revoke", strings.NewReader(form.Encode()))
	revokeReq.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	revokeRec := httptest.NewRecorder()
	h.ServeHTTP(revokeRec, revokeReq)
	if revokeRec.Code != http.StatusSeeOther {
		t.Fatalf("malformed invite revoke = %d, want 303: %s", revokeRec.Code, revokeRec.Body.String())
	}
	if inv, err := s.Onboard.GetInvite(context.Background(), invalidOpenID); err != nil || inv.Status != onboardstore.InviteRevoked {
		t.Fatalf("malformed invite status after revoke = %v (%v), want revoked", inv, err)
	}

	deadRec := httptest.NewRecorder()
	h.ServeHTTP(deadRec, httptest.NewRequestWithContext(context.Background(), http.MethodGet,
		"/onboarding?status=invited&id="+invalidRevokedID, nil))
	if deadRec.Code != http.StatusOK {
		t.Fatalf("malformed revoked detail = %d, want 200: %s", deadRec.Code, deadRec.Body.String())
	}
	deadDetail := deadRec.Body.String()
	if !strings.Contains(deadDetail, "mint a fresh invite to re-nominate") || strings.Contains(deadDetail, "/onboarding/invite/"+invalidRevokedID+"/revoke") {
		t.Fatalf("malformed revoked detail has wrong remediation or action: %s", deadDetail)
	}
	for _, absent := range []string{"Pre-bound fields", "unrestricted", "ESCALATED", "iam-admin"} {
		if strings.Contains(deadDetail, absent) {
			t.Fatalf("malformed revoked detail must not contain %q: %s", absent, deadDetail)
		}
	}
}

// Legacy revoked invites — rows revoked before the revoked_at column existed
// — fall back to the TTL for archiving. Their badge must not claim the link
// will be revoked in N days.
func TestOnboardingInvitedTab_LegacyRevokedBadgeNoFutureTense(t *testing.T) {
	s := newOnboardTestServer(t)
	s.Cfg.InviteArchiveAfter = 7 * 24 * time.Hour
	h := s.Routes()
	future := time.Now().Add(20 * 24 * time.Hour).Unix()

	revokedID := seedInvite(t, s, "op@example.org", "legacy revoked", future, onboardstore.InviteRevoked)
	if _, err := s.Onboard.DB().Exec("UPDATE onboard_invites SET revoked_at = NULL WHERE id = $1", revokedID); err != nil {
		t.Fatalf("clear revoked_at: %v", err)
	}

	req := httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/onboarding?status=invited", nil)
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("invited tab (legacy revoked) = %d: %s", rec.Code, rec.Body.String())
	}
	body := rec.Body.String()
	revStart := strings.Index(body, "legacy revoked")
	if revStart == -1 {
		t.Fatalf("legacy revoked invite row not found in invited tab body:\n%s", body)
	}
	revEnd := strings.Index(body[revStart:], "</a>")
	if revEnd == -1 {
		t.Fatalf("legacy revoked invite row closing tag not found in invited tab body:\n%s", body)
	}
	row := body[revStart : revStart+revEnd]
	if strings.Contains(row, "revoked in") {
		t.Fatalf("legacy revoked badge must not render future tense 'revoked in ...':\n%s", row)
	}

	rec = httptest.NewRecorder()
	req = httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/onboarding?status=invited&id="+revokedID, nil)
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("legacy revoked detail = %d: %s", rec.Code, rec.Body.String())
	}
	detail := rec.Body.String()
	start := strings.Index(detail, `<div class="ob-detail">`)
	end := strings.Index(detail, `<h4 class="ob-kv-head">`)
	if start == -1 || end == -1 || end <= start {
		t.Fatalf("legacy revoked detail markup did not contain expected detail block:\n%s", detail)
	}
	panel := detail[start:end]
	if strings.Contains(panel, "after revocation") || strings.Contains(panel, "for 7 days") {
		t.Fatalf("legacy revoked detail must not claim a revocation-anchored archive window:\n%s", panel)
	}
	if !strings.Contains(detail, "archives in") {
		t.Fatalf("legacy revoked page must still show an archive window badge (archives in ...):\n%s", detail)
	}
}

// The Archived section at the bottom of the Invited tab is always rendered,
// even when no invites have been archived yet, so an operator can see where
// dead links will go.
func TestOnboardingInvitedTab_ArchivedSectionRendersWhenEmpty(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()

	req := httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/onboarding?status=invited", nil)
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("invited tab = %d: %s", rec.Code, rec.Body.String())
	}
	body := rec.Body.String()
	if !strings.Contains(body, `<details class="ob-archive">`) {
		t.Fatalf("invited tab must render the Archived section even when empty")
	}
	if !strings.Contains(body, "Archived (0)") {
		t.Fatalf("invited tab Archived summary must show a zero count when empty:\n%s", body)
	}
}

// Web revoke: CSRF-gated, redirects back to the Invited tab, flips the store
// row, and refuses a second revoke (the row is no longer open).
func TestOnboardingInviteWebRevoke(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	id := seedInvite(t, s, "op@example.org", "", time.Now().Add(time.Hour).Unix(), "")

	post := func(withCSRF bool) *httptest.ResponseRecorder {
		form := url.Values{}
		if withCSRF {
			form.Set("csrf", s.csrfNonce)
		}
		r := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/onboarding/invite/"+id+"/revoke", strings.NewReader(form.Encode()))
		r.Header.Set("Content-Type", "application/x-www-form-urlencoded")
		w := httptest.NewRecorder()
		h.ServeHTTP(w, r)
		return w
	}

	if w := post(false); w.Code != http.StatusBadRequest {
		t.Fatalf("no-CSRF revoke = %d, want 400", w.Code)
	}
	if w := post(true); w.Code != http.StatusSeeOther || w.Header().Get("Location") != "/onboarding?status=invited" {
		t.Fatalf("revoke = %d loc=%q, want 303 to the Invited tab", w.Code, w.Header().Get("Location"))
	}
	inv, err := s.Onboard.GetInvite(context.Background(), id)
	if err != nil || inv.Status != onboardstore.InviteRevoked {
		t.Fatalf("post-revoke status = %v (%v), want revoked", inv, err)
	}
	if w := post(true); w.Code != http.StatusConflict {
		t.Fatalf("double revoke = %d, want 409", w.Code)
	}
}

// Sponsor sessions (ADR 1072) see and revoke only their OWN invites on the
// Invited tab — a foreign invite reads as absent from list, detail, and revoke.
func TestOnboardingInvitedTab_SponsorScoped(t *testing.T) {
	s := newOnboardTestServer(t)
	s.Cfg.DevBypassAuth = false
	portalURL, _ := fakeSponsorPortal(t, nil, map[string]sponsorLookupResponse{
		"sponsora@example.org": {Org: "0ff404dd", Alias: "pat-example", SponsorAuthorized: true, SponsorMaxTier: "developer"},
	})
	s.Cfg.PortalInternalURL = portalURL
	s.Cfg.PortalLookupToken = "svc-token"
	h := s.Routes()

	future := time.Now().Add(time.Hour).Unix()
	ownID := seedInvite(t, s, "sponsora@example.org", "mine", future, "")
	foreignID := seedInvite(t, s, "someone-else", "not mine", future, "")
	if _, err := s.Onboard.DB().ExecContext(context.Background(),
		`UPDATE onboard_invites SET prebound_json = $1 WHERE id = $2`,
		`{"tier":"iam-admin"`, foreignID); err != nil {
		t.Fatalf("corrupt foreign invite metadata: %v", err)
	}

	get := func(path string) *httptest.ResponseRecorder {
		r := httptest.NewRequestWithContext(context.Background(), http.MethodGet, path, nil)
		r.Header.Set("X-Forwarded-Email", "sponsora@example.org")
		r.Header.Set("X-Forwarded-Groups", "DEVELOPER")
		w := httptest.NewRecorder()
		h.ServeHTTP(w, r)
		return w
	}

	w := get("/onboarding?status=invited")
	if w.Code != http.StatusOK {
		t.Fatalf("sponsor invited tab = %d: %s", w.Code, w.Body.String())
	}
	if !strings.Contains(w.Body.String(), ownID) || strings.Contains(w.Body.String(), foreignID) {
		t.Fatalf("sponsor invited tab must list own invite only")
	}
	if w := get("/onboarding?status=invited&id=" + foreignID); w.Code != http.StatusNotFound {
		t.Fatalf("foreign invite detail = %d, want 404", w.Code)
	}

	form := url.Values{}
	form.Set("csrf", s.csrfNonce)
	r := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/onboarding/invite/"+foreignID+"/revoke", strings.NewReader(form.Encode()))
	r.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	r.Header.Set("X-Forwarded-Email", "sponsora@example.org")
	r.Header.Set("X-Forwarded-Groups", "DEVELOPER")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, r)
	if rec.Code != http.StatusNotFound {
		t.Fatalf("foreign revoke = %d, want 404", rec.Code)
	}
}

// The invite record carries what actually went out: the address it was sent to,
// when and how the console released it, the absolute deadline plus the time
// left, and every pre-bound field (including the ones left to the enrollee).
// Minted through the real web form so the recipient is recorded at mint —
// before any send is attempted — not only on a successful delivery.
func TestOnboardingInviteRecordShowsDelivery(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()

	form := url.Values{}
	form.Set("csrf", s.csrfNonce)
	form.Set("env", "zagros")
	form.Set("tier", "developer")
	form.Set("machine_quota", "2")
	form.Set("collab", "1")
	form.Set("ttl_hours", "48")
	form.Set("note", "romania cohort")
	form.Set("candidate_email", "person-49@example.org")
	r := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/onboarding/invite", strings.NewReader(form.Encode()))
	r.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	w := httptest.NewRecorder()
	h.ServeHTTP(w, r)
	if w.Code != http.StatusOK {
		t.Fatalf("mint = %d: %s", w.Code, w.Body.String())
	}

	invs, err := s.Onboard.ListInvites(context.Background(), "")
	if err != nil || len(invs) != 1 {
		t.Fatalf("list invites = %v (%v), want exactly one", invs, err)
	}
	inv := invs[0]
	// The mailer is unconfigured here, so nothing was sent — the address is
	// still on the record, and the row must not claim a delivery.
	if inv.CandidateEmail != "person-49@example.org" {
		t.Fatalf("candidate_email = %q, want the address typed at mint", inv.CandidateEmail)
	}
	if inv.SentAt != nil || inv.SentVia != "" {
		t.Fatalf("unsent invite claims delivery: sent_at=%v via=%q", inv.SentAt, inv.SentVia)
	}

	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/onboarding?status=invited&id="+inv.ID, nil))
	if rec.Code != http.StatusOK {
		t.Fatalf("invite detail = %d: %s", rec.Code, rec.Body.String())
	}
	body := rec.Body.String()
	for _, want := range []string{
		"person-49@example.org",                                     // who it went to
		"this console sent nothing",                            // delivery, stated honestly
		time.Unix(inv.ExpiresAt, 0).UTC().Format(time.RFC3339), // absolute deadline
		"lifetime 2d from minting",                             // the 48h TTL the sponsor chose
		"romania cohort",                                       // the note
		"developer", "enrollee's choice",                       // pre-bound tier + an open field
		"Machine quota", "Collab", "Sponsor ceiling",
	} {
		if !strings.Contains(body, want) {
			t.Fatalf("invite record missing %q:\n%s", want, body)
		}
	}

	// A recorded send shows the route and the time it left.
	if err := s.Onboard.RecordInviteSend(context.Background(), inv.ID, "person-49@example.org", onboardstore.SentViaEmail, time.Now().Unix()); err != nil {
		t.Fatalf("record send: %v", err)
	}
	rec = httptest.NewRecorder()
	h.ServeHTTP(rec, httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/onboarding?status=invited&id="+inv.ID, nil))
	if !strings.Contains(rec.Body.String(), "emailed by this console at") {
		t.Fatalf("invite record lost the recorded send:\n%s", rec.Body.String())
	}
}

// The dead-invite lifecycle explanations follow the configured window on both list and detail panes, for expired and revoked invites with sweep on and off.
func TestOnboardingInvitedTab_DeadInviteExplanations(t *testing.T) {
	s := newOnboardTestServer(t)
	// 14 days configured window
	s.Cfg.InviteArchiveAfter = 14 * 24 * time.Hour
	h := s.Routes()

	now := time.Now().UTC()
	// Expired: expired 2 hours ago
	expiresAt := now.Add(-2 * time.Hour).Unix()
	expiredID := seedInvite(t, s, "op@example.org", "expired-link", expiresAt, "")

	// Revoked: expires next year, but revoked 1 hour ago
	revokedID := seedInvite(t, s, "op@example.org", "revoked-link", now.Add(365*24*time.Hour).Unix(), "")
	if err := s.Onboard.RevokeInvite(context.Background(), revokedID); err != nil {
		t.Fatalf("seed revoke: %v", err)
	}
	// RevokeInvite stamps revoked_at for this row, so staleness anchors at this revocation.

	getDetail := func(id string) string {
		req := httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/onboarding?status=invited&id="+id, nil)
		rec := httptest.NewRecorder()
		h.ServeHTTP(rec, req)
		if rec.Code != http.StatusOK {
			t.Fatalf("detail %s = %d: %s", id, rec.Code, rec.Body.String())
		}
		return rec.Body.String()
	}

	bodyExp := getDetail(expiredID)
	// Exact remaining time, not a prefix: expiry is stored at second precision
	// and read a beat later, so the 14d window minus the 2h already elapsed
	// truncates to 333 whole hours, which twoUnit renders as "13d 21h".
	for _, want := range []string{
		"stays on this list for 14 days as a record",
		"moves to Archived automatically on",
		"Dead invites are kept, never deleted",
		"· in 13d 21h",
		"14 days after expiry, automatic",
		"expired 2h ago · archives in 13d 21h", // badge text
	} {
		if !strings.Contains(bodyExp, want) {
			t.Errorf("sweep-ON expired invite missing %q:\n%s", want, bodyExp)
		}
	}

	bodyRev := getDetail(revokedID)
	for _, want := range []string{
		"stays on this list for 14 days as a record",
		"moves to Archived automatically on",
		"This link was revoked on",
		"14 days after revocation, automatic",
	} {
		if !strings.Contains(bodyRev, want) {
			t.Errorf("sweep-ON revoked invite missing %q:\n%s", want, bodyRev)
		}
	}

	// Every window string on the page derives from the configured value. With
	// a 14-day window nothing may still read the old hard-coded "7 days" (or
	// a "7d" countdown); this guards the property the template grep could not.
	// Match the lifecycle phrases, not the bare token: invite ids are random
	// hex and one in a handful contains "7d" (inv-97d7b1...), which made a
	// whole-body substring check fail on unrelated runs.
	for name, body := range map[string]string{"expired detail": bodyExp, "revoked detail": bodyRev} {
		for _, stale := range []string{"for 7 days", "7 days after", "after 7 days", "archives in 7d", "in 7d "} {
			if strings.Contains(body, stale) {
				t.Errorf("%s renders %q with a 14-day window configured:\n%s", name, stale, body)
			}
		}
	}

	// Switch sweep OFF
	s.Cfg.InviteArchiveAfter = 0

	bodyExpOff := getDetail(expiredID)
	for _, want := range []string{
		"stays on this list permanently as a record",
		"Dead invites are kept, never deleted",
		"never · automatic archiving is disabled on this console",
	} {
		if !strings.Contains(bodyExpOff, want) {
			t.Errorf("sweep-OFF expired invite missing %q:\n%s", want, bodyExpOff)
		}
	}
	// Make sure the future archive time is NOT shown
	for _, unwanted := range []string{
		"moves to Archived automatically on",
		"archives in",
	} {
		if strings.Contains(bodyExpOff, unwanted) {
			t.Errorf("sweep-OFF expired invite incorrectly contains %q:\n%s", unwanted, bodyExpOff)
		}
	}
}

// When a dead invite's archive time is already in the past (sweep lag or the
// console being down for a while), the UI must not pretend it is still a
// future event. The badge reads "archives within the hour" and the ARCHIVES
// row reads "overdue, moves to Archived within the hour" instead of "in 40m".
func TestOnboardingInvitedTab_ArchiveOverdueUsesWithinTheHourLabels(t *testing.T) {
	s := newOnboardTestServer(t)
	// Short archive window so we can create a clearly overdue row.
	s.Cfg.InviteArchiveAfter = 2 * time.Hour
	h := s.Routes()

	now := time.Now().UTC()
	expiresAt := now.Add(-3 * time.Hour).Unix() // dead 3h ago
	overdueID := seedInvite(t, s, "op@example.org", "overdue-expired", expiresAt, "")

	req := httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/onboarding?status=invited", nil)
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("invited tab (overdue) = %d: %s", rec.Code, rec.Body.String())
	}
	body := rec.Body.String()
	if !strings.Contains(body, "archives within the hour") {
		t.Fatalf("overdue invite list badge must say 'archives within the hour':\n%s", body)
	}

	rec = httptest.NewRecorder()
	req = httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/onboarding?status=invited&id="+overdueID, nil)
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("overdue invite detail = %d: %s", rec.Code, rec.Body.String())
	}
	det := rec.Body.String()
	if !strings.Contains(det, "overdue, moves to Archived within the hour") {
		t.Fatalf("overdue invite ARCHIVES row missing 'overdue, moves to Archived within the hour':\n%s", det)
	}
	if !strings.Contains(det, "moves to Archived automatically within the hour") {
		t.Fatalf("overdue invite detail note must say it archives within the hour, not name a past date:\n%s", det)
	}
	if strings.Contains(det, "then moves to Archived automatically on") {
		t.Fatalf("overdue invite detail note must not promise a (past) archive date:\n%s", det)
	}
	if strings.Contains(det, "archives in") {
		t.Fatalf("overdue invite ARCHIVES row must not render 'archives in ...':\n%s", det)
	}
}

// The intersection the two tests above miss: a legacy revoked row (revoked
// before revoked_at was recorded, so it archives off the link's own expiry)
// whose archive time has ALREADY passed. The legacy wording must survive —
// nothing may claim a revocation moment that was never recorded — while the
// timing claim agrees with the badge and the ARCHIVES row: within the hour, not
// a date in the past.
func TestOnboardingInvitedTab_OverdueLegacyRevokedNoteSaysWithinTheHour(t *testing.T) {
	s := newOnboardTestServer(t)
	s.Cfg.InviteArchiveAfter = 2 * time.Hour
	h := s.Routes()

	// Expiry 5h ago with a 2h window puts the archive time 3h in the past.
	expiresAt := time.Now().UTC().Add(-5 * time.Hour).Unix()
	id := seedInvite(t, s, "op@example.org", "legacy-revoked-overdue", expiresAt, "")
	// A row revoked before revoked_at existed. The store has no path that
	// writes this shape any more, so the fixture sets it directly — the same
	// way onboardstore's own legacy-row tests do.
	if _, err := s.Onboard.DB().ExecContext(context.Background(),
		`UPDATE onboard_invites SET status = $1, revoked_at = NULL WHERE id = $2`,
		onboardstore.InviteRevoked, id); err != nil {
		t.Fatalf("seed legacy revoked row: %v", err)
	}

	rec := httptest.NewRecorder()
	req := httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/onboarding?status=invited&id="+id, nil)
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("legacy revoked overdue detail = %d: %s", rec.Code, rec.Body.String())
	}
	det := rec.Body.String()

	for _, want := range []string{
		"Its revocation time was not recorded",
		"moves to Archived automatically within the hour",
		"overdue, moves to Archived within the hour",
	} {
		if !strings.Contains(det, want) {
			t.Errorf("legacy revoked overdue note missing %q:\n%s", want, det)
		}
	}
	if strings.Contains(det, "then moves to Archived automatically on") {
		t.Errorf("legacy revoked overdue note must not name a past archive date:\n%s", det)
	}
}

// Selecting an archived invite must unfold the Archived section: the selected
// row lives inside it, and a folded <details> would hide the highlight the
// operator just clicked. Unselected or a dead-but-not-archived selection keeps
// it folded.
func TestOnboardingInvitedTab_ArchivedSelectionUnfoldsSection(t *testing.T) {
	s := newOnboardTestServer(t)
	s.Cfg.InviteArchiveAfter = 7 * 24 * time.Hour
	h := s.Routes()

	now := time.Now().UTC()
	archivedID := seedInvite(t, s, "op@example.org", "long-dead", now.Add(-10*24*time.Hour).Unix(), "")
	if _, err := s.Onboard.ArchiveStaleInvites(context.Background(), s.Cfg.InviteArchiveAfter); err != nil {
		t.Fatalf("archive sweep: %v", err)
	}
	deadID := seedInvite(t, s, "op@example.org", "freshly-dead", now.Add(-2*time.Hour).Unix(), "")

	get := func(q string) string {
		req := httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/onboarding?status=invited"+q, nil)
		rec := httptest.NewRecorder()
		h.ServeHTTP(rec, req)
		if rec.Code != http.StatusOK {
			t.Fatalf("GET %s = %d: %s", q, rec.Code, rec.Body.String())
		}
		return rec.Body.String()
	}

	if body := get("&id=" + archivedID); !strings.Contains(body, `<details class="ob-archive" open>`) {
		t.Errorf("archived selection must render the Archived section open:\n%s", body)
	} else if !strings.Contains(body, "No action available · an archived link already fails closed") {
		t.Errorf("archived selection missing its no-action note:\n%s", body)
	}
	for name, q := range map[string]string{"nothing selected": "", "dead selected": "&id=" + deadID} {
		if body := get(q); strings.Contains(body, `<details class="ob-archive" open>`) {
			t.Errorf("%s must keep the Archived section folded:\n%s", name, body)
		}
	}
}

// A submitted invite is not listed on the Invited tab, but the detail route
// loads any id, so a bookmarked link still renders the pane. inviteToView once
// gave that row no display state, which left a blank state word in the header
// line and an empty actions area. It must say what happened (used, became a
// proposal) and never offer Revoke.
func TestOnboardingInvitedTab_SubmittedInviteDetailExplainsNoAction(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()

	future := time.Now().Add(48 * time.Hour).Unix()
	submittedID := seedInvite(t, s, "op@example.org", "already submitted", future, onboardstore.InviteSubmitted)

	req := httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/onboarding?status=invited&id="+submittedID, nil)
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("submitted invite detail = %d: %s", rec.Code, rec.Body.String())
	}
	det := rec.Body.String()
	for _, want := range []string{
		"No action available · this link was used and became proposal <code>",
		"reviewed from the queue, not from here",
	} {
		if !strings.Contains(det, want) {
			t.Errorf("submitted invite detail missing %q:\n%s", want, det)
		}
	}
	for _, unwanted := range []string{
		"Revoke invite",
		"To re-nominate, mint a fresh invite",
		"a  link already fails closed",   // the empty-state rendering of the generic tail
		"This link is  and fails closed", // blank state word in the header line
	} {
		if strings.Contains(det, unwanted) {
			t.Errorf("submitted invite detail must not render %q:\n%s", unwanted, det)
		}
	}
}
