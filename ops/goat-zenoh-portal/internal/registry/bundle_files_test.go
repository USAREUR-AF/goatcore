package registry

import (
	"context"
	"os"
	"path/filepath"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/pgtest"
)

func bundleFile(t *testing.T, dir, name string) string {
	t.Helper()
	p := filepath.Join(dir, name)
	if err := os.WriteFile(p, []byte("key-material"), 0o600); err != nil {
		t.Fatal(err)
	}
	return p
}

func requireGone(t *testing.T, p string) {
	t.Helper()
	if _, err := os.Stat(p); !os.IsNotExist(err) {
		t.Errorf("%s still on disk (stat err %v)", filepath.Base(p), err)
	}
}

func requireKept(t *testing.T, p string) {
	t.Helper()
	if _, err := os.Stat(p); err != nil {
		t.Errorf("%s removed: %v", filepath.Base(p), err)
	}
}

func bundleTestRegistry(t *testing.T, prefixes ...string) (*Registry, string) {
	t.Helper()
	reg, err := Open(pgtest.DSN(t))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = reg.Close() })
	dir := t.TempDir()
	reg.SetBundleDir(dir)
	for _, p := range prefixes {
		if err := reg.CreateVendor(context.Background(), &Vendor{Prefix: p, APIKeyHash: "$2a$10$placeholderhash", DisplayName: p}); err != nil {
			t.Fatal(err)
		}
	}
	return reg, dir
}

// A rotate (MarkBundle re-pointing bundle_path) removes the tarball the row
// named before, unless another row still names it; the new tarball stays.
func TestMarkBundle_removesPreviousTarball(t *testing.T) {
	ctx := context.Background()
	reg, dir := bundleTestRegistry(t, "rot-a", "rot-b")
	a1 := bundleFile(t, dir, "goat-rot-a-1.tar.gz")
	a2 := bundleFile(t, dir, "goat-rot-a-2.tar.gz")
	if err := reg.MarkBundle(ctx, "rot-a", "t1", a1); err != nil {
		t.Fatal(err)
	}
	if err := reg.MarkBundle(ctx, "rot-a", "t2", a2); err != nil {
		t.Fatal(err)
	}
	requireGone(t, a1)
	requireKept(t, a2)

	// Re-marking the same path keeps it.
	if err := reg.MarkBundle(ctx, "rot-a", "t3", a2); err != nil {
		t.Fatal(err)
	}
	requireKept(t, a2)

	// A path another row still names is kept.
	shared := bundleFile(t, dir, "goat-rot-shared.tar.gz")
	b2 := bundleFile(t, dir, "goat-rot-b-2.tar.gz")
	if err := reg.MarkBundle(ctx, "rot-a", "t4", shared); err != nil {
		t.Fatal(err)
	}
	requireGone(t, a2)
	if err := reg.MarkBundle(ctx, "rot-b", "u1", shared); err != nil {
		t.Fatal(err)
	}
	if err := reg.MarkBundle(ctx, "rot-b", "u2", b2); err != nil {
		t.Fatal(err)
	}
	requireKept(t, shared) // rot-a still names it

	// Guards: a previous path outside the directory, or not *.tar.gz, stays.
	outside := bundleFile(t, t.TempDir(), "goat-rot-a-out.tar.gz")
	notGz := bundleFile(t, dir, "goat-rot-a.txt")
	for _, p := range []string{outside, notGz} {
		if err := reg.MarkBundle(ctx, "rot-a", "g", p); err != nil {
			t.Fatal(err)
		}
	}
	if err := reg.MarkBundle(ctx, "rot-a", "g2", b2); err != nil {
		t.Fatal(err)
	}
	requireKept(t, outside)
	requireKept(t, notGz)
}

// DeleteVendor removes the deleted org's tarball unless another row names it.
func TestDeleteVendor_removesTarball(t *testing.T) {
	ctx := context.Background()
	reg, dir := bundleTestRegistry(t, "del-a", "del-b", "del-c")
	a := bundleFile(t, dir, "goat-del-a.tar.gz")
	shared := bundleFile(t, dir, "goat-del-shared.tar.gz")
	for p, path := range map[string]string{"del-a": a, "del-b": shared, "del-c": shared} {
		if err := reg.MarkBundle(ctx, p, "tok-"+p, path); err != nil {
			t.Fatal(err)
		}
	}
	if err := reg.DeleteVendor(ctx, "del-a"); err != nil {
		t.Fatal(err)
	}
	requireGone(t, a)
	if err := reg.DeleteVendor(ctx, "del-b"); err != nil {
		t.Fatal(err)
	}
	requireKept(t, shared) // del-c still names it
	if err := reg.DeleteVendor(ctx, "del-c"); err != nil {
		t.Fatal(err)
	}
	requireGone(t, shared)
}

// ClearSlotCert and a re-stamp (StampSlotCert) remove the slot's previous
// tarball, but not while the org's vendor row still names it.
func TestSlotCert_removesPreviousTarball(t *testing.T) {
	ctx := context.Background()
	reg, dir := bundleTestRegistry(t, "slot-org")
	if _, err := reg.InitializeSlotPool(ctx, "test-salt", 2); err != nil {
		t.Fatal(err)
	}
	s, err := reg.AllocateSlot(ctx, "sys-1")
	if err != nil {
		t.Fatal(err)
	}
	p1 := bundleFile(t, dir, "goat-slot-1.tar.gz")
	p2 := bundleFile(t, dir, "goat-slot-2.tar.gz")
	if err := reg.StampSlotCert(ctx, s.SlotID, "s1", p1); err != nil {
		t.Fatal(err)
	}
	if err := reg.StampSlotCert(ctx, s.SlotID, "s2", p2); err != nil {
		t.Fatal(err)
	}
	requireGone(t, p1)

	// The vendor row names p2 too: clearing the slot keeps it.
	if err := reg.MarkBundle(ctx, "slot-org", "v2", p2); err != nil {
		t.Fatal(err)
	}
	if err := reg.ClearSlotCert(ctx, s.SlotID); err != nil {
		t.Fatal(err)
	}
	requireKept(t, p2)

	// Once no row names it, clearing removes it.
	p3 := bundleFile(t, dir, "goat-slot-3.tar.gz")
	if err := reg.StampSlotCert(ctx, s.SlotID, "s3", p3); err != nil {
		t.Fatal(err)
	}
	if err := reg.ClearSlotCert(ctx, s.SlotID); err != nil {
		t.Fatal(err)
	}
	requireGone(t, p3)
	if err := reg.ClearSlotCert(ctx, "no-such-slot"); err == nil {
		t.Error("ClearSlotCert on an unknown slot: no error")
	}
	if err := reg.StampSlotCert(ctx, "no-such-slot", "x", p2); err == nil {
		t.Error("StampSlotCert on an unknown slot: no error")
	}
}

// With no bundle directory set, nothing is removed.
func TestMarkBundle_noBundleDirKeepsFiles(t *testing.T) {
	ctx := context.Background()
	reg, dir := bundleTestRegistry(t, "nodir")
	reg.SetBundleDir("")
	a1 := bundleFile(t, dir, "goat-nodir-1.tar.gz")
	if err := reg.MarkBundle(ctx, "nodir", "t1", a1); err != nil {
		t.Fatal(err)
	}
	if err := reg.MarkBundle(ctx, "nodir", "t2", filepath.Join(dir, "goat-nodir-2.tar.gz")); err != nil {
		t.Fatal(err)
	}
	requireKept(t, a1)
}
