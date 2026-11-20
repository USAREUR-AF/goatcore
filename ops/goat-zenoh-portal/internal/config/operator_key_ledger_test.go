package config

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// GHSA-2w7v review r2, finding 2 (fail-to-pass). A revoked env key must stay
// refused when the key directory is emptied (reload) or missing (restart),
// and the loss of the tombstone must be reported, not silent.
func TestOperatorKeyLedger_revocationSurvivesEmptyAndMissingDir(t *testing.T) {
	base := t.TempDir()
	dir := filepath.Join(base, "operator-keys.d")
	if err := os.MkdirAll(dir, 0o750); err != nil {
		t.Fatal(err)
	}
	env := map[string]string{"ada": "op_legacy_env_ada", "bg": "op_bg"}
	writeKeyFile(t, dir, TombstoneFileName("ada"), "ada\n")
	s := mustStore(t, env, dir)
	if got := s.Match([]byte("op_legacy_env_ada")); got != "" {
		t.Fatalf("precondition: tombstoned env key accepted: %q", got)
	}

	// Reload with the tombstone gone (directory emptied).
	if err := os.Remove(filepath.Join(dir, TombstoneFileName("ada"))); err != nil {
		t.Fatal(err)
	}
	if _, err := s.Reload(); err != nil {
		t.Fatal(err)
	}
	if got := s.Match([]byte("op_legacy_env_ada")); got != "" {
		t.Errorf("empty dir: revoked env key accepted again: %q", got)
	}
	if got := strings.Join(s.RevokedWithoutTombstone(), ","); got != "ada" {
		t.Errorf("RevokedWithoutTombstone() = %q, want ada", got)
	}
	if got := s.Match([]byte("op_bg")); got != "bg" {
		t.Errorf("an unrevoked env key must still work, got %q", got)
	}

	// Restart with the directory missing entirely.
	if err := os.RemoveAll(dir); err != nil {
		t.Fatal(err)
	}
	s2, err := NewOperatorKeyStore(env, dir)
	if err != nil {
		t.Fatalf("boot with missing dir: %v", err)
	}
	if got := s2.Match([]byte("op_legacy_env_ada")); got != "" {
		t.Errorf("restart without the key volume: revoked env key accepted: %q", got)
	}
	if got := strings.Join(s2.EnvRevoked(), ","); got != "ada" {
		t.Errorf("EnvRevoked() after restart = %q, want ada", got)
	}
	if got := strings.Join(s2.RevokedWithoutTombstone(), ","); got != "ada" {
		t.Errorf("RevokedWithoutTombstone() after restart = %q, want ada", got)
	}
	if _, err := os.Stat(DefaultRevocationLedgerPath(dir)); err != nil {
		t.Errorf("ledger not beside the key dir: %v", err)
	}
}

