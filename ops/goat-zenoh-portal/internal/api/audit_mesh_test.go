package api

import (
	"context"
	"encoding/json"
	"encoding/pem"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/config"
)

// Model the non-admin mesh boundary: the caller can dial only the portal.
// Storage remains reachable from the portal, using its own S3 credentials.
func TestAuditMetadataWithoutClientStorageReachability(t *testing.T) {
	now := time.Now().UTC()
	stem := "site/" + now.Format("2006/01/02/15/") + fmt.Sprint(now.UnixMilli())
	keys := []string{stem + "-1.ndjson", stem + "-2-v2-meta.ndjson"}
	var storageReads atomic.Int64
	store := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		storageReads.Add(1)
		if !strings.HasPrefix(r.Header.Get("Authorization"), "AWS4-HMAC-SHA256 ") {
			t.Error("storage request did not use server-side S3 authorization")
			w.WriteHeader(http.StatusForbidden)
			return
		}
		if r.URL.Path == "/audit-integrity" {
			fmt.Fprintf(w, "<ListBucketResult><IsTruncated>false</IsTruncated><Contents><Key>%s</Key></Contents><Contents><Key>%s</Key></Contents></ListBucketResult>", keys[0], keys[1])
			return
		}
		record := map[string]any{
			"vendor": "slot", "key_expr": "slot/data", "ts": now.Format(time.RFC3339Nano),
			"payload_hash": "sha256:synthetic", "payload_len": 7,
		}
		if r.URL.Path == "/audit-integrity/"+keys[0] {
			record["payload_bytes_b64"] = "cHJpdmF0ZQ=="
		} else if r.URL.Path == "/audit-integrity/"+keys[1] {
			record["schema_version"] = 2
			record["sample_kind"] = "put"
		} else {
			w.WriteHeader(http.StatusNotFound)
			return
		}
		_ = json.NewEncoder(w).Encode(record)
	}))
	defer store.Close()
	ca := filepath.Join(t.TempDir(), "storage-ca.pem")
	if err := os.WriteFile(ca, pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: store.Certificate().Raw}), 0600); err != nil {
		t.Fatal(err)
	}
	s := authTestServer(t, &config.Config{
		OperatorAPIKeys: map[string]string{"operator": "fixture-key"},
		AuditS3Endpoint: store.URL, AuditS3CABundle: ca, AuditS3Bucket: "audit-integrity",
		AuditS3Region: "us-east-1", AuditS3AccessKey: "fixture", AuditS3SecretKey: "fixture",
		AuditSubscriberID: "site",
	})
	portal := httptest.NewTLSServer(s.Routes())
	defer portal.Close()
	portalURL, err := url.Parse(portal.URL)
	if err != nil {
		t.Fatal(err)
	}
	var blocked atomic.Int64
	transport := portal.Client().Transport.(*http.Transport).Clone()
	defer transport.CloseIdleConnections()
	transport.DialContext = func(ctx context.Context, network, address string) (net.Conn, error) {
		if address != portalURL.Host {
			blocked.Add(1)
			return nil, fmt.Errorf("fixture mesh policy blocks storage access")
		}
		return (&net.Dialer{}).DialContext(ctx, network, address)
	}
	client := &http.Client{Transport: transport, Timeout: 5 * time.Second, CheckRedirect: func(*http.Request, []*http.Request) error {
		return http.ErrUseLastResponse
	}}
	if response, err := client.Get(store.URL); err == nil {
		response.Body.Close()
		t.Fatal("caller unexpectedly reached storage")
	}
	get := func(path string) []byte {
		t.Helper()
		req, err := http.NewRequest(http.MethodGet, portal.URL+path, nil)
		if err != nil {
			t.Fatal(err)
		}
		req.Header.Set("Authorization", "Bearer fixture-key")
		resp, err := client.Do(req)
		if err != nil {
			t.Fatal(err)
		}
		defer resp.Body.Close()
		body, err := io.ReadAll(resp.Body)
		if err != nil || resp.StatusCode != http.StatusOK {
			t.Fatalf("portal read: status=%d err=%v body=%s", resp.StatusCode, err, body)
		}
		return body
	}
	var listing auditPresignResponse
	if err := json.Unmarshal(get("/api/audit/presign?projection=1&since=10m"), &listing); err != nil {
		t.Fatal(err)
	}
	if len(listing.Objects) != 2 || listing.Truncated {
		t.Fatalf("unexpected listing: %+v", listing)
	}
	for _, object := range listing.Objects {
		u, err := url.Parse(object.URL)
		if err != nil || u.IsAbs() || u.Host != "" || u.Path != "/api/audit/object" {
			t.Fatalf("object escaped the portal: %q", object.URL)
		}
		body := get(object.URL)
		if strings.Contains(string(body), "payload_bytes_b64") || strings.Contains(string(body), "cHJpdmF0ZQ") {
			t.Fatal("payload crossed the metadata route")
		}
		if !strings.Contains(string(body), "sha256:synthetic") || !strings.Contains(string(body), "slot/data") {
			t.Fatal("metadata needed to confirm publication is missing")
		}
	}
	if storageReads.Load() != 3 || blocked.Load() != 1 {
		t.Fatalf("unexpected paths: storage reads=%d blocked caller dials=%d", storageReads.Load(), blocked.Load())
	}
	rows, err := s.reg.AuditEvents(context.Background(), "audit.metadata.read", 10)
	if err != nil || len(rows) != 2 || rows[0].Actor != "operator" || rows[1].Actor != "operator" {
		t.Fatalf("missing operator read attribution: rows=%+v err=%v", rows, err)
	}
}
