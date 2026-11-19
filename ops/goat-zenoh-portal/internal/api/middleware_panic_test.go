package api

import (
	"bytes"
	"encoding/json"
	"io"
	"log"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"
)

// syncBuffer is a bytes.Buffer safe to write from a server goroutine while the
// test goroutine reads it.
type syncBuffer struct {
	mu  sync.Mutex
	buf bytes.Buffer
}

func (b *syncBuffer) Write(p []byte) (int, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.buf.Write(p)
}

func (b *syncBuffer) String() string {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.buf.String()
}

// captureLog routes the default slog logger into a buffer for one test.
func captureLog(t *testing.T) *syncBuffer {
	t.Helper()
	buf := &syncBuffer{}
	prev := slog.Default()
	slog.SetDefault(slog.New(slog.NewJSONHandler(buf, &slog.HandlerOptions{Level: slog.LevelDebug})))
	t.Cleanup(func() { slog.SetDefault(prev) })
	return buf
}

// waitAccessLines polls buf until it holds want access records or a second
// passes, for a request served on another goroutine.
func waitAccessLines(t *testing.T, buf *syncBuffer, want int) []map[string]any {
	t.Helper()
	deadline := time.Now().Add(time.Second)
	for {
		lines := accessLines(t, buf)
		if len(lines) >= want || time.Now().After(deadline) {
			return lines
		}
		time.Sleep(5 * time.Millisecond)
	}
}

// accessLines returns the decoded "access" records in buf.
func accessLines(t *testing.T, buf *syncBuffer) []map[string]any {
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

func TestMiddleware_panicAfterWriteAbortsInsteadOfFinishing(t *testing.T) {
	buf := captureLog(t)
	h := (&Server{}).middleware(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"partial":`))
		panic("boom mid-body")
	}))
	w := httptest.NewRecorder()
	func() {
		defer func() {
			// Returning normally would let net/http finish the response as a
			// success; the middleware must abort it instead.
			if rec := recover(); rec != http.ErrAbortHandler {
				t.Fatalf("recovered %v, want http.ErrAbortHandler so the connection is dropped", rec)
			}
		}()
		h.ServeHTTP(w, httptest.NewRequest("GET", "/api/catalog", nil))
	}()
	if body := w.Body.String(); body != `{"partial":` {
		t.Fatalf("body %q: a problem body was appended to the half-sent response", body)
	}
	if !strings.Contains(buf.String(), "handler panic") {
		t.Fatalf("the original panic was not logged:\n%s", buf)
	}
	lines := accessLines(t, buf)
	if len(lines) != 1 || lines[0]["status"] != float64(500) {
		t.Fatalf("want one access line recording the failure as 500, got %v", lines)
	}
}

// Over a real connection, through the production middleware chain: a panic
// after the response has started must reach the client as an error, never as
// a complete-looking success. With gzip in the chain this is the common case,
// because the gzip layer commits its buffered bytes during the unwind.
func TestChain_panicAfterResponseStartedIsATruncationTheClientSees(t *testing.T) {
	cases := []struct {
		name           string
		acceptEncoding string
		body           []byte
	}{
		{"identity", "", []byte(`{"partial":`)},
		{"gzip", "gzip", []byte(`{"partial":"` + strings.Repeat("x", gzipMinSize*4) + ``)},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			buf := captureLog(t)
			h := (&Server{}).chain(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
				w.Header().Set("Content-Type", "application/json")
				_, _ = w.Write(tc.body)
				panic("boom mid-body")
			}))
			srv := httptest.NewUnstartedServer(h)
			srv.Config.ErrorLog = log.New(io.Discard, "", 0)
			srv.Start()
			defer srv.Close()

			req, _ := http.NewRequest("GET", srv.URL+"/api/catalog", nil)
			if tc.acceptEncoding != "" {
				// Set explicitly so the client does not decompress for us;
				// the test reads the raw response.
				req.Header.Set("Accept-Encoding", tc.acceptEncoding)
			}
			resp, err := srv.Client().Do(req)
			if err == nil {
				_, err = io.ReadAll(resp.Body)
				resp.Body.Close()
			}
			if err == nil {
				t.Fatalf("client read a complete %d response; the truncation was invisible", resp.StatusCode)
			}
			lines := waitAccessLines(t, buf, 1)
			if len(lines) != 1 || lines[0]["status"] != float64(500) {
				t.Fatalf("want one access line recording 500, got %v", lines)
			}
		})
	}
}

// A writer that cannot flush sends nothing, so a Flush on it must not mark
// the response as started: a later panic can still answer the 500 problem.
func TestStatusWriter_flushWithoutAFlusherDoesNotStartTheResponse(t *testing.T) {
	sw := &statusWriter{ResponseWriter: noFlushWriter{httptest.NewRecorder()}, status: http.StatusOK}
	sw.Flush()
	if sw.wroteHeader {
		t.Fatal("Flush on a non-flushing writer marked the response as started")
	}
	sw2 := &statusWriter{ResponseWriter: httptest.NewRecorder(), status: http.StatusOK}
	sw2.Flush()
	if !sw2.wroteHeader {
		t.Fatal("Flush on a flushing writer must mark the implicit 200 as sent")
	}
}

// noFlushWriter hides the recorder's Flush.
type noFlushWriter struct{ w *httptest.ResponseRecorder }

func (n noFlushWriter) Header() http.Header         { return n.w.Header() }
func (n noFlushWriter) Write(b []byte) (int, error) { return n.w.Write(b) }
func (n noFlushWriter) WriteHeader(code int)        { n.w.WriteHeader(code) }

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
