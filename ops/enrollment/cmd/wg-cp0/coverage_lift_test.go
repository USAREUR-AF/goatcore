// coverage_lift_test.go — focused tests aimed at the lower-coverage
// command bodies + SSH dial-failure paths.
//
// The SSH-backed Read / execOne paths cannot exercise a real handshake
// from a unit test, but their *dial-failure* branches matter for
// production triage (operator's relay is unreachable; the orchestrator
// must surface an actionable error). These tests point the readers at
// closed TCP ports so the dial fails fast, validating the unreachable-
// to-error pipeline without requiring a real SSH server.

package main

import (
	"context"
	"encoding/json"
	"errors"
	"net"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/enrollment/peerset"
)

// closedPort returns a host:port string for a port that we briefly
// listened on and then closed. The underlying port is very likely to
// remain closed for the duration of the test (the kernel will not
// reassign immediately under TIME_WAIT for ephemeral ports), so a
// dial against it gets a fast ECONNREFUSED.
func closedPort(t *testing.T) string {
	t.Helper()
	l, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("listen: %v", err)
	}
	addr := l.Addr().String()
	_ = l.Close()
	return addr
}

// fakeAgentSocket spins up a tiny unix-socket "ssh-agent" that accepts
// the handshake long enough for agent.NewClient.Signers() to see an
// empty signer list and return the "no signers" error. We just open a
// listener that immediately closes the connection; the agent client
// surfaces this as "list agent signers" failure, which still exercises
// the loadAgentSigners non-empty-socket branch.
//
// Uses /tmp for a short path because macOS limits unix socket paths
// to 104 chars and t.TempDir() can exceed that.
func fakeAgentSocket(t *testing.T) string {
	t.Helper()
	f, err := os.CreateTemp("/tmp", "wgcp0a*.s")
	if err != nil {
		t.Fatalf("create tmp: %v", err)
	}
	sock := f.Name()
	_ = f.Close()
	_ = os.Remove(sock) // listen below recreates as a socket
	l, err := net.Listen("unix", sock)
	if err != nil {
		t.Fatalf("listen unix %s: %v", sock, err)
	}
	t.Cleanup(func() {
		_ = l.Close()
		_ = os.Remove(sock)
	})
	go func() {
		for {
			c, err := l.Accept()
			if err != nil {
				return
			}
			_ = c.Close()
		}
	}()
	return sock
}

func TestSSHLiveStateReader_ReadWithSigners_DialFails(t *testing.T) {
	// Drive readWithSigners directly with empty signers + a closed
	// TCP port. The goroutine fan-out runs; readOne dials the closed
	// port; state.UnreachableErr is populated; no panic.
	addr := closedPort(t)
	r := NewSSHLiveStateReader(SSHReaderOptions{
		Username:        "test",
		PerRelayTimeout: 250 * time.Millisecond,
		Concurrency:     2,
	})
	r.opts.EndpointFromRelay = func(_ peerset.Relay) string { return addr }

	relays := []peerset.Relay{
		{Name: "relay-a", MeshAddress: "1.2.3.4"},
		{Name: "relay-b", MeshAddress: "1.2.3.5"},
	}
	state := r.readWithSigners(context.Background(), relays, nil)
	if len(state.PerRelay) != 2 {
		t.Fatalf("PerRelay count = %d, want 2", len(state.PerRelay))
	}
	for name, row := range state.PerRelay {
		if row.Reachable {
			t.Errorf("relay %s should be unreachable", name)
		}
		if row.UnreachableErr == "" {
			t.Errorf("relay %s should have UnreachableErr populated", name)
		}
	}
	if state.FetchedAt.IsZero() {
		t.Error("FetchedAt should be populated")
	}
}

func TestSSHLiveStateReader_ReadOne_NoEndpoint(t *testing.T) {
	// Relay with no usable mesh address → readOne returns immediately
	// with UnreachableErr populated.
	r := NewSSHLiveStateReader(SSHReaderOptions{PerRelayTimeout: 100 * time.Millisecond})
	relay := peerset.Relay{Name: "relay-x"} // no MeshAddress
	state := r.readOne(context.Background(), relay, nil)
	if state.Reachable {
		t.Error("Reachable should be false")
	}
	if !strings.Contains(state.UnreachableErr, "no usable SSH endpoint") {
		t.Errorf("UnreachableErr = %q, want 'no usable SSH endpoint'", state.UnreachableErr)
	}
}

