package handlers

import (
	"bytes"
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/audit"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/config"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/hydra"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/kratos"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/permissions"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/pgtest"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/store"
)

type kratosAdminRoundTripper struct {
	cur map[string]interface{}
}

func (rt *kratosAdminRoundTripper) RoundTrip(req *http.Request) (*http.Response, error) {
	w := httptest.NewRecorder()
	w.Header().Set("Content-Type", "application/json")
	switch {
	case req.Method == http.MethodGet && req.URL.Path == "/admin/identities":
		if req.URL.Query().Get("page") == "0" {
			_ = json.NewEncoder(w).Encode([]interface{}{rt.cur})
		} else {
			_ = json.NewEncoder(w).Encode([]interface{}{})
		}
	case req.Method == http.MethodGet && strings.HasPrefix(req.URL.Path, "/admin/identities/"):
		_ = json.NewEncoder(w).Encode(rt.cur)
	case req.Method == http.MethodPatch && strings.HasPrefix(req.URL.Path, "/admin/identities/"):
		var ops []map[string]interface{}
		_ = json.NewDecoder(req.Body).Decode(&ops)
		for _, op := range ops {
			switch op["path"] {
			case "/traits/groups":
				rt.cur["traits"].(map[string]interface{})["groups"] = op["value"]
			case "/metadata_public/groups":
				// F-572 / ADR 1188 — groups now land on admin-managed metadata_public.
				mp, _ := rt.cur["metadata_public"].(map[string]interface{})
				if mp == nil {
					mp = map[string]interface{}{}
					rt.cur["metadata_public"] = mp
				}
				mp["groups"] = op["value"]
			case "/state":
				rt.cur["state"] = op["value"]
			}
		}
		_ = json.NewEncoder(w).Encode(rt.cur)
	default:
		w.WriteHeader(http.StatusNotFound)
	}
	return w.Result(), nil
}

// fakeKratos serves the minimal admin API the console's kratos.Client calls,
// backed by one mutable identity so a PATCH is observable.
func fakeKratos(t *testing.T, id, email, name string, groups []string, state string) (*kratos.Client, func() (grps []string, st string)) {
	t.Helper()
	gi := make([]interface{}, len(groups))
	for i, g := range groups {
		gi[i] = g
	}
	// A migrated identity: groups in admin-managed metadata_public (ADR
	// 1188) as well as traits. An identity with no metadata groups ranks
	// unknown (GHSA-vr6w), so tests of ordinary administration use migrated
	// ones; the unmigrated case has its own tests.
	cur := map[string]interface{}{
		"id": id, "state": state,
		"traits":          map[string]interface{}{"email": email, "name": name, "groups": gi},
		"metadata_public": map[string]interface{}{"groups": gi},
	}
	rt := &kratosAdminRoundTripper{cur: cur}
	read := func() ([]string, string) {
		// Read effective groups the way the accessor does — metadata_public first
		// (where PatchGroups writes per ADR 1188), then traits.
		var out []string
		groupsFrom := func(m map[string]interface{}) ([]interface{}, bool) {
			if m == nil {
				return nil, false
			}
			arr, ok := m["groups"].([]interface{})
			return arr, ok
		}
		mp, _ := cur["metadata_public"].(map[string]interface{})
		arr, ok := groupsFrom(mp)
		if !ok {
			arr, _ = groupsFrom(cur["traits"].(map[string]interface{}))
		}
		for _, g := range arr {
			out = append(out, g.(string))
		}
		return out, cur["state"].(string)
	}
	cli := kratos.NewClient("https://kratos.example.test")
	cli.HTTP = &http.Client{Transport: rt}
	return cli, read
}

// hydraStaticRoundTripper returns an empty consent-session list for any
// request. Used by unit tests that exercise handlers which call Hydra but
// do not care about the consent rows themselves.
type hydraStaticRoundTripper struct{}

func (hydraStaticRoundTripper) RoundTrip(req *http.Request) (*http.Response, error) {
	w := httptest.NewRecorder()
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(http.StatusOK)
	_, _ = w.Write([]byte("[]"))
	return w.Result(), nil
}

