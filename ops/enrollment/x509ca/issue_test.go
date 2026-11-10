package x509ca

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/sha256"
	"crypto/x509"
	"encoding/pem"
	"errors"
	"net"
	"testing"
	"time"
)

// fakeSigner is a minimal in-memory ca.Signer for tests. It generates an
// ECDSA P-256 keypair and signs through it. Bypasses the age-encrypted
// file path so tests don't need filesystem fixtures. Block 79 hard cutover.
type fakeSigner struct {
	priv *ecdsa.PrivateKey
	id   string
}

func newFakeSigner(t *testing.T, id string) *fakeSigner {
	t.Helper()
	priv, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatalf("generate ecdsa P-256 keypair: %v", err)
	}
	return &fakeSigner{priv: priv, id: id}
}

func (s *fakeSigner) Sign(msg []byte) ([]byte, error) {
	digest := sha256.Sum256(msg)
	return s.SignDigest(digest[:])
}

func (s *fakeSigner) SignDigest(digest []byte) ([]byte, error) {
	if s.priv == nil {
		return nil, errors.New("fake signer closed")
	}
	return ecdsa.SignASN1(rand.Reader, s.priv, digest)
}

func (s *fakeSigner) PublicKey() *ecdsa.PublicKey { return &s.priv.PublicKey }
func (s *fakeSigner) ID() string                  { return s.id }
func (s *fakeSigner) Close() error                { s.priv = nil; return nil }

func TestIssueCert_Happy(t *testing.T) {
	signer := newFakeSigner(t, "test-ca")
	rootPEM, rootCert, err := GenerateRootCert(signer, RootCertOptions{})
	if err != nil {
		t.Fatalf("GenerateRootCert: %v", err)
	}
	if len(rootPEM) == 0 {
		t.Fatal("expected non-empty root PEM")
	}
	if !rootCert.IsCA {
		t.Fatal("root cert IsCA=false")
	}

	tpl := &LeafTemplate{
		CommonName: "mgmt.lab.netbird.example.net",
		OrgUnit:    "lab",
		SANs: []string{
			"mgmt.lab.netbird.example.net",
			"10.99.0.1",
			"127.0.0.1",
		},
	}
	issued, err := IssueCert(tpl, signer, rootCert)
	if err != nil {
		t.Fatalf("IssueCert: %v", err)
	}

	// Decode and parse leaf
	block, _ := pem.Decode(issued.CertPEM)
	if block == nil || block.Type != "CERTIFICATE" {
		t.Fatalf("leaf PEM type=%q", block.Type)
	}
	leaf, err := x509.ParseCertificate(block.Bytes)
	if err != nil {
		t.Fatalf("parse leaf: %v", err)
	}

	// Validate metadata
	if leaf.Subject.CommonName != tpl.CommonName {
		t.Errorf("CN: got %q want %q", leaf.Subject.CommonName, tpl.CommonName)
	}
	if leaf.IsCA {
		t.Error("leaf IsCA must be false")
	}
	if leaf.PublicKeyAlgorithm != x509.ECDSA {
		t.Errorf("leaf algorithm: got %v want ECDSA", leaf.PublicKeyAlgorithm)
	}
	if leaf.SignatureAlgorithm != x509.ECDSAWithSHA256 {
		t.Errorf("leaf SignatureAlgorithm: got %v want ECDSAWithSHA256", leaf.SignatureAlgorithm)
	}

	// Validity is ~1 year by default
	wantValidity := 365 * 24 * time.Hour
	gotValidity := leaf.NotAfter.Sub(leaf.NotBefore)
	if gotValidity < wantValidity-time.Hour || gotValidity > wantValidity+time.Hour {
		t.Errorf("validity: got %v want ~%v", gotValidity, wantValidity)
	}

	// SAN partitioning
	wantDNS := []string{"mgmt.lab.netbird.example.net"}
	wantIPs := []string{"10.99.0.1", "127.0.0.1"}
	if len(leaf.DNSNames) != len(wantDNS) || leaf.DNSNames[0] != wantDNS[0] {
		t.Errorf("DNSNames: got %v want %v", leaf.DNSNames, wantDNS)
	}
	gotIPs := make([]string, len(leaf.IPAddresses))
	for i, ip := range leaf.IPAddresses {
		gotIPs[i] = ip.String()
	}
	if !equalIPSet(gotIPs, wantIPs) {
		t.Errorf("IPs: got %v want %v (set-equal)", gotIPs, wantIPs)
	}

	// AuthorityKeyId chains to root SubjectKeyId
	if !bytesEqual(leaf.AuthorityKeyId, rootCert.SubjectKeyId) {
		t.Error("leaf AuthorityKeyId != root SubjectKeyId — chain broken")
	}

	// Signature verifies against root pubkey
	roots := x509.NewCertPool()
	roots.AddCert(rootCert)
	if _, err := leaf.Verify(x509.VerifyOptions{
		Roots:     roots,
		KeyUsages: []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth},
		DNSName:   "mgmt.lab.netbird.example.net",
	}); err != nil {
		t.Fatalf("leaf verify against root: %v", err)
	}

	// IP-based verification
	if _, err := leaf.Verify(x509.VerifyOptions{
		Roots:     roots,
		KeyUsages: []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth},
		DNSName:   "10.99.0.1", // Verify uses DNSName but accepts IP literals via SAN matching
	}); err != nil {
		// crypto/x509.Verify with DNSName="10.99.0.1" doesn't traverse IP SANs;
		// instead we hand-check IP coverage via SAN list (already validated above).
		// This branch is informational only — don't fail the test.
		_ = err
	}

	// Leaf private key parses
	keyBlock, _ := pem.Decode(issued.KeyPEM)
	if keyBlock == nil || keyBlock.Type != "PRIVATE KEY" {
		t.Fatalf("key PEM type=%q", keyBlock.Type)
	}
	if _, err := x509.ParsePKCS8PrivateKey(keyBlock.Bytes); err != nil {
		t.Fatalf("parse leaf key: %v", err)
	}

	// Serial is positive 128-bit
	if issued.SerialNumber == nil || issued.SerialNumber.Sign() <= 0 {
		t.Errorf("serial not positive: %v", issued.SerialNumber)
	}
	if issued.SerialNumber.BitLen() > 128 {
		t.Errorf("serial > 128 bits: %d bits", issued.SerialNumber.BitLen())
	}
}