func TestSSHLiveStateReader_ReadOne_DialFail(t *testing.T) {
	// Closed TCP port → dial error captured into UnreachableErr.
	addr := closedPort(t)
	r := NewSSHLiveStateReader(SSHReaderOptions{PerRelayTimeout: 250 * time.Millisecond})
	r.opts.EndpointFromRelay = func(_ peerset.Relay) string { return addr }

	state := r.readOne(context.Background(), peerset.Relay{Name: "relay-x", MeshAddress: "1.2.3.4"}, nil)
	if state.Reachable {
		t.Errorf("Reachable should be false; got %+v", state)
	}
	if !strings.Contains(state.UnreachableErr, "dial") {
		t.Errorf("UnreachableErr = %q, want 'dial' substring", state.UnreachableErr)
	}
	// FetchDurationMs may be 0 when the dial fails on the same
	// millisecond it started; only assert it's non-negative.
	if state.FetchDurationMs < 0 {
		t.Errorf("FetchDurationMs = %d, want >= 0", state.FetchDurationMs)
	}
}

func TestSSHLiveStateReader_ReadWithSigners_CtxCanceled(t *testing.T) {
	// Canceled context → goroutine takes the ctx.Done branch on the
	// semaphore acquire and records the cancel error. Saturate the
	// semaphore before issuing the cancel to maximize the probability
	// of hitting the ctx-Done branch.
	r := NewSSHLiveStateReader(SSHReaderOptions{
		Concurrency:     1,
		PerRelayTimeout: 5 * time.Second,
	})
	// Endpoint that hangs forever — ensures the first slot stays
	// occupied until ctx fires.
	r.opts.EndpointFromRelay = func(_ peerset.Relay) string { return "240.0.0.1:1" }

	ctx, cancel := context.WithCancel(context.Background())
	relays := []peerset.Relay{
		{Name: "relay-a", MeshAddress: "1.1.1.1"},
		{Name: "relay-b", MeshAddress: "1.1.1.2"},
		{Name: "relay-c", MeshAddress: "1.1.1.3"},
	}
	cancel()
	state := r.readWithSigners(ctx, relays, nil)
	// At least one relay should be marked unreachable. The exact
	// per-relay outcome depends on goroutine scheduling, but the
	// shape — three rows, none Reachable — must hold.
	if len(state.PerRelay) != 3 {
		t.Fatalf("PerRelay count = %d, want 3", len(state.PerRelay))
	}
	for name, row := range state.PerRelay {
		if row.Reachable {
			t.Errorf("relay %s should be unreachable after cancel", name)
		}
	}
}

func TestSSHLiveStateReader_PerRelayDialFails_ReturnsUnreachable(t *testing.T) {
	addr := closedPort(t)
	sock := fakeAgentSocket(t)

	r := NewSSHLiveStateReader(SSHReaderOptions{
		Username:        "test",
		AgentSocket:     sock,
		PerRelayTimeout: 250 * time.Millisecond,
		Concurrency:     2,
	})
	relays := []peerset.Relay{
		{Name: "relay-x", MeshAddress: strings.Split(addr, ":")[0]},
	}
	// Override the endpoint resolver so we hit the closed port directly.
	r.opts.EndpointFromRelay = func(_ peerset.Relay) string { return addr }

	state, err := r.Read(context.Background(), relays)
	// loadAgentSigners may either succeed (depending on agent
	// implementation tolerance for an empty-handshake socket) or fail.
	// In either case the test is exercising the goroutine fan-out + the
	// readOne dial-failure path; no panic, deterministic shape.
	if err != nil && !strings.Contains(err.Error(), "ssh agent") {
		t.Fatalf("Read returned non-agent error: %v", err)
	}
	if err == nil {
		// Goroutine fan-out reached readOne; assert the relay row is
		// marked unreachable with a populated error.
		row, ok := state.PerRelay["relay-x"]
		if !ok {
			t.Fatal("relay-x row missing from PerRelay")
		}
		if row.Reachable {
			t.Errorf("row.Reachable should be false; got %+v", row)
		}
		if row.UnreachableErr == "" {
			t.Errorf("row.UnreachableErr should be populated; got empty")
		}
	}
}