// A ledger that exists but cannot be parsed stops the boot: starting without
// the revocations it records would re-accept revoked keys.
func TestOperatorKeyLedger_corruptLedgerIsFatal(t *testing.T) {
	dir := t.TempDir()
	ledger := filepath.Join(t.TempDir(), "revoked.json")
	if err := os.WriteFile(ledger, []byte("{not json"), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := NewOperatorKeyStoreWithLedger(map[string]string{"ada": "op_a"}, dir, ledger); err == nil {
		t.Fatal("a corrupt revocation ledger must fail the boot")
	}
}

// GHSA-2w7v review r2, finding 4 (fail-to-pass). One key value pinned under
// two env handles: revoking one handle refuses the value under both.
func TestOperatorKeyLedger_revokedValueRefusedUnderEveryHandle(t *testing.T) {
	dir := t.TempDir()
	writeKeyFile(t, dir, TombstoneFileName("ada"), "ada\n")
	s := mustStore(t, map[string]string{"ada": "op_shared", "ada-alt": "op_shared", "bob": "op_bob"}, dir)
	if got := s.Match([]byte("op_shared")); got != "" {
		t.Errorf("revoked key value still accepted under another handle: %q", got)
	}
	if got := strings.Join(s.EnvRevoked(), ","); got != "ada,ada-alt" {
		t.Errorf("EnvRevoked() = %q, want ada,ada-alt", got)
	}
	if got := s.Match([]byte("op_bob")); got != "bob" {
		t.Errorf("unrelated key refused: %q", got)
	}
}

// First boot: no ledger file yet. The portal starts with an empty ledger and
// writes the file at the first revocation it reads.
func TestOperatorKeyLedger_firstBootWithoutLedgerFile(t *testing.T) {
	dir := t.TempDir()
	ledger := filepath.Join(t.TempDir(), "revoked.json")
	s, err := NewOperatorKeyStoreWithLedger(map[string]string{"ada": "op_a"}, dir, ledger)
	if err != nil {
		t.Fatalf("first boot without a ledger file must start: %v", err)
	}
	if got := s.Match([]byte("op_a")); got != "ada" {
		t.Fatalf("env key refused on first boot with an empty ledger: %q", got)
	}
	if _, err := os.Stat(ledger); !os.IsNotExist(err) {
		t.Fatalf("ledger written with nothing revoked: %v", err)
	}
	writeKeyFile(t, dir, TombstoneFileName("ada"), "ada\n")
	if _, err := s.Reload(); err != nil {
		t.Fatal(err)
	}
	body, err := os.ReadFile(ledger)
	if err != nil || !strings.Contains(string(body), `"ada"`) {
		t.Fatalf("ledger after the first revocation = %q, %v", body, err)
	}
}

// GHSA-2w7v review r3, finding 2 (fail-to-pass). A key file under a dotted
// handle (".revoked-x.key") is not a tombstone: its line never reaches the
// ledger, so the key secret is never persisted or logged.
func TestOperatorKeyLedger_dottedKeyFileIsNotATombstone(t *testing.T) {
	dir := t.TempDir()
	ledger := filepath.Join(t.TempDir(), "revoked.json")
	writeKeyFile(t, dir, RevokedFilePrefix+"x.key", ".revoked-x:op_SECRET_VALUE_1234\n")
	writeKeyFile(t, dir, TombstoneFileName("ada"), "ada\n")
	s, err := NewOperatorKeyStoreWithLedger(map[string]string{"ada": "op_a"}, dir, ledger)
	if err != nil {
		t.Fatal(err)
	}
	body, _ := os.ReadFile(ledger)
	if strings.Contains(string(body), "SECRET") {
		t.Fatalf("key secret persisted in the ledger: %s", body)
	}
	if got := strings.Join(s.RevokedWithoutTombstone(), ","); got != "" {
		t.Errorf("RevokedWithoutTombstone() = %q, want none", got)
	}
	if !strings.Contains(string(body), `"ada"`) {
		t.Errorf("hex-named tombstone not read: %s", body)
	}
}

// GHSA-2w7v review r3, finding 3 (fail-to-pass). A handle the ledger holds
// with no tombstone on disk still gets its env value recorded, so the value
// stays refused after the deployment renames the env handle.
func TestOperatorKeyLedger_ledgerOnlyHandleValueRefusedAfterRename(t *testing.T) {
	dir := t.TempDir()
	ledger := filepath.Join(t.TempDir(), "revoked.json")
	writeKeyFile(t, dir, TombstoneFileName("ada"), "ada\n")
	if _, err := NewOperatorKeyStoreWithLedger(map[string]string{}, dir, ledger); err != nil {
		t.Fatal(err)
	}
	if err := os.Remove(filepath.Join(dir, TombstoneFileName("ada"))); err != nil {
		t.Fatal(err)
	}
	s2, err := NewOperatorKeyStoreWithLedger(map[string]string{"ada": "op_K"}, dir, ledger)
	if err != nil {
		t.Fatal(err)
	}
	if got := s2.Match([]byte("op_K")); got != "" {
		t.Fatalf("ledger handle ada accepted: %q", got)
	}
	s3, err := NewOperatorKeyStoreWithLedger(map[string]string{"ada2": "op_K"}, dir, ledger)
	if err != nil {
		t.Fatal(err)
	}
	if got := s3.Match([]byte("op_K")); got != "" {
		t.Errorf("value refused for ada accepted under renamed handle: %q", got)
	}
}

// GHSA-2w7v review r3, finding 4 (fail-to-pass). A zero-length ledger (an
// interrupted write) is unknown state, not corruption: the portal boots,
// refuses every handle that still has a tombstone, reports the state, and
// rewrites the ledger. A corrupt non-empty ledger stays fatal (see above).
func TestOperatorKeyLedger_emptyLedgerBootsAndRebuildsFromTombstones(t *testing.T) {
	dir := t.TempDir()
	ledger := filepath.Join(t.TempDir(), "revoked.json")
	if err := os.WriteFile(ledger, nil, 0o600); err != nil {
		t.Fatal(err)
	}
	writeKeyFile(t, dir, TombstoneFileName("ada"), "ada\n")
	s, err := NewOperatorKeyStoreWithLedger(map[string]string{"ada": "op_a", "bg": "op_bg"}, dir, ledger)
	if err != nil {
		t.Fatalf("an empty ledger must not stop the boot: %v", err)
	}
	if got := s.Match([]byte("op_a")); got != "" {
		t.Errorf("tombstoned env key accepted with an empty ledger: %q", got)
	}
	if got := s.Match([]byte("op_bg")); got != "bg" {
		t.Errorf("break-glass refused: %q", got)
	}
	if s.RevocationLedgerError() == nil {
		t.Error("the empty-ledger state is not reported")
	}
	if body, _ := os.ReadFile(ledger); !strings.Contains(string(body), `"ada"`) {
		t.Errorf("ledger not rebuilt from tombstones: %q", body)
	}
}

// GHSA-2w7v review r4, INFO 2. A missing ledger beside an existing key
// directory is reported as unknown state (the role creates an empty ledger at
// deploy, so "missing" means lost), while the portal still boots and rebuilds
// from the tombstones. With no key directory at all nothing is flagged.
func TestOperatorKeyLedger_missingLedgerBesideKeyDirIsReported(t *testing.T) {
	dir := t.TempDir()
	ledger := filepath.Join(t.TempDir(), "revoked.json")
	writeKeyFile(t, dir, TombstoneFileName("ada"), "ada\n")
	s, err := NewOperatorKeyStoreWithLedger(map[string]string{"ada": "op_a", "bg": "op_bg"}, dir, ledger)
	if err != nil {
		t.Fatalf("a missing ledger must not stop the boot: %v", err)
	}
	if s.RevocationLedgerError() == nil {
		t.Error("a missing ledger beside an existing key directory is not reported")
	}
	if got := s.Match([]byte("op_a")); got != "" {
		t.Errorf("tombstoned env key accepted: %q", got)
	}
	none, err := NewOperatorKeyStoreWithLedger(map[string]string{"bg": "op_bg"}, filepath.Join(t.TempDir(), "absent"), filepath.Join(t.TempDir(), "r.json"))
	if err != nil {
		t.Fatal(err)
	}
	if none.RevocationLedgerError() != nil {
		t.Errorf("no key directory: flagged anyway: %v", none.RevocationLedgerError())
	}
}

// GHSA-2w7v review r4, INFO 3. The portal itself refuses a ledger inside the
// key directory, not only the role's assert.
func TestOperatorKeyLedger_ledgerInsideKeyDirIsRefused(t *testing.T) {
	dir := t.TempDir()
	for _, p := range []string{filepath.Join(dir, ".revoked.json"), filepath.Join(dir, "sub", "r.json")} {
		if _, err := NewOperatorKeyStoreWithLedger(map[string]string{}, dir, p); err == nil {
			t.Errorf("ledger %s inside the key directory accepted", p)
		}
	}
	if _, err := NewOperatorKeyStoreWithLedger(map[string]string{}, dir, dir+"-sibling.json"); err != nil {
		t.Errorf("a sibling ledger refused: %v", err)
	}
}
