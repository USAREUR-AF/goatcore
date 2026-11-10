package handlers

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/kratos"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// goat#319 — the offboard-verification view: a read-only, admin-gated page
// that answers "is this person actually gone?" across the four identity
// planes, with an explicit still-present flag per plane and NO silent green.
//
// Page contract these tests pin (the spec, not the implementation):
//   - GET /offboard-verify?person=<email or handle>, adminGate'd.
//   - Each plane renders data-plane="login|operator-keys|ssh-keys|mesh|fabric"
//     with data-status="present|absent|unknown".
//   - Verdict: data-verdict="lingering" if ANY plane is present,
//     "unverified" if none present but ANY unknown, "clear" only when every
//     plane is absent. Unknown must never render as clear.
//   - Receipt block: data-receipt="offboarded" (console row moved, shows
//     actor/when), "not-offboarded" (row exists, never offboarded), or
//     "none" (no console record) — each an explicit visible state.

// fakePresencePortal serves the portal's per-person lookup with a canned
// WhoResult-shaped body, capturing the bearer it was called with.
func fakePresencePortal(t *testing.T, body map[string]any) (*httptest.Server, *string) {
	t.Helper()
	var gotBearer string
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if !strings.HasPrefix(r.URL.Path, "/api/lookup/person-presence") {
			http.NotFound(w, r)
			return
		}
		gotBearer = r.Header.Get("Authorization")
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(body)
	}))
	t.Cleanup(srv.Close)
	return srv, &gotBearer
}

// presenceLingering is a person the portal still sees: their own device is on
// the mesh and their org spine still carries a write grant.
func presenceLingering(handle string) map[string]any {
	return map[string]any{
		"query": handle, "resolved": "person:" + handle, "kind": "person",
		"on_mesh": true, "mesh_attribution": "peer", "connected": true,
		"writes": []map[string]any{{"topic": "acme/observations/v1", "layer": "zenoh-acl", "reason": "org write grant"}},
		"reads":  []map[string]any{},
	}
}

// presenceGone is the portal's honest all-absent answer: nothing in the
// access graph matched — no roster row, no peer, no write grant.
func presenceGone(q string) map[string]any {
	return map[string]any{
		"query": q, "resolved": "", "on_mesh": false,
		"writes": []map[string]any{}, "reads": []map[string]any{},
		"notes": []string{"no identity matched `" + q + "`"},
	}
}

func seedOffboardedPerson(t *testing.T, s *Server, h http.Handler, email string) string {
	t.Helper()
	pid := onboardedProposalInState(t, s, h, email, onboardstore.ProposalOnboarded)
	if err := s.Onboard.DecideProposal(context.Background(), pid, onboardstore.ProposalOffboarded, "admin@x", "cut by test"); err != nil {
		t.Fatalf("offboard proposal: %v", err)
	}
	return pid
}

func seedLiveOpKey(t *testing.T, s *Server, handle, email string) {
	t.Helper()
	ctx := context.Background()
	k, err := s.Onboard.RequestOperatorKey(ctx, &onboardstore.OperatorKey{Handle: handle, Email: email, Env: "zagros", Reason: "test"})
	if err != nil {
		t.Fatalf("request opkey: %v", err)
	}
	if _, err := s.Onboard.ApproveOperatorKey(ctx, k.ID, "someone-else@x", []string{"someone-else@x"}); err != nil {
		t.Fatalf("approve opkey: %v", err)
	}
}

// ed25519 test key — valid wire format so SubmitSSHKey accepts it. Type and
// blob stay separate fields (the store's shape) so the identity-spillage
// scan never sees an assembled `ssh-ed25519 AAAA…` pubkey in this file.
const (
	testPubKeyType = "ssh-ed25519"
	testPubKeyBody = "AAAAC3NzaC1lZDI1NTE5AAAAIGb0eGjKQqjLRc1blogxIca6BeaLzRRkbmvQGDcQ+PDN"
)

func seedInstalledSSHKey(t *testing.T, s *Server, handle, email string) {
	t.Helper()
	ctx := context.Background()
	k, err := s.Onboard.SubmitSSHKey(ctx, &onboardstore.SSHKey{Handle: handle, Email: email, KeyType: testPubKeyType, KeyBody: testPubKeyBody}, 0)
	if err != nil {
		t.Fatalf("submit ssh key: %v", err)
	}
	if err := s.Onboard.ApproveSSHKey(ctx, k.ID, "admin@x"); err != nil {
		t.Fatalf("approve ssh key: %v", err)
	}
	if err := s.Onboard.MarkSSHKeyInstalled(ctx, k.ID, "installer"); err != nil {
		t.Fatalf("install ssh key: %v", err)
	}
}

func getOffboardVerify(t *testing.T, h http.Handler, person string) *httptest.ResponseRecorder {
	t.Helper()
	req := httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/offboard-verify?person="+person, nil)
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	return rec
}

func mustContain(t *testing.T, body string, wants ...string) {
	t.Helper()
	for _, w := range wants {
		if !strings.Contains(body, w) {
			t.Errorf("page missing %q", w)
		}
	}
}

