package handlers

import (
	"context"
	"net/http"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

func queueAndClaimOffboard(t *testing.T, s *Server, h http.Handler) string {
	t.Helper()
	id := seedOffboardRow(t, s, onboardstore.ProposalOnboarded)
	if ok, err := s.Onboard.RequestOffboardBy(context.Background(), id, "admin@example.org", testActorID); err != nil || !ok {
		t.Fatalf("request offboard: %v %v", ok, err)
	}
	if c := postMachine(t, h, "/api/onboard/proposals/"+id+"/offboard-claim", nil); c.code != http.StatusOK {
		t.Fatalf("claim: %d %s", c.code, c.body)
	}
	return id
}

// When the runner reports that the cut's operator-action record was not
// written, the row keeps that, the Offboarded card says so instead of an
// unqualified success, and the console audits the result as partial.
func TestOffboardResultCarriesOperatorAuditFailure(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	id := queueAndClaimOffboard(t, s, h)
	res := postMachine(t, h, "/api/onboard/proposals/"+id+"/result", map[string]any{
		"status": onboardstore.ProposalOffboarded, "operator_audit": "failed",
		"operator_audit_error": "started: not on the audit fabric (router unreachable); kept in /repo/out/audit/operator-actions.log",
	})
	if res.code != http.StatusOK {
		t.Fatalf("result: %d %s", res.code, res.body)
	}
	p := mustGetProposal(t, s, id)
	if p.Status != onboardstore.ProposalOffboarded || !strings.HasPrefix(p.OffboardOperatorAudit, "failed: started: not on the audit fabric") {
		t.Fatalf("row = %q / %q", p.Status, p.OffboardOperatorAudit)
	}
	page := adminGet(t, h, "/onboarding?status=offboarded&id="+id)
	if !strings.Contains(page, "Offboarded, operator-action record not written:") || !strings.Contains(page, "router unreachable") {
		t.Fatal("Offboarded card does not say the operator-action record was not written")
	}
	found := false
	for _, e := range s.Audit.Recent(50) {
		if e.Action == "onboard_offboard_result" {
			found = true
			if e.Result != "partial" || !strings.Contains(e.ErrorMessage, "operator-action record not written") || e.After["approver_identity"] != testActorID {
				t.Errorf("onboard_offboard_result = %s %q %v", e.Result, e.ErrorMessage, e.After)
			}
		}
	}
	if !found {
		t.Fatal("no onboard_offboard_result audit event")
	}
}

// A re-queue after a failed cut clears the first attempt's operator-audit
// mark, and a second attempt that reports no outcome (an older runner) lands
// as "not reported" on the Offboarded card, never as a stale "recorded".
func TestOffboardOperatorAuditIsPerAttempt(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	id := queueAndClaimOffboard(t, s, h)
	if res := postMachine(t, h, "/api/onboard/proposals/"+id+"/result", map[string]any{
		"status": onboardstore.ProposalOffboardFailed, "error": "mgmt API unreachable", "operator_audit": "recorded",
	}); res.code != http.StatusOK {
		t.Fatalf("first result: %d %s", res.code, res.body)
	}
	if p := mustGetProposal(t, s, id); p.Status != onboardstore.ProposalOffboardFailed || p.OffboardOperatorAudit != "recorded" {
		t.Fatalf("after attempt 1: %q / %q", p.Status, p.OffboardOperatorAudit)
	}
	if ok, err := s.Onboard.RequestOffboardBy(context.Background(), id, "admin@example.org", testActorID); err != nil || !ok {
		t.Fatalf("re-queue: %v %v", ok, err)
	}
	if p := mustGetProposal(t, s, id); p.OffboardOperatorAudit != "" {
		t.Fatalf("after re-queue the row still carries %q from the failed attempt", p.OffboardOperatorAudit)
	}
	if c := postMachine(t, h, "/api/onboard/proposals/"+id+"/offboard-claim", nil); c.code != http.StatusOK {
		t.Fatalf("re-claim: %d %s", c.code, c.body)
	}
	if res := postMachine(t, h, "/api/onboard/proposals/"+id+"/result", map[string]any{
		"status": onboardstore.ProposalOffboarded,
	}); res.code != http.StatusOK {
		t.Fatalf("second result: %d %s", res.code, res.body)
	}
	if p := mustGetProposal(t, s, id); p.Status != onboardstore.ProposalOffboarded || p.OffboardOperatorAudit != "" {
		t.Fatalf("after attempt 2: %q / %q, want offboarded with no mark", p.Status, p.OffboardOperatorAudit)
	}
	page := adminGet(t, h, "/onboarding?status=offboarded&id="+id)
	if !strings.Contains(page, "Operator-action record: not reported.") {
		t.Fatal("an offboarded cut with no reported outcome does not say so")
	}
}

// The status and the operator-audit mark are one statement: a single
// SetOffboardResult call leaves both, and it refuses a non-offboard status.
func TestSetOffboardResultWritesStatusAndMarkTogether(t *testing.T) {
	s := newOnboardTestServer(t)
	id := queueAndClaimOffboard(t, s, s.Routes())
	if err := s.Onboard.SetOffboardResult(context.Background(), id, onboardstore.ProposalOffboarded, "", "failed: not on the audit fabric"); err != nil {
		t.Fatal(err)
	}
	p := mustGetProposal(t, s, id)
	if p.Status != onboardstore.ProposalOffboarded || p.OffboardOperatorAudit != "failed: not on the audit fabric" {
		t.Fatalf("row = %q / %q", p.Status, p.OffboardOperatorAudit)
	}
	if err := s.Onboard.SetOffboardResult(context.Background(), id, onboardstore.ProposalOnboarded, "", ""); err == nil {
		t.Fatal("SetOffboardResult accepted an onboarding status")
	}
}

// A recorded outcome shows no warning; a "failed" without a reason, or an
// unknown outcome, is refused before anything is written.
func TestOffboardResultOperatorAuditValues(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	id := queueAndClaimOffboard(t, s, h)
	for _, bad := range []map[string]any{
		{"status": onboardstore.ProposalOffboarded, "operator_audit": "failed"},
		{"status": onboardstore.ProposalOffboarded, "operator_audit": "maybe"},
	} {
		if res := postMachine(t, h, "/api/onboard/proposals/"+id+"/result", bad); res.code != http.StatusBadRequest {
			t.Fatalf("%v: %d %s, want 400", bad, res.code, res.body)
		}
	}
	if res := postMachine(t, h, "/api/onboard/proposals/"+id+"/result", map[string]any{
		"status": onboardstore.ProposalOffboarded, "operator_audit": "recorded"}); res.code != http.StatusOK {
		t.Fatalf("recorded: %d %s", res.code, res.body)
	}
	if p := mustGetProposal(t, s, id); p.OffboardOperatorAudit != "recorded" {
		t.Fatalf("operator audit = %q, want recorded", p.OffboardOperatorAudit)
	}
	page := adminGet(t, h, "/onboarding?status=offboarded&id="+id)
	if strings.Contains(page, "operator-action record not written") || strings.Contains(page, "not reported.") {
		t.Fatal("a recorded cut shows a not-written or not-reported warning")
	}
}
