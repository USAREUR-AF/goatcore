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

// Machine realm, GHSA-mxf4-8rv7-774w round 2: a per-person operator key is
// tied to its holder's identity on every request; deleting or demoting an
// identity takes its keys; a key is filed only for a person on the roster; and
// the console's own changes reach the live-identity cache at once.

func adminForm(t *testing.T, s *Server, path string, form url.Values) *httptest.ResponseRecorder {
	t.Helper()
	form.Set("csrf", s.csrfNonce)
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, path, strings.NewReader(form.Encode()))
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	req.Header.Set("X-Forwarded-Email", "other-admin@example.test")
	req.Header.Set("X-Forwarded-User", testActorID)
	req.Header.Set("X-Forwarded-Groups", "PLATFORM-ADMIN")
	rec := httptest.NewRecorder()
	s.Routes().ServeHTTP(rec, req)
	return rec
}

func keyGone(t *testing.T, f *opKeyFixture, k *onboardstore.OperatorKey) bool {
	t.Helper()
	row, err := f.s.Onboard.OperatorKey(context.Background(), k.ID)
	if err != nil {
		t.Fatal(err)
	}
	return row.Status == onboardstore.OperatorKeyRevoked && !secretOnDisk(t, f.dir, k.Handle, k.Secret)
}

// F1: deleting the identity from the console takes its operator keys.
func TestDeleteIdentity_revokesOperatorKeys(t *testing.T) {
	f := newOpKeyFixture(t, onboardstore.ProposalOnboarded)
	f.s.Cfg.StepUpEnforced = false
	rec := adminForm(t, f.s, "/users/id-alice/delete?idp=tenant", url.Values{"confirm_email": {"alice@example.test"}})
	if rec.Code != http.StatusSeeOther {
		t.Fatalf("delete: %d %s", rec.Code, rec.Body.String())
	}
	if !keyGone(t, f, f.mine) || !keyGone(t, f, f.pending) {
		t.Fatal("alice's operator keys survived the delete of her identity")
	}
	if keyGone(t, f, f.theirs) {
		t.Fatal("deleting alice revoked bob's key")
	}
}

// F2: a group change that leaves no admin group takes the keys, on the web
// path and the API path; one that keeps an admin group does not.
func TestGroupChange_demotionRevokesOperatorKeys(t *testing.T) {
	t.Run("web keeps an admin group", func(t *testing.T) {
		f := newOpKeyFixture(t, onboardstore.ProposalOnboarded)
		f.s.Cfg.StepUpEnforced = false
		rec := adminForm(t, f.s, "/users/id-alice/groups?idp=tenant", url.Values{"groups": {"IAM-ADMIN,data.view.all"}})
		if rec.Code != http.StatusSeeOther {
			t.Fatalf("group change: %d", rec.Code)
		}
		if keyGone(t, f, f.mine) {
			t.Fatal("a group change that keeps IAM-ADMIN revoked the key")
		}
	})
	t.Run("web", func(t *testing.T) {
		f := newOpKeyFixture(t, onboardstore.ProposalOnboarded)
		f.s.Cfg.StepUpEnforced = false
		rec := adminForm(t, f.s, "/users/id-alice/groups?idp=tenant", url.Values{"groups": {""}})
		if groups, _ := f.read(); len(groups) != 0 {
			t.Fatalf("demotion did not land: %d %v", rec.Code, groups)
		}
		if !keyGone(t, f, f.mine) || !keyGone(t, f, f.pending) {
			t.Fatal("demoted out of every admin group, alice keeps a live operator key")
		}
		if keyGone(t, f, f.theirs) {
			t.Fatal("demoting alice revoked bob's key")
		}
	})
	t.Run("api", func(t *testing.T) {
		f := newOpKeyFixture(t, onboardstore.ProposalOnboarded)
		body, _ := json.Marshal(map[string]interface{}{"id": "id-alice", "groups": []string{"data.view.all"}})
		req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/api/users/groups", bytes.NewReader(body))
		req.Header.Set("Authorization", "Bearer site-runner-key")
		rec := httptest.NewRecorder()
		f.s.Routes().ServeHTTP(rec, req)
		if rec.Code != http.StatusOK {
			t.Fatalf("api demotion: %d %s", rec.Code, rec.Body.String())
		}
		if !keyGone(t, f, f.mine) {
			t.Fatal("api demotion left alice's operator key live")
		}
	})
}

