// Package server is the pod's local HTTPS API (pod-carriage-and-provenance.md §7.4) and the
// Gatepost page. Every /v1 route needs a client certificate the pod's CA roots verify; only an
// identity named as local authority in the held generation may change rules.
package server

import (
	"crypto/tls"
	"crypto/x509"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/agent"
	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/generation"
	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/ledger"
	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/rules"
)

// Server serves the API.
type Server struct {
	Agent        *agent.Agent
	GatepostPage string
}

// TLSConfig builds the listener TLS settings: the pod's certificate, and client certificates
// verified against caRoots when presented. /v1 routes refuse a request without one.
func TLSConfig(certFile, keyFile, caRoots string) (*tls.Config, error) {
	pair, err := tls.LoadX509KeyPair(certFile, keyFile)
	if err != nil {
		return nil, fmt.Errorf("pod certificate: %w", err)
	}
	raw, err := os.ReadFile(caRoots) // #nosec G304 -- configured path
	if err != nil {
		return nil, fmt.Errorf("CA roots: %w", err)
	}
	pool := x509.NewCertPool()
	if !pool.AppendCertsFromPEM(raw) {
		return nil, fmt.Errorf("CA roots %s hold no certificate", caRoots)
	}
	return &tls.Config{
		MinVersion:   tls.VersionTLS12,
		Certificates: []tls.Certificate{pair},
		ClientCAs:    pool,
		ClientAuth:   tls.VerifyClientCertIfGiven,
	}, nil
}

// Handler returns the routes.
func (s *Server) Handler() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("GET /{$}", s.gatepost)
	mux.HandleFunc("GET /v1/status", s.signedIn(s.status))
	mux.HandleFunc("GET /v1/rules", s.signedIn(s.rules))
	mux.HandleFunc("GET /v1/pending", s.signedIn(s.pending))
	mux.HandleFunc("POST /v1/rules", s.authority(s.allow))
	mux.HandleFunc("POST /v1/rules/{id}/extend", s.authority(s.extend))
	mux.HandleFunc("POST /v1/rules/{id}/reactivate", s.authority(s.reactivate))
	mux.HandleFunc("POST /v1/sources/{slot}/revoke", s.authority(s.revoke))
	return mux
}

type apiError struct {
	Error string `json:"error"`
	Code  string `json:"code"`
}

func writeJSON(w http.ResponseWriter, code int, v any) {
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Cache-Control", "no-store")
	w.WriteHeader(code)
	enc := json.NewEncoder(w)
	enc.SetIndent("", "  ")
	if err := enc.Encode(v); err != nil {
		// The client went away mid-response; nothing more can be sent.
		return
	}
}

func writeErr(w http.ResponseWriter, err error) {
	code, name := http.StatusInternalServerError, "internal"
	switch {
	case errors.Is(err, agent.ErrInvalid):
		code, name = http.StatusBadRequest, "invalid"
	case errors.Is(err, agent.ErrForbidden):
		code, name = http.StatusForbidden, "not_authorized"
	case errors.Is(err, agent.ErrNotFound):
		code, name = http.StatusNotFound, "not_found"
	case errors.Is(err, agent.ErrConflict):
		code, name = http.StatusConflict, "conflict"
	}
	writeJSON(w, code, apiError{Error: err.Error(), Code: name})
}

func clientCN(r *http.Request) (string, bool) {
	if r.TLS == nil || len(r.TLS.VerifiedChains) == 0 || len(r.TLS.PeerCertificates) == 0 {
		return "", false
	}
	return r.TLS.PeerCertificates[0].Subject.CommonName, true
}

func (s *Server) signedIn(next http.HandlerFunc) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		if _, ok := clientCN(r); !ok {
			writeJSON(w, http.StatusUnauthorized, apiError{
				Error: "sign in with a goat certificate this pod's CA roots trust", Code: "no_client_certificate"})
			return
		}
		next(w, r)
	}
}

type authorityHandler func(http.ResponseWriter, *http.Request, generation.Authority, agent.Change)

func (s *Server) authority(next authorityHandler) http.HandlerFunc {
	return s.signedIn(func(w http.ResponseWriter, r *http.Request) {
		cn, _ := clientCN(r)
		who, err := s.Agent.Authorize(cn)
		if err != nil {
			writeErr(w, err)
			return
		}
		var c agent.Change
		body, err := io.ReadAll(io.LimitReader(r.Body, 64<<10))
		if err != nil {
			writeErr(w, fmt.Errorf("%w: %w", agent.ErrInvalid, err))
			return
		}
		if err := json.Unmarshal(body, &c); err != nil {
			writeErr(w, fmt.Errorf("%w: body must be JSON {source?, until, reason}: %w", agent.ErrInvalid, err))
			return
		}
		next(w, r, who, c)
	})
}

