package main

// Additional edge-case tests bringing cmd_onboard / cmd_rotate /
// cmd_remove / cmd_resurrect per-file statement coverage over the
// 85% bar set by the session-B prompt acceptance criteria. These
// exercise:
//   - parse-error paths (unknown flag mid-command)
//   - positional-arity errors
//   - load errors (peerset file is unreadable / malformed)
//   - validation error branches not hit by the happy-path tests
//   - --apply requires --bundle-out checks

import (
	"errors"
	"io"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/enrollment/peerset"
)

// errSyntheticApply is a non-errSessionAPending error used by the
// applier-swap tests below to force the cmdOnboard / cmdRotate /
// cmdRemove / cmdResurrect "generic error" branch (the one that
// doesn't print the legacy-path hint).
var errSyntheticApply = errors.New("synthetic apply failure for coverage")

// lifecyclePlanArgs is the function-signature shape of the package
// applyLifecyclePlan var. Tests use swapApplier to substitute a
// synthetic version. The shape is intentionally minimal — the test
// helper just records that the call happened with the given plan.
//
// The real applier (Session A's substrate) will take richer arguments;
// at that point this shim moves into a small adapter that calls the
// real Applier with the right types. Until then the swap-and-restore
// pattern is enough.
type lifecyclePlanArgs struct {
	PeerSet *peerset.PeerSet
	Plan    *lifecyclePlan
}

func swapApplier(fn func(*lifecyclePlanArgs) error) func() {
	prev := applyLifecyclePlan
	applyLifecyclePlan = func(ps *peerset.PeerSet, plan *lifecyclePlan, _ io.Writer) error {
		return fn(&lifecyclePlanArgs{PeerSet: ps, Plan: plan})
	}
	return func() { applyLifecyclePlan = prev }
}

func restoreApplier(restore func()) {
	restore()
}

// --- onboard edges -------------------------------------------------------

func TestRun_Onboard_UnknownFlag(t *testing.T) {
	code, _, stderr := runArgs(t, "onboard", "p", "--frobnicate", "x")
	if code != 2 {
		t.Errorf("code = %d, want 2", code)
	}
	if !strings.Contains(stderr, "unknown flag") {
		t.Errorf("stderr = %q", stderr)
	}
}

func TestRun_Onboard_TooManyPositionals(t *testing.T) {
	code, _, stderr := runArgs(t, "onboard", "p1", "p2",
		"--site", "s", "--role", "r", "--wg-cp0-address", "1.2.3.4/24")
	if code != 2 {
		t.Errorf("code = %d, want 2", code)
	}
	if !strings.Contains(stderr, "exactly one positional") {
		t.Errorf("stderr = %q", stderr)
	}
}

func TestRun_Onboard_PeerNameRequired(t *testing.T) {
	// Empty positional via no-positional path is covered elsewhere;
	// here we exercise the explicit-empty-string guard inside
	// validateOnboardArgs via a positional with --site filled.
	path := seedPeerset(t, nil, nil)
	code, _, stderr := runArgs(t, "onboard",
		"--site", "lab", "--role", "mgmt-host",
		"--wg-cp0-address", "198.18.0.99/24",
		"--path", path)
	if code != 2 {
		t.Errorf("code = %d, want 2", code)
	}
	if !strings.Contains(stderr, "positional") {
		t.Errorf("stderr = %q", stderr)
	}
}

func TestRun_Onboard_LoadFailsOnMalformedPeerset(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "bad.json")
	// schema_version missing → ErrSchemaInvalid via peerset.parse
	if err := os.WriteFile(path, []byte(`{"peers": [],}`), 0o644); err != nil {
		t.Fatalf("write: %v", err)
	}
	code, _, stderr := runArgs(t, "onboard", "p",
		"--site", "s", "--role", "r",
		"--wg-cp0-address", "10.0.0.1/24",
		"--path", path)
	if code != 1 {
		t.Errorf("code = %d, want 1", code)
	}
	if !strings.Contains(stderr, "load peerset") && !strings.Contains(stderr, "schema") && !strings.Contains(stderr, "parse") {
		t.Errorf("stderr = %q", stderr)
	}
}

