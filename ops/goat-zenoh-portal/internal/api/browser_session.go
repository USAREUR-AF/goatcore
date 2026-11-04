package api

// Browser sessions keep API credentials in this process, never Web Storage.
// Separate modules keep this small implementation local to each service.
import (
	"crypto/rand"
	"encoding/hex"
	"net"
	"net/http"
	"net/url"
	"sync"
	"time"
)

const browserSessionTTL = 8 * time.Hour
const maxBrowserSessions = 4096

type browserSession struct {
	credential string
	expires    time.Time
}

type browserSessions struct {
	mu      sync.Mutex
	entries map[string]browserSession
}

func browserSameOrigin(r *http.Request) bool {
	if r.Header.Get("Sec-Fetch-Site") == "cross-site" {
		return false
	}
	origin, err := url.Parse(r.Header.Get("Origin"))
	if err != nil || origin.Host == "" || origin.Host != r.Host || origin.User != nil || origin.Path != "" || origin.RawQuery != "" || origin.Fragment != "" {
		return false
	}
	return origin.Scheme == "https" || (origin.Scheme == "http" && browserLoopback(r.Host))
}

func browserLoopback(host string) bool {
	if h, _, err := net.SplitHostPort(host); err == nil {
		host = h
	}
	return host == "localhost" || (net.ParseIP(host) != nil && net.ParseIP(host).IsLoopback())
}

func (s *browserSessions) create(w http.ResponseWriter, r *http.Request, credential string) bool {
	var raw [32]byte
	if _, err := rand.Read(raw[:]); err != nil {
		writeError(w, http.StatusInternalServerError, "session", "cannot create session")
		return false
	}
	id := hex.EncodeToString(raw[:])
	now := time.Now()
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.entries == nil {
		s.entries = make(map[string]browserSession)
	}
	for key, session := range s.entries {
		if !now.Before(session.expires) {
			delete(s.entries, key)
		}
	}
	// A successful replacement invalidates the browser's old session.
	if old, err := r.Cookie(browserSessionCookie); err == nil {
		delete(s.entries, old.Value)
	}
	if len(s.entries) >= maxBrowserSessions {
		writeError(w, http.StatusServiceUnavailable, "session", "session capacity reached")
		return false
	}
	s.entries[id] = browserSession{credential: credential, expires: now.Add(browserSessionTTL)}
	http.SetCookie(w, &http.Cookie{
		Name: browserSessionCookie, Value: id, Path: "/", HttpOnly: true,
		Secure: !browserLoopback(r.Host), SameSite: http.SameSiteStrictMode,
		MaxAge: int(browserSessionTTL.Seconds()),
	})
	return true
}

func (s *browserSessions) credential(r *http.Request) string {
	// Cookie-authenticated writes require an explicit same-origin browser.
	// Bearer-token clients retain their existing, separate authentication path.
	if r.Method != http.MethodGet && r.Method != http.MethodHead && !browserSameOrigin(r) {
		return ""
	}
	cookie, err := r.Cookie(browserSessionCookie)
	if err != nil {
		return ""
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	session, ok := s.entries[cookie.Value]
	if !ok {
		return ""
	}
	if !time.Now().Before(session.expires) {
		delete(s.entries, cookie.Value)
		return ""
	}
	return session.credential
}

func (s *browserSessions) logout(w http.ResponseWriter, r *http.Request) {
	if cookie, err := r.Cookie(browserSessionCookie); err == nil {
		s.mu.Lock()
		delete(s.entries, cookie.Value)
		s.mu.Unlock()
	}
	http.SetCookie(w, &http.Cookie{
		Name: browserSessionCookie, Path: "/", HttpOnly: true,
		Secure: !browserLoopback(r.Host), SameSite: http.SameSiteStrictMode, MaxAge: -1,
	})
}

const browserSessionCookie = "goat_portal_session"

// handleBrowserSession exchanges a validated bearer for an opaque session.
// Underlying credentials are checked again by AuthMiddleware on every request,
// so revocation and role changes are effective without waiting for expiry.
func (s *Server) handleBrowserSession(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Cache-Control", "no-store")
	if !browserSameOrigin(r) {
		writeError(w, http.StatusForbidden, "auth", "same-origin request required")
		return
	}
	if r.Method == http.MethodDelete {
		s.browserSessions.logout(w, r)
		w.WriteHeader(http.StatusNoContent)
		return
	}
	token := extractToken(r)
	if token == "" || len(token) > 8192 {
		writeError(w, http.StatusUnauthorized, "auth", "invalid token")
		return
	}
	principal, err := s.resolvePrincipal(r.Context(), token)
	if err != nil || principal == nil {
		writeError(w, http.StatusUnauthorized, "auth", "invalid token")
		return
	}
	if s.browserSessions.create(w, r, token) {
		w.WriteHeader(http.StatusNoContent)
	}
}