func TestSSHRelayMutator_ExecOne_DialFail(t *testing.T) {
	// Bypass the agent-load path by supplying the agent socket via a
	// fakeAgentSocket. If that path errors first, the test still
	// exercises useful surface; if it succeeds, execOne progresses to
	// dial which fails on the closed port.
	addr := closedPort(t)
	sock := fakeAgentSocket(t)
	m := NewSSHRelayMutator(SSHReaderOptions{
		Username:        "test",
		AgentSocket:     sock,
		PerRelayTimeout: 250 * time.Millisecond,
	})
	m.opts.EndpointFromRelay = func(_ peerset.Relay) string { return addr }

	relay := peerset.Relay{Name: "relay-x", MeshAddress: "1.2.3.4"}
	err := m.PersistConfig(context.Background(), relay)
	if err == nil {
		t.Fatal("expected error from execOne")
	}
}

func TestSSHRelayMutator_RemovePeer_DialFail(t *testing.T) {
	addr := closedPort(t)
	sock := fakeAgentSocket(t)
	m := NewSSHRelayMutator(SSHReaderOptions{
		Username:        "test",
		AgentSocket:     sock,
		PerRelayTimeout: 250 * time.Millisecond,
	})
	m.opts.EndpointFromRelay = func(_ peerset.Relay) string { return addr }

	relay := peerset.Relay{Name: "relay-x", MeshAddress: "1.2.3.4"}
	err := m.RemovePeer(context.Background(), relay, "PK")
	if err == nil {
		t.Fatal("expected error from execOne")
	}
}

func TestSSHRelayMutator_DialFailure_ReturnsError(t *testing.T) {
	addr := closedPort(t)
	sock := fakeAgentSocket(t)

	m := NewSSHRelayMutator(SSHReaderOptions{
		Username:        "test",
		AgentSocket:     sock,
		PerRelayTimeout: 250 * time.Millisecond,
	})
	relay := peerset.Relay{Name: "relay-x"}
	m.opts.EndpointFromRelay = func(_ peerset.Relay) string { return addr }

	err := m.AddPeer(context.Background(), relay, "PK", "198.18.0.10/32", 25)
	if err == nil {
		t.Fatal("expected dial / handshake error")
	}
	// Path can fail at agent-load OR at dial (depending on the fake
	// agent's behavior). Both are acceptable as long as the call
	// surfaces *an* actionable error.
	msg := err.Error()
	if !strings.Contains(msg, "dial") &&
		!strings.Contains(msg, "ssh") &&
		!strings.Contains(msg, "agent") {
		t.Errorf("err = %v (expected dial/ssh/agent in message)", err)
	}
}

func TestRun_Validate_IncludeLiveState_OfflineMode(t *testing.T) {
	withOffline(t, func() {
		code, stdout, _ := runArgs(t, "validate", "--path", fixturePath, "--include-live-state")
		if code != 0 {
			t.Errorf("code = %d, want 0", code)
		}
		// Offline mode supplies an empty PerRelay snapshot; no drift to
		// report → cross-check PASS line.
		if !strings.Contains(stdout, "live-state cross-check") {
			t.Errorf("expected live-state cross-check line in output; got %q", stdout)
		}
	})
}

