// applier_test.go — unit tests for the reconcile pipeline.
//
// Coverage targets per the prompt:
//   - Applier construction + planner registration + dispatch
//   - Plan happy + error paths
//   - Apply with mock RelayMutator, including:
//       * happy path → "ok"
//       * one mid-step failure → "partial"
//       * every step fails → "fail"
//       * PreFlight failure → fail before any wire move
//   - planReconcile against representative LiveState shapes:
//       * empty live (every declared peer would be added)
//       * matching live (no changes)
//       * rotation case (old pubkey present, new pubkey missing,
//         with peer.PubkeyHistory containing the old)
//       * unknown peer on relay + AllowRemove=true → remove
//       * unknown peer on relay + AllowRemove=false → no-op
//       * unreachable relay → diff carries Unreachable=true, no
//         mutation
//
// The mocks (MockLiveStateReader, MockRelayMutator,
// RecordingAuditEmitter) live in live_state.go + audit.go so tests
// in any file can use them.

package main

import (
	"context"
	"errors"
	"fmt"
	"strings"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/enrollment/peerset"
)

// fixedClock is a deterministic clock for audit + correlation ID tests.
func fixedClock(ts string) func() time.Time {
	t, err := time.Parse(time.RFC3339, ts)
	if err != nil {
		panic(err)
	}
	return func() time.Time { return t }
}

// twoPeerSet returns a tiny PeerSet with two peers + two relays.
// Stable input for the reconcile tests.
func twoPeerSet() *peerset.PeerSet {
	return &peerset.PeerSet{
		SchemaVersion: peerset.SchemaVersion,
		Peers: []peerset.Peer{
			{Name: "alpha", WGCP0Address: "198.18.0.10", CurrentPubkey: "PK_ALPHA", Site: "lab"},
			{Name: "bravo", WGCP0Address: "198.18.0.11", CurrentPubkey: "PK_BRAVO", Site: "lab"},
		},
		Relays: []peerset.Relay{
			{Name: "relay-rla", Endpoint: "relay-rla.example:51821", MeshAddress: "198.18.0.2"},
			{Name: "relay-rlb", Endpoint: "relay-rlb.example:51821", MeshAddress: "198.18.0.3"},
		},
	}
}

func savePeerSetForTest(t *testing.T, ps *peerset.PeerSet, dir string) string {
	t.Helper()
	path := dir + "/peerset.json"
	if err := ps.Save(path); err != nil {
		t.Fatalf("save peerset: %v", err)
	}
	return path
}

func TestNewApplier_RequiresPath(t *testing.T) {
	_, err := NewApplier(ApplierOptions{})
	if err == nil {
		t.Fatal("NewApplier with empty PeerSetPath should fail")
	}
	if !strings.Contains(err.Error(), "PeerSetPath required") {
		t.Errorf("err = %v, want PeerSetPath required", err)
	}
}

func TestNewApplier_Default_Has_Reconcile_Planner(t *testing.T) {
	ps := twoPeerSet()
	path := savePeerSetForTest(t, ps, t.TempDir())
	a, err := NewApplier(ApplierOptions{
		PeerSetPath: path,
		Live:        &MockLiveStateReader{State: LiveState{PerRelay: map[string]*RelayLiveState{}}},
	})
	if err != nil {
		t.Fatalf("NewApplier: %v", err)
	}
	plan, err := a.Plan(context.Background(), ReconcileIntent{})
	if err != nil {
		t.Fatalf("Plan: %v", err)
	}
	if plan.Intent.Kind() != "reconcile" {
		t.Errorf("plan.Intent.Kind() = %q, want reconcile", plan.Intent.Kind())
	}
	if !plan.DryRun {
		t.Error("Plan should return DryRun=true")
	}
	if plan.CorrelationID == "" {
		t.Error("CorrelationID should be populated")
	}
}

