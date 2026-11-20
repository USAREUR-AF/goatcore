package config

import (
	"fmt"
	"os"
	"path/filepath"
)

// CanonicalizeVendorBundleDir replaces VendorBundleDir with its canonical
// form: created if missing (mode 0750, as certmint would), made absolute, and
// every symlink resolved (GHSA-qc75). Bundle paths are stored as minted from
// this directory, and the bundle sweep compares them against it; a relative
// or symlinked spelling would make a live row look as if it named nothing. An
// error means the directory cannot be resolved, and the portal must not start.
func (c *Config) CanonicalizeVendorBundleDir() error {
	if c.VendorBundleDir == "" {
		return nil
	}
	if err := os.MkdirAll(c.VendorBundleDir, 0o750); err != nil {
		return fmt.Errorf("vendor bundle dir %q: %w", c.VendorBundleDir, err)
	}
	abs, err := filepath.Abs(c.VendorBundleDir)
	if err != nil {
		return fmt.Errorf("vendor bundle dir %q: %w", c.VendorBundleDir, err)
	}
	real, err := filepath.EvalSymlinks(abs)
	if err != nil {
		return fmt.Errorf("vendor bundle dir %q: %w", c.VendorBundleDir, err)
	}
	if fi, err := os.Stat(real); err != nil || !fi.IsDir() {
		return fmt.Errorf("vendor bundle dir %q is not a directory", c.VendorBundleDir)
	}
	c.VendorBundleDir = real
	return nil
}
