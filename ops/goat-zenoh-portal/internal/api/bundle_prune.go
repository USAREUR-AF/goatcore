package api

import (
	"context"
	"log/slog"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/certmint"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/config"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

// BundlePruneInterval is how often RunBundlePrune sweeps VendorBundleDir.
const BundlePruneInterval = 15 * time.Minute

// PruneExpiredBundles sweeps VendorBundleDir for tarballs nothing serves any
// more (GHSA-qc75). A tarball holds an org's private key. A file is removed
// when all of these hold:
//   - it is a regular *.tar.gz directly inside Abs(VendorBundleDir) (a
//     symlink, a directory or any other name is left alone);
//   - its mtime is older than BundleTTL, so a mint in progress, which writes
//     the file before the registry names it, is never touched;
//   - no vendor or slot row with a link still inside BundleTTL names it,
//     checked again right before the removal.
//
// So it removes both a tarball whose row's link has expired and one no row
// names at all (re-pointed by a rotate, a re-claimed slot, a delete, a mint
// whose registry write failed, a removal the after-commit fast path missed).
//
// It also removes a mint's stage directory (.stage-<prefix>-<stamp>, which
// holds the leaf private key and the plaintext api key) that a mint killed
// mid-way left behind: a real directory (not a symlink) directly inside
// Abs(VendorBundleDir), named exactly as certmint names it, older than
// BundleTTL. It returns how many files and stage directories it removed.
func (s *Server) PruneExpiredBundles(ctx context.Context, now time.Time) (int, error) {
	if s.cfg.VendorBundleDir == "" {
		return 0, nil
	}
	dir := registry.CanonicalDir(s.cfg.VendorBundleDir)
	ttl := s.cfg.BundleTTL
	if ttl <= 0 {
		ttl = config.DefaultBundleTTL
	}
	cutoff := now.Add(-ttl)
	entries, err := os.ReadDir(dir)
	if err != nil {
		if os.IsNotExist(err) {
			return 0, nil
		}
		return 0, err
	}
	removed := 0
	for _, e := range entries {
		if e.IsDir() && certmint.IsStageDirName(e.Name()) {
			if removeStaleStageDir(dir, e.Name(), cutoff) {
				removed++
			}
			continue
		}
		if !e.Type().IsRegular() || !strings.HasSuffix(e.Name(), ".tar.gz") {
			continue
		}
		info, err := e.Info() // lstat: a symlink was already skipped above
		if err != nil || !info.ModTime().Before(cutoff) {
			continue
		}
		p := filepath.Join(dir, e.Name())
		live, err := s.reg.BundlePathLive(ctx, p, cutoff.Unix())
		if err != nil {
			return removed, err
		}
		if live {
			continue
		}
		ok, err := registry.RemoveBundleFile(dir, p)
		if err != nil {
			slog.Warn("bundle prune: remove failed", "file", e.Name(), "err", err)
			continue
		}
		if ok {
			removed++
		}
	}
	return removed, nil
}

// RunBundlePrune sweeps in the background, once at start and then every
// interval until ctx ends (interval<=0: the first sweep only). Errors are
// logged; they never stop the portal.
func (s *Server) RunBundlePrune(ctx context.Context, interval time.Duration) {
	run := func() {
		n, err := s.PruneExpiredBundles(ctx, time.Now())
		if err != nil {
			slog.Error("bundle prune failed", "err", err)
			return
		}
		if n > 0 {
			slog.Info("bundle prune: removed expired bundle tarballs", "count", n)
		}
	}
	go func() {
		run()
		if interval <= 0 {
			return
		}
		t := time.NewTicker(interval)
		defer t.Stop()
		for {
			select {
			case <-ctx.Done():
				return
			case <-t.C:
				run()
			}
		}
	}()
}

// removeStaleStageDir removes dir/name if, checked again with Lstat right
// before, it is still a real directory (not a symlink) older than cutoff.
func removeStaleStageDir(dir, name string, cutoff time.Time) bool {
	p := filepath.Join(dir, name)
	info, err := os.Lstat(p)
	if err != nil || !info.IsDir() || info.Mode()&os.ModeSymlink != 0 || !info.ModTime().Before(cutoff) {
		return false
	}
	if err := os.RemoveAll(p); err != nil {
		slog.Warn("bundle prune: stage dir remove failed", "dir", name, "err", err)
		return false
	}
	return true
}
