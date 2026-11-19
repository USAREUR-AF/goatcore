package api_test

// Cache classes on the wire: validators and 304 on the documented reads,
// no-store on everything that returns a credential or a person, and the
// body caps that answer 413 instead of truncating.

import (
	"bytes"
	"net/http"
	"strings"
	"testing"
)

func TestCache_ListRevalidates(t *testing.T) {
	ts := newTestServer(t)
	c := &http.Client{}
	url := ts.URL + "/api/vendors"
	resp := doRaw(t, c, "GET", url, "dev-operator-key", nil)
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("status = %d", resp.StatusCode)
	}
	tag := resp.Header.Get("ETag")
	if tag == "" || resp.Header.Get("Cache-Control") != "private, no-cache" {
		t.Fatalf("ETag = %q, Cache-Control = %q", tag, resp.Header.Get("Cache-Control"))
	}
	if !strings.Contains(resp.Header.Get("Vary"), "Authorization") {
		t.Errorf("Vary = %q", resp.Header.Get("Vary"))
	}
	if body := mustReadBody(t, resp); !strings.Contains(body, `"vendors"`) && !strings.Contains(body, `"orgs"`) && !strings.HasPrefix(body, "[") {
		t.Errorf("body = %q", body)
	}

	resp = doRaw(t, c, "GET", url, "dev-operator-key", map[string]string{"If-None-Match": tag})
	if resp.StatusCode != http.StatusNotModified {
		t.Fatalf("status = %d, want 304", resp.StatusCode)
	}
	if got := mustReadBody(t, resp); got != "" {
		t.Errorf("304 carried a body: %q", got)
	}
	if resp.Header.Get("ETag") != tag {
		t.Errorf("304 ETag = %q, want %q", resp.Header.Get("ETag"), tag)
	}
}

func TestCache_GzipTagIsDistinctAndRevalidates(t *testing.T) {
	ts := newTestServer(t)
	c := noAutoGzip()
	apiKey, _, _ := registerVendorAndTopic(t, ts.URL, c)
	// Enough topics to push the list past the gzip threshold.
	for i := 0; i < 8; i++ {
		resp := doJSON(t, c, "POST", ts.URL+"/api/vendors/caracal/topics", apiKey, map[string]any{
			"key_pattern": "caracal/tracks/lane" + itoa(int64(i)) + "/v1",
			"format":      "json",
			"description": strings.Repeat("a lane of tracks ", 8),
		})
		if resp.StatusCode != http.StatusCreated {
			t.Fatalf("create topic %d: %d", i, resp.StatusCode)
		}
		resp.Body.Close()
	}
	url := ts.URL + "/api/vendors/caracal/topics"
	identity := doRaw(t, c, "GET", url, apiKey, nil)
	gz := doRaw(t, c, "GET", url, apiKey, map[string]string{"Accept-Encoding": "gzip"})
	if gz.Header.Get("Content-Encoding") != "gzip" {
		t.Fatalf("list did not compress: %d bytes identity", len(mustReadBody(t, identity)))
	}
	it, gt := identity.Header.Get("ETag"), gz.Header.Get("ETag")
	if it == "" || gt == "" || it == gt || !strings.HasSuffix(gt, `-gz"`) {
		t.Errorf("identity ETag %q, gzip ETag %q", it, gt)
	}
	for _, tag := range []string{it, gt} {
		resp := doRaw(t, c, "GET", url, apiKey, map[string]string{"If-None-Match": tag, "Accept-Encoding": "gzip"})
		if resp.StatusCode != http.StatusNotModified {
			t.Errorf("If-None-Match %s: status %d, want 304", tag, resp.StatusCode)
		}
	}
}

func TestCache_ExportYAMLCharsetAndValidator(t *testing.T) {
	ts := newTestServer(t)
	c := &http.Client{}
	apiKey, _, _ := registerVendorAndTopic(t, ts.URL, c)
	resp := doRaw(t, c, "GET", ts.URL+"/api/vendors/caracal/topics/export?format=yaml", apiKey, nil)
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("status = %d", resp.StatusCode)
	}
	if ct := resp.Header.Get("Content-Type"); ct != "application/yaml; charset=utf-8" {
		t.Errorf("Content-Type = %q", ct)
	}
	if resp.Header.Get("ETag") == "" {
		t.Error("export carries no ETag")
	}
}

