package config

import (
	"os"
	"path/filepath"
	"testing"
)

// GHSA-2w7v: a person holds a legacy env-pinned key for handle ada,
// is later issued a console key for the same handle, and is then revoked in
// the console (the console removes <dir>/ada). Security property: after the
// revoke, either no key for ada is accepted, or the store still reports that
// the revocation was incomplete.
func TestOperatorKeyStore_dirRemovalOfPinnedHandleIsNeverSilent(t *testing.T) {
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "ada"), []byte("ada:op_console_ada\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	s, err := NewOperatorKeyStore(map[string]string{"ada": "op_legacy_env_ada"}, dir)
	if err != nil {
		t.Fatal(err)
	}
	t.Logf("before revoke: Match(env)=%q Match(console)=%q Shadowed=%v",
		s.Match([]byte("op_legacy_env_ada")), s.Match([]byte("op_console_ada")), s.Shadowed())
	if err := os.Remove(filepath.Join(dir, "ada")); err != nil { // console revoke
		t.Fatal(err)
	}
	if _, err := s.Reload(); err != nil {
		t.Fatal(err)
	}
	got := s.Match([]byte("op_legacy_env_ada"))
	t.Logf("after revoke:  Match(env)=%q Shadowed=%v Handles=%v", got, s.Shadowed(), s.Handles())
	if got == "ada" && len(s.Shadowed()) == 0 {
		t.Errorf("revoked handle ada is still accepted via its env key and nothing reports the incomplete revocation")
	}
}
