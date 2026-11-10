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

// onboardedForActions seeds one onboarded proposal with a device key and a
// handset key, the state the reissue button is offered in.
func onboardedForActions(t *testing.T, s *Server) *onboardstore.Proposal {
	t.Helper()
	ctx := context.Background()
	p := &onboardstore.Proposal{Sponsor: "jane-admin", Env: "zagros", Handle: "pat-liaison", Email: "person-105@example.org",
		ManifestJSON: `{"identity":{"handle":"pat-liaison","email":"person-105@example.org","name":"Pat Liaison","status":"active","added":"2026-06-01","added_by":"admin@example.test"},"netbird":{"enabled":true,"sites":["zagros"],"role":"peer","auto_groups":["OBSERVER"],"handset_limit":1},"ory_site":{"enabled":false},"ory_idp":{"enabled":false},"zenoh":{"enabled":false}}`, Tier: "observer"}
	if err := s.Onboard.CreateProposal(ctx, p); err != nil {
		t.Fatal(err)
	}
	if err := s.Onboard.SetProposalStatus(ctx, p.ID, onboardstore.ProposalOnboarded); err != nil {
		t.Fatal(err)
	}
	if err := s.Onboard.SetProposalResult(ctx, p.ID, onboardstore.ProposalOnboarded, "DEVICE-KEY", "OLD-HANDSET", "", "", "", false); err != nil {
		t.Fatal(err)
	}
	return p
}

func postReissueButton(t *testing.T, s *Server, h http.Handler, id string) *httptest.ResponseRecorder {
	t.Helper()
	form := url.Values{}
	form.Set("csrf", s.csrfNonce)
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/onboarding/"+id+"/handset-key/reissue", strings.NewReader(form.Encode()))
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	return rec
}

func claimMachineAction(t *testing.T, h http.Handler, id string) string {
	t.Helper()
	res := postMachine(t, h, "/api/onboard/actions/"+id+"/claim", nil)
	if res.code != http.StatusOK {
		t.Fatalf("claim: want 200, got %d: %s", res.code, res.body)
	}
	var action onboardstore.Action
	if err := json.Unmarshal([]byte(res.body), &action); err != nil || action.ClaimToken == "" {
		t.Fatalf("claim response=%s err=%v", res.body, err)
	}
	return action.ClaimToken
}

func TestHandsetReissueButtonQueuesOnce(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	p := onboardedForActions(t, s)

	rec := postReissueButton(t, s, h, p.ID)
	if rec.Code != http.StatusSeeOther || !strings.Contains(rec.Header().Get("Location"), "flash="+flashHandsetQueued) {
		t.Fatalf("button: want 303 to ?flash=%s, got %d %q", flashHandsetQueued, rec.Code, rec.Header().Get("Location"))
	}
	queued, err := s.Onboard.ListActions(context.Background(), onboardstore.ActionQueued)
	if err != nil || len(queued) != 1 || queued[0].Kind != onboardstore.ActionReissueHandset || queued[0].ProposalID != p.ID {
		t.Fatalf("queue after button = %+v, %v", queued, err)
	}
	// A second click while the first is pending queues nothing and says so.
	rec = postReissueButton(t, s, h, p.ID)
	if !strings.Contains(rec.Header().Get("Location"), "flash="+flashHandsetPending) {
		t.Fatalf("second click: want flash=%s, got %q", flashHandsetPending, rec.Header().Get("Location"))
	}
	if again, _ := s.Onboard.ListActions(context.Background(), onboardstore.ActionQueued); len(again) != 1 {
		t.Fatalf("second click queued another action: %d", len(again))
	}
	// A missing CSRF token is refused before anything is queued.
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/onboarding/"+p.ID+"/handset-key/reissue", nil)
	nrec := httptest.NewRecorder()
	h.ServeHTTP(nrec, req)
	if nrec.Code != http.StatusForbidden {
		t.Fatalf("no csrf: want 403, got %d", nrec.Code)
	}
}

func TestHandsetReissueButtonRefusesNonOnboarded(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	p := &onboardstore.Proposal{Sponsor: "jane-admin", Env: "zagros", Handle: "sam", Email: "person-111@example.org", ManifestJSON: `{}`}
	if err := s.Onboard.CreateProposal(context.Background(), p); err != nil {
		t.Fatal(err)
	}
	rec := postReissueButton(t, s, h, p.ID)
	if !strings.Contains(rec.Header().Get("Location"), "flash="+flashHandsetState) {
		t.Fatalf("proposed row: want flash=%s, got %q", flashHandsetState, rec.Header().Get("Location"))
	}
	if q, _ := s.Onboard.ListActions(context.Background(), ""); len(q) != 0 {
		t.Fatalf("nothing should be queued, got %d", len(q))
	}
}