func TestRunLiveStateCrossCheck_DriftRows(t *testing.T) {
	// Hand-crafted PeerSet + LiveState that provoke each drift kind:
	//   - peer DECLARED but ABSENT on relay
	//   - peer ROTATED-AWAY pubkey still present on relay
	//   - UNKNOWN peer pubkey on relay
	ps := &peerset.PeerSet{
		Peers: []peerset.Peer{
			{
				Name: "alpha", CurrentPubkey: "PK_ALPHA_NEW",
				PubkeyHistory: []peerset.PubkeyHistoryEntry{{Pubkey: "PK_ALPHA_OLD"}},
			},
			{Name: "bravo", CurrentPubkey: "PK_BRAVO"},
		},
		Relays: []peerset.Relay{{Name: "relay-rla"}},
	}
	live := &MockLiveStateReader{State: LiveState{
		PerRelay: map[string]*RelayLiveState{
			"relay-rla": {Reachable: true, Peers: map[string]LivePeer{
				// bravo's CurrentPubkey is here — no drift on bravo.
				"PK_BRAVO": {Pubkey: "PK_BRAVO"},
				// alpha's old pubkey is still on the relay; current is missing.
				"PK_ALPHA_OLD": {Pubkey: "PK_ALPHA_OLD"},
				// truly unknown pubkey.
				"PK_GHOST": {Pubkey: "PK_GHOST"},
			}},
		},
	}}
	rows := runLiveStateCrossCheck(ps, live, &strings.Builder{})
	if len(rows) != 3 {
		t.Fatalf("rows = %d, want 3 (1 absent + 1 rotated-away + 1 unknown); got %v", len(rows), rows)
	}
	joined := strings.Join(rows, "\n")
	if !strings.Contains(joined, "DECLARED but ABSENT") {
		t.Errorf("missing DECLARED-but-ABSENT row")
	}
	if !strings.Contains(joined, "ROTATED-AWAY") {
		t.Errorf("missing ROTATED-AWAY row")
	}
	if !strings.Contains(joined, "UNKNOWN peer") {
		t.Errorf("missing UNKNOWN row")
	}
}

func TestRunLiveStateCrossCheck_NilReader_ReturnsNil(t *testing.T) {
	rows := runLiveStateCrossCheck(&peerset.PeerSet{}, nil, &strings.Builder{})
	if rows != nil {
		t.Errorf("nil reader should yield nil rows; got %+v", rows)
	}
}

func TestRunLiveStateCrossCheck_ReadError_NonFatal(t *testing.T) {
	live := &MockLiveStateReader{Err: errors.New("simulated read error")}
	var stderr strings.Builder
	rows := runLiveStateCrossCheck(&peerset.PeerSet{}, live, &stderr)
	if rows != nil {
		t.Errorf("read error should yield nil rows; got %+v", rows)
	}
	if !strings.Contains(stderr.String(), "non-fatal") {
		t.Errorf("expected non-fatal warning in stderr; got %q", stderr.String())
	}
}

func TestRun_Diff_JSON_Mode(t *testing.T) {
	withOffline(t, func() {
		code, stdout, _ := runArgs(t, "diff", "--path", fixturePath, "--json", "--no-color")
		if code != 0 {
			t.Errorf("code = %d, want 0 (no relays in fixture → empty plan)", code)
		}
		var got map[string]any
		if err := json.Unmarshal([]byte(stdout), &got); err != nil {
			t.Fatalf("json parse: %v\n%s", err, stdout)
		}
		if got["intent_kind"] != "reconcile" {
			t.Errorf("intent_kind = %v", got["intent_kind"])
		}
	})
}

func TestRun_Diff_SiteFilter(t *testing.T) {
	withOffline(t, func() {
		code, _, _ := runArgs(t, "diff", "--path", fixturePath, "--site", "lab")
		if code != 0 {
			t.Errorf("code = %d, want 0", code)
		}
	})
}

func TestRun_Diff_RemoveFlag_Surfaces(t *testing.T) {
	withOffline(t, func() {
		code, _, _ := runArgs(t, "diff", "--path", fixturePath, "--remove")
		if code != 0 {
			t.Errorf("code = %d, want 0", code)
		}
	})
}

func TestRun_Diff_NoColor(t *testing.T) {
	withOffline(t, func() {
		code, _, _ := runArgs(t, "diff", "--path", fixturePath, "--no-color")
		if code != 0 {
			t.Errorf("code = %d, want 0", code)
		}
	})
}

func TestRun_Diff_BadFlag(t *testing.T) {
	code, _, stderr := runArgs(t, "diff", "--bogus", "x")
	if code != 2 {
		t.Errorf("code = %d, want 2", code)
	}
	if !strings.Contains(stderr, "unknown flag") {
		t.Errorf("stderr = %q", stderr)
	}
}

