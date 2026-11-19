// Package opkeys writes the on-host operator-key files the portal reads.
//
// The portal re-reads its key directory while running, so this package is the
// whole delivery mechanism: writing a file grants, removing one revokes, and
// the portal notices within one reload interval. Nothing restarts, and nobody
// edits an envrc on a laptop.
//
// The console is the record; the directory is the enforcement point. They can
// drift — a file left behind after a database restore, a write that failed
// while a row said active — so Reconcile exists to make the directory match
// the record and to say what it changed.
package opkeys

import (
	"context"
	"encoding/hex"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
)

// Dir is a portal operator-key directory.
type Dir struct {
	Path string
	// Owner/Group are the numeric ids the portal container runs as (65532 for
	// the Chainguard base). A file the portal cannot read is a key that
	// silently does not work, which presents as "my key was approved and the
	// Portal still refuses me".
	UID, GID int
}

// Managed reports whether a directory is configured. An unconfigured Dir makes
// every operation a no-op rather than an error, so a goatnet that has not
// enabled the directory tier degrades to the old behavior instead of failing
// every approval.
func (d Dir) Managed() bool { return strings.TrimSpace(d.Path) != "" }

// fileFor is the per-handle path. One file per handle rather than one shared
// file: revoking is then a delete, which cannot half-succeed the way rewriting
// a shared file can.
func (d Dir) fileFor(handle string) string {
	return filepath.Join(d.Path, safeHandle(handle)+".key")
}

