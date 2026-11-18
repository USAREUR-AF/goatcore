package handlers

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/kratos"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/permissions"
)

const (
	r7IAM  = "a1a1a1a1-1111-4111-8111-11111111abcd"
	r7PA   = "22222222-2222-4222-8222-222222222222"
	r7PA2  = "33333333-3333-4333-8333-333333333333"
	r7IAM2 = "66666666-6666-4666-8666-666666666666"
	r7U    = "55555555-5555-4555-8555-555555555555"
)

// r7Kratos is a multi-identity fake Kratos admin API. Every identity's
// session is aal2 and fresh unless stale is set, in which case it is aal1
// and the actor's step-up challenge is issued.
type r7Kratos struct {
	ids     map[string]map[string]interface{}
	mints   []string
	created int
	stale   bool
}

func r7Ident(id, email string, groups ...string) map[string]interface{} {
	gi := make([]interface{}, len(groups))
	for i, g := range groups {
		gi[i] = g
	}
	return map[string]interface{}{
		"id": id, "state": "active",
		"traits":          map[string]interface{}{"email": email, "name": email},
		"metadata_public": map[string]interface{}{"groups": gi},
	}
}

func (rt *r7Kratos) RoundTrip(req *http.Request) (*http.Response, error) {
	w := httptest.NewRecorder()
	w.Header().Set("Content-Type", "application/json")
	p := req.URL.Path
	// Kratos resolves an identity UUID in any case.
	seg := strings.Split(strings.ToLower(strings.TrimPrefix(p, "/admin/identities/")), "/")
	switch {
	case req.Method == http.MethodGet && p == "/admin/identities":
		all := []interface{}{}
		if req.URL.Query().Get("page") == "0" {
			for _, v := range rt.ids {
				all = append(all, v)
			}
		}
		_ = json.NewEncoder(w).Encode(all)
	case req.Method == http.MethodPost && p == "/admin/identities":
		var body map[string]interface{}
		_ = json.NewDecoder(req.Body).Decode(&body)
		rt.created++
		id := fmt.Sprintf("44444444-4444-4444-4444-%012d", rt.created)
		body["id"], body["state"] = id, "active"
		rt.ids[id] = body
		_ = json.NewEncoder(w).Encode(body)
	case req.Method == http.MethodPost && strings.HasPrefix(p, "/admin/recovery/"):
		var body map[string]string
		_ = json.NewDecoder(req.Body).Decode(&body)
		rt.mints = append(rt.mints, body["identity_id"])
		_ = json.NewEncoder(w).Encode(map[string]interface{}{
			"recovery_link": "https://auth.example/self-service/recovery?flow=tc-" + body["identity_id"],
			"recovery_code": "TC-CODE",
			"expires_at":    time.Now().Add(time.Hour).UTC().Format(time.RFC3339),
		})
	case len(seg) >= 2 && seg[1] == "sessions" && req.Method == http.MethodGet:
		if req.URL.Query().Get("page") != "1" {
			_ = json.NewEncoder(w).Encode([]interface{}{})
			return w.Result(), nil
		}
		now := time.Now().UTC()
		aal, methods := "aal2", []interface{}{
			map[string]interface{}{"method": "password", "aal": "aal1", "completed_at": now.Format(time.RFC3339)},
			map[string]interface{}{"method": "totp", "aal": "aal2", "completed_at": now.Format(time.RFC3339)},
		}
		if rt.stale {
			aal, methods = "aal1", methods[:1]
		}
		_ = json.NewEncoder(w).Encode([]interface{}{map[string]interface{}{
			"id": "sess-" + seg[0], "active": true,
			"expires_at": now.Add(time.Hour).Format(time.RFC3339), "authenticated_at": now.Format(time.RFC3339),
			"authenticator_assurance_level": aal, "authentication_methods": methods,
		}})
	case len(seg) >= 2 && (seg[1] == "sessions" || seg[1] == "credentials") && req.Method == http.MethodDelete:
		w.WriteHeader(http.StatusNoContent)
	case strings.HasPrefix(p, "/admin/identities/") && len(seg) == 1:
		cur, ok := rt.ids[seg[0]]
		if !ok {
			w.WriteHeader(http.StatusNotFound)
			return w.Result(), nil
		}
		if req.Method == http.MethodPatch {
			var ops []map[string]interface{}
			_ = json.NewDecoder(req.Body).Decode(&ops)
			for _, op := range ops {
				switch op["path"] {
				case "/metadata_public/groups":
					cur["metadata_public"].(map[string]interface{})["groups"] = op["value"]
				case "/traits/groups":
					cur["traits"].(map[string]interface{})["groups"] = op["value"]
				case "/state":
					cur["state"] = op["value"]
				}
			}
		}
		_ = json.NewEncoder(w).Encode(cur)
	default:
		w.WriteHeader(http.StatusNotFound)
	}
	return w.Result(), nil
}

