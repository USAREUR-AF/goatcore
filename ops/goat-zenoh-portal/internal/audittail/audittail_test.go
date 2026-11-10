package audittail

import (
	"context"
	"encoding/base64"
	"encoding/xml"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/pgtest"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

func TestVendorOfKey(t *testing.T) {
	cases := map[string]string{
		"aardwolf/hello/v1":       "aardwolf",
		"aardwolf/observations/x": "aardwolf",
		"":                        "",
		"no-slash":                "",
		"/leading-slash":          "",
	}
	for in, want := range cases {
		if got := vendorOfKey(in); got != want {
			t.Errorf("vendorOfKey(%q) = %q, want %q", in, got, want)
		}
	}
}

func TestIsHelloWorld(t *testing.T) {
	helloPayload := func(s string) string {
		return base64.StdEncoding.EncodeToString([]byte(s))
	}
	tests := []struct {
		name   string
		ev     auditEvent
		prefix string
		want   bool
	}{
		{
			name:   "canonical hello",
			ev:     auditEvent{KeyExpr: "aardwolf/hello/v1", PayloadBytesB64: helloPayload("hello from aardwolf at 1716340000.123")},
			prefix: "aardwolf",
			want:   true,
		},
		{
			name:   "wrong key, hello payload",
			ev:     auditEvent{KeyExpr: "aardwolf/observations/x", PayloadBytesB64: helloPayload("hello from aardwolf at 1716340000.123")},
			prefix: "aardwolf",
			want:   false,
		},
		{
			name:   "right key, wrong vendor in payload",
			ev:     auditEvent{KeyExpr: "aardwolf/hello/v1", PayloadBytesB64: helloPayload("hello from otter at 1716340000.123")},
			prefix: "aardwolf",
			want:   false,
		},
		{
			name:   "right key, garbage payload",
			ev:     auditEvent{KeyExpr: "aardwolf/hello/v1", PayloadBytesB64: helloPayload("custom hello msg")},
			prefix: "aardwolf",
			want:   false,
		},
		{
			name:   "right key, no inline payload",
			ev:     auditEvent{KeyExpr: "aardwolf/hello/v1", PayloadBytesB64: ""},
			prefix: "aardwolf",
			want:   false,
		},
		{
			name:   "payload with trailing whitespace",
			ev:     auditEvent{KeyExpr: "aardwolf/hello/v1", PayloadBytesB64: helloPayload("hello from aardwolf at 1716340000.123\n")},
			prefix: "aardwolf",
			want:   true,
		},
	}
	for _, tc := range tests {
		if got := isHelloWorld(&tc.ev, tc.prefix); got != tc.want {
			t.Errorf("%s: got %v, want %v", tc.name, got, tc.want)
		}
	}
}

func TestParseTS(t *testing.T) {
	// Reference value: time.Date(2026,5,21,14,0,0,0,time.UTC).Unix()
	const want = int64(1779372000)
	if got := parseTS("2026-05-21T14:00:00Z"); got != want {
		t.Errorf("RFC3339 parseTS = %d, want %d", got, want)
	}
	if got := parseTS("2026-05-21T14:00:00.123Z"); got != want {
		t.Errorf("RFC3339 millis parseTS = %d, want %d (sub-second floor)", got, want)
	}
	if got := parseTS(""); got != 0 {
		t.Errorf("empty parseTS = %d, want 0", got)
	}
	if got := parseTS("not a date"); got != 0 {
		t.Errorf("garbage parseTS = %d, want 0", got)
	}
}

// fakeS3 mimics MinIO well enough to drive Tailer.PullOnce end-to-end.
// Ignores SigV4 verification — we trust the signing happened by
// observing that the request reached us with the right key.
type fakeS3 struct {
	bucket  string
	objects map[string]string // key → NDJSON body
	failGet map[string]bool   // key → serve 500 instead of the body
}

func (f *fakeS3) handler() http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		// path is /<bucket> for list; /<bucket>/<key> for get.
		p := strings.TrimPrefix(r.URL.Path, "/")
		if p == f.bucket {
			// list-type=2
			q := r.URL.Query()
			if q.Get("list-type") != "2" {
				http.Error(w, "unsupported", 400)
				return
			}
			prefix := q.Get("prefix")
			startAfter := q.Get("start-after")
			keys := make([]string, 0, len(f.objects))
			for k := range f.objects {
				if !strings.HasPrefix(k, prefix) {
					continue
				}
				if k <= startAfter {
					continue
				}
				keys = append(keys, k)
			}
			// sort
			for i := 1; i < len(keys); i++ {
				for j := i; j > 0 && keys[j-1] > keys[j]; j-- {
					keys[j-1], keys[j] = keys[j], keys[j-1]
				}
			}
			type content struct {
				Key string `xml:"Key"`
			}
			body := struct {
				XMLName     xml.Name  `xml:"ListBucketResult"`
				IsTruncated string    `xml:"IsTruncated"`
				Contents    []content `xml:"Contents"`
			}{IsTruncated: "false"}
			for _, k := range keys {
				body.Contents = append(body.Contents, content{Key: k})
			}
			w.Header().Set("Content-Type", "application/xml")
			b, _ := xml.Marshal(body)
			_, _ = w.Write(b)
			return
		}
		if strings.HasPrefix(p, f.bucket+"/") {
			key := strings.TrimPrefix(p, f.bucket+"/")
			if f.failGet[key] {
				http.Error(w, "injected failure", 500)
				return
			}
			if body, ok := f.objects[key]; ok {
				w.Header().Set("Content-Type", "application/x-ndjson")
				_, _ = w.Write([]byte(body))
				return
			}
			http.NotFound(w, r)
			return
		}
		http.NotFound(w, r)
	})
}

