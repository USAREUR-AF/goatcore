package handlers

import (
	"context"
	"net/http"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/kratos"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// GHSA-2w7v. A revoked operator key is verifiably gone only if its revocation
// tombstone is on disk: without one, a key for the handle pinned in the
// portal's PORTAL_OPERATOR_API_KEYS is still accepted. The plane must say
// unknown (never clear) until the tombstone exists.
func TestOffboardVerifyRevokedKeyWithoutTombstoneIsNotClear(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	const email, handle = "person-106@example.org", "pat-liaison"
	seedOffboardedPerson(t, s, h, email)
	s.Kratos = map[string]*kratos.Client{"tenant": fakeKratosIdentityList(t, email, "inactive")}
	portal, _ := fakePresencePortal(t, presenceGone(email))
	s.Cfg.PortalInternalURL = portal.URL
	s.Cfg.PortalLookupToken = "console-secret"
	s.Cfg.OperatorKeyDir = t.TempDir()

	ctx := context.Background()
	k, err := s.Onboard.RequestOperatorKey(ctx, &onboardstore.OperatorKey{Handle: handle, Email: email, Env: "zagros", Reason: "test"})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := s.Onboard.ApproveOperatorKey(ctx, k.ID, "someone-else@x", []string{"someone-else@x"}); err != nil {
		t.Fatal(err)
	}
	// Revoked in the record the way it was before tombstones: no tombstone.
	if _, err := s.Onboard.RevokeOperatorKey(ctx, k.ID, "admin@x", "pre-tombstone revoke"); err != nil {
		t.Fatal(err)
	}

	rec := getOffboardVerify(t, h, email)
	if rec.Code != http.StatusOK {
		t.Fatalf("want 200, got %d", rec.Code)
	}
	mustContain(t, rec.Body.String(),
		`data-plane="operator-keys" data-status="unknown"`,
		`data-verdict="unverified"`,
	)

	if err := s.operatorKeyDir().Revoke(handle); err != nil {
		t.Fatal(err)
	}
	rec = getOffboardVerify(t, h, email)
	mustContain(t, rec.Body.String(),
		`data-plane="operator-keys" data-status="absent"`,
		`data-verdict="clear"`,
	)
}