// --- rotate edges --------------------------------------------------------

func TestRun_Rotate_UnknownFlag(t *testing.T) {
	code, _, stderr := runArgs(t, "rotate", "p", "--frobnicate", "x")
	if code != 2 {
		t.Errorf("code = %d, want 2", code)
	}
	if !strings.Contains(stderr, "unknown flag") {
		t.Errorf("stderr = %q", stderr)
	}
}

func TestRun_Rotate_TooManyPositionals(t *testing.T) {
	code, _, stderr := runArgs(t, "rotate", "p1", "p2", "--reason", "x")
	if code != 2 {
		t.Errorf("code = %d, want 2", code)
	}
	if !strings.Contains(stderr, "exactly one positional") {
		t.Errorf("stderr = %q", stderr)
	}
}

func TestRun_Rotate_ApplyRequiresBundleOut(t *testing.T) {
	path := seedPeerset(t, nil, nil)
	code, _, stderr := runArgs(t, "rotate", "lab-a",
		"--reason", "test",
		"--apply",
		"--path", path)
	if code != 2 {
		t.Errorf("code = %d, want 2", code)
	}
	if !strings.Contains(stderr, "--bundle-out") {
		t.Errorf("stderr = %q", stderr)
	}
}

func TestRun_Rotate_LoadFailsOnMalformedPeerset(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "bad.json")
	if err := os.WriteFile(path, []byte(`{not even json`), 0o644); err != nil {
		t.Fatalf("write: %v", err)
	}
	code, _, stderr := runArgs(t, "rotate", "p", "--reason", "x", "--path", path)
	if code != 1 {
		t.Errorf("code = %d, want 1", code)
	}
	if !strings.Contains(stderr, "load peerset") && !strings.Contains(stderr, "schema") && !strings.Contains(stderr, "parse") {
		t.Errorf("stderr = %q", stderr)
	}
}

// --- remove edges --------------------------------------------------------

func TestRun_Remove_UnknownFlag(t *testing.T) {
	code, _, stderr := runArgs(t, "remove", "p", "--frobnicate", "x")
	if code != 2 {
		t.Errorf("code = %d, want 2", code)
	}
	if !strings.Contains(stderr, "unknown flag") {
		t.Errorf("stderr = %q", stderr)
	}
}

func TestRun_Remove_TooManyPositionals(t *testing.T) {
	code, _, stderr := runArgs(t, "remove", "p1", "p2", "--reason", "x")
	if code != 2 {
		t.Errorf("code = %d, want 2", code)
	}
	if !strings.Contains(stderr, "exactly one positional") {
		t.Errorf("stderr = %q", stderr)
	}
}

func TestRun_Remove_LoadFailsOnMalformedPeerset(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "bad.json")
	if err := os.WriteFile(path, []byte(`}`), 0o644); err != nil {
		t.Fatalf("write: %v", err)
	}
	code, _, stderr := runArgs(t, "remove", "p", "--reason", "x", "--path", path)
	if code != 1 {
		t.Errorf("code = %d, want 1", code)
	}
	if !strings.Contains(stderr, "load peerset") && !strings.Contains(stderr, "schema") && !strings.Contains(stderr, "parse") {
		t.Errorf("stderr = %q", stderr)
	}
}

// --- resurrect edges -----------------------------------------------------

func TestRun_Resurrect_UnknownFlag(t *testing.T) {
	code, _, stderr := runArgs(t, "resurrect", "p", "--frobnicate", "x")
	if code != 2 {
		t.Errorf("code = %d, want 2", code)
	}
	if !strings.Contains(stderr, "unknown flag") {
		t.Errorf("stderr = %q", stderr)
	}
}

func TestRun_Resurrect_TooManyPositionals(t *testing.T) {
	code, _, stderr := runArgs(t, "resurrect", "p1", "p2")
	if code != 2 {
		t.Errorf("code = %d, want 2", code)
	}
	if !strings.Contains(stderr, "exactly one positional") {
		t.Errorf("stderr = %q", stderr)
	}
}