func TestRun_Diff_UnexpectedPositional(t *testing.T) {
	code, _, stderr := runArgs(t, "diff", "junk")
	if code != 2 {
		t.Errorf("code = %d, want 2", code)
	}
	if !strings.Contains(stderr, "positional") {
		t.Errorf("stderr = %q", stderr)
	}
}

func TestBuildIOLayer_OfflineReturnsMock(t *testing.T) {
	withOffline(t, func() {
		live, mut := buildIOLayer(false)
		if live == nil {
			t.Fatal("offline mode should still return a live reader (the mock)")
		}
		if mut != nil {
			t.Error("offline mode should NOT return a mutator")
		}
		// Read should produce an empty PerRelay map without error.
		state, err := live.Read(context.Background(), nil)
		if err != nil {
			t.Errorf("mock Read: %v", err)
		}
		if state.PerRelay == nil {
			t.Error("mock Read should populate PerRelay map")
		}
	})
}

func TestPickAuditEmitter_HonorsEnvOverride(t *testing.T) {
	dir := t.TempDir()
	custom := filepath.Join(dir, "custom-audit.jsonl")
	t.Setenv("WG_CP0_AUDIT_PATH", custom)

	em := pickAuditEmitter()
	fa, ok := em.(*FileAuditEmitter)
	if !ok {
		t.Fatalf("pickAuditEmitter returned %T, want *FileAuditEmitter", em)
	}
	if fa.Path != custom {
		t.Errorf("Path = %q, want %q", fa.Path, custom)
	}
}

func TestResolveOperator_WithUserAndEmail(t *testing.T) {
	t.Setenv("USER", "test-user")

	got := ResolveOperator()
	if got == "" {
		t.Error("ResolveOperator returned empty")
	}
	if !strings.HasPrefix(got, "test-user") {
		t.Errorf("got %q, want prefix test-user", got)
	}
}

func TestResolveOperator_NoUserOrUsername(t *testing.T) {
	oldUser := os.Getenv("USER")
	oldUsername := os.Getenv("USERNAME")
	_ = os.Unsetenv("USER")
	_ = os.Unsetenv("USERNAME")
	defer func() {
		_ = os.Setenv("USER", oldUser)
		_ = os.Setenv("USERNAME", oldUsername)
	}()

	got := ResolveOperator()
	// Must be non-empty in all environments per the function contract.
	if got == "" {
		t.Error("ResolveOperator returned empty")
	}
}

func TestApplier_Apply_NilPlan_Errors(t *testing.T) {
	ps := twoPeerSet()
	path := savePeerSetForTest(t, ps, t.TempDir())
	a, _ := NewApplier(ApplierOptions{PeerSetPath: path})

	_, err := a.Apply(context.Background(), nil)
	if err == nil || !strings.Contains(err.Error(), "nil plan") {
		t.Errorf("expected nil-plan error; got %v", err)
	}
}

func TestApplier_Apply_PlanWithoutIntent_Errors(t *testing.T) {
	ps := twoPeerSet()
	path := savePeerSetForTest(t, ps, t.TempDir())
	a, _ := NewApplier(ApplierOptions{PeerSetPath: path})

	_, err := a.Apply(context.Background(), &ApplyPlan{StatePlan: &PeerSetDiff{}, LivePlan: &LiveStateDiff{}})
	if err == nil || !strings.Contains(err.Error(), "no Intent") {
		t.Errorf("expected no-Intent error; got %v", err)
	}
}

func TestApplier_Apply_AuditEmitFailure_StillReturnsResult(t *testing.T) {
	ps := twoPeerSet()
	path := savePeerSetForTest(t, ps, t.TempDir())
	live := &MockLiveStateReader{State: LiveState{PerRelay: map[string]*RelayLiveState{
		"relay-rla": {Reachable: true},
		"relay-rlb": {Reachable: true},
	}}}
	audit := &RecordingAuditEmitter{Err: errors.New("audit-emit failed")}
	a, _ := NewApplier(ApplierOptions{
		PeerSetPath: path, Live: live, Audit: audit,
	})
	plan, _ := a.Plan(context.Background(), ReconcileIntent{AllowAdd: true})
	res, err := a.Apply(context.Background(), plan)
	// Audit-emit failure must NOT fail the apply (audit is observability).
	if err != nil {
		t.Errorf("apply should not fail on audit-emit error; got %v", err)
	}
	if res == nil || res.Outcome != "ok" {
		t.Errorf("res = %+v, want Outcome ok", res)
	}
}

