package main

import (
	"crypto/x509"
	"encoding/pem"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"filippo.io/age"

	"github.com/usareur-af/goatcore/ops/enrollment/ca"
	"github.com/usareur-af/goatcore/ops/enrollment/x509ca"
)

// newTestCA writes an age-encrypted CA key, its identity file and a
// self-signed root cert under dir, the three inputs cert-sign reads.
func newTestCA(t *testing.T, dir string) (rootCert, rootKey, identityPath string) {
	t.Helper()
	identity, err := age.GenerateX25519Identity()
	if err != nil {
		t.Fatal(err)
	}
	identityPath = filepath.Join(dir, "keys.txt")
	if err := os.WriteFile(identityPath, []byte(identity.String()+"\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	rootKey = filepath.Join(dir, "ca.key.age")
	if _, err := ca.GenerateAndWrite(rootKey, []age.Recipient{identity.Recipient()}); err != nil {
		t.Fatal(err)
	}
	signer, err := ca.NewAgeSigner("test-ca", rootKey, []age.Identity{identity})
	if err != nil {
		t.Fatal(err)
	}
	defer signer.Close() //nolint:errcheck // the root cert is complete before signer cleanup
	rootPEM, _, err := x509ca.GenerateRootCert(signer, x509ca.RootCertOptions{CommonName: "test-net Root CA"})
	if err != nil {
		t.Fatal(err)
	}
	rootCert = filepath.Join(dir, "test-ca.pem")
	if err := os.WriteFile(rootCert, rootPEM, 0o600); err != nil {
		t.Fatal(err)
	}
	return rootCert, rootKey, identityPath
}

// TestOrgFlagSetsTheLeafOrganization: --org is the Subject O of the leaf, so
// a CA's leaves carry the organisation its caller states and not one built
// into the binary; without it the x509ca default applies. The audit log is
// the file --audit-log names.
func TestOrgFlagSetsTheLeafOrganization(t *testing.T) {
	work := t.TempDir()
	rootCert, rootKey, identityPath := newTestCA(t, work)
	for _, tc := range []struct{ name, org, want string }{
		{"stated", "Example Org", "Example Org"},
		{"default", "", x509ca.DefaultOrganization},
	} {
		t.Run(tc.name, func(t *testing.T) {
			out := t.TempDir()
			certPath := filepath.Join(out, "leaf-cert.pem")
			auditLog := filepath.Join(out, "audit", "ca-audit.jsonl")
			err := run(rootCert, rootKey, identityPath, "", "site-a", tc.org, "leaf.example.test",
				24*time.Hour, "clientAuth", "auto", certPath, filepath.Join(out, "leaf-key.pem"),
				auditLog, "", "", []string{"leaf.example.test"}, nil)
			if err != nil {
				t.Fatalf("run: %v", err)
			}
			raw, err := os.ReadFile(certPath)
			if err != nil {
				t.Fatal(err)
			}
			block, _ := pem.Decode(raw)
			if block == nil {
				t.Fatal("leaf cert holds no PEM block")
			}
			leaf, err := x509.ParseCertificate(block.Bytes)
			if err != nil {
				t.Fatal(err)
			}
			if got := strings.Join(leaf.Subject.Organization, ","); got != tc.want {
				t.Errorf("leaf O %q, want %q", got, tc.want)
			}
			if got := strings.Join(leaf.Subject.OrganizationalUnit, ","); got != "site-a" {
				t.Errorf("leaf OU %q, want site-a", got)
			}
			logged, err := os.ReadFile(auditLog)
			if err != nil {
				t.Fatalf("the audit entry is not in the file --audit-log names: %v", err)
			}
			if !strings.Contains(string(logged), "leaf.example.test") {
				t.Errorf("audit entry does not name the leaf:\n%s", logged)
			}
		})
	}
}
