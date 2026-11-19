package handlers

// Regression tests for the revoke-failure leg of applyGroupChange: a group
// change whose requested session revoke fails must NOT be audited as "ok" or
// returned as a 200 — the live sessions still carry the old groups (the exact
// condition revokeSessions exists to prevent). Red-team finding, HIGH.

import (
	"bytes"
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/kratos"
)

type kratosRevokeRoundTripper struct {
	cur         map[string]interface{}
	revokeFails bool
	revokeCalls int
}

func (rt *kratosRevokeRoundTripper) RoundTrip(req *http.Request) (*http.Response, error) {
	w := httptest.NewRecorder()
	w.Header().Set("Content-Type", "application/json")
	switch {
	case req.Method == http.MethodDelete && strings.HasSuffix(req.URL.Path, "/sessions"):
		rt.revokeCalls++
		if rt.revokeFails {
			w.WriteHeader(http.StatusInternalServerError)
			_, _ = w.Write([]byte(`{"error":{"message":"kratos session store unavailable"}}`))
			break
		}
		w.WriteHeader(http.StatusNoContent)
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
			}
		}
		_ = json.NewEncoder(w).Encode(rt.cur)
	default:
		w.WriteHeader(http.StatusNotFound)
	}
	return w.Result(), nil
}

// fakeKratosWithRevoke is fakeKratos plus an explicit DELETE
// /admin/identities/{id}/sessions route whose outcome the test controls.
func fakeKratosWithRevoke(t *testing.T, id, email, name string, groups []string, revokeFails bool) (*kratos.Client, func() []string, func() int) {
	t.Helper()
	gi := make([]interface{}, len(groups))
	for i, g := range groups {
		gi[i] = g
	}
	cur := map[string]interface{}{
		"id": id, "state": "active",
		"traits": map[string]interface{}{"email": email, "name": name, "groups": gi},
	}
	rt := &kratosRevokeRoundTripper{cur: cur, revokeFails: revokeFails}
	readGroups := func() []string {
		// Effective groups: metadata_public first (where PatchGroups writes per
		// ADR 1188), then traits.
		var out []string
		arr, ok := func() ([]interface{}, bool) {
			if mp, ok := cur["metadata_public"].(map[string]interface{}); ok {
				if a, ok := mp["groups"].([]interface{}); ok {
					return a, true
				}
			}
			return nil, false
		}()
		if !ok {
			arr, _ = cur["traits"].(map[string]interface{})["groups"].([]interface{})
		}
		for _, g := range arr {
			out = append(out, g.(string))
		}
		return out
	}
	readRevokeCalls := func() int { return rt.revokeCalls }
	cli := kratos.NewClient("https://kratos.example.test")
	cli.HTTP = &http.Client{Transport: rt}
	return cli, readGroups, readRevokeCalls
}

// TestApplyGroupChange_revokeFailureIsNotOk drives the operator-key surface
// (POST /api/users/groups, funneled through applyGroupChange) with
// revoke_sessions:true against a Kratos whose session revoke fails, and
// asserts the failure is surfaced, not swallowed:
//   - the handler does NOT return 200 (it 502s like every other Kratos
//     failure in this path),
//   - the patch_groups audit record is NOT result:"ok" and records
//     revoked_sessions:false,
//   - the group change itself persisted (partial-failure semantics: the
//     change is not rolled back, the revoke gap is what gets flagged).
func TestApplyGroupChange_revokeFailureIsNotOk(t *testing.T) {
	kc, readGroups, revokeCalls := fakeKratosWithRevoke(t, "id-1", "w@example.org", "Casey", nil, true)
	srv := newUsersAPITestServer(t, kc)
	// A service key: an admin-bearing grant from a per-person key needs a
	// resolved owner (GHSA-7788), which this fixture does not model.
	srv.Cfg.OnboardOperatorAPIKeys["svc-key"] = "operator"
	h := srv.Routes()

	body, _ := json.Marshal(map[string]interface{}{
		"email": "w@example.org", "groups": []string{"IAM-ADMIN"}, "revoke_sessions": true,
	})
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/api/users/groups", bytes.NewReader(body))
	req.Header.Set("Authorization", "Bearer svc-key")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)

	if rec.Code == http.StatusOK {
		t.Fatalf("revoke failure returned 200 — a failed session revoke must surface; body=%s", rec.Body.String())
	}
	if rec.Code != http.StatusBadGateway {
		t.Fatalf("status = %d, want 502; body=%s", rec.Code, rec.Body.String())
	}
	if n := revokeCalls(); n != 1 {
		t.Fatalf("revoke calls = %d, want 1", n)
	}

	var patchEvents int
	for _, e := range srv.Audit.Recent(50) {
		if e.Action != "patch_groups" {
			continue
		}
		patchEvents++
		if e.Result == "ok" {
			t.Fatalf("patch_groups audited result=%q after a failed revoke — must not be ok; event=%+v", e.Result, e)
		}
		if e.Result != "error" {
			t.Fatalf("patch_groups audit result = %q, want %q", e.Result, "error")
		}
		if rv, ok := e.After["revoked_sessions"]; !ok || rv != false {
			t.Fatalf("patch_groups audit after.revoked_sessions = %v (present=%v), want false", rv, ok)
		}
		if e.ErrorMessage == "" {
			t.Fatalf("patch_groups audit on revoke failure carries no error message")
		}
	}
	if patchEvents == 0 {
		t.Fatalf("no patch_groups audit event emitted")
	}

	// The group change itself persisted — only the revoke leg failed.
	if g := readGroups(); len(g) != 1 || g[0] != "IAM-ADMIN" {
		t.Fatalf("kratos groups = %v, want [IAM-ADMIN] (change persists; revoke gap is flagged)", g)
	}
}

// TestApplyGroupChange_revokeSuccess_AuditOk is the control: when the revoke
// succeeds, the change is a 200 and the single patch_groups audit record is
// result:"ok" with revoked_sessions:true.
func TestApplyGroupChange_revokeSuccess_AuditOk(t *testing.T) {
	kc, _, revokeCalls := fakeKratosWithRevoke(t, "id-1", "w@example.org", "Casey", nil, false)
	srv := newUsersAPITestServer(t, kc)
	// A service key: an admin-bearing grant from a per-person key needs a
	// resolved owner (GHSA-7788), which this fixture does not model.
	srv.Cfg.OnboardOperatorAPIKeys["svc-key"] = "operator"
	h := srv.Routes()

	body, _ := json.Marshal(map[string]interface{}{
		"email": "w@example.org", "groups": []string{"IAM-ADMIN"}, "revoke_sessions": true,
	})
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/api/users/groups", bytes.NewReader(body))
	req.Header.Set("Authorization", "Bearer svc-key")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)

	if rec.Code != http.StatusOK {
		t.Fatalf("status = %d, want 200; body=%s", rec.Code, rec.Body.String())
	}
	if n := revokeCalls(); n != 1 {
		t.Fatalf("revoke calls = %d, want 1", n)
	}
	for _, e := range srv.Audit.Recent(50) {
		if e.Action != "patch_groups" {
			continue
		}
		if e.Result != "ok" {
			t.Fatalf("patch_groups audit result = %q, want ok; event=%+v", e.Result, e)
		}
		if rv, ok := e.After["revoked_sessions"]; !ok || rv != true {
			t.Fatalf("patch_groups audit after.revoked_sessions = %v, want true", rv)
		}
	}
}
