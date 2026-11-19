package handlers

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

func httptestReqJSON(t *testing.T, path, key string) *http.Request {
	t.Helper()
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, path, strings.NewReader(`{}`))
	req.Header.Set("Authorization", "Bearer "+key)
	req.Header.Set("Content-Type", "application/json")
	return req
}

func httptestRecord(h http.Handler, req *http.Request) *httptest.ResponseRecorder {
	w := httptest.NewRecorder()
	h.ServeHTTP(w, req)
	return w
}

// rv5-vr6w F1: the machine invite door applies the browser form's tier
// rule. A per-person key may not pre-bind an invite to the platform tier (or
// to a tier the table cannot place); it keeps lower tiers; a service key is
// uncapped.
func TestTierCeiling_machineInviteTier(t *testing.T) {
	s, _ := tgKeyServer(t)
	mint := func(key, tier string) int {
		b, _ := json.Marshal(map[string]any{"env": "zagros", "tier": tier, "ttl_hours": 24})
		return tgAPI(t, s, http.MethodPost, key, "/api/onboard/invite", string(b)).Code
	}
	for _, tier := range []string{"platform-admin", "no-such-tier"} {
		if c := mint("k-casey", tier); c != http.StatusForbidden {
			t.Fatalf("per-person key invite at %q: %d, want 403", tier, c)
		}
	}
	for _, tier := range []string{"iam-admin", "developer", ""} {
		if c := mint("k-casey", tier); c != http.StatusCreated {
			t.Fatalf("per-person key invite at %q: %d, want 201", tier, c)
		}
	}
	if c := mint("k-runner", "platform-admin"); c != http.StatusForbidden {
		t.Fatalf("service key invite at platform-admin: %d, want 403 (ADR 1071 as amended)", c)
	}
	if c := mint("k-runner", "iam-admin"); c != http.StatusCreated {
		t.Fatalf("service key invite at iam-admin: %d, want 201", c)
	}
}

// rv5-vr6w F2: no machine route is admitted by prefix. A route under
// /api/onboard/ or /api/org-registry/ that is not listed is refused.
func TestTargetGate_machineRouteUnderOnboardPrefixRefused(t *testing.T) {
	s, _ := tgKeyServer(t)
	mux := http.NewServeMux()
	ran := false
	h := func(w http.ResponseWriter, r *http.Request) { ran = true; w.WriteHeader(http.StatusOK) }
	mux.HandleFunc("POST /api/onboard/new-thing", s.onboardAPIGate(s.machineTargetGate(h)))
	mux.HandleFunc("POST /api/org-registry/new-thing/{id}", s.onboardAPIGate(s.machineTargetGate(h)))
	for _, path := range []string{"/api/onboard/new-thing", "/api/org-registry/new-thing/x"} {
		req := httptestReqJSON(t, path, "k-runner")
		w := httptestRecord(mux, req)
		if w.Code != http.StatusForbidden || ran {
			t.Fatalf("unlisted machine route %s: %d ran=%v, want 403 and not run", path, w.Code, ran)
		}
	}
}

// r10 (rv7 F1): a per-person key may not claim, or post the result of, a
// follow-up action (mesh-key reissue) for a person holding an admin tier;
// the runner's service key may.
func TestTierCeiling_followUpActionsCapped(t *testing.T) {
	s, _ := tgKeyServer(t)
	enqueue := func(handle, tier string) string {
		p := tgProposal(t, s, handle, onboardstore.ProposalOnboarded, tier, "{}")
		a, err := s.Onboard.EnqueueAction(context.Background(), p.ID, onboardstore.ActionReissueHandset, "pa@example.org")
		if err != nil {
			t.Fatal(err)
		}
		return a.ID
	}
	pa := enqueue("pa-act", "platform-admin")
	if w := tgAPI(t, s, http.MethodPost, "k-casey", "/api/onboard/actions/"+pa+"/claim", "{}"); w.Code != http.StatusForbidden {
		t.Fatalf("per-person claim of a platform admin's reissue: %d %s, want 403", w.Code, w.Body.String())
	}
	if w := tgAPI(t, s, http.MethodPost, "k-casey", "/api/onboard/actions/"+pa+"/result", `{"status":"done","handset_key":"HK-X"}`); w.Code != http.StatusForbidden {
		t.Fatalf("per-person result of a platform admin's reissue: %d %s, want 403", w.Code, w.Body.String())
	}
	got, _ := s.Onboard.GetAction(context.Background(), pa)
	if got.Status != onboardstore.ActionQueued {
		t.Fatalf("action status %s after refused calls, want queued", got.Status)
	}
	// IAM-ADMIN is an admin tier too: the same rule as GHSA-7788.
	iam := enqueue("iam-act", "iam-admin")
	if w := tgAPI(t, s, http.MethodPost, "k-casey", "/api/onboard/actions/"+iam+"/claim", "{}"); w.Code != http.StatusForbidden {
		t.Fatalf("per-person claim of an IAM-ADMIN's reissue: %d, want 403", w.Code)
	}
	if w := tgAPI(t, s, http.MethodPost, "k-runner", "/api/onboard/actions/"+pa+"/claim", "{}"); w.Code != http.StatusOK {
		t.Fatalf("service claim: %d %s", w.Code, w.Body.String())
	}
	dev := enqueue("dev-act", "developer")
	if w := tgAPI(t, s, http.MethodPost, "k-casey", "/api/onboard/actions/"+dev+"/claim", "{}"); w.Code != http.StatusOK {
		t.Fatalf("per-person claim of a developer's reissue: %d %s", w.Code, w.Body.String())
	}
}

