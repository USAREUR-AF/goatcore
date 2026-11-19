package audittail

import (
	"context"
	"crypto/rand"
	"crypto/rsa"
	"crypto/tls"
	"crypto/x509"
	"encoding/pem"
	"math/big"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func fixtureCA(t *testing.T, s *httptest.Server) string {
	t.Helper()
	p := filepath.Join(t.TempDir(), "ca.pem")
	if err := os.WriteFile(p, pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: s.Certificate().Raw}), 0600); err != nil {
		t.Fatal(err)
	}
	return p
}
func TestStorageTLSRejectsHTTPMissingTrustAndWrongAuthority(t *testing.T) {
	s := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { w.WriteHeader(200) }))
	defer s.Close()
	ca := fixtureCA(t, s)
	for _, cfg := range []Config{{Endpoint: "http://127.0.0.1", CABundle: ca}, {Endpoint: s.URL}, {Endpoint: s.URL + "?secret=x", CABundle: ca}} {
		if _, err := StorageClient(cfg); err == nil {
			t.Fatal("accepted insecure storage configuration")
		}
	}
	c, err := StorageClient(Config{Endpoint: s.URL, CABundle: ca})
	if err != nil {
		t.Fatal(err)
	}
	resp, err := c.Get(s.URL)
	if err != nil {
		t.Fatal(err)
	}
	resp.Body.Close()
}

func TestMetadataFieldTypes(t *testing.T) {
	for _, name := range []string{"payload_hash", "key_expr", "namespace_slot", "schema_version", "payload_len"} {
		for _, bad := range []string{`{"payload_bytes_b64":"BODY_SENTINEL"}`, `[]`, `true`} {
			if validateMetadataField(name, []byte(bad)) == nil {
				t.Fatalf("accepted nested/wrong type in %s", name)
			}
		}
	}
}
func TestStorageRedirectRejected(t *testing.T) {
	var followed bool
	target := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { followed = true }))
	defer target.Close()
	source := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { http.Redirect(w, r, target.URL, 302) }))
	defer source.Close()
	client, err := StorageClient(Config{Endpoint: source.URL, CABundle: fixtureCA(t, source)})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := client.Get(source.URL); err == nil || followed {
		t.Fatal("storage redirect followed")
	}
}

func TestStorageExpiredCertificateRejected(t *testing.T) {
	key, err := rsa.GenerateKey(rand.Reader, 2048)
	if err != nil {
		t.Fatal(err)
	}
	cert := &x509.Certificate{SerialNumber: big.NewInt(1), NotBefore: time.Now().Add(-48 * time.Hour), NotAfter: time.Now().Add(-24 * time.Hour), IPAddresses: []net.IP{net.ParseIP("127.0.0.1")}, KeyUsage: x509.KeyUsageDigitalSignature | x509.KeyUsageCertSign, ExtKeyUsage: []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth}, IsCA: true, BasicConstraintsValid: true}
	der, err := x509.CreateCertificate(rand.Reader, cert, cert, &key.PublicKey, key)
	if err != nil {
		t.Fatal(err)
	}
	server := httptest.NewUnstartedServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { t.Error("expired certificate reached handler") }))
	server.TLS = &tls.Config{Certificates: []tls.Certificate{{Certificate: [][]byte{der}, PrivateKey: key}}}
	server.StartTLS()
	defer server.Close()
	client, err := StorageClient(Config{Endpoint: server.URL, CABundle: fixtureCA(t, server)})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := client.Get(server.URL); err == nil {
		t.Fatal("expired storage certificate accepted")
	}
}

// The storage certificate must name the configured host. httptest's
// certificate is valid for example.com and 127.0.0.1; both names below dial
// the same listener, so only the name differs between them.
func TestStorageWrongHostNameRejected(t *testing.T) {
	s := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { w.WriteHeader(200) }))
	defer s.Close()
	_, port, err := net.SplitHostPort(s.Listener.Addr().String())
	if err != nil {
		t.Fatal(err)
	}
	get := func(host string) error {
		endpoint := "https://" + net.JoinHostPort(host, port)
		c, err := StorageClient(Config{Endpoint: endpoint, CABundle: fixtureCA(t, s)})
		if err != nil {
			t.Fatal(err)
		}
		tr := c.Transport.(*http.Transport)
		tr.DialContext = func(ctx context.Context, network, _ string) (net.Conn, error) {
			return (&net.Dialer{}).DialContext(ctx, network, s.Listener.Addr().String())
		}
		resp, err := c.Get(endpoint)
		if err == nil {
			resp.Body.Close()
		}
		return err
	}
	if err := get("example.com"); err != nil {
		t.Fatalf("certificate name the fixture is valid for was refused: %v", err)
	}
	err = get("storage.invalid")
	if err == nil {
		t.Fatal("storage certificate for another host name accepted")
	}
	if !strings.Contains(err.Error(), "not storage.invalid") {
		t.Fatalf("refused for a reason other than the host name: %v", err)
	}
	t.Log(err)
}
