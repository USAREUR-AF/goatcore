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

// Every registered route (read from the router itself, so split lines,
// loops and other prefixes are all seen), browser and machine, with or
// without a path variable, is classified by name (route_classes_vr6w.go).
// The target tables' routes are wrapped by their gate; no table names an
// unregistered route (except stack-only entries); and no route sits in both
// a target table and the not-target list.
func TestTargetGate_coversEveryTargetRoute(t *testing.T) {
	kc, _ := fakeKratos(t, "id-x", "x@example.org", "X", nil, "active")
	s := newUsersAPITestServer(t, kc)
	_ = s.Routes()
	routes := s.registeredRoutes()
	if len(routes) < 100 {
		t.Fatalf("recorded only %d routes", len(routes))
	}
	seen := map[string]bool{}
	for _, e := range routes {
		seen[e.pattern] = true
		if !routeClassifiedVr6w(e.pattern) {
			t.Errorf("route %q is not classified by name; list it as a target route or in routeNotTargetVr6w with a reason (it is refused at runtime)", e.pattern)
			continue
		}
		// Each target route names the gate it must be wrapped in, and that
		// gate is the one built for it (not merely some gate).
		want := expectedGateVr6w(e.pattern)
		target := want != ""
		if target && !containsString(e.gates, want) {
			t.Errorf("target route %q must be wrapped in %s; it is wrapped in %v", e.pattern, want, e.gates)
		}
		// A wildcard {action} route lists its actions by name.
		if strings.HasSuffix(e.pattern, "/{action}") {
			if len(wildcardActionsVr6w[e.pattern]) == 0 {
				t.Errorf("route %q takes an {action} wildcard but wildcardActionsVr6w lists no actions for it", e.pattern)
			}
		}
		if _, nt := routeNotTargetVr6w[e.pattern]; nt && (target || machineRouteClassified(e.pattern)) {
			t.Errorf("route %q is both classified by a gate table and listed in routeNotTargetVr6w", e.pattern)
		}
		// A machine route (/api/...) the machine gate wraps must be in its
		// tables; it refuses anything else.
		if _, path, _ := strings.Cut(e.pattern, " "); strings.HasPrefix(path, "/api/") && e.gated && !machineRouteClassified(e.pattern) {
			t.Errorf("gated machine route %q is not listed by machineTargetGate", e.pattern)
		}
	}
	check := func(table string, keys []string, reason func(string) string) {
		for _, p := range keys {
			if !seen[p] && !strings.HasPrefix(reason(p), stackOnlyVr6w) {
				t.Errorf("%s names %q, which is not registered", table, p)
			}
		}
	}
	none := func(string) string { return "" }
	keysOf := func(m map[string]string) []string {
		var out []string
		for k := range m {
			out = append(out, k)
		}
		return out
	}
	var bt, pg []string
	for k := range browserTargetRoutes {
		bt = append(bt, k)
	}
	pg = keysOf(proposalGatedRoutesVr6w)
	var mt []string
	for k := range machineTargetRoutes {
		mt = append(mt, k)
	}
	check("browserTargetRoutes", bt, none)
	check("proposalGatedRoutesVr6w", pg, none)
	check("machineTargetRoutes", mt, none)
	check("machineProposalRoutes", keysOf(machineProposalRoutes), none)
	check("machineQueueRoutesVr6w", keysOf(machineQueueRoutesVr6w), none)
	check("machineActionRoutesVr6w", keysOf(machineActionRoutesVr6w), none)
	var wa []string
	for k := range wildcardActionsVr6w {
		wa = append(wa, k)
	}
	check("wildcardActionsVr6w", wa, none)
	check("machineRecordRoutesVr6w", keysOf(machineRecordRoutesVr6w), func(p string) string { return machineRecordRoutesVr6w[p] })
	check("routeNotTargetVr6w", keysOf(routeNotTargetVr6w), func(p string) string { return routeNotTargetVr6w[p] })
}

