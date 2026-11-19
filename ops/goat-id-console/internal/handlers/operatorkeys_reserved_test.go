package handlers

import (
	"context"
	"net/http"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// GHSA-q7qx: an operator principal may request a key for another person, but
// not under a service label the portal trusts to delegate attribution.
func TestOperatorKeyAPI_refusesReservedServiceHandle(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	for _, handle := range []string{"operator", "id-console", "onboard-api", "Operator"} {
		w := machineReq(t, h, "runner-key", "POST", "/api/onboard/operator-keys",
			`{"handle":"`+handle+`","env":"zagros"}`)
		if w.Code != http.StatusBadRequest || !strings.Contains(w.Body.String(), "reserved_handle") {
			t.Errorf("request for %q = %d %s, want 400 reserved_handle", handle, w.Code, w.Body.String())
		}
	}
	refused := 0
	for _, e := range s.Audit.Recent(50) {
		if e.Action == "operator_key_request_refused" {
			refused++
		}
	}
	if refused != 4 {
		t.Errorf("refusal audit events = %d, want 4", refused)
	}
	// A person's own handle still works. The person is on the onboarding
	// roster, which an operator-realm request may also be required to name.
	if err := s.Onboard.CreateProposal(context.Background(), &onboardstore.Proposal{
		Sponsor: "sponsor-1", Env: "zagros", Handle: "jane-operator", Email: "jane@example.test",
		Status: onboardstore.ProposalOnboarded, ManifestJSON: "{}", Tier: "platform-admin",
	}); err != nil {
		t.Fatalf("seed roster record: %v", err)
	}
	if w := machineReq(t, h, "runner-key", "POST", "/api/onboard/operator-keys",
		`{"handle":"jane-operator","email":"jane@example.test","env":"zagros"}`); w.Code != http.StatusOK {
		t.Fatalf("request for jane-operator = %d; %s", w.Code, w.Body.String())
	}
}
