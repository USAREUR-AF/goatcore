package api

import (
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/config"
)

func TestRootCADownloadName(t *testing.T) {
	for in, want := range map[string]string{
		"galaxy-ca-root.pem": "galaxy-ca-root.pem",
		"  galaxy.pem  ":     "galaxy.pem",
		"":                   "root-ca.pem",
		"   ":                "root-ca.pem",
		"../etc/passwd":      "root-ca.pem",
		`dir\name.pem`:       "root-ca.pem",
		`bad"name.pem`:       "root-ca.pem",
	} {
		if got := rootCADownloadName(in); got != want {
			t.Errorf("rootCADownloadName(%q) = %q; want %q", in, got, want)
		}
	}
}

// The root-CA download is saved under the goatnet's in-tarball CA-root name.
func TestRootCABundle_ContentDispositionUsesRootCertName(t *testing.T) {
	dir := t.TempDir()
	pem := filepath.Join(dir, "root.pem")
	if err := os.WriteFile(pem, []byte("-----BEGIN CERTIFICATE-----\nAA==\n-----END CERTIFICATE-----\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	for name, want := range map[string]string{
		"galaxy-ca-root.pem": `inline; filename="galaxy-ca-root.pem"`,
		"":                   `inline; filename="root-ca.pem"`,
	} {
		s := &Server{cfg: &config.Config{CARootCert: pem, OnboardingRootCertName: name}}
		rec := httptest.NewRecorder()
		s.handleRootCABundle(rec, httptest.NewRequest(http.MethodGet, "/peering/root-ca.pem", nil))
		if rec.Code != http.StatusOK {
			t.Fatalf("status %d", rec.Code)
		}
		if got := rec.Header().Get("Content-Disposition"); got != want {
			t.Errorf("RootCertName %q: Content-Disposition = %q; want %q", name, got, want)
		}
	}
}
