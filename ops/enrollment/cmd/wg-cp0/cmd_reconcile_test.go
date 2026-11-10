// cmd_reconcile_test.go — exercises the cmdReconcile + cmdDiff
// subcommand bodies through run() with WG_CP0_OFFLINE so the SSH
// fan-out is replaced by an empty MockLiveStateReader.

package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/enrollment/peerset"
)

// peersetPeer is a tiny constructor for tests that need a peerset.Peer
// without the type assertions cluttering the test body.
func peersetPeer(name, addr, pubkey string) peerset.Peer {
	return peerset.Peer{Name: name, WGCP0Address: addr, CurrentPubkey: pubkey}
}

func withOffline(t *testing.T, f func()) {
	t.Helper()
	old := os.Getenv("WG_CP0_OFFLINE")
	_ = os.Setenv("WG_CP0_OFFLINE", "1")
	defer func() {
		if old == "" {
			_ = os.Unsetenv("WG_CP0_OFFLINE")
		} else {
			_ = os.Setenv("WG_CP0_OFFLINE", old)
		}
	}()
	f()
}

func TestRun_Reconcile_DryRun_PrintsPlan(t *testing.T) {
	withOffline(t, func() {
		code, stdout, _ := runArgs(t, "reconcile", "--path", fixturePath)
		if code != 0 {
			t.Errorf("code = %d, want 0", code)
		}
		if !strings.Contains(stdout, "wg-cp0 reconcile") {
			t.Errorf("header missing: %q", stdout)
		}
	})
}

func TestRun_Reconcile_DryRun_JSON_Parseable(t *testing.T) {
	withOffline(t, func() {
		code, stdout, _ := runArgs(t, "reconcile", "--path", fixturePath, "--json", "--no-color")
		if code != 0 {
			t.Fatalf("code = %d", code)
		}
		var got map[string]any
		if err := json.Unmarshal([]byte(stdout), &got); err != nil {
			t.Fatalf("json: %v\n%s", err, stdout)
		}
		if got["intent_kind"] != "reconcile" {
			t.Errorf("intent_kind = %v", got["intent_kind"])
		}
	})
}

func TestRun_Reconcile_UnknownFlag(t *testing.T) {
	code, _, stderr := runArgs(t, "reconcile", "--bogus", "x")
	if code != 2 {
		t.Errorf("code = %d, want 2", code)
	}
	if !strings.Contains(stderr, "unknown flag") {
		t.Errorf("stderr = %q", stderr)
	}
}

func TestRun_Reconcile_UnexpectedPositional(t *testing.T) {
	code, _, stderr := runArgs(t, "reconcile", "unexpected")
	if code != 2 {
		t.Errorf("code = %d, want 2", code)
	}
	if !strings.Contains(stderr, "positional") {
		t.Errorf("stderr = %q", stderr)
	}
}

func TestRun_Reconcile_Help(t *testing.T) {
	code, stdout, _ := runArgs(t, "reconcile", "--help")
	if code != 0 {
		t.Errorf("code = %d, want 0", code)
	}
	if !strings.Contains(stdout, "wg-cp0 reconcile") {
		t.Errorf("help missing: %q", stdout)
	}
}

func TestRun_Reconcile_Apply_NoLiveReaderInOffline(t *testing.T) {
	// In offline mode (no live reader, no relay mutator) --apply
	// should still execute the (empty) plan and emit an "ok" outcome.
	withOffline(t, func() {
		dir := t.TempDir()
		auditPath := filepath.Join(dir, "audit.jsonl")
		t.Setenv("WG_CP0_AUDIT_PATH", auditPath)

		code, stdout, stderr := runArgs(t, "reconcile", "--path", fixturePath, "--apply")
		if code != 0 {
			t.Errorf("code = %d, want 0; stderr = %q", code, stderr)
		}
		if !strings.Contains(stdout, "apply: ok") {
			t.Errorf("apply outcome missing: %q", stdout)
		}
		// The audit file should exist with at least one row.
		if _, err := os.Stat(auditPath); err != nil {
			t.Errorf("audit file missing: %v", err)
		}
	})
}

func TestRun_Diff_NoDrift_Exit0(t *testing.T) {
	// Diff against the fixture in offline mode. The fixture has 11
	// peers + no relays, so the LivePlan is empty. The reconcile
	// against zero relays yields an empty plan; diff exits 0.
	withOffline(t, func() {
		code, _, _ := runArgs(t, "diff", "--path", fixturePath)
		if code != 0 {
			t.Errorf("code = %d, want 0 (empty fixture has no relays so no drift)", code)
		}
	})
}

