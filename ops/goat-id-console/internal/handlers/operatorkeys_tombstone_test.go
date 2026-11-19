package handlers

import (
	"context"
	"encoding/json"
	"net/http"
	"os"
	"path/filepath"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/opkeys"
)

// seedOpKeyHolder puts handle on the onboarding roster with the address the
// tests request under, so a request passes a roster check where one applies.
func seedOpKeyHolder(t *testing.T, s *Server, handle string) {
	t.Helper()
	if err := s.Onboard.CreateProposal(context.Background(), &onboardstore.Proposal{Sponsor: "sponsor-1", Env: "zagros",
		Handle: handle, Email: "x@example.org", Status: onboardstore.ProposalOnboarded, ManifestJSON: "{}", Tier: "platform-admin"}); err != nil {
		t.Fatalf("seed roster person %s: %v", handle, err)
	}
}

func requestOpKey(t *testing.T, h http.Handler, key, handle string) string {
	t.Helper()
	w := machineReq(t, h, key, "POST", "/api/onboard/operator-keys",
		`{"handle":"`+handle+`","email":"x@example.org","reason":"x"}`)
	if w.Code != http.StatusOK {
		t.Fatalf("request %q = %d %s", handle, w.Code, w.Body.String())
	}
	var row map[string]any
	if err := json.Unmarshal(w.Body.Bytes(), &row); err != nil {
		t.Fatal(err)
	}
	id, _ := row["id"].(string)
	return id
}

// GHSA-2w7v review, finding 1 (fail-to-pass). One operator principal requests
// a key for an env-tier handle and withdraws its own never-approved request.
// That must not tombstone the handle: a tombstone refuses the env key, so this
// was a single-actor way to disable a break-glass or service key. Reconcile
// --apply must not do it either.
func TestOperatorKeyRevoke_withdrawnRequestNeverTombstones(t *testing.T) {
	s := newOnboardTestServer(t)
	s.Cfg.OperatorKeyDir = t.TempDir()
	seedOpKeyHolder(t, s, "breakglass")
	h := s.Routes()

	id := requestOpKey(t, h, "runner-key", "breakglass")
	if w := machineReq(t, h, "runner-key", "POST", "/api/onboard/operator-keys/"+id+"/revoke", ""); w.Code != http.StatusOK {
		t.Fatalf("revoke = %d %s", w.Code, w.Body.String())
	}
	if ok, _ := s.operatorKeyDir().Tombstoned("breakglass"); ok {
		t.Fatal("a withdrawn, never-approved request wrote a tombstone for breakglass")
	}
	if w := machineReq(t, h, "runner-key", "POST", "/api/onboard/operator-keys/reconcile?apply=true", ""); w.Code != http.StatusOK {
		t.Fatalf("reconcile = %d %s", w.Code, w.Body.String())
	}
	if ok, _ := s.operatorKeyDir().Tombstoned("breakglass"); ok {
		t.Fatal("reconcile --apply tombstoned a handle the console never issued")
	}
}

// The advisory's case still holds end to end: revoking an ISSUED key writes
// the tombstone and removes the file.
func TestOperatorKeyRevoke_issuedKeyIsTombstoned(t *testing.T) {
	s := newOnboardTestServer(t)
	s.Cfg.OperatorKeyDir = t.TempDir()
	s.Cfg.OnboardOperatorAPIKeys["approver-key"] = "approver"
	seedOpKeyHolder(t, s, "ada")
	h := s.Routes()

	id := requestOpKey(t, h, "runner-key", "ada")
	if w := machineReq(t, h, "approver-key", "POST", "/api/onboard/operator-keys/"+id+"/approve", ""); w.Code != http.StatusOK {
		t.Fatalf("approve = %d %s", w.Code, w.Body.String())
	}
	if w := machineReq(t, h, "approver-key", "POST", "/api/onboard/operator-keys/"+id+"/revoke", ""); w.Code != http.StatusOK {
		t.Fatalf("revoke = %d %s", w.Code, w.Body.String())
	}
	if ok, _ := s.operatorKeyDir().Tombstoned("ada"); !ok {
		t.Fatal("revoking an issued key wrote no tombstone")
	}
	if _, err := os.Stat(filepath.Join(s.Cfg.OperatorKeyDir, "ada.key")); !os.IsNotExist(err) {
		t.Fatalf("key file still present: %v", err)
	}
}

// GHSA-2w7v review, finding 2. A handle a tombstone cannot name is refused at
// request time, so it can never be issued and then be unrevocable.
func TestOperatorKeyRequest_refusesHandleATombstoneCannotName(t *testing.T) {
	s := newOnboardTestServer(t)
	s.Cfg.OperatorKeyDir = t.TempDir()
	h := s.Routes()
	w := machineReq(t, h, "runner-key", "POST", "/api/onboard/operator-keys",
		`{"handle":"pat liaison","email":"pat@example.org","reason":"x"}`)
	if w.Code != http.StatusBadRequest {
		t.Fatalf("request with a space in the handle = %d %s, want 400", w.Code, w.Body.String())
	}
	if opkeys.CheckHandle("pat-liaison") != nil {
		t.Fatal("an ordinary handle must pass")
	}
}

// GHSA-2w7v review r2, finding 1 (fail-to-pass). One operator principal files
// a request for another handle, approves it itself and revokes it, which used
// to write a tombstone (and so disable an env-pinned key) with no second
// person. Approval must now need a principal other than the requester.
func TestOperatorKeyApprove_requesterCannotApproveOwnRequest(t *testing.T) {
	s := newOnboardTestServer(t)
	s.Cfg.OperatorKeyDir = t.TempDir()
	seedOpKeyHolder(t, s, "breakglass")
	h := s.Routes()

	id := requestOpKey(t, h, "runner-key", "breakglass")
	w := machineReq(t, h, "runner-key", "POST", "/api/onboard/operator-keys/"+id+"/approve", "")
	if w.Code != http.StatusForbidden {
		t.Fatalf("approve by the requesting principal = %d %s, want 403", w.Code, w.Body.String())
	}
	machineReq(t, h, "runner-key", "POST", "/api/onboard/operator-keys/"+id+"/revoke", "")
	if ok, _ := s.operatorKeyDir().Tombstoned("breakglass"); ok {
		t.Fatal("one operator principal wrote a revocation tombstone for breakglass")
	}
	row, err := s.Onboard.OperatorKey(context.Background(), id)
	if err != nil {
		t.Fatal(err)
	}
	if row.RequestedBy != "onboard-runner" {
		t.Errorf("RequestedBy = %q, want the filing principal onboard-runner", row.RequestedBy)
	}
}