func TestRegisterPlanner_AcceptsCustomKind(t *testing.T) {
	ps := twoPeerSet()
	path := savePeerSetForTest(t, ps, t.TempDir())
	a, _ := NewApplier(ApplierOptions{PeerSetPath: path})

	called := false
	custom := func(env *PlanEnv, intent Intent) (*ApplyPlan, error) {
		called = true
		return &ApplyPlan{StatePlan: &PeerSetDiff{}, LivePlan: &LiveStateDiff{}}, nil
	}
	RegisterPlanner(a, "custom-kind", custom)

	type fakeIntent struct{}
	_ = fakeIntent{}
	plan, err := a.Plan(context.Background(), intentWithKind("custom-kind"))
	if err != nil {
		t.Fatalf("Plan: %v", err)
	}
	if !called {
		t.Error("planner should have fired")
	}
	if plan.Intent.Kind() != "custom-kind" {
		t.Errorf("got %q, want custom-kind", plan.Intent.Kind())
	}
}

func TestRegisterPlanner_PanicsOnDuplicate(t *testing.T) {
	defer func() {
		if r := recover(); r == nil {
			t.Fatal("expected panic on duplicate planner registration")
		}
	}()
	ps := twoPeerSet()
	path := savePeerSetForTest(t, ps, t.TempDir())
	a, _ := NewApplier(ApplierOptions{PeerSetPath: path})
	noop := func(env *PlanEnv, intent Intent) (*ApplyPlan, error) {
		return &ApplyPlan{}, nil
	}
	RegisterPlanner(a, "reconcile", noop) // duplicate — must panic
}

func TestApplier_Plan_NoLiveReader_LivePlanEmpty(t *testing.T) {
	ps := twoPeerSet()
	path := savePeerSetForTest(t, ps, t.TempDir())
	a, _ := NewApplier(ApplierOptions{PeerSetPath: path}) // no Live
	plan, err := a.Plan(context.Background(), ReconcileIntent{})
	if err != nil {
		t.Fatalf("Plan: %v", err)
	}
	if plan.LivePlan == nil {
		t.Fatal("LivePlan should be non-nil")
	}
	// With no live data, every declared relay produces an
	// "Unreachable" row.
	if len(plan.LivePlan.PerRelay) != len(ps.Relays) {
		t.Errorf("LivePlan rows = %d, want %d", len(plan.LivePlan.PerRelay), len(ps.Relays))
	}
	for _, rd := range plan.LivePlan.PerRelay {
		if !rd.Unreachable {
			t.Errorf("relay %s should be Unreachable; got %+v", rd.Relay, rd)
		}
	}
}

func TestReconcile_AddMissingPeer(t *testing.T) {
	ps := twoPeerSet()
	path := savePeerSetForTest(t, ps, t.TempDir())
	live := &MockLiveStateReader{State: LiveState{
		PerRelay: map[string]*RelayLiveState{
			"relay-rla": {Reachable: true, Peers: map[string]LivePeer{"PK_ALPHA": {Pubkey: "PK_ALPHA", AllowedIPs: "198.18.0.10/32,198.18.0.0/24"}}},
			"relay-rlb": {Reachable: true, Peers: map[string]LivePeer{"PK_ALPHA": {Pubkey: "PK_ALPHA", AllowedIPs: "198.18.0.10/32,198.18.0.0/24"}}},
		},
	}}
	mutator := &MockRelayMutator{}
	a, _ := NewApplier(ApplierOptions{PeerSetPath: path, Live: live, Relay: mutator})

	plan, err := a.Plan(context.Background(), ReconcileIntent{AllowAdd: true})
	if err != nil {
		t.Fatalf("Plan: %v", err)
	}
	// Expect bravo to be added on both relays.
	var addOps []LivePeerOp
	for _, rd := range plan.LivePlan.PerRelay {
		addOps = append(addOps, rd.WouldAdd...)
	}
	if len(addOps) != 2 {
		t.Errorf("WouldAdd count = %d, want 2", len(addOps))
	}
	for _, op := range addOps {
		if op.PeerName != "bravo" {
			t.Errorf("op.PeerName = %q, want bravo", op.PeerName)
		}
		if op.NewPubkey != "PK_BRAVO" {
			t.Errorf("op.NewPubkey = %q, want PK_BRAVO", op.NewPubkey)
		}
	}

	// Apply — two AddPeer calls + two PersistConfig calls.
	res, err := a.Apply(context.Background(), plan)
	if err != nil {
		t.Fatalf("Apply: %v", err)
	}
	if res.Outcome != "ok" {
		t.Errorf("Outcome = %q, want ok (steps: %+v)", res.Outcome, res.PerStep)
	}
	if len(mutator.AddCalls) != 2 {
		t.Errorf("AddCalls = %d, want 2", len(mutator.AddCalls))
	}
	if len(mutator.PersistCalls) != 2 {
		t.Errorf("PersistCalls = %d, want 2", len(mutator.PersistCalls))
	}
}

