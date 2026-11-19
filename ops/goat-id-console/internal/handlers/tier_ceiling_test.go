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

func TestAdminTierRank(t *testing.T) {
	if adminTierRank([]string{"PLATFORM-ADMIN"}) <= adminTierRank([]string{"IAM-ADMIN"}) {
		t.Fatal("PLATFORM-ADMIN must outrank IAM-ADMIN")
	}
	if adminTierRank([]string{"ADMINS"}) != adminTierRank([]string{"PLATFORM-ADMIN"}) {
		t.Fatal("ADMINS must rank with PLATFORM-ADMIN")
	}
	if adminTierRank([]string{"IAM-ADMIN"}) <= adminTierRank([]string{"VIEWER"}) {
		t.Fatal("IAM-ADMIN must outrank a non-admin")
	}
}

// --- multi-identity harness ---------------------------------------------

const (
	tcIAM = "11111111-1111-1111-1111-111111111111"
	tcPA  = "22222222-2222-2222-2222-222222222222"
	tcPA2 = "33333333-3333-3333-3333-333333333333"
	tcU   = "55555555-5555-5555-5555-555555555555"
)

// tcKratos is a multi-identity fake Kratos admin API. Every identity's
// session is aal2 and fresh unless stale is set, in which case it is aal1
// and the actor's step-up challenge is issued.
type tcKratos struct {
	ids     map[string]map[string]interface{}
	mints   []string
	created int
	stale   bool
}