func newUsersAPITestServer(t *testing.T, kc *kratos.Client) *Server {
	t.Helper()
	ob, err := onboardstore.Open(pgtest.DSN(t))
	if err != nil {
		t.Fatalf("open store: %v", err)
	}
	t.Cleanup(func() { _ = ob.Close() })
	cfg := &config.Config{
		OnboardOperatorAPIKeys: map[string]string{"runner-key": "onboard-runner"},
		IdPs:                   map[string]config.IdP{"tenant": {Key: "tenant", Label: "Tenant"}},
		IdPOrder:               []string{"tenant"},
		DefaultIdP:             "tenant",
		// Mirror the production fail-closed default (config.go: STEP_UP_ENFORCED
		// defaults true) so the AAL2-gated paths are exercised as deployed on a
		// goatnet that enforces step-up. The STEP_UP_ENFORCED=false policy branch
		// is covered separately (stepup_policy_test.go, reset_2fa_test.go).
		StepUpEnforced: true,
		// Mirror the production AAL2_FRESH_WINDOW_SEC default (300s). Load()
		// always sets this; a zero value here would make every step-up expire
		// the instant it completed.
		AAL2FreshWindow: 5 * time.Minute,
		// The realm the actor's own posture resolves against — Load() defaults
		// it to the default IdP, mirrored here.
		StepUpSSOIdP: "tenant",
	}
	perms := &permissions.Matrix{
		SchemaVersion: 1,
		Groups: []permissions.Group{
			{Name: "ADMINS", Grants: []permissions.Grant{{Service: "id-admin-console", Access: "admin"}}},
			{Name: "IAM-ADMIN", Grants: []permissions.Grant{{Service: "id-admin-console", Access: "admin"}}},
			{Name: "PLATFORM-ADMIN", Grants: []permissions.Grant{{Service: "id-admin-console", Access: "admin"}}},
		},
	}
	hc := hydra.NewClient("https://hydra.example.test")
	hc.HTTP = &http.Client{Transport: hydraStaticRoundTripper{}}
	srv, err := NewServer(cfg, map[string]*kratos.Client{"tenant": kc}, map[string]*hydra.Client{"tenant": hc}, store.NewMemory(), audit.NewStderrEmitter(50), perms, ob)
	if err != nil {
		t.Fatalf("NewServer: %v", err)
	}
	return srv
}

// TestApiPatchUserGroups_operatorKeyGrantsByEmail proves the machine endpoint
// funnels through the same applyGroupChange core as the web UI: an operator-key
// caller grants IAM-ADMIN by email and the change lands in Kratos.
func TestApiPatchUserGroups_operatorKeyGrantsByEmail(t *testing.T) {
	kc, read := fakeKratos(t, "id-1", "w@example.org", "Casey", nil, "active")
	srv := newUsersAPITestServer(t, kc)
	// A service key (ADR 1071, uncapped). A per-person key needs a resolved
	// owner at or above the tier it grants (GHSA-7788, admin_tier_review_test.go).
	srv.Cfg.OnboardOperatorAPIKeys["svc-key"] = "operator"
	h := srv.Routes()

	body, _ := json.Marshal(map[string]interface{}{"email": "w@example.org", "groups": []string{"IAM-ADMIN"}})
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/api/users/groups", bytes.NewReader(body))
	req.Header.Set("Authorization", "Bearer svc-key")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)

	if rec.Code != http.StatusOK {
		t.Fatalf("status = %d, want 200; body=%s", rec.Code, rec.Body.String())
	}
	groups, _ := read()
	if len(groups) != 1 || groups[0] != "IAM-ADMIN" {
		t.Fatalf("kratos groups = %v, want [IAM-ADMIN]", groups)
	}
}

// A non-operator (no bearer key) is rejected before any Kratos call.
func TestApiPatchUserGroups_rejectsMissingKey(t *testing.T) {
	kc, read := fakeKratos(t, "id-1", "w@example.org", "Casey", nil, "active")
	h := newUsersAPITestServer(t, kc).Routes()

	body, _ := json.Marshal(map[string]interface{}{"email": "w@example.org", "groups": []string{"IAM-ADMIN"}})
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/api/users/groups", bytes.NewReader(body))
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)

	if rec.Code != http.StatusUnauthorized {
		t.Fatalf("status = %d, want 401", rec.Code)
	}
	if groups, _ := read(); len(groups) != 0 {
		t.Fatalf("unauthorized call must not touch Kratos; groups=%v", groups)
	}
}