func tgGet(t *testing.T, s *Server, as tcActor, path string) *httptest.ResponseRecorder {
	t.Helper()
	req := httptest.NewRequestWithContext(context.Background(), http.MethodGet, path, nil)
	req.Header.Set("X-Forwarded-Email", as.email)
	if as.id != "" {
		req.Header.Set("X-Forwarded-User", as.id)
	}
	req.Header.Set("X-Forwarded-Groups", as.groups)
	rec := httptest.NewRecorder()
	s.Routes().ServeHTTP(rec, req)
	return rec
}

// r2 F1: rotating a fabric credential is capped and stepped up.
func TestTargetGate_rotateCredential(t *testing.T) {
	s, rt := tcServer(t)
	s.Permissions.StepUpRequired = append(s.Permissions.StepUpRequired, "rotate_credential")
	rotated := 0
	portal := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		rotated++
		_ = json.NewEncoder(w).Encode(map[string]string{"slot_id": "slot-1", "system_handle": "x",
			"api_key": "vp_NEW", "bundle_url": "/api/org-bundle/slot-1/onetime.tar.gz", "uri_san": "spiffe://x"})
	}))
	defer portal.Close()
	s.Cfg.PortalInternalURL, s.Cfg.PortalLookupToken, s.Cfg.NotifyPortalURL = portal.URL, "tok", "https://portal.example"
	if rec := tcPost(t, s, tcAsIAM, "/users/"+tcPA+"/rotate-credential?idp=tenant", url.Values{"rotate_nonce": {s.mintRotateNonce()}}); rec.Code != http.StatusForbidden {
		t.Fatalf("IAM-ADMIN rotating a platform admin's credential: %d, want 403", rec.Code)
	}
	rt.stale = true
	wantStepUpRedirect(t, "AAL1 rotate", tcPost(t, s, tcAsIAM, "/users/"+tcU+"/rotate-credential?idp=tenant", url.Values{"rotate_nonce": {s.mintRotateNonce()}}))
	if rotated != 0 {
		t.Fatalf("portal rotated %d credentials", rotated)
	}
	rt.stale = false
	if rec := tcPost(t, s, tcAsIAM, "/users/"+tcU+"/rotate-credential?idp=tenant", url.Values{"rotate_nonce": {s.mintRotateNonce()}}); rec.Code != http.StatusOK || rotated != 1 {
		t.Fatalf("stepped-up IAM-ADMIN rotating a viewer: %d rotated=%d", rec.Code, rotated)
	}
}

// r2 F3: no delete, disable or session revoke above one's rank, browser or
// machine; within rank, the handler's own step-up still applies.
func TestTargetGate_takedowns(t *testing.T) {
	s, rt := tcServer(t)
	for _, c := range []struct {
		path string
		form url.Values
	}{
		{"/users/" + tcPA + "/delete?idp=tenant", url.Values{"confirm_email": {"pa@example.org"}}},
		{"/users/" + tcPA + "/state?idp=tenant", url.Values{"state": {"inactive"}}},
		{"/users/" + tcPA + "/revoke-sessions?idp=tenant", url.Values{}},
		{"/users/" + tcPA + "/email-reference?idp=tenant", url.Values{}},
	} {
		if rec := tcPost(t, s, tcAsIAM, c.path, c.form); rec.Code != http.StatusForbidden {
			t.Fatalf("IAM-ADMIN %s: %d, want 403", c.path, rec.Code)
		}
	}
	if _, ok := rt.ids[tcPA]; !ok || rt.ids[tcPA]["state"] != "active" {
		t.Fatal("platform admin deleted or disabled")
	}
	p := &onboardstore.Proposal{Sponsor: "sponsor-1", Env: "zagros", Handle: "casey", Email: "iam@example.org",
		Status: onboardstore.ProposalOnboarded, ManifestJSON: "{}", Tier: "developer"}
	if err := s.Onboard.CreateProposal(context.Background(), p); err != nil {
		t.Fatal(err)
	}
	s.Cfg.OnboardOperatorAPIKeys["k-casey"] = "casey"
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/api/users/state", strings.NewReader(`{"id":"`+tcPA+`","state":"inactive"}`))
	req.Header.Set("Authorization", "Bearer k-casey")
	w := httptest.NewRecorder()
	s.Routes().ServeHTTP(w, req)
	if w.Code != http.StatusForbidden || rt.ids[tcPA]["state"] != "active" {
		t.Fatalf("IAM-ADMIN's key disabling a platform admin: %d %s", w.Code, w.Body.String())
	}
	rt.stale = true
	wantStepUpRedirect(t, "AAL1 platform admin deleting a platform admin",
		tcPost(t, s, tcAsPA, "/users/"+tcPA2+"/delete?idp=tenant", url.Values{"confirm_email": {"pa2@example.org"}}))
}

