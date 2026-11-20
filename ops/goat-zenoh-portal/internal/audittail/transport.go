package audittail

import (
	"crypto/tls"
	"crypto/x509"
	"fmt"
	"net/http"
	"net/url"
	"os"
	"time"
)

// StorageClient uses only the declared organization trust bundle. Never follow
// a storage redirect: its target has not been authorized to receive the request.
func StorageClient(cfg Config) (*http.Client, error) {
	u, err := url.Parse(cfg.Endpoint)
	if err != nil || u.Scheme != "https" || u.Host == "" || u.User != nil || u.RawQuery != "" || u.Fragment != "" || (u.Path != "" && u.Path != "/") {
		return nil, fmt.Errorf("storage endpoint must be an HTTPS authority")
	}
	pem, err := os.ReadFile(cfg.CABundle)
	if err != nil {
		return nil, fmt.Errorf("read storage CA bundle: %w", err)
	}
	roots := x509.NewCertPool()
	if !roots.AppendCertsFromPEM(pem) {
		return nil, fmt.Errorf("storage CA bundle contains no certificates")
	}
	transport := http.DefaultTransport.(*http.Transport).Clone()
	transport.TLSClientConfig = &tls.Config{RootCAs: roots, MinVersion: tls.VersionTLS12}
	return &http.Client{Transport: transport, Timeout: 60 * time.Second, CheckRedirect: func(*http.Request, []*http.Request) error { return fmt.Errorf("storage redirects are forbidden") }}, nil
}
