package probe

import (
	"context"
	"encoding/pem"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"
)

func TestServiceHTTPSVerifiesCertificate(t *testing.T) {
	server := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusOK)
	}))
	defer server.Close()
	p := &ServiceProbe{}
	if got := p.checkHTTP(context.Background(), ServiceCheck{Name: "untrusted", URL: server.URL}); got.Status != Fail {
		t.Fatalf("untrusted certificate passed: %+v", got)
	}
	// A private service can supply explicit trust roots without disabling
	// certificate or hostname validation.
	caFile := filepath.Join(t.TempDir(), "root.pem")
	if err := os.WriteFile(caFile, pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: server.Certificate().Raw}), 0600); err != nil {
		t.Fatal(err)
	}
	if got := p.checkHTTP(context.Background(), ServiceCheck{Name: "trusted", URL: server.URL, CAFile: caFile}); got.Status != Pass {
		t.Fatalf("configured CA did not authenticate service: %+v", got)
	}
	if got := p.checkHTTP(context.Background(), ServiceCheck{Name: "missing-ca", URL: server.URL, CAFile: caFile + ".missing"}); got.Status != Fail {
		t.Fatal("missing configured CA passed")
	}
}
