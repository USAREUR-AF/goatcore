package main

import (
	"bytes"
	"encoding/json"
	"path/filepath"
	"strings"
	"testing"
)

const fixturePath = "../../peerset/testdata/v1-flat.json"

func runArgs(t *testing.T, args ...string) (int, string, string) {
	t.Helper()
	var stdout, stderr bytes.Buffer
	code := run(args, &stdout, &stderr)
	return code, stdout.String(), stderr.String()
}

func TestRun_NoArgs(t *testing.T) {
	code, _, stderr := runArgs(t)
	if code == 0 {
		t.Fatalf("no-args should fail; got code %d", code)
	}
	if !strings.Contains(stderr, "usage") {
		t.Errorf("stderr should mention usage; got %q", stderr)
	}
}

func TestRun_UnknownSubcommand(t *testing.T) {
	code, _, stderr := runArgs(t, "frobnicate")
	if code != 2 {
		t.Errorf("code = %d, want 2", code)
	}
	if !strings.Contains(stderr, "unknown subcommand") {
		t.Errorf("stderr = %q", stderr)
	}
}

func TestRun_HelpFlag(t *testing.T) {
	for _, flag := range []string{"-h", "--help", "help"} {
		code, stdout, _ := runArgs(t, flag)
		if code != 0 || !strings.Contains(stdout, "wg-cp0") {
			t.Errorf("%s: code=%d stdout=%q", flag, code, stdout)
		}
	}
}

func TestRun_Validate_PassesOnFixture(t *testing.T) {
	code, stdout, _ := runArgs(t, "validate", "--path", fixturePath)
	if code != 0 {
		t.Errorf("code = %d, want 0", code)
	}
	if !strings.Contains(stdout, "PASS") {
		t.Errorf("stdout = %q", stdout)
	}
}

func TestRun_Validate_MissingFileIsClean(t *testing.T) {
	// A missing file is treated as an empty v2 envelope by peerset.Load,
	// which passes validation; the orchestrator surface returns 0.
	code, stdout, _ := runArgs(t, "validate", "--path", filepath.Join(t.TempDir(), "absent.json"))
	if code != 0 {
		t.Errorf("code = %d, want 0", code)
	}
	if !strings.Contains(stdout, "PASS") {
		t.Errorf("stdout = %q", stdout)
	}
}

func TestRun_List_PrintsCanonicalFleet(t *testing.T) {
	code, stdout, _ := runArgs(t, "list", "--path", fixturePath)
	if code != 0 {
		t.Fatalf("code = %d", code)
	}
	// Canonical peers from CLAUDE.md fleet must appear.
	wants := []string{"lab-a", "prov-conus", "op-laptop"}
	for _, w := range wants {
		if !strings.Contains(stdout, w) {
			t.Errorf("list output missing peer %q", w)
		}
	}
}

func TestRun_List_FilterBySite(t *testing.T) {
	// The v1 fixture migrates without site metadata, so a --site filter
	// produces an empty result. That's the correct shape — the test
	// asserts the filter logic runs.
	code, stdout, _ := runArgs(t, "list", "--path", fixturePath, "--site", "lab")
	if code != 0 {
		t.Fatalf("code = %d, want 0", code)
	}
	for _, p := range []string{"lab-a", "prov-conus"} {
		if strings.Contains(stdout, p) {
			t.Errorf("filter should have excluded %q (v1 has no site metadata)", p)
		}
	}
	// Header row still present.
	if !strings.Contains(stdout, "NAME") {
		t.Errorf("table header missing: %q", stdout)
	}
}

func TestRun_List_JSON(t *testing.T) {
	code, stdout, _ := runArgs(t, "list", "--path", fixturePath, "--json")
	if code != 0 {
		t.Fatalf("code = %d, want 0", code)
	}
	var payload struct {
		SchemaVersion int                     `json:"schema_version"`
		Peers         []struct{ Name string } `json:"peers"`
	}
	if err := json.Unmarshal([]byte(stdout), &payload); err != nil {
		t.Fatalf("json parse: %v", err)
	}
	if payload.SchemaVersion != 2 || len(payload.Peers) != 11 {
		t.Errorf("payload = %+v", payload)
	}
}

func TestRun_Show_PrintsPeer(t *testing.T) {
	code, stdout, _ := runArgs(t, "show", "lab-a", "--path", fixturePath)
	if code != 0 {
		t.Fatalf("code = %d", code)
	}
	if !strings.Contains(stdout, "198.18.0.1") {
		t.Errorf("address not in output: %q", stdout)
	}
}

func TestRun_Show_MissingPeer(t *testing.T) {
	code, _, stderr := runArgs(t, "show", "ghost", "--path", fixturePath)
	if code != 1 {
		t.Errorf("code = %d, want 1", code)
	}
	if !strings.Contains(stderr, "not found") {
		t.Errorf("stderr = %q", stderr)
	}
}

func TestRun_Show_RequiresPeerName(t *testing.T) {
	code, _, stderr := runArgs(t, "show", "--path", fixturePath)
	if code != 2 {
		t.Errorf("code = %d, want 2", code)
	}
	if !strings.Contains(stderr, "positional") {
		t.Errorf("stderr = %q", stderr)
	}
}

// TestRun_AllSubcommandsAreWired confirms every documented subcommand
// dispatches to a real handler now that all three Block 22R.2 sessions
// (A: reconcile + diff; B: lifecycle; C: metadata) are merged. Each
// listed subcommand is invoked with no flags; we assert the dispatch
// did NOT fall through to the EX_USAGE 64 "reserved by Session" path.
// Subcommands may legitimately exit with codes 1 or 2 here (missing
// args, etc.); 64 specifically means dispatch found no handler.
func TestRun_AllSubcommandsAreWired(t *testing.T) {
	for _, cmd := range []string{
		// Session A
		"diff", "list", "reconcile", "show", "validate",
		// Session B
		"onboard", "remove", "resurrect", "rotate",
		// Session C
		"add-relay", "update-endpoints", "update-ttl",
	} {
		code, _, stderr := runArgs(t, cmd)
		if code == 64 {
			t.Errorf("%s: dispatch returned 64 (reserved); want a real handler. stderr=%q", cmd, stderr)
		}
	}
}

func TestRun_Validate_RejectsInvalidPath(t *testing.T) {
	dir := t.TempDir()
	p := filepath.Join(dir, "bad.json")
	// schema_version missing → ErrSchemaInvalid
	if err := writeFile(p, []byte(`{"peers": []}`)); err != nil {
		t.Fatal(err)
	}
	code, _, stderr := runArgs(t, "validate", "--path", p)
	if code != 1 {
		t.Errorf("code = %d, want 1", code)
	}
	if !strings.Contains(stderr, "FAIL") && !strings.Contains(stderr, "schema") {
		t.Errorf("stderr = %q", stderr)
	}
}

func TestRun_UnknownFlag(t *testing.T) {
	code, _, stderr := runArgs(t, "list", "--bogus", "x")
	if code != 2 {
		t.Errorf("code = %d, want 2", code)
	}
	if !strings.Contains(stderr, "unknown flag") {
		t.Errorf("stderr = %q", stderr)
	}
}

func TestRun_Show_JSON(t *testing.T) {
	code, stdout, _ := runArgs(t, "show", "prov-conus", "--path", fixturePath, "--json")
	if code != 0 {
		t.Fatalf("code = %d", code)
	}
	var got map[string]any
	if err := json.Unmarshal([]byte(stdout), &got); err != nil {
		t.Fatalf("json: %v", err)
	}
	if got["name"] != "prov-conus" {
		t.Errorf("got %+v", got)
	}
}