func TestActionAPIClaimAndResultStoresTheNewHandsetKey(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	p := onboardedForActions(t, s)
	a, err := s.Onboard.EnqueueAction(context.Background(), p.ID, onboardstore.ActionReissueHandset, "maintainer@example.invalid")
	if err != nil {
		t.Fatal(err)
	}

	// The runner lists the queue and sees the handle + env it needs.
	list := getMachine(t, h, "/api/onboard/actions?status=queued")
	if list.code != http.StatusOK {
		t.Fatalf("list: want 200, got %d: %s", list.code, list.body)
	}
	var rows []onboardstore.Action
	if err := json.Unmarshal([]byte(list.body), &rows); err != nil || len(rows) != 1 || rows[0].Handle != "pat-liaison" || rows[0].Env != "zagros" {
		t.Fatalf("list rows = %s (%v)", list.body, err)
	}

	// Claim wins once; the second claim is a 409.
	claimToken := claimMachineAction(t, h, a.ID)
	if c := postMachine(t, h, "/api/onboard/actions/"+a.ID+"/claim", nil); c.code != http.StatusConflict {
		t.Fatalf("second claim: want 409, got %d", c.code)
	}
	got, _ := s.Onboard.GetAction(context.Background(), a.ID)
	if got.ClaimedBy != "onboard-runner" {
		t.Fatalf("claimed_by = %q, want the runner's per-handle key name", got.ClaimedBy)
	}

	// Done with a key: the proposal carries the new key, the device key is
	// untouched, and the action records that the welcome resend was skipped
	// (no mailer in tests) rather than pretending it went out.
	res := postMachine(t, h, "/api/onboard/actions/"+a.ID+"/result", map[string]any{"status": "done", "claim_token": claimToken, "handset_key": "NEW-HANDSET"})
	if res.code != http.StatusOK {
		t.Fatalf("result: want 200, got %d: %s", res.code, res.body)
	}
	pp, _ := s.Onboard.GetProposal(context.Background(), p.ID)
	if pp.HandsetKey != "NEW-HANDSET" || pp.SetupKey != "DEVICE-KEY" || pp.Status != onboardstore.ProposalOnboarded {
		t.Fatalf("proposal after result: handset=%q setup=%q status=%q", pp.HandsetKey, pp.SetupKey, pp.Status)
	}
	got, _ = s.Onboard.GetAction(context.Background(), a.ID)
	if got.Status != onboardstore.ActionDone || !strings.Contains(got.ResultJSON, `"welcome_resend":"skipped"`) {
		t.Fatalf("action after result: %+v", got)
	}
	// A result on a finished action is refused.
	if r2 := postMachine(t, h, "/api/onboard/actions/"+a.ID+"/result", map[string]any{"status": "done", "claim_token": claimToken, "handset_key": "X"}); r2.code != http.StatusConflict {
		t.Fatalf("late result: want 409, got %d", r2.code)
	}
}

func TestActionAPIResultAllowsTokenlessReissueButRejectsWrongToken(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	p := onboardedForActions(t, s)

	tokenless, err := s.Onboard.EnqueueAction(context.Background(), p.ID, onboardstore.ActionReissueHandset, "admin@example.test")
	if err != nil {
		t.Fatal(err)
	}
	if err := s.Onboard.ClaimAction(context.Background(), tokenless.ID, "onboard-runner"); err != nil {
		t.Fatal(err)
	}
	res := postMachine(t, h, "/api/onboard/actions/"+tokenless.ID+"/result", map[string]any{"status": "failed", "error": "legacy failure"})
	if res.code != http.StatusOK {
		t.Fatalf("tokenless reissue result: want 200, got %d: %s", res.code, res.body)
	}
	if got, err := s.Onboard.GetAction(context.Background(), tokenless.ID); err != nil || got.Status != onboardstore.ActionFailed {
		t.Fatalf("tokenless result action=%+v err=%v, want failed", got, err)
	}

	wrong, err := s.Onboard.EnqueueAction(context.Background(), p.ID, onboardstore.ActionReissueHandset, "admin@example.test")
	if err != nil {
		t.Fatal(err)
	}
	if err := s.Onboard.ClaimAction(context.Background(), wrong.ID, "onboard-runner"); err != nil {
		t.Fatal(err)
	}
	res = postMachine(t, h, "/api/onboard/actions/"+wrong.ID+"/result", map[string]any{"status": "failed", "error": "failed", "claim_token": "wrong-token"})
	if res.code != http.StatusConflict {
		t.Fatalf("wrong token: want 409, got %d: %s", res.code, res.body)
	}
}