func TestReconcile_NoChange_LiveMatchesDeclared(t *testing.T) {
	ps := twoPeerSet()
	path := savePeerSetForTest(t, ps, t.TempDir())
	matching := map[string]LivePeer{
		"PK_ALPHA": {Pubkey: "PK_ALPHA", AllowedIPs: "198.18.0.10/32,198.18.0.0/24"},
		"PK_BRAVO": {Pubkey: "PK_BRAVO", AllowedIPs: "198.18.0.11/32,198.18.0.0/24"},
	}
	live := &MockLiveStateReader{State: LiveState{
		PerRelay: map[string]*RelayLiveState{
			"relay-rla": {Reachable: true, Peers: matching},
			"relay-rlb": {Reachable: true, Peers: matching},
		},
	}}
	a, _ := NewApplier(ApplierOptions{PeerSetPath: path, Live: live})
	plan, err := a.Plan(context.Background(), ReconcileIntent{AllowAdd: true})
	if err != nil {
		t.Fatalf("Plan: %v", err)
	}
	if !plan.IsEmpty() {
		t.Errorf("Plan should be empty when live matches declared; got %+v", plan)
	}
}

func TestReconcile_RotationCase(t *testing.T) {
	ps := twoPeerSet()
	// Alpha has rotated; old key still on relay.
	ps.Peers[0].PubkeyHistory = []peerset.PubkeyHistoryEntry{
		{Pubkey: "PK_ALPHA_OLD", RotatedAt: time.Now(), Reason: "test rotation"},
	}
	path := savePeerSetForTest(t, ps, t.TempDir())
	live := &MockLiveStateReader{State: LiveState{
		PerRelay: map[string]*RelayLiveState{
			"relay-rla": {Reachable: true, Peers: map[string]LivePeer{
				"PK_ALPHA_OLD": {Pubkey: "PK_ALPHA_OLD", AllowedIPs: "198.18.0.10/32,198.18.0.0/24"},
				"PK_BRAVO":     {Pubkey: "PK_BRAVO", AllowedIPs: "198.18.0.11/32,198.18.0.0/24"},
			}},
			"relay-rlb": {Reachable: true, Peers: map[string]LivePeer{
				"PK_ALPHA_OLD": {Pubkey: "PK_ALPHA_OLD", AllowedIPs: "198.18.0.10/32,198.18.0.0/24"},
				"PK_BRAVO":     {Pubkey: "PK_BRAVO", AllowedIPs: "198.18.0.11/32,198.18.0.0/24"},
			}},
		},
	}}
	mutator := &MockRelayMutator{}
	a, _ := NewApplier(ApplierOptions{PeerSetPath: path, Live: live, Relay: mutator})
	plan, err := a.Plan(context.Background(), ReconcileIntent{AllowAdd: true})
	if err != nil {
		t.Fatalf("Plan: %v", err)
	}
	rotateCount := 0
	for _, rd := range plan.LivePlan.PerRelay {
		rotateCount += len(rd.WouldRotate)
	}
	if rotateCount != 2 {
		t.Errorf("WouldRotate = %d, want 2 (one per relay)", rotateCount)
	}

	_, err = a.Apply(context.Background(), plan)
	if err != nil {
		t.Fatalf("Apply: %v", err)
	}
	if len(mutator.RemoveCalls) != 2 {
		t.Errorf("RemoveCalls = %d, want 2", len(mutator.RemoveCalls))
	}
	if len(mutator.AddCalls) != 2 {
		t.Errorf("AddCalls = %d, want 2", len(mutator.AddCalls))
	}
}