func openReg(t *testing.T) *registry.Registry {
	t.Helper()
	r, err := registry.Open(pgtest.DSN(t))
	if err != nil {
		t.Fatalf("open reg: %v", err)
	}
	t.Cleanup(func() { _ = r.Close() })
	return r
}

func TestPullOnceReplayThenIncremental(t *testing.T) {
	b64 := func(s string) string { return base64.StdEncoding.EncodeToString([]byte(s)) }
	mkEvent := func(vendor, keyExpr, ts, payload string) string {
		return fmt.Sprintf(`{"vendor":"%s","key_expr":"%s","ts":"%s","payload_hash":"sha256:x","payload_len":%d,"payload_bytes_b64":"%s"}`,
			vendor, keyExpr, ts, len(payload), b64(payload))
	}

	// Historical batch — landed before the portal first booted.
	hist := mkEvent("aardwolf", "aardwolf/hello/v1", "2026-05-20T10:00:00Z",
		"hello from aardwolf at 1716200000.000")
	histKey := "site-A/2026/05/20/10/1716199000000-1.ndjson"

	// Newer batch with two events: otter hello, then aardwolf real.
	live := mkEvent("otter", "otter/hello/v1", "2026-05-21T11:00:00Z",
		"hello from otter at 1716300000.000") + "\n" +
		mkEvent("aardwolf", "aardwolf/observations/wind/v1", "2026-05-21T11:00:30Z",
			"observed: 12 knots")
	liveKey := "site-A/2026/05/21/11/1716291600000-2.ndjson"

	srv := httptest.NewServer((&fakeS3{
		bucket: "audit-integrity",
		objects: map[string]string{
			histKey: hist,
			liveKey: live,
		},
	}).handler())
	defer srv.Close()

	r := openReg(t)
	ctx := context.Background()
	for _, p := range []string{"aardwolf", "otter"} {
		if err := r.CreateVendor(ctx, &registry.Vendor{Prefix: p, APIKeyHash: "h"}); err != nil {
			t.Fatalf("create %s: %v", p, err)
		}
	}

	tl := New(Config{
		Endpoint:     srv.URL,
		Bucket:       "audit-integrity",
		Region:       "us-east-1",
		AccessKey:    "key",
		SecretKey:    "secret",
		SubscriberID: "site-A",
	}, r)
	if !tl.Enabled() {
		t.Fatalf("tailer should be enabled")
	}

	// Replay: cursor empty → pulls both historical + live.
	n, cur, err := tl.PullOnce(ctx)
	if err != nil {
		t.Fatalf("first pull: %v", err)
	}
	if n != 3 {
		t.Fatalf("first pull events = %d, want 3", n)
	}
	if cur != liveKey {
		t.Fatalf("cursor after first pull = %q, want %q", cur, liveKey)
	}

	rows, _ := r.ListProgress(ctx, true)
	by := map[string]registry.VendorProgressRow{}
	for _, row := range rows {
		by[row.Prefix] = row
	}
	if got := by["aardwolf"]; got.L3At == nil {
		t.Errorf("aardwolf L3At should be set (replay hello)")
	}
	if got := by["aardwolf"]; got.L4At == nil {
		t.Errorf("aardwolf L4At should be set (live real publish)")
	}
	if got := by["otter"]; got.L3At == nil {
		t.Errorf("otter L3At should be set")
	}
	if got := by["otter"]; got.L4At != nil {
		t.Errorf("otter L4At should still be nil; got %v", got.L4At)
	}

	// Second pull — cursor caught up; no new objects.
	n2, cur2, err := tl.PullOnce(ctx)
	if err != nil {
		t.Fatalf("second pull: %v", err)
	}
	if n2 != 0 {
		t.Fatalf("second pull events = %d, want 0", n2)
	}
	if cur2 != cur {
		t.Fatalf("cursor moved on idempotent re-pull: %q → %q", cur, cur2)
	}
}

