package handlers

// The rest of the self door's rules (#1373): a token that binds to no
// onboarded person is refused rather than filed under its org anchor; the
// door starts under the holder rule (platform-admin-lifecycle.md § Operator
// key, ADR 1182: a level below platform-admin is refused, a request needs a
// reason); every other verb on the door stays operator-only; and an operator
// key with no handle is a caller error, not a self request.

import (
	"context"
	"net/http"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// newSelfDoorRulesServer adds to newSelfDoorServer:
//   - "ines-key": iam-admin cap, login bound, onboarded as "ines";
//   - "nolink-key": platform-admin cap, but no login bound to the identity;
//   - "ghost-key": platform-admin cap, login bound, but no onboarding record.
func newSelfDoorRulesServer(t *testing.T) (*Server, http.Handler) {
	t.Helper()
	s := newOnboardTestServer(t)
	portalURL, rt := fakeSponsorPortal(t, nil, nil)
	rt.keys["robin-key"] = portalWhoami{
		Org: "7b9b61a4ae2373d6f0a0535d2b72b4ea", Alias: "robin-operator",
		SponsorAuthorized: true, SponsorMaxTier: "platform-admin", SponsorEmail: "robin@example.org",
	}
	rt.keys["ines-key"] = portalWhoami{
		Org: "00330033", Alias: "ines-admin", SponsorAuthorized: true, SponsorMaxTier: "iam-admin",
		SponsorEmail: "ines@example.org",
	}
	rt.keys["nolink-key"] = portalWhoami{
		Org: "00110011", Alias: "nolink", SponsorAuthorized: true, SponsorMaxTier: "platform-admin",
	}
	rt.keys["ghost-key"] = portalWhoami{
		Org: "00220022", Alias: "ghost", SponsorAuthorized: true, SponsorMaxTier: "platform-admin",
		SponsorEmail: "ghost@example.org",
	}
	s.Cfg.PortalInternalURL = portalURL
	seedPlatformAdminPerson(t, s, "robin", "robin@example.org")
	seedPlatformAdminPerson(t, s, "ines", "ines@example.org")
	return s, s.Routes()
}

func auditAfter(t *testing.T, s *Server, action string) map[string]interface{} {
	t.Helper()
	for _, e := range s.Audit.Recent(50) {
		if e.Action == action {
			return e.After
		}
	}
	t.Fatalf("no %q audit event emitted", action)
	return nil
}

func TestOperatorKeyAPI_selfDoorFollowsTheHolderRule(t *testing.T) {
	s, h := newSelfDoorRulesServer(t)

	// An iam-admin holds no operator key (ADR 1182): refused before any lookup,
	// the recorded level named, the attempt audited, nothing recorded.
	w := machineReq(t, h, "ines-key", "POST", "/api/onboard/operator-keys", `{"reason":"admin CLI use"}`)
	if w.Code != http.StatusForbidden || !strings.Contains(w.Body.String(), "not_platform_admin") {
		t.Fatalf("iam-admin self request = %d %s, want 403 not_platform_admin", w.Code, w.Body.String())
	}
	if !strings.Contains(w.Body.String(), "iam-admin") || !strings.Contains(w.Body.String(), "platform-admin grant") {
		t.Fatalf("refusal names neither the level nor the promotion path: %s", w.Body.String())
	}
	if after := auditAfter(t, s, "operator_key_self_request_refused"); after["tier"] != "iam-admin" {
		t.Fatalf("refusal audit after = %v, want tier iam-admin", after)
	}

	// A platform admin with no reason: 400, nothing recorded.
	w = machineReq(t, h, "robin-key", "POST", "/api/onboard/operator-keys", `{}`)
	if w.Code != http.StatusBadRequest || !strings.Contains(w.Body.String(), "missing_reason") {
		t.Fatalf("self request without reason = %d %s, want 400 missing_reason", w.Code, w.Body.String())
	}
	w = machineReq(t, h, "robin-key", "POST", "/api/onboard/operator-keys", `{"reason":"   "}`)
	if w.Code != http.StatusBadRequest {
		t.Fatalf("self request with blank reason = %d, want 400", w.Code)
	}

	rows, err := s.Onboard.ListOperatorKeys(context.Background(), "")
	if err != nil {
		t.Fatalf("list: %v", err)
	}
	if len(rows) != 0 {
		t.Fatalf("refused requests recorded %d row(s): %+v", len(rows), rows)
	}

	// The on-behalf door is not under the rule here: an operator key files for
	// an iam-admin with no reason, as before, and the queue decides.
	w = machineReq(t, h, "runner-key", "POST", "/api/onboard/operator-keys", `{"handle":"ines"}`)
	if w.Code != http.StatusOK {
		t.Fatalf("operator on-behalf request for ines = %d; %s", w.Code, w.Body.String())
	}
}

func TestOperatorKeyAPI_unboundParticipantRefused(t *testing.T) {
	s, h := newSelfDoorRulesServer(t)

	for _, key := range []string{"nolink-key", "ghost-key"} {
		w := machineReq(t, h, key, "POST", "/api/onboard/operator-keys", `{"reason":"x"}`)
		if w.Code != http.StatusForbidden || !strings.Contains(w.Body.String(), "unbound_principal") {
			t.Fatalf("%s request = %d %s, want 403 unbound_principal", key, w.Code, w.Body.String())
		}
		// The refusal names the door that can bind them.
		if !strings.Contains(w.Body.String(), "/operator-key") {
			t.Fatalf("%s refusal does not name the console page: %s", key, w.Body.String())
		}
		w = machineReq(t, h, key, "GET", "/api/onboard/operator-keys/any-id/collect", "")
		if w.Code != http.StatusForbidden || !strings.Contains(w.Body.String(), "unbound_principal") {
			t.Fatalf("%s collect = %d %s, want 403 unbound_principal", key, w.Code, w.Body.String())
		}
	}
	// Nothing was filed under an anchor.
	rows, err := s.Onboard.ListOperatorKeys(context.Background(), "")
	if err != nil {
		t.Fatalf("list: %v", err)
	}
	if len(rows) != 0 {
		t.Fatalf("unbound requests recorded %d row(s): %+v", len(rows), rows)
	}
}

func TestOperatorKeyAPI_participantStaysOutOfOperatorVerbs(t *testing.T) {
	s, h := newSelfDoorRulesServer(t)
	own := seedOperatorKeyRequest(t, s, "robin", "robin@example.org")

	cases := []struct{ method, path string }{
		{"GET", "/api/onboard/operator-keys"},
		{"POST", "/api/onboard/operator-keys/" + own.ID + "/approve"},
		{"POST", "/api/onboard/operator-keys/" + own.ID + "/reject"},
		{"POST", "/api/onboard/operator-keys/" + own.ID + "/revoke"},
		{"POST", "/api/onboard/operator-keys/reconcile"},
	}
	for _, c := range cases {
		w := machineReq(t, h, "robin-key", c.method, c.path, "")
		if w.Code != http.StatusForbidden || !strings.Contains(w.Body.String(), "operator_only") {
			t.Fatalf("%s %s = %d %s, want 403 operator_only", c.method, c.path, w.Code, w.Body.String())
		}
	}
	if cur, _ := s.Onboard.OperatorKey(context.Background(), own.ID); cur.Status != onboardstore.OperatorKeyRequested {
		t.Fatalf("own row after refused decisions = %q, want requested", cur.Status)
	}
}

func TestOperatorKeyAPI_operatorWithoutHandleIsACallerError(t *testing.T) {
	_, h := newSelfDoorRulesServer(t)

	// An operator principal binds to nobody but itself, and filing its own key
	// by omission would hide the on-behalf intent.
	w := machineReq(t, h, "runner-key", "POST", "/api/onboard/operator-keys", `{"reason":"x"}`)
	if w.Code != http.StatusBadRequest || !strings.Contains(w.Body.String(), "missing_handle") {
		t.Fatalf("operator request without handle = %d %s, want 400 missing_handle", w.Code, w.Body.String())
	}
}