// r10 (rv7 hardening b): a wildcard {action} route runs only the actions
// its list names; any other action is 404 before the handler.
func TestTargetGate_wildcardActionsListed(t *testing.T) {
	s, _ := tgKeyServer(t)
	for pattern, actions := range wildcardActionsVr6w {
		base := strings.TrimSuffix(strings.TrimPrefix(pattern, "POST "), "{action}")
		base = strings.Replace(base, "{id}", "no-such-id", 1)
		post := func(action string) *httptest.ResponseRecorder {
			return tcPost(t, s, tcAsPA, base+action+"?idp=tenant", url.Values{})
		}
		if rec := post("not-a-listed-action"); rec.Code != http.StatusNotFound || rec.Body.String() != "404 page not found\n" {
			t.Fatalf("%s: unlisted action: %d %q, want the mux 404", pattern, rec.Code, rec.Body.String())
		}
		for _, a := range actions {
			rec := post(a)
			if rec.Body.String() == "404 page not found\n" {
				t.Fatalf("%s: listed action %q did not reach the handler", pattern, a)
			}
			// The handler's own switch knows every listed action (each
			// handler answers an action it does not implement with
			// "unknown action").
			if strings.Contains(rec.Body.String(), "unknown action") {
				t.Fatalf("%s: listed action %q is not one its handler implements", pattern, a)
			}
		}
	}
}

// r9 (rv6 F1): the browser onboarding invite form caps an admin session at
// its own tier, as POST /invites and the machine invite do.
func TestTierCeiling_browserOnboardingInviteTier(t *testing.T) {
	s, _ := tgKeyServer(t)
	mint := func(as tcActor, tier string) (int, bool) {
		before, _ := s.Onboard.ListInvites(context.Background(), "")
		rec := tcPost(t, s, as, "/onboarding/invite?idp=tenant", url.Values{"tier": {tier}, "ttl_hours": {"24"}, "env": {"zagros"}})
		after, _ := s.Onboard.ListInvites(context.Background(), "")
		return rec.Code, len(after) > len(before)
	}
	if code, minted := mint(tcAsIAM, "platform-admin"); code != http.StatusForbidden || minted {
		t.Fatalf("IAM-ADMIN invite at platform-admin: %d minted=%v, want 403 and none", code, minted)
	}
	if code, minted := mint(tcAsIAM, "no-such-tier"); code != http.StatusForbidden || minted {
		t.Fatalf("IAM-ADMIN invite at an unknown tier: %d minted=%v, want 403 and none", code, minted)
	}
	for _, c := range []struct {
		as   tcActor
		tier string
	}{{tcAsIAM, "iam-admin"}, {tcAsIAM, "developer"}, {tcAsPA, "platform-admin"}} {
		if code, minted := mint(c.as, c.tier); code == http.StatusForbidden || !minted {
			t.Fatalf("%s invite at %q: %d minted=%v, want minted", c.as.groups, c.tier, code, minted)
		}
	}
}