func TestRun_Diff_NoApplyFlag(t *testing.T) {
	code, _, stderr := runArgs(t, "diff", "--apply")
	if code != 2 {
		t.Errorf("code = %d, want 2", code)
	}
	if !strings.Contains(stderr, "--apply is not valid") {
		t.Errorf("stderr = %q", stderr)
	}
}

func TestRun_Diff_Help(t *testing.T) {
	code, stdout, _ := runArgs(t, "diff", "--help")
	if code != 0 {
		t.Errorf("code = %d", code)
	}
	if !strings.Contains(stdout, "Read-only") {
		t.Errorf("help missing: %q", stdout)
	}
}

func TestRun_Reconcile_SiteFilter(t *testing.T) {
	withOffline(t, func() {
		code, stdout, _ := runArgs(t, "reconcile", "--path", fixturePath, "--site", "lab", "--no-color")
		if code != 0 {
			t.Errorf("code = %d", code)
		}
		// Just assert it runs cleanly and prints the header. The v1
		// fixture has no site metadata after migrate, so the filter
		// degrades to an empty set.
		if !strings.Contains(stdout, "PLAN") {
			t.Errorf("plan header missing: %q", stdout)
		}
	})
}

func TestRun_Reconcile_RemoveFlag_Surfaces(t *testing.T) {
	withOffline(t, func() {
		code, _, _ := runArgs(t, "reconcile", "--path", fixturePath, "--remove")
		if code != 0 {
			t.Errorf("code = %d", code)
		}
	})
}

func TestParseReconcileFlags_Defaults(t *testing.T) {
	f, pos, err := parseReconcileFlags(nil)
	if err != nil {
		t.Fatalf("parseReconcileFlags: %v", err)
	}
	if len(pos) != 0 {
		t.Errorf("pos = %v", pos)
	}
	if f.path != defaultPath {
		t.Errorf("path = %q", f.path)
	}
	if f.apply {
		t.Error("apply should default false")
	}
}

func TestParseReconcileFlags_BooleanFlags(t *testing.T) {
	args := []string{"--apply", "--no-add", "--remove", "--json", "--color"}
	f, _, err := parseReconcileFlags(args)
	if err != nil {
		t.Fatalf("parseReconcileFlags: %v", err)
	}
	if !f.apply || !f.noAdd || !f.allowRemove || !f.asJSON || !f.color {
		t.Errorf("flags = %+v", f)
	}
}

func TestParseReconcileFlags_ValueFlags(t *testing.T) {
	args := []string{"--path", "/x", "--site", "lab"}
	f, _, err := parseReconcileFlags(args)
	if err != nil {
		t.Fatalf("parseReconcileFlags: %v", err)
	}
	if f.path != "/x" || f.site != "lab" {
		t.Errorf("flags = %+v", f)
	}
}

func TestParseReconcileFlags_MissingValue(t *testing.T) {
	_, _, err := parseReconcileFlags([]string{"--site"})
	if err == nil {
		t.Error("expected missing-value error")
	}
}

func TestSummarizeApply_Nil(t *testing.T) {
	got := summarizeApply(nil)
	if !strings.Contains(got, "nil") {
		t.Errorf("got %q", got)
	}
}

func TestSummarizeApply_Failure(t *testing.T) {
	r := &ApplyResult{
		Outcome: "fail",
		PerStep: []StepOutcome{
			{Kind: "relay-add", Outcome: "fail", Err: "boom"},
		},
	}
	out := summarizeApply(r)
	if !strings.Contains(out, "fail") || !strings.Contains(out, "boom") {
		t.Errorf("out = %q", out)
	}
}

func TestJoinAllowedIPs_DefaultsFleetSubnet(t *testing.T) {
	p := peersetPeer("alpha", "198.18.0.10", "PK_ALPHA")
	got := joinAllowedIPs(p)
	if !strings.Contains(got, "198.18.0.0/24") || !strings.Contains(got, "198.18.0.10/32") {
		t.Errorf("got %q", got)
	}
}

func TestJoinAllowedIPs_RespectsExplicitFleet(t *testing.T) {
	p := peersetPeer("alpha", "198.18.0.10", "PK_ALPHA")
	p.AllowedIPsExtra = []string{"198.18.0.0/24"}
	got := joinAllowedIPs(p)
	// Should NOT double the fleet subnet.
	if strings.Count(got, "198.18.0.0/24") != 1 {
		t.Errorf("got %q (fleet subnet should appear once)", got)
	}
}

func TestAllowedIPsEqual_SetSemantics(t *testing.T) {
	if !allowedIPsEqual("a,b", "b,a") {
		t.Error("set-equal should be true regardless of order")
	}
	if allowedIPsEqual("a", "a,b") {
		t.Error("different sets should be unequal")
	}
	if !allowedIPsEqual("", "") {
		t.Error("empty == empty")
	}
}