// The main control: a per-person operator key works only while its holder's
// identity is active and holds an admin group.
func TestOperatorOwnerGate(t *testing.T) {
	setup := func(t *testing.T, groups []string, state string) (*Server, func() ([]string, string)) {
		t.Helper()
		const id = "0b1f0000-0000-4000-8000-0000000000f1"
		forgetIdentity(id) // the caches are process-wide; each subtest starts clean
		kc, read := fakeKratosMeta(t, id, "alice@example.test", "Alice", groups, state)
		s := newUsersAPITestServer(t, kc)
		s.Cfg.LiveIdentityCheck = true
		s.Cfg.NotifyGoatnet = "zagros"
		s.Cfg.OnboardOperatorAPIKeys["alice-key"] = "op-alice"
		s.Cfg.OnboardOperatorAPIKeys["ghost-key"] = "ghost"
		s.Cfg.OnboardOperatorAPIKeys["svc-key"] = "e2e-operator"
		seedRosterPerson(t, s, "op-alice", "alice@example.test")
		return s, read
	}
	get := func(s *Server, key string) int {
		return machineReq(t, s.Routes(), key, http.MethodGet, "/api/onboard/proposals", "").Code
	}
	t.Run("active admin passes", func(t *testing.T) {
		s, _ := setup(t, []string{"IAM-ADMIN"}, "active")
		if c := get(s, "alice-key"); c != http.StatusOK {
			t.Fatalf("active admin's key: %d", c)
		}
	})
	t.Run("inactive holder refused and audited", func(t *testing.T) {
		s, _ := setup(t, []string{"IAM-ADMIN"}, "inactive")
		if c := get(s, "alice-key"); c != http.StatusForbidden {
			t.Fatalf("inactive holder's key: %d, want 403", c)
		}
		if after := auditAfter(t, s, "operator_key_owner_refused"); after["handle"] != "op-alice" {
			t.Fatalf("refusal audit = %v", after)
		}
	})
	t.Run("demoted holder refused", func(t *testing.T) {
		s, _ := setup(t, []string{"data.view.all"}, "active")
		if c := get(s, "alice-key"); c != http.StatusForbidden {
			t.Fatalf("demoted holder's key: %d, want 403", c)
		}
	})
	t.Run("unresolvable handle refused", func(t *testing.T) {
		s, _ := setup(t, []string{"IAM-ADMIN"}, "active")
		if c := get(s, "ghost-key"); c != http.StatusForbidden {
			t.Fatalf("key tied to nobody: %d, want 403", c)
		}
	})
	t.Run("service keys exempt", func(t *testing.T) {
		s, _ := setup(t, nil, "inactive")
		if c := get(s, "runner-key"); c != http.StatusOK {
			t.Fatalf("onboard-runner key: %d", c)
		}
		if c := get(s, "svc-key"); c != http.StatusForbidden {
			t.Fatalf("unlisted e2e-operator key: %d, want 403", c)
		}
		s.Cfg.ServiceOperatorHandles = []string{"e2e-operator"}
		if c := get(s, "svc-key"); c != http.StatusOK {
			t.Fatalf("listed service key: %d", c)
		}
	})
	t.Run("identity service down refuses", func(t *testing.T) {
		forgetIdentity("0b1f0000-0000-4000-8000-0000000000f1")
		kc, _ := fakeKratosRevokeSessionsTargetError(t, http.StatusInternalServerError)
		s := newUsersAPITestServer(t, kc)
		s.Cfg.LiveIdentityCheck = true
		s.Cfg.OnboardOperatorAPIKeys["alice-key"] = "op-alice"
		seedRosterPerson(t, s, "op-alice", "alice@example.test")
		if c := get(s, "alice-key"); c != http.StatusServiceUnavailable && c != http.StatusForbidden {
			t.Fatalf("identity service down: %d, want a refusal", c)
		}
	})
}

