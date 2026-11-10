package config

import "testing"

// A portal that signs for real must name its CA. A defaulted --ca-id would let
// a portal started without it mint leaves and write audit lines under another
// goatnet's CA instead of refusing to start.
func TestValidate_CertSignRequiresCAID(t *testing.T) {
	c := baseValidConfig()
	c.CertSignFake = false
	c.CABundleDir = "/etc/goat-zenoh-portal/ca"
	c.AgeIdentityPath = "/etc/goat-age/keys.txt"
	c.CARootCert = "/etc/goat-zenoh-portal/ca/onboarding-roots.pem"

	c.CertSignCAID = ""
	if err := c.Validate(); err == nil {
		t.Fatal("expected an error when cert-sign has no --ca-id")
	}

	// Whitespace is no CA id either: cert-sign would be handed a blank --ca-id.
	c.CertSignCAID = "  "
	if err := c.Validate(); err == nil {
		t.Fatal("expected an error when --ca-id is only whitespace")
	}

	c.CertSignCAID = "dev-galaxy-ca-ecdsa-2026-09-03"
	if err := c.Validate(); err != nil {
		t.Fatalf("validate with a CA id: %v", err)
	}
}

// Fake mode mints nothing, so it needs no CA id.
func TestValidate_CertSignFakeNeedsNoCAID(t *testing.T) {
	c := baseValidConfig()
	c.CertSignCAID = ""
	if err := c.Validate(); err != nil {
		t.Fatalf("validate in fake mode without a CA id: %v", err)
	}
}

// No goatnet's CA id is compiled in as the flag default.
func TestLoadFromFlags_CAIDHasNoDefault(t *testing.T) {
	t.Setenv("PORTAL_CA_ID", "")
	cfg, err := LoadFromFlags(nil)
	if err != nil {
		t.Fatalf("LoadFromFlags: %v", err)
	}
	if cfg.CertSignCAID != "" {
		t.Fatalf("CertSignCAID default = %q, want empty", cfg.CertSignCAID)
	}
}

// The Subject O defaults to empty (the role omits the flag; cert-sign keeps its
// own default) and is read from PORTAL_CERT_SIGN_ORG.
func TestLoadFromFlags_CertSignOrg(t *testing.T) {
	t.Setenv("PORTAL_CERT_SIGN_ORG", "")
	cfg, err := LoadFromFlags(nil)
	if err != nil {
		t.Fatalf("LoadFromFlags: %v", err)
	}
	if cfg.CertSignOrg != "" {
		t.Fatalf("CertSignOrg default = %q, want empty", cfg.CertSignOrg)
	}

	t.Setenv("PORTAL_CERT_SIGN_ORG", "Example Org")
	cfg, err = LoadFromFlags(nil)
	if err != nil {
		t.Fatalf("LoadFromFlags: %v", err)
	}
	if cfg.CertSignOrg != "Example Org" {
		t.Fatalf("CertSignOrg from env = %q, want %q", cfg.CertSignOrg, "Example Org")
	}

	cfg, err = LoadFromFlags([]string{"--cert-org=Flag Org"})
	if err != nil {
		t.Fatalf("LoadFromFlags: %v", err)
	}
	if cfg.CertSignOrg != "Flag Org" {
		t.Fatalf("CertSignOrg from flag = %q, want %q", cfg.CertSignOrg, "Flag Org")
	}
}