// Omitting "groups" is a 400 (distinct from an explicit [] clear).
func TestApiPatchUserGroups_requiresGroupsField(t *testing.T) {
	kc, _ := fakeKratos(t, "id-1", "w@example.org", "Casey", nil, "active")
	h := newUsersAPITestServer(t, kc).Routes()

	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/api/users/groups",
		strings.NewReader(`{"email":"w@example.org"}`))
	req.Header.Set("Authorization", "Bearer runner-key")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)

	if rec.Code != http.StatusBadRequest {
		t.Fatalf("status = %d, want 400; body=%s", rec.Code, rec.Body.String())
	}
}

// TestApiPatchUserState_operatorKeyReactivates proves the state sibling flips an
// inactive identity back to active — the reactivate-before-grant path.
func TestApiPatchUserState_operatorKeyReactivates(t *testing.T) {
	// A migrated identity: an operator key may not reactivate an unmigrated
	// one, which ranks unknown (GHSA-7788, ADR 1071 as amended).
	kc, read := fakeKratosMigrated7788(t, "id-1", "j@example.org", "Riley", nil, "inactive")
	h := newUsersAPITestServer(t, kc).Routes()

	body, _ := json.Marshal(map[string]interface{}{"email": "j@example.org", "state": "active"})
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/api/users/state", bytes.NewReader(body))
	req.Header.Set("Authorization", "Bearer runner-key")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)

	if rec.Code != http.StatusOK {
		t.Fatalf("status = %d, want 200; body=%s", rec.Code, rec.Body.String())
	}
	if _, st := read(); st != "active" {
		t.Fatalf("state = %q, want active", st)
	}
}

// An operator key may not raise its own holder.
//
// This route takes no step-up and no second person — the key IS the
// authorization — so without this guard the holder of any key could add
// PLATFORM-ADMIN to their own login, and the audit would carry their handle on
// both halves of the escalation. The principal here is named by the login email
// the identity carries, which is the spelling a person-held key resolves to
// when no onboarding handle matches.
func TestApiPatchUserGroups_refusesSelfEscalation(t *testing.T) {
	kc, read := fakeKratos(t, "id-self", "self@example.org", "Self", []string{"DEVELOPER"}, "active")
	s := newUsersAPITestServer(t, kc)
	s.Cfg.OnboardOperatorAPIKeys = map[string]string{"self-key": "self@example.org"}
	h := s.Routes()

	body, _ := json.Marshal(map[string]interface{}{"email": "self@example.org", "groups": []string{"PLATFORM-ADMIN"}})
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/api/users/groups", bytes.NewReader(body))
	req.Header.Set("Authorization", "Bearer self-key")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)

	if rec.Code != http.StatusForbidden {
		t.Fatalf("status = %d, want 403; body=%s", rec.Code, rec.Body.String())
	}
	groups, _ := read()
	if len(groups) != 1 || groups[0] != "DEVELOPER" {
		t.Fatalf("kratos groups = %v, want them untouched [DEVELOPER]", groups)
	}
}

// The same key changing SOMEBODY ELSE still works: the guard refuses self, not
// the route. A key whose owner does not resolve may still make a non-admin
// change; admin-bearing changes need a resolved owner (GHSA-7788).
func TestApiPatchUserGroups_selfGuardDoesNotBlockOthers(t *testing.T) {
	// A migrated identity: an unmigrated one ranks unknown, and a per-person
	// key may not act on it (GHSA-7788).
	kc, read := fakeKratosMigrated7788(t, "id-other", "other@example.org", "Other", nil, "active")
	s := newUsersAPITestServer(t, kc)
	s.Cfg.OnboardOperatorAPIKeys = map[string]string{"self-key": "self@example.org"}
	s.Permissions.Groups = append(s.Permissions.Groups, permissions.Group{Name: "DEVELOPER"})
	h := s.Routes()

	body, _ := json.Marshal(map[string]interface{}{"email": "other@example.org", "groups": []string{"DEVELOPER"}})
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/api/users/groups", bytes.NewReader(body))
	req.Header.Set("Authorization", "Bearer self-key")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)

	if rec.Code != http.StatusOK {
		t.Fatalf("status = %d, want 200; body=%s", rec.Code, rec.Body.String())
	}
	if groups, _ := read(); len(groups) != 1 || groups[0] != "DEVELOPER" {
		t.Fatalf("kratos groups = %v, want [DEVELOPER]", groups)
	}
}
