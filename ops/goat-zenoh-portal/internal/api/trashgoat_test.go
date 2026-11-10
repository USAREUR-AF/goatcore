package api_test

import (
	"net/http"
	"net/http/httptest"
	"strconv"
	"strings"
	"testing"
)

// Trashgoat endpoints (operator-only). Lifecycle round-trip + auth
// gating + payload/cadence validation.

const sampleBytesPayload = `{"kind":"bytes","size_bytes":4096,"content":"random"}`
const sampleOneshot = `{"kind":"oneshot"}`
const sampleEvery = `{"kind":"every","interval_ms":1000}`
const sampleBurst = `{"kind":"burst","count":10,"spacing_ms":50,"then_idle":true}`
const sampleChunked = `{"kind":"chunked","total_size_bytes":12500000,"chunk_size_bytes":1500000,"content":"random"}`
const sampleJSONShape = `{"kind":"json_shape","template":{"id":"$uuid","ts":"$now_ms"}}`
const sampleProtobufShape = `{"kind":"protobuf_shape","message_type":"notional.tracks_demo.v1.Entity","fields":{}}`

func createSampleSpec(t *testing.T, ts *httptest.Server, slug string) map[string]any {
	t.Helper()
	c := &http.Client{}
	resp := doTrashgoatPostRaw(t, c, ts.URL+"/api/admin/trashgoat/specs", "dev-operator-key",
		`{"slug":"`+slug+`","description":"smoke","topic_pattern":"trashgoat/`+slug+`/v1",`+
			`"payload_spec":`+sampleBytesPayload+`,"cadence_spec":`+sampleOneshot+`}`)
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("create: status=%d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}
	return mustReadJSON(t, resp)
}

// doTrashgoatPostRaw posts a literal JSON body (avoids re-encoding the
// pre-built spec strings, which is closer to what the UI actually sends).
func doTrashgoatPostRaw(t *testing.T, c *http.Client, url, bearer, body string) *http.Response {
	t.Helper()
	return doRawJSON(t, c, "POST", url, bearer, body)
}

func doRawJSON(t *testing.T, c *http.Client, method, url, bearer, body string) *http.Response {
	t.Helper()
	var req *http.Request
	var err error
	if body == "" {
		req, err = http.NewRequest(method, url, nil)
	} else {
		req, err = http.NewRequest(method, url, strings.NewReader(body))
	}
	if err != nil {
		t.Fatalf("new req: %v", err)
	}
	if bearer != "" {
		req.Header.Set("Authorization", "Bearer "+bearer)
	}
	if body != "" {
		req.Header.Set("Content-Type", "application/json")
	}
	resp, err := c.Do(req)
	if err != nil {
		t.Fatalf("do: %v", err)
	}
	return resp
}

func TestTrashgoat_CreateAndListAndLifecycle(t *testing.T) {
	ts := newTestServer(t)
	c := &http.Client{}

	// 1. List empty.
	resp := doJSON(t, c, "GET", ts.URL+"/api/admin/trashgoat/specs", "dev-operator-key", nil)
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("list: status=%d", resp.StatusCode)
	}
	body := mustReadJSON(t, resp)
	if specs, ok := body["specs"].([]any); !ok || len(specs) != 0 {
		t.Errorf("list expected [], got %#v", body["specs"])
	}

	// 2. Create draft.
	got := createSampleSpec(t, ts, "first-shot")
	idF, ok := got["id"].(float64)
	if !ok || idF == 0 {
		t.Fatalf("create response missing id: %#v", got)
	}
	id := int(idF)
	if got["state"] != "draft" {
		t.Errorf("state = %v, want draft", got["state"])
	}

	// 3. Get.
	resp = doJSON(t, c, "GET", ts.URL+"/api/admin/trashgoat/specs/"+itoaInt(id), "dev-operator-key", nil)
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("get: status=%d", resp.StatusCode)
	}

	// 4. Activate.
	resp = doJSON(t, c, "POST", ts.URL+"/api/admin/trashgoat/specs/"+itoaInt(id)+"/activate", "dev-operator-key", nil)
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("activate: status=%d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}
	body = mustReadJSON(t, resp)
	if body["state"] != "active" {
		t.Errorf("state after activate = %v, want active", body["state"])
	}
	if body["last_activated_at"] == nil {
		t.Error("last_activated_at not stamped")
	}

	// 5. Activate again — 409 invalid_transition.
	resp = doJSON(t, c, "POST", ts.URL+"/api/admin/trashgoat/specs/"+itoaInt(id)+"/activate", "dev-operator-key", nil)
	if resp.StatusCode != http.StatusConflict {
		t.Errorf("re-activate: want 409, got %d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}

	// 6. Pause.
	resp = doJSON(t, c, "POST", ts.URL+"/api/admin/trashgoat/specs/"+itoaInt(id)+"/pause", "dev-operator-key", nil)
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("pause: status=%d", resp.StatusCode)
	}
	body = mustReadJSON(t, resp)
	if body["state"] != "paused" {
		t.Errorf("state after pause = %v, want paused", body["state"])
	}

	// 7. Cleanse — orthogonal; state stays paused.
	resp = doJSON(t, c, "POST", ts.URL+"/api/admin/trashgoat/specs/"+itoaInt(id)+"/cleanse", "dev-operator-key", nil)
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("cleanse: status=%d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}
	body = mustReadJSON(t, resp)
	if body["state"] != "paused" {
		t.Errorf("state after cleanse = %v, should remain paused", body["state"])
	}
	if body["last_cleansed_at"] == nil {
		t.Error("last_cleansed_at not stamped")
	}

	// 8. Retire.
	resp = doJSON(t, c, "POST", ts.URL+"/api/admin/trashgoat/specs/"+itoaInt(id)+"/retire", "dev-operator-key", nil)
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("retire: status=%d", resp.StatusCode)
	}
	body = mustReadJSON(t, resp)
	if body["state"] != "retired" {
		t.Errorf("state after retire = %v, want retired", body["state"])
	}

	// 9. Delete retired spec.
	resp = doJSON(t, c, "DELETE", ts.URL+"/api/admin/trashgoat/specs/"+itoaInt(id), "dev-operator-key", nil)
	if resp.StatusCode != http.StatusNoContent {
		t.Fatalf("delete: status=%d", resp.StatusCode)
	}

	// 10. Subsequent get → 404.
	resp = doJSON(t, c, "GET", ts.URL+"/api/admin/trashgoat/specs/"+itoaInt(id), "dev-operator-key", nil)
	if resp.StatusCode != http.StatusNotFound {
		t.Errorf("get after delete: want 404, got %d", resp.StatusCode)
	}
}

func TestTrashgoat_AuthOperatorOnly(t *testing.T) {
	ts := newTestServer(t)
	c := &http.Client{}

	// 1. Operator registers a vendor + grabs its vendor key (which the
	//    test uses to verify the vendor key is rejected on
	//    /api/admin/trashgoat/* endpoints).
	resp := doJSON(t, c, "POST", ts.URL+"/api/vendors", "dev-operator-key", map[string]any{
		"prefix":       "otter",
		"auto_approve": true,
	})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("register vendor: status=%d", resp.StatusCode)
	}
	regResp := mustReadJSON(t, resp)
	vendorKey, _ := regResp["api_key"].(string)
	if vendorKey == "" {
		t.Fatal("missing api_key")
	}

	// 2. No bearer → 401.
	resp, _ = http.Get(ts.URL + "/api/admin/trashgoat/specs")
	if resp.StatusCode != http.StatusUnauthorized {
		t.Errorf("no-auth list: want 401, got %d", resp.StatusCode)
	}

	// 3. Vendor key → 403.
	resp = doJSON(t, c, "GET", ts.URL+"/api/admin/trashgoat/specs", vendorKey, nil)
	if resp.StatusCode != http.StatusForbidden {
		t.Errorf("vendor-key list: want 403, got %d", resp.StatusCode)
	}

	// 4. Vendor key cannot create either.
	resp = doTrashgoatPostRaw(t, c, ts.URL+"/api/admin/trashgoat/specs", vendorKey,
		`{"slug":"x","description":"","topic_pattern":"trashgoat/x/v1","payload_spec":`+
			sampleBytesPayload+`,"cadence_spec":`+sampleOneshot+`}`)
	if resp.StatusCode != http.StatusForbidden {
		t.Errorf("vendor-key create: want 403, got %d", resp.StatusCode)
	}
}