func (rt *r7Kratos) groups(id string) []string {
	arr, _ := rt.ids[id]["metadata_public"].(map[string]interface{})["groups"].([]interface{})
	var out []string
	for _, g := range arr {
		out = append(out, fmt.Sprint(g))
	}
	return out
}

type r7Actor struct{ email, id, groups string }

var (
	r7AsIAM = r7Actor{"iam@example.org", r7IAM, "IAM-ADMIN"}
	r7AsPA  = r7Actor{"pa@example.org", r7PA, "PLATFORM-ADMIN"}
)

func r7Post(t *testing.T, s *Server, as r7Actor, path string, form url.Values) *httptest.ResponseRecorder {
	t.Helper()
	form.Set("csrf", s.csrfNonce)
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, path, strings.NewReader(form.Encode()))
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	req.Header.Set("X-Forwarded-Email", as.email)
	if as.id != "" {
		req.Header.Set("X-Forwarded-User", as.id)
	}
	req.Header.Set("X-Forwarded-Groups", as.groups)
	rec := httptest.NewRecorder()
	s.Routes().ServeHTTP(rec, req)
	return rec
}

func r7Server(t *testing.T) (*Server, *r7Kratos) {
	t.Helper()
	rt := &r7Kratos{ids: map[string]map[string]interface{}{
		r7IAM:  r7Ident(r7IAM, "iam@example.org", "IAM-ADMIN"),
		r7PA:   r7Ident(r7PA, "pa@example.org", "PLATFORM-ADMIN"),
		r7PA2:  r7Ident(r7PA2, "pa2@example.org", "PLATFORM-ADMIN"),
		r7U:    r7Ident(r7U, "u@example.org", "VIEWER"),
		r7IAM2: r7Ident(r7IAM2, "iam2@example.org", "IAM-ADMIN"),
	}}
	cli := kratos.NewClient("https://kratos.example.test")
	cli.HTTP = &http.Client{Transport: rt}
	s := newUsersAPITestServer(t, cli)
	s.Permissions.Groups = append(s.Permissions.Groups, permissions.Group{Name: "VIEWER"}, permissions.Group{Name: "DEVELOPER"})
	s.Permissions.StepUpRequired = append(s.Permissions.StepUpRequired,
		"grant_PLATFORM-ADMIN", "revoke_PLATFORM-ADMIN", "delete_identity", "revoke_sessions_of_admin")
	return s, rt
}

func wantStepUpRedirect7788(t *testing.T, what string, rec *httptest.ResponseRecorder) {
	t.Helper()
	if rec.Code != http.StatusSeeOther || !strings.HasPrefix(rec.Header().Get("Location"), "/step-up?nonce=") {
		t.Fatalf("%s: status %d Location %q, want 303 to /step-up?nonce=", what, rec.Code, rec.Header().Get("Location"))
	}
}

// Regression tests for the GHSA-7788 review: each was an exploit that
// succeeded against the first fix.

