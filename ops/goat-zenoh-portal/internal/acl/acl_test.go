package acl_test

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/acl"
)

// fixtureConfig is the minimal config.json5 shape the role's template
// renders — just enough to exercise the four BEGIN/END markers the
// renderer edits between. Real config has way more (transport, plugins,
// universal-read rule, operator subject, etc.) but the markers + their
// surrounding indent are what the renderer cares about.
const fixtureConfig = `// goat-zenoh-portal: rendered config fixture for tests.
{
  mode: "router",
  access_control: {
    enabled: true,
    default_permission: "deny",
    rules: [
      // BEGIN: per-vendor write rules
      // END: per-vendor write rules
    ],
    subjects: [
      // BEGIN: per-vendor subjects
      // END: per-vendor subjects
    ],
    policies: [
      // BEGIN: per-vendor policies
      // END: per-vendor policies
    ],
  },
  plugins: {
    storage_manager: {
      storages: {
        // BEGIN: per-vendor storages
        // END: per-vendor storages
      },
    },
  },
}
`

func setupFixture(t *testing.T) (configPath, storageDir string) {
	t.Helper()
	dir := t.TempDir()
	configPath = filepath.Join(dir, "config.json5")
	if err := os.WriteFile(configPath, []byte(fixtureConfig), 0o644); err != nil {
		t.Fatalf("write fixture: %v", err)
	}
	storageDir = filepath.Join(dir, "rocksdb")
	if err := os.MkdirAll(storageDir, 0o755); err != nil {
		t.Fatalf("mkdir storage: %v", err)
	}
	return configPath, storageDir
}

func TestRender_InsertsAllFourStanzas(t *testing.T) {
	configPath, storageDir := setupFixture(t)
	r := &acl.Renderer{
		ConfigPath:       configPath,
		StorageDirPrefix: storageDir,
		ReloadFake:       true,
	}
	if err := r.Render(context.Background(), []string{"acme"}); err != nil {
		t.Fatalf("render: %v", err)
	}
	body, _ := os.ReadFile(configPath)
	s := string(body)
	for _, frag := range []string{
		`"trial-peer-w-acme"`,
		`key_exprs: ["acme/**"]`,
		`"vendor-acme"`,
		`cert_common_names: ["acme-trial-peer"]`,
		`rules: ["trial-peer-w-acme", "trial-peer-w-attest-schema"]`,
		`subjects: ["vendor-acme"]`,
		`"acme": {`,
		`key_expr: "acme/**"`,
		`dir: "` + storageDir + `/acme"`,
		`time_to_live: 604800000`,
	} {
		if !strings.Contains(s, frag) {
			t.Errorf("missing fragment %q in:\n%s", frag, s)
		}
	}
	// Storage dir was provisioned.
	if _, err := os.Stat(filepath.Join(storageDir, "acme")); err != nil {
		t.Errorf("storage dir not created: %v", err)
	}
}

func TestRender_Idempotent(t *testing.T) {
	configPath, storageDir := setupFixture(t)
	r := &acl.Renderer{
		ConfigPath:       configPath,
		StorageDirPrefix: storageDir,
		ReloadFake:       true,
	}
	if err := r.Render(context.Background(), []string{"acme"}); err != nil {
		t.Fatalf("first render: %v", err)
	}
	// Capture the file size after the first render.
	first, _ := os.ReadFile(configPath)
	if err := r.Render(context.Background(), []string{"acme"}); err != nil {
		t.Fatalf("second render: %v", err)
	}
	second, _ := os.ReadFile(configPath)
	if len(first) != len(second) {
		t.Errorf("re-render changed file size (%d → %d); should be no-op", len(first), len(second))
	}
	// The rule id appears twice per apply (once in the rule definition,
	// once in the policy's rules-ref). Idempotency means count stays
	// at 2, not 4.
	count := strings.Count(string(second), `"trial-peer-w-acme"`)
	if count != 2 {
		t.Errorf("expected 2 occurrences of trial-peer-w-acme (rule + policy ref), got %d", count)
	}
	// Subject + storage anchors are single-occurrence.
	if strings.Count(string(second), `"vendor-acme"`) != 2 {
		// 2 because the policy's subjects-ref also names it. Rule of thumb:
		// the policy block double-references both anchors.
		t.Errorf("expected 2 vendor-acme occurrences, got %d",
			strings.Count(string(second), `"vendor-acme"`))
	}
	if strings.Count(string(second), `cert_common_names: ["acme-trial-peer"]`) != 1 {
		t.Errorf("subject cert_common_names duplicated on re-render")
	}
	if strings.Count(string(second), `dir: "`+filepath.Join(filepath.Dir(configPath), "rocksdb", "acme")+`"`) > 1 {
		t.Errorf("storage stanza duplicated on re-render")
	}
}

