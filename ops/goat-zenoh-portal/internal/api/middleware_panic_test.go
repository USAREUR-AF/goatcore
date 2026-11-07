package api

import (
	"bytes"
	"encoding/json"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

// captureLog routes the default slog logger into a buffer for one test.
func captureLog(t *testing.T) *bytes.Buffer {
	t.Helper()
	var buf bytes.Buffer
	prev := slog.Default()
	slog.SetDefault(slog.New(slog.NewJSONHandler(&buf, &slog.HandlerOptions{Level: slog.LevelDebug})))
	t.Cleanup(func() { slog.SetDefault(prev) })
	return &buf
}

// accessLines returns the decoded "access" records in buf.
func accessLines(t *testing.T, buf *bytes.Buffer) []map[string]any {
	t.Helper()
	var out []map[string]any
	for _, line := range strings.Split(strings.TrimSpace(buf.String()), "\n") {
		if line == "" {
			continue
		}
		var rec map[string]any
		if err := json.Unmarshal([]byte(line), &rec); err != nil {
			t.Fatalf("log line is not JSON: %q", line)
		}
		if rec["msg"] == "access" {
			out = append(out, rec)
		}
	}
	return out
}

func TestMiddleware_panicBeforeWriteAnswers500AndIsAccessLogged(t *testing.T) {
	buf := captureLog(t)
	h := (&Server{}).middleware(http.HandlerFunc(func(http.ResponseWriter, *http.Request) {
		panic("boom")
	}))
	w := httptest.NewRecorder()
	h.ServeHTTP(w, httptest.NewRequest("GET", "/api/catalog", nil))

	if w.Code != http.StatusInternalServerError {
		t.Fatalf("status %d, want 500", w.Code)
	}
	if ct := w.Header().Get("Content-Type"); ct != problemContentType {
		t.Fatalf("Content-Type %q, want the problem type", ct)
	}
	lines := accessLines(t, buf)
	if len(lines) != 1 {
		t.Fatalf("got %d access lines, want 1 (a panicking request used to log none):\n%s", len(lines), buf)
	}
	if got := lines[0]["status"]; got != float64(500) {
		t.Fatalf("access line status %v, want 500", got)
	}
	if !strings.Contains(buf.String(), `"stack"`) {
		t.Fatalf("panic line carries no stack:\n%s", buf)
	}
}

func TestMiddleware_panicAfterWriteDoesNotAppendAProblemBody(t *testing.T) {
	buf := captureLog(t)
	h := (&Server{}).middleware(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"partial":`))
		panic("boom mid-body")
	}))
	w := httptest.NewRecorder()
	h.ServeHTTP(w, httptest.NewRequest("GET", "/api/catalog", nil))

	if w.Code != http.StatusOK {
		t.Fatalf("status %d: the 200 was already sent and cannot change", w.Code)
	}
	if body := w.Body.String(); body != `{"partial":` {
		t.Fatalf("body %q: a problem body was appended to the half-sent response", body)
	}
	lines := accessLines(t, buf)
	if len(lines) != 1 || lines[0]["status"] != float64(500) {
		t.Fatalf("want one access line recording the failure as 500, got %v", lines)
	}
}

func TestMiddleware_errAbortHandlerIsReRaised(t *testing.T) {
	buf := captureLog(t)
	h := (&Server{}).middleware(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write([]byte("partial"))
		panic(http.ErrAbortHandler)
	}))
	w := httptest.NewRecorder()
	func() {
		defer func() {
			if rec := recover(); rec != http.ErrAbortHandler {
				t.Fatalf("recovered %v, want http.ErrAbortHandler re-raised for net/http", rec)
			}
		}()
		h.ServeHTTP(w, httptest.NewRequest("GET", "/api/topics/stream", nil))
	}()
	if body := w.Body.String(); body != "partial" {
		t.Fatalf("body %q: an aborted response must not get a problem body", body)
	}
	if strings.Contains(buf.String(), "handler panic") {
		t.Fatalf("a deliberate abort was logged as a handler panic:\n%s", buf)
	}
	if n := len(accessLines(t, buf)); n != 1 {
		t.Fatalf("got %d access lines, want 1", n)
	}
}

func TestMiddleware_successIsAccessLoggedOnce(t *testing.T) {
	buf := captureLog(t)
	h := (&Server{}).middleware(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusNoContent)
	}))
	w := httptest.NewRecorder()
	h.ServeHTTP(w, httptest.NewRequest("GET", "/api/health", nil))
	lines := accessLines(t, buf)
	if len(lines) != 1 || lines[0]["status"] != float64(204) {
		t.Fatalf("want one access line with 204, got %v", lines)
	}
}
