package config

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// A reserved service label in PORTAL_OPERATOR_API_KEYS is skipped, except
// id-console, whose key is provisioned there. The other entries still load.
func TestParseOperatorAPIKeys_refusesReservedServiceHandles(t *testing.T) {
	got, err := parseOperatorAPIKeys("operator:k-person,onboard-api:k2,Operator:k3,ID-Console:k4,id-console:k-console,jane:k-jane")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	want := map[string]string{"id-console": "k-console", "jane": "k-jane"}
	if len(got) != len(want) {
		t.Fatalf("got %#v, want %#v", got, want)
	}
	for h, k := range want {
		if got[h] != k {
			t.Errorf("handle %q = %q, want %q", h, got[h], k)
		}
	}
}

// No directory file can supply a service label, even one that is not pinned
// in the env tier. Other files in the same directory are still accepted.
func TestOperatorKeyStore_dirRefusesReservedServiceHandles(t *testing.T) {
	dir := t.TempDir()
	for name, body := range map[string]string{
		"operator.key":    "operator:k-dir-operator\n",
		"id-console.key":  "id-console:k-dir-console\n",
		"onboard-api.key": "onboard-api:k-dir-onboard\n",
		"sneaky.key":      "# a reserved label inside an innocuous file\nOnboard-API:k-dir-sneaky\n",
		"jane.key":        "jane:k-jane\n",
	} {
		if err := os.WriteFile(filepath.Join(dir, name), []byte(body), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	s := mustStore(t, nil, dir)
	for _, tok := range []string{"k-dir-operator", "k-dir-console", "k-dir-onboard", "k-dir-sneaky"} {
		if got := s.Match([]byte(tok)); got != "" {
			t.Errorf("dir key %s accepted as %q, want refused", tok, got)
		}
	}
	if got := s.Match([]byte("k-jane")); got != "jane" {
		t.Errorf("ordinary dir key: Match = %q, want jane", got)
	}
	if got, want := strings.Join(s.Refused(), ","), "Onboard-API,id-console,onboard-api,operator"; got != want {
		t.Errorf("Refused() = %q, want %q", got, want)
	}
	for _, h := range s.Refused() {
		if strings.HasPrefix(h, "k-") {
			t.Errorf("Refused() leaked a key: %q", h)
		}
	}
}

// The env-pinned id-console key keeps working beside a directory that tries to
// redefine it, and the directory copy is not reported as shadowed (it was
// refused before the collision mattered).
func TestOperatorKeyStore_envIDConsoleSurvivesDirAttempt(t *testing.T) {
	dir := t.TempDir()
	writeKeyFile(t, dir, "id-console.key", "id-console:k-dir-console\n")
	s := mustStore(t, map[string]string{"id-console": "k-console"}, dir)
	if got := s.Match([]byte("k-console")); got != "id-console" {
		t.Errorf("env id-console key: Match = %q, want id-console", got)
	}
	if got := s.Match([]byte("k-dir-console")); got != "" {
		t.Errorf("dir id-console key accepted as %q", got)
	}
}
