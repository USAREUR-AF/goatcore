package handlers

import (
	"context"
	"net/http"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// Revoking an invite through the operator API (`goat admin invites revoke`)
// records the same onboard_revoke_invite event the browser button does,
// attributed to the key's handle. Before, the API path changed the invite and
// left no audit record at all, and the CLI emits none of its own.
func TestAPIRevokeInviteIsAudited(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	inv := &onboardstore.Invite{Sponsor: "dev@bypass.local", Env: "zagros"}
	if _, err := s.Onboard.MintInvite(context.Background(), inv); err != nil {
		t.Fatal(err)
	}

	if res := postMachine(t, h, "/api/onboard/invites/"+inv.ID+"/revoke", nil); res.code != http.StatusOK {
		t.Fatalf("revoke: %d %s", res.code, res.body)
	}
	// A second revoke is refused (no longer open) and that refusal is recorded too.
	if res := postMachine(t, h, "/api/onboard/invites/"+inv.ID+"/revoke", nil); res.code != http.StatusConflict {
		t.Fatalf("second revoke: %d %s, want 409", res.code, res.body)
	}

	var results []string
	for _, e := range s.Audit.Recent(50) {
		if e.Action != "onboard_revoke_invite" {
			continue
		}
		if e.Actor.Email != "onboard-runner" {
			t.Errorf("actor = %q, want the operator key's handle onboard-runner", e.Actor.Email)
		}
		if e.Target.InviteTokenPrefix != inv.ID {
			t.Errorf("target = %q, want invite %s", e.Target.InviteTokenPrefix, inv.ID)
		}
		results = append(results, e.Result)
	}
	if len(results) != 2 {
		t.Fatalf("onboard_revoke_invite events = %v, want one ok and one error", results)
	}
	seen := map[string]bool{results[0]: true, results[1]: true}
	if !seen["ok"] || !seen["error"] {
		t.Fatalf("onboard_revoke_invite results = %v, want ok and error", results)
	}
}