// F3 (the reviewer's exploit): a key filed under a handle that names nobody
// cannot be used to reactivate its holder, because the machine gate cannot tie
// it to an identity; and the operator door no longer files such a key.
func TestNonRosterHandleKey(t *testing.T) {
	ctx := context.Background()
	kc, read := fakeKratosMeta(t, "id-alice", "alice@example.test", "Alice", []string{"IAM-ADMIN"}, "active")
	s := newUsersAPITestServer(t, kc)
	s.Cfg.LiveIdentityCheck = true
	s.Cfg.NotifyGoatnet = "zagros"
	s.Cfg.OperatorKeyDir = t.TempDir()
	s.Cfg.StepUpEnforced = false
	seedRosterPerson(t, s, "op-alice", "alice@example.test")

	// The door refuses to file it.
	if w := machineReq(t, s.Routes(), "runner-key", http.MethodPost, "/api/onboard/operator-keys",
		`{"handle":"alice-ci","env":"zagros"}`); w.Code != http.StatusBadRequest || !strings.Contains(w.Body.String(), "not_on_roster") {
		t.Fatalf("non-roster handle request: %d %s", w.Code, w.Body.String())
	}
	if w := machineReq(t, s.Routes(), "runner-key", http.MethodPost, "/api/onboard/operator-keys",
		`{"handle":"op-alice","email":"mallory@example.test","env":"zagros"}`); w.Code != http.StatusBadRequest {
		t.Fatalf("roster handle with a foreign email: %d %s", w.Code, w.Body.String())
	}
	w := machineReq(t, s.Routes(), "runner-key", http.MethodPost, "/api/onboard/operator-keys",
		`{"handle":"op-alice","env":"zagros"}`)
	var row onboardstore.OperatorKey
	_ = json.Unmarshal(w.Body.Bytes(), &row)
	if w.Code != http.StatusOK || row.Email != "alice@example.test" {
		t.Fatalf("roster handle request: %d %+v", w.Code, row)
	}

	// A row that got in anyway (filed before this fix) is refused at use.
	pre, err := s.Onboard.RequestOperatorKey(ctx, &onboardstore.OperatorKey{Handle: "alice-ci", Env: "zagros", Reason: "ci"})
	if err != nil {
		t.Fatal(err)
	}
	pre, err = s.Onboard.ApproveOperatorKey(ctx, pre.ID, "bob@example.test", []string{"bob@example.test", "op-bob"}) //nolint:staticcheck // behavior kept as reviewed for disclosure; lint cleanup goatcore#1028
	if err != nil {
		t.Fatal(err)
	}
	s.Cfg.OnboardOperatorAPIKeys["alice-ci-secret"] = "alice-ci"
	s.Cfg.LiveIdentityCheck = false // postState carries no subject; the deactivation is not under test
	if rec := postState(t, s, "id-alice", "other-admin@example.test", "inactive"); rec.Code != http.StatusSeeOther {
		t.Fatalf("deactivation: %d", rec.Code)
	}
	s.Cfg.LiveIdentityCheck = true
	body, _ := json.Marshal(map[string]string{"email": "alice@example.test", "state": "active"})
	req := httptest.NewRequestWithContext(ctx, http.MethodPost, "/api/users/state", bytes.NewReader(body))
	req.Header.Set("Authorization", "Bearer alice-ci-secret")
	rec := httptest.NewRecorder()
	s.Routes().ServeHTTP(rec, req)
	if _, st := read(); rec.Code == http.StatusOK || st == "active" {
		t.Fatalf("non-roster key reactivated its holder: %d, state %q", rec.Code, st)
	}
}

// F4: the console's own deactivation reaches an open session at once, not
// after liveActorTTL. (The fake identity service answers every id with the
// same identity, so the deactivating admin passes the live gate too.)
func TestLiveActorCacheDroppedByConsoleDeactivation(t *testing.T) {
	const sub = "0b1f0000-0000-4000-8000-0000000000e2"
	kc, read := fakeKratosMeta(t, sub, "a@example.org", "A", []string{"IAM-ADMIN"}, "active")
	s := newUsersAPITestServer(t, kc)
	s.Cfg.LiveIdentityCheck = true
	s.Cfg.StepUpEnforced = false
	if code := liveGateGet(t, s, sub, "IAM-ADMIN"); code == http.StatusForbidden {
		t.Fatalf("active admin refused: %d", code)
	}
	form := url.Values{"state": {"inactive"}, "csrf": {s.csrfNonce}}
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/users/"+sub+"/state?idp=tenant", strings.NewReader(form.Encode()))
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	req.Header.Set("X-Forwarded-Email", "other@example.org")
	req.Header.Set("X-Forwarded-User", "0b1f0000-0000-4000-8000-0000000000e3")
	req.Header.Set("X-Forwarded-Groups", "IAM-ADMIN")
	rec := httptest.NewRecorder()
	s.Routes().ServeHTTP(rec, req)
	if _, st := read(); st != "inactive" {
		t.Fatalf("deactivation did not land: %d %s", rec.Code, rec.Body.String())
	}
	if code := liveGateGet(t, s, sub, "IAM-ADMIN"); code != http.StatusForbidden {
		t.Fatalf("within the cache TTL after the console's deactivation: %d, want 403", code)
	}
}
