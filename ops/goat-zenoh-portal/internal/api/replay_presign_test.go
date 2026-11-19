package api

import (
	"context"
	"encoding/json"
	"encoding/pem"
	"fmt"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/config"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"
)

func TestReplayGateAndMetadataProjection(t *testing.T) {
	var reads atomic.Int64
	key := "site/2026/10/26/12/1793000000000-1.ndjson"
	body := `{"vendor":"slot","key_expr":"slot/data","ts":"2026-10-26T12:00:00Z","payload_hash":"legacy","payload_len":7,"payload_bytes_b64":"cHJpdmF0ZQ==","unexpected":"private"}`
	store := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		reads.Add(1)
		if r.URL.Path == "/audit-integrity" {
			fmt.Fprintf(w, "<ListBucketResult><IsTruncated>false</IsTruncated><Contents><Key>%s</Key></Contents></ListBucketResult>", key)
		} else {
			fmt.Fprintln(w, body)
		}
	}))
	defer store.Close()
	ca := filepath.Join(t.TempDir(), "ca.pem")
	if err := os.WriteFile(ca, pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: store.Certificate().Raw}), 0600); err != nil {
		t.Fatal(err)
	}
	cfg := &config.Config{OperatorAPIKeys: map[string]string{"operator": "fixture-key"}, AuditS3Endpoint: store.URL, AuditS3CABundle: ca, AuditS3Bucket: "audit-integrity", AuditS3Region: "us-east-1", AuditS3AccessKey: "fixture", AuditS3SecretKey: "fixture", AuditSubscriberID: "site"}
	s := authTestServer(t, cfg)
	mux := s.Routes()
	vendorKey := seedPrincipal(t, s.reg, "history-vendor", false, "")
	for _, path := range []string{"/api/audit/presign?projection=1", "/api/audit/object?key=" + url.QueryEscape(key), "/api/replay/presign"} {
		for _, credential := range []struct {
			token  string
			status int
		}{{vendorKey, 403}, {"invalid-operator", 401}} {
			r := httptest.NewRequest("GET", path, nil)
			r.Header.Set("Authorization", "Bearer "+credential.token)
			w := httptest.NewRecorder()
			mux.ServeHTTP(w, r)
			if w.Code != credential.status || reads.Load() != 0 || strings.Contains(w.Body.String(), "slot/data") {
				t.Fatalf("denied history route reached data: %s status %d", path, w.Code)
			}
		}
	}
	delete(cfg.OperatorAPIKeys, "operator")
	r := httptest.NewRequest("GET", "/api/audit/object?key="+url.QueryEscape(key), nil)
	r.Header.Set("Authorization", "Bearer fixture-key")
	revoked := httptest.NewRecorder()
	mux.ServeHTTP(revoked, r)
	if revoked.Code != 401 || reads.Load() != 0 {
		t.Fatal("revoked operator read history")
	}
	cfg.OperatorAPIKeys["operator"] = "fixture-key"
	request := func(path, token string, fn http.HandlerFunc) *httptest.ResponseRecorder {
		r := httptest.NewRequest("GET", path, nil)
		if token != "" {
			r.Header.Set("Authorization", "Bearer "+token)
		}
		w := httptest.NewRecorder()
		noStore(s.requireOperator(fn))(w, r)
		return w
	}
	for _, fn := range []http.HandlerFunc{s.handleAuditPresign, s.handleAuditObject, s.handleReplayPresign} {
		w := request("/", "", fn)
		if w.Code != 401 && w.Code != 403 {
			t.Fatal(w.Code)
		}
	}
	if reads.Load() != 0 {
		t.Fatal("unauthorized storage read")
	}
	if w := request("/api/replay/presign", "fixture-key", s.handleReplayPresign); w.Code != 503 {
		t.Fatal(w.Code)
	}
	if w := request("/api/audit/presign", "fixture-key", s.handleAuditPresign); w.Code != 426 {
		t.Fatal(w.Code)
	}
	w := request("/api/audit/presign?projection=1", "fixture-key", s.handleAuditPresign)
	if w.Code != 200 {
		t.Fatalf("listing %d %s", w.Code, w.Body.String())
	}
	var listing auditPresignResponse
	if err := json.Unmarshal(w.Body.Bytes(), &listing); err != nil {
		t.Fatal(err)
	}
	if len(listing.Objects) != 1 || listing.Objects[0].Source != "legacy-projection" || strings.Contains(listing.Objects[0].URL, "X-Amz") {
		t.Fatal("raw history URL")
	}
	w = request(listing.Objects[0].URL, "fixture-key", s.handleAuditObject)
	if w.Code != 200 || strings.Contains(w.Body.String(), "payload_bytes_b64") || strings.Contains(w.Body.String(), "private") || strings.Contains(w.Body.String(), "cHJpdmF0ZQ") {
		t.Fatalf("projection %d %s", w.Code, w.Body.String())
	}
	if w.Header().Get("Cache-Control") != "no-store" {
		t.Fatal("cacheable metadata")
	}
	rows, err := s.reg.AuditEvents(context.Background(), "audit.metadata.read", 10)
	if err != nil || len(rows) != 1 || rows[0].Actor != "operator" {
		t.Fatal("missing read attribution", err)
	}
	before := reads.Load()
	w = request("/api/audit/object?key="+url.QueryEscape(strings.Replace(key, "site/", "foreign/", 1)), "fixture-key", s.handleAuditObject)
	if w.Code != 400 || reads.Load() != before {
		t.Fatal("foreign key reached storage")
	}
	body += "\n{broken"
	w = request(listing.Objects[0].URL, "fixture-key", s.handleAuditObject)
	if w.Code != 502 || strings.Contains(w.Body.String(), "slot/data") {
		t.Fatal("partial success")
	}
	body = `{"payload_hash":{"payload_bytes_b64":"BODY_SENTINEL"}}`
	w = request(listing.Objects[0].URL, "fixture-key", s.handleAuditObject)
	if w.Code != 502 || strings.Contains(w.Body.String(), "BODY_SENTINEL") {
		t.Fatal("nested body crossed projection")
	}
	for _, invalid := range []string{
		`{"schema_version":2,"key_expr":"SCHEMA_SENTINEL"}`,
		`{"schema_version":99,"key_expr":"SCHEMA_SENTINEL"}`,
		`{"key_expr":"` + strings.Repeat("x", 16385) + `"}`,
		`{"key_expr":"` + strings.Repeat("x", 4<<20) + `"}`,
	} {
		body = invalid
		w = request(listing.Objects[0].URL, "fixture-key", s.handleAuditObject)
		if w.Code != 502 || strings.Contains(w.Body.String(), "SCHEMA_SENTINEL") || w.Body.Len() > 1024 {
			t.Fatal("invalid projection crossed whole-object boundary", w.Code)
		}
	}
	if err := s.reg.Close(); err != nil {
		t.Fatal(err)
	}
	before = reads.Load()
	w = httptest.NewRecorder()
	s.handleAuditObject(w, httptest.NewRequest("GET", listing.Objects[0].URL, nil))
	if w.Code != 500 || reads.Load() != before {
		t.Fatal("access log failure allowed read")
	}
}