// r9 (rv6 note): a welcome resend that carries new mesh keys does not
// overwrite the stored keys of a person above the actor's tier.
func TestTierCeiling_welcomeResendKeepsHigherTierKeys(t *testing.T) {
	s, _ := tgKeyServer(t)
	mk := func(handle, tier string) *onboardstore.Proposal {
		p := tgProposal(t, s, handle, onboardstore.ProposalOnboarded, tier, "{}")
		if err := s.Onboard.SetProposalResult(context.Background(), p.ID, onboardstore.ProposalOnboarded, "OLD-SK", "OLD-HK", "", "", "", false); err != nil {
			t.Fatal(err)
		}
		return p
	}
	pa := mk("pa-resend", "platform-admin")
	rec := tcPost(t, s, tcAsIAM, "/onboarding/"+pa.ID+"/welcome/resend?idp=tenant", url.Values{"setup_key": {"NEW-SK"}, "handset_key": {"NEW-HK"}})
	got, _ := s.Onboard.GetProposal(context.Background(), pa.ID)
	if rec.Code != http.StatusForbidden || got.SetupKey != "OLD-SK" || got.HandsetKey != "OLD-HK" {
		t.Fatalf("IAM-ADMIN resend with keys for a platform admin: %d keys %s/%s, want 403 and unchanged", rec.Code, got.SetupKey, got.HandsetKey)
	}
	dev := mk("dev-resend", "developer")
	if rec := tcPost(t, s, tcAsIAM, "/onboarding/"+dev.ID+"/welcome/resend?idp=tenant", url.Values{"setup_key": {"NEW-SK"}}); rec.Code == http.StatusForbidden {
		t.Fatalf("IAM-ADMIN resend with a key for a developer: 403 %s", rec.Body.String())
	}
	if got, _ := s.Onboard.GetProposal(context.Background(), dev.ID); got.SetupKey != "NEW-SK" {
		t.Fatalf("developer key not replaced: %s", got.SetupKey)
	}
}

// r9 (rv6 F2): browser routes are denied by name, not by the shape of the
// path. The reviewer's three mutations, registered on the console's own
// recording mux: a body-target admin route, an admin route under a new
// prefix, and an onboardGate route without {id}. Each is refused (403) and
// its handler never runs.
func TestTargetGate_browserRoutesDenyByName(t *testing.T) {
	s, _ := tgKeyServer(t)
	ran := 0
	h := func(w http.ResponseWriter, r *http.Request) { ran++; w.WriteHeader(http.StatusOK) }
	m := s.newRouteMux()
	m.HandleFunc("POST /users/recovery-link", s.adminGate(s.targetGate(h)))
	m.HandleFunc("POST /accounts/recover", s.adminGate(s.targetGate(h)))
	m.HandleFunc("POST /onboarding/recover", s.onboardGate(h))
	for _, path := range []string{"/users/recovery-link", "/accounts/recover", "/onboarding/recover"} {
		form := url.Values{"id": {tcPA}, "csrf": {s.csrfNonce}}
		req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, path+"?idp=tenant", strings.NewReader(form.Encode()))
		req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
		req.Header.Set("X-Forwarded-Email", tcAsIAM.email)
		req.Header.Set("X-Forwarded-User", tcAsIAM.id)
		req.Header.Set("X-Forwarded-Groups", tcAsIAM.groups)
		w := httptestRecord(m, req)
		if w.Code != http.StatusForbidden {
			t.Fatalf("unlisted browser route %s: %d, want 403", path, w.Code)
		}
	}
	if ran != 0 {
		t.Fatalf("an unlisted route's handler ran %d times", ran)
	}
}

// r8: a per-person key gets the approve's cap on claim, result, retry and
// archive; the runner's service key keeps claim and result.
func TestTierCeiling_perPersonLifecycleActions(t *testing.T) {
	s, _ := tgKeyServer(t)
	status := func(id string) string {
		got, _ := s.Onboard.GetProposal(context.Background(), id)
		return got.Status
	}
	for _, c := range []struct{ action, from, body string }{
		{"claim", onboardstore.ProposalApproved, ""},
		{"result", onboardstore.ProposalOnboarding, `{"status":"onboarded","setup_key":"SK"}`},
		{"retry", onboardstore.ProposalFailed, ""},
		{"archive", onboardstore.ProposalOnboarded, ""},
	} {
		p := tgProposal(t, s, "pa-"+c.action, c.from, "", tgManifest("pa-"+c.action, []string{"PLATFORM-ADMIN"}))
		if w := tgAPI(t, s, http.MethodPost, "k-casey", "/api/onboard/proposals/"+p.ID+"/"+c.action, c.body); w.Code != http.StatusForbidden || status(p.ID) != c.from {
			t.Fatalf("per-person %s of a PLATFORM-ADMIN row: %d %s, status %s", c.action, w.Code, w.Body.String(), status(p.ID))
		}
		d := tgProposal(t, s, "dev-"+c.action, c.from, "developer", tgManifest("dev-"+c.action, []string{"DEVELOPER"}))
		if w := tgAPI(t, s, http.MethodPost, "k-casey", "/api/onboard/proposals/"+d.ID+"/"+c.action, c.body); w.Code == http.StatusForbidden {
			t.Fatalf("per-person %s of a developer row: 403 %s", c.action, w.Body.String())
		}
	}
	p := tgProposal(t, s, "pa-svc", onboardstore.ProposalApproved, "", tgManifest("pa-svc", []string{"PLATFORM-ADMIN"}))
	if w := tgAPI(t, s, http.MethodPost, "k-runner", "/api/onboard/proposals/"+p.ID+"/claim", ""); w.Code != http.StatusOK {
		t.Fatalf("service claim: %d %s", w.Code, w.Body.String())
	}
	if w := tgAPI(t, s, http.MethodPost, "k-runner", "/api/onboard/proposals/"+p.ID+"/result", `{"status":"onboarded","setup_key":"SK"}`); w.Code != http.StatusOK {
		t.Fatalf("service result: %d %s", w.Code, w.Body.String())
	}
}