func TestReconcile_UnknownPeer_NoRemoveByDefault(t *testing.T) {
	ps := twoPeerSet()
	path := savePeerSetForTest(t, ps, t.TempDir())
	live := &MockLiveStateReader{State: LiveState{
		PerRelay: map[string]*RelayLiveState{
			"relay-rla": {Reachable: true, Peers: map[string]LivePeer{
				"PK_ALPHA": {Pubkey: "PK_ALPHA", AllowedIPs: "198.18.0.10/32,198.18.0.0/24"},
				"PK_BRAVO": {Pubkey: "PK_BRAVO", AllowedIPs: "198.18.0.11/32,198.18.0.0/24"},
				"PK_ROGUE": {Pubkey: "PK_ROGUE", AllowedIPs: "198.18.0.99/32,198.18.0.0/24"},
			}},
			"relay-rlb": {Reachable: true, Peers: map[string]LivePeer{
				"PK_ALPHA": {Pubkey: "PK_ALPHA", AllowedIPs: "198.18.0.10/32,198.18.0.0/24"},
				"PK_BRAVO": {Pubkey: "PK_BRAVO", AllowedIPs: "198.18.0.11/32,198.18.0.0/24"},
			}},
		},
	}}
	a, _ := NewApplier(ApplierOptions{PeerSetPath: path, Live: live, Relay: &MockRelayMutator{}})

	// No --remove: rogue peer is NOT in the diff.
	plan, err := a.Plan(context.Background(), ReconcileIntent{AllowAdd: true, AllowRemove: false})
	if err != nil {
		t.Fatalf("Plan: %v", err)
	}
	for _, rd := range plan.LivePlan.PerRelay {
		if len(rd.WouldRemove) > 0 {
			t.Errorf("AllowRemove=false should produce no removals; got %+v", rd.WouldRemove)
		}
	}

	// With --remove: rogue peer is enumerated on relay-rla only.
	plan2, err := a.Plan(context.Background(), ReconcileIntent{AllowAdd: true, AllowRemove: true})
	if err != nil {
		t.Fatalf("Plan: %v", err)
	}
	removeCount := 0
	for _, rd := range plan2.LivePlan.PerRelay {
		removeCount += len(rd.WouldRemove)
	}
	if removeCount != 1 {
		t.Errorf("WouldRemove = %d, want 1 (relay-rla only)", removeCount)
	}
}

func TestReconcile_UnreachableRelay_NoMutations(t *testing.T) {
	ps := twoPeerSet()
	path := savePeerSetForTest(t, ps, t.TempDir())
	live := &MockLiveStateReader{State: LiveState{
		PerRelay: map[string]*RelayLiveState{
			"relay-rla": {Reachable: false, UnreachableErr: "dial timeout"},
			"relay-rlb": {Reachable: true, Peers: map[string]LivePeer{
				"PK_ALPHA": {Pubkey: "PK_ALPHA", AllowedIPs: "198.18.0.10/32,198.18.0.0/24"},
				"PK_BRAVO": {Pubkey: "PK_BRAVO", AllowedIPs: "198.18.0.11/32,198.18.0.0/24"},
			}},
		},
	}}
	a, _ := NewApplier(ApplierOptions{PeerSetPath: path, Live: live, Relay: &MockRelayMutator{}})
	plan, err := a.Plan(context.Background(), ReconcileIntent{AllowAdd: true})
	if err != nil {
		t.Fatalf("Plan: %v", err)
	}

	// relay-rla is unreachable; no Mutations should be queued for it.
	for _, m := range plan.Mutations {
		switch mm := m.(type) {
		case *RelayAddPeerMutation:
			if mm.Relay.Name == "relay-rla" {
				t.Errorf("unreachable relay-rla should not have AddPeer mutation")
			}
		case *RelayRotatePubkeyMutation:
			if mm.Relay.Name == "relay-rla" {
				t.Errorf("unreachable relay-rla should not have Rotate mutation")
			}
		case *RelayRemovePeerMutation:
			if mm.Relay.Name == "relay-rla" {
				t.Errorf("unreachable relay-rla should not have Remove mutation")
			}
		}
	}
	// The Unreachable row should still appear in the diff.
	foundUnreachable := false
	for _, rd := range plan.LivePlan.PerRelay {
		if rd.Relay == "relay-rla" && rd.Unreachable {
			foundUnreachable = true
		}
	}
	if !foundUnreachable {
		t.Error("relay-rla Unreachable=true row missing from LivePlan")
	}
}