// safeHandle keeps a handle from escaping the directory. A handle arrives from
// a session, but a path built from any external string without this is one
// traversal away from writing somewhere it should not.
func safeHandle(h string) string {
	h = strings.TrimSpace(h)
	h = strings.ReplaceAll(h, "/", "_")
	h = strings.ReplaceAll(h, `\`, "_")
	h = strings.ReplaceAll(h, "..", "_")
	return h
}

// Write places a key. It writes a temp file and renames, so a portal reload
// that lands mid-write sees either the old file or the new one, never half of
// one — the portal skips `.tmp` names for the same reason.
func (d Dir) Write(handle, secret string) error {
	if !d.Managed() {
		return nil
	}
	if strings.TrimSpace(handle) == "" || strings.TrimSpace(secret) == "" {
		return fmt.Errorf("operator key file needs both a handle and a key")
	}
	if err := os.MkdirAll(d.Path, 0o750); err != nil {
		return fmt.Errorf("create operator-key dir: %w", err)
	}
	final := d.fileFor(handle)
	tmp := final + ".tmp"
	body := fmt.Sprintf("# managed by goat-id-console — edit through the console, not here\n%s:%s\n",
		safeHandle(handle), secret)
	if err := os.WriteFile(tmp, []byte(body), 0o640); err != nil {
		return fmt.Errorf("write operator-key temp file: %w", err)
	}
	if d.UID > 0 {
		// Best-effort: a console running unprivileged in a container may not be
		// able to chown, and a readable-by-group file is usually enough. A hard
		// failure here would block an approval over file ownership cosmetics.
		_ = os.Chown(tmp, d.UID, d.GID) //nolint:errcheck // group-readable mode is sufficient when container chown is unavailable
	}
	if err := os.Rename(tmp, final); err != nil {
		return fmt.Errorf("place operator-key file: %w", err)
	}
	return nil
}

// Remove revokes by deletion. A missing file is success: the desired end state
// is "this key is not accepted", and it already is not.
//
// GHSA-2w7v: Remove is the revoke of a key this console ISSUED. It writes the
// handle's revocation tombstone first (atomically), so the portal also refuses
// an env-pinned key for the handle, and only then deletes the key file; if the
// tombstone cannot be written the file is kept and the error returned (the key
// is still live). A legacy handle a tombstone cannot name (CheckHandle) only
// has its file deleted; RevokeIssued reports that case to the operator.
//
// Never call Remove for a row the console never issued (a withdrawn request,
// a handle known only from a requested or rejected row): a tombstone there
// would disable an env-tier key nobody decided to revoke, and the file under
// that name is not the console's. Those paths go through WithdrawUnissued.
func (d Dir) Remove(handle string) error {
	if !d.Managed() {
		return nil
	}
	h := strings.TrimSpace(handle)
	if CheckHandle(h) == nil {
		if err := d.writeTombstone(h); err != nil {
			return err
		}
	}
	return d.removeKeyFile(handle)
}

// removeKeyFile deletes the key file only. A missing file is success.
func (d Dir) removeKeyFile(handle string) error {
	if err := os.Remove(d.fileFor(handle)); err != nil && !os.IsNotExist(err) {
		return fmt.Errorf("remove operator-key file: %w", err)
	}
	return nil
}

// WithdrawUnissued is the only directory action for a row the console never
// issued a key for: none. It never writes a tombstone and never deletes a key
// file (deploy roles name service keys <handle>.key in this directory).
func (d Dir) WithdrawUnissued(string) error { return nil }

// RevokedPrefix names the revocation tombstone the portal reads (it must
// equal the portal's config.RevokedFilePrefix). Dot-prefixed so a portal that
// predates tombstones skips it rather than failing to parse it.
const RevokedPrefix = ".revoked-"

// tombstoneTempPrefix names a tombstone being written. It cannot collide with
// a tombstone name (a handle ending in ".tmp" still gets a tombstone the
// portal reads), and as a dotfile every portal skips it.
const tombstoneTempPrefix = ".tmp-revoked-"

// tombstoneFor names a handle's tombstone by the hex of the handle, so two
// different handles can never share a tombstone file (the portal reads the
// handle from the content, not the name).
func (d Dir) tombstoneFor(handle string) string {
	return filepath.Join(d.Path, RevokedPrefix+hex.EncodeToString([]byte(strings.TrimSpace(handle))))
}

// CheckHandle is the one handle rule for a key the console may issue: a handle
// a tombstone can name on a line of its own. Applied at request time
// (onboardstore) and before writing a tombstone.
func CheckHandle(handle string) error {
	h := strings.TrimSpace(handle)
	// A leading '.' is refused: "<handle>.key" would be a dotfile the portal
	// skips, and a handle starting with a tombstone or temp prefix would name
	// a file that looks like a tombstone (GHSA-2w7v review r3).
	if h == "" || strings.HasPrefix(h, ".") || strings.ContainsAny(h, " \t\r\n#:/\\") || strings.Contains(h, "..") {
		return fmt.Errorf("operator key handle %q must be non-empty, not start with '.', and have no whitespace, '#', ':', '/', '\\' or '..'", handle)
	}
	return nil
}

// Revoke ends a handle's portal access fail-closed (GHSA-2w7v). It writes a
// tombstone naming the handle FIRST, then removes the key file. The tombstone
// makes the portal refuse an env-pinned key (PORTAL_OPERATOR_API_KEYS) for the
// same handle, which removing the file alone never did. The tombstone is never
// removed by the console: a later re-issue for the handle is a directory key
// and still works, while the old pinned key stays refused. Restoring a pinned
// key is a host action: delete the env pin, then the tombstone.
func (d Dir) Revoke(handle string) error {
	if !d.Managed() {
		return nil
	}
	h := strings.TrimSpace(handle)
	if err := CheckHandle(h); err != nil {
		return fmt.Errorf("cannot be tombstoned: %w", err)
	}
	return d.Remove(h)
}

// writeTombstone places the handle's tombstone through a temp file and a
// rename, so the portal never reads half of one.
func (d Dir) writeTombstone(h string) error {
	if err := os.MkdirAll(d.Path, 0o750); err != nil {
		return fmt.Errorf("create operator-key dir: %w", err)
	}
	final := d.tombstoneFor(h)
	tmp := filepath.Join(d.Path, tombstoneTempPrefix+hex.EncodeToString([]byte(h)))
	body := fmt.Sprintf("# revocation tombstone written by goat-id-console; the portal refuses any key for this handle from PORTAL_OPERATOR_API_KEYS\n%s\n", h)
	if err := os.WriteFile(tmp, []byte(body), 0o640); err != nil {
		return fmt.Errorf("write operator-key tombstone: %w", err)
	}
	if d.UID > 0 {
		_ = os.Chown(tmp, d.UID, d.GID) //nolint:errcheck // group-readable mode is sufficient when container chown is unavailable
	}
	if err := os.Rename(tmp, final); err != nil {
		return fmt.Errorf("place operator-key tombstone: %w", err)
	}
	return nil
}

// RevokeIssued is the console's revoke. Only a handle the console actually
// issued a key for (issued: the row was approved at some point) gets a
// tombstone: a tombstone also refuses the handle's env-tier key, so writing one
// for a never-approved request would let a single operator principal disable
// any break-glass or service key by requesting and withdrawing (GHSA-2w7v
// review). A never-issued row touches nothing on disk: a key file under its
// handle is not the console's (deploy roles name service keys <handle>.key).
//
// A legacy issued handle the tombstone rule refuses still has its key file
// removed; note then says plainly that no tombstone was written, so an env pin
// for that handle would still be accepted. err is non-nil only when the key
// file itself could not be removed (the key is still live).
func (d Dir) RevokeIssued(handle string, issued bool) (note string, err error) {
	if !issued {
		return "", d.WithdrawUnissued(handle)
	}
	if cerr := CheckHandle(handle); cerr != nil {
		if rerr := d.removeKeyFile(handle); rerr != nil {
			return "", rerr
		}
		return "key file removed, but NO revocation tombstone was written (" + cerr.Error() +
			"): a key for this handle pinned in the portal's PORTAL_OPERATOR_API_KEYS is still accepted", nil
	}
	return "", d.Revoke(handle)
}

// Tombstoned reports whether the directory holds a revocation tombstone for
// handle. An unmanaged directory reports false.
func (d Dir) Tombstoned(handle string) (bool, error) {
	if !d.Managed() {
		return false, nil
	}
	body, err := os.ReadFile(d.tombstoneFor(handle))
	if err != nil {
		if os.IsNotExist(err) {
			return false, nil
		}
		return false, fmt.Errorf("read operator-key tombstone: %w", err)
	}
	for _, line := range strings.Split(string(body), "\n") {
		if strings.TrimSpace(line) == strings.TrimSpace(handle) {
			return true, nil
		}
	}
	return false, nil
}

// Handles lists the handles the directory currently grants, sorted.
func (d Dir) Handles() ([]string, error) {
	if !d.Managed() {
		return nil, nil
	}
	entries, err := os.ReadDir(d.Path)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, fmt.Errorf("read operator-key dir: %w", err)
	}
	var out []string
	for _, e := range entries {
		n := e.Name()
		if e.IsDir() || !strings.HasSuffix(n, ".key") || strings.HasPrefix(n, ".") {
			continue
		}
		out = append(out, strings.TrimSuffix(n, ".key"))
	}
	sort.Strings(out)
	return out, nil
}

// ActiveLister is the slice of the store Reconcile needs. Kept narrow so the
// writer does not depend on the whole onboarding store.
//
// KnownHandles is what makes --apply safe. The directory also holds keys this
// console never issued: service keys a component's own ansible role writes
// (the onboard-runner, microscope) and human keys that predate the registry.
// Those have no row at all, and deleting them would be the console asserting
// ownership of credentials it did not create.
type ActiveLister interface {
	ActiveHandles(ctx context.Context, env string) ([]string, error)
	// KnownHandles is every handle this console has a row for, in ANY state.
	KnownHandles(ctx context.Context, env string) ([]string, error)
}

// RevokedLister is optional. A lister that also reports REVOKED handles lets
// Reconcile find revocations made before tombstones existed (GHSA-2w7v). It
// must list only rows the console actually issued a key for (approved at some
// point), never a withdrawn request.
type RevokedLister interface {
	RevokedHandles(ctx context.Context, env string) ([]string, error)
}

// Result is what a reconcile found, split three ways because the three cases
// call for different actions and lumping them together produced a live footgun.
type Result struct {
	// Stale: this console issued the key and has since revoked or rejected it,
	// but the file is still there. The portal accepts a key the record says was
	// taken away. This is the one --apply removes.
	Stale []string
	// Unmanaged: a file for a handle this console has no row for at all —
	// service keys written by their own ansible role, or keys that predate the
	// registry. Reported so nobody is surprised by them, never touched.
	Unmanaged []string
	// Missing: an active row with no file. That person holds a key the portal
	// refuses. Not repairable here: the secret is deliberately not retained,
	// and inventing one would hand somebody a credential nobody approved.
	Missing []string
	// Untombstoned: a handle this console revoked with no revocation
	// tombstone on disk, so an env-pinned key for it would still be accepted
	// by the portal. --apply writes the tombstone (GHSA-2w7v).
	Untombstoned []string
}

// Reconcile compares the record against the directory the portal actually
// reads, and is deliberately conservative about what it will delete.
//
// The first version treated every file without an ACTIVE row as an unexplained
// grant and offered to remove it. Run against a real goatnet that would have
// deleted the onboard-runner's key and two human keys issued before this
// registry existed — none of which the console ever created. Ownership is the
// dividing line now: a handle this console has a row for is one it may enforce;
// a handle it has never heard of belongs to somebody else and is only reported.
func (d Dir) Reconcile(ctx context.Context, s ActiveLister, env string, apply bool) (Result, error) {
	var res Result
	if !d.Managed() {
		return res, nil
	}
	active, err := s.ActiveHandles(ctx, env)
	if err != nil {
		return res, err
	}
	known, err := s.KnownHandles(ctx, env)
	if err != nil {
		return res, err
	}
	isActive := map[string]bool{}
	for _, h := range active {
		isActive[safeHandle(h)] = true
	}
	isKnown := map[string]bool{}
	for _, h := range known {
		isKnown[safeHandle(h)] = true
	}
	// Issued-and-revoked handles: the only ones whose key file this console
	// may delete. A handle known only from a requested or rejected row never
	// had a console key, and a file under it belongs to somebody else (deploy
	// roles name service keys <handle>.key), GHSA-2w7v review.
	var revoked []string
	rl, hasRevoked := s.(RevokedLister)
	if hasRevoked {
		if revoked, err = rl.RevokedHandles(ctx, env); err != nil {
			return res, err
		}
	}
	wasIssued := map[string]bool{}
	for _, h := range revoked {
		wasIssued[safeHandle(h)] = true
	}
	have, err := d.Handles()
	if err != nil {
		return res, err
	}
	hasFile := map[string]bool{}
	for _, h := range have {
		hasFile[h] = true
		switch {
		case isActive[h]:
			// agrees
		case isKnown[h] && wasIssued[h]:
			res.Stale = append(res.Stale, h)
		default:
			res.Unmanaged = append(res.Unmanaged, h)
		}
	}
	for h := range isActive {
		if !hasFile[h] {
			res.Missing = append(res.Missing, h)
		}
	}
	if hasRevoked {
		seen := map[string]bool{}
		for _, h := range revoked {
			if seen[h] || isActive[safeHandle(h)] {
				continue
			}
			seen[h] = true
			t, terr := d.Tombstoned(h)
			if terr != nil {
				return res, terr
			}
			if !t {
				res.Untombstoned = append(res.Untombstoned, h)
			}
		}
	}
	sort.Strings(res.Stale)
	sort.Strings(res.Unmanaged)
	sort.Strings(res.Missing)
	sort.Strings(res.Untombstoned)
	if apply {
		for _, h := range res.Stale {
			if rerr := d.Remove(h); rerr != nil {
				return res, rerr
			}
		}
		for _, h := range res.Untombstoned {
			if CheckHandle(h) != nil {
				continue // reported; a tombstone cannot name it (legacy handle)
			}
			if rerr := d.Revoke(h); rerr != nil {
				return res, rerr
			}
		}
	}
	return res, nil
}