// r2 F2: a per-person key whose owner does not resolve cannot raise a puppet
// to the platform tier. (The review drove this with the runner key, which is
// a service key and uncapped by decision, ADR 1071.)
func TestTargetGate_unresolvedKeyCannotGrantPlatformTier(t *testing.T) {
	s, rt := tcServer(t)
	s.Cfg.OnboardOperatorAPIKeys["k-nobody"] = "nobody"
	body := `{"id":"` + tcU + `","groups":["VIEWER","PLATFORM-ADMIN"]}`
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/api/users/groups", strings.NewReader(body))
	req.Header.Set("Authorization", "Bearer k-nobody")
	w := httptest.NewRecorder()
	s.Routes().ServeHTTP(w, req)
	if w.Code != http.StatusForbidden || containsString(rt.groups(tcU), "PLATFORM-ADMIN") {
		t.Fatalf("unresolved key granting PLATFORM-ADMIN: %d groups=%v", w.Code, rt.groups(tcU))
	}
}

// r2 F4: the welcome package is scoped to the sponsor's own proposals and
// takes the mint step-up.
func TestTargetGate_welcomePackage(t *testing.T) {
	s, rt := tcServer(t)
	s.Cfg.NotifyGoatnet = "zagros"
	p := &onboardstore.Proposal{Sponsor: "someone-else@example.org", Env: "zagros", Handle: "u-handle", Email: "u@example.org",
		Status: onboardstore.ProposalOnboarded, ManifestJSON: `{"netbird":{"enabled":false}}`}
	if err := s.Onboard.CreateProposal(context.Background(), p); err != nil {
		t.Fatal(err)
	}
	rt.stale = true
	wantStepUpRedirect(t, "AAL1 welcome download", tgGet(t, s, tcAsIAM, "/onboarding/"+p.ID+"/welcome?idp=tenant"))
	if len(rt.mints) != 0 {
		t.Fatalf("recovery link minted before step-up: %v", rt.mints)
	}
	rt.stale = false
	if rec := tgGet(t, s, tcAsIAM, "/onboarding/"+p.ID+"/welcome?idp=tenant"); rec.Code != http.StatusOK || len(rt.mints) != 1 {
		t.Fatalf("stepped-up admin download: %d mints=%v", rec.Code, rt.mints)
	}
	portal := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_ = json.NewEncoder(w).Encode(map[string]interface{}{"vendor": "acme", "alias": "acme", "sponsor_authorized": true, "sponsor_max_tier": "browse"})
	}))
	defer portal.Close()
	s.Cfg.PortalInternalURL, s.Cfg.PortalLookupToken = portal.URL, "tok"
	if rec := tgGet(t, s, tcActor{"tg-sponsor@acme.example", "", ""}, "/onboarding/"+p.ID+"/welcome?idp=tenant"); rec.Code != http.StatusNotFound || len(rt.mints) != 1 {
		t.Fatalf("sponsor downloading another sponsor's package: %d mints=%v", rec.Code, rt.mints)
	}
}

func TestCeilingServiceHandles(t *testing.T) {
	t.Setenv("ONBOARD_SERVICE_OPERATOR_HANDLES", "svc-a, svc-b")
	for _, h := range []string{" operator ", "onboard-api", "id-console", "onboard-runner", "onboard-runner-zagros", "svc-a", "svc-b"} {
		if !isServiceOperatorKeyHandleVr6w(h) {
			t.Errorf("%q is a service handle", h)
		}
	}
	for _, h := range []string{"", "  ", "casey", "Operator", "onboard-runnerx", "svc-c"} {
		if isServiceOperatorKeyHandleVr6w(h) {
			t.Errorf("%q is not a service handle", h)
		}
	}
}

