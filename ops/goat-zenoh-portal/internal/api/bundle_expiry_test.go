package api

import (
	"bytes"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// A bundle link holds the org's private key and is its only credential, so it
// works for BundleTTL after it is minted and answers 410 after that, without
// serving the tarball.
func TestBundleDownload_expiresAfterTTL(t *testing.T) {
	srv, _ := bulkTestServer(t, "exp-co")
	ctx := t.Context()
	tarball := filepath.Join(t.TempDir(), "exp-co-onboarding.tar.gz")
	if err := os.WriteFile(tarball, []byte("key-material"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := srv.reg.MarkBundle(ctx, "exp-co", "tok-exp", tarball); err != nil {
		t.Fatal(err)
	}
	get := func() *httptest.ResponseRecorder {
		w := httptest.NewRecorder()
		srv.Routes().ServeHTTP(w, httptest.NewRequest("GET", "/api/org-bundle/exp-co/tok-exp.tar.gz", nil))
		return w
	}
	if w := get(); w.Code != http.StatusOK {
		t.Fatalf("fresh link: status %d, want 200", w.Code)
	}
	old := time.Now().Add(-25 * time.Hour).Unix()
	if _, err := srv.reg.DB().ExecContext(ctx, `UPDATE vendors SET bundle_minted_at = $1 WHERE prefix = 'exp-co'`, old); err != nil {
		t.Fatal(err)
	}
	w := get()
	if w.Code != http.StatusGone {
		t.Fatalf("link older than the TTL: status %d, want 410", w.Code)
	}
	if w.Body.String() == "key-material" {
		t.Fatal("an expired link served the tarball")
	}
	if _, err := srv.reg.DB().ExecContext(ctx, `UPDATE vendors SET bundle_minted_at = NULL WHERE prefix = 'exp-co'`); err != nil {
		t.Fatal(err)
	}
	if w := get(); w.Code != http.StatusGone {
		t.Fatalf("link with no mint time: status %d, want 410", w.Code)
	}
}

func TestBundleLinkExpired(t *testing.T) {
	now := time.Unix(1_800_000_000, 0)
	cases := []struct {
		minted int64
		ttl    time.Duration
		want   bool
	}{
		{now.Add(-time.Hour).Unix(), 24 * time.Hour, false},
		{now.Add(-25 * time.Hour).Unix(), 24 * time.Hour, true},
		{now.Add(-2 * time.Hour).Unix(), time.Hour, true},
		{now.Add(-time.Hour).Unix(), 0, false}, // unset ttl = 24 h default
		{0, 24 * time.Hour, true},
	}
	for _, c := range cases {
		if got := bundleLinkExpired(c.minted, c.ttl, now); got != c.want {
			t.Errorf("bundleLinkExpired(%d, %s) = %v, want %v", c.minted, c.ttl, got, c.want)
		}
	}
}

func TestRedactBundleToken(t *testing.T) {
	cases := map[string]string{
		"/api/org-bundle/acme/0123abcd.tar.gz":      "/api/org-bundle/acme/REDACTED",
		"/api/vendor-bundle/acme/0123abcd.tar.gz":   "/api/vendor-bundle/acme/REDACTED",
		"/api/org-bundle/acme":                      "/api/org-bundle/REDACTED",
		"/api/orgs/acme":                            "/api/orgs/acme",
		"/API/Org-Bundle/acme/0123abcd.tar.gz":      "/API/Org-Bundle/acme/REDACTED",
		"//api/org-bundle/acme/0123abcd.tar.gz":     "//api/org-bundle/acme/REDACTED",
		"/x/../api/org-bundle/acme/0123abcd.tar.gz": "/x/../api/org-bundle/acme/REDACTED",
		"/api//vendor-bundle/acme/0123abcd.tar.gz":  "/api//vendor-bundle/acme/REDACTED",
	}
	for in, want := range cases {
		if got := redactBundleToken(in); got != want {
			t.Errorf("redactBundleToken(%q) = %q, want %q", in, got, want)
		}
	}
}

// A handler that panics on a bundle request logs the path with the token
// redacted: the panic line is at ERROR, the level most likely shipped and kept.
func TestPanicLogRedactsBundleToken(t *testing.T) {
	srv, _ := bulkTestServer(t, "panic-co")
	var buf bytes.Buffer
	saved := slog.Default()
	slog.SetDefault(slog.New(slog.NewTextHandler(&buf, &slog.HandlerOptions{Level: slog.LevelDebug})))
	t.Cleanup(func() { slog.SetDefault(saved) })

	const tok = "paniktok0123456789abcdef"
	h := srv.middleware(http.HandlerFunc(func(http.ResponseWriter, *http.Request) { panic("boom") }))
	for _, p := range []string{
		"/api/org-bundle/panic-co/" + tok + ".tar.gz",
		"/api/vendor-bundle/panic-co/" + tok + ".tar.gz",
		"/API/org-bundle/panic-co/" + tok + ".tar.gz",
	} {
		w := httptest.NewRecorder()
		h.ServeHTTP(w, httptest.NewRequest("GET", p, nil))
		if w.Code != http.StatusInternalServerError {
			t.Fatalf("%s: status %d, want 500", p, w.Code)
		}
	}
	out := buf.String()
	if !strings.Contains(out, "handler panic") {
		t.Fatalf("no panic line logged:\n%s", out)
	}
	if strings.Contains(out, tok) {
		t.Fatalf("bundle token in the log:\n%s", out)
	}
}

// An unclean bundle path is answered by the mux with a redirect before any
// handler runs; its access line must not carry the token either.
func TestAccessLogRedactsUncleanBundlePath(t *testing.T) {
	srv, _ := bulkTestServer(t, "unclean-co")
	var buf bytes.Buffer
	saved := slog.Default()
	slog.SetDefault(slog.New(slog.NewTextHandler(&buf, &slog.HandlerOptions{Level: slog.LevelDebug})))
	t.Cleanup(func() { slog.SetDefault(saved) })

	const tok = "uncleantok0123456789abcdef"
	for _, p := range []string{
		"//api/org-bundle/unclean-co/" + tok + ".tar.gz",
		"/x/../api/org-bundle/unclean-co/" + tok + ".tar.gz",
	} {
		w := httptest.NewRecorder()
		srv.Routes().ServeHTTP(w, httptest.NewRequest("GET", p, nil))
		if w.Code < 300 || w.Code >= 400 {
			t.Errorf("%s: status %d, want a redirect", p, w.Code)
		}
	}
	out := buf.String()
	if !strings.Contains(out, "msg=access") {
		t.Fatalf("no access line logged:\n%s", out)
	}
	if strings.Contains(out, tok) {
		t.Fatalf("bundle token in the log:\n%s", out)
	}
}