func TestCache_NeverStoredRoutes(t *testing.T) {
	ts := newTestServer(t)
	c := &http.Client{}
	// Each answers before its handler runs (401/403/404), so the class shows
	// on the header without a fixture behind the route.
	routes := []struct{ method, path string }{
		{"GET", "/api/whoami"},
		{"GET", "/api/access/roster"},
		{"GET", "/api/access/peer"},
		{"POST", "/api/access/peers"},
		{"GET", "/api/audit/presign"},
		{"GET", "/api/audit/object"},
		{"GET", "/api/replay/presign"},
		{"GET", "/api/admin/snapshot"},
		{"GET", "/api/admin/snapshot/meta"},
		{"GET", "/api/admin/vendor-activity"},
		{"GET", "/api/admin/vendor-activity/caracal/events"},
		{"GET", "/api/org-bundle/caracal/nope"},
		{"GET", "/api/vendor-bundle/caracal/nope"},
		{"POST", "/api/vendors"},
		{"POST", "/api/orgs"},
		{"POST", "/api/vendors/caracal/rotate"},
		{"POST", "/api/orgs/rotate"},
		{"POST", "/api/orgs/collect"},
		{"POST", "/api/machines/enroll"},
		{"POST", "/api/machines/enroll-bulk"},
		{"POST", "/api/machines/anchor/mesh-key"}, // A person's details, a sponsorship lookup by email, a data-role or
		// audience resolution and a recipient certificate.
		{"GET", "/api/orgs/sponsorship"},
		{"GET", "/api/lookup/person-presence"},
		{"GET", "/api/access/data-roles"},
		{"GET", "/api/access/audience"},
		{"GET", "/api/access/recipient-cert"},
	}
	for _, rt := range routes {
		resp := doRaw(t, c, rt.method, ts.URL+rt.path, "", nil)
		if cc := resp.Header.Get("Cache-Control"); cc != "no-store" {
			t.Errorf("%s %s: Cache-Control = %q (status %d)", rt.method, rt.path, cc, resp.StatusCode)
		}
		if resp.Header.Get("ETag") != "" {
			t.Errorf("%s %s: carries an ETag", rt.method, rt.path)
		}
		resp.Body.Close()
	}
}

func TestCache_CredentialResponsesAreNeverStored(t *testing.T) {
	ts := newTestServer(t)
	c := &http.Client{}
	resp := doJSON(t, c, "POST", ts.URL+"/api/vendors", "dev-operator-key", map[string]any{
		"prefix": "nostore", "display_name": "No Store", "auto_approve": true,
	})
	if resp.StatusCode != http.StatusCreated || resp.Header.Get("Cache-Control") != "no-store" {
		t.Errorf("create vendor: status %d, Cache-Control %q", resp.StatusCode, resp.Header.Get("Cache-Control"))
	}
	resp.Body.Close()
}

func TestBodyCap_OversizeJSONIs413(t *testing.T) {
	ts := newTestServer(t)
	c := &http.Client{}
	big := `{"prefix":"big","display_name":"` + strings.Repeat("x", 1<<20) + `"}`
	req, _ := http.NewRequest("POST", ts.URL+"/api/vendors", bytes.NewReader([]byte(big)))
	req.Header.Set("Authorization", "Bearer dev-operator-key")
	req.Header.Set("Content-Type", "application/json")
	resp, err := c.Do(req)
	if err != nil {
		t.Fatalf("post: %v", err)
	}
	assertProblem(t, resp, http.StatusRequestEntityTooLarge, "payload_too_large")
}

// A truncated import used to parse and, in replace mode, retire every topic
// the cut removed. Now it is refused whole.
func TestBodyCap_OversizeReplaceImportRetiresNothing(t *testing.T) {
	ts := newTestServer(t)
	c := &http.Client{}
	apiKey, _, keyPattern := registerVendorAndTopic(t, ts.URL, c)

	var doc strings.Builder
	doc.WriteString(`{"org":"caracal","topics":[{"key_pattern":"` + keyPattern + `","format":"protobuf:Track"}`)
	for doc.Len() < (1<<20)+64 {
		doc.WriteString(`,{"key_pattern":"caracal/pad/` + itoa(int64(doc.Len())) + `","format":"json","description":"` + strings.Repeat("p", 200) + `"}`)
	}
	doc.WriteString(`]}`)
	req, _ := http.NewRequest("POST", ts.URL+"/api/vendors/caracal/topics/import?mode=replace", strings.NewReader(doc.String()))
	req.Header.Set("Authorization", "Bearer "+apiKey)
	req.Header.Set("Content-Type", "application/json")
	resp, err := c.Do(req)
	if err != nil {
		t.Fatalf("import: %v", err)
	}
	assertProblem(t, resp, http.StatusRequestEntityTooLarge, "payload_too_large")

	list := doRaw(t, c, "GET", ts.URL+"/api/vendors/caracal/topics", apiKey, nil)
	body := mustReadBody(t, list)
	if !strings.Contains(body, keyPattern) || strings.Contains(body, `"retired_at"`) {
		t.Errorf("topic list after a refused import: %s", body)
	}
}

