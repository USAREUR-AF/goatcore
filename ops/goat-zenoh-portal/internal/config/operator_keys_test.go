package config

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// parseOperatorAPIKeys accepts the documented "handle:key,handle:key" form,
// trims whitespace, and ignores blank comma-entries.
func TestParseOperatorAPIKeys_Valid(t *testing.T) {
	got, err := parseOperatorAPIKeys(" jane-operator : k1 , alice-op:k2 , ")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	want := map[string]string{"jane-operator": "k1", "alice-op": "k2"}
	if len(got) != len(want) {
		t.Fatalf("got %d keys, want %d: %#v", len(got), len(want), got)
	}
	for h, k := range want {
		if got[h] != k {
			t.Errorf("handle %q = %q, want %q", h, got[h], k)
		}
	}
}

// An empty raw value yields an empty (non-nil) map and no error.
func TestParseOperatorAPIKeys_Empty(t *testing.T) {
	got, err := parseOperatorAPIKeys("")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(got) != 0 {
		t.Errorf("expected empty map, got %#v", got)
	}
}

// Malformed pairs (no colon, two colons, empty handle/key) are rejected.
func TestParseOperatorAPIKeys_Malformed(t *testing.T) {
	for _, raw := range []string{
		"nocolonkey",  // no colon
		"a:b:c",       // two colons
		":onlykey",    // empty handle
		"onlyhandle:", // empty key
		"ok:k,bad",    // second entry malformed
	} {
		if _, err := parseOperatorAPIKeys(raw); err == nil {
			t.Errorf("raw %q: expected error, got none", raw)
		}
	}
}

func TestParseOperatorAPIKeys_ErrorNeverContainsValue(t *testing.T) {
	const marker = "TEST_ONLY_KEY_MUST_NOT_APPEAR"
	cases := []struct{ raw, entry string }{
		{marker, "entry 1"},
		{":" + marker, "entry 1"},
		{"alice:" + marker + ":bad", "entry 1"},
		{"good:key," + marker, "entry 2"},
	}
	for _, tc := range cases {
		_, err := parseOperatorAPIKeys(tc.raw)
		if err == nil {
			t.Fatal("expected malformed-pair error")
		}
		if strings.Contains(err.Error(), marker) || !strings.HasPrefix(err.Error(), "operator-api-keys "+tc.entry+" ") {
			t.Fatal("error must identify the entry without including a key value")
		}
	}
}

// Validate passes with only the per-person map set (no legacy single key).
func TestValidate_OperatorKeysMapSatisfiesRequirement(t *testing.T) {
	c := baseValidConfig()
	c.OperatorAPIKey = ""
	c.OperatorAPIKeys = map[string]string{"jane-operator": "k1"}
	if err := c.Validate(); err != nil {
		t.Fatalf("validate with map-only operator keys: %v", err)
	}
}

// Validate still passes with only the legacy single key (backward compat).
func TestValidate_LegacySingleKeySatisfiesRequirement(t *testing.T) {
	c := baseValidConfig()
	c.OperatorAPIKey = "legacy"
	c.OperatorAPIKeys = nil
	if err := c.Validate(); err != nil {
		t.Fatalf("validate with legacy single key: %v", err)
	}
}

// Validate fails when neither operator-key form is configured.
func TestValidate_NoOperatorKeyIsError(t *testing.T) {
	c := baseValidConfig()
	c.OperatorAPIKey = ""
	c.OperatorAPIKeys = nil
	if err := c.Validate(); err == nil {
		t.Fatal("expected error when no operator key configured")
	}
}

// LoadFromFlags merges the env-provided per-person map (single key omitted).
func TestLoadFromFlags_ParsesOperatorAPIKeys(t *testing.T) {
	t.Setenv("PORTAL_OPERATOR_API_KEYS", "jane-operator:k1,alice-op:k2")
	cfg, err := LoadFromFlags(nil)
	if err != nil {
		t.Fatalf("LoadFromFlags: %v", err)
	}
	if cfg.OperatorAPIKeys["jane-operator"] != "k1" || cfg.OperatorAPIKeys["alice-op"] != "k2" {
		t.Errorf("operator keys not parsed: %#v", cfg.OperatorAPIKeys)
	}
}

// baseValidConfig returns a Config that passes Validate so per-field tests can
// flip one knob at a time.
func baseValidConfig() *Config {
	return &Config{
		Listen:              "127.0.0.1:8090",
		DBDSN:               "postgres://portal@127.0.0.1/portal_registry",
		IdentityModel:       IdentityModelPerVendor,
		ZenohRestFake:       true,
		ACLConfigPath:       "/tmp/config.json5",
		ACLStorageDirPrefix: "/tmp/rocksdb",
		ACLReloadFake:       true,
		CertSignFake:        true,
		OperatorAPIKey:      "op-key",
	}
}

// mergeOperatorKeysFromDir reads service keys from files in a dir, merges them
// into the map (env wins on conflict), ignores comments/blanks, errors on a
// malformed line, and is a no-op for an empty/missing dir.
func TestMergeOperatorKeysFromDir(t *testing.T) {
	dir := t.TempDir()
	writeFile := func(name, body string) {
		if err := os.WriteFile(filepath.Join(dir, name), []byte(body), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	writeFile("onboard-runner.key", "# the runner's own key\nonboard-runner-galaxy:svc-key-1\n")
	writeFile("other.key", "another-svc:svc-key-2\n\n")

	// env already carries a human key AND a colliding handle — env must win.
	m := map[string]string{"jane-operator": "human-key", "another-svc": "ENV-WINS"}
	if err := mergeOperatorKeysFromDir(dir, m); err != nil {
		t.Fatalf("merge: %v", err)
	}
	if m["onboard-runner-galaxy"] != "svc-key-1" {
		t.Errorf("service key not merged: %q", m["onboard-runner-galaxy"])
	}
	if m["jane-operator"] != "human-key" {
		t.Errorf("env human key clobbered: %q", m["jane-operator"])
	}
	if m["another-svc"] != "ENV-WINS" {
		t.Errorf("env should win on conflict, got %q", m["another-svc"])
	}

	// Empty path + missing dir are no-ops.
	base := map[string]string{"x": "y"}
	if err := mergeOperatorKeysFromDir("", base); err != nil || len(base) != 1 {
		t.Errorf("empty dir should be a no-op: err=%v len=%d", err, len(base))
	}
	if err := mergeOperatorKeysFromDir(filepath.Join(dir, "does-not-exist"), base); err != nil {
		t.Errorf("missing dir should be a no-op, got %v", err)
	}

	// A malformed line is a hard error (a typo can't silently drop auth).
	writeFile("bad.key", "no-colon-here\n")
	if err := mergeOperatorKeysFromDir(dir, map[string]string{}); err == nil {
		t.Error("expected error on malformed line")
	}
}