func TestApply_AuditRecord_Emitted(t *testing.T) {
	ps := twoPeerSet()
	path := savePeerSetForTest(t, ps, t.TempDir())
	live := &MockLiveStateReader{State: LiveState{PerRelay: map[string]*RelayLiveState{
		"relay-rla": {Reachable: true, Peers: map[string]LivePeer{"PK_ALPHA": {Pubkey: "PK_ALPHA", AllowedIPs: "198.18.0.10/32,198.18.0.0/24"}}},
		"relay-rlb": {Reachable: true, Peers: map[string]LivePeer{"PK_ALPHA": {Pubkey: "PK_ALPHA", AllowedIPs: "198.18.0.10/32,198.18.0.0/24"}}},
	}}}
	mutator := &MockRelayMutator{}
	audit := &RecordingAuditEmitter{}
	a, _ := NewApplier(ApplierOptions{
		PeerSetPath: path,
		Live:        live,
		Relay:       mutator,
		Audit:       audit,
		Now:         fixedClock("2026-05-14T22:00:00Z"),
	})
	plan, err := a.Plan(context.Background(), ReconcileIntent{AllowAdd: true})
	if err != nil {
		t.Fatalf("Plan: %v", err)
	}
	if _, err := a.Apply(context.Background(), plan); err != nil {
		t.Fatalf("Apply: %v", err)
	}
	if len(audit.Records) != 1 {
		t.Fatalf("audit records = %d, want 1", len(audit.Records))
	}
	rec := audit.Records[0]
	if rec.IntentKind != "reconcile" {
		t.Errorf("IntentKind = %q, want reconcile", rec.IntentKind)
	}
	if rec.Outcome != "ok" {
		t.Errorf("Outcome = %q, want ok", rec.Outcome)
	}
	if !strings.HasPrefix(rec.CorrelationID, "wg-cp0:") {
		t.Errorf("CorrelationID = %q, want wg-cp0:* prefix", rec.CorrelationID)
	}
	if rec.PlanSummary.Adds != 1 || len(rec.PlanSummary.AffectedPeers) != 1 || rec.PlanSummary.AffectedPeers[0] != "bravo" {
		t.Errorf("PlanSummary unexpected: %+v", rec.PlanSummary)
	}
}

func TestApply_DryRun_Does_Not_Emit_Audit(t *testing.T) {
	// Spelled out: Apply emits; Plan does not. The cmd layer's
	// audit-routing avoids emission on the dry-run path, but the
	// Applier itself only emits inside Apply.
	ps := twoPeerSet()
	path := savePeerSetForTest(t, ps, t.TempDir())
	live := &MockLiveStateReader{State: LiveState{PerRelay: map[string]*RelayLiveState{
		"relay-rla": {Reachable: true},
		"relay-rlb": {Reachable: true},
	}}}
	audit := &RecordingAuditEmitter{}
	a, _ := NewApplier(ApplierOptions{
		PeerSetPath: path,
		Live:        live,
		Audit:       audit,
	})
	if _, err := a.Plan(context.Background(), ReconcileIntent{AllowAdd: true}); err != nil {
		t.Fatalf("Plan: %v", err)
	}
	if len(audit.Records) != 0 {
		t.Errorf("Plan should not emit audit; got %d records", len(audit.Records))
	}
}

func TestApply_PreFlightFailure_Aborts_Before_Wire(t *testing.T) {
	ps := twoPeerSet()
	path := savePeerSetForTest(t, ps, t.TempDir())
	live := &MockLiveStateReader{State: LiveState{PerRelay: map[string]*RelayLiveState{
		"relay-rla": {Reachable: true},
		"relay-rlb": {Reachable: true},
	}}}
	mutator := &MockRelayMutator{}
	audit := &RecordingAuditEmitter{}
	a, _ := NewApplier(ApplierOptions{
		PeerSetPath: path, Live: live, Relay: mutator, Audit: audit,
	})

	// Inject a Mutation with a guaranteed-fail PreFlight.
	plan, _ := a.Plan(context.Background(), ReconcileIntent{AllowAdd: true})
	plan.Mutations = append(plan.Mutations, &alwaysFailingPreFlight{})

	res, err := a.Apply(context.Background(), plan)
	if err == nil {
		t.Fatal("Apply should have returned an error")
	}
	if res.Outcome != "fail" {
		t.Errorf("Outcome = %q, want fail", res.Outcome)
	}
	if len(mutator.AddCalls) > 0 {
		t.Errorf("PreFlight failure must not run wire mutations; AddCalls = %d", len(mutator.AddCalls))
	}
	// Audit record IS emitted even on failure (the audit log is the
	// post-incident reconstruction source).
	if len(audit.Records) != 1 {
		t.Errorf("audit records = %d, want 1 (failure emits)", len(audit.Records))
	}
}