func TestApplier_Apply_NilAudit_Skips(t *testing.T) {
	ps := twoPeerSet()
	path := savePeerSetForTest(t, ps, t.TempDir())
	live := &MockLiveStateReader{State: LiveState{PerRelay: map[string]*RelayLiveState{
		"relay-rla": {Reachable: true},
		"relay-rlb": {Reachable: true},
	}}}
	a, _ := NewApplier(ApplierOptions{PeerSetPath: path, Live: live}) // no Audit

	plan, _ := a.Plan(context.Background(), ReconcileIntent{AllowAdd: true})
	res, err := a.Apply(context.Background(), plan)
	if err != nil {
		t.Errorf("apply should succeed with nil audit; got %v", err)
	}
	if res.Outcome != "ok" {
		t.Errorf("Outcome = %q, want ok", res.Outcome)
	}
}

func TestPeerSetDiff_IsEmpty_AllVariants(t *testing.T) {
	if !((*PeerSetDiff)(nil)).IsEmpty() {
		t.Error("nil PeerSetDiff should be empty")
	}
	d := &PeerSetDiff{}
	if !d.IsEmpty() {
		t.Error("zero-value PeerSetDiff should be empty")
	}
	d.Added = []PeerDelta{{Name: "x"}}
	if d.IsEmpty() {
		t.Error("non-empty Added should be non-empty")
	}
	d = &PeerSetDiff{TTLUpdated: []PeerDelta{{Name: "y"}}}
	if d.IsEmpty() {
		t.Error("non-empty TTLUpdated should be non-empty")
	}
	d = &PeerSetDiff{RelaysAdded: []RelayDelta{{Name: "r"}}}
	if d.IsEmpty() {
		t.Error("non-empty RelaysAdded should be non-empty")
	}
}

func TestLiveStateDiff_IsEmpty_AllVariants(t *testing.T) {
	if !((*LiveStateDiff)(nil)).IsEmpty() {
		t.Error("nil LiveStateDiff should be empty")
	}
	d := &LiveStateDiff{}
	if !d.IsEmpty() {
		t.Error("zero-value LiveStateDiff should be empty")
	}
	d.PerRelay = []RelayLiveDiff{{Relay: "r"}}
	if !d.IsEmpty() {
		t.Error("LiveStateDiff with empty per-relay row should still be empty")
	}
	d.PerRelay = []RelayLiveDiff{{Relay: "r", WouldAdd: []LivePeerOp{{PeerName: "x"}}}}
	if d.IsEmpty() {
		t.Error("LiveStateDiff with WouldAdd entry should be non-empty")
	}
}

func TestApplyPlan_IsEmpty_NilSafe(t *testing.T) {
	if !((*ApplyPlan)(nil)).IsEmpty() {
		t.Error("nil plan should be empty")
	}
	p := &ApplyPlan{}
	if !p.IsEmpty() {
		t.Error("zero-value plan should be empty")
	}
	p.StatePlan = &PeerSetDiff{Added: []PeerDelta{{Name: "x"}}}
	if p.IsEmpty() {
		t.Error("plan with non-empty StatePlan should not be empty")
	}
}

func TestRun_Validate_BadFlag(t *testing.T) {
	code, _, stderr := runArgs(t, "validate", "--bogus", "x")
	if code != 2 {
		t.Errorf("code = %d, want 2", code)
	}
	if !strings.Contains(stderr, "unknown flag") {
		t.Errorf("stderr = %q", stderr)
	}
}

func TestRun_Validate_UnexpectedPositional(t *testing.T) {
	code, _, stderr := runArgs(t, "validate", "junk")
	if code != 2 {
		t.Errorf("code = %d, want 2", code)
	}
	if !strings.Contains(stderr, "positional") {
		t.Errorf("stderr = %q", stderr)
	}
}