func tcIdent(id, email string, groups ...string) map[string]interface{} {
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

func (rt *tcKratos) RoundTrip(req *http.Request) (*http.Response, error) {
	w := httptest.NewRecorder()
	w.Header().Set("Content-Type", "application/json")
	p := req.URL.Path
	seg := strings.Split(strings.TrimPrefix(p, "/admin/identities/"), "/")
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

func (rt *tcKratos) groups(id string) []string {
	arr, _ := rt.ids[id]["metadata_public"].(map[string]interface{})["groups"].([]interface{})
	var out []string
	for _, g := range arr {
		out = append(out, fmt.Sprint(g))
	}
	return out
}

type tcActor struct{ email, id, groups string }

var (
	tcAsIAM = tcActor{"iam@example.org", tcIAM, "IAM-ADMIN"}
	tcAsPA  = tcActor{"pa@example.org", tcPA, "PLATFORM-ADMIN"}
)

func tcPost(t *testing.T, s *Server, as tcActor, path string, form url.Values) *httptest.ResponseRecorder {
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

func tcServer(t *testing.T) (*Server, *tcKratos) {
	t.Helper()
	rt := &tcKratos{ids: map[string]map[string]interface{}{
		tcIAM: tcIdent(tcIAM, "iam@example.org", "IAM-ADMIN"),
		tcPA:  tcIdent(tcPA, "pa@example.org", "PLATFORM-ADMIN"),
		tcPA2: tcIdent(tcPA2, "pa2@example.org", "PLATFORM-ADMIN"),
		tcU:   tcIdent(tcU, "u@example.org", "VIEWER"),
	}}
	cli := kratos.NewClient("https://kratos.example.test")
	cli.HTTP = &http.Client{Transport: rt}
	s := newUsersAPITestServer(t, cli)
	s.Permissions.Groups = append(s.Permissions.Groups, permissions.Group{Name: "VIEWER"}, permissions.Group{Name: "DEVELOPER"})
	s.Permissions.StepUpRequired = append(s.Permissions.StepUpRequired,
		"mint_recovery_link", "reset_2fa", "grant_PLATFORM-ADMIN", "revoke_PLATFORM-ADMIN", "delete_identity")
	return s, rt
}

func wantStepUpRedirect(t *testing.T, what string, rec *httptest.ResponseRecorder) {
	t.Helper()
	if rec.Code != http.StatusSeeOther || !strings.HasPrefix(rec.Header().Get("Location"), "/step-up?nonce=") {
		t.Fatalf("%s: status %d Location %q, want 303 to /step-up?nonce=", what, rec.Code, rec.Header().Get("Location"))
	}
}

// The review's bypass: demote the platform admin, take the login, re-grant.
// Each producer step is refused, so the ceiling on the consumers holds.
func TestTierCeiling_groupChangeBypassClosed(t *testing.T) {
	s, rt := tcServer(t)
	if rec := tcPost(t, s, tcAsIAM, "/users/"+tcPA+"/groups?idp=tenant", url.Values{"groups": {""}}); rec.Code != http.StatusForbidden {
		t.Fatalf("IAM-ADMIN revoking PLATFORM-ADMIN: %d, want 403", rec.Code)
	}
	if g := rt.groups(tcPA); !containsString(g, "PLATFORM-ADMIN") {
		t.Fatalf("platform admin demoted: %v", g)
	}
	if rec := tcPost(t, s, tcAsIAM, "/users/"+tcPA+"/groups?idp=tenant", url.Values{"groups": {"PLATFORM-ADMIN,VIEWER"}}); rec.Code != http.StatusForbidden {
		t.Fatalf("IAM-ADMIN editing a platform admin's groups: %d, want 403", rec.Code)
	}
	if rec := tcPost(t, s, tcAsIAM, "/users/"+tcU+"/groups?idp=tenant", url.Values{"groups": {"VIEWER,PLATFORM-ADMIN"}}); rec.Code != http.StatusForbidden {
		t.Fatalf("IAM-ADMIN granting PLATFORM-ADMIN: %d, want 403", rec.Code)
	}
	if g := rt.groups(tcU); containsString(g, "PLATFORM-ADMIN") {
		t.Fatalf("PLATFORM-ADMIN granted by an IAM-ADMIN: %v", g)
	}
	for _, path := range []string{"/users/" + tcPA + "/recovery-link?idp=tenant", "/users/" + tcPA + "/reset-2fa?idp=tenant"} {
		if rec := tcPost(t, s, tcAsIAM, path, url.Values{}); rec.Code != http.StatusForbidden {
			t.Fatalf("%s by an IAM-ADMIN: %d, want 403", path, rec.Code)
		}
	}
	if len(rt.mints) != 0 {
		t.Fatalf("recovery links minted: %v", rt.mints)
	}
	// Within tier: an IAM-ADMIN manages a non-admin, a platform admin manages anyone.
	if rec := tcPost(t, s, tcAsIAM, "/users/"+tcU+"/groups?idp=tenant", url.Values{"groups": {"VIEWER,DEVELOPER"}}); rec.Code != http.StatusSeeOther {
		t.Fatalf("IAM-ADMIN changing a non-admin's groups: %d, want 303; %s", rec.Code, rec.Body.String())
	}
	if rec := tcPost(t, s, tcAsPA, "/users/"+tcPA2+"/groups?idp=tenant", url.Values{"groups": {"PLATFORM-ADMIN,VIEWER"}}); rec.Code != http.StatusSeeOther {
		t.Fatalf("platform admin changing a platform admin's groups: %d, want 303; %s", rec.Code, rec.Body.String())
	}
}

// Nobody adds groups to their own identity, admin or not; removing is fine.
func TestTierCeiling_selfGrantRefused(t *testing.T) {
	s, rt := tcServer(t)
	for _, groups := range []string{"IAM-ADMIN,PLATFORM-ADMIN", "IAM-ADMIN,VIEWER"} {
		if rec := tcPost(t, s, tcAsIAM, "/users/"+tcIAM+"/groups?idp=tenant", url.Values{"groups": {groups}}); rec.Code != http.StatusForbidden {
			t.Fatalf("IAM-ADMIN self-grant %q: %d, want 403", groups, rec.Code)
		}
	}
	if g := rt.groups(tcIAM); len(g) != 1 || g[0] != "IAM-ADMIN" {
		t.Fatalf("self-grant landed: %v", g)
	}
	if rec := tcPost(t, s, tcAsPA, "/users/"+tcPA+"/groups?idp=tenant", url.Values{"groups": {"PLATFORM-ADMIN,VIEWER"}}); rec.Code != http.StatusForbidden {
		t.Fatalf("platform admin self-grant: %d, want 403", rec.Code)
	}
}

// Self is the session's identity id. An identity that merely shares the
// actor's email is not the actor and gets no exemption.
func TestTierCeiling_selfIsIdentityIDNotEmail(t *testing.T) {
	s, rt := tcServer(t)
	rt.ids[tcPA2] = tcIdent(tcPA2, "iam@example.org", "PLATFORM-ADMIN")
	if rec := tcPost(t, s, tcAsIAM, "/users/"+tcPA2+"/recovery-link?idp=tenant", url.Values{}); rec.Code != http.StatusForbidden {
		t.Fatalf("mint on a platform admin sharing the actor's email: %d, want 403", rec.Code)
	}
	if rec := tcPost(t, s, tcAsIAM, "/users/"+tcPA2+"/groups?idp=tenant", url.Values{"groups": {""}}); rec.Code != http.StatusForbidden {
		t.Fatalf("demoting a platform admin sharing the actor's email: %d, want 403", rec.Code)
	}
	// A session with no identity id may not raise anyone's admin tier.
	anon := tcActor{"pa@example.org", "", "PLATFORM-ADMIN"}
	if rec := tcPost(t, s, anon, "/users/"+tcU+"/groups?idp=tenant", url.Values{"groups": {"VIEWER,IAM-ADMIN"}}); rec.Code != http.StatusForbidden {
		t.Fatalf("admin grant from a session with no identity id: %d, want 403", rec.Code)
	}
}

// Creating or inviting above one's own tier is refused; creating returns a
// recovery link, so it takes the mint step-up.
func TestTierCeiling_createAndInvite(t *testing.T) {
	s, rt := tcServer(t)
	if rec := tcPost(t, s, tcAsIAM, "/users/new?idp=tenant", url.Values{"email": {"evil@example.org"}, "groups": {"PLATFORM-ADMIN"}}); rec.Code != http.StatusForbidden {
		t.Fatalf("IAM-ADMIN creating a platform admin: %d, want 403", rec.Code)
	}
	if rt.created != 0 || len(rt.mints) != 0 {
		t.Fatalf("identity created (%d) or link minted (%v)", rt.created, rt.mints)
	}
	if rec := tcPost(t, s, tcAsIAM, "/invites?idp=tenant", url.Values{"preset_groups": {"PLATFORM-ADMIN"}, "ttl_hours": {"24"}}); rec.Code != http.StatusForbidden {
		t.Fatalf("IAM-ADMIN inviting a platform admin: %d, want 403", rec.Code)
	}
	if rec := tcPost(t, s, tcAsIAM, "/users/new?idp=tenant", url.Values{"email": {"ok@example.org"}, "groups": {"VIEWER"}}); rec.Code != http.StatusOK || len(rt.mints) != 1 {
		t.Fatalf("IAM-ADMIN creating a viewer with a fresh step-up: %d mints=%v", rec.Code, rt.mints)
	}

	rt.stale = true
	wantStepUpRedirect(t, "create without a fresh second factor",
		tcPost(t, s, tcAsIAM, "/users/new?idp=tenant", url.Values{"email": {"x@example.org"}, "groups": {"VIEWER"}}))
	if rt.created != 1 || len(rt.mints) != 1 {
		t.Fatalf("create ran without step-up: created=%d mints=%v", rt.created, rt.mints)
	}
}

// Minting a recovery link takes step-up when the policy lists it: the
// handler redirects into the step-up flow and never reaches Kratos.
func TestRecoveryLink_takesStepUp(t *testing.T) {
	s, rt := tcServer(t)
	rt.stale = true
	wantStepUpRedirect(t, "mint without a fresh second factor",
		tcPost(t, s, tcAsPA, "/users/"+tcU+"/recovery-link?idp=tenant", url.Values{}))
	if len(rt.mints) != 0 {
		t.Fatalf("recovery link minted before step-up: %v", rt.mints)
	}
	rt.stale = false
	if rec := tcPost(t, s, tcAsIAM, "/users/"+tcU+"/recovery-link?idp=tenant", url.Values{}); rec.Code != http.StatusOK || len(rt.mints) != 1 {
		t.Fatalf("mint within tier with a fresh step-up: %d mints=%v", rec.Code, rt.mints)
	}
}

// The ceiling also holds on the operator-key path: a per-person key acts at
// its owner's tier, so the review's demote/mint/re-grant bypass cannot run
// through POST /api/users/groups either. A service key stays uncapped.
func TestTierCeiling_operatorKeyActsAtOwnersTier(t *testing.T) {
	s, rt := tcServer(t)
	p := &onboardstore.Proposal{Sponsor: "sponsor-1", Env: "zagros", Handle: "casey", Email: "iam@example.org",
		Status: onboardstore.ProposalOnboarded, ManifestJSON: "{}", Tier: "developer"}
	if err := s.Onboard.CreateProposal(context.Background(), p); err != nil {
		t.Fatal(err)
	}
	s.Cfg.OnboardOperatorAPIKeys["k-casey"] = "casey"
	s.Cfg.OnboardOperatorAPIKeys["k-svc"] = "operator"
	api := func(key, body string) int {
		req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/api/users/groups", strings.NewReader(body))
		req.Header.Set("Authorization", "Bearer "+key)
		w := httptest.NewRecorder()
		s.Routes().ServeHTTP(w, req)
		return w.Code
	}
	if code := api("k-casey", `{"id":"`+tcPA+`","groups":[]}`); code != http.StatusForbidden {
		t.Fatalf("IAM-ADMIN's key demoting a platform admin: %d, want 403", code)
	}
	if code := api("k-casey", `{"id":"`+tcU+`","groups":["VIEWER","PLATFORM-ADMIN"]}`); code != http.StatusForbidden {
		t.Fatalf("IAM-ADMIN's key granting PLATFORM-ADMIN: %d, want 403", code)
	}
	if g := rt.groups(tcPA); !containsString(g, "PLATFORM-ADMIN") {
		t.Fatalf("platform admin demoted: %v", g)
	}
	// A service key is not held to the per-person platform-tier rule: it
	// edits a platform admin's identity. (Whether a service key may GRANT
	// the platform tier is GHSA-7788's rule, revision 8: it may not.)
	if code := api("k-svc", `{"id":"`+tcPA+`","groups":["PLATFORM-ADMIN","VIEWER"]}`); code != http.StatusOK {
		t.Fatalf("service key editing a platform admin's groups: %d, want 200", code)
	}
}