func TestPullSkipsSyntheticAndUnknownVendors(t *testing.T) {
	b64 := func(s string) string { return base64.StdEncoding.EncodeToString([]byte(s)) }
	mkEv := func(vendor, keyExpr, payload string) string {
		ts := time.Now().UTC().Format(time.RFC3339)
		return fmt.Sprintf(`{"vendor":"%s","key_expr":"%s","ts":"%s","payload_bytes_b64":"%s"}`,
			vendor, keyExpr, ts, b64(payload))
	}
	body := strings.Join([]string{
		mkEv("aardwolf", "aardwolf/hello/v1", "hello from aardwolf at 1716200000.000"),
		mkEv("notional", "notional/x/y/v1", "synthetic test"),
		mkEv("ghost", "ghost/x/y/v1", "no vendor row exists for this prefix"),
	}, "\n")

	srv := httptest.NewServer((&fakeS3{
		bucket: "audit-integrity",
		objects: map[string]string{
			"site-A/2026/05/21/11/1.ndjson": body,
		},
	}).handler())
	defer srv.Close()

	r := openReg(t)
	ctx := context.Background()
	if err := r.CreateVendor(ctx, &registry.Vendor{Prefix: "aardwolf", APIKeyHash: "h"}); err != nil {
		t.Fatalf("create aardwolf: %v", err)
	}
	if err := r.CreateVendor(ctx, &registry.Vendor{Prefix: "notional", APIKeyHash: "h", Synthetic: true}); err != nil {
		t.Fatalf("create notional: %v", err)
	}

	tl := New(Config{
		Endpoint:     srv.URL,
		Bucket:       "audit-integrity",
		Region:       "us-east-1",
		AccessKey:    "k",
		SecretKey:    "s",
		SubscriberID: "site-A",
	}, r)

	_, _, err := tl.PullOnce(ctx)
	if err != nil {
		t.Fatalf("pull: %v", err)
	}
	rows, _ := r.ListProgress(ctx, true)
	by := map[string]registry.VendorProgressRow{}
	for _, row := range rows {
		by[row.Prefix] = row
	}
	if got := by["aardwolf"]; got.L3At == nil {
		t.Errorf("aardwolf L3 should be set")
	}
	if got := by["notional"]; got.L3At != nil || got.L4At != nil {
		t.Errorf("notional should be skipped (synthetic); L3=%v L4=%v", got.L3At, got.L4At)
	}
	if _, present := by["ghost"]; present {
		t.Errorf("ghost should not appear (no vendor row)")
	}
}

func TestDisabledTailerIsNoop(t *testing.T) {
	tl := New(Config{}, nil)
	if tl.Enabled() {
		t.Fatalf("empty config should be disabled")
	}
}

