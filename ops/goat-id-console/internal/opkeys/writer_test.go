package opkeys

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

type fakeActive struct{ active, known []string }

func (f fakeActive) ActiveHandles(context.Context, string) ([]string, error) {
	return f.active, nil
}

func (f fakeActive) KnownHandles(context.Context, string) ([]string, error) {
	return f.known, nil
}

// The file IS the grant. Its contents must be exactly what the portal parses,
// because a file the portal cannot read is a key that silently does not work —
// which presents to the person as "it was approved and the Portal still refuses
// me", the least debuggable failure in the whole flow.
func TestWrite_producesALineThePortalParses(t *testing.T) {
	d := Dir{Path: t.TempDir()}
	if err := d.Write("ada", "op_deadbeef"); err != nil {
		t.Fatalf("write: %v", err)
	}
	body, err := os.ReadFile(filepath.Join(d.Path, "ada.key"))
	if err != nil {
		t.Fatal(err)
	}
	var pair string
	for _, line := range strings.Split(string(body), "\n") {
		if line != "" && !strings.HasPrefix(line, "#") {
			pair = line
		}
	}
	if pair != "ada:op_deadbeef" {
		t.Errorf("payload line = %q, want %q", pair, "ada:op_deadbeef")
	}
	if !strings.Contains(string(body), "managed by goat-id-console") {
		t.Error("the file must say what manages it, or somebody will hand-edit it")
	}
}

// Revoking is a delete, and the desired end state of deleting something that is
// already gone has been reached. Treating that as an error would make a retried
// revoke look like a failure.
func TestRemove_isIdempotent(t *testing.T) {
	d := Dir{Path: t.TempDir()}
	if err := d.Write("ada", "op_x"); err != nil {
		t.Fatal(err)
	}
	if err := d.Remove("ada"); err != nil {
		t.Fatalf("first remove: %v", err)
	}
	if err := d.Remove("ada"); err != nil {
		t.Errorf("removing an absent key must succeed, got %v", err)
	}
}

// A handle reaches this code from a session. Building a path from any external
// string without neutralizing separators is one traversal away from writing
// outside the directory.
func TestWrite_handleCannotEscapeTheDirectory(t *testing.T) {
	root := t.TempDir()
	d := Dir{Path: filepath.Join(root, "keys")}
	if err := d.Write("../../etc/evil", "op_x"); err != nil {
		t.Fatalf("write: %v", err)
	}
	if _, err := os.Stat(filepath.Join(root, "etc")); !os.IsNotExist(err) {
		t.Fatal("a handle escaped the key directory")
	}
	entries, err := os.ReadDir(d.Path)
	if err != nil || len(entries) != 1 {
		t.Fatalf("expected exactly one contained file, got %v (err %v)", entries, err)
	}
}

// Reconcile splits three ways because the three cases call for different
// actions, and the version that did not caused a real near-miss: run against a
// live goatnet it classified the onboard-runner's key and two pre-registry
// human keys as "unexplained grants" and offered to delete them.
//
//	ada    — active row + file      → agrees
//	ghost  — row we revoked + file  → STALE, the record says access was removed
//	runner — file, no row at all    → not ours, report only
//	bob    — active row, no file    → holds a key the portal refuses
func TestReconcile_deletesOnlyWhatThisConsoleIssued(t *testing.T) {
	d := Dir{Path: t.TempDir()}
	for _, h := range []string{"ada", "ghost", "runner"} {
		if err := d.Write(h, "op_"+h); err != nil {
			t.Fatal(err)
		}
	}
	// ghost was issued and then revoked (RevokedHandles lists only those).
	store := fakeRevoked{fakeActive{
		active: []string{"ada", "bob"},
		known:  []string{"ada", "bob", "ghost"}, // runner was never requested here
	}, []string{"ghost"}}

	res, err := d.Reconcile(context.Background(), store, "zagros", false)
	if err != nil {
		t.Fatalf("reconcile: %v", err)
	}
	if strings.Join(res.Stale, ",") != "ghost" {
		t.Errorf("stale = %v, want [ghost]", res.Stale)
	}
	if strings.Join(res.Unmanaged, ",") != "runner" {
		t.Errorf("unmanaged = %v, want [runner]", res.Unmanaged)
	}
	if strings.Join(res.Missing, ",") != "bob" {
		t.Errorf("missing = %v, want [bob]", res.Missing)
	}
	// Dry run must not have touched anything.
	if _, err := os.Stat(filepath.Join(d.Path, "ghost.key")); err != nil {
		t.Error("a dry-run reconcile removed a file")
	}

	if _, err := d.Reconcile(context.Background(), store, "zagros", true); err != nil {
		t.Fatalf("apply: %v", err)
	}
	if _, err := os.Stat(filepath.Join(d.Path, "ghost.key")); !os.IsNotExist(err) {
		t.Error("apply must remove a grant this console revoked")
	}
	// THE POINT: a key this console never issued survives --apply. Deleting it
	// would break a service whose own deploy wrote it.
	if _, err := os.Stat(filepath.Join(d.Path, "runner.key")); err != nil {
		t.Error("apply deleted a key this console does not own")
	}
	if _, err := os.Stat(filepath.Join(d.Path, "ada.key")); err != nil {
		t.Error("apply must leave an active grant alone")
	}
}