// Finding 1: an upper-cased spelling of one's own UUID and a session email
// that no longer matches the identity's traits.email do not hide the actor.
func TestRV7788_SelfGrantBypass_UppercaseIDAndStaleSessionEmail(t *testing.T) {
	s, rt := r7Server(t)
	stale := r7Actor{"old-address@example.org", r7IAM, "IAM-ADMIN"}
	for _, id := range []string{r7IAM, strings.ToUpper(r7IAM)} {
		rec := r7Post(t, s, stale, "/users/"+id+"/groups?idp=tenant", url.Values{"groups": {"IAM-ADMIN,PLATFORM-ADMIN"}})
		if rec.Code != http.StatusForbidden {
			t.Fatalf("self grant via %s with a stale session email: %d, want 403", id, rec.Code)
		}
	}
	if g := rt.groups(r7IAM); containsString(g, "PLATFORM-ADMIN") {
		t.Fatalf("self grant landed: %v", g)
	}
}

// Finding 2: an IAM-ADMIN cannot mint a platform admin's recovery link, and
// nobody mints one for an admin-bearing identity without step-up.
func TestRV7788_MintRecoveryForPlatformAdmin_NoStepUp(t *testing.T) {
	s, rt := r7Server(t)
	rt.stale = true
	for _, path := range []string{"/users/" + r7PA + "/recovery-link?idp=tenant", "/users/" + r7PA + "/reset-2fa?idp=tenant"} {
		if rec := r7Post(t, s, r7AsIAM, path, url.Values{}); rec.Code != http.StatusForbidden {
			t.Fatalf("AAL1 IAM-ADMIN %s on a platform admin: %d, want 403", path, rec.Code)
		}
	}
	rt.stale = false
	if rec := r7Post(t, s, r7AsIAM, "/users/"+r7PA+"/recovery-link?idp=tenant", url.Values{}); rec.Code != http.StatusForbidden {
		t.Fatalf("stepped-up IAM-ADMIN minting a platform admin's link: %d, want 403", rec.Code)
	}
	rt.stale = true
	wantStepUpRedirect7788(t, "AAL1 platform admin minting an IAM-ADMIN's link",
		r7Post(t, s, r7AsPA, "/users/"+r7IAM2+"/recovery-link?idp=tenant", url.Values{}))
	if len(rt.mints) != 0 {
		t.Fatalf("recovery links minted: %v", rt.mints)
	}
	rt.stale = false
	if rec := r7Post(t, s, r7AsPA, "/users/"+r7IAM2+"/recovery-link?idp=tenant", url.Values{}); rec.Code != http.StatusOK || len(rt.mints) != 1 {
		t.Fatalf("stepped-up platform admin minting an IAM-ADMIN's link: %d mints=%v", rec.Code, rt.mints)
	}
}

// The tier rule on group changes: an IAM-ADMIN cannot demote, edit or make
// a platform admin.
func TestRV7788_IAMAdminCannotReachPlatformAdminGroups(t *testing.T) {
	s, rt := r7Server(t)
	for _, c := range []struct{ id, groups string }{{r7PA, ""}, {r7PA, "PLATFORM-ADMIN,IAM-ADMIN"}, {r7U, "VIEWER,PLATFORM-ADMIN"}} {
		if rec := r7Post(t, s, r7AsIAM, "/users/"+c.id+"/groups?idp=tenant", url.Values{"groups": {c.groups}}); rec.Code != http.StatusForbidden {
			t.Fatalf("IAM-ADMIN setting %s to %q: %d, want 403", c.id, c.groups, rec.Code)
		}
	}
	if g := rt.groups(r7PA); len(g) != 1 || g[0] != "PLATFORM-ADMIN" {
		t.Fatalf("platform admin's groups changed: %v", g)
	}
	if g := rt.groups(r7U); containsString(g, "PLATFORM-ADMIN") {
		t.Fatalf("PLATFORM-ADMIN granted by an IAM-ADMIN: %v", g)
	}
	if rec := r7Post(t, s, r7AsPA, "/users/"+r7U+"/groups?idp=tenant", url.Values{"groups": {"VIEWER,IAM-ADMIN"}}); rec.Code != http.StatusSeeOther {
		t.Fatalf("platform admin granting IAM-ADMIN: %d, want 303; %s", rec.Code, rec.Body.String())
	}
}