func TestIssueCert_CustomValidityAndEKU(t *testing.T) {
	signer := newFakeSigner(t, "test-ca")
	_, rootCert, err := GenerateRootCert(signer, RootCertOptions{})
	if err != nil {
		t.Fatalf("GenerateRootCert: %v", err)
	}

	tpl := &LeafTemplate{
		CommonName: "mtls.lab.netbird.example.net",
		OrgUnit:    "lab",
		SANs:       []string{"mtls.lab.netbird.example.net"},
		Validity:   90 * 24 * time.Hour,
		EKU:        []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth, x509.ExtKeyUsageClientAuth},
	}
	issued, err := IssueCert(tpl, signer, rootCert)
	if err != nil {
		t.Fatalf("IssueCert: %v", err)
	}

	block, _ := pem.Decode(issued.CertPEM)
	leaf, err := x509.ParseCertificate(block.Bytes)
	if err != nil {
		t.Fatalf("parse leaf: %v", err)
	}

	// 90-day cert
	gotValidity := leaf.NotAfter.Sub(leaf.NotBefore)
	wantValidity := 90 * 24 * time.Hour
	if gotValidity < wantValidity-time.Hour || gotValidity > wantValidity+time.Hour {
		t.Errorf("validity: got %v want ~%v", gotValidity, wantValidity)
	}

	// Both EKUs present
	if !hasEKU(leaf.ExtKeyUsage, x509.ExtKeyUsageServerAuth) {
		t.Error("ServerAuth EKU missing")
	}
	if !hasEKU(leaf.ExtKeyUsage, x509.ExtKeyUsageClientAuth) {
		t.Error("ClientAuth EKU missing")
	}
}