func TestActionAPIResultReleaseRequiresClaimToken(t *testing.T) {
	s := newOnboardTestServer(t)
	a, err := s.Onboard.EnqueueHoldAction(context.Background(), "zagros", "peer-1", time.Now().UTC(), "peer-one", "admin@example.test", `{}`)
	if err != nil {
		t.Fatal(err)
	}
	if err := s.Onboard.ClaimAction(context.Background(), a.ID, "new-runner"); err != nil {
		t.Fatal(err)
	}
	res := postMachine(t, s.Routes(), "/api/onboard/actions/"+a.ID+"/result", map[string]any{
		"status": "failed", "release": map[string]any{"outcome": "refused", "audit_outcome": "recorded"},
	})
	if res.code != http.StatusConflict {
		t.Fatalf("tokenless release result: want 409, got %d: %s", res.code, res.body)
	}
}

func TestActionAPIDoneWithoutKeyIsRecordedFailed(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	p := onboardedForActions(t, s)
	a, _ := s.Onboard.EnqueueAction(context.Background(), p.ID, onboardstore.ActionReissueHandset, "x")
	claimToken := claimMachineAction(t, h, a.ID)
	res := postMachine(t, h, "/api/onboard/actions/"+a.ID+"/result", map[string]any{"status": "done", "claim_token": claimToken})
	if res.code != http.StatusOK {
		t.Fatalf("result: want 200, got %d: %s", res.code, res.body)
	}
	got, _ := s.Onboard.GetAction(context.Background(), a.ID)
	if got.Status != onboardstore.ActionFailed || !strings.Contains(got.Error, "without a handset key") {
		t.Fatalf("done-without-key should be failed: %+v", got)
	}
	pp, _ := s.Onboard.GetProposal(context.Background(), p.ID)
	if pp.HandsetKey != "OLD-HANDSET" {
		t.Fatalf("old key must survive a keyless result, got %q", pp.HandsetKey)
	}
	// The queue is free again for a retry.
	if _, err := s.Onboard.EnqueueAction(context.Background(), p.ID, onboardstore.ActionReissueHandset, "x"); err != nil {
		t.Fatalf("re-enqueue after failed: %v", err)
	}
}

func postDeviceKeyButton(t *testing.T, s *Server, h http.Handler, id, uses string) *httptest.ResponseRecorder {
	t.Helper()
	form := url.Values{}
	form.Set("csrf", s.csrfNonce)
	form.Set("uses", uses)
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/onboarding/"+id+"/device-key/reissue", strings.NewReader(form.Encode()))
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	return rec
}

func TestDeviceKeyReissueButtonQueuesWithUses(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	p := onboardedForActions(t, s)

	for _, bad := range []string{"0", "101", "five"} {
		rec := postDeviceKeyButton(t, s, h, p.ID, bad)
		if !strings.Contains(rec.Header().Get("Location"), "flash="+flashDeviceKeyUses) {
			t.Fatalf("uses=%q: want flash=%s, got %q", bad, flashDeviceKeyUses, rec.Header().Get("Location"))
		}
	}
	if q, _ := s.Onboard.ListActions(context.Background(), ""); len(q) != 0 {
		t.Fatalf("a bad uses value queued %d action(s)", len(q))
	}

	rec := postDeviceKeyButton(t, s, h, p.ID, "7")
	if rec.Code != http.StatusSeeOther || !strings.Contains(rec.Header().Get("Location"), "flash="+flashDeviceKeyQueued) {
		t.Fatalf("button: want 303 to ?flash=%s, got %d %q", flashDeviceKeyQueued, rec.Code, rec.Header().Get("Location"))
	}
	queued, err := s.Onboard.ListActions(context.Background(), onboardstore.ActionQueued)
	if err != nil || len(queued) != 1 || queued[0].Kind != onboardstore.ActionReissueDevice || queued[0].Params != `{"uses":7}` {
		t.Fatalf("queue after button = %+v, %v", queued, err)
	}
	if rec := postDeviceKeyButton(t, s, h, p.ID, ""); !strings.Contains(rec.Header().Get("Location"), "flash="+flashDeviceKeyPending) {
		t.Fatalf("second click: want flash=%s, got %q", flashDeviceKeyPending, rec.Header().Get("Location"))
	}
}

