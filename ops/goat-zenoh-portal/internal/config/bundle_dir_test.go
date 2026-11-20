package config

import (
	"os"
	"path/filepath"
	"testing"
)

// A relative or symlinked VendorBundleDir is replaced by its absolute,
// symlink-free form; one that cannot be resolved is an error.
func TestCanonicalizeVendorBundleDir(t *testing.T) {
	root, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	real := filepath.Join(root, "bundles")
	if err := os.Mkdir(real, 0o750); err != nil {
		t.Fatal(err)
	}
	link := filepath.Join(root, "link")
	if err := os.Symlink(real, link); err != nil {
		t.Fatal(err)
	}
	t.Chdir(root)
	for _, in := range []string{"bundles", "./bundles/", link, "link", real} {
		c := &Config{VendorBundleDir: in}
		if err := c.CanonicalizeVendorBundleDir(); err != nil {
			t.Fatalf("%q: %v", in, err)
		}
		if c.VendorBundleDir != real {
			t.Errorf("%q -> %q, want %q", in, c.VendorBundleDir, real)
		}
	}
	// Missing: created, then canonical.
	c := &Config{VendorBundleDir: "new/sub"}
	if err := c.CanonicalizeVendorBundleDir(); err != nil || c.VendorBundleDir != filepath.Join(root, "new", "sub") {
		t.Errorf("missing dir: %q, %v", c.VendorBundleDir, err)
	}
	// A file, or a dangling symlink, cannot be the directory.
	file := filepath.Join(root, "afile")
	if err := os.WriteFile(file, nil, 0o600); err != nil {
		t.Fatal(err)
	}
	dangling := filepath.Join(root, "dangling")
	if err := os.Symlink(filepath.Join(root, "nowhere"), dangling); err != nil {
		t.Fatal(err)
	}
	for _, in := range []string{file, dangling} {
		c := &Config{VendorBundleDir: in}
		if err := c.CanonicalizeVendorBundleDir(); err == nil {
			t.Errorf("%q: no error", in)
		}
	}
}
