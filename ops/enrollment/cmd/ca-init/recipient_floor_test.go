package main

import (
	"bytes"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"filippo.io/age"
)

// buildCAInit builds the binary under test into a temp dir.
func buildCAInit(t *testing.T) string {
	t.Helper()
	bin := filepath.Join(t.TempDir(), "ca-init")
	if out, err := command(t, "go", "build", "-o", bin, ".").CombinedOutput(); err != nil {
		t.Fatalf("go build: %v\n%s", err, out)
	}
	return bin
}

// newIdentities makes n age identities, writes the first to an identity file,
// and returns that file plus the comma-joined recipient list.
func newIdentities(t *testing.T, dir string, n int) (string, string) {
	t.Helper()
	recipients := make([]string, 0, n)
	var first *age.X25519Identity
	for i := 0; i < n; i++ {
		id, err := age.GenerateX25519Identity()
		if err != nil {
			t.Fatal(err)
		}
		if first == nil {
			first = id
		}
		recipients = append(recipients, id.Recipient().String())
	}
	idPath := filepath.Join(dir, "keys.txt")
	if err := os.WriteFile(idPath, []byte(first.String()+"\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	return idPath, strings.Join(recipients, ",")
}

// A CA key encrypted to one recipient is lost with that one identity. ca-init
// refuses to write one unless the operator asks for it explicitly, and it
// refuses before generating anything.
func TestDevModeRefusesASingleRecipient(t *testing.T) {
	bin := buildCAInit(t)
	work := t.TempDir()
	_, one := newIdentities(t, work, 1)
	keyPath := filepath.Join(work, "ca.key.age")

	out, err := command(t, bin, "--mode", "dev", "--id", "floor-ca",
		"--out", keyPath, "--recipients", one).CombinedOutput()
	if err == nil {
		t.Fatalf("one recipient was accepted:\n%s", out)
	}
	if !strings.Contains(string(out), "at least 2 age recipients") {
		t.Errorf("refusal does not state the floor:\n%s", out)
	}
	if !strings.Contains(string(out), "--allow-single-recipient") {
		t.Errorf("refusal does not name the override:\n%s", out)
	}
	if _, statErr := os.Stat(keyPath); statErr == nil {
		t.Error("the refused run still wrote a CA key")
	}
}

func TestDevModeAcceptsTwoRecipients(t *testing.T) {
	bin := buildCAInit(t)
	work := t.TempDir()
	_, two := newIdentities(t, work, 2)
	keyPath := filepath.Join(work, "ca.key.age")

	out, err := command(t, bin, "--mode", "dev", "--id", "floor-ca",
		"--out", keyPath, "--recipients", two).CombinedOutput()
	if err != nil {
		t.Fatalf("two recipients refused: %v\n%s", err, out)
	}
	if n := recipientCountOf(t, keyPath); n != 2 {
		t.Errorf("key file carries %d recipient stanzas, want 2", n)
	}
}

func TestSingleRecipientOverrideIsAcceptedAndAnnounced(t *testing.T) {
	bin := buildCAInit(t)
	work := t.TempDir()
	_, one := newIdentities(t, work, 1)

	out, err := command(t, bin, "--mode", "dev", "--id", "floor-ca",
		"--out", filepath.Join(work, "ca.key.age"), "--recipients", one,
		"--allow-single-recipient").CombinedOutput()
	if err != nil {
		t.Fatalf("override refused: %v\n%s", err, out)
	}
	if !strings.Contains(string(out), "single age recipient") {
		t.Errorf("override ran without a warning:\n%s", out)
	}
}

// The intermediate key is encrypted to the same recipients as the root, so the
// same floor applies when only an intermediate is minted.
func TestIntermediateOnlyRefusesASingleRecipient(t *testing.T) {
	bin := buildCAInit(t)
	work := t.TempDir()
	idPath, one := newIdentities(t, work, 1)
	intKey := filepath.Join(work, "int.key.age")

	out, err := command(t, bin, "--mode", "dev", "--intermediate-only",
		"--out", filepath.Join(work, "ca.key.age"), "--recipients", one,
		"--age-identity", idPath,
		"--gen-root-cert", filepath.Join(work, "root.pem"),
		"--gen-intermediate-cert", filepath.Join(work, "int.pem"),
		"--intermediate-out-key", intKey).CombinedOutput()
	if err == nil {
		t.Fatalf("intermediate-only with one recipient was accepted:\n%s", out)
	}
	if !strings.Contains(string(out), "at least 2 age recipients") {
		t.Errorf("refusal does not state the floor:\n%s", out)
	}
	if _, statErr := os.Stat(intKey); statErr == nil {
		t.Error("the refused run still wrote an intermediate key")
	}
}

// mint runs a dev ceremony and returns the key, root cert and public key paths.
func mint(t *testing.T, bin, work, id, rootCN string, recipientCount int, extra ...string) (string, string, string) {
	t.Helper()
	idPath, recipients := newIdentities(t, work, recipientCount)
	pubDir := filepath.Join(work, "public-keys")
	if err := os.MkdirAll(pubDir, 0o750); err != nil {
		t.Fatal(err)
	}
	keyPath := filepath.Join(work, id+".key.age")
	rootPath := filepath.Join(work, id+"-root.pem")
	args := []string{"--mode", "dev", "--id", id, "--out", keyPath,
		"--recipients", recipients, "--age-identity", idPath,
		"--gen-root-cert", rootPath, "--public-key-dir", pubDir}
	if rootCN != "" {
		args = append(args, "--root-cn", rootCN)
	}
	args = append(args, extra...)
	if out, err := command(t, bin, args...).CombinedOutput(); err != nil {
		t.Fatalf("mint %s: %v\n%s", id, err, out)
	}
	return keyPath, rootPath, filepath.Join(pubDir, "dev-"+id+".pem")
}

func verifyCommand(t *testing.T, bin, key, root, pub string) (string, error) {
	t.Helper()
	out, err := command(t, bin, "--verify", "--out", key, "--root-cert", root, "--public-key", pub).CombinedOutput()
	return string(out), err
}

func TestVerifyPassesAWellFormedCA(t *testing.T) {
	bin := buildCAInit(t)
	key, root, pub := mint(t, bin, t.TempDir(), "good-ca", "good-net Root CA", 2)
	if out, err := verifyCommand(t, bin, key, root, pub); err != nil {
		t.Fatalf("verify failed a well-formed CA: %v\n%s", err, out)
	}
}

func TestVerifyFlagsASingleRecipientKey(t *testing.T) {
	bin := buildCAInit(t)
	key, root, pub := mint(t, bin, t.TempDir(), "solo-ca", "solo-net Root CA", 1, "--allow-single-recipient")
	out, err := verifyCommand(t, bin, key, root, pub)
	if err == nil {
		t.Fatalf("verify passed a single-recipient key:\n%s", out)
	}
	if !strings.Contains(out, "1 age recipient") {
		t.Errorf("verify did not report the recipient count:\n%s", out)
	}
}

func TestVerifyFlagsLegacyAndUnexpandedRootCN(t *testing.T) {
	bin := buildCAInit(t)
	for _, cn := range []string{legacyRootCN, "$SITE Root CA"} {
		key, root, pub := mint(t, bin, t.TempDir(), "cn-ca", cn, 2)
		out, err := verifyCommand(t, bin, key, root, pub)
		if err == nil {
			t.Errorf("verify passed root CN %q:\n%s", cn, out)
			continue
		}
		if !strings.Contains(out, "root CN") {
			t.Errorf("verify of CN %q did not name the CN problem:\n%s", cn, out)
		}
	}
}

func TestVerifyFlagsAPublicKeyThatIsNotTheRoots(t *testing.T) {
	bin := buildCAInit(t)
	work := t.TempDir()
	key, root, _ := mint(t, bin, work, "one-ca", "one-net Root CA", 2)
	_, _, otherPub := mint(t, bin, work, "two-ca", "two-net Root CA", 2)
	out, err := verifyCommand(t, bin, key, root, otherPub)
	if err == nil {
		t.Fatalf("verify passed a public key from a different CA:\n%s", out)
	}
	if !strings.Contains(out, "does not match") {
		t.Errorf("verify did not report the public key mismatch:\n%s", out)
	}
}

// Encrypters other than filippo.io/age (rage, for one) add a random "grease"
// stanza. It carries no key and must not count toward the floor.
func TestCountAgeRecipientStanzasIgnoresGrease(t *testing.T) {
	header := "age-encryption.org/v1\n" +
		"-> X25519 AAAA\nYmJi\n" +
		"-> X25519 BBBB\nY2Nj\n" +
		"-> 9z-grease x y\nZGRk\n" +
		"--- bWFj\n"
	n, err := countAgeRecipientStanzas(bytes.NewReader([]byte(header + "payload")))
	if err != nil {
		t.Fatal(err)
	}
	if n != 2 {
		t.Errorf("counted %d recipient stanzas, want 2", n)
	}
	if _, err := countAgeRecipientStanzas(strings.NewReader("not an age file\n")); err == nil {
		t.Error("a non-age file was counted")
	}
}

func recipientCountOf(t *testing.T, path string) int {
	t.Helper()
	f, err := os.Open(path)
	if err != nil {
		t.Fatal(err)
	}
	defer f.Close()
	n, err := countAgeRecipientStanzas(f)
	if err != nil {
		t.Fatal(err)
	}
	return n
}

func TestDevModeRefusesDuplicateRecipientsBeforeWriting(t *testing.T) {
	bin := buildCAInit(t)
	work := t.TempDir()
	_, one := newIdentities(t, work, 1)
	for _, override := range []bool{false, true} {
		key := filepath.Join(work, "duplicate.key.age")
		args := []string{"--mode", "dev", "--id", "duplicate-ca", "--out", key, "--recipients", one + ", " + one}
		if override {
			args = append(args, "--allow-single-recipient")
		}
		out, err := command(t, bin, args...).CombinedOutput()
		if err == nil || !strings.Contains(string(out), "duplicate age recipient") {
			t.Fatalf("duplicate accepted: %v %s", err, out)
		}
		if _, err := os.Stat(key); !os.IsNotExist(err) {
			t.Fatalf("refused run created key: %v", err)
		}
	}
}