func TestTrashgoat_InvalidPayloadAndCadenceShapes(t *testing.T) {
	ts := newTestServer(t)
	c := &http.Client{}

	cases := []struct {
		name       string
		payload    string
		cadence    string
		wantSubstr string
	}{
		{
			name:       "missing payload kind",
			payload:    `{"size_bytes":1024}`,
			cadence:    sampleOneshot,
			wantSubstr: "payload_spec.kind",
		},
		{
			name:       "unknown payload kind",
			payload:    `{"kind":"bogus"}`,
			cadence:    sampleOneshot,
			wantSubstr: "unknown kind",
		},
		{
			name:       "bytes size > 64 MiB",
			payload:    `{"kind":"bytes","size_bytes":99999999999}`,
			cadence:    sampleOneshot,
			wantSubstr: "size_bytes",
		},
		{
			name:       "chunked chunk > 4 MiB",
			payload:    `{"kind":"chunked","total_size_bytes":1048576,"chunk_size_bytes":99999999}`,
			cadence:    sampleOneshot,
			wantSubstr: "chunk_size_bytes",
		},
		{
			name:       "chunked chunk > total",
			payload:    `{"kind":"chunked","total_size_bytes":2048,"chunk_size_bytes":4096}`,
			cadence:    sampleOneshot,
			wantSubstr: "chunk_size_bytes",
		},
		{
			name:       "json_shape needs template object",
			payload:    `{"kind":"json_shape","template":"hello"}`,
			cadence:    sampleOneshot,
			wantSubstr: "template",
		},
		{
			name:       "protobuf_shape needs fully-qualified message type",
			payload:    `{"kind":"protobuf_shape","message_type":"Entity"}`,
			cadence:    sampleOneshot,
			wantSubstr: "message_type",
		},
		{
			name:       "cadence missing kind",
			payload:    sampleBytesPayload,
			cadence:    `{"interval_ms":100}`,
			wantSubstr: "cadence_spec.kind",
		},
		{
			name:       "cadence every needs positive interval",
			payload:    sampleBytesPayload,
			cadence:    `{"kind":"every","interval_ms":0}`,
			wantSubstr: "interval_ms",
		},
		{
			name:       "burst count must be positive",
			payload:    sampleBytesPayload,
			cadence:    `{"kind":"burst","count":0,"spacing_ms":10}`,
			wantSubstr: "count",
		},
		{
			name:       "content unknown directive",
			payload:    `{"kind":"bytes","size_bytes":1024,"content":"weird"}`,
			cadence:    sampleOneshot,
			wantSubstr: "content",
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			body := `{"slug":"x-` + slugify(tc.name) + `","description":"","topic_pattern":"trashgoat/x-` +
				slugify(tc.name) + `/v1","payload_spec":` + tc.payload + `,"cadence_spec":` + tc.cadence + `}`
			resp := doTrashgoatPostRaw(t, c, ts.URL+"/api/admin/trashgoat/specs", "dev-operator-key", body)
			if resp.StatusCode != http.StatusBadRequest {
				t.Fatalf("want 400, got %d body=%s", resp.StatusCode, mustReadBody(t, resp))
			}
			if got := mustReadBody(t, resp); !containsSubstr(got, tc.wantSubstr) {
				t.Errorf("body %q does not contain %q", got, tc.wantSubstr)
			}
		})
	}
}