// A per-person operator key does not act on the platform tier even when its
// owner resolves to a PLATFORM-ADMIN; it keeps its other actions (IAM-ADMIN
// changes are GHSA-7788's rule). The fenced bodies are parsed once, strictly.
func TestTargetGate_perPersonKeyNeverTouchesAdminTier(t *testing.T) {
	s, rt := tcServer(t)
	p := &onboardstore.Proposal{Sponsor: "sponsor-1", Env: "zagros", Handle: "pat", Email: "pa@example.org",
		Status: onboardstore.ProposalOnboarded, ManifestJSON: "{}", Tier: "platform-admin"}
	if err := s.Onboard.CreateProposal(context.Background(), p); err != nil {
		t.Fatal(err)
	}
	s.Cfg.OnboardOperatorAPIKeys["k-pat"] = "pat"
	api := func(path, body string) int {
		req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, path, strings.NewReader(body))
		req.Header.Set("Authorization", "Bearer k-pat")
		w := httptest.NewRecorder()
		s.Routes().ServeHTTP(w, req)
		return w.Code
	}
	for _, c := range []struct{ path, body string }{
		{"/api/users/groups", `{"id":"` + tcU + `","groups":["VIEWER","PLATFORM-ADMIN"]}`},
		{"/api/users/groups", `{"id":"` + tcPA2 + `","groups":[]}`},
		{"/api/users/groups", `{"id":"` + tcPA2 + `","groups":["PLATFORM-ADMIN","DEVELOPER"]}`},
		{"/api/users/state", `{"id":"` + tcPA2 + `","state":"inactive"}`},
	} {
		if code := api(c.path, c.body); code != http.StatusForbidden {
			t.Fatalf("PLATFORM-ADMIN-owned per-person key %s %s: %d, want 403", c.path, c.body, code)
		}
	}
	if code := api("/api/users/groups", `{"id":"`+tcU+`","groups":["VIEWER","DEVELOPER"]}`); code != http.StatusOK {
		t.Fatalf("per-person key, non-admin change: %d", code)
	}
	for _, body := range []string{
		`{"id":"` + tcPA2 + `","state":"inactive"} x`,
		`{"id":"` + tcPA2 + `","state":"inactive","extra":1}`,
	} {
		if code := api("/api/users/state", body); code != http.StatusBadRequest {
			t.Fatalf("state body %q: %d, want 400", body, code)
		}
	}
	if rt.ids[tcPA2]["state"] != "active" || !containsString(rt.groups(tcPA2), "PLATFORM-ADMIN") {
		t.Fatalf("platform admin changed: %v %v", rt.ids[tcPA2]["state"], rt.groups(tcPA2))
	}
}

// vr6w r3 F4: on the operator-key approval path (no browser groups) a
// per-person key may not approve a proposal whose tier or preset grants an
// admin tier; a service key keeps ADR 1071 behaviour.
func TestTargetGate_operatorKeyApprovalCeiling(t *testing.T) {
	s := &Server{}
	req := func(handle string) *http.Request {
		r := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/api/onboard/proposals/x/approve", nil)
		return r.WithContext(context.WithValue(r.Context(), ctxKeyOnboardPrincipal, onboardPrincipal{Handle: handle, IsOperator: true}))
	}
	for _, c := range []struct {
		handle string
		p      onboardstore.Proposal
		refuse bool
	}{
		{"casey", onboardstore.Proposal{Email: "a@example.org", Tier: "platform-admin"}, true},
		{"casey", onboardstore.Proposal{Email: "a@example.org", Tier: "iam-admin"}, true},
		{"casey", onboardstore.Proposal{Email: "a@example.org", Tier: "developer", ManifestJSON: `{"ory_site":{"groups":["IAM-ADMIN"]}}`}, true},
		{"casey", onboardstore.Proposal{Email: "a@example.org", Tier: "developer", ManifestJSON: `not json`}, true},
		{"casey", onboardstore.Proposal{Email: "a@example.org", Tier: "developer"}, false},
		{"onboard-runner-zagros", onboardstore.Proposal{Email: "a@example.org", Tier: "iam-admin"}, false},
		{"onboard-runner-zagros", onboardstore.Proposal{Email: "a@example.org", Tier: "platform-admin"}, true},
	} {
		p := c.p
		e := s.approvalCeiling(req(c.handle), &p, c.handle)
		if (e != nil) != c.refuse {
			t.Errorf("%s approving tier %q manifest %q: refused=%v, want %v (%v)", c.handle, p.Tier, p.ManifestJSON, e != nil, c.refuse, e)
		}
	}
}