// r5: restore and retry put a proposal back into the runner's mint queue
// without an approve, so they carry the approve's admin tier ceiling:
// register -> archive -> restore by a per-person key, a per-person retry,
// and a browser restore or retry above the actor's tier are refused.
func TestTierCeiling_restoreAndRetryRankTheRow(t *testing.T) {
	s, _ := tgKeyServer(t)
	body, _ := json.Marshal(map[string]string{"handle": "puppet", "env": "zagros", "email": "puppet@evil.example",
		"manifest_json": tgManifest("puppet", []string{"PLATFORM-ADMIN"}), "tier": "platform-admin"})
	// The row is registered with the service key: GHSA-7788 refuses a
	// per-person key's register of an admin row, and this test is about the
	// restore and retry that follow.
	w := tgAPI(t, s, http.MethodPost, "k-runner", "/api/onboard/register", string(body))
	if w.Code != http.StatusCreated {
		t.Fatalf("register: %d %s", w.Code, w.Body.String())
	}
	var reg struct {
		ID string `json:"id"`
	}
	_ = json.Unmarshal(w.Body.Bytes(), &reg)
	if w := tgAPI(t, s, http.MethodPost, "k-runner", "/api/onboard/proposals/"+reg.ID+"/archive", ""); w.Code != http.StatusOK {
		t.Fatalf("archive: %d %s", w.Code, w.Body.String())
	}
	if w := tgAPI(t, s, http.MethodPost, "k-casey", "/api/onboard/proposals/"+reg.ID+"/restore", ""); w.Code != http.StatusForbidden {
		t.Fatalf("per-person restore of a PLATFORM-ADMIN row: %d %s, want 403", w.Code, w.Body.String())
	}
	if got, _ := s.Onboard.GetProposal(context.Background(), reg.ID); got.Status != onboardstore.ProposalArchived {
		t.Fatalf("status %s, want archived", got.Status)
	}
	f := tgProposal(t, s, "pa-fail", onboardstore.ProposalFailed, "", tgManifest("pa-fail", []string{"PLATFORM-ADMIN"}))
	if w := tgAPI(t, s, http.MethodPost, "k-casey", "/api/onboard/proposals/"+f.ID+"/retry", ""); w.Code != http.StatusForbidden {
		t.Fatalf("per-person retry of a PLATFORM-ADMIN row: %d %s, want 403", w.Code, w.Body.String())
	}
	asIAM := tcActor{"iam@example.org", tcIAM, "IAM-ADMIN,GRAFANA-READONLY"}
	if rec := tcPost(t, s, asIAM, "/onboarding/"+reg.ID+"/restore?idp=tenant", url.Values{}); rec.Code != http.StatusForbidden {
		t.Fatalf("IAM-ADMIN restore of a PLATFORM-ADMIN row: %d, want 403", rec.Code)
	}
	tcPost(t, s, asIAM, "/onboarding/"+f.ID+"/retry?idp=tenant", url.Values{})
	if got, _ := s.Onboard.GetProposal(context.Background(), f.ID); got.Status != onboardstore.ProposalFailed {
		t.Fatalf("IAM-ADMIN retry of a PLATFORM-ADMIN row: status %s, want failed", got.Status)
	}
	// A service key may not re-queue the platform tier (ADR 1071 as
	// amended); a platform admin in the console may.
	if w := tgAPI(t, s, http.MethodPost, "k-runner", "/api/onboard/proposals/"+reg.ID+"/restore", ""); w.Code != http.StatusForbidden {
		t.Fatalf("service key restore of the platform tier: %d %s, want 403", w.Code, w.Body.String())
	}
	if rec := tcPost(t, s, tcAsPA, "/onboarding/"+reg.ID+"/restore?idp=tenant", url.Values{}); rec.Code == http.StatusForbidden {
		t.Fatalf("platform admin restore: 403 %s", rec.Body.String())
	}
	if rec := tcPost(t, s, tcAsPA, "/onboarding/"+f.ID+"/retry?idp=tenant", url.Values{}); rec.Code == http.StatusForbidden {
		t.Fatalf("platform admin retry: 403 %s", rec.Body.String())
	}
}