// A goatnet that has not configured the directory must degrade to the old
// behavior rather than failing every approval.
func TestUnmanagedDirIsANoOp(t *testing.T) {
	var d Dir
	if d.Managed() {
		t.Fatal("empty path must not report as managed")
	}
	if err := d.Write("ada", "op_x"); err != nil {
		t.Errorf("write on unmanaged dir: %v", err)
	}
	if err := d.Remove("ada"); err != nil {
		t.Errorf("remove on unmanaged dir: %v", err)
	}
	if h, err := d.Handles(); err != nil || h != nil {
		t.Errorf("handles on unmanaged dir = %v, %v", h, err)
	}
}

type fakeRevoked struct {
	fakeActive
	revoked []string
}

func (f fakeRevoked) RevokedHandles(context.Context, string) ([]string, error) {
	return f.revoked, nil
}

// GHSA-2w7v. Revoke writes a tombstone naming the handle (the portal refuses
// an env-pinned key for it) and removes the key file. The tombstone body is
// exactly what the portal reads: one handle per non-comment line.
func TestRevoke_writesTombstoneThenRemovesFile(t *testing.T) {
	d := Dir{Path: t.TempDir()}
	if err := d.Write("ada", "op_secret"); err != nil {
		t.Fatal(err)
	}
	if err := d.Revoke("ada"); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(filepath.Join(d.Path, "ada.key")); !os.IsNotExist(err) {
		t.Errorf("key file still present after revoke: %v", err)
	}
	body, err := os.ReadFile(d.tombstoneFor("ada"))
	if err != nil {
		t.Fatalf("no tombstone: %v", err)
	}
	var handles []string
	for _, l := range strings.Split(string(body), "\n") {
		l = strings.TrimSpace(l)
		if l != "" && !strings.HasPrefix(l, "#") {
			handles = append(handles, l)
		}
	}
	if strings.Join(handles, ",") != "ada" {
		t.Errorf("tombstone names %v, want [ada]", handles)
	}
	if ok, err := d.Tombstoned("ada"); err != nil || !ok {
		t.Errorf("Tombstoned(ada) = %v, %v", ok, err)
	}
	if hs, _ := d.Handles(); len(hs) != 0 {
		t.Errorf("Handles() must not list a tombstone: %v", hs)
	}
	// A re-issue keeps the tombstone: the pinned env key stays refused.
	if err := d.Write("ada", "op_new"); err != nil {
		t.Fatal(err)
	}
	if ok, _ := d.Tombstoned("ada"); !ok {
		t.Error("re-issue removed the tombstone")
	}
}

// A handle that could inject another line into the tombstone is refused.
func TestRevoke_refusesHandleThatCouldNameAnother(t *testing.T) {
	d := Dir{Path: t.TempDir()}
	if err := d.Revoke("ada\nbreakglass"); err == nil {
		t.Error("a multi-line handle must be refused")
	}
}