func TestApply_PartialFailure_OutcomeIs_Partial(t *testing.T) {
	ps := twoPeerSet()
	path := savePeerSetForTest(t, ps, t.TempDir())
	live := &MockLiveStateReader{State: LiveState{PerRelay: map[string]*RelayLiveState{
		"relay-rla": {Reachable: true, Peers: map[string]LivePeer{"PK_ALPHA": {Pubkey: "PK_ALPHA", AllowedIPs: "198.18.0.10/32,198.18.0.0/24"}}},
		"relay-rlb": {Reachable: true, Peers: map[string]LivePeer{"PK_ALPHA": {Pubkey: "PK_ALPHA", AllowedIPs: "198.18.0.10/32,198.18.0.0/24"}}},
	}}}
	mutator := &MockRelayMutator{AddErr: errors.New("simulated wire error")}
	audit := &RecordingAuditEmitter{}
	a, _ := NewApplier(ApplierOptions{
		PeerSetPath: path, Live: live, Relay: mutator, Audit: audit,
	})
	plan, _ := a.Plan(context.Background(), ReconcileIntent{AllowAdd: true})
	// Inject an always-OK mutation so okCount > 0; expected Outcome
	// = "partial". Append the OK one at the end.
	plan.Mutations = append(plan.Mutations, &alwaysOK{})
	res, _ := a.Apply(context.Background(), plan)
	if res.Outcome != "partial" {
		t.Errorf("Outcome = %q, want partial (steps: %+v)", res.Outcome, res.PerStep)
	}
}

// --- helpers ---

// intentWithKind is a minimal Intent for the registry test.
type intentWithKind string

// Kind implements Intent.
func (i intentWithKind) Kind() string { return string(i) }

// alwaysFailingPreFlight returns an error from PreFlight.
type alwaysFailingPreFlight struct{}

func (alwaysFailingPreFlight) Kind() string                      { return "test-fail-preflight" }
func (alwaysFailingPreFlight) Describe() string                  { return "test fail preflight" }
func (alwaysFailingPreFlight) PreFlight(_ context.Context) error { return errors.New("nope") }
func (alwaysFailingPreFlight) Apply(_ context.Context) error     { return nil }

// alwaysOK is a no-op Mutation that always succeeds.
type alwaysOK struct{}

func (alwaysOK) Kind() string                      { return "test-ok" }
func (alwaysOK) Describe() string                  { return "test ok" }
func (alwaysOK) PreFlight(_ context.Context) error { return nil }
func (alwaysOK) Apply(_ context.Context) error     { return nil }

// reconciler dummy reasoning — verify ApplyResult.PerStep ordering
// matches plan.Mutations order. (Property test.)
func TestApply_PerStepOrder_Matches_Plan(t *testing.T) {
	ps := twoPeerSet()
	path := savePeerSetForTest(t, ps, t.TempDir())
	a, _ := NewApplier(ApplierOptions{
		PeerSetPath: path,
		Live:        &MockLiveStateReader{State: LiveState{PerRelay: map[string]*RelayLiveState{"relay-rla": {Reachable: true}, "relay-rlb": {Reachable: true}}}},
		Relay:       &MockRelayMutator{},
	})
	plan, _ := a.Plan(context.Background(), ReconcileIntent{})
	plan.Mutations = []Mutation{
		&taggedMutation{name: "first"},
		&taggedMutation{name: "second"},
		&taggedMutation{name: "third"},
	}
	res, _ := a.Apply(context.Background(), plan)
	if len(res.PerStep) != 3 {
		t.Fatalf("PerStep = %d, want 3", len(res.PerStep))
	}
	for i, want := range []string{"first", "second", "third"} {
		if !strings.Contains(res.PerStep[i].Describe, want) {
			t.Errorf("PerStep[%d] = %+v, want describe containing %q", i, res.PerStep[i], want)
		}
	}
}

type taggedMutation struct{ name string }

func (m *taggedMutation) Kind() string                      { return "test-tagged" }
func (m *taggedMutation) Describe() string                  { return fmt.Sprintf("tagged:%s", m.name) }
func (m *taggedMutation) PreFlight(_ context.Context) error { return nil }
func (m *taggedMutation) Apply(_ context.Context) error     { return nil }