// RulesResponse is GET /v1/rules.
type RulesResponse struct {
	Rules       []rules.Rule       `json:"rules"`
	Revocations []rules.Revocation `json:"revocations"`
}

func (s *Server) status(w http.ResponseWriter, _ *http.Request) {
	writeJSON(w, http.StatusOK, s.Agent.Status())
}

func (s *Server) rules(w http.ResponseWriter, _ *http.Request) {
	res := s.Agent.Compute()
	out := RulesResponse{Rules: res.Rules, Revocations: res.Revocations}
	if out.Rules == nil {
		out.Rules = []rules.Rule{}
	}
	if out.Revocations == nil {
		out.Revocations = []rules.Revocation{}
	}
	writeJSON(w, http.StatusOK, out)
}

func (s *Server) pending(w http.ResponseWriter, _ *http.Request) {
	writeJSON(w, http.StatusOK, map[string][]ledger.Entry{"pending": s.Agent.Pending()})
}

func (s *Server) done(w http.ResponseWriter, code int, e ledger.Entry, err error) {
	if err != nil && e.Seq == 0 {
		writeErr(w, err)
		return
	}
	resp := map[string]any{"entry": e}
	if err != nil {
		// Recorded in the ledger, but the router did not take it (it rolled back).
		resp["warning"] = err.Error()
		code = http.StatusAccepted
	}
	writeJSON(w, code, resp)
}

func (s *Server) allow(w http.ResponseWriter, r *http.Request, who generation.Authority, c agent.Change) {
	e, err := s.Agent.Allow(r.Context(), who, c)
	s.done(w, http.StatusCreated, e, err)
}

func (s *Server) extend(w http.ResponseWriter, r *http.Request, who generation.Authority, c agent.Change) {
	e, err := s.Agent.Extend(r.Context(), who, r.PathValue("id"), c)
	s.done(w, http.StatusOK, e, err)
}

func (s *Server) reactivate(w http.ResponseWriter, r *http.Request, who generation.Authority, c agent.Change) {
	e, err := s.Agent.Reactivate(r.Context(), who, r.PathValue("id"), c)
	s.done(w, http.StatusOK, e, err)
}

func (s *Server) revoke(w http.ResponseWriter, r *http.Request, who generation.Authority, c agent.Change) {
	e, err := s.Agent.Revoke(r.Context(), who, r.PathValue("slot"), c)
	s.done(w, http.StatusOK, e, err)
}

const placeholder = `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Gatepost</title></head>
<body style="font-family: system-ui, sans-serif; max-width: 40rem; margin: 2rem auto; padding: 0 1rem">
<h1>Gatepost</h1>
<p>The Gatepost page is not installed on this pod. The same actions are available with the
<code>goat pod</code> commands: <code>goat pod status</code>, <code>goat pod rules ls</code>,
<code>goat pod rules allow</code>, <code>goat pod rules extend</code>, <code>goat pod rules revoke</code>,
<code>goat pod rules reactivate</code>, <code>goat pod pending</code>.</p>
</body></html>
`

func (s *Server) gatepost(w http.ResponseWriter, _ *http.Request) {
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	w.Header().Set("Content-Security-Policy", "default-src 'self'; style-src 'self' 'unsafe-inline'; script-src 'self' 'unsafe-inline'")
	w.Header().Set("X-Content-Type-Options", "nosniff")
	if s.GatepostPage != "" {
		if raw, err := os.ReadFile(s.GatepostPage); err == nil {
			if _, err := w.Write(raw); err != nil {
				return
			}
			return
		}
	}
	if _, err := w.Write([]byte(placeholder)); err != nil {
		return
	}
}

// NewHTTPServer wraps the handler with timeouts.
func NewHTTPServer(addr string, h http.Handler, tc *tls.Config) *http.Server {
	return &http.Server{
		Addr: addr, Handler: h, TLSConfig: tc,
		ReadHeaderTimeout: 10 * time.Second, ReadTimeout: 30 * time.Second,
		WriteTimeout: 180 * time.Second, IdleTimeout: 120 * time.Second,
	}
}