// Finding 3: no admin-bearing identity is created or invited directly, at
// any tier, so no creator is ever shown a new admin login.
func TestRV7788_IAMAdminCreatesPlatformAdminPuppet(t *testing.T) {
	s, rt := r7Server(t)
	for _, as := range []r7Actor{r7AsIAM, r7AsPA} {
		for _, g := range []string{"PLATFORM-ADMIN", "IAM-ADMIN"} {
			rec := r7Post(t, s, as, "/users/new?idp=tenant", url.Values{"email": {"puppet@example.org"}, "groups": {g}})
			if rec.Code != http.StatusForbidden {
				t.Fatalf("%s creating %s: %d, want 403", as.groups, g, rec.Code)
			}
		}
	}
	if rt.created != 0 || len(rt.mints) != 0 {
		t.Fatalf("created=%d mints=%v", rt.created, rt.mints)
	}
}

func TestRV7788_IAMAdminMintsPlatformAdminInvite(t *testing.T) {
	s, _ := r7Server(t)
	for _, as := range []r7Actor{r7AsIAM, r7AsPA} {
		for _, g := range []string{"PLATFORM-ADMIN", "IAM-ADMIN"} {
			rec := r7Post(t, s, as, "/invites?idp=tenant", url.Values{"preset_groups": {g}, "ttl_hours": {"24"}, "email": {"puppet@example.org"}})
			if rec.Code != http.StatusForbidden {
				t.Fatalf("%s inviting %s: %d, want 403", as.groups, g, rec.Code)
			}
		}
	}
}

// Negative control: the IAM-ADMIN paths the first fix gated still challenge
// a password-only actor, and a non-admin change is not challenged.
func TestRV7788_IAMAdminPathsChallengeAtAAL1(t *testing.T) {
	s, rt := r7Server(t)
	s.Permissions.StepUpRequired = append(s.Permissions.StepUpRequired, "grant_ADMINS", "revoke_ADMINS")
	rt.stale = true
	wantStepUpRedirect7788(t, "revoke IAM-ADMIN", r7Post(t, s, r7AsPA, "/users/"+r7IAM2+"/groups?idp=tenant", url.Values{"groups": {""}}))
	wantStepUpRedirect7788(t, "grant IAM-ADMIN", r7Post(t, s, r7AsPA, "/users/"+r7U+"/groups?idp=tenant", url.Values{"groups": {"VIEWER,IAM-ADMIN"}}))
	wantStepUpRedirect7788(t, "revoke an IAM-ADMIN's sessions", r7Post(t, s, r7AsPA, "/users/"+r7IAM2+"/revoke-sessions?idp=tenant", url.Values{}))
	wantStepUpRedirect7788(t, "disable an IAM-ADMIN", r7Post(t, s, r7AsPA, "/users/"+r7IAM2+"/state?idp=tenant", url.Values{"state": {"inactive"}}))
	if rec := r7Post(t, s, r7AsIAM, "/users/"+r7U+"/groups?idp=tenant", url.Values{"groups": {"VIEWER,DEVELOPER"}}); rec.Code != http.StatusSeeOther {
		t.Fatalf("non-admin change by an AAL1 IAM-ADMIN: %d, want 303", rec.Code)
	}
}

// --- r2 review: the operator-key path and the take-down handlers ---------

func r7API(t *testing.T, s *Server, key, path, body string) *httptest.ResponseRecorder {
	t.Helper()
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, path, strings.NewReader(body))
	req.Header.Set("Authorization", "Bearer "+key)
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	s.Routes().ServeHTTP(w, req)
	return w
}

// r7OwnKey files an onboarding record for handle at email and gives the
// handle a per-person operator key.
func r7OwnKey(t *testing.T, s *Server, key, handle, email string) {
	t.Helper()
	p := &onboardstore.Proposal{Sponsor: "sponsor-1", Env: "zagros", Handle: handle, Email: email,
		Status: onboardstore.ProposalOnboarded, ManifestJSON: "{}", Tier: "developer"}
	if err := s.Onboard.CreateProposal(context.Background(), p); err != nil {
		t.Fatal(err)
	}
	s.Cfg.OnboardOperatorAPIKeys[key] = handle
}

