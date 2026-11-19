package certmint_test

import (
	"context"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/hex"
	"encoding/pem"
	"math/big"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/certmint"
)

// writeStubLeaf writes a self-signed PEM leaf for a cert-sign stub to copy
// into --out-cert, and returns its path and certificate.
func writeStubLeaf(t *testing.T, dir string) (string, *x509.Certificate) {
	t.Helper()
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	tmpl := &x509.Certificate{
		SerialNumber: big.NewInt(0x5eed1eaf),
		Subject:      pkix.Name{CommonName: "0123456789abcdef0123456789abcdef.g3"},
		NotBefore:    time.Unix(1_800_000_000, 0).UTC(),
		NotAfter:     time.Unix(1_831_536_000, 0).UTC(),
	}
	der, err := x509.CreateCertificate(rand.Reader, tmpl, tmpl, &key.PublicKey, key)
	if err != nil {
		t.Fatal(err)
	}
	c, err := x509.ParseCertificate(der)
	if err != nil {
		t.Fatal(err)
	}
	p := filepath.Join(dir, "stub-leaf.pem")
	if err := os.WriteFile(p, pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der}), 0o644); err != nil {
		t.Fatal(err)
	}
	return p, c
}

// A real mint returns the serial and validity of the leaf cert-sign wrote,
// which the slot registry records for the reuse quarantine (GHSA-rr3p).
func TestMint_ReturnsLeafSerialAndValidity(t *testing.T) {
	stubDir := t.TempDir()
	leaf, want := writeStubLeaf(t, stubDir)
	stub := filepath.Join(stubDir, "cert-sign")
	script := "#!/bin/sh\n" +
		"while [ $# -gt 0 ]; do\n" +
		"  case $1 in\n" +
		"    --out-cert) cp " + leaf + " \"$2\"; shift 2;;\n" +
		"    --out-key)  echo stub-key  > \"$2\"; shift 2;;\n" +
		"    *) shift;;\n" +
		"  esac\n" +
		"done\n"
	if err := os.WriteFile(stub, []byte(script), 0o755); err != nil {
		t.Fatal(err)
	}
	m := &certmint.Minter{
		BinaryPath:      stub,
		CABundleDir:     stubDir,
		AgeIdentityPath: filepath.Join(stubDir, "age.key"),
		VendorBundleDir: t.TempDir(),
	}
	res, err := m.Mint(context.Background(), "0123456789abcdef0123456789abcdef", "vp_k",
		certmint.MintOptions{CommonName: "0123456789abcdef0123456789abcdef.g3"})
	if err != nil {
		t.Fatalf("mint: %v", err)
	}
	if res.Serial != hex.EncodeToString(want.SerialNumber.Bytes()) {
		t.Errorf("serial = %q, want %x", res.Serial, want.SerialNumber.Bytes())
	}
	if !res.NotBefore.Equal(want.NotBefore) || !res.NotAfter.Equal(want.NotAfter) {
		t.Errorf("validity = [%v, %v], want [%v, %v]", res.NotBefore, res.NotAfter, want.NotBefore, want.NotAfter)
	}

	// A cert-sign that writes something other than a PEM certificate fails
	// the mint: an unrecordable leaf must not be handed out.
	bad := filepath.Join(stubDir, "cert-sign-bad")
	if err := os.WriteFile(bad, []byte("#!/bin/sh\nwhile [ $# -gt 0 ]; do case $1 in --out-cert) echo junk > \"$2\"; shift 2;; --out-key) echo k > \"$2\"; shift 2;; *) shift;; esac; done\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	m.BinaryPath = bad
	if _, err := m.Mint(context.Background(), "0123456789abcdef0123456789abcdef", "vp_k", certmint.MintOptions{}); err == nil {
		t.Fatal("mint with an unparseable leaf succeeded")
	}
}

// A fake mint synthesizes a serial and the configured validity.
func TestMint_FakeLeafValidity(t *testing.T) {
	m := &certmint.Minter{Fake: true, VendorBundleDir: t.TempDir(), Validity: 48 * time.Hour}
	res, err := m.Mint(context.Background(), "acme", "vp_k", certmint.MintOptions{})
	if err != nil {
		t.Fatal(err)
	}
	if len(res.Serial) != 32 {
		t.Errorf("fake serial %q, want 16 random bytes in hex", res.Serial)
	}
	if got := res.NotAfter.Sub(res.NotBefore); got != 48*time.Hour {
		t.Errorf("fake validity = %v, want 48h", got)
	}
}

// The free-slot placeholder CN is never minted (GHSA-rr3p F5).
func TestMint_refusesUnassignedCN(t *testing.T) {
	m := &certmint.Minter{Fake: true, VendorBundleDir: t.TempDir()}
	for _, cn := range []string{"0123456789abcdef0123456789abcdef.unassigned", "trial-peer-read.unassigned"} {
		if _, err := m.Mint(context.Background(), "0123456789abcdef0123456789abcdef", "k", certmint.MintOptions{CommonName: cn}); err == nil {
			t.Errorf("minted CN %q", cn)
		}
	}
	if _, err := m.Mint(context.Background(), "0123456789abcdef0123456789abcdef", "k",
		certmint.MintOptions{CommonName: "0123456789abcdef0123456789abcdef.g1"}); err != nil {
		t.Errorf("a generation CN was refused: %v", err)
	}
}