func TestRun_Resurrect_LoadFailsOnMalformedPeerset(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "bad.json")
	if err := os.WriteFile(path, []byte(`@`), 0o644); err != nil {
		t.Fatalf("write: %v", err)
	}
	code, _, stderr := runArgs(t, "resurrect", "p", "--path", path)
	if code != 1 {
		t.Errorf("code = %d, want 1", code)
	}
	if !strings.Contains(stderr, "load peerset") && !strings.Contains(stderr, "schema") && !strings.Contains(stderr, "parse") {
		t.Errorf("stderr = %q", stderr)
	}
}

// --- helper edges --------------------------------------------------------

// dashOr's "non-empty" branch is hit by every happy-path preview, but
// the "empty" branch isn't always exercised since happy-path test
// fixtures fill all metadata. Confirm both paths explicitly.
func TestDashOr(t *testing.T) {
	if got := dashOr(""); got != "-" {
		t.Errorf("dashOr(\"\") = %q; want -", got)
	}
	if got := dashOr("lab"); got != "lab" {
		t.Errorf("dashOr(\"lab\") = %q; want passthrough", got)
	}
}

func TestAddrIPOnly(t *testing.T) {
	for _, tc := range []struct {
		in, want string
	}{
		{"198.18.0.1/24", "198.18.0.1"},
		{"198.18.0.1", "198.18.0.1"},
		{"", ""},
	} {
		if got := addrIPOnly(tc.in); got != tc.want {
			t.Errorf("addrIPOnly(%q) = %q; want %q", tc.in, got, tc.want)
		}
	}
}

// PeerAllowedIPs has three branches; the rotate/remove cases land on
// the default branch that returns "<from peerset>". Confirm.
func TestPeerAllowedIPs_DefaultBranch(t *testing.T) {
	if got := peerAllowedIPs(&lifecyclePlan{}); got != "<from peerset>" {
		t.Errorf("peerAllowedIPs(empty) = %q; want default branch", got)
	}
}

// --- resurrect coverage extensions --------------------------------------

func TestRun_Resurrect_EmptyPeerName(t *testing.T) {
	// An operator passing `wg-cp0 resurrect ""` should hit the
	// explicit empty-name guard in cmdResurrect (line 42).
	code, _, stderr := runArgs(t, "resurrect", "")
	if code != 2 {
		t.Errorf("code = %d, want 2", code)
	}
	if !strings.Contains(stderr, "cannot be empty") {
		t.Errorf("stderr = %q", stderr)
	}
}

func TestRun_Resurrect_RejectsPubkeyCollisionWithActive(t *testing.T) {
	// Synthesize a peerset where a deprecated peer's pubkey is
	// claimed by an active peer of a different name + different
	// address.
	dir := t.TempDir()
	path := filepath.Join(dir, "peerset.json")
	if err := os.WriteFile(path, []byte(`{
		"schema_version": 2,
		"peers": [{"name": "other-peer", "wg_cp0_address": "198.18.0.99", "current_pubkey": "abc-shared==", "role": "mgmt-host", "site": "lab"}],
		"deprecated_peers": [{"name": "old-peer", "wg_cp0_address": "198.18.0.5", "current_pubkey": "abc-shared==", "removed_at": "2026-01-01T00:00:00Z", "removed_reason": "test"}],
		"relays": [],
		"metadata": {"schema_min_compatible_version": 2}
	}`), 0o644); err != nil {
		t.Fatalf("write: %v", err)
	}
	code, _, stderr := runArgs(t, "resurrect", "old-peer", "--path", path)
	if code != 1 {
		t.Errorf("code = %d, want 1", code)
	}
	if !strings.Contains(stderr, "pubkey is held by active peer") {
		t.Errorf("stderr = %q", stderr)
	}
}