// r3 decision: a per-person operator key never grants or revokes an
// admin-bearing group and never acts on an identity holding one, whatever
// its owner resolves to (an IAM-ADMIN, a PLATFORM-ADMIN, or nobody). It keeps
// its non-admin actions. A service key is uncapped (ADR 1071).
func TestRV7788r3_PerPersonKeyNeverTouchesAdminIdentities(t *testing.T) {
	s, rt := r7Server(t)
	r7OwnKey(t, s, "k-casey", "casey", "iam@example.org")
	r7OwnKey(t, s, "k-pat", "pat", "pa@example.org") // owner resolves to a PLATFORM-ADMIN
	s.Cfg.OnboardOperatorAPIKeys["k-nobody"] = "nobody"
	refused := []struct{ path, body string }{
		{"/api/users/groups", `{"id":"` + r7U + `","groups":["VIEWER","PLATFORM-ADMIN"]}`},
		{"/api/users/groups", `{"id":"` + r7U + `","groups":["VIEWER","IAM-ADMIN"]}`},
		{"/api/users/groups", `{"id":"` + r7IAM2 + `","groups":[]}`},
		{"/api/users/groups", `{"id":"` + r7PA2 + `","groups":["PLATFORM-ADMIN","DEVELOPER"]}`},
		{"/api/users/state", `{"id":"` + r7PA2 + `","state":"inactive"}`},
		{"/api/users/state", `{"id":"` + r7IAM2 + `","state":"active"}`},
	}
	for _, key := range []string{"k-casey", "k-pat", "k-nobody"} {
		for _, c := range refused {
			if w := r7API(t, s, key, c.path, c.body); w.Code != http.StatusForbidden {
				t.Fatalf("%s %s %s: %d %s, want 403", key, c.path, c.body, w.Code, w.Body.String())
			}
		}
	}
	if g := rt.groups(r7U); containsString(g, "PLATFORM-ADMIN") || containsString(g, "IAM-ADMIN") {
		t.Fatalf("admin group granted by a per-person key: %v", g)
	}
	if rt.ids[r7PA2]["state"] != "active" || len(rt.groups(r7IAM2)) == 0 {
		t.Fatalf("admin identity changed by a per-person key: pa2 state=%v iam2 groups=%v", rt.ids[r7PA2]["state"], rt.groups(r7IAM2))
	}
	// Non-admin work still goes through.
	if w := r7API(t, s, "k-casey", "/api/users/groups", `{"id":"`+r7U+`","groups":["VIEWER","DEVELOPER"]}`); w.Code != http.StatusOK {
		t.Fatalf("per-person key, non-admin group change: %d %s", w.Code, w.Body.String())
	}
	if w := r7API(t, s, "k-casey", "/api/users/state", `{"id":"`+r7U+`","state":"inactive"}`); w.Code != http.StatusOK {
		t.Fatalf("per-person key offboarding a non-admin: %d %s", w.Code, w.Body.String())
	}
	// A service key keeps the ADR 1071 behaviour, except the platform tier
	// (revision 8): it grants IAM-ADMIN but not PLATFORM-ADMIN.
	s.Cfg.OnboardOperatorAPIKeys["k-svc"] = "onboard-runner-zagros"
	if w := r7API(t, s, "k-svc", "/api/users/groups", `{"id":"`+r7U+`","groups":["VIEWER","IAM-ADMIN"]}`); w.Code != http.StatusOK {
		t.Fatalf("service key granting IAM-ADMIN: %d %s", w.Code, w.Body.String())
	}
}