func TestRender_MultiVendor(t *testing.T) {
	configPath, storageDir := setupFixture(t)
	r := &acl.Renderer{
		ConfigPath:       configPath,
		StorageDirPrefix: storageDir,
		ReloadFake:       true,
	}
	if err := r.Render(context.Background(), []string{"alpha", "bravo", "charlie"}); err != nil {
		t.Fatalf("render: %v", err)
	}
	body, _ := os.ReadFile(configPath)
	s := string(body)
	for _, v := range []string{"alpha", "bravo", "charlie"} {
		if !strings.Contains(s, `"trial-peer-w-`+v+`"`) {
			t.Errorf("missing rule for %s", v)
		}
		if !strings.Contains(s, `"vendor-`+v+`"`) {
			t.Errorf("missing subject for %s", v)
		}
		if _, err := os.Stat(filepath.Join(storageDir, v)); err != nil {
			t.Errorf("storage dir for %s not created: %v", v, err)
		}
	}
}

func TestRender_AtomicWrite_NoLeftoverTmp(t *testing.T) {
	configPath, storageDir := setupFixture(t)
	r := &acl.Renderer{
		ConfigPath:       configPath,
		StorageDirPrefix: storageDir,
		ReloadFake:       true,
	}
	if err := r.Render(context.Background(), []string{"x"}); err != nil {
		t.Fatalf("render: %v", err)
	}
	entries, _ := os.ReadDir(filepath.Dir(configPath))
	for _, e := range entries {
		if strings.HasPrefix(e.Name(), ".zenoh-config-") {
			t.Errorf("temp file left behind: %s", e.Name())
		}
	}
}

func TestRender_MissingMarker(t *testing.T) {
	dir := t.TempDir()
	configPath := filepath.Join(dir, "config.json5")
	// Config missing the storages marker.
	bad := strings.Replace(fixtureConfig, "        // END: per-vendor storages", "", 1)
	if err := os.WriteFile(configPath, []byte(bad), 0o644); err != nil {
		t.Fatalf("write: %v", err)
	}
	r := &acl.Renderer{
		ConfigPath:       configPath,
		StorageDirPrefix: filepath.Join(dir, "rocksdb"),
		ReloadFake:       true,
	}
	err := r.Render(context.Background(), []string{"x"})
	if err == nil || !strings.Contains(err.Error(), "missing marker") {
		t.Errorf("expected missing-marker error, got: %v", err)
	}
}

func TestRender_RequiresConfigPath(t *testing.T) {
	r := &acl.Renderer{ReloadFake: true, StorageDirPrefix: "/tmp"}
	if err := r.Render(context.Background(), []string{"x"}); err == nil {
		t.Errorf("expected error on empty ConfigPath")
	}
}

func TestRender_RequiresStorageDirPrefix(t *testing.T) {
	dir := t.TempDir()
	configPath := filepath.Join(dir, "config.json5")
	_ = os.WriteFile(configPath, []byte(fixtureConfig), 0o644)
	r := &acl.Renderer{ConfigPath: configPath, ReloadFake: true}
	if err := r.Render(context.Background(), []string{"x"}); err == nil {
		t.Errorf("expected error on empty StorageDirPrefix")
	}
}

func TestRender_RequiresContainerNameWhenNotFake(t *testing.T) {
	configPath, storageDir := setupFixture(t)
	r := &acl.Renderer{
		ConfigPath:       configPath,
		StorageDirPrefix: storageDir,
		// ReloadFake: false (zero value)
		// ContainerName: "" (zero value) — should fail
	}
	if err := r.Render(context.Background(), []string{"x"}); err == nil {
		t.Errorf("expected error when ContainerName empty and not in fake mode")
	}
}

func TestRender_EmptyVendorList_NoOp(t *testing.T) {
	configPath, storageDir := setupFixture(t)
	pre, _ := os.ReadFile(configPath)
	r := &acl.Renderer{
		ConfigPath:       configPath,
		StorageDirPrefix: storageDir,
		ReloadFake:       true,
	}
	if err := r.Render(context.Background(), []string{}); err != nil {
		t.Fatalf("render with empty list should not error: %v", err)
	}
	post, _ := os.ReadFile(configPath)
	if string(pre) != string(post) {
		t.Errorf("empty vendor list mutated config")
	}
}

// Regression for F-155: vendor prefixes containing hyphens (org-navy,
// dept-1, cpe-demo, …) must produce a parseable JSON5 storage key. The
// pre-fix generator emitted bare-identifier keys (`org-navy: {`) which
// JSON5 rejects because hyphens are not legal identifier characters,
// silently wedging every router restart after the first hyphenated vendor
// mint. Pin the quoted form so future edits to applyVendorStanzas can't
// regress the parser-safety property.
func TestRender_HyphenatedVendorEmitsQuotedStorageKey(t *testing.T) {
	for _, v := range []string{"org-navy", "dept-1", "cpe-demo"} {
		v := v
		t.Run(v, func(t *testing.T) {
			configPath, storageDir := setupFixture(t)
			r := &acl.Renderer{
				ConfigPath:       configPath,
				StorageDirPrefix: storageDir,
				ReloadFake:       true,
			}
			if err := r.Render(context.Background(), []string{v}); err != nil {
				t.Fatalf("render: %v", err)
			}
			body, _ := os.ReadFile(configPath)
			s := string(body)
			if !strings.Contains(s, `"`+v+`": {`) {
				t.Errorf("storage key for %q must be quoted; got:\n%s", v, s)
			}
			// Belt and suspenders: the unquoted form must be absent at
			// the storage-stanza indent (8 spaces). The vendor name
			// legitimately appears unquoted elsewhere (rule key_exprs,
			// storage dir path) so don't grep the whole file — just the
			// stanza-opening line shape.
			if strings.Contains(s, "        "+v+": {") {
				t.Errorf("storage key for %q emitted unquoted; would fail JSON5 parse:\n%s", v, s)
			}
		})
	}
}

