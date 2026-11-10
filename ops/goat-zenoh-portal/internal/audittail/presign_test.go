package audittail

import (
	"net/url"
	"strconv"
	"strings"
	"testing"
	"time"
)

func testPresigner() *Presigner {
	return NewPresigner(Config{
		Endpoint:     "http://198.18.0.5:9000",
		Bucket:       "audit-integrity",
		Region:       "us-east-1",
		AccessKey:    "AKIATEST",
		SecretKey:    "secrettestkey",
		SubscriberID: "site-A",
	})
}

func TestPresignGetURLShape(t *testing.T) {
	p := testPresigner()
	raw, err := p.PresignGet("site-A/2026/06/11/13/000-1.ndjson", 5*time.Minute)
	if err != nil {
		t.Fatalf("PresignGet: %v", err)
	}
	u, err := url.Parse(raw)
	if err != nil {
		t.Fatalf("parse presigned url %q: %v", raw, err)
	}
	if u.Scheme != "http" || u.Host != "198.18.0.5:9000" {
		t.Errorf("unexpected scheme/host: %s://%s", u.Scheme, u.Host)
	}
	// Path-style: /<bucket>/<key>, slashes preserved (not %2F).
	wantPath := "/audit-integrity/site-A/2026/06/11/13/000-1.ndjson"
	if u.Path != wantPath {
		t.Errorf("path = %q, want %q", u.Path, wantPath)
	}
	q := u.Query()
	for _, k := range []string{
		"X-Amz-Algorithm", "X-Amz-Credential", "X-Amz-Date",
		"X-Amz-Expires", "X-Amz-SignedHeaders", "X-Amz-Signature",
	} {
		if q.Get(k) == "" {
			t.Errorf("missing query param %s", k)
		}
	}
	if q.Get("X-Amz-Algorithm") != "AWS4-HMAC-SHA256" {
		t.Errorf("algorithm = %q", q.Get("X-Amz-Algorithm"))
	}
	if q.Get("X-Amz-Expires") != "300" {
		t.Errorf("expires = %q, want 300", q.Get("X-Amz-Expires"))
	}
	if !strings.HasPrefix(q.Get("X-Amz-Credential"), "AKIATEST/") {
		t.Errorf("credential = %q", q.Get("X-Amz-Credential"))
	}
	// Signature is 64 lowercase hex chars.
	sig := q.Get("X-Amz-Signature")
	if len(sig) != 64 {
		t.Errorf("signature length = %d, want 64 (%q)", len(sig), sig)
	}
}

func TestPresignGetIsDeterministicWithinASecond(t *testing.T) {
	// Two signs in the same second produce the same signature (the date
	// stamp is second-resolution). Guards against accidental nondeterminism
	// like map iteration leaking into the canonical query.
	p := testPresigner()
	a, err := p.PresignGet("site-A/x.ndjson", time.Minute)
	if err != nil {
		t.Fatal(err)
	}
	b, err := p.PresignGet("site-A/x.ndjson", time.Minute)
	if err != nil {
		t.Fatal(err)
	}
	sa := mustSig(t, a)
	sb := mustSig(t, b)
	if sa != sb {
		t.Errorf("same-second signatures differ:\n  %s\n  %s", sa, sb)
	}
}

func TestPresignDifferentKeysDifferentSignatures(t *testing.T) {
	p := testPresigner()
	a, _ := p.PresignGet("site-A/a.ndjson", time.Minute)
	b, _ := p.PresignGet("site-A/b.ndjson", time.Minute)
	if mustSig(t, a) == mustSig(t, b) {
		t.Errorf("different keys produced the same signature")
	}
}

func mustSig(t *testing.T, raw string) string {
	t.Helper()
	u, err := url.Parse(raw)
	if err != nil {
		t.Fatalf("parse %q: %v", raw, err)
	}
	return u.Query().Get("X-Amz-Signature")
}

func TestS3EscapePathKeepsSlashes(t *testing.T) {
	got := s3EscapePath("site-A/2026/06/11/13/000-1.ndjson")
	if got != "site-A/2026/06/11/13/000-1.ndjson" {
		t.Errorf("escape mangled a safe key: %q", got)
	}
	// A space in a segment is percent-encoded; slashes stay.
	if got := s3EscapePath("a b/c"); got != "a%20b/c" {
		t.Errorf("escape = %q, want a%%20b/c", got)
	}
}

func TestSinceStartAfterIsMillisecondGranular(t *testing.T) {
	// F-196: the bound must include the epoch-millis, not stop at the hour —
	// an hour-granular bound returns the oldest objects of the since-hour and
	// the 500-cap drops the recent tail.
	since := time.Date(2026, 6, 12, 14, 52, 25, 0, time.UTC)
	got := sinceStartAfter("galaxy-A/", since)
	want := "galaxy-A/2026/06/12/14/" + strconv.FormatInt(since.UnixMilli(), 10)
	if got != want {
		t.Errorf("start-after = %q, want %q", got, want)
	}
	// It must sit AT the since time: after objects earlier in the same hour
	// and before objects later in the hour (lexical == chronological).
	earlier := "galaxy-A/2026/06/12/14/" + strconv.FormatInt(since.Add(-10*time.Minute).UnixMilli(), 10) + "-3.ndjson"
	later := "galaxy-A/2026/06/12/14/" + strconv.FormatInt(since.Add(2*time.Minute).UnixMilli(), 10) + "-7.ndjson"
	if !(earlier < got) {
		t.Errorf("earlier-in-hour object %q should sort before start-after %q", earlier, got)
	}
	if !(got < later) {
		t.Errorf("later-in-hour object %q should sort after start-after %q", later, got)
	}
}

func TestSinceStartAfterZeroIsEmpty(t *testing.T) {
	if got := sinceStartAfter("galaxy-A/", time.Time{}); got != "" {
		t.Errorf("zero since should yield empty start-after, got %q", got)
	}
}

func TestPresignerEnabledGate(t *testing.T) {
	if (&Presigner{}).Enabled() {
		t.Error("empty presigner should be disabled")
	}
	if NewPresigner(Config{Endpoint: "http://x:9000", Bucket: "b"}).Enabled() {
		t.Error("missing credentials should be disabled")
	}
	if !testPresigner().Enabled() {
		t.Error("fully configured presigner should be enabled")
	}
}