// r3 finding 3: the fenced machine routes parse their body once, strictly;
// trailing data or an unknown field is refused before anything runs.
func TestRV7788r3_MachineBodyParsedOnceStrictly(t *testing.T) {
	s, rt := r7Server(t)
	r7OwnKey(t, s, "k-casey", "casey", "iam@example.org")
	s.Cfg.OnboardOperatorAPIKeys["k-svc"] = "operator"
	for _, key := range []string{"k-casey", "k-svc"} {
		for _, body := range []string{
			`{"id":"` + r7PA2 + `","state":"inactive"} x`,
			`{"id":"` + r7PA2 + `","state":"inactive"}{"id":"` + r7U + `"}`,
			`{"id":"` + r7PA2 + `","state":"inactive","extra":1}`,
			`{"id":"` + r7PA2 + `","reason":"x","state":"inactive"}`,
		} {
			if w := r7API(t, s, key, "/api/users/state", body); w.Code != http.StatusBadRequest {
				t.Fatalf("%s state body %q: %d, want 400", key, body, w.Code)
			}
		}
	}
	if rt.ids[r7PA2]["state"] != "active" {
		t.Fatalf("platform admin disabled: %v", rt.ids[r7PA2]["state"])
	}
}

// r3 findings 1 and 2 as regressions: an intake row filed by the key holder
// (naming a platform admin's address, or an address nobody holds) changes
// nothing, because no decision reads the key's owner any more.
func TestRV7788r3_ForgedIntakeRowChangesNothing(t *testing.T) {
	for _, email := range []string{"pa@example.org", "nobody-holds-this@example.org"} {
		s, rt := r7Server(t)
		r7OwnKey(t, s, "k-casey", "casey", "iam@example.org")
		body := `{"env":"zagros","manifest":` + strings.Replace(strings.Replace(gateCleanManifest,
			`"email":"pat@demoforge.example"`, `"email":"`+email+`"`, 1), `"handle":"pat-vendor"`, `"handle":"casey"`, 1) + `}`
		// Another fix in the stack may refuse the forged row itself (409
		// handle_taken); either way the key must gain nothing from it.
		if w := r7API(t, s, "k-casey", "/api/onboard/intake", body); w.Code != http.StatusCreated && w.Code != http.StatusConflict {
			t.Fatalf("intake: %d %s", w.Code, w.Body.String())
		}
		for _, c := range []struct{ path, body string }{
			{"/api/users/groups", `{"id":"` + r7U + `","groups":["VIEWER","PLATFORM-ADMIN"]}`},
			{"/api/users/groups", `{"id":"` + r7PA2 + `","groups":[]}`},
			{"/api/users/state", `{"id":"` + r7PA2 + `","state":"inactive"}`},
		} {
			if w := r7API(t, s, "k-casey", c.path, c.body); w.Code != http.StatusForbidden {
				t.Fatalf("after an intake row naming %s, %s %s: %d, want 403", email, c.path, c.body, w.Code)
			}
		}
		if containsString(rt.groups(r7U), "PLATFORM-ADMIN") || rt.ids[r7PA2]["state"] != "active" || len(rt.groups(r7PA2)) == 0 {
			t.Fatalf("changed: u=%v pa2=%v/%v", rt.groups(r7U), rt.groups(r7PA2), rt.ids[r7PA2]["state"])
		}
	}
}

// r2 finding 3: nobody disables, revokes the sessions of, or deletes an
// identity above their tier, on the browser or the machine path; within
// tier it takes step-up.
func TestRV7788r2_NoTakedownAboveTier(t *testing.T) {
	s, rt := r7Server(t)
	for _, c := range []struct {
		path string
		form url.Values
	}{
		{"/users/" + r7PA + "/revoke-sessions?idp=tenant", url.Values{}},
		{"/users/" + r7PA + "/state?idp=tenant", url.Values{"state": {"inactive"}}},
		{"/users/" + r7PA2 + "/delete?idp=tenant", url.Values{"confirm_email": {"pa2@example.org"}}},
	} {
		if rec := r7Post(t, s, r7AsIAM, c.path, c.form); rec.Code != http.StatusForbidden {
			t.Fatalf("IAM-ADMIN %s on a platform admin: %d, want 403", c.path, rec.Code)
		}
	}
	if rt.ids[r7PA]["state"] != "active" {
		t.Fatalf("platform admin disabled: %v", rt.ids[r7PA]["state"])
	}
	r7OwnKey(t, s, "k-casey", "casey", "iam@example.org")
	if w := r7API(t, s, "k-casey", "/api/users/state", `{"id":"`+r7PA+`","state":"inactive"}`); w.Code != http.StatusForbidden || rt.ids[r7PA]["state"] != "active" {
		t.Fatalf("IAM-ADMIN's key disabling a platform admin: %d state=%v", w.Code, rt.ids[r7PA]["state"])
	}
	rt.stale = true
	wantStepUpRedirect7788(t, "AAL1 platform admin deleting an IAM-ADMIN",
		r7Post(t, s, r7AsPA, "/users/"+r7IAM2+"/delete?idp=tenant", url.Values{"confirm_email": {"iam2@example.org"}}))
}