func TestTrashgoat_AcceptsAllPayloadKinds(t *testing.T) {
	ts := newTestServer(t)
	c := &http.Client{}
	cases := []struct {
		slug    string
		payload string
		cadence string
	}{
		{"shot-bytes", sampleBytesPayload, sampleOneshot},
		{"shot-chunked", sampleChunked, sampleOneshot},
		{"shot-json", sampleJSONShape, sampleEvery},
		{"shot-proto", sampleProtobufShape, sampleBurst},
	}
	for _, tc := range cases {
		t.Run(tc.slug, func(t *testing.T) {
			body := `{"slug":"` + tc.slug + `","description":"","topic_pattern":"trashgoat/` + tc.slug +
				`/v1","payload_spec":` + tc.payload + `,"cadence_spec":` + tc.cadence + `}`
			resp := doTrashgoatPostRaw(t, c, ts.URL+"/api/admin/trashgoat/specs", "dev-operator-key", body)
			if resp.StatusCode != http.StatusCreated {
				t.Fatalf("create %s: status=%d body=%s", tc.slug, resp.StatusCode, mustReadBody(t, resp))
			}
		})
	}
}

func TestTrashgoat_DuplicateSlug409(t *testing.T) {
	ts := newTestServer(t)
	c := &http.Client{}

	createSampleSpec(t, ts, "dup")
	resp := doTrashgoatPostRaw(t, c, ts.URL+"/api/admin/trashgoat/specs", "dev-operator-key",
		`{"slug":"dup","description":"","topic_pattern":"trashgoat/dup/v1","payload_spec":`+
			sampleBytesPayload+`,"cadence_spec":`+sampleOneshot+`}`)
	if resp.StatusCode != http.StatusConflict {
		t.Errorf("dup slug: want 409, got %d", resp.StatusCode)
	}
}

