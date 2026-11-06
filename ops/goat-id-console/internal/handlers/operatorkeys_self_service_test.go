package handlers

// Self-request door (#1373): a non-operator caller — a participant's own vp_
// key, resolved by onboardAPIGate the same way every other machine-realm
// route resolves one — may request an operator key for THEMSELVES ONLY. This
// is the CLI-side counterpart of the console's session-authenticated
// self-service /operator-key page: before this, `goat admin operator-key
// request` required already holding an operator key, so a first-time
// platform-admin had no CLI door at all. Widening who may REQUEST changes
// nothing about who may APPROVE — the two-person rule (operatorkeys_twoperson_test.go)
// and the collect-once binding (operatorkeys_collect_authz_test.go) are
// untouched and re-asserted here for the non-operator path specifically.
//
// "Themselves" is the ROSTER handle. A participant principal's Handle is its
// org anchor (on a slot-pool goatnet a 32-hex UUID), so the door binds through
// the login the portal recorded on the identity and the onboarding record
// filed under it — the same resolver the browser page uses. The stub keys
// below therefore carry a SponsorEmail, and each has an Onboarded proposal.

import (
	"context"
	"encoding/json"
	"net/http"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// newSelfDoorServer wires participant keys through the stubbed portal whoami:
//   - "robin-key": platform-admin cap, login robin@example.org, onboarded as "robin";
//   - "other-key": platform-admin cap, login else@example.org, onboarded as "somebody-else";
//   - "dev-key": NOT sponsor-authorized, so onboardAPIGate refuses it upstream.
func newSelfDoorServer(t *testing.T) (*Server, http.Handler) {
	t.Helper()
	s := newOnboardTestServer(t)
	portalURL, rt := fakeSponsorPortal(t, nil, nil)
	rt.keys["robin-key"] = portalWhoami{
		Org: "7b9b61a4ae2373d6f0a0535d2b72b4ea", Alias: "robin-operator",
		SponsorAuthorized: true, SponsorMaxTier: "platform-admin", SponsorEmail: "robin@example.org",
	}
	rt.keys["other-key"] = portalWhoami{
		Org: "00220022", Alias: "else-operator",
		SponsorAuthorized: true, SponsorMaxTier: "platform-admin", SponsorEmail: "else@example.org",
	}
	rt.keys["dev-key"] = portalWhoami{Org: "some-dev-org", SponsorAuthorized: false}
	s.Cfg.PortalInternalURL = portalURL
	seedPlatformAdminPerson(t, s, "robin", "robin@example.org")
	seedPlatformAdminPerson(t, s, "somebody-else", "else@example.org")
	return s, s.Routes()
}

// seedPlatformAdminPerson files an Onboarded platform-admin proposal so the
// email → handle resolver has a record to bind through and the ADR 1182 tier
// gate lets the request past. main's seedOnboardedPerson
// (operatorkeys_eligibility_test.go) seeds a person with no tier.
func seedPlatformAdminPerson(t *testing.T, s *Server, handle, email string) {
	t.Helper()
	p := &onboardstore.Proposal{
		ID: "p-" + handle, Sponsor: "onboard-runner", Env: "zagros", Handle: handle,
		Email: email, Status: onboardstore.ProposalOnboarded, ManifestJSON: "{}",
		Tier: "platform-admin", Complete: true,
	}
	if err := s.Onboard.CreateProposal(context.Background(), p); err != nil {
		t.Fatalf("seed onboarded person %s: %v", handle, err)
	}
}

func TestApiRequestOperatorKey_selfServiceDoor(t *testing.T) {
	s, h := newSelfDoorServer(t)

	// Omitting "handle" in the body binds the request to the caller: the roster
	// handle behind the token, never the org anchor the principal carries. The
	// row also carries the bound login, so the approval mail has an
	// address.
	w := machineReq(t, h, "robin-key", "POST", "/api/onboard/operator-keys",
		`{"env":"zagros","reason":"org creation via goat CLI"}`)
	if w.Code != http.StatusOK {
		t.Fatalf("self-request (handle omitted) = %d, want 200; %s", w.Code, w.Body.String())
	}
	var row onboardstore.OperatorKey
	if err := json.Unmarshal(w.Body.Bytes(), &row); err != nil {
		t.Fatalf("decode response: %v", err)
	}
	if row.Handle != "robin" || row.Email != "robin@example.org" {
		t.Fatalf("self-request row = %+v, want the roster handle robin and login robin@example.org", row)
	}
	if row.Status != onboardstore.OperatorKeyRequested {
		t.Fatalf("self-request status = %q, want %q", row.Status, onboardstore.OperatorKeyRequested)
	}
	if after := auditAfter(t, s, "operator_key_request"); after["self"] != true || after["handle"] != "robin" {
		t.Fatalf("audit after = %v, want self for robin", after)
	}

	// Explicitly naming your OWN handle in the body, in any case, works the same
	// way and lands on the same live row (the store dedupes per handle and env).
	w2 := machineReq(t, h, "robin-key", "POST", "/api/onboard/operator-keys",
		`{"handle":"Robin","env":"zagros","reason":"again"}`)
	if w2.Code != http.StatusOK {
		t.Fatalf("self-request (own handle named) = %d, want 200; %s", w2.Code, w2.Body.String())
	}
	var row2 onboardstore.OperatorKey
	_ = json.Unmarshal(w2.Body.Bytes(), &row2)
	if row2.ID != row.ID {
		t.Fatalf("re-requesting while one is pending should return the SAME row (dedup), got a new id %q vs %q", row2.ID, row.ID)
	}

	// Naming somebody ELSE's handle is refused and audited — a non-operator key
	// never gets to act "for somebody else"; that stays the operator door. The
	// org anchor is "somebody else" too: it is not a person.
	for _, other := range []string{"someone-else", "7b9b61a4ae2373d6f0a0535d2b72b4ea"} {
		w3 := machineReq(t, h, "robin-key", "POST", "/api/onboard/operator-keys",
			`{"handle":"`+other+`","env":"zagros","reason":"for a colleague"}`)
		if w3.Code != http.StatusForbidden || !strings.Contains(w3.Body.String(), "self_only") {
			t.Fatalf("self-request for %s = %d, want 403 self_only; %s", other, w3.Code, w3.Body.String())
		}
	}
	if after := auditAfter(t, s, "operator_key_self_request_refused"); after["caller"] != "robin" {
		t.Fatalf("refusal audit after = %v, want the caller named", after)
	}
	rows, err := s.Onboard.ListOperatorKeys(context.Background(), "")
	if err != nil {
		t.Fatalf("list: %v", err)
	}
	if len(rows) != 1 {
		t.Fatalf("refused requests recorded rows: %+v", rows)
	}

	// A non-sponsor-authorized participant key never reaches this handler at
	// all — onboardAPIGate refuses it upstream (existing ADR 1071 behavior,
	// re-asserted here so a future change to that gate is caught by this file
	// too, not just sponsor_scope_test.go).
	w4 := machineReq(t, h, "dev-key", "POST", "/api/onboard/operator-keys", `{"env":"zagros","reason":"x"}`)
	if w4.Code != http.StatusForbidden {
		t.Fatalf("non-sponsor-authorized self-request = %d, want 403 (refused by onboardAPIGate, not this handler); %s", w4.Code, w4.Body.String())
	}

	// The operator door is completely unchanged: it may still name any handle,
	// with no reason, and the queue decides.
	w5 := machineReq(t, h, "runner-key", "POST", "/api/onboard/operator-keys",
		`{"handle":"someone-else","env":"zagros"}`)
	if w5.Code != http.StatusOK {
		t.Fatalf("operator request for somebody else = %d, want 200; %s", w5.Code, w5.Body.String())
	}
	var row5 onboardstore.OperatorKey
	_ = json.Unmarshal(w5.Body.Bytes(), &row5)
	if row5.Handle != "someone-else" {
		t.Fatalf("operator request handle = %q, want someone-else", row5.Handle)
	}
	if after := auditAfter(t, s, "operator_key_request"); after["self"] != false || after["via"] != "api" {
		t.Fatalf("on-behalf audit after = %v, want via api, not self", after)
	}
}

func TestApiCollectOperatorKey_selfServiceDoor(t *testing.T) {
	s, h := newSelfDoorServer(t)

	// Self-request, then a DIFFERENT admin approves (two-person rule) — a
	// non-operator principal cannot approve at all, so this uses the store
	// directly, matching how an approval actually happens in practice.
	w := machineReq(t, h, "robin-key", "POST", "/api/onboard/operator-keys", `{"env":"zagros","reason":"first key"}`)
	if w.Code != http.StatusOK {
		t.Fatalf("self-request = %d; %s", w.Code, w.Body.String())
	}
	var row onboardstore.OperatorKey
	_ = json.Unmarshal(w.Body.Bytes(), &row)
	if _, err := s.Onboard.ApproveOperatorKey(context.Background(), row.ID, "admin", []string{"admin"}); err != nil {
		t.Fatalf("approve: %v", err)
	}

	// The self-requester collects their OWN key via the API — this is the
	// missing half of the round trip: before this change, apiCollectOperatorKey
	// was operator-only, so a self-requester could never actually retrieve the
	// secret their own request produced through the CLI. The delivery names
	// the roster handle, which is also the key file's name on the portal host.
	wc := machineReq(t, h, "robin-key", "GET", "/api/onboard/operator-keys/"+row.ID+"/collect", "")
	if wc.Code != http.StatusOK {
		t.Fatalf("self-collect own key = %d, want 200; %s", wc.Code, wc.Body.String())
	}
	var collected map[string]string
	_ = json.Unmarshal(wc.Body.Bytes(), &collected)
	if collected["handle"] != "robin" || !strings.HasPrefix(collected["key"], "op_") {
		t.Fatalf("self-collect delivery = %v, want handle robin and an op_ key", collected)
	}

	// A second collect attempt is refused (delivered exactly once) — unchanged
	// store-level behavior, re-asserted here for the non-operator path.
	wc2 := machineReq(t, h, "robin-key", "GET", "/api/onboard/operator-keys/"+row.ID+"/collect", "")
	if wc2.Code != http.StatusConflict {
		t.Fatalf("second self-collect = %d, want 409 already_delivered; %s", wc2.Code, wc2.Body.String())
	}

	// A different non-operator principal cannot collect robin's key by naming
	// his handle — the self-match check applies to every principal, operator or
	// not (H2, operatorkeys_collect_authz_test.go's rule, now exercised for a
	// non-operator caller too). Bound to self, another person's row reads as
	// absent, never as a leak, and its secret is not spent.
	w2 := machineReq(t, h, "other-key", "POST", "/api/onboard/operator-keys", `{"env":"zagros","reason":"mine"}`)
	var row2 onboardstore.OperatorKey
	_ = json.Unmarshal(w2.Body.Bytes(), &row2)
	if _, err := s.Onboard.ApproveOperatorKey(context.Background(), row2.ID, "admin", []string{"admin"}); err != nil {
		t.Fatalf("approve row2: %v", err)
	}
	wForeign := machineReq(t, h, "robin-key", "GET",
		"/api/onboard/operator-keys/"+row2.ID+"/collect?handle=somebody-else", "")
	if wForeign.Code != http.StatusForbidden {
		t.Fatalf("cross-principal collect by query handle = %d, want 403; %s", wForeign.Code, wForeign.Body.String())
	}
	wBound := machineReq(t, h, "robin-key", "GET", "/api/onboard/operator-keys/"+row2.ID+"/collect", "")
	if wBound.Code != http.StatusNotFound {
		t.Fatalf("collect other's row bound to self = %d, want 404; %s", wBound.Code, wBound.Body.String())
	}
	if cur, _ := s.Onboard.OperatorKey(context.Background(), row2.ID); !cur.Deliverable() {
		t.Fatalf("other's row after refused attempts is no longer deliverable: %+v", cur)
	}
}