func TestServiceOperatorKeyHandles(t *testing.T) {
	t.Setenv("ONBOARD_SERVICE_OPERATOR_HANDLES", "svc-a, svc-b")
	for _, h := range []string{" operator ", "onboard-api", "id-console", "onboard-runner", "onboard-runner-zagros", "svc-a", "svc-b"} {
		if !isServiceOperatorKeyHandle7788(h) {
			t.Errorf("%q is a service handle", h)
		}
	}
	for _, h := range []string{"", "  ", "casey", "Operator", "onboard-runnerx", "svc-c"} {
		if isServiceOperatorKeyHandle7788(h) {
			t.Errorf("%q is not a service handle", h)
		}
	}
}

// r4: the onboarding approve pipeline is a door that mints an admin login
// (the runner writes the manifest's ory_site.groups). A per-person operator
// key may not advance a proposal that would create an admin-bearing
// identity, by its tier or by its manifest groups; a service key may; a
// browser admin may not advance one above their own admin tier.
func r7Proposal(t *testing.T, s *Server, handle, status, tier string, groups []string) *onboardstore.Proposal {
	t.Helper()
	mj := "{}"
	if groups != nil {
		var m map[string]any
		_ = json.Unmarshal([]byte(gateCleanManifest), &m)
		m["identity"].(map[string]any)["handle"] = handle
		m["identity"].(map[string]any)["email"] = handle + "@example.org"
		m["ory_site"].(map[string]any)["groups"] = groups
		b, _ := json.Marshal(m)
		mj = string(b)
	}
	p := &onboardstore.Proposal{Sponsor: "someone-else@example.org", Env: "zagros", Handle: handle, Email: handle + "@example.org",
		Status: status, Tier: tier, ManifestJSON: mj, Complete: true}
	if err := s.Onboard.CreateProposal(context.Background(), p); err != nil {
		t.Fatal(err)
	}
	return p
}

