package main

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/enrollment/peerset"
)

// seedPeerset writes a v2 envelope under tmpDir for the lifecycle
// subcommand tests. Returns the path. Active peers come from the
// active slice; deprecated peers from the deprecated slice.
func seedPeerset(t *testing.T, active []peerset.Peer, deprecated []peerset.DeprecatedPeer) string {
	t.Helper()
	dir := t.TempDir()
	path := filepath.Join(dir, "peerset.json")
	ps := &peerset.PeerSet{
		SchemaVersion: peerset.SchemaVersion,
		Peers:         active,
		Relays: []peerset.Relay{
			{Name: "relay-rla", Site: "rla", Endpoint: "1.2.3.4:51821", MeshAddress: "198.18.0.2"},
			{Name: "relay-rlb", Site: "rlb", Endpoint: "5.6.7.8:51821", MeshAddress: "198.18.0.3"},
			{Name: "relay-rlc", Site: "rlc", Endpoint: "9.10.11.12:51821", MeshAddress: "198.18.0.4"},
		},
		DeprecatedPeers: deprecated,
	}
	if err := ps.Save(path); err != nil {
		t.Fatalf("seed peerset: %v", err)
	}
	return path
}

// activePeer is the canonical "old peer" the tests rotate / remove
// against.
func activePeer() peerset.Peer {
	return peerset.Peer{
		Name:          "lab-a",
		WGCP0Address:  "198.18.0.1",
		CurrentPubkey: "kwtaja-old-pubkey-base64-22-chars-long==",
		Role:          "mgmt-host",
		Site:          "lab",
	}
}

// --- onboard subcommand --------------------------------------------------

func TestRun_Onboard_DryRunShowsAddPlan(t *testing.T) {
	path := seedPeerset(t, []peerset.Peer{activePeer()}, nil)
	code, stdout, stderr := runArgs(t, "onboard", "new-orin",
		"--site", "lab", "--role", "mgmt-host",
		"--wg-cp0-address", "198.18.0.99/24",
		"--path", path)
	if code != 0 {
		t.Fatalf("code = %d (stderr = %q)", code, stderr)
	}
	if !strings.Contains(stdout, "ADD peers[]: new-orin (198.18.0.99)") {
		t.Errorf("dry-run output missing ADD line: %q", stdout)
	}
	if !strings.Contains(stdout, "Re-run with --apply") {
		t.Errorf("dry-run output missing --apply hint: %q", stdout)
	}
	// The peerset file must be unchanged after dry-run.
	ps, _ := peerset.Load(path)
	if _, ok := ps.Lookup("new-orin"); ok {
		t.Error("dry-run should NOT have mutated the peerset")
	}
}

func TestRun_Onboard_RejectsDuplicateName(t *testing.T) {
	path := seedPeerset(t, []peerset.Peer{activePeer()}, nil)
	code, _, stderr := runArgs(t, "onboard", "lab-a",
		"--site", "lab", "--role", "mgmt-host",
		"--wg-cp0-address", "198.18.0.50/24",
		"--path", path)
	if code != 1 {
		t.Errorf("code = %d, want 1", code)
	}
	if !strings.Contains(stderr, "already in active set") {
		t.Errorf("stderr should mention duplicate name; got %q", stderr)
	}
}

func TestRun_Onboard_RejectsDuplicateAddress(t *testing.T) {
	path := seedPeerset(t, []peerset.Peer{activePeer()}, nil)
	code, _, stderr := runArgs(t, "onboard", "different-name",
		"--site", "lab", "--role", "mgmt-host",
		"--wg-cp0-address", "198.18.0.1/24",
		"--path", path)
	if code != 1 {
		t.Errorf("code = %d, want 1", code)
	}
	if !strings.Contains(stderr, "already held by active peer") {
		t.Errorf("stderr should mention address collision; got %q", stderr)
	}
}