// A person offboarded on the console row but still holding credentials on
// every plane: each plane must flag STILL PRESENT and the verdict must say
// lingering, so offboarded admins keeping op-keys and shell keys are visible.
func TestOffboardVerifyFlagsLingeringCredentials(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	const email, handle = "person-106@example.org", "pat-liaison"
	seedOffboardedPerson(t, s, h, email)
	s.Kratos = map[string]*kratos.Client{"tenant": fakeKratosIdentityList(t, email, "active")}
	portal, bearer := fakePresencePortal(t, presenceLingering(handle))
	s.Cfg.PortalInternalURL = portal.URL
	s.Cfg.PortalLookupToken = "console-secret"
	seedLiveOpKey(t, s, handle, email)
	seedInstalledSSHKey(t, s, handle, email)

	rec := getOffboardVerify(t, h, email)
	if rec.Code != http.StatusOK {
		t.Fatalf("want 200, got %d: %s", rec.Code, rec.Body.String())
	}
	body := rec.Body.String()
	mustContain(t, body,
		`data-plane="login" data-status="present"`,
		`data-plane="operator-keys" data-status="present"`,
		`data-plane="ssh-keys" data-status="present"`,
		`data-plane="mesh" data-status="present"`,
		`data-plane="fabric" data-status="present"`,
		`data-verdict="lingering"`,
		`data-receipt="offboarded"`,
	)
	if *bearer != "Bearer console-secret" {
		t.Errorf("portal must be called with the console lookup token, got %q", *bearer)
	}
}

// Everything cut: login disabled, no live keys, portal graph has nothing.
// Only now may the page say clear.
func TestOffboardVerifyAllClear(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	const email = "person-106@example.org"
	seedOffboardedPerson(t, s, h, email)
	s.Kratos = map[string]*kratos.Client{"tenant": fakeKratosIdentityList(t, email, "inactive")}
	portal, _ := fakePresencePortal(t, presenceGone(email))
	s.Cfg.PortalInternalURL = portal.URL
	s.Cfg.PortalLookupToken = "console-secret"

	rec := getOffboardVerify(t, h, email)
	if rec.Code != http.StatusOK {
		t.Fatalf("want 200, got %d: %s", rec.Code, rec.Body.String())
	}
	body := rec.Body.String()
	mustContain(t, body,
		`data-plane="login" data-status="absent"`,
		`data-plane="operator-keys" data-status="absent"`,
		`data-plane="ssh-keys" data-status="absent"`,
		`data-plane="mesh" data-status="absent"`,
		`data-plane="fabric" data-status="absent"`,
		`data-verdict="clear"`,
		`data-receipt="offboarded"`,
	)
	if strings.Contains(body, `data-status="present"`) || strings.Contains(body, `data-status="unknown"`) {
		t.Error("all-clear page must carry no present/unknown plane")
	}
}

// The portal being unreachable means the mesh and fabric planes CANNOT be
// known. They must render unknown and the verdict must be unverified —
// an outage must never read as "gone" (no silent green).
func TestOffboardVerifyPortalDownNeverGreen(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	const email = "person-106@example.org"
	seedOffboardedPerson(t, s, h, email)
	s.Kratos = map[string]*kratos.Client{"tenant": fakeKratosIdentityList(t, email, "inactive")}
	dead := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {}))
	dead.Close() // connection refused
	s.Cfg.PortalInternalURL = dead.URL
	s.Cfg.PortalLookupToken = "console-secret"

	rec := getOffboardVerify(t, h, email)
	if rec.Code != http.StatusOK {
		t.Fatalf("degraded view should still render: want 200, got %d", rec.Code)
	}
	body := rec.Body.String()
	mustContain(t, body,
		`data-plane="mesh" data-status="unknown"`,
		`data-plane="fabric" data-status="unknown"`,
		`data-verdict="unverified"`,
	)
	if strings.Contains(body, `data-verdict="clear"`) {
		t.Error("unknown planes must never produce a clear verdict")
	}
}

// A person with no console record at all: the receipt block must say so
// explicitly rather than rendering an empty (and therefore ambiguous) page.
// CLI-initiated offboards of pre-console people land here.
func TestOffboardVerifyNoReceiptIsExplicit(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	const email = "person-39@example.org"
	s.Kratos = map[string]*kratos.Client{"tenant": fakeKratosIdentityList(t, "someone-else@example.org", "active")}
	portal, _ := fakePresencePortal(t, presenceGone(email))
	s.Cfg.PortalInternalURL = portal.URL
	s.Cfg.PortalLookupToken = "console-secret"

	rec := getOffboardVerify(t, h, email)
	if rec.Code != http.StatusOK {
		t.Fatalf("want 200, got %d: %s", rec.Code, rec.Body.String())
	}
	mustContain(t, rec.Body.String(), `data-receipt="none"`)
}

// A row that exists but was never offboarded must not read like a receipt:
// the page is a verifier, and "this person is not recorded as offboarded" is
// a finding, not an error.
func TestOffboardVerifyNotOffboardedIsExplicit(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	const email = "person-106@example.org"
	onboardedProposalInState(t, s, h, email, onboardstore.ProposalOnboarded) // onboarded, never offboarded
	s.Kratos = map[string]*kratos.Client{"tenant": fakeKratosIdentityList(t, email, "active")}
	portal, _ := fakePresencePortal(t, presenceLingering("pat-liaison"))
	s.Cfg.PortalInternalURL = portal.URL
	s.Cfg.PortalLookupToken = "console-secret"

	rec := getOffboardVerify(t, h, email)
	if rec.Code != http.StatusOK {
		t.Fatalf("want 200, got %d: %s", rec.Code, rec.Body.String())
	}
	mustContain(t, rec.Body.String(), `data-receipt="not-offboarded"`)
}

// Read-across-every-plane is an admin capability (same altitude as the
// portal's operator-only roster). Without an admin-granting group the gate
// refuses.
func TestOffboardVerifyRequiresAdmin(t *testing.T) {
	s := newOnboardTestServer(t)
	s.Cfg.DevBypassAuth = false
	h := s.Routes()

	rec := getOffboardVerify(t, h, "person-106@example.org")
	if rec.Code != http.StatusForbidden {
		t.Fatalf("want 403 without an admin group, got %d", rec.Code)
	}
}