func TestRV7788r4_PerPersonKeyCannotAdvanceAdminProposal(t *testing.T) {
	s, _ := r7Server(t)
	r7OwnKey(t, s, "k-casey", "casey", "iam@example.org")
	s.Cfg.OnboardOperatorAPIKeys["k-svc"] = "onboard-runner-zagros"
	for _, c := range []struct {
		name, handle, status, tier, stage string
		groups                            []string
	}{
		{"platform-admin tier", "pa-tier", onboardstore.ProposalProposed, "platform-admin", "sponsor-approve", nil},
		{"tier-less PLATFORM-ADMIN manifest", "pa-man", onboardstore.ProposalProposed, "", "sponsor-approve", []string{"PLATFORM-ADMIN"}},
		{"developer tier with ADMINS manifest", "adm-man", onboardstore.ProposalSponsorApproved, "developer", "approve", []string{"ADMINS"}},
		{"tier-less IAM-ADMIN manifest", "iam-man", onboardstore.ProposalSponsorApproved, "", "approve", []string{"IAM-ADMIN"}},
	} {
		p := r7Proposal(t, s, c.handle, c.status, c.tier, c.groups)
		w := r7API(t, s, "k-casey", "/api/onboard/proposals/"+p.ID+"/"+c.stage, `{}`)
		got, _ := s.Onboard.GetProposal(context.Background(), p.ID)
		if w.Code != http.StatusForbidden || got.Status != c.status {
			t.Fatalf("%s: per-person key %s: %d %s, status %s; want 403 and unchanged", c.name, c.stage, w.Code, w.Body.String(), got.Status)
		}
	}
	// A service key vouches for an IAM-ADMIN row; the platform tier is the
	// console's alone (ADR 1071 as amended).
	p := r7Proposal(t, s, "iam-svc", onboardstore.ProposalProposed, "", []string{"IAM-ADMIN"})
	if w := r7API(t, s, "k-svc", "/api/onboard/proposals/"+p.ID+"/sponsor-approve", `{}`); w.Code != http.StatusOK {
		t.Fatalf("service key sponsor-approve of an IAM-ADMIN row: %d %s, want 200", w.Code, w.Body.String())
	}
	p = r7Proposal(t, s, "pa-svc", onboardstore.ProposalProposed, "", []string{"PLATFORM-ADMIN"})
	if w := r7API(t, s, "k-svc", "/api/onboard/proposals/"+p.ID+"/sponsor-approve", `{}`); w.Code != http.StatusForbidden {
		t.Fatalf("service key sponsor-approve of a PLATFORM-ADMIN row: %d %s, want 403", w.Code, w.Body.String())
	}
	// A per-person key keeps non-admin proposals.
	p = r7Proposal(t, s, "dev", onboardstore.ProposalProposed, "developer", []string{"DEVELOPER"})
	if w := r7API(t, s, "k-casey", "/api/onboard/proposals/"+p.ID+"/sponsor-approve", `{}`); w.Code != http.StatusOK {
		t.Fatalf("per-person key sponsor-approve of a developer: %d %s, want 200", w.Code, w.Body.String())
	}
	// An unreadable manifest counts as admin-bearing.
	bad := &onboardstore.Proposal{Sponsor: "x@example.org", Env: "zagros", Handle: "bad", Email: "bad@example.org",
		Status: onboardstore.ProposalProposed, ManifestJSON: "{not json", Complete: true}
	if err := s.Onboard.CreateProposal(context.Background(), bad); err != nil {
		t.Fatal(err)
	}
	if w := r7API(t, s, "k-casey", "/api/onboard/proposals/"+bad.ID+"/sponsor-approve", `{}`); w.Code != http.StatusForbidden {
		t.Fatalf("per-person key on an unreadable manifest: %d, want 403", w.Code)
	}
}

func TestRV7788r4_BrowserApprovalRanksManifestGroups(t *testing.T) {
	s, _ := r7Server(t)
	asIAM := r7Actor{"iam@example.org", r7IAM, "IAM-ADMIN,GRAFANA-READONLY"}
	for _, tier := range []string{"", "developer"} {
		p := r7Proposal(t, s, "puppet-"+tier+"x", onboardstore.ProposalSponsorApproved, tier, []string{"PLATFORM-ADMIN"})
		rec := r7Post(t, s, asIAM, "/onboarding/"+p.ID+"/approve?idp=tenant", url.Values{"relationship": {"assigned"}})
		got, _ := s.Onboard.GetProposal(context.Background(), p.ID)
		if rec.Code != http.StatusForbidden || got.Status != onboardstore.ProposalSponsorApproved {
			t.Fatalf("tier %q: IAM-ADMIN approving a PLATFORM-ADMIN manifest: %d, status %s; want 403", tier, rec.Code, got.Status)
		}
	}
	// A platform admin may approve it.
	p := r7Proposal(t, s, "pa-ok", onboardstore.ProposalSponsorApproved, "", []string{"PLATFORM-ADMIN"})
	if rec := r7Post(t, s, r7AsPA, "/onboarding/"+p.ID+"/approve?idp=tenant", url.Values{"relationship": {"assigned"}}); rec.Code == http.StatusForbidden {
		t.Fatalf("platform admin approving a PLATFORM-ADMIN manifest: 403 %s", rec.Body.String())
	}
}