// Reconcile reports revoked handles that have no tombstone and --apply
// writes one; an active handle is never reported.
func TestReconcile_writesMissingTombstones(t *testing.T) {
	d := Dir{Path: t.TempDir()}
	lister := fakeRevoked{fakeActive{active: []string{"cy"}, known: []string{"ada", "bob", "cy"}}, []string{"ada", "bob", "cy"}}
	if err := d.Revoke("bob"); err != nil {
		t.Fatal(err)
	}
	res, err := d.Reconcile(context.Background(), lister, "", false)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Join(res.Untombstoned, ",") != "ada" {
		t.Fatalf("Untombstoned = %v, want [ada]", res.Untombstoned)
	}
	if ok, _ := d.Tombstoned("ada"); ok {
		t.Fatal("dry run wrote a tombstone")
	}
	if _, err := d.Reconcile(context.Background(), lister, "", true); err != nil {
		t.Fatal(err)
	}
	if ok, _ := d.Tombstoned("ada"); !ok {
		t.Error("apply did not write the tombstone")
	}
}

// GHSA-2w7v review: a never-issued row's revoke writes no tombstone, and a
// legacy issued handle the tombstone rule refuses still loses its key file,
// with the missing tombstone said plainly.
func TestRevokeIssued_onlyIssuedRowsTombstoneAndLegacyHandlesStillRevoke(t *testing.T) {
	d := Dir{Path: t.TempDir()}
	if note, err := d.RevokeIssued("breakglass", false); err != nil || note != "" {
		t.Fatalf("RevokeIssued(not issued) = %q, %v", note, err)
	}
	if ok, _ := d.Tombstoned("breakglass"); ok {
		t.Fatal("a never-issued row wrote a tombstone")
	}

	legacy := filepath.Join(d.Path, "pat liaison.key")
	if err := os.WriteFile(legacy, []byte("pat liaison:op_x\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	note, err := d.RevokeIssued("pat liaison", true)
	if err != nil {
		t.Fatalf("legacy revoke failed: %v", err)
	}
	if _, serr := os.Stat(legacy); !os.IsNotExist(serr) {
		t.Errorf("legacy key file still present: %v", serr)
	}
	if !strings.Contains(note, "NO revocation tombstone") {
		t.Errorf("note = %q, want the missing tombstone said plainly", note)
	}
}

// A handle ending in ".tmp" gets a tombstone under the name the portal reads,
// and the temp name never starts with RevokedPrefix.
func TestRevoke_tmpSuffixHandleGetsAReadableTombstone(t *testing.T) {
	d := Dir{Path: t.TempDir()}
	if err := d.Revoke("foo.tmp"); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(d.tombstoneFor("foo.tmp")); err != nil {
		t.Fatalf("tombstone missing: %v", err)
	}
	entries, _ := os.ReadDir(d.Path)
	for _, e := range entries {
		if strings.HasPrefix(e.Name(), tombstoneTempPrefix) {
			t.Errorf("temp tombstone left behind: %s", e.Name())
		}
	}
	if !strings.HasPrefix(RevokedPrefix, ".") || strings.HasPrefix(tombstoneTempPrefix, RevokedPrefix) {
		t.Error("temp prefix collides with the tombstone prefix")
	}
}

// GHSA-2w7v review r2, finding 3 (fail-to-pass): two handles that used to map
// to one tombstone file name ("ops/ada" and "ops_ada") no longer can: '/', '\'
// and '..' are refused, and tombstones are named by the hex of the handle.
func TestRevoke_tombstoneNamesCannotCollide(t *testing.T) {
	for _, h := range []string{"ops/ada", `ops\ada`, "ops..ada"} {
		if CheckHandle(h) == nil {
			t.Errorf("CheckHandle(%q) accepted a handle that could collide", h)
		}
	}
	d := Dir{Path: t.TempDir()}
	if d.tombstoneFor("ops_ada") == d.tombstoneFor("ops-ada") || d.tombstoneFor("Ada") == d.tombstoneFor("ada") {
		t.Fatal("distinct handles share a tombstone file name")
	}
	if err := d.Revoke("ops_ada"); err != nil {
		t.Fatal(err)
	}
	if err := d.Revoke("ops-ada"); err != nil {
		t.Fatal(err)
	}
	for _, h := range []string{"ops_ada", "ops-ada"} {
		if ok, _ := d.Tombstoned(h); !ok {
			t.Errorf("tombstone for %s lost", h)
		}
	}
}

// GHSA-2w7v review r2, finding 5 (fail-to-pass): withdrawing a never-issued
// request, or reconciling a handle known only from such a row, never deletes a
// key file under that handle (deploy roles write service keys as <handle>.key).
func TestNeverIssuedRowsNeverDeleteAKeyFile(t *testing.T) {
	d := Dir{Path: t.TempDir()}
	if err := d.Write("grant-keyd", "op_svc"); err != nil {
		t.Fatal(err)
	}
	if _, err := d.RevokeIssued("grant-keyd", false); err != nil {
		t.Fatal(err)
	}
	lister := fakeRevoked{fakeActive{known: []string{"grant-keyd"}}, nil}
	res, err := d.Reconcile(context.Background(), lister, "", true)
	if err != nil {
		t.Fatal(err)
	}
	if len(res.Stale) != 0 {
		t.Errorf("Stale = %v, want none for a never-issued row", res.Stale)
	}
	if _, err := os.Stat(d.fileFor("grant-keyd")); err != nil {
		t.Fatalf("service key file deleted: %v", err)
	}
}

// GHSA-2w7v (stacking with callers that revoke through Remove, such as
// offboard): Remove of an issued key writes the tombstone before it deletes
// the file; if the tombstone cannot be placed, the file is kept and the error
// returned; a legacy handle a tombstone cannot name still loses its file.
// WithdrawUnissued touches nothing.
func TestRemove_tombstonesFirstAndWithdrawTouchesNothing(t *testing.T) {
	d := Dir{Path: t.TempDir()}
	if err := d.Write("ada", "op_a"); err != nil {
		t.Fatal(err)
	}
	if err := d.Remove("ada"); err != nil {
		t.Fatal(err)
	}
	if ok, _ := d.Tombstoned("ada"); !ok {
		t.Error("Remove did not tombstone an issued handle")
	}
	if _, err := os.Stat(d.fileFor("ada")); !os.IsNotExist(err) {
		t.Errorf("key file kept: %v", err)
	}

	// Tombstone cannot be placed (a directory sits at its path): file kept.
	if err := d.Write("bob", "op_b"); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(filepath.Join(d.tombstoneFor("bob"), "x"), 0o750); err != nil {
		t.Fatal(err)
	}
	if err := d.Remove("bob"); err == nil {
		t.Error("Remove reported success without a tombstone")
	}
	if _, err := os.Stat(d.fileFor("bob")); err != nil {
		t.Errorf("key file deleted although the tombstone failed: %v", err)
	}

	legacy := filepath.Join(d.Path, "pat liaison.key")
	if err := os.WriteFile(legacy, []byte("pat liaison:op_x\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := d.Remove("pat liaison"); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(legacy); !os.IsNotExist(err) {
		t.Errorf("legacy key file kept: %v", err)
	}

	if err := d.Write("grant-keyd", "op_svc"); err != nil {
		t.Fatal(err)
	}
	if err := d.WithdrawUnissued("grant-keyd"); err != nil {
		t.Fatal(err)
	}
	if ok, _ := d.Tombstoned("grant-keyd"); ok {
		t.Error("WithdrawUnissued wrote a tombstone")
	}
	if _, err := os.Stat(d.fileFor("grant-keyd")); err != nil {
		t.Errorf("WithdrawUnissued deleted the key file: %v", err)
	}
}

// GHSA-2w7v review r3, finding 2 (fail-to-pass): a handle starting with '.'
// (".revoked-x" would make "<dir>/.revoked-x.key" look like a tombstone) is
// refused.
func TestCheckHandle_refusesLeadingDot(t *testing.T) {
	for _, h := range []string{".revoked-x", ".tmp-revoked-x", ".ada"} {
		if CheckHandle(h) == nil {
			t.Errorf("CheckHandle(%q) accepted a dotted handle", h)
		}
	}
	if CheckHandle("ada.x") != nil {
		t.Error("an inner '.' must stay allowed")
	}
}
