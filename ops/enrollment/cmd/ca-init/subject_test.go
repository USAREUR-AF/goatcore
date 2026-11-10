package main

import (
	"crypto/x509"
	"encoding/pem"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/enrollment/x509ca"
)

func readCert(t *testing.T, path string) *x509.Certificate {
	t.Helper()
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	block, _ := pem.Decode(raw)
	if block == nil {
		t.Fatalf("%s holds no PEM block", path)
	}
	cert, err := x509.ParseCertificate(block.Bytes)
	if err != nil {
		t.Fatal(err)
	}
	return cert
}

// TestSubjectFlagsNameTheRootAndIntermediate: --org sets the Subject O of the
// root and the intermediate and --intermediate-cn the intermediate's CN, so a
// ceremony does not take either from a default built into the binary. Without
// the flags the x509ca defaults apply.
func TestSubjectFlagsNameTheRootAndIntermediate(t *testing.T) {
	bin := buildCAInit(t)
	for _, tc := range []struct {
		name, org, intCN, wantOrg, wantIntCN string
	}{
		{"stated", "Example Org", "Example Intermediate CA", "Example Org", "Example Intermediate CA"},
		{"defaults", "", "", x509ca.DefaultOrganization, x509ca.DefaultIntermediateCommonName},
	} {
		t.Run(tc.name, func(t *testing.T) {
			work := t.TempDir()
			intCert := filepath.Join(work, "int.pem")
			extra := []string{"--gen-intermediate-cert", intCert, "--intermediate-out-key", filepath.Join(work, "int.key.age")}
			if tc.org != "" {
				extra = append(extra, "--org", tc.org, "--intermediate-cn", tc.intCN)
			}
			_, rootPath, _ := mint(t, bin, work, "subject-ca", "subject-net Root CA", 2, extra...)
			root := readCert(t, rootPath)
			if got := strings.Join(root.Subject.Organization, ","); got != tc.wantOrg {
				t.Errorf("root O %q, want %q", got, tc.wantOrg)
			}
			inter := readCert(t, intCert)
			if got := strings.Join(inter.Subject.Organization, ","); got != tc.wantOrg {
				t.Errorf("intermediate O %q, want %q", got, tc.wantOrg)
			}
			if inter.Subject.CommonName != tc.wantIntCN {
				t.Errorf("intermediate CN %q, want %q", inter.Subject.CommonName, tc.wantIntCN)
			}
		})
	}
}

// TestVerifyFlagsADeclaredSharedRootCN: --shared-root-cn names a subject that
// several roots of one CA carry; --verify refuses a root with it, and passes
// the same root when the flag names another subject.
func TestVerifyFlagsADeclaredSharedRootCN(t *testing.T) {
	bin := buildCAInit(t)
	key, root, pub := mint(t, bin, t.TempDir(), "shared-ca", "Example Shared Root CA", 2)
	args := []string{"--verify", "--out", key, "--root-cert", root, "--public-key", pub}
	out, err := command(t, bin, append(args, "--shared-root-cn", "Example Shared Root CA")...).CombinedOutput()
	if err == nil {
		t.Fatalf("verify passed a root carrying the declared shared CN:\n%s", out)
	}
	if !strings.Contains(string(out), "root CN is the legacy shared default") {
		t.Errorf("verify did not name the CN problem:\n%s", out)
	}
	if out, err := command(t, bin, append(args, "--shared-root-cn", "Another Root CA")...).CombinedOutput(); err != nil {
		t.Fatalf("verify failed a root whose CN is not the declared shared one: %v\n%s", err, out)
	}
	if out, err := command(t, bin, args...).CombinedOutput(); err != nil {
		t.Fatalf("verify without --shared-root-cn failed a root that names itself: %v\n%s", err, out)
	}
}
