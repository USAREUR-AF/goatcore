package handlers

// The machine realm resolves a caller to a per-handle operator principal
// and stashes it under a context key the browser Actor never fills.
// Every audit event a machine-realm handler emits must name that principal;
// reading the zero browser Actor instead records the action as nobody. #234
// fixed this for the decision path (approve / reject / revoke); this test
// covers its siblings on the same realm — request, collect, and reconcile.

import (
	"context"
	"net/http"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/opkeys"
)

func TestOperatorKeyAPI_auditCarriesActingPrincipal(t *testing.T) {
	// newOnboardTestServer maps the bearer "runner-key" to this handle.
	const principal = "onboard-runner"

	actorOf := func(t *testing.T, s *Server, action string) string {
		t.Helper()
		for _, e := range s.Audit.Recent(50) {
			if e.Action == action {
				return e.Actor.Email
			}
		}
		t.Fatalf("no %q audit event emitted", action)
		return ""
	}

	t.Run("request", func(t *testing.T) {
		s := newOnboardTestServer(t)
		h := s.Routes()
		// The operator door files only for a person on the roster.
		seedRosterPerson(t, s, "someone-else", "someone-else@example.org")
		if w := machineReq(t, h, "runner-key", "POST", "/api/onboard/operator-keys",
			`{"handle":"someone-else","env":"zagros"}`); w.Code != http.StatusOK {
			t.Fatalf("request = %d; %s", w.Code, w.Body.String())
		}
		if got := actorOf(t, s, "operator_key_request"); got != principal {
			t.Fatalf("operator_key_request actor = %q, want %q", got, principal)
		}
	})

	t.Run("collect", func(t *testing.T) {
		s := newOnboardTestServer(t)
		h := s.Routes()
		// H2: collection is bound to the CALLING principal, so the key must
		// belong to the caller (runner-key resolves to "onboard-runner").
		row := seedOperatorKeyRequest(t, s, principal, "")
		// A different decider approves it, so it is Active with a live secret to
		// collect.
		if _, err := s.Onboard.ApproveOperatorKey(context.Background(), row.ID, "admin", []string{"admin"}); err != nil {
			t.Fatalf("approve: %v", err)
		}
		if w := machineReq(t, h, "runner-key", "GET",
			"/api/onboard/operator-keys/"+row.ID+"/collect", ""); w.Code != http.StatusOK {
			t.Fatalf("collect = %d; %s", w.Code, w.Body.String())
		}
		if got := actorOf(t, s, "operator_key_collect"); got != principal {
			t.Fatalf("operator_key_collect actor = %q, want %q", got, principal)
		}
	})

	t.Run("reconcile apply", func(t *testing.T) {
		s := newOnboardTestServer(t)
		dir := t.TempDir()
		s.Cfg.OperatorKeyDir = dir
		h := s.Routes()
		// A file for a handle this console ISSUED and then revoked is the stale
		// case --apply removes — the only reconcile outcome that emits an audit
		// event. (A never-issued row's file is never touched, GHSA-2w7v.)
		if err := (opkeys.Dir{Path: dir}).Write("ghost", "op_ghost"); err != nil {
			t.Fatalf("seed key file: %v", err)
		}
		ghost := seedOperatorKeyRequest(t, s, "ghost", "ghost@example.org")
		if _, err := s.Onboard.ApproveOperatorKey(context.Background(), ghost.ID, "someone-else@x", []string{"someone-else@x"}); err != nil {
			t.Fatalf("approve ghost: %v", err)
		}
		if _, err := s.Onboard.RevokeOperatorKey(context.Background(), ghost.ID, "admin@x", ""); err != nil {
			t.Fatalf("revoke ghost: %v", err)
		}
		if w := machineReq(t, h, "runner-key", "POST",
			"/api/onboard/operator-keys/reconcile?apply=true&env=zagros", ""); w.Code != http.StatusOK {
			t.Fatalf("reconcile = %d; %s", w.Code, w.Body.String())
		}
		if got := actorOf(t, s, "operator_key_reconcile"); got != principal {
			t.Fatalf("operator_key_reconcile actor = %q, want %q", got, principal)
		}
	})
}
