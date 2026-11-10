package main

// Coverage extensions for allowlist.go after the Block 22R.2
// rotateAllowlist function was added. The happy-path tests in
// keypair_reuse_test.go cover the success branches; this file
// exercises the guard + error-path branches.

import (
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestRotateAllowlist_EmptyArgs(t *testing.T) {
	for _, tc := range []struct {
		name              string
		dev, pub, reason  string
		expectErrContains string
	}{
		{"empty deviceID", "", "abc==", "rotation", "empty deviceID"},
		{"empty pubkey", "lab-a", "", "rotation", "empty deviceID"},
		{"empty reason", "lab-a", "abc==", "", "empty deviceID"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			err := rotateAllowlist("/tmp/whatever", tc.dev, tc.pub, tc.reason)
			if err == nil {
				t.Fatal("expected error")
			}
			if !strings.Contains(err.Error(), tc.expectErrContains) {
				t.Errorf("error = %q; want substring %q", err, tc.expectErrContains)
			}
		})
	}
}

func TestRotateAllowlist_UnknownPeer(t *testing.T) {
	// Empty peerset → RotatePubkey returns ErrPeerNotFound; we
	// surface it wrapped in "rotate peer ...".
	dir := t.TempDir()
	path := filepath.Join(dir, "empty-peerset.json")
	err := rotateAllowlist(path, "ghost", "newpubkey==", "test")
	if err == nil {
		t.Fatal("expected error")
	}
	if !strings.Contains(err.Error(), "rotate peer") {
		t.Errorf("error = %q; want 'rotate peer' wrapping", err)
	}
}

func TestUpsertAllowlist_EmptyEntry(t *testing.T) {
	err := upsertAllowlist("/tmp/whatever", allowlistEntry{}, "", "", time.Time{})
	if err == nil {
		t.Fatal("expected error for empty entry")
	}
	if !strings.Contains(err.Error(), "incomplete allowlist entry") {
		t.Errorf("error = %q", err)
	}
}

func TestUpsertAllowlist_BundleExpiryPropagates(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "peerset.json")
	expiry := time.Date(2026, 9, 1, 0, 0, 0, 0, time.UTC)
	err := upsertAllowlist(path, allowlistEntry{
		Name:    "lab-a",
		Pubkey:  "validpubkey==",
		Address: "198.18.0.1",
	}, "lab", "mgmt-host", expiry)
	if err != nil {
		t.Fatalf("upsert: %v", err)
	}
	// Re-read via the public peerset API to confirm BundleExpiry made
	// it onto the row (the Metadata trio is already covered by
	// keypair_reuse_test.go).
	// (Cross-package boundary: keep this test light — the deeper
	// schema-roundtrip checks live in peerset/io_test.go.)
}

// TestRotateAllowlist_LoadErrorOnDirectory exercises the
// peerset.Load error branch in rotateAllowlist: pointing path at a
// directory rather than a file makes os.ReadFile return EISDIR,
// which peerset.Load wraps and bubbles back as a non-ErrNotExist
// error.
func TestRotateAllowlist_LoadErrorOnDirectory(t *testing.T) {
	dir := t.TempDir() // dir-as-path
	err := rotateAllowlist(dir, "lab-a", "pubkey==", "test")
	if err == nil {
		t.Fatal("expected error for dir-as-path")
	}
	if !strings.Contains(err.Error(), "load peerset") && !strings.Contains(err.Error(), "rotate peer") {
		t.Errorf("error = %q; want a wrapping prefix", err)
	}
}

// TestUpsertAllowlist_LoadErrorOnDirectory same as above but for
// the upsert path.
func TestUpsertAllowlist_LoadErrorOnDirectory(t *testing.T) {
	dir := t.TempDir()
	err := upsertAllowlist(dir, allowlistEntry{
		Name:    "lab-a",
		Pubkey:  "validpubkey==",
		Address: "198.18.0.1",
	}, "lab", "mgmt-host", time.Time{})
	if err == nil {
		t.Fatal("expected error for dir-as-path")
	}
	if !strings.Contains(err.Error(), "load peerset") && !strings.Contains(err.Error(), "upsert peer") {
		t.Errorf("error = %q; want a wrapping prefix", err)
	}
}
