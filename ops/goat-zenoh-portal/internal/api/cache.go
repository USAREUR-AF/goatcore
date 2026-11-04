package api

// Cache classes for API responses. A route is in one of three classes:
//
//   - revalidated: the documented read operations. The response carries
//     Cache-Control: private, no-cache, Vary: Authorization and a strong ETag,
//     and a GET with a matching If-None-Match is answered 304. A browser may
//     keep the body but must ask before reusing it. The catalog's org rows
//     carry the per-org publish counters (issue #1339), which the audit
//     tailer moves about once a minute on a live goatnet, so its validator
//     changes on that cadence and a repeat reader mostly gets a 200; accepted.
//   - never stored: anything that returns a credential, a presigned URL or a
//     person's details. Cache-Control: no-store, no validator.
//   - everything else: unchanged, no cache headers, as before this file.
//
// The class is chosen at route registration (revalidated(), noStore()), so
// the ETag layer below never has to guess from a path.

import (
	"bytes"
	"net/http"
	"strconv"
	"strings"
)

const (
	cacheRevalidate = "private, no-cache"
	cacheNoStore    = "no-store"

	// maxETagBody bounds what etagWriter holds per request. A body past it
	// is streamed from that point on, without a validator, so a large export
	// or a wide topic read costs the memory it did before this layer and no
	// more than that.
	maxETagBody = 1 << 20
)

// revalidated marks a read route as cacheable with revalidation. The headers
// go out on every status; only a 200 gets a validator (conditionalMiddleware).
func revalidated(next http.HandlerFunc) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		h := w.Header()
		h.Set("Cache-Control", cacheRevalidate)
		// Both coding and credential vary the body; the 304 must carry both,
		// and it never reaches the gzip layer's own Vary (no body to code).
		h.Add("Vary", "Authorization")
  h.Add("Vary", "Cookie")
		h.Add("Vary", "Accept-Encoding")
		next(w, r)
	}
}

// noStore marks a route whose response must never be written to a cache.
func noStore(next http.HandlerFunc) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Cache-Control", cacheNoStore)
		next(w, r)
	}
}

// conditionalMiddleware turns a revalidated 200 on a GET or HEAD into a
// validated response: it holds the body, tags it, and answers 304 when the
// client already holds that tag. It sits inside gzipMiddleware so the tag is
// computed over the identity body; the gzip layer derives its own tag from it.
func conditionalMiddleware(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodGet && r.Method != http.MethodHead {
			next.ServeHTTP(w, r)
			return
		}
		ew := &etagWriter{ResponseWriter: w, req: r}
		next.ServeHTTP(ew, r)
		ew.finish()
	})
}

// etagWriter buffers a revalidated 200 until the handler returns, up to
// maxETagBody. Any other response passes straight through, including a
// stream; so does the rest of a body that outgrows the buffer.
type etagWriter struct {
	http.ResponseWriter
	req         *http.Request
	status      int
	wroteHeader bool
	buffering   bool
	body        bytes.Buffer
}

func (e *etagWriter) WriteHeader(status int) {
	if e.wroteHeader {
		return
	}
	e.wroteHeader = true
	e.status = status
	h := e.Header()
	// A handler that set its own validator (http.ServeContent) is left alone.
	e.buffering = status == http.StatusOK && h.Get("Cache-Control") == cacheRevalidate && h.Get("ETag") == ""
	if !e.buffering {
		e.ResponseWriter.WriteHeader(status)
	}
}

func (e *etagWriter) Write(b []byte) (int, error) {
	if !e.wroteHeader {
		e.WriteHeader(http.StatusOK)
	}
	if e.buffering {
		if e.body.Len()+len(b) > maxETagBody {
			e.spill()
			return e.ResponseWriter.Write(b)
		}
		return e.body.Write(b)
	}
	return e.ResponseWriter.Write(b)
}

// spill gives up on a validator for a body past maxETagBody: the header goes
// out as the handler set it, the bytes held so far follow, and every later
// write streams through.
func (e *etagWriter) spill() {
	e.buffering = false
	e.ResponseWriter.WriteHeader(e.status)
	if e.body.Len() > 0 {
		_, _ = e.ResponseWriter.Write(e.body.Bytes())
	}
	e.body = bytes.Buffer{}
}

// Flush forwards to the writer beneath; a buffered response has nothing to
// flush yet, and no revalidated route streams.
func (e *etagWriter) Flush() {
	if e.buffering {
		return
	}
	if f, ok := e.ResponseWriter.(http.Flusher); ok {
		f.Flush()
	}
}

// Unwrap lets http.ResponseController reach the writer beneath.
func (e *etagWriter) Unwrap() http.ResponseWriter { return e.ResponseWriter }

// finish writes the held response: 304 when the client's tag matches either
// content coding, the full body otherwise.
func (e *etagWriter) finish() {
	if !e.buffering {
		return
	}
	body := e.body.Bytes()
	tag := etagFor(body, "")
	h := e.Header()
	h.Set("ETag", tag)
	inm := e.req.Header.Get("If-None-Match")
	if etagMatches(inm, tag) || etagMatches(inm, gzipETag(tag)) {
		h.Del("Content-Length")
		h.Del("Content-Type")
		e.ResponseWriter.WriteHeader(http.StatusNotModified)
		return
	}
	h.Set("Content-Length", strconv.Itoa(len(body)))
	e.ResponseWriter.WriteHeader(http.StatusOK)
	if e.req.Method != http.MethodHead {
		_, _ = e.ResponseWriter.Write(body)
	}
}

// gzipETag is the tag of the gzip coding of a body whose identity tag is tag,
// the same "-gz" convention the static asset table uses.
func gzipETag(tag string) string {
	if !strings.HasSuffix(tag, `"`) || strings.HasSuffix(tag, `-gz"`) {
		return tag
	}
	return tag[:len(tag)-1] + `-gz"`
}