// vr6w r3 F5: above the actor's tier the welcome package carries no
// credential at all; mesh-key reissue for such a person is refused.
func TestTargetGate_welcomeWithholdsEveryCredentialAboveTier(t *testing.T) {
	s, rt := tcServer(t)
	s.Cfg.NotifyGoatnet = "zagros"
	p := &onboardstore.Proposal{Sponsor: "someone-else@example.org", Env: "zagros", Handle: "pa-handle", Email: "pa@example.org",
		Status: onboardstore.ProposalOnboarded, Tier: "platform-admin", Escalated: true, SetupKey: "NB-SETUP-SECRET-PA",
		ManifestJSON: `{"netbird":{"enabled":true}}`}
	if err := s.Onboard.CreateProposal(context.Background(), p); err != nil {
		t.Fatal(err)
	}
	if err := s.Onboard.SetProposalResult(context.Background(), p.ID, onboardstore.ProposalOnboarded, "NB-SETUP-SECRET-PA", "NB-HANDSET-SECRET-PA", "", "", "", false); err != nil {
		t.Fatal(err)
	}
	rec := tgGet(t, s, tcAsIAM, "/onboarding/"+p.ID+"/welcome?idp=tenant")
	body := rec.Body.String()
	for _, secret := range []string{"NB-SETUP-SECRET-PA", "NB-HANDSET-SECRET-PA", "self-service/recovery", "TC-CODE"} {
		if strings.Contains(body, secret) {
			t.Errorf("IAM-ADMIN's package for a platform admin carries %q (status %d)", secret, rec.Code)
		}
	}
	if len(rt.mints) != 0 {
		t.Errorf("recovery link minted: %v", rt.mints)
	}
	for _, path := range []string{"/onboarding/" + p.ID + "/device-key/reissue", "/onboarding/" + p.ID + "/handset-key/reissue"} {
		if rec := tcPost(t, s, tcAsIAM, path+"?idp=tenant", url.Values{}); rec.Code != http.StatusForbidden {
			t.Errorf("IAM-ADMIN %s for a platform admin: %d, want 403", path, rec.Code)
		}
	}
	// A platform admin still gets the full package.
	if rec := tgGet(t, s, tcAsPA, "/onboarding/"+p.ID+"/welcome?idp=tenant"); rec.Code != http.StatusOK || !strings.Contains(rec.Body.String(), "NB-SETUP-SECRET-PA") {
		t.Errorf("platform admin's download: %d, setup key present=%v", rec.Code, strings.Contains(rec.Body.String(), "NB-SETUP-SECRET-PA"))
	}
}

// r4 helpers: casey is an IAM-ADMIN holding the per-person key k-casey.
func tgKeyServer(t *testing.T) (*Server, *tcKratos) {
	t.Helper()
	s, rt := tcServer(t)
	s.Cfg.NotifyGoatnet = "zagros"
	p := &onboardstore.Proposal{Sponsor: "sponsor-1", Env: "zagros", Handle: "casey", Email: "iam@example.org",
		Status: onboardstore.ProposalOnboarded, ManifestJSON: "{}", Tier: "iam-admin"}
	if err := s.Onboard.CreateProposal(context.Background(), p); err != nil {
		t.Fatal(err)
	}
	s.Cfg.OnboardOperatorAPIKeys["k-casey"] = "casey"
	s.Cfg.OnboardOperatorAPIKeys["k-runner"] = "onboard-runner"
	return s, rt
}