func TestRun_Onboard_RejectsResurrectionViaOnboard(t *testing.T) {
	path := seedPeerset(t, nil, []peerset.DeprecatedPeer{
		{
			Peer:          activePeer(),
			RemovedAt:     time.Now().UTC(),
			RemovedReason: "previously decommissioned",
		},
	})
	code, _, stderr := runArgs(t, "onboard", "lab-a",
		"--site", "lab", "--role", "mgmt-host",
		"--wg-cp0-address", "198.18.0.1/24",
		"--path", path)
	if code != 1 {
		t.Errorf("code = %d, want 1", code)
	}
	if !strings.Contains(stderr, "deprecated_peers[]") {
		t.Errorf("stderr should point at resurrect; got %q", stderr)
	}
}

func TestRun_Onboard_RejectsMissingFlags(t *testing.T) {
	path := seedPeerset(t, nil, nil)
	for _, tc := range []struct {
		name    string
		args    []string
		wantMsg string
	}{
		{"no positional", []string{"onboard"}, "positional arg"},
		{"no site", []string{"onboard", "p", "--role", "r", "--wg-cp0-address", "1.2.3.4/24"}, "--site"},
		{"no role", []string{"onboard", "p", "--site", "s", "--wg-cp0-address", "1.2.3.4/24"}, "--role"},
		{"no address", []string{"onboard", "p", "--site", "s", "--role", "r"}, "--wg-cp0-address"},
		{"non-CIDR", []string{"onboard", "p", "--site", "s", "--role", "r", "--wg-cp0-address", "1.2.3.4"}, "CIDR"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			args := append([]string{}, tc.args...)
			args = append(args, "--path", path)
			code, _, stderr := runArgs(t, args...)
			if code != 2 {
				t.Errorf("code = %d, want 2", code)
			}
			if !strings.Contains(stderr, tc.wantMsg) {
				t.Errorf("stderr should contain %q; got %q", tc.wantMsg, stderr)
			}
		})
	}
}

func TestRun_Onboard_ApplyRequiresBundleOut(t *testing.T) {
	path := seedPeerset(t, nil, nil)
	code, _, stderr := runArgs(t, "onboard", "new-orin",
		"--site", "lab", "--role", "mgmt-host",
		"--wg-cp0-address", "198.18.0.99/24",
		"--apply",
		"--path", path)
	if code != 2 {
		t.Errorf("code = %d, want 2", code)
	}
	if !strings.Contains(stderr, "--bundle-out") {
		t.Errorf("stderr should mention --bundle-out; got %q", stderr)
	}
}

func TestRun_Onboard_ApplyReturnsSessionAPending(t *testing.T) {
	path := seedPeerset(t, nil, nil)
	bundleOut := filepath.Join(t.TempDir(), "bundle.cbor")
	code, _, stderr := runArgs(t, "onboard", "new-orin",
		"--site", "lab", "--role", "mgmt-host",
		"--wg-cp0-address", "198.18.0.99/24",
		"--bundle-out", bundleOut,
		"--apply",
		"--path", path)
	if code != 1 {
		t.Errorf("code = %d, want 1", code)
	}
	if !strings.Contains(stderr, "Session A") || !strings.Contains(stderr, "Applier") {
		t.Errorf("stderr should reference Session A's Applier; got %q", stderr)
	}
	if !strings.Contains(stderr, "Legacy path") {
		t.Errorf("stderr should surface legacy-path command; got %q", stderr)
	}
}

// --- rotate subcommand ---------------------------------------------------

func TestRun_Rotate_DryRunShowsRotatePlan(t *testing.T) {
	path := seedPeerset(t, []peerset.Peer{activePeer()}, nil)
	code, stdout, stderr := runArgs(t, "rotate", "lab-a",
		"--reason", "scheduled 90d rotation",
		"--path", path)
	if code != 0 {
		t.Fatalf("code = %d (stderr = %q)", code, stderr)
	}
	if !strings.Contains(stdout, "ROTATE peer lab-a") {
		t.Errorf("dry-run output missing ROTATE line: %q", stdout)
	}
	if !strings.Contains(stdout, "scheduled 90d rotation") {
		t.Errorf("dry-run output missing reason: %q", stdout)
	}
	if !strings.Contains(stdout, "pubkey_history") {
		t.Errorf("dry-run output missing history note: %q", stdout)
	}
}

