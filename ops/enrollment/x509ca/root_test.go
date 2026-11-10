package x509ca

import (
	"crypto/x509"
	"encoding/pem"
	"strings"
	"testing"
	"time"
)

func TestGenerateRootCert_Defaults(t *testing.T) {
	signer := newFakeSigner(t, "test-ca")
	pemBytes, parsed, err := GenerateRootCert(signer, RootCertOptions{})
	if err != nil {
		t.Fatalf("GenerateRootCert: %v", err)
	}

	// PEM is well-formed
	block, _ := pem.Decode(pemBytes)
	if block == nil || block.Type != "CERTIFICATE" {
		t.Fatalf("PEM type=%q (want CERTIFICATE)", block.Type)
	}

	// Defaults
	if parsed.Subject.CommonName != DefaultRootCommonName {
		t.Errorf("CN: got %q", parsed.Subject.CommonName)
	}
	if !parsed.IsCA {
		t.Error("IsCA must be true")
	}
	if parsed.MaxPathLen != 0 || !parsed.MaxPathLenZero {
		t.Errorf("MaxPathLen: got %d zero=%v want 0/true (single-tier per ADR 0407)", parsed.MaxPathLen, parsed.MaxPathLenZero)
	}
	if parsed.KeyUsage&x509.KeyUsageCertSign == 0 {
		t.Error("KeyUsageCertSign missing")
	}
	if parsed.KeyUsage&x509.KeyUsageCRLSign == 0 {
		t.Error("KeyUsageCRLSign missing")
	}

	// 10-year default
	gotValidity := parsed.NotAfter.Sub(parsed.NotBefore)
	wantValidity := 10 * 365 * 24 * time.Hour
	if gotValidity < wantValidity-time.Hour {
		t.Errorf("validity: got %v want ~%v", gotValidity, wantValidity)
	}

	// Self-signed: Issuer == Subject
	if parsed.Issuer.CommonName != parsed.Subject.CommonName {
		t.Errorf("self-signed: Issuer.CN=%q Subject.CN=%q", parsed.Issuer.CommonName, parsed.Subject.CommonName)
	}

	// SubjectKeyId is present and 20 bytes (SHA-1 of SPKI)
	if len(parsed.SubjectKeyId) != 20 {
		t.Errorf("SubjectKeyId length: got %d want 20", len(parsed.SubjectKeyId))
	}
}

func TestGenerateRootCert_CustomOptions(t *testing.T) {
	signer := newFakeSigner(t, "test-ca")
	opts := RootCertOptions{
		Organization: "Test Org",
		CommonName:   "Test Root",
		OrgUnit:      "test-ou",
		Validity:     5 * 365 * 24 * time.Hour,
	}
	_, parsed, err := GenerateRootCert(signer, opts)
	if err != nil {
		t.Fatalf("GenerateRootCert: %v", err)
	}

	if parsed.Subject.Organization[0] != "Test Org" {
		t.Errorf("O: got %v", parsed.Subject.Organization)
	}
	if parsed.Subject.CommonName != "Test Root" {
		t.Errorf("CN: got %q", parsed.Subject.CommonName)
	}
	if parsed.Subject.OrganizationalUnit[0] != "test-ou" {
		t.Errorf("OU: got %v", parsed.Subject.OrganizationalUnit)
	}
}

func TestLoadRootCert_RoundTrip(t *testing.T) {
	signer := newFakeSigner(t, "test-ca")
	pemBytes, original, err := GenerateRootCert(signer, RootCertOptions{})
	if err != nil {
		t.Fatalf("GenerateRootCert: %v", err)
	}

	loaded, err := LoadRootCert(pemBytes)
	if err != nil {
		t.Fatalf("LoadRootCert: %v", err)
	}

	if !loaded.Equal(original) {
		t.Error("loaded cert not equal to original")
	}
}

func TestLoadRootCert_RejectsNonCert(t *testing.T) {
	cases := []struct {
		name string
		pem  string
	}{
		{"empty", ""},
		{"not pem", "not a cert"},
		{"wrong type", "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIA==\n-----END PRIVATE KEY-----\n"},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			if _, err := LoadRootCert([]byte(c.pem)); err == nil {
				t.Fatal("expected error, got nil")
			}
		})
	}
}

func TestLoadRootCert_RejectsNonCA(t *testing.T) {
	// Generate a leaf cert and try to load it as a root.
	signer := newFakeSigner(t, "test-ca")
	_, rootCert, _ := GenerateRootCert(signer, RootCertOptions{})
	tpl := &LeafTemplate{CommonName: "leaf", OrgUnit: "x", SANs: []string{"leaf"}}
	issued, err := IssueCert(tpl, signer, rootCert)
	if err != nil {
		t.Fatalf("IssueCert: %v", err)
	}
	if _, err := LoadRootCert(issued.CertPEM); err == nil || !strings.Contains(err.Error(), "not a CA") {
		t.Fatalf("expected 'not a CA' error, got: %v", err)
	}
}
