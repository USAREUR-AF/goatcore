package api

import (
	"context"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

type sweepFixture struct {
	t   *testing.T
	srv *Server
	dir string
	now time.Time
}

func newSweepFixture(t *testing.T, first string) *sweepFixture {
	srv, _ := bulkTestServer(t, first)
	dir := t.TempDir()
	srv.cfg.VendorBundleDir = dir
	srv.cfg.BundleTTL = 24 * time.Hour
	srv.reg.SetBundleDir(dir)
	return &sweepFixture{t: t, srv: srv, dir: dir, now: time.Now()}
}

// file writes a tarball-like file with the given age (mtime now-age).
func (f *sweepFixture) file(dir, name string, age time.Duration) string {
	f.t.Helper()
	p := filepath.Join(dir, name)
	if err := os.WriteFile(p, []byte("key-material"), 0o600); err != nil {
		f.t.Fatal(err)
	}
	f.age(p, age)
	return p
}

func (f *sweepFixture) age(p string, age time.Duration) {
	f.t.Helper()
	m := f.now.Add(-age)
	if err := os.Chtimes(p, m, m); err != nil {
		f.t.Fatal(err)
	}
}

func (f *sweepFixture) vendor(prefix string) {
	f.t.Helper()
	approved := f.now.Unix()
	if err := f.srv.reg.CreateVendor(context.Background(), &registry.Vendor{
		Prefix: prefix, APIKeyHash: "$2a$10$placeholderhash", DisplayName: prefix,
		ApprovedAt: &approved, Approver: ptrString("test"),
	}); err != nil {
		f.t.Fatal(err)
	}
}

// mark names path on prefix's row, with bundle_minted_at now-age (nil: NULL).
func (f *sweepFixture) mark(prefix, path string, age *time.Duration) {
	f.t.Helper()
	ctx := context.Background()
	if err := f.srv.reg.MarkBundle(ctx, prefix, "tok-"+prefix, path); err != nil {
		f.t.Fatal(err)
	}
	var minted *int64
	if age != nil {
		v := f.now.Add(-*age).Unix()
		minted = &v
	}
	if _, err := f.srv.reg.DB().ExecContext(ctx, `UPDATE vendors SET bundle_minted_at = $1 WHERE prefix = $2`, minted, prefix); err != nil {
		f.t.Fatal(err)
	}
}

func (f *sweepFixture) sweep() int {
	f.t.Helper()
	n, err := f.srv.PruneExpiredBundles(context.Background(), f.now)
	if err != nil {
		f.t.Fatal(err)
	}
	return n
}

func gone(t *testing.T, p string) {
	t.Helper()
	if _, err := os.Lstat(p); !os.IsNotExist(err) {
		t.Errorf("%s still on disk (lstat err %v)", filepath.Base(p), err)
	}
}

func kept(t *testing.T, p string) {
	t.Helper()
	if _, err := os.Lstat(p); err != nil {
		t.Errorf("%s removed: %v", filepath.Base(p), err)
	}
}

func dur(d time.Duration) *time.Duration { return &d }

// The sweep removes an old tarball no live row names (expired row, NULL-stamp
// row, no row at all), and keeps: a file a live row names whatever its mtime,
// a file shared by an expired and a live row, a fresh unreferenced file (a
// mint in progress), a symlink (not followed), a non-.tar.gz file, and an
// expired row's file outside the directory.
func TestPruneExpiredBundles_directorySweep(t *testing.T) {
	f := newSweepFixture(t, "sw-expired")
	outside := t.TempDir()
	old := 48 * time.Hour
	for _, p := range []string{"sw-live", "sw-shared-old", "sw-shared-new", "sw-nostamp", "sw-outside"} {
		f.vendor(p)
	}

	expired := f.file(f.dir, "goat-sw-expired.tar.gz", old)
	f.mark("sw-expired", expired, dur(25*time.Hour))
	live := f.file(f.dir, "goat-sw-live.tar.gz", old) // old mtime, live row
	f.mark("sw-live", live, dur(time.Hour))
	shared := f.file(f.dir, "goat-sw-shared.tar.gz", old)
	f.mark("sw-shared-old", shared, dur(48*time.Hour))
	f.mark("sw-shared-new", shared, dur(time.Hour))
	nostamp := f.file(f.dir, "goat-sw-nostamp.tar.gz", old)
	f.mark("sw-nostamp", nostamp, nil)
	far := f.file(outside, "goat-sw-outside.tar.gz", old)
	f.mark("sw-outside", far, dur(25*time.Hour))
	orphan := f.file(f.dir, "goat-sw-orphan.tar.gz", old)
	inProgress := f.file(f.dir, "goat-sw-minting.tar.gz", time.Minute)
	notGz := f.file(f.dir, "goat-sw-notes.txt", old)
	target := f.file(outside, "goat-sw-target.tar.gz", old)
	link := filepath.Join(f.dir, "goat-sw-link.tar.gz")
	if err := os.Symlink(target, link); err != nil {
		t.Fatal(err)
	}

	if n := f.sweep(); n != 3 {
		t.Errorf("removed %d files, want 3 (expired, nostamp, orphan)", n)
	}
	for _, p := range []string{expired, nostamp, orphan} {
		gone(t, p)
	}
	for _, p := range []string{live, shared, far, inProgress, notGz, link, target} {
		kept(t, p)
	}
	if n := f.sweep(); n != 0 {
		t.Errorf("second sweep removed %d, want 0", n)
	}
	// The expired link answers 410, not a 500 for the missing file.
	w := httptest.NewRecorder()
	f.srv.Routes().ServeHTTP(w, httptest.NewRequest("GET", "/api/org-bundle/sw-expired/tok-sw-expired.tar.gz", nil))
	if w.Code != http.StatusGone {
		t.Errorf("pruned link: status %d, want 410", w.Code)
	}
}

// 1a: a slot re-claim (AllocateSlot) NULLs bundle_path without removing the
// old tenant's tarball; the sweep removes it once it is older than the TTL.
func TestPruneExpiredBundles_sweepsReclaimedSlotOrphan(t *testing.T) {
	f := newSweepFixture(t, "sw-unused")
	ctx := context.Background()
	reg := f.srv.reg
	if _, err := reg.InitializeSlotPool(ctx, "sweep-salt", 1); err != nil {
		t.Fatal(err)
	}
	slot, err := reg.AllocateSlot(ctx, "sys-1")
	if err != nil {
		t.Fatal(err)
	}
	f.vendor(slot.SlotID)
	p := f.file(f.dir, "goat-slot-tenant1.tar.gz", 0)
	if err := reg.MarkBundle(ctx, slot.SlotID, "t1", p); err != nil {
		t.Fatal(err)
	}
	if err := reg.StampSlotCert(ctx, slot.SlotID, "t1", p); err != nil {
		t.Fatal(err)
	}
	if err := reg.DeleteVendor(ctx, slot.SlotID); err != nil {
		t.Fatal(err)
	}
	kept(t, p) // the slot row still names it
	if _, err := reg.RevokeAssignedSlot(ctx, slot.SlotID); err != nil {
		t.Fatal(err)
	}
	again, err := reg.AllocateSlot(ctx, "sys-2")
	if err != nil {
		t.Fatal(err)
	}
	if again.SlotID != slot.SlotID {
		t.Fatalf("re-claim got slot %s, want %s", again.SlotID, slot.SlotID)
	}
	kept(t, p) // orphaned: no row names it, the fast path never saw it

	f.age(p, 25*time.Hour)
	if n := f.sweep(); n != 1 {
		t.Errorf("removed %d, want 1", n)
	}
	gone(t, p)
}

// 1b: two overlapping rotates of one org. The second UPDATE's RETURNING
// snapshot still shows the pre-statement path, so the fast path drops that
// one and the first rotate's tarball is left named by no row; the sweep
// removes it once it is older than the TTL, and keeps the live one.
func TestPruneExpiredBundles_sweepsOverlappingRotateOrphan(t *testing.T) {
	f := newSweepFixture(t, "sw-epq")
	ctx := context.Background()
	reg := f.srv.reg
	p0 := f.file(f.dir, "goat-sw-epq-0.tar.gz", 0)
	pA := f.file(f.dir, "goat-sw-epq-A.tar.gz", 0)
	pB := f.file(f.dir, "goat-sw-epq-B.tar.gz", 0)
	if err := reg.MarkBundle(ctx, "sw-epq", "t0", p0); err != nil {
		t.Fatal(err)
	}

	// Rotate A: the same statement as MarkBundle, held open in a transaction.
	tx, err := reg.DB().BeginTxx(ctx, nil)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := tx.ExecContext(ctx,
		`UPDATE vendors AS v SET bundle_token = $1, bundle_path = $2, bundle_minted_at = $3
		   FROM vendors AS o WHERE v.prefix = $4 AND o.prefix = v.prefix`,
		"tA", pA, time.Now().Unix(), "sw-epq"); err != nil {
		_ = tx.Rollback()
		t.Fatal(err)
	}
	// Rotate B blocks on A's row lock.
	done := make(chan error, 1)
	go func() { done <- reg.MarkBundle(ctx, "sw-epq", "tB", pB) }()
	time.Sleep(300 * time.Millisecond)
	if err := tx.Commit(); err != nil {
		t.Fatal(err)
	}
	if err := <-done; err != nil {
		t.Fatal(err)
	}
	var named string
	if err := reg.DB().GetContext(ctx, &named, `SELECT bundle_path FROM vendors WHERE prefix = 'sw-epq'`); err != nil {
		t.Fatal(err)
	}
	if named != pB {
		t.Fatalf("row names %s, want %s", named, pB)
	}
	if _, err := os.Stat(pA); err != nil {
		t.Skipf("rotate A's tarball already removed (the race did not interleave): %v", err)
	}

	f.age(pA, 25*time.Hour)
	f.age(pB, 25*time.Hour) // old mtime, but the live row names it
	if n := f.sweep(); n != 1 {
		t.Errorf("removed %d, want 1", n)
	}
	gone(t, pA)
	kept(t, pB)
}

// A mint killed mid-way leaves its stage directory (leaf key, plaintext api
// key) behind. The sweep removes an old one; it keeps a fresh one (a mint in
// progress), does not follow a symlink named like one, and leaves an old
// directory with any other name alone.
func TestPruneExpiredBundles_staleStageDirs(t *testing.T) {
	f := newSweepFixture(t, "sw-stage")
	old := 48 * time.Hour
	mkStage := func(name string, age time.Duration) string {
		t.Helper()
		d := filepath.Join(f.dir, name)
		if err := os.Mkdir(d, 0o750); err != nil {
			t.Fatal(err)
		}
		f.file(d, "sw-stage-key.pem", age)
		f.file(d, "portal-api-key", age)
		f.age(d, age)
		return d
	}
	stale := mkStage(".stage-sw-stage-20250101T000000Z", old)
	fresh := mkStage(".stage-sw-stage-20991231T235959Z", time.Minute)
	other := mkStage(".stage-notes", old) // not certmint's shape
	plain := mkStage("archive", old)      // any other directory
	outside := t.TempDir()                // symlink target with old contents
	target := f.file(outside, "secret.pem", old)
	f.age(outside, old)
	link := filepath.Join(f.dir, ".stage-sw-link-20250101T000000Z")
	if err := os.Symlink(outside, link); err != nil {
		t.Fatal(err)
	}

	if n := f.sweep(); n != 1 {
		t.Errorf("removed %d entries, want 1 (the stale stage dir)", n)
	}
	gone(t, stale)
	for _, p := range []string{fresh, filepath.Join(fresh, "sw-stage-key.pem"), other, plain, link, outside, target} {
		kept(t, p)
	}
}

// A relative VendorBundleDir (rows hold "rel/x.tar.gz" as certmint minted
// them) must not make a live row look dead: the sweep compares canonical
// paths, so an old-mtime file a live row names stays, and an expired row's
// file is still removed.
func TestPruneExpiredBundles_relativeBundleDir(t *testing.T) {
	f := newSweepFixture(t, "sw-rel-live")
	f.vendor("sw-rel-old")
	root, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	if err := os.Mkdir(filepath.Join(root, "rel"), 0o750); err != nil {
		t.Fatal(err)
	}
	t.Chdir(root)
	f.srv.cfg.VendorBundleDir = "rel"
	f.srv.reg.SetBundleDir("rel")

	live := f.file("rel", "goat-sw-rel-live.tar.gz", 48*time.Hour)
	f.mark("sw-rel-live", live, dur(time.Hour))
	old := f.file("rel", "goat-sw-rel-old.tar.gz", 48*time.Hour)
	f.mark("sw-rel-old", old, dur(25*time.Hour))
	if n := f.sweep(); n != 1 {
		t.Errorf("removed %d, want 1", n)
	}
	kept(t, live)
	gone(t, old)
}

// A symlinked VendorBundleDir: rows hold the real spelling and the link
// spelling; a live row protects its file under either, and an expired row's
// file is removed under either. The after-commit fast path keeps a file a
// sibling row names under the other spelling.
func TestPruneExpiredBundles_symlinkedBundleDir(t *testing.T) {
	f := newSweepFixture(t, "sw-ln-a")
	for _, p := range []string{"sw-ln-b", "sw-ln-c", "sw-ln-d", "sw-ln-e"} {
		f.vendor(p)
	}
	root, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	real := filepath.Join(root, "real")
	if err := os.Mkdir(real, 0o750); err != nil {
		t.Fatal(err)
	}
	link := filepath.Join(root, "link")
	if err := os.Symlink(real, link); err != nil {
		t.Fatal(err)
	}
	f.srv.cfg.VendorBundleDir = link
	f.srv.reg.SetBundleDir(link)

	liveReal := f.file(real, "goat-sw-ln-a.tar.gz", 48*time.Hour)
	f.mark("sw-ln-a", liveReal, dur(time.Hour))
	liveLink := f.file(real, "goat-sw-ln-b.tar.gz", 48*time.Hour)
	f.mark("sw-ln-b", filepath.Join(link, "goat-sw-ln-b.tar.gz"), dur(time.Hour))
	oldLink := f.file(real, "goat-sw-ln-c.tar.gz", 48*time.Hour)
	f.mark("sw-ln-c", filepath.Join(link, "goat-sw-ln-c.tar.gz"), dur(25*time.Hour))
	if n := f.sweep(); n != 1 {
		t.Errorf("removed %d, want 1", n)
	}
	kept(t, liveReal)
	kept(t, liveLink)
	gone(t, oldLink)

	// Fast path: d and e name one file under different spellings; re-pointing
	// d keeps it (e still names it), re-pointing e then removes it.
	shared := f.file(real, "goat-sw-ln-shared.tar.gz", 0)
	f.mark("sw-ln-d", filepath.Join(link, "goat-sw-ln-shared.tar.gz"), dur(0))
	f.mark("sw-ln-e", shared, dur(0))
	next := f.file(real, "goat-sw-ln-next.tar.gz", 0)
	f.mark("sw-ln-d", next, dur(0))
	kept(t, shared)
	f.mark("sw-ln-e", next, dur(0))
	gone(t, shared)
}
