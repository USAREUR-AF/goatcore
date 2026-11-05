package api

import (
	"context"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

func sessionRequest(method string) *http.Request {
	r := httptest.NewRequestWithContext(context.Background(), method, "https://portal.example.test/api/browser-session", nil)
	r.Header.Set("Origin", "https://portal.example.test")
	return r
}

func TestBrowserCookieFlagsForLoopbackAndExternalHosts(t *testing.T) {
	for _, tc := range []struct {
		host   string
		secure bool
	}{
		{"localhost:8888", false}, {"127.0.0.1:8888", false},
		{"[::1]:8888", false}, {"portal.example.test", true},
		{"localhost.example.test", true}, {"127.0.0.1.example.test", true},
	} {
		t.Run(tc.host, func(t *testing.T) {
			var sessions browserSessions
			r := sessionRequest(http.MethodPost)
			r.Host = tc.host
			w := httptest.NewRecorder()
			if !sessions.create(w, r, "synthetic") {
				t.Fatal("create failed")
			}
			cookie := w.Result().Cookies()[0]
			if cookie.Secure != tc.secure || !cookie.HttpOnly || cookie.SameSite != http.SameSiteStrictMode {
				t.Fatalf("cookie security flags for %q: %+v", tc.host, cookie)
			}
			r.AddCookie(cookie)
			cleared := httptest.NewRecorder()
			sessions.logout(cleared, r)
			cookie = cleared.Result().Cookies()[0]
			if cookie.Secure != tc.secure || !cookie.HttpOnly || cookie.SameSite != http.SameSiteStrictMode || cookie.MaxAge != -1 {
				t.Fatalf("logout cookie flags for %q: %+v", tc.host, cookie)
			}
		})
	}
}

func TestBrowserSessionLifecycle(t *testing.T) {
	var sessions browserSessions
	login := sessionRequest(http.MethodPost)
	w := httptest.NewRecorder()
	const credential = "synthetic-credential-not-for-production"
	if !sessions.create(w, login, credential) {
		t.Fatal("create failed")
	}
	cookies := w.Result().Cookies()
	if len(cookies) != 1 {
		t.Fatal("missing cookie")
	}
	cookie := cookies[0]
	if !cookie.HttpOnly || !cookie.Secure || cookie.SameSite != http.SameSiteStrictMode || cookie.Path != "/" {
		t.Fatalf("cookie security flags: %+v", cookie)
	}
	if strings.Contains(cookie.Value, credential) || len(cookie.Value) != 64 {
		t.Fatal("cookie must be opaque")
	}
	request := sessionRequest(http.MethodGet)
	request.AddCookie(cookie)
	if sessions.credential(request) != credential {
		t.Fatal("session did not survive navigation")
	}
	request.Method = http.MethodPost
	request.Header.Set("Origin", "https://attacker.example.test")
	if sessions.credential(request) != "" {
		t.Fatal("accepted cross-origin write")
	}
	request.Header.Del("Origin")
	if sessions.credential(request) != "" {
		t.Fatal("accepted originless cookie write")
	}
	request.Header.Set("Origin", "https://portal.example.test")
	if sessions.credential(request) != credential {
		t.Fatal("rejected same-origin write")
	}
	sessions.logout(httptest.NewRecorder(), request)
	if sessions.credential(request) != "" {
		t.Fatal("logout left a live session")
	}
}

func TestBrowserSessionExpiryCapacityAndReplacement(t *testing.T) {
	var sessions browserSessions
	r := sessionRequest(http.MethodPost)
	w := httptest.NewRecorder()
	sessions.create(w, r, "fixture")
	old := w.Result().Cookies()[0]
	r.AddCookie(old)
	next := httptest.NewRecorder()
	sessions.create(next, r, "replacement")
	if _, ok := sessions.entries[old.Value]; ok {
		t.Fatal("replacement kept the old session")
	}
	cookie := next.Result().Cookies()[0]
	sessions.entries[cookie.Value] = browserSession{"fixture", time.Now().Add(-time.Second)}
	get := sessionRequest(http.MethodGet)
	get.AddCookie(cookie)
	if sessions.credential(get) != "" {
		t.Fatal("expired session authenticated")
	}
	if _, ok := sessions.entries[cookie.Value]; ok {
		t.Fatal("expired session was retained")
	}
	for i := 0; i < maxBrowserSessions; i++ {
		sessions.entries[strings.Repeat("x", i+1)] = browserSession{"fixture", time.Now().Add(time.Hour)}
	}
	full := httptest.NewRecorder()
	if sessions.create(full, sessionRequest(http.MethodPost), "fixture") || full.Code != http.StatusServiceUnavailable {
		t.Fatal("session capacity is not bounded")
	}
}

func TestBrowserSessionOriginBoundary(t *testing.T) {
	for _, origin := range []string{"", "null", "https://attacker.example.test", "http://portal.example.test", "https://portal.example.test.evil", "https://portal.example.test/path"} {
		r := sessionRequest(http.MethodPost)
		r.Header.Set("Origin", origin)
		if browserSameOrigin(r) {
			t.Errorf("accepted origin %q", origin)
		}
	}
	r := sessionRequest(http.MethodPost)
	r.Header.Set("Sec-Fetch-Site", "cross-site")
	if browserSameOrigin(r) {
		t.Fatal("accepted cross-site fetch")
	}
	local := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "http://127.0.0.1:8888/api/browser-session", nil)
	local.Header.Set("Origin", "http://127.0.0.1:8888")
	if !browserSameOrigin(local) {
		t.Fatal("loopback development origin refused")
	}
}