func TestRun_Reconcile_EmptyPath_NewApplierFails(t *testing.T) {
	withOffline(t, func() {
		// Force the empty PeerSetPath path through cmdReconcile.
		// peerset.Load on "" returns an error, but cmdReconcile's
		// dispatch first builds NewApplier — which itself errors on
		// empty path. Pass --path '' to take that branch.
		code, _, stderr := runArgs(t, "reconcile", "--path", "")
		if code != 1 {
			t.Errorf("code = %d, want 1", code)
		}
		if !strings.Contains(stderr, "PeerSetPath required") &&
			!strings.Contains(stderr, "load peerset") {
			t.Errorf("stderr = %q (expected NewApplier or peerset.Load failure)", stderr)
		}
	})
}

func TestRun_Diff_EmptyPath_Errors(t *testing.T) {
	withOffline(t, func() {
		code, _, stderr := runArgs(t, "diff", "--path", "")
		if code != 1 {
			t.Errorf("code = %d, want 1", code)
		}
		if !strings.Contains(stderr, "PeerSetPath required") &&
			!strings.Contains(stderr, "load peerset") {
			t.Errorf("stderr = %q", stderr)
		}
	})
}

func TestRun_Reconcile_InvalidJSON_PlanFails(t *testing.T) {
	dir := t.TempDir()
	bad := filepath.Join(dir, "bad.json")
	if err := os.WriteFile(bad, []byte("{not json"), 0o644); err != nil {
		t.Fatal(err)
	}
	withOffline(t, func() {
		code, _, stderr := runArgs(t, "reconcile", "--path", bad)
		if code != 1 {
			t.Errorf("code = %d, want 1", code)
		}
		if !strings.Contains(stderr, "plan") && !strings.Contains(stderr, "peerset") {
			t.Errorf("stderr = %q", stderr)
		}
	})
}

func TestFilterPeers_NoFilter_ReturnsInput(t *testing.T) {
	in := []peerset.Peer{{Name: "alpha"}, {Name: "bravo"}}
	got := filterPeers(in, "", "")
	if len(got) != 2 {
		t.Errorf("len = %d, want 2", len(got))
	}
}

func TestFilterPeers_ExcludesByRole(t *testing.T) {
	in := []peerset.Peer{
		{Name: "alpha", Role: "operator"},
		{Name: "bravo", Role: "device"},
	}
	got := filterPeers(in, "", "operator")
	if len(got) != 1 || got[0].Name != "alpha" {
		t.Errorf("got %+v", got)
	}
}

func TestSummarizeApply_Outcomes(t *testing.T) {
	r := &ApplyResult{Outcome: "ok", PerStep: []StepOutcome{{Outcome: "ok"}}}
	out := summarizeApply(r)
	if !strings.Contains(out, "ok") {
		t.Errorf("out = %q", out)
	}
}

// TestApplier_Apply_AllStepsFail covers the okCount==0 path which sets
// Outcome to "fail" and Err to "every step failed".
func TestApplier_Apply_AllStepsFail(t *testing.T) {
	ps := twoPeerSet()
	path := savePeerSetForTest(t, ps, t.TempDir())
	a, _ := NewApplier(ApplierOptions{PeerSetPath: path})
	plan, _ := a.Plan(context.Background(), ReconcileIntent{})
	plan.Mutations = []Mutation{
		&alwaysFailingApply{},
		&alwaysFailingApply{},
	}
	res, err := a.Apply(context.Background(), plan)
	if err == nil || res.Outcome != "fail" {
		t.Errorf("Outcome = %q, err = %v; want fail/non-nil", res.Outcome, err)
	}
}

// alwaysFailingApply succeeds at PreFlight but fails Apply.
type alwaysFailingApply struct{}

func (alwaysFailingApply) Kind() string                      { return "test-fail-apply" }
func (alwaysFailingApply) Describe() string                  { return "test fail apply" }
func (alwaysFailingApply) PreFlight(_ context.Context) error { return nil }
func (alwaysFailingApply) Apply(_ context.Context) error     { return errors.New("simulated wire failure") }