func TestRun_Rotate_RejectsMissingReason(t *testing.T) {
	path := seedPeerset(t, []peerset.Peer{activePeer()}, nil)
	code, _, stderr := runArgs(t, "rotate", "lab-a", "--path", path)
	if code != 2 {
		t.Errorf("code = %d, want 2", code)
	}
	if !strings.Contains(stderr, "--reason") {
		t.Errorf("stderr should mention --reason; got %q", stderr)
	}
}

func TestRun_Rotate_RejectsUnknownPeer(t *testing.T) {
	path := seedPeerset(t, []peerset.Peer{activePeer()}, nil)
	code, _, stderr := runArgs(t, "rotate", "nonexistent",
		"--reason", "test",
		"--path", path)
	if code != 1 {
		t.Errorf("code = %d, want 1", code)
	}
	if !strings.Contains(stderr, "not in active set") {
		t.Errorf("stderr should mention unknown peer; got %q", stderr)
	}
}

func TestRun_Rotate_DeprecatedPeerSuggestsResurrect(t *testing.T) {
	path := seedPeerset(t, nil, []peerset.DeprecatedPeer{
		{Peer: activePeer(), RemovedAt: time.Now().UTC(), RemovedReason: "old"},
	})
	code, _, stderr := runArgs(t, "rotate", "lab-a",
		"--reason", "test",
		"--path", path)
	if code != 1 {
		t.Errorf("code = %d, want 1", code)
	}
	if !strings.Contains(stderr, "resurrect") {
		t.Errorf("stderr should suggest resurrect; got %q", stderr)
	}
}

func TestRun_Rotate_ApplyReturnsSessionAPending(t *testing.T) {
	path := seedPeerset(t, []peerset.Peer{activePeer()}, nil)
	code, _, stderr := runArgs(t, "rotate", "lab-a",
		"--reason", "test",
		"--bundle-out", filepath.Join(t.TempDir(), "b.cbor"),
		"--apply",
		"--path", path)
	if code != 1 {
		t.Errorf("code = %d, want 1", code)
	}
	if !strings.Contains(stderr, "Session A") {
		t.Errorf("stderr should reference Session A; got %q", stderr)
	}
}

// --- remove subcommand ---------------------------------------------------

func TestRun_Remove_DryRunShowsRemoveProminently(t *testing.T) {
	path := seedPeerset(t, []peerset.Peer{activePeer()}, nil)
	code, stdout, stderr := runArgs(t, "remove", "lab-a",
		"--reason", "host decommissioned",
		"--path", path)
	if code != 0 {
		t.Fatalf("code = %d (stderr = %q)", code, stderr)
	}
	if !strings.Contains(stdout, "REMOVE peers[]: lab-a") {
		t.Errorf("dry-run output missing REMOVE line: %q", stdout)
	}
	if !strings.Contains(stdout, "SOFT-DELETE") {
		t.Errorf("dry-run output missing soft-delete clarification: %q", stdout)
	}
}

func TestRun_Remove_RejectsMissingReason(t *testing.T) {
	path := seedPeerset(t, []peerset.Peer{activePeer()}, nil)
	code, _, stderr := runArgs(t, "remove", "lab-a", "--path", path)
	if code != 2 {
		t.Errorf("code = %d, want 2", code)
	}
	if !strings.Contains(stderr, "--reason") {
		t.Errorf("stderr should mention --reason; got %q", stderr)
	}
}

func TestRun_Remove_AlreadyDeprecated(t *testing.T) {
	path := seedPeerset(t, nil, []peerset.DeprecatedPeer{
		{Peer: activePeer(), RemovedAt: time.Now().UTC(), RemovedReason: "old"},
	})
	code, _, stderr := runArgs(t, "remove", "lab-a",
		"--reason", "test",
		"--path", path)
	if code != 1 {
		t.Errorf("code = %d, want 1", code)
	}
	if !strings.Contains(stderr, "already in deprecated_peers") {
		t.Errorf("stderr should mention already-deprecated state; got %q", stderr)
	}
}

