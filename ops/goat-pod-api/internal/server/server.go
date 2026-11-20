// Package server is the pod's local HTTPS API (pod-carriage-and-provenance.md §7.4) and the
// Gatepost page. Every /v1 route needs a client certificate the pod's CA roots verify; only an
// identity named as local authority in the held generation may change rules.
package server

import (
	"crypto/tls"
	"crypto/x509"
	_ "embed"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/agent"
	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/generation"
	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/rules"
)

// Server serves the API.
type Server struct {
	Agent *agent.Agent
	// GatepostPage, when set and readable, replaces the built-in Gatepost page.
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

type apiErrorBody struct {
	Error apiErrorDetail `json:"error"`
}

type apiErrorDetail struct {
	Code    string `json:"code"`
	Message string `json:"message"`
	What    string `json:"what"`
	Why     string `json:"why,omitempty"`
	Fix     string `json:"fix,omitempty"`
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
	var ae *agent.APIError
	if !errors.As(err, &ae) {
		ae = &agent.APIError{Status: http.StatusInternalServerError, Code: "internal", What: err.Error()}
	}
	writeJSON(w, ae.Status, apiErrorBody{Error: apiErrorDetail{
		Code: ae.Code, Message: ae.What, What: ae.What, Why: ae.Why, Fix: ae.Fix}})
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
			writeErr(w, agent.Err(http.StatusUnauthorized, "not_signed_in",
				"No client certificate was presented.",
				"The pod API signs people in with their goat certificate.",
				"Present a goat certificate this pod's CA roots trust (goat pod uses the active profile's)."))
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
			writeErr(w, agent.Err(http.StatusBadRequest, "invalid_request", err.Error(), "", ""))
			return
		}
		if err := json.Unmarshal(body, &c); err != nil {
			writeErr(w, agent.Err(http.StatusBadRequest, "invalid_request",
				"The body must be JSON {source, until, reason}: "+err.Error(), "", "Send until as an RFC 3339 time."))
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

func (s *Server) status(w http.ResponseWriter, r *http.Request) {
	st := s.Agent.Status()
	cn, _ := clientCN(r)
	you := &agent.You{CertCN: cn, Identity: cn}
	if who, err := s.Agent.Authorize(cn); err == nil {
		you.Identity, you.MayChange = who.Identity, true
	}
	st.You = you
	writeJSON(w, http.StatusOK, st)
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
	writeJSON(w, http.StatusOK, map[string][]agent.PendingItem{"pending": s.Agent.Pending()})
}

func (s *Server) done(w http.ResponseWriter, code int, res agent.Result, err error) {
	if err != nil {
		writeErr(w, err)
		return
	}
	if res.Warning != "" {
		// Recorded in the ledger, but the router did not take it (it rolled back).
		code = http.StatusAccepted
	}
	writeJSON(w, code, res)
}

func (s *Server) allow(w http.ResponseWriter, r *http.Request, who generation.Authority, c agent.Change) {
	res, err := s.Agent.Allow(r.Context(), who, c)
	s.done(w, http.StatusCreated, res, err)
}

func (s *Server) extend(w http.ResponseWriter, r *http.Request, who generation.Authority, c agent.Change) {
	res, err := s.Agent.Extend(r.Context(), who, r.PathValue("id"), c)
	s.done(w, http.StatusOK, res, err)
}

func (s *Server) reactivate(w http.ResponseWriter, r *http.Request, who generation.Authority, c agent.Change) {
	res, err := s.Agent.Reactivate(r.Context(), who, r.PathValue("id"), c)
	s.done(w, http.StatusOK, res, err)
}

func (s *Server) revoke(w http.ResponseWriter, r *http.Request, who generation.Authority, c agent.Change) {
	res, err := s.Agent.Revoke(r.Context(), who, r.PathValue("slot"), c)
	s.done(w, http.StatusOK, res, err)
}

// gatepostPage is ops/fieldkit/gatepost/gatepost.html, copied into this module so the binary
// carries it (go:embed cannot reach outside the module). CI fails when the copy drifts:
// refresh it with `cp ops/fieldkit/gatepost/gatepost.html ops/goat-pod-api/internal/server/`.
//
//go:embed gatepost.html
var gatepostPage []byte

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
	if _, err := w.Write(gatepostPage); err != nil {
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
