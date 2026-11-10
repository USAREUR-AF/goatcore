package main

import (
	"crypto/x509"
	"encoding/pem"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	"filippo.io/age"
)

func TestRootSubjectCN(t *testing.T) {
	cn, derived := rootSubjectCN("galaxy-prod Root CA", "dev-galaxy-prod-ca")
	if cn != "galaxy-prod Root CA" || derived {
		t.Errorf("explicit --root-cn: got (%q, %v), want it kept, not derived", cn, derived)
	}
	a, _ := rootSubjectCN("", "dev-galaxy-ca-2026-09-03")
	b, derived := rootSubjectCN("", "dev-hobby-ca-2026-07-15")
	if !derived || a == b {
		t.Errorf("two CA ids derived %q and %q: roots of different goatnets must not share a subject", a, b)
	}
	if !strings.HasPrefix(a, "dev-galaxy-ca-2026-09-03") {
		t.Errorf("derived CN %q does not name its CA id", a)
	}
}

func TestPublicKeyPath(t *testing.T) {
	if got := publicKeyPath("/keys", "dev-x"); got != "/keys/dev-x.pem" {
		t.Errorf("got %q, want /keys/dev-x.pem", got)
	}
	if got := publicKeyPath("", "dev-x"); got != "" {
		t.Errorf("no --public-key-dir: got %q, want no path (no implicit write into the trusted key dir)", got)
	}
}

func TestWritePublicKeyNeverOverwrites(t *testing.T) {
	p := filepath.Join(t.TempDir(), "dev-x.pem")
	if err := refuseExisting(p); err != nil {
		t.Fatalf("absent file refused: %v", err)
	}
	if err := writePublicKey(p, []byte("first")); err != nil {
		t.Fatal(err)
	}
	if err := refuseExisting(p); err == nil {
		t.Error("an existing key file was not refused")
	}
	if err := writePublicKey(p, []byte("second")); err == nil {
		t.Error("writePublicKey replaced an existing key file")
	}
	got, _ := os.ReadFile(p)
	if string(got) != "first" {
		t.Errorf("key file content %q, want the first write kept", got)
	}
	st, _ := os.Stat(p)
	if st.Mode().Perm() != 0o644 {
		t.Errorf("mode %o, want 0644", st.Mode().Perm())
	}
}

// TestDevCeremonyPersistsKeyAndNamesRoot runs the built binary through a dev
// ceremony: the public key lands in --public-key-dir, matches stdout, and the
// root carries a CN derived from the CA id (#393). A rerun with the same id is
// refused before a new key is generated.
func TestDevCeremonyPersistsKeyAndNamesRoot(t *testing.T) {
	work := t.TempDir()
	bin := filepath.Join(work, "ca-init")
	build := command(t, "go", "build", "-o", bin, ".")
	if out, err := build.CombinedOutput(); err != nil {
		t.Fatalf("go build: %v\n%s", err, out)
	}
	identity, err := age.GenerateX25519Identity()
	if err != nil {
		t.Fatal(err)
	}
	idPath := filepath.Join(work, "keys.txt")
	if err := os.WriteFile(idPath, []byte(identity.String()+"\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	pubDir := filepath.Join(work, "public-keys")
	if err := os.Mkdir(pubDir, 0o750); err != nil {
		t.Fatal(err)
	}
	args := []string{
		"--mode", "dev", "--id", "test-net-ca",
		"--out", filepath.Join(work, "ca.key.age"),
		"--recipients", identity.Recipient().String() + "," + secondRecipient(t),
		"--gen-root-cert", filepath.Join(work, "root.pem"),
		"--age-identity", idPath,
		"--public-key-dir", pubDir,
	}
	cmd := command(t, bin, args...)
	var stderr strings.Builder
	cmd.Stderr = &stderr
	stdout, err := cmd.Output()
	if err != nil {
		t.Fatalf("ca-init: %v\n%s", err, stderr.String())
	}

	written, err := os.ReadFile(filepath.Join(pubDir, "dev-test-net-ca.pem"))
	if err != nil {
		t.Fatalf("public key not persisted: %v\n%s", err, stderr.String())
	}
	if string(written) != string(stdout) {
		t.Errorf("persisted PEM differs from the PEM on stdout")
	}

	rootPEM, err := os.ReadFile(filepath.Join(work, "root.pem"))
	if err != nil {
		t.Fatal(err)
	}
	block, _ := pem.Decode(rootPEM)
	if block == nil {
		t.Fatal("root.pem holds no PEM block")
	}
	root, err := x509.ParseCertificate(block.Bytes)
	if err != nil {
		t.Fatal(err)
	}
	if root.Subject.CommonName != "dev-test-net-ca Root CA" {
		t.Errorf("root CN %q, want it derived from the CA id", root.Subject.CommonName)
	}
	if !strings.Contains(stderr.String(), "derived from the CA id") {
		t.Errorf("derived CN was not announced on stderr:\n%s", stderr.String())
	}

	args[5] = filepath.Join(work, "ca2.key.age") // --out
	rerun := command(t, bin, args...)
	out, err := rerun.CombinedOutput()
	if err == nil {
		t.Fatalf("a rerun with an existing key file succeeded:\n%s", out)
	}
	if _, statErr := os.Stat(filepath.Join(work, "ca2.key.age")); statErr == nil {
		t.Error("the refused rerun still generated a new CA key")
	}
}

// TestNoFlagWritesNothing: without --public-key-dir, a run from inside the
// repo must not drop a key into the trusted public-keys directory.
func TestNoFlagWritesNothing(t *testing.T) {
	work := t.TempDir()
	bin := filepath.Join(work, "ca-init")
	if out, err := command(t, "go", "build", "-o", bin, ".").CombinedOutput(); err != nil {
		t.Fatalf("go build: %v\n%s", err, out)
	}
	identity, err := age.GenerateX25519Identity()
	if err != nil {
		t.Fatal(err)
	}
	keysDir, err := filepath.Abs(filepath.Join("..", "..", "public-keys"))
	if err != nil {
		t.Fatal(err)
	}
	before, _ := os.ReadDir(keysDir)
	cmd := command(t, bin, "--mode", "dev", "--id", "test-noflag-ca",
		"--out", filepath.Join(work, "ca.key.age"),
		"--recipients", identity.Recipient().String()+","+secondRecipient(t))
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("ca-init: %v\n%s", err, out)
	}
	if !strings.Contains(string(out), "public key NOT persisted") {
		t.Errorf("missing the not-persisted warning:\n%s", out)
	}
	after, _ := os.ReadDir(keysDir)
	if len(after) != len(before) {
		t.Errorf("a run without --public-key-dir changed %s (%d -> %d entries)", keysDir, len(before), len(after))
	}
}

// command runs `go build` or the binary this test just built, bound to the
// test's context.
func command(t *testing.T, name string, args ...string) *exec.Cmd {
	return exec.CommandContext(t.Context(), name, args...) //nolint:gosec // fixed go build argv or the test-built binary
}

// secondRecipient returns a fresh recipient so a ceremony meets the
// two-recipient floor (#1509).
func secondRecipient(t *testing.T) string {
	t.Helper()
	id, err := age.GenerateX25519Identity()
	if err != nil {
		t.Fatal(err)
	}
	return id.Recipient().String()
}