func tgAPI(t *testing.T, s *Server, method, key, path, body string) *httptest.ResponseRecorder {
	t.Helper()
	req := httptest.NewRequestWithContext(context.Background(), method, path, strings.NewReader(body))
	req.Header.Set("Authorization", "Bearer "+key)
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	s.Routes().ServeHTTP(w, req)
	return w
}

func tgManifest(handle string, groups []string) string {
	var m map[string]any
	_ = json.Unmarshal([]byte(gateCleanManifest), &m)
	m["identity"].(map[string]any)["handle"] = handle
	m["identity"].(map[string]any)["email"] = handle + "@example.org"
	m["ory_site"].(map[string]any)["groups"] = groups
	b, _ := json.Marshal(m)
	return string(b)
}

func tgProposal(t *testing.T, s *Server, handle, status, tier, manifest string) *onboardstore.Proposal {
	t.Helper()
	p := &onboardstore.Proposal{Sponsor: "someone-else@example.org", Env: "zagros", Handle: handle, Email: handle + "@example.org",
		Status: status, Tier: tier, ManifestJSON: manifest, Complete: true, Escalated: true}
	if err := s.Onboard.CreateProposal(context.Background(), p); err != nil {
		t.Fatal(err)
	}
	return p
}

// r4 F1: the approval ceiling ranks what approving would write (the tier's
// groups and the manifest's ory_site.groups), not the tier slug alone.
func TestTierCeiling_approvalRanksManifestGroups(t *testing.T) {
	s, _ := tgKeyServer(t)
	pa := tgManifest("pa-x", []string{"PLATFORM-ADMIN"})
	// Per-person key: a tier-less PLATFORM-ADMIN manifest is refused at both stages.
	p := tgProposal(t, s, "pa-a", onboardstore.ProposalProposed, "", pa)
	if w := tgAPI(t, s, http.MethodPost, "k-casey", "/api/onboard/proposals/"+p.ID+"/sponsor-approve", `{}`); w.Code != http.StatusForbidden {
		t.Fatalf("per-person sponsor-approve of a tier-less PLATFORM-ADMIN manifest: %d %s", w.Code, w.Body.String())
	}
	p = tgProposal(t, s, "pa-b", onboardstore.ProposalSponsorApproved, "developer", pa)
	if w := tgAPI(t, s, http.MethodPost, "k-casey", "/api/onboard/proposals/"+p.ID+"/approve", `{}`); w.Code != http.StatusForbidden {
		t.Fatalf("per-person approve of a developer-tier PLATFORM-ADMIN manifest: %d %s", w.Code, w.Body.String())
	}
	// Browser IAM-ADMIN: refused for a tier-less and a developer-tier PLATFORM-ADMIN manifest.
	asIAM := tcActor{"iam@example.org", tcIAM, "IAM-ADMIN,GRAFANA-READONLY"}
	for _, tier := range []string{"", "developer"} {
		p := tgProposal(t, s, "pa-c"+tier, onboardstore.ProposalSponsorApproved, tier, tgManifest("pa-c"+tier, []string{"PLATFORM-ADMIN"}))
		rec := tcPost(t, s, asIAM, "/onboarding/"+p.ID+"/approve?idp=tenant", url.Values{"relationship": {"assigned"}})
		got, _ := s.Onboard.GetProposal(context.Background(), p.ID)
		if rec.Code != http.StatusForbidden || got.Status != onboardstore.ProposalSponsorApproved {
			t.Fatalf("tier %q: IAM-ADMIN approve of a PLATFORM-ADMIN manifest: %d status %s", tier, rec.Code, got.Status)
		}
	}
	// A service key may not vouch for the platform tier (ADR 1071 as
	// amended) but is uncapped below it; a per-person key keeps non-admin
	// proposals.
	p = tgProposal(t, s, "pa-d", onboardstore.ProposalProposed, "", tgManifest("pa-d", []string{"PLATFORM-ADMIN"}))
	if w := tgAPI(t, s, http.MethodPost, "k-runner", "/api/onboard/proposals/"+p.ID+"/sponsor-approve", `{}`); w.Code != http.StatusForbidden {
		t.Fatalf("service key sponsor-approve of the platform tier: %d %s, want 403", w.Code, w.Body.String())
	}
	p = tgProposal(t, s, "iam-d", onboardstore.ProposalProposed, "", tgManifest("iam-d", []string{"IAM-ADMIN"}))
	if w := tgAPI(t, s, http.MethodPost, "k-runner", "/api/onboard/proposals/"+p.ID+"/sponsor-approve", `{}`); w.Code != http.StatusOK {
		t.Fatalf("service key sponsor-approve of IAM-ADMIN: %d %s", w.Code, w.Body.String())
	}
	p = tgProposal(t, s, "dev-e", onboardstore.ProposalProposed, "developer", tgManifest("dev-e", []string{"DEVELOPER"}))
	if w := tgAPI(t, s, http.MethodPost, "k-casey", "/api/onboard/proposals/"+p.ID+"/sponsor-approve", `{}`); w.Code != http.StatusOK {
		t.Fatalf("per-person sponsor-approve of a developer: %d %s", w.Code, w.Body.String())
	}
}

