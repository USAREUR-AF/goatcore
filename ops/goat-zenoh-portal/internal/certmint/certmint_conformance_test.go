package certmint_test

// The F-270 conformance gate — the portal-mint half.
//
// A portal-minted leaf is the key a member unwraps granted data with
// (docs/design/grant-keyed-encryption.md §3a): its public key is the DEK wrap
// target, ECDH over P-256 (envelope.RecipientFromCert / grant-keyd's EC-only
// rule). Ed25519 can sign but cannot ECDH — so if certmint's argv ever drops
// --key-type ecdsa-p256, every portal-minted participant silently loses the
// ability to decrypt .
//
// This test runs the REAL production path — certmint.Mint shelling to the real
// cert-sign against a throwaway offline CA — and asserts the minted leaf is
// ECDSA-P256. The wrap→unwrap half of the chain runs in
// ops/grant-crypto/conformance/portal-mint/run.sh (the grant-crypto.yml
// portal-mint-conformance job), which builds the binaries, runs this test, and
// then seals+opens a DEK against the leaf this test exports.
//
// Skipped unless CERTMINT_CONFORMANCE_BIN_DIR points at a directory holding
// cert-sign, ca-init, and age-keygen binaries. If CERTMINT_CONFORMANCE_OUT is
// set, the minted cert+key PEMs are copied there for the wrap/unwrap leg.

import (
	"context"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/x509"
	"encoding/pem"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/certmint"
)

func TestMint_RealCertSign_LeafIsP256Recipient(t *testing.T) {
	binDir := os.Getenv("CERTMINT_CONFORMANCE_BIN_DIR")
	if binDir == "" {
		t.Skip("CERTMINT_CONFORMANCE_BIN_DIR unset; run via ops/grant-crypto/conformance/portal-mint/run.sh")
	}

	dir := t.TempDir()
	// cert-sign appends to a CA audit log under $HOME by default; keep the
	// conformance mint's audit line out of the operator's real audit log.
	t.Setenv("HOME", dir)
	run := func(name string, args ...string) string {
		t.Helper()
		out, err := exec.Command(filepath.Join(binDir, name), args...).CombinedOutput()
		if err != nil {
			t.Fatalf("%s %v: %v\n%s", name, args, err, out)
		}
		return string(out)
	}

	// Throwaway offline CA: an age identity, then ca-init in dev mode. File
	// names inside caDir must be exactly what Minter derives (root-cert.pem +
	// ca.key.age under CABundleDir).
	ident := filepath.Join(dir, "age-identity.txt")
	run("age-keygen", "-o", ident)
	agePub := strings.TrimSpace(run("age-keygen", "-y", ident))
	caDir := filepath.Join(dir, "ca")
	if err := os.MkdirAll(caDir, 0o700); err != nil {
		t.Fatal(err)
	}
	run("ca-init",
		"--mode", "dev",
		"--out", filepath.Join(caDir, "ca.key.age"),
		"--recipients", agePub,
		"--allow-single-recipient", // throwaway CA; the floor is for real CAs (#1509)
		"--id", "conformance-ca",
		"--gen-root-cert", filepath.Join(caDir, "root-cert.pem"),
		"--age-identity", ident,
	)

	onb := filepath.Join(dir, "onboarding.md")
	if err := os.WriteFile(onb, []byte("# conformance onboarding\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	m := &certmint.Minter{
		BinaryPath:       filepath.Join(binDir, "cert-sign"),
		CABundleDir:      caDir,
		AgeIdentityPath:  ident,
		CAID:             "conformance-ca",
		SiteID:           "conformance",
		Org:              "Conformance Org",
		URIPrefix:        "urn:goat:galaxy:org=",
		DNSSuffix:        "galaxy.peer",
		CARoot:           filepath.Join(caDir, "root-cert.pem"),
		OnboardingPacket: onb,
		VendorBundleDir:  filepath.Join(dir, "bundles"),
		Validity:         24 * time.Hour,
	}
	// Slot-pool shape (ADR 1025): CN = the bare anchor, exactly as GALAXY mints.
	res, err := m.Mint(context.Background(), "acme", "vp_conformance", certmint.MintOptions{CommonName: "acme"})
	if err != nil {
		t.Fatalf("real-mode mint: %v", err)
	}

	// The stage dir is removed by Mint; the leaf survives only inside the
	// tarball. Pull cert + key out of it.
	bodies, _ := readTarball(t, res.TarballPath)
	certPEM, ok := bodies["acme-cert.pem"]
	if !ok {
		t.Fatal("tarball missing acme-cert.pem")
	}
	keyPEM, ok := bodies["acme-key.pem"]
	if !ok {
		t.Fatal("tarball missing acme-key.pem")
	}

	block, _ := pem.Decode([]byte(certPEM))
	if block == nil || block.Type != "CERTIFICATE" {
		t.Fatalf("minted cert is not a CERTIFICATE PEM (got %q)", certPEM[:min(80, len(certPEM))])
	}
	cert, err := x509.ParseCertificate(block.Bytes)
	if err != nil {
		t.Fatalf("parse minted cert: %v", err)
	}

	// THE contract: the leaf public key must be ECDSA over P-256 so it can be
	// a DEK wrap target. An Ed25519 leaf here is F-270 regressed.
	pub, ok := cert.PublicKey.(*ecdsa.PublicKey)
	if !ok {
		t.Fatalf("minted leaf public key is %T; the recipient contract requires *ecdsa.PublicKey P-256", cert.PublicKey)
	}
	if pub.Curve != elliptic.P256() {
		t.Fatalf("minted leaf curve is %s; the recipient contract requires P-256", pub.Curve.Params().Name)
	}
	// The goatnet's Subject O reaches the leaf (cert-sign --org).
	if len(cert.Subject.Organization) != 1 || cert.Subject.Organization[0] != "Conformance Org" {
		t.Errorf("leaf O = %q; want [\"Conformance Org\"] from Minter.Org", cert.Subject.Organization)
	}
	if cert.Subject.CommonName != "acme" {
		t.Errorf("leaf CN = %q; want the bare anchor \"acme\" (slot-pool shape)", cert.Subject.CommonName)
	}

	// Export for the wrap/unwrap leg of the conformance chain.
	if out := os.Getenv("CERTMINT_CONFORMANCE_OUT"); out != "" {
		if err := os.MkdirAll(out, 0o750); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(out, "acme-cert.pem"), []byte(certPEM), 0o644); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(out, "acme-key.pem"), []byte(keyPEM), 0o600); err != nil {
			t.Fatal(err)
		}
	}
}
