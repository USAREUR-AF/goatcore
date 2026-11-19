package handlers

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/audit"
)

// Model a PATCH that echoes active while a database guard retains inactive.
type stateGuardTransport struct{ base http.RoundTripper }

func (rt stateGuardTransport) RoundTrip(r *http.Request) (*http.Response, error) {
	if r.Method != http.MethodPatch {
		return rt.base.RoundTrip(r)
	}
	w := httptest.NewRecorder()
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(map[string]any{
		"id": "fixture-identity", "state": "active",
		"traits": map[string]any{"email": "returning@example.test"},
	})
	return w.Result(), nil
}

func TestApiPatchUserStateGuardRefusalIsNotAuditedAsSuccess(t *testing.T) {
	cli, read := fakeKratosMigrated7788(t, "fixture-identity", "returning@example.test", "Fixture", nil, "inactive")
	cli.HTTP.Transport = stateGuardTransport{base: cli.HTTP.Transport}
	s := newUsersAPITestServer(t, cli)
	s.Audit = audit.NewStderrEmitter(8)
	r := httptest.NewRequestWithContext(t.Context(), http.MethodPost, "/api/users/state",
		strings.NewReader(`{"email":"returning@example.test","state":"active"}`))
	r.Header.Set("Authorization", "Bearer runner-key")
	w := httptest.NewRecorder()
	s.Routes().ServeHTTP(w, r)
	if w.Code != http.StatusBadGateway {
		t.Fatalf("state change returned %d, want verification error: %s", w.Code, w.Body)
	}
	if _, state := read(); state != "inactive" {
		t.Fatalf("guarded identity changed to %q", state)
	}
	events := s.Audit.Recent(8)
	if len(events) != 1 || events[0].Action != "patch_state" || events[0].Result != "error" {
		t.Fatalf("guard refusal was not audited as error: %+v", events)
	}
}

func TestReinstatementCheckRequiresOperatorAndReasonWithoutMutating(t *testing.T) {
	for _, tc := range []struct {
		name, key, reason string
		want              int
	}{
		{"missing operator", "", "approved return", http.StatusUnauthorized},
		{"unknown key", "unknown", "approved return", http.StatusUnauthorized},
		{"missing reason", "runner-key", "", http.StatusBadRequest},
		{"multiline reason", "runner-key", "one\ntwo", http.StatusBadRequest},
		{"explicit operator request", "runner-key", "approved return", http.StatusOK},
	} {
		t.Run(tc.name, func(t *testing.T) {
			cli, read := fakeKratosMigrated7788(t, "fixture-identity", "returning@example.test", "Fixture", nil, "inactive")
			s := newUsersAPITestServer(t, cli)
			s.Cfg.NotifyGoatnet = "fixture"
			raw, _ := json.Marshal(map[string]string{"email": "returning@example.test", "reason": tc.reason, "actor": "forged", "env": "fixture"})
			r := httptest.NewRequestWithContext(t.Context(), http.MethodPost, "/api/users/reinstatement-check", strings.NewReader(string(raw)))
			if tc.key != "" {
				r.Header.Set("Authorization", "Bearer "+tc.key)
			}
			w := httptest.NewRecorder()
			s.Routes().ServeHTTP(w, r)
			if w.Code != tc.want {
				t.Fatalf("status %d, want %d: %s", w.Code, tc.want, w.Body)
			}
			if _, state := read(); state != "inactive" {
				t.Fatalf("authorization changed state: %s", state)
			}
			if tc.want == http.StatusOK {
				var receipt map[string]string
				if err := json.Unmarshal(w.Body.Bytes(), &receipt); err != nil {
					t.Fatal(err)
				}
				if receipt["id"] != "fixture-identity" || receipt["actor"] != "onboard-runner" {
					t.Fatalf("untrusted receipt: %v", receipt)
				}
				events := s.Audit.Recent(8)
				if len(events) != 1 || events[0].Action != "reinstate_identity_requested" || events[0].Result != "requested" {
					t.Fatalf("request was not distinguished from completed recovery: %+v", events)
				}
			}
		})
	}
}

func TestReinstatementCheckRefusesForeignEnvironment(t *testing.T) {
	cli, read := fakeKratosMigrated7788(t, "fixture-identity", "returning@example.test", "Fixture", nil, "inactive")
	s := newUsersAPITestServer(t, cli)
	s.Cfg.NotifyGoatnet = "fixture"
	r := httptest.NewRequestWithContext(t.Context(), http.MethodPost, "/api/users/reinstatement-check", strings.NewReader(`{"env":"other","email":"returning@example.test","reason":"approved return"}`))
	r.Header.Set("Authorization", "Bearer runner-key")
	w := httptest.NewRecorder()
	s.Routes().ServeHTTP(w, r)
	if w.Code != http.StatusConflict {
		t.Fatalf("foreign console accepted: %d %s", w.Code, w.Body)
	}
	if _, state := read(); state != "inactive" {
		t.Fatalf("foreign request mutated identity: %s", state)
	}
}