func TestRun_Remove_UnknownPeer(t *testing.T) {
	path := seedPeerset(t, []peerset.Peer{activePeer()}, nil)
	code, _, stderr := runArgs(t, "remove", "ghost",
		"--reason", "test",
		"--path", path)
	if code != 1 {
		t.Errorf("code = %d, want 1", code)
	}
	if !strings.Contains(stderr, "not in active set") {
		t.Errorf("stderr should mention unknown peer; got %q", stderr)
	}
}

func TestRun_Remove_ApplyReturnsSessionAPending(t *testing.T) {
	path := seedPeerset(t, []peerset.Peer{activePeer()}, nil)
	code, _, stderr := runArgs(t, "remove", "lab-a",
		"--reason", "test",
		"--apply",
		"--path", path)
	if code != 1 {
		t.Errorf("code = %d, want 1", code)
	}
	if !strings.Contains(stderr, "Session A") {
		t.Errorf("stderr should reference Session A; got %q", stderr)
	}
}

// --- resurrect subcommand ------------------------------------------------

func TestRun_Resurrect_DryRunShowsRestorePlan(t *testing.T) {
	path := seedPeerset(t, nil, []peerset.DeprecatedPeer{
		{
			Peer:          activePeer(),
			RemovedAt:     time.Now().UTC().Add(-24 * time.Hour),
			RemovedReason: "accidental removal",
		},
	})
	code, stdout, stderr := runArgs(t, "resurrect", "lab-a", "--path", path)
	if code != 0 {
		t.Fatalf("code = %d (stderr = %q)", code, stderr)
	}
	if !strings.Contains(stdout, "RESURRECT peers[]: lab-a") {
		t.Errorf("dry-run output missing RESURRECT line: %q", stdout)
	}
	if !strings.Contains(stdout, "accidental removal") {
		t.Errorf("dry-run output missing removal-reason context: %q", stdout)
	}
}

func TestRun_Resurrect_RejectsAlreadyActive(t *testing.T) {
	path := seedPeerset(t, []peerset.Peer{activePeer()}, nil)
	code, _, stderr := runArgs(t, "resurrect", "lab-a", "--path", path)
	if code != 1 {
		t.Errorf("code = %d, want 1", code)
	}
	if !strings.Contains(stderr, "already in active set") {
		t.Errorf("stderr should mention already-active state; got %q", stderr)
	}
}

func TestRun_Resurrect_RejectsCompletelyUnknown(t *testing.T) {
	path := seedPeerset(t, nil, nil)
	code, _, stderr := runArgs(t, "resurrect", "ghost", "--path", path)
	if code != 1 {
		t.Errorf("code = %d, want 1", code)
	}
	if !strings.Contains(stderr, "not in deprecated_peers") {
		t.Errorf("stderr should mention not-in-deprecated; got %q", stderr)
	}
}

func TestRun_Resurrect_RejectsNameCollisionWithActive(t *testing.T) {
	// Active peer with the same name as the deprecated row.
	// (peerset.Save normalizes the dep peer's Peer.Name, so this
	// shape is achievable by hand-editing the JSON; we synthesize
	// it directly to exercise the preview-time collision check.)
	dir := t.TempDir()
	path := filepath.Join(dir, "peerset.json")
	ps := &peerset.PeerSet{
		SchemaVersion: peerset.SchemaVersion,
		Peers: []peerset.Peer{
			{Name: "lab-a", WGCP0Address: "198.18.0.50", CurrentPubkey: "new-key=="},
		},
		DeprecatedPeers: []peerset.DeprecatedPeer{
			{Peer: activePeer(), RemovedAt: time.Now().UTC(), RemovedReason: "old"},
		},
	}
	if err := ps.Save(path); err != nil {
		t.Fatalf("save: %v", err)
	}
	code, _, stderr := runArgs(t, "resurrect", "lab-a", "--path", path)
	if code != 1 {
		t.Errorf("code = %d, want 1", code)
	}
	if !strings.Contains(stderr, "already exists") {
		t.Errorf("stderr should mention collision; got %q", stderr)
	}
}