func TestRender_StorageTTLOverride(t *testing.T) {
	configPath, storageDir := setupFixture(t)
	r := &acl.Renderer{
		ConfigPath:       configPath,
		StorageDirPrefix: storageDir,
		ReloadFake:       true,
		StorageTTLMs:     12345,
	}
	if err := r.Render(context.Background(), []string{"x"}); err != nil {
		t.Fatalf("render: %v", err)
	}
	body, _ := os.ReadFile(configPath)
	if !strings.Contains(string(body), "time_to_live: 12345") {
		t.Errorf("custom TTL not applied:\n%s", string(body))
	}
}

// 83F revoke-path symmetry: Render → RemoveVendor leaves the config
// byte-equivalent to the fresh fixture (modulo create-then-remove being
// an exact inverse). Pins the lockstep between applyVendorStanzas and
// removeVendorStanzas.
func TestRemoveVendor_RoundTrip(t *testing.T) {
	configPath, storageDir := setupFixture(t)
	r := &acl.Renderer{
		ConfigPath:       configPath,
		StorageDirPrefix: storageDir,
		ReloadFake:       true,
	}
	if err := r.Render(context.Background(), []string{"acme"}); err != nil {
		t.Fatalf("render: %v", err)
	}
	// Verify the per-vendor storage dir got created.
	if _, err := os.Stat(filepath.Join(storageDir, "acme")); err != nil {
		t.Fatalf("storage dir missing after render: %v", err)
	}

	if err := r.RemoveVendor(context.Background(), "acme"); err != nil {
		t.Fatalf("remove: %v", err)
	}
	body, _ := os.ReadFile(configPath)
	got := string(body)
	if got != fixtureConfig {
		t.Errorf("post-remove config != fresh fixture:\n---got---\n%s\n---want---\n%s\n", got, fixtureConfig)
	}
	// Storage dir should be gone (best-effort cleanup).
	if _, err := os.Stat(filepath.Join(storageDir, "acme")); !os.IsNotExist(err) {
		t.Errorf("storage dir for removed vendor should not exist; stat err = %v", err)
	}
}

// RemoveVendor on a vendor that was never added is a no-op + no error.
func TestRemoveVendor_NoOpWhenAbsent(t *testing.T) {
	configPath, storageDir := setupFixture(t)
	r := &acl.Renderer{
		ConfigPath:       configPath,
		StorageDirPrefix: storageDir,
		ReloadFake:       true,
	}
	if err := r.RemoveVendor(context.Background(), "ghost"); err != nil {
		t.Fatalf("remove absent: %v", err)
	}
	body, _ := os.ReadFile(configPath)
	if string(body) != fixtureConfig {
		t.Errorf("no-op remove changed config:\n%s", string(body))
	}
}

// RemoveVendor pulls only the targeted vendor, not its neighbors.
func TestRemoveVendor_LeavesOthers(t *testing.T) {
	configPath, storageDir := setupFixture(t)
	r := &acl.Renderer{
		ConfigPath:       configPath,
		StorageDirPrefix: storageDir,
		ReloadFake:       true,
	}
	if err := r.Render(context.Background(), []string{"acme", "shield-ai", "skydio"}); err != nil {
		t.Fatalf("render: %v", err)
	}
	if err := r.RemoveVendor(context.Background(), "shield-ai"); err != nil {
		t.Fatalf("remove: %v", err)
	}
	body, _ := os.ReadFile(configPath)
	got := string(body)
	if strings.Contains(got, "shield-ai") {
		t.Errorf("removed vendor still present in config:\n%s", got)
	}
	if !strings.Contains(got, "trial-peer-w-acme") || !strings.Contains(got, "trial-peer-w-skydio") {
		t.Errorf("neighboring vendors removed accidentally:\n%s", got)
	}
}

// RemoveVendor requires ConfigPath, StorageDirPrefix, vendor.
func TestRemoveVendor_RequiresFields(t *testing.T) {
	cases := []struct {
		name string
		r    *acl.Renderer
		v    string
	}{
		{"no config path", &acl.Renderer{StorageDirPrefix: "/x"}, "v"},
		{"no storage prefix", &acl.Renderer{ConfigPath: "/x"}, "v"},
		{"empty vendor", &acl.Renderer{ConfigPath: "/x", StorageDirPrefix: "/x"}, ""},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			if err := c.r.RemoveVendor(context.Background(), c.v); err == nil {
				t.Errorf("expected error from RemoveVendor for %q, got nil", c.name)
			}
		})
	}
}