func TestCache_SchemaFetchRevalidatesOnDigest(t *testing.T) {
	ts := newTestServer(t)
	c := &http.Client{}
	apiKey, topicID, keyPattern := registerVendorAndTopic(t, ts.URL, c)
	body, ctype := uploadSchemaForm(t, validTrackFDSBytes(t, ""), "v1", "backward", "protobuf-fds")
	resp := doUpload(t, c, ts.URL+"/api/vendors/caracal/topics/"+itoa(topicID)+"/schemas", apiKey, body, ctype)
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("upload: %d %s", resp.StatusCode, mustReadBody(t, resp))
	}
	resp.Body.Close()

	url := ts.URL + "/api/schemas/" + keyPattern
	first := doRaw(t, c, "GET", url, apiKey, nil)
	if first.StatusCode != http.StatusOK {
		t.Fatalf("fetch: %d", first.StatusCode)
	}
	tag := first.Header.Get("ETag")
	if cd := first.Header.Get("Content-Disposition"); !strings.HasPrefix(cd, "attachment; filename=") || strings.ContainsAny(cd, "/*") {
		t.Errorf("Content-Disposition = %q", cd)
	}
	if tag != `"`+first.Header.Get("X-Schema-SHA256")+`"` || first.Header.Get("Cache-Control") != "private, no-cache" {
		t.Errorf("ETag = %q, X-Schema-SHA256 = %q, Cache-Control = %q", tag, first.Header.Get("X-Schema-SHA256"), first.Header.Get("Cache-Control"))
	}
	if first.Header.Get("Last-Modified") == "" || first.Header.Get("Content-Length") == "" {
		t.Errorf("Last-Modified = %q, Content-Length = %q", first.Header.Get("Last-Modified"), first.Header.Get("Content-Length"))
	}
	first.Body.Close()

	again := doRaw(t, c, "GET", url, apiKey, map[string]string{"If-None-Match": tag})
	if again.StatusCode != http.StatusNotModified {
		t.Errorf("If-None-Match: %d, want 304", again.StatusCode)
	}
	again.Body.Close()
	since := doRaw(t, c, "GET", url, apiKey, map[string]string{"If-Modified-Since": first.Header.Get("Last-Modified")})
	if since.StatusCode != http.StatusNotModified {
		t.Errorf("If-Modified-Since: %d, want 304", since.StatusCode)
	}
	since.Body.Close()
}

func TestBodyCap_PublicRouteIs413(t *testing.T) {
	ts := newTestServer(t)
	c := &http.Client{}
	big := `{"kind":"zenohd","content":"` + strings.Repeat("x", 1<<20) + `"}`
	req, _ := http.NewRequest("POST", ts.URL+"/api/peering/lint", bytes.NewReader([]byte(big)))
	req.Header.Set("Content-Type", "application/json")
	resp, err := c.Do(req)
	if err != nil {
		t.Fatalf("post: %v", err)
	}
	assertProblem(t, resp, http.StatusRequestEntityTooLarge, "payload_too_large")
}

func TestCache_VaryNamesBothOnThe200AndThe304(t *testing.T) {
	ts := newTestServer(t)
	c := &http.Client{}
	url := ts.URL + "/api/vendors"
	first := doRaw(t, c, "GET", url, "dev-operator-key", map[string]string{"Accept-Encoding": "gzip"})
	vary := strings.Join(first.Header.Values("Vary"), ", ")
	if !strings.Contains(vary, "Authorization") || strings.Count(vary, "Accept-Encoding") != 1 {
		t.Errorf("200 Vary = %q", vary)
	}
	tag := first.Header.Get("ETag")
	first.Body.Close()
	again := doRaw(t, c, "GET", url, "dev-operator-key", map[string]string{"Accept-Encoding": "gzip", "If-None-Match": tag})
	vary = strings.Join(again.Header.Values("Vary"), ", ")
	if again.StatusCode != http.StatusNotModified || !strings.Contains(vary, "Authorization") || !strings.Contains(vary, "Accept-Encoding") {
		t.Errorf("304: status %d, Vary = %q", again.StatusCode, vary)
	}
	again.Body.Close()
}