func TestRun_Resurrect_ApplyReturnsSessionAPending(t *testing.T) {
	path := seedPeerset(t, nil, []peerset.DeprecatedPeer{
		{Peer: activePeer(), RemovedAt: time.Now().UTC(), RemovedReason: "test"},
	})
	code, _, stderr := runArgs(t, "resurrect", "lab-a", "--apply", "--path", path)
	if code != 1 {
		t.Errorf("code = %d, want 1", code)
	}
	if !strings.Contains(stderr, "Session A") {
		t.Errorf("stderr should reference Session A; got %q", stderr)
	}
}

// --- shared helpers (parseLifecycleFlags / preview emitter) --------------

func TestParseLifecycleFlags_UnknownFlag(t *testing.T) {
	_, _, err := parseLifecycleFlags([]string{"--frobnicate", "x"}, nil)
	if err == nil {
		t.Fatal("expected unknown-flag error")
	}
	if !strings.Contains(err.Error(), "unknown flag") {
		t.Errorf("err = %q", err)
	}
}

func TestParseLifecycleFlags_FlagRequiresValue(t *testing.T) {
	_, _, err := parseLifecycleFlags([]string{"--path"}, nil)
	if err == nil {
		t.Fatal("expected value-required error")
	}
	if !strings.Contains(err.Error(), "requires a value") {
		t.Errorf("err = %q", err)
	}
}

func TestParseLifecycleFlags_ApplyIsBoolean(t *testing.T) {
	// --apply takes no value; positional after it must still be
	// consumed as a positional, not as --apply's value.
	la, pos, err := parseLifecycleFlags([]string{"--apply", "lab-a"}, nil)
	if err != nil {
		t.Fatalf("parse: %v", err)
	}
	if !la.Apply {
		t.Error("Apply should be true")
	}
	if len(pos) != 1 || pos[0] != "lab-a" {
		t.Errorf("positional = %v; want [lab-a]", pos)
	}
}

func TestEmitPreview_StructuredOutput(t *testing.T) {
	plan := &lifecyclePlan{
		Op:       "onboard",
		PeerName: "new-orin",
		StateAdd: &peerset.Peer{
			Name:         "new-orin",
			WGCP0Address: "198.18.0.99",
			Role:         "mgmt-host",
			Site:         "lab",
		},
		RelayAdd:       []string{derivePubkeyPlaceholder()},
		BundleMint:     true,
		BundleOutPath:  "/tmp/b.cbor",
		AffectedRelays: []string{"relay-rla", "relay-rlb"},
		NotesForOp:     []string{"sample note"},
	}
	var buf bytes.Buffer
	emitPreview(&buf, plan, false)
	got := buf.String()
	wants := []string{
		"PLAN — wg-cp0 onboard new-orin",
		"State plan",
		"ADD peers[]: new-orin (198.18.0.99)",
		"Apply plan",
		"Mint bundle at /tmp/b.cbor",
		"Affected relays: relay-rla, relay-rlb",
		"sample note",
		"Re-run with --apply",
	}
	for _, w := range wants {
		if !strings.Contains(got, w) {
			t.Errorf("preview missing %q in:\n%s", w, got)
		}
	}
}

// --- structural sanity checks --------------------------------------------

func TestStructuralPlanShape(t *testing.T) {
	// Sanity check: hasStateChange and hasApplyChange flip true/false
	// per the plan composition. Cheap insurance against silent
	// regressions in the preview formatter.
	cases := []struct {
		name             string
		plan             *lifecyclePlan
		wantStateChange  bool
		wantApplyChange  bool
	}{
		{"empty", &lifecyclePlan{}, false, false},
		{"add-only", &lifecyclePlan{StateAdd: &peerset.Peer{}}, true, false},
		{"relay-only", &lifecyclePlan{RelayAdd: []string{"x"}}, false, true},
		{"both", &lifecyclePlan{StateRemove: &peerset.Peer{}, RelayRemove: []string{"x"}}, true, true},
		{"rotate", &lifecyclePlan{StateRotate: &rotatePlanDetail{}}, true, false},
		{"resurrect", &lifecyclePlan{StateResurrect: &peerset.Peer{}}, true, false},
	}
	for _, tc := range cases {
		if got := hasStateChange(tc.plan); got != tc.wantStateChange {
			t.Errorf("%s: hasStateChange = %v, want %v", tc.name, got, tc.wantStateChange)
		}
		if got := hasApplyChange(tc.plan); got != tc.wantApplyChange {
			t.Errorf("%s: hasApplyChange = %v, want %v", tc.name, got, tc.wantApplyChange)
		}
	}
}