func TestTrashgoat_PatchOnActiveRefused(t *testing.T) {
	ts := newTestServer(t)
	c := &http.Client{}
	got := createSampleSpec(t, ts, "patchme")
	id := int(got["id"].(float64))
	// activate
	resp := doJSON(t, c, "POST", ts.URL+"/api/admin/trashgoat/specs/"+itoaInt(id)+"/activate", "dev-operator-key", nil)
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("activate: status=%d", resp.StatusCode)
	}
	// patch → 409 immutable
	resp = doRawJSON(t, c, "PATCH", ts.URL+"/api/admin/trashgoat/specs/"+itoaInt(id), "dev-operator-key",
		`{"description":"new"}`)
	if resp.StatusCode != http.StatusConflict {
		t.Errorf("patch-on-active: want 409, got %d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}
}

func TestTrashgoat_DeleteOnDraftRefused(t *testing.T) {
	ts := newTestServer(t)
	c := &http.Client{}
	got := createSampleSpec(t, ts, "stickyone")
	id := int(got["id"].(float64))
	resp := doJSON(t, c, "DELETE", ts.URL+"/api/admin/trashgoat/specs/"+itoaInt(id), "dev-operator-key", nil)
	if resp.StatusCode != http.StatusConflict {
		t.Errorf("delete-draft: want 409, got %d", resp.StatusCode)
	}
}

func TestTrashgoat_PauseFromDraftRefused(t *testing.T) {
	ts := newTestServer(t)
	c := &http.Client{}
	got := createSampleSpec(t, ts, "draftpause")
	id := int(got["id"].(float64))
	resp := doJSON(t, c, "POST", ts.URL+"/api/admin/trashgoat/specs/"+itoaInt(id)+"/pause", "dev-operator-key", nil)
	if resp.StatusCode != http.StatusConflict {
		t.Errorf("pause-from-draft: want 409, got %d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}
}

func TestTrashgoat_PatchDescriptionOnDraftOK(t *testing.T) {
	ts := newTestServer(t)
	c := &http.Client{}
	got := createSampleSpec(t, ts, "patchok")
	id := int(got["id"].(float64))
	resp := doRawJSON(t, c, "PATCH", ts.URL+"/api/admin/trashgoat/specs/"+itoaInt(id), "dev-operator-key",
		`{"description":"updated text","notes":"experiment notes"}`)
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("patch: status=%d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}
	body := mustReadJSON(t, resp)
	if body["description"] != "updated text" {
		t.Errorf("description = %v", body["description"])
	}
	if body["notes"] != "experiment notes" {
		t.Errorf("notes = %v", body["notes"])
	}
}

// --- helpers ---------------------------------------------------------

func itoaInt(i int) string { return strconv.Itoa(i) }

func slugify(s string) string {
	out := make([]byte, 0, len(s))
	for i := 0; i < len(s); i++ {
		c := s[i]
		switch {
		case c >= 'a' && c <= 'z', c >= '0' && c <= '9':
			out = append(out, c)
		case c >= 'A' && c <= 'Z':
			out = append(out, c+('a'-'A'))
		case c == ' ' || c == '-' || c == '_':
			if len(out) > 0 && out[len(out)-1] != '-' {
				out = append(out, '-')
			}
		}
	}
	// trim trailing dash
	for len(out) > 0 && out[len(out)-1] == '-' {
		out = out[:len(out)-1]
	}
	if len(out) == 0 {
		return "x"
	}
	return string(out)
}

func containsSubstr(s, sub string) bool { return strings.Contains(s, sub) }