// r4 F3: nobody queues or marks the access cut of a person above their tier.
func TestTierCeiling_offboardAboveTier(t *testing.T) {
	s, _ := tgKeyServer(t)
	s.Permissions.StepUpRequired = append(s.Permissions.StepUpRequired, "offboard_identity")
	p := tgProposal(t, s, "pa-off", onboardstore.ProposalOnboarded, "platform-admin", "{}")
	rec := tcPost(t, s, tcAsIAM, "/onboarding/"+p.ID+"/offboard-cut?idp=tenant", url.Values{})
	if got, _ := s.Onboard.GetProposal(context.Background(), p.ID); rec.Code != http.StatusForbidden || got.Status != onboardstore.ProposalOnboarded {
		t.Fatalf("IAM-ADMIN offboard-cut of a platform admin: %d status %s", rec.Code, got.Status)
	}
	p2 := tgProposal(t, s, "pa-off2", onboardstore.ProposalOnboarded, "platform-admin", "{}")
	if w := tgAPI(t, s, http.MethodPost, "k-casey", "/api/onboard/proposals/"+p2.ID+"/offboard", `{"note":"bye"}`); w.Code != http.StatusForbidden {
		t.Fatalf("per-person key offboard of a platform admin: %d %s", w.Code, w.Body.String())
	}
	// The person's identity counts too: a developer-tier row for an address
	// that holds PLATFORM-ADMIN in Kratos.
	p3 := &onboardstore.Proposal{Sponsor: "x@example.org", Env: "zagros", Handle: "pa-id", Email: "pa@example.org",
		Status: onboardstore.ProposalOnboarded, Tier: "developer", ManifestJSON: "{}", Complete: true}
	if err := s.Onboard.CreateProposal(context.Background(), p3); err != nil {
		t.Fatal(err)
	}
	if w := tgAPI(t, s, http.MethodPost, "k-casey", "/api/onboard/proposals/"+p3.ID+"/offboard", `{"note":"bye"}`); w.Code != http.StatusForbidden {
		t.Fatalf("per-person key offboard of a platform admin's developer row: %d %s", w.Code, w.Body.String())
	}
	// Within tier, and with a service key, the cut is not refused by the ceiling.
	p4 := tgProposal(t, s, "dev-off", onboardstore.ProposalOnboarded, "developer", "{}")
	if w := tgAPI(t, s, http.MethodPost, "k-casey", "/api/onboard/proposals/"+p4.ID+"/offboard", `{"note":"bye"}`); w.Code == http.StatusForbidden {
		t.Fatalf("per-person key offboard of a developer: 403 %s", w.Body.String())
	}
	if w := tgAPI(t, s, http.MethodPost, "k-runner", "/api/onboard/proposals/"+p2.ID+"/offboard", `{"note":"bye"}`); w.Code == http.StatusForbidden {
		t.Fatalf("service key offboard of a platform admin: 403 %s", w.Body.String())
	}
}

