package handlers

import (
	"context"
	"encoding/json"
	"net/http"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// The offboard queue (goat#372): the console records an admin-approved real
// cut as offboard-requested; the onboard-runner claims it atomically
// (offboard-requested → offboarding) and writes the terminal result back
// (offboarded | offboard-failed). This is the console half of the runner
// contract whose runner half lives in ops/onboard-runner/offboard_test.go —
// the state names and transitions must match there and in the fake portal.

// seedOffboardRow inserts a proposal directly in the store and advances it to
// the given lifecycle status.
func seedOffboardRow(t *testing.T, s *Server, status string) string {
	t.Helper()
	p := &onboardstore.Proposal{Sponsor: "sponsor@example.org", Env: "zagros", Handle: "jane",
		Email: "person-47@example.org", ManifestJSON: "{}"}
	if err := s.Onboard.CreateProposal(context.Background(), p); err != nil {
		t.Fatalf("create proposal: %v", err)
	}
	if err := s.Onboard.SetProposalStatus(context.Background(), p.ID, status); err != nil {
		t.Fatalf("set status %s: %v", status, err)
	}
	return p.ID
}

func mustGetProposal(t *testing.T, s *Server, id string) *onboardstore.Proposal {
	t.Helper()
	p, err := s.Onboard.GetProposal(context.Background(), id)
	if err != nil {
		t.Fatalf("get proposal: %v", err)
	}
	return p
}

// The full arc: request → visible on the runner's poll with the approving
// admin attached → claimed exactly once → terminal offboarded.
func TestOffboardQueue_FullArc(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	id := seedOffboardRow(t, s, onboardstore.ProposalOnboarded)

	ok, err := s.Onboard.RequestOffboard(context.Background(), id, "adminbob")
	if err != nil || !ok {
		t.Fatalf("RequestOffboard = %v, %v — want true from an onboarded row", ok, err)
	}

	// The runner's poll: the row lists under offboard-requested and carries
	// who approved the cut (goat#372 spec C — that's the audit attribution).
	list := getMachine(t, h, "/api/onboard/proposals?status="+onboardstore.ProposalOffboardRequested)
	if list.code != http.StatusOK {
		t.Fatalf("list offboard-requested: want 200, got %d: %s", list.code, list.body)
	}
	var rows []onboardstore.Proposal
	if err := json.Unmarshal([]byte(list.body), &rows); err != nil {
		t.Fatalf("decode list: %v", err)
	}
	if len(rows) != 1 || rows[0].ID != id {
		t.Fatalf("list = %+v, want the requested row", rows)
	}
	if rows[0].OffboardRequestedBy != "adminbob" {
		t.Errorf("offboard_requested_by = %q, want adminbob", rows[0].OffboardRequestedBy)
	}

	// Exactly one claim wins; the loser's re-claim conflicts (two runners, or
	// one runner re-polling a row it already holds).
	claim := postMachine(t, h, "/api/onboard/proposals/"+id+"/offboard-claim", nil)
	if claim.code != http.StatusOK {
		t.Fatalf("offboard-claim: want 200, got %d: %s", claim.code, claim.body)
	}
	if got := mustGetProposal(t, s, id).Status; got != onboardstore.ProposalOffboarding {
		t.Fatalf("after claim status = %q, want offboarding", got)
	}
	if again := postMachine(t, h, "/api/onboard/proposals/"+id+"/offboard-claim", nil); again.code != http.StatusConflict {
		t.Fatalf("second offboard-claim: want 409, got %d: %s", again.code, again.body)
	}

	res := postMachine(t, h, "/api/onboard/proposals/"+id+"/result",
		map[string]any{"status": onboardstore.ProposalOffboarded})
	if res.code != http.StatusOK {
		t.Fatalf("result offboarded: want 200, got %d: %s", res.code, res.body)
	}
	if got := mustGetProposal(t, s, id).Status; got != onboardstore.ProposalOffboarded {
		t.Fatalf("terminal status = %q, want offboarded", got)
	}
}

// The request gate: only post-provisioning rows can queue a real cut, and a
// row already queued cannot be queued twice (two admins clicking at once —
// the second click is a no-op, not a second revoke).
func TestOffboardQueue_RequestGuards(t *testing.T) {
	s := newOnboardTestServer(t)

	pre := seedOffboardRow(t, s, onboardstore.ProposalApproved)
	if ok, err := s.Onboard.RequestOffboard(context.Background(), pre, "adminbob"); err != nil || ok {
		t.Errorf("RequestOffboard on approved = %v, %v — a pre-provisioning row is rejected, not offboarded", ok, err)
	}

	id := seedOffboardRow(t, s, onboardstore.ProposalEnrolled)
	if ok, err := s.Onboard.RequestOffboard(context.Background(), id, "adminbob"); err != nil || !ok {
		t.Fatalf("first RequestOffboard = %v, %v — want true from enrolled", ok, err)
	}
	if ok, err := s.Onboard.RequestOffboard(context.Background(), id, "adminalice"); err != nil || ok {
		t.Errorf("second RequestOffboard = %v, %v — want false, the cut is already queued", ok, err)
	}
	// The first requester's attribution survives the second click.
	if got := mustGetProposal(t, s, id).OffboardRequestedBy; got != "adminbob" {
		t.Errorf("offboard_requested_by = %q, want the FIRST requester adminbob", got)
	}
}

// A failed cut stays loud and retryable: the runner writes offboard-failed
// with the error (the person still has access), and an admin can re-queue it.
func TestOffboardQueue_FailureIsLoudAndRetryable(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	id := seedOffboardRow(t, s, onboardstore.ProposalOnboarded)

	if ok, _ := s.Onboard.RequestOffboard(context.Background(), id, "adminbob"); !ok {
		t.Fatal("request failed")
	}
	if c := postMachine(t, h, "/api/onboard/proposals/"+id+"/offboard-claim", nil); c.code != http.StatusOK {
		t.Fatalf("claim: %d %s", c.code, c.body)
	}
	res := postMachine(t, h, "/api/onboard/proposals/"+id+"/result",
		map[string]any{"status": onboardstore.ProposalOffboardFailed, "error": "revoke: peer cut refused"})
	if res.code != http.StatusOK {
		t.Fatalf("result offboard-failed: want 200, got %d: %s", res.code, res.body)
	}
	p := mustGetProposal(t, s, id)
	if p.Status != onboardstore.ProposalOffboardFailed {
		t.Fatalf("status = %q, want offboard-failed", p.Status)
	}
	if !strings.Contains(p.Note, "peer cut refused") {
		t.Errorf("note = %q — the failure reason must be visible to the admin queue", p.Note)
	}

	if ok, err := s.Onboard.RequestOffboard(context.Background(), id, "adminalice"); err != nil || !ok {
		t.Errorf("re-request after failure = %v, %v — want true (retry path)", ok, err)
	}
}

// The claim gate: a row nobody queued cannot be claimed — the runner never
// cuts anyone on its own initiative.
func TestOffboardQueue_ClaimRequiresRequestedState(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	id := seedOffboardRow(t, s, onboardstore.ProposalOnboarded)

	if c := postMachine(t, h, "/api/onboard/proposals/"+id+"/offboard-claim", nil); c.code != http.StatusConflict {
		t.Errorf("offboard-claim on onboarded: want 409, got %d: %s", c.code, c.body)
	}
	if got := mustGetProposal(t, s, id).Status; got != onboardstore.ProposalOnboarded {
		t.Errorf("status = %q — a refused claim must not move the row", got)
	}
}

// Results respect which leg claimed the row: an onboarding row cannot take an
// offboard result and an offboarding row cannot take an onboard result — a
// crossed writeback is a 409 state conflict, not a silent lifecycle jump.
func TestOffboardQueue_ResultCrossStateGuards(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()

	onboarding := seedOffboardRow(t, s, onboardstore.ProposalOnboarding)
	if r := postMachine(t, h, "/api/onboard/proposals/"+onboarding+"/result",
		map[string]any{"status": onboardstore.ProposalOffboarded}); r.code != http.StatusConflict {
		t.Errorf("offboarded result on an onboarding row: want 409, got %d: %s", r.code, r.body)
	}

	offboarding := seedOffboardRow(t, s, onboardstore.ProposalOffboarding)
	if r := postMachine(t, h, "/api/onboard/proposals/"+offboarding+"/result",
		map[string]any{"status": onboardstore.ProposalOnboarded}); r.code != http.StatusConflict {
		t.Errorf("onboarded result on an offboarding row: want 409, got %d: %s", r.code, r.body)
	}
}