func TestRun_Resurrect_RejectsAddressCollisionWithActive(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "peerset.json")
	if err := os.WriteFile(path, []byte(`{
		"schema_version": 2,
		"peers": [{"name": "other-peer", "wg_cp0_address": "198.18.0.5", "current_pubkey": "different==", "role": "mgmt-host", "site": "lab"}],
		"deprecated_peers": [{"name": "old-peer", "wg_cp0_address": "198.18.0.5", "current_pubkey": "olds==", "removed_at": "2026-01-01T00:00:00Z", "removed_reason": "test"}],
		"relays": [],
		"metadata": {"schema_min_compatible_version": 2}
	}`), 0o644); err != nil {
		t.Fatalf("write: %v", err)
	}
	code, _, stderr := runArgs(t, "resurrect", "old-peer", "--path", path)
	if code != 1 {
		t.Errorf("code = %d, want 1", code)
	}
	if !strings.Contains(stderr, "address") && !strings.Contains(stderr, "held by active") {
		t.Errorf("stderr = %q", stderr)
	}
}

// --- non-pending applier-error path -------------------------------------
// applyLifecyclePlan is a package-level var so tests can swap it. The
// "non-pending" branch (apply returns a non-errSessionAPending error)
// is the path uncovered by the happy-path tests; confirm it surfaces
// a generic-error message rather than the legacy-path hint.

func TestRun_Onboard_ApplyNonPendingError(t *testing.T) {
	defer restoreApplier(swapApplier(func(*lifecyclePlanArgs) error {
		return errSyntheticApply
	}))
	path := seedPeerset(t, nil, nil)
	bundleOut := filepath.Join(t.TempDir(), "b.cbor")
	code, _, stderr := runArgs(t, "onboard", "new-orin",
		"--site", "lab", "--role", "mgmt-host",
		"--wg-cp0-address", "198.18.0.99/24",
		"--bundle-out", bundleOut,
		"--apply",
		"--path", path)
	if code != 1 {
		t.Errorf("code = %d, want 1", code)
	}
	if strings.Contains(stderr, "Legacy path") {
		t.Errorf("non-pending error should not surface legacy-path hint; got %q", stderr)
	}
	if !strings.Contains(stderr, "synthetic apply") {
		t.Errorf("stderr should surface the underlying error; got %q", stderr)
	}
}

func TestRun_Rotate_ApplyNonPendingError(t *testing.T) {
	defer restoreApplier(swapApplier(func(*lifecyclePlanArgs) error {
		return errSyntheticApply
	}))
	path := seedPeerset(t, []peerset.Peer{activePeer()}, nil)
	code, _, stderr := runArgs(t, "rotate", "lab-a",
		"--reason", "test",
		"--bundle-out", filepath.Join(t.TempDir(), "b.cbor"),
		"--apply",
		"--path", path)
	if code != 1 {
		t.Errorf("code = %d, want 1", code)
	}
	if strings.Contains(stderr, "Legacy path") {
		t.Errorf("non-pending error should not surface legacy-path hint; got %q", stderr)
	}
}

func TestRun_Remove_ApplyNonPendingError(t *testing.T) {
	defer restoreApplier(swapApplier(func(*lifecyclePlanArgs) error {
		return errSyntheticApply
	}))
	path := seedPeerset(t, []peerset.Peer{activePeer()}, nil)
	code, _, stderr := runArgs(t, "remove", "lab-a",
		"--reason", "test", "--apply", "--path", path)
	if code != 1 {
		t.Errorf("code = %d, want 1", code)
	}
	if strings.Contains(stderr, "Legacy path") {
		t.Errorf("non-pending error should not surface legacy-path hint; got %q", stderr)
	}
}

func TestRun_Resurrect_ApplyNonPendingError(t *testing.T) {
	defer restoreApplier(swapApplier(func(*lifecyclePlanArgs) error {
		return errSyntheticApply
	}))
	path := seedPeerset(t, nil, []peerset.DeprecatedPeer{
		{Peer: activePeer(), RemovedAt: time.Now().UTC(), RemovedReason: "test"},
	})
	code, _, stderr := runArgs(t, "resurrect", "lab-a", "--apply", "--path", path)
	if code != 1 {
		t.Errorf("code = %d, want 1", code)
	}
	if strings.Contains(stderr, "Legacy path") {
		t.Errorf("non-pending error should not surface legacy-path hint; got %q", stderr)
	}
}