func TestPullOnceRepullDoesNotDoubleCount(t *testing.T) {
	b64 := func(s string) string { return base64.StdEncoding.EncodeToString([]byte(s)) }
	mkEvent := func(keyExpr, ts, payload string) string {
		return fmt.Sprintf(`{"vendor":"aardwolf","key_expr":"%s","ts":"%s","payload_bytes_b64":"%s"}`,
			keyExpr, ts, b64(payload))
	}
	body := strings.Join([]string{
		mkEvent("aardwolf/hello/v1", "2026-05-21T11:00:00Z", "hello from aardwolf at 1716289200.000"),
		mkEvent("aardwolf/observations/wind/v1", "2026-05-21T11:00:10Z", "observed: 12 knots"),
		mkEvent("aardwolf/observations/wind/v1", "2026-05-21T11:00:20Z", "observed: 14 knots"),
	}, "\n")
	key := "site-A/2026/05/21/11/1716291600000-1.ndjson"

	srv := httptest.NewServer((&fakeS3{
		bucket:  "audit-integrity",
		objects: map[string]string{key: body},
	}).handler())
	defer srv.Close()

	r := openReg(t)
	ctx := context.Background()
	if err := r.CreateVendor(ctx, &registry.Vendor{Prefix: "aardwolf", APIKeyHash: "h"}); err != nil {
		t.Fatalf("create: %v", err)
	}
	tl := New(Config{
		Endpoint:     srv.URL,
		Bucket:       "audit-integrity",
		Region:       "us-east-1",
		AccessKey:    "k",
		SecretKey:    "s",
		SubscriberID: "site-A",
	}, r)

	if _, _, err := tl.PullOnce(ctx); err != nil {
		t.Fatalf("first pull: %v", err)
	}
	rows, _ := r.ListProgress(ctx, true)
	// DistinctTopics is no longer a registry column — it comes from the
	// activity source, so the registry-level assertion is the per-org
	// publish total.
	if rows[0].PublishCount != 3 {
		t.Fatalf("first pull activity = count %d, want 3", rows[0].PublishCount)
	}

	// Same object content is still in the bucket; the cursor (committed
	// in the same tx as the counts) keeps it from re-applying.
	n2, cur2, err := tl.PullOnce(ctx)
	if err != nil {
		t.Fatalf("second pull: %v", err)
	}
	if n2 != 0 || cur2 != key {
		t.Fatalf("second pull = (%d, %q), want (0, %q)", n2, cur2, key)
	}
	rows, _ = r.ListProgress(ctx, true)
	if rows[0].PublishCount != 3 {
		t.Fatalf("re-pull double-counted: PublishCount = %d, want 3", rows[0].PublishCount)
	}
}

func TestPullOnceMidPullFailureKeepsCompletedObjects(t *testing.T) {
	b64 := func(s string) string { return base64.StdEncoding.EncodeToString([]byte(s)) }
	mkEvent := func(keyExpr, ts, payload string) string {
		return fmt.Sprintf(`{"vendor":"aardwolf","key_expr":"%s","ts":"%s","payload_bytes_b64":"%s"}`,
			keyExpr, ts, b64(payload))
	}
	key1 := "site-A/2026/05/21/11/1716291600000-1.ndjson"
	key2 := "site-A/2026/05/21/12/1716295200000-1.ndjson"
	fake := &fakeS3{
		bucket: "audit-integrity",
		objects: map[string]string{
			key1: mkEvent("aardwolf/observations/wind/v1", "2026-05-21T11:00:00Z", "observed: 12 knots"),
			key2: mkEvent("aardwolf/observations/wind/v1", "2026-05-21T12:00:00Z", "observed: 14 knots"),
		},
		failGet: map[string]bool{key2: true},
	}
	srv := httptest.NewServer(fake.handler())
	defer srv.Close()

	r := openReg(t)
	ctx := context.Background()
	if err := r.CreateVendor(ctx, &registry.Vendor{Prefix: "aardwolf", APIKeyHash: "h"}); err != nil {
		t.Fatalf("create: %v", err)
	}
	tl := New(Config{
		Endpoint:     srv.URL,
		Bucket:       "audit-integrity",
		Region:       "us-east-1",
		AccessKey:    "k",
		SecretKey:    "s",
		SubscriberID: "site-A",
	}, r)

	// key1 applies and commits (data + cursor in one tx); key2's GET
	// fails, so the pull errors out with the cursor parked at key1.
	if _, _, err := tl.PullOnce(ctx); err == nil {
		t.Fatalf("pull should fail on key2's injected 500")
	}
	c, err := r.GetAuditCursor(ctx, "site-A")
	if err != nil {
		t.Fatalf("get cursor: %v", err)
	}
	if c.LastKey != key1 {
		t.Fatalf("cursor = %q, want %q (last completed object)", c.LastKey, key1)
	}
	rows, _ := r.ListProgress(ctx, true)
	if rows[0].PublishCount != 1 {
		t.Fatalf("PublishCount = %d, want 1 (key1 only)", rows[0].PublishCount)
	}

	// Next tick: only key2 replays, and lands exactly once.
	fake.failGet[key2] = false
	n, cur, err := tl.PullOnce(ctx)
	if err != nil {
		t.Fatalf("retry pull: %v", err)
	}
	if n != 1 || cur != key2 {
		t.Fatalf("retry pull = (%d, %q), want (1, %q)", n, cur, key2)
	}
	rows, _ = r.ListProgress(ctx, true)
	if rows[0].PublishCount != 2 {
		t.Fatalf("PublishCount = %d, want 2 (no double count)", rows[0].PublishCount)
	}
}