// r4 F2: a per-person key reads no platform-tier person's mesh keys.
func TestTierCeiling_proposalReadsRedactAboveTier(t *testing.T) {
	s, _ := tgKeyServer(t)
	pa := tgProposal(t, s, "pa-keys", onboardstore.ProposalOnboarded, "platform-admin", "{}")
	dev := tgProposal(t, s, "dev-keys", onboardstore.ProposalOnboarded, "developer", "{}")
	for _, p := range []*onboardstore.Proposal{pa, dev} {
		if err := s.Onboard.SetProposalResult(context.Background(), p.ID, onboardstore.ProposalOnboarded, "SETUP-"+p.Handle, "HANDSET-"+p.Handle, "/bundles/"+p.Handle, "", "", false); err != nil {
			t.Fatal(err)
		}
	}
	for _, path := range []string{"/api/onboard/proposals/" + pa.ID, "/api/onboard/proposals?status=onboarded"} {
		w := tgAPI(t, s, http.MethodGet, "k-casey", path, "")
		if w.Code != http.StatusOK || strings.Contains(w.Body.String(), "SETUP-pa-keys") || strings.Contains(w.Body.String(), "HANDSET-pa-keys") || strings.Contains(w.Body.String(), "/bundles/pa-keys") {
			t.Fatalf("per-person key read %s: %d, platform admin credentials present", path, w.Code)
		}
		if strings.Contains(path, "?") && !strings.Contains(w.Body.String(), "SETUP-dev-keys") {
			t.Fatalf("per-person key list lost a developer's setup key")
		}
	}
	if w := tgAPI(t, s, http.MethodGet, "k-runner", "/api/onboard/proposals/"+pa.ID, ""); !strings.Contains(w.Body.String(), "SETUP-pa-keys") {
		t.Fatalf("service key read of a platform admin row: %d, credentials missing", w.Code)
	}
}

// r4 coverage: a route the gate wraps but does not classify is refused at
// runtime (default deny), browser and machine.
func TestTargetGate_unclassifiedRouteRefused(t *testing.T) {
	s, rt := tcServer(t)
	mux := http.NewServeMux()
	mux.HandleFunc("POST /identities/{id}/state", s.adminGate(s.targetGate(s.handleUserPatchState)))
	mux.HandleFunc("POST /api/identities/state", s.onboardAPIGate(s.machineTargetGate(s.apiPatchUserState)))
	form := url.Values{"state": {"inactive"}, "csrf": {s.csrfNonce}}
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/identities/"+tcPA+"/state?idp=tenant", strings.NewReader(form.Encode()))
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	req.Header.Set("X-Forwarded-Email", tcAsIAM.email)
	req.Header.Set("X-Forwarded-User", tcAsIAM.id)
	req.Header.Set("X-Forwarded-Groups", tcAsIAM.groups)
	rec := httptest.NewRecorder()
	mux.ServeHTTP(rec, req)
	if rec.Code != http.StatusForbidden || rt.ids[tcPA]["state"] != "active" {
		t.Fatalf("unclassified browser route: %d state=%v, want 403 and active", rec.Code, rt.ids[tcPA]["state"])
	}
	s.Cfg.OnboardOperatorAPIKeys["k-runner"] = "onboard-runner"
	req = httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/api/identities/state", strings.NewReader(`{"id":"`+tcPA+`","state":"inactive"}`))
	req.Header.Set("Authorization", "Bearer k-runner")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	mux.ServeHTTP(w, req)
	if w.Code != http.StatusForbidden || rt.ids[tcPA]["state"] != "active" {
		t.Fatalf("unclassified machine route: %d state=%v, want 403 and active", w.Code, rt.ids[tcPA]["state"])
	}
}