// --- v1 peerset interop --------------------------------------------------

func TestRun_Onboard_AgainstFreshPath(t *testing.T) {
	// A fresh path (no peerset.json yet) should onboard cleanly —
	// peerset.Load returns empty v2 envelope; nothing collides.
	path := filepath.Join(t.TempDir(), "fresh.json")
	code, stdout, stderr := runArgs(t, "onboard", "first-peer",
		"--site", "lab", "--role", "mgmt-host",
		"--wg-cp0-address", "198.18.0.1/24",
		"--path", path)
	if code != 0 {
		t.Fatalf("code = %d (stderr = %q)", code, stderr)
	}
	if !strings.Contains(stdout, "ADD peers[]: first-peer") {
		t.Errorf("dry-run missing ADD: %q", stdout)
	}
}

// --- preview shape regression --------------------------------------------

func TestEmitPreview_RotateShape(t *testing.T) {
	plan := &lifecyclePlan{
		Op:       "rotate",
		PeerName: "lab-a",
		StateRotate: &rotatePlanDetail{
			PeerName:    "lab-a",
			PriorPubkey: "abc-prior=",
			Reason:      "compromise response",
		},
		RelayRemove: []string{"abc-prior="},
		RelayAdd:    []string{derivePubkeyPlaceholder()},
	}
	var buf bytes.Buffer
	emitPreview(&buf, plan, true) // --apply path: verb shifts
	got := buf.String()
	if !strings.Contains(got, "APPLY") {
		t.Errorf("expected APPLY verb; got %q", got)
	}
	if !strings.Contains(got, "ROTATE peer lab-a") {
		t.Errorf("missing ROTATE line: %q", got)
	}
	if !strings.Contains(got, "compromise response") {
		t.Errorf("missing reason: %q", got)
	}
}

// Probe: --path default falls through to defaultPath when not set.
func TestParseLifecycleFlags_DefaultPath(t *testing.T) {
	la, _, err := parseLifecycleFlags([]string{}, nil)
	if err != nil {
		t.Fatalf("parse: %v", err)
	}
	if la.Path != defaultPath {
		t.Errorf("Path = %q; want default %q", la.Path, defaultPath)
	}
}

// Verify the dry-run preview is JSON-clean enough for downstream
// tooling to scrape — every line is text, no JSON object dump that
// would confuse an interactive grep.
func TestRun_Onboard_DryRunIsTextOnly(t *testing.T) {
	path := seedPeerset(t, nil, nil)
	_, stdout, _ := runArgs(t, "onboard", "p",
		"--site", "s", "--role", "r",
		"--wg-cp0-address", "10.99.0.1/24",
		"--path", path)
	// Look for stray JSON markers — the preview output should be
	// human-readable text with field-name : value pairs, not a JSON
	// object dump.
	if json.Valid([]byte(stdout)) {
		t.Errorf("dry-run output should not be parseable as a single JSON value; got %q", stdout)
	}
}

// Probe: the peerset is not mutated after a failed dry-run.
func TestRun_Onboard_NoMutationOnFailedDryRun(t *testing.T) {
	path := seedPeerset(t, []peerset.Peer{activePeer()}, nil)
	before, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read peerset: %v", err)
	}
	_, _, _ = runArgs(t, "onboard", "lab-a",
		"--site", "lab", "--role", "mgmt-host",
		"--wg-cp0-address", "198.18.0.99/24",
		"--path", path)
	after, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read peerset: %v", err)
	}
	if !bytes.Equal(before, after) {
		t.Error("peerset content changed after a failed dry-run")
	}
}
