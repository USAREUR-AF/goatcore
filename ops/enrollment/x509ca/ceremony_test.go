package x509ca

import (
	"bytes"
	"crypto/rand"
	"crypto/x509"
	"encoding/pem"
	"os"
	"path/filepath"
	"testing"

	"filippo.io/age"

	"github.com/usareur-af/goatcore/ops/enrollment/bundle"
	"github.com/usareur-af/goatcore/ops/enrollment/ca"
)

// TestCeremony_EndToEnd exercises the full operator workflow as code:
//
//  1. Generate an Ed25519 CA key encrypted with age (matches ca-init's
//     dev-mode output)
//  2. Decrypt + load it as a ca.Signer (matches cert-sign's input)
//  3. Generate the root cert (matches `ca-init --gen-root-cert`)
//  4. Issue a leaf cert (matches cert-sign's IssueCert)
//  5. Build an enrollment bundle, embed the root cert as
//     TLSTrustAnchor, sign with the same root key
//  6. Verify: bundle signature checks against ca pubkey; leaf chains to
//     root extracted from bundle
//
// This is the contract a fresh-environment ceremony must honor end-to-end.
func TestCeremony_EndToEnd(t *testing.T) {
	tmp := t.TempDir()
	caKeyPath := filepath.Join(tmp, "ca.key.age")

	// Step 1 — generate age-encrypted CA key
	identity, err := age.GenerateX25519Identity()
	if err != nil {
		t.Fatalf("generate age identity: %v", err)
	}
	caPub, err := ca.GenerateAndWrite(caKeyPath, []age.Recipient{identity.Recipient()})
	if err != nil {
		t.Fatalf("ca.GenerateAndWrite: %v", err)
	}

	// Step 2 — decrypt + load as Signer
	signer, err := ca.NewAgeSigner("test-ca", caKeyPath, []age.Identity{identity})
	if err != nil {
		t.Fatalf("ca.NewAgeSigner: %v", err)
	}
	t.Cleanup(func() {
		if err := signer.Close(); err != nil {
			t.Errorf("close signer: %v", err)
		}
	})
	loadedPub := signer.PublicKey()
	if loadedPub.X.Cmp(caPub.X) != 0 || loadedPub.Y.Cmp(caPub.Y) != 0 {
		t.Fatal("decrypted key produces different pubkey than generation")
	}

	// Step 3 — generate root cert
	rootPEM, rootCert, err := GenerateRootCert(signer, RootCertOptions{
		Organization: "goat",
		CommonName:   "goat Root CA — Integration Test",
	})
	if err != nil {
		t.Fatalf("GenerateRootCert: %v", err)
	}

	// Step 4 — issue a leaf cert for the lab mgmt-API SAN list
	tpl := &LeafTemplate{
		CommonName: "mgmt.lab.netbird.example.net",
		OrgUnit:    "lab",
		SANs: []string{
			"mgmt.lab.netbird.example.net",
			"10.99.0.1",
			"127.0.0.1",
		},
	}
	leaf, err := IssueCert(tpl, signer, rootCert)
	if err != nil {
		t.Fatalf("IssueCert: %v", err)
	}

	// Step 5 — build bundle with root cert (DER) embedded as TLSTrustAnchor
	rootBlock, _ := pem.Decode(rootPEM)
	if rootBlock == nil {
		t.Fatal("decode root PEM: nil block")
	}
	rootDER := rootBlock.Bytes

	b := makeMinimalBundle(t)
	b.TLSTrustAnchor = rootDER
	if err := b.Sign(ca.SignFunc(signer)); err != nil {
		t.Fatalf("bundle sign: %v", err)
	}

	// Round-trip the bundle through CBOR
	wire, err := b.Marshal()
	if err != nil {
		t.Fatalf("bundle marshal: %v", err)
	}
	parsedBundle, err := bundle.Unmarshal(wire)
	if err != nil {
		t.Fatalf("bundle unmarshal: %v", err)
	}
	if err := parsedBundle.Verify(signer.PublicKey()); err != nil {
		t.Fatalf("bundle signature verify: %v", err)
	}

	// Step 6 — extract trust anchor from bundle, verify leaf chains to it
	if !bytes.Equal(parsedBundle.TLSTrustAnchor, rootDER) {
		t.Fatal("bundle TLSTrustAnchor != original rootDER")
	}
	rootFromBundle, err := x509.ParseCertificate(parsedBundle.TLSTrustAnchor)
	if err != nil {
		t.Fatalf("parse root from bundle: %v", err)
	}

	leafBlock, _ := pem.Decode(leaf.CertPEM)
	if leafBlock == nil {
		t.Fatal("decode leaf PEM: nil block")
	}
	leafCert, err := x509.ParseCertificate(leafBlock.Bytes)
	if err != nil {
		t.Fatalf("parse leaf: %v", err)
	}

	// The end-to-end check: leaf chains to the trust anchor extracted
	// from a bundle that itself was signature-verified.
	roots := x509.NewCertPool()
	roots.AddCert(rootFromBundle)
	if _, err := leafCert.Verify(x509.VerifyOptions{
		Roots:     roots,
		KeyUsages: []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth},
		DNSName:   "mgmt.lab.netbird.example.net",
	}); err != nil {
		t.Fatalf("end-to-end leaf-via-bundle-anchor verify: %v", err)
	}
}

// makeMinimalBundle returns a bundle with the smallest set of fields the
// ceremony test cares about. Mirrors fixtureBundle in bundle_test.go but
// is reproduced here to avoid an import cycle (x509ca → bundle works;
// bundle → x509ca would not).
func makeMinimalBundle(t *testing.T) *bundle.EnrollmentBundle {
	t.Helper()
	nonce := make([]byte, 16)
	if _, err := rand.Read(nonce); err != nil {
		t.Fatalf("gen nonce: %v", err)
	}
	peerPub := make([]byte, 32)
	if _, err := rand.Read(peerPub); err != nil {
		t.Fatalf("gen peer pubkey: %v", err)
	}
	return &bundle.EnrollmentBundle{
		Version:    1,
		DeviceID:   "ceremony-test-device",
		PeerPubkey: peerPub,
		ACLGroups:  []string{"test"},
		Site:       "lab",
		Nonce:      nonce,
		CAID:       "test-ca",
	}
}

// _ = bytes.Equal silences go vet on the unused-import-after-edit hazard;
// retain for readability in case the file gets refactored.
var _ = os.Stdout // keep "os" import alive in case we add file IO later
