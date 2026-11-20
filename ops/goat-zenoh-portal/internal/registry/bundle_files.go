package registry

import (
	"context"
	"errors"
	"io/fs"
	"log/slog"
	"os"
	"path/filepath"
	"strings"
)

// SetBundleDir names the directory minted bundle tarballs live in
// (config VendorBundleDir). With it set, a write that stops a row naming a
// tarball (MarkBundle, StampSlotCert re-pointing it, DeleteVendor,
// ClearSlotCert) removes that tarball once no other row names it: it holds a
// private key and nothing serves it any more (GHSA-qc75). Empty (the default)
// leaves files alone. Call it once at startup, before the registry is shared,
// with the canonical directory (config CanonicalizeVendorBundleDir).
func (r *Registry) SetBundleDir(dir string) { r.bundleDir = dir }

// CanonicalDir is dir made absolute with every symlink resolved; it falls
// back to the cleaned absolute path when dir cannot be resolved.
func CanonicalDir(dir string) string {
	abs, err := filepath.Abs(dir)
	if err != nil {
		return filepath.Clean(dir)
	}
	if real, err := filepath.EvalSymlinks(abs); err == nil {
		return real
	}
	return abs
}

// CanonicalPath is p with its directory canonicalised (CanonicalDir) and its
// base name kept: the file itself may already be gone. A stored bundle_path
// and a path the sweep builds from VendorBundleDir compare equal under it
// whatever relative spelling or symlinked directory either was written with.
func CanonicalPath(p string) string {
	return filepath.Join(CanonicalDir(filepath.Dir(p)), filepath.Base(p))
}

// RemoveBundleFile removes the tarball at p if, and only if, it is a *.tar.gz
// directly inside dir (both canonicalised). It reports whether a file was
// removed; a file that is already gone is not an error.
func RemoveBundleFile(dir, p string) (bool, error) {
	if dir == "" || p == "" {
		return false, nil
	}
	abs := CanonicalPath(p)
	if filepath.Dir(abs) != CanonicalDir(dir) || !strings.HasSuffix(abs, ".tar.gz") {
		return false, nil
	}
	if err := os.Remove(abs); err != nil {
		if errors.Is(err, fs.ErrNotExist) {
			return false, nil
		}
		return false, err
	}
	return true, nil
}

// BundlePathLive reports whether a vendor or slot row whose link is still
// within its lifetime (bundle_minted_at at or after cutoff, Unix seconds)
// names path. The directory sweep asks it right before each removal.
func (r *Registry) BundlePathLive(ctx context.Context, path string, cutoff int64) (bool, error) {
	return r.bundlePathNamed(ctx, path, &cutoff)
}

// bundlePathNamed reports whether a row names path, comparing canonical forms
// (CanonicalPath) on both sides, so a relative or symlinked VendorBundleDir,
// in the config or in a stored bundle_path, cannot make a row look as if it
// names nothing. A stored path whose directory no longer resolves (a moved
// directory) counts as naming path when its base name is the same: unsure
// means keep. With cutoff set, only rows minted at or after it count.
func (r *Registry) bundlePathNamed(ctx context.Context, path string, cutoff *int64) (bool, error) {
	want := CanonicalPath(path)
	name := filepath.Base(want)
	like := "%" + strings.NewReplacer(`\`, `\\`, `%`, `\%`, `_`, `\_`).Replace(name)
	q := `SELECT bundle_path FROM vendors WHERE bundle_path LIKE $1
	      UNION ALL SELECT bundle_path FROM slots WHERE bundle_path LIKE $1`
	args := []any{like}
	if cutoff != nil {
		q = `SELECT bundle_path FROM vendors WHERE bundle_path LIKE $1 AND bundle_minted_at >= $2
		     UNION ALL SELECT bundle_path FROM slots WHERE bundle_path LIKE $1 AND bundle_minted_at >= $2`
		args = append(args, *cutoff)
	}
	var stored []string
	if err := r.reader().SelectContext(ctx, &stored, q, args...); err != nil {
		return false, err
	}
	for _, s := range stored {
		if filepath.Base(s) != name {
			continue
		}
		if CanonicalPath(s) == want {
			return true, nil
		}
		if _, err := filepath.EvalSymlinks(filepath.Dir(s)); err != nil {
			return true, nil // its directory is gone or unreadable: keep
		}
	}
	return false, nil
}

// dropBundleIfUnnamed is called after a committed write that stopped a row
// naming old: the fast path. It removes the file when no vendor or slot row
// names it any more (a sibling row may still share it). It runs on a context
// the request cannot cancel. Failures are logged, not returned: the registry
// write has already succeeded. A file this misses (an error, a crash before
// it ran, a concurrent write it could not see) is left to the directory sweep
// (api PruneExpiredBundles), which removes it once it is older than the TTL.
func (r *Registry) dropBundleIfUnnamed(ctx context.Context, old, current string) {
	if r.bundleDir == "" || old == "" || old == current {
		return
	}
	ctx = context.WithoutCancel(ctx)
	named, err := r.bundlePathNamed(ctx, old, nil)
	if err != nil {
		slog.Warn("bundle cleanup: lookup failed; tarball kept", "file", filepath.Base(old), "err", err)
		return
	}
	if named {
		return
	}
	if _, err := RemoveBundleFile(r.bundleDir, old); err != nil {
		slog.Warn("bundle cleanup: remove failed", "file", filepath.Base(old), "err", err)
	}
}