func TestActionAPIResultStoresTheNewDeviceKey(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	p := onboardedForActions(t, s)
	a, err := s.Onboard.EnqueueActionWithParams(context.Background(), p.ID, onboardstore.ActionReissueDevice, "maintainer@example.invalid", `{"uses":5}`)
	if err != nil {
		t.Fatal(err)
	}
	claimToken := claimMachineAction(t, h, a.ID)

	// done without a key is a failure, and the old key survives.
	b, _ := s.Onboard.EnqueueAction(context.Background(), onboardedForActions(t, s).ID, onboardstore.ActionReissueDevice, "x")
	bClaimToken := claimMachineAction(t, h, b.ID)
	postMachine(t, h, "/api/onboard/actions/"+b.ID+"/result", map[string]any{"status": "done", "claim_token": bClaimToken, "handset_key": "WRONG-FIELD"})
	if got, _ := s.Onboard.GetAction(context.Background(), b.ID); got.Status != onboardstore.ActionFailed || !strings.Contains(got.Error, "without a setup key") {
		t.Fatalf("device done-without-setup-key should be failed: %+v", got)
	}

	res := postMachine(t, h, "/api/onboard/actions/"+a.ID+"/result", map[string]any{"status": "done", "claim_token": claimToken, "setup_key": "NEW-DEVICE"})
	if res.code != http.StatusOK {
		t.Fatalf("result: want 200, got %d: %s", res.code, res.body)
	}
	pp, _ := s.Onboard.GetProposal(context.Background(), p.ID)
	if pp.SetupKey != "NEW-DEVICE" || pp.HandsetKey != "OLD-HANDSET" || pp.Status != onboardstore.ProposalOnboarded {
		t.Fatalf("proposal after result: setup=%q handset=%q status=%q", pp.SetupKey, pp.HandsetKey, pp.Status)
	}
	got, _ := s.Onboard.GetAction(context.Background(), a.ID)
	if got.Status != onboardstore.ActionDone || !strings.Contains(got.ResultJSON, `"setup_key_set":true`) || strings.Contains(got.ResultJSON, "NEW-DEVICE") {
		t.Fatalf("action after result must record the set, never the key: %+v", got)
	}
}

func TestReissueResultAfterOffboardRequestDoesNotPublishOrDeliver(t *testing.T) {
	for _, kind := range []string{onboardstore.ActionReissueDevice, onboardstore.ActionReissueHandset} {
		t.Run(kind, func(t *testing.T) {
			s := newOnboardTestServer(t)
			h := s.Routes()
			ctx := context.Background()
			p := onboardedForActions(t, s)
			a, err := s.Onboard.EnqueueAction(ctx, p.ID, kind, "fixture")
			if err != nil {
				t.Fatal(err)
			}
			if c := postMachine(t, h, "/api/onboard/actions/"+a.ID+"/claim", nil); c.code != http.StatusOK {
				t.Fatalf("claim: %d", c.code)
			}
			if ok, err := s.Onboard.RequestOffboard(ctx, p.ID, "fixture"); err != nil || !ok {
				t.Fatalf("request: %v %v", ok, err)
			}
			res := postMachine(t, h, "/api/onboard/actions/"+a.ID+"/result", map[string]any{"status": "done", "setup_key": "NEW-DEVICE", "handset_key": "NEW-HANDSET"})
			if res.code != http.StatusConflict {
				t.Fatalf("late result: %d %s", res.code, res.body)
			}
			pp, err := s.Onboard.GetProposal(ctx, p.ID)
			if err != nil {
				t.Fatal(err)
			}
			if pp.Status != onboardstore.ProposalOffboardRequested || pp.SetupKey != "DEVICE-KEY" || pp.HandsetKey != "OLD-HANDSET" {
				t.Fatal("late result changed proposal")
			}
			action, err := s.Onboard.GetAction(ctx, a.ID)
			if err != nil {
				t.Fatal(err)
			}
			if action.Status != onboardstore.ActionFailed || action.ResultJSON != "{}" {
				t.Fatal("late result published delivery metadata")
			}
			if err := s.Onboard.ClaimOffboard(ctx, p.ID); err != nil {
				t.Fatal(err)
			}
		})
	}
}