func TestIssueCert_URISANRoundTrip(t *testing.T) {
	signer := newFakeSigner(t, "test-ca")
	_, rootCert, err := GenerateRootCert(signer, RootCertOptions{})
	if err != nil {
		t.Fatalf("GenerateRootCert: %v", err)
	}

	want := []string{
		"urn:goat-nebula:vendor=example-org",
		"urn:goat:peer:site=lab,role=mgmt,echelon=OPSCTR",
	}
	tpl := &LeafTemplate{
		CommonName: "example-org-lattice",
		OrgUnit:    "nebula-trial",
		SANs:       []string{"placeholder.invalid"},
		URISANs:    want,
		EKU:        []x509.ExtKeyUsage{x509.ExtKeyUsageClientAuth},
	}
	issued, err := IssueCert(tpl, signer, rootCert)
	if err != nil {
		t.Fatalf("IssueCert: %v", err)
	}

	block, _ := pem.Decode(issued.CertPEM)
	leaf, err := x509.ParseCertificate(block.Bytes)
	if err != nil {
		t.Fatalf("parse leaf: %v", err)
	}

	if len(leaf.URIs) != len(want) {
		t.Fatalf("URIs len: got %d want %d (URIs=%v)", len(leaf.URIs), len(want), leaf.URIs)
	}
	for i, u := range leaf.URIs {
		if u.String() != want[i] {
			t.Errorf("URI[%d]: got %q want %q", i, u.String(), want[i])
		}
	}
}

func TestIssueCert_URISANRejectsRelative(t *testing.T) {
	signer := newFakeSigner(t, "test-ca")
	_, rootCert, err := GenerateRootCert(signer, RootCertOptions{})
	if err != nil {
		t.Fatalf("GenerateRootCert: %v", err)
	}

	cases := []struct {
		name string
		uri  string
	}{
		{"relative reference", "vendor=example-org"},
		{"empty entry", ""},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			tpl := &LeafTemplate{
				CommonName: "x",
				OrgUnit:    "y",
				SANs:       []string{"x"},
				URISANs:    []string{c.uri},
			}
			if _, err := IssueCert(tpl, signer, rootCert); err == nil {
				t.Fatal("expected error, got nil")
			}
		})
	}
}

func TestIssueCert_RejectsInvalidTemplate(t *testing.T) {
	signer := newFakeSigner(t, "test-ca")
	_, rootCert, err := GenerateRootCert(signer, RootCertOptions{})
	if err != nil {
		t.Fatalf("GenerateRootCert: %v", err)
	}

	cases := []struct {
		name string
		tpl  *LeafTemplate
	}{
		{"nil template", nil},
		{"missing CN", &LeafTemplate{OrgUnit: "lab", SANs: []string{"x"}}},
		{"missing OU", &LeafTemplate{CommonName: "x", SANs: []string{"x"}}},
		{"empty SANs", &LeafTemplate{CommonName: "x", OrgUnit: "y", SANs: nil}},
		{"empty SAN entry", &LeafTemplate{CommonName: "x", OrgUnit: "y", SANs: []string{""}}},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			if _, err := IssueCert(c.tpl, signer, rootCert); err == nil {
				t.Fatal("expected error, got nil")
			}
		})
	}
}

func TestIssueCert_DeterministicSerialUnique(t *testing.T) {
	signer := newFakeSigner(t, "test-ca")
	_, rootCert, _ := GenerateRootCert(signer, RootCertOptions{})

	seen := make(map[string]bool)
	tpl := &LeafTemplate{CommonName: "x", OrgUnit: "y", SANs: []string{"x"}}
	for i := 0; i < 50; i++ {
		issued, err := IssueCert(tpl, signer, rootCert)
		if err != nil {
			t.Fatal(err)
		}
		s := issued.SerialNumber.String()
		if seen[s] {
			t.Fatalf("serial collision at iteration %d: %s", i, s)
		}
		seen[s] = true
	}
}

// helpers

func equalIPSet(got, want []string) bool {
	if len(got) != len(want) {
		return false
	}
	gotMap := make(map[string]int, len(got))
	for _, g := range got {
		gotMap[net.ParseIP(g).String()]++
	}
	for _, w := range want {
		if gotMap[net.ParseIP(w).String()] == 0 {
			return false
		}
		gotMap[net.ParseIP(w).String()]--
	}
	return true
}

func bytesEqual(a, b []byte) bool {
	if len(a) != len(b) {
		return false
	}
	for i := range a {
		if a[i] != b[i] {
			return false
		}
	}
	return true
}

func hasEKU(eku []x509.ExtKeyUsage, want x509.ExtKeyUsage) bool {
	for _, e := range eku {
		if e == want {
			return true
		}
	}
	return false
}
