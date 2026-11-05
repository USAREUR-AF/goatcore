// Package server provides the SNITCH HTTP API and dashboard.
package server

import (
	"context"
	_ "embed"
	"encoding/json"
	"fmt"
	"log"
	"net/http"
	"os"
	"runtime"
	"strconv"
	"time"

	"github.com/usareur-af/goatcore/ops/snitch/internal/auth"
	"github.com/usareur-af/goatcore/ops/snitch/internal/collector"
	"github.com/usareur-af/goatcore/ops/snitch/internal/identity"
	"github.com/usareur-af/goatcore/ops/snitch/internal/measure"
	"github.com/usareur-af/goatcore/ops/snitch/internal/store"
	"github.com/usareur-af/goatcore/ops/snitch/internal/types"
)

//go:embed dashboard.html
var dashboardHTML []byte

//go:embed inventory.html
var inventoryHTML []byte

//go:embed install.sh
var installScript []byte

// Config holds server configuration.
type Config struct {
	Collector   *collector.Collector
	Auth        *auth.Auth
	IsCollector bool
}

// Server is the SNITCH HTTP server.
type Server struct {
	cfg Config
	mux *http.ServeMux
}

// New creates a server with auth-protected routes.
func New(cfg Config) *Server {
	s := &Server{cfg: cfg, mux: http.NewServeMux()}
	a := cfg.Auth
	s.mux.HandleFunc("POST /api/browser-session", a.BrowserSession)
	s.mux.HandleFunc("DELETE /api/browser-session", a.BrowserSession)

	// Public endpoints (no auth).
	s.mux.HandleFunc("/", s.handleDashboard)
	s.mux.HandleFunc("/inventory", s.handleInventory)
	s.mux.HandleFunc("/health", s.handleHealth)
	s.mux.HandleFunc("/api/gate-status", s.handleGateStatus)
	// Probe-policy: probes poll this every cycle. Public so probes
	// don't need to authenticate to learn the current policy. The
	// policy itself is non-sensitive (it's a probe-throttle hint).
	s.mux.HandleFunc("/api/probe-policy", s.handleProbePolicy)

	// Probe endpoint.
	s.mux.Handle("/api/report", a.Require(auth.RoleProbe, s.handleReport))

	// Admin: set the fleet-wide probe policy.
	s.mux.Handle("/api/probe-policy/set", a.Require(auth.RoleAdmin, s.handleSetProbePolicy))

	// Viewer endpoints (read-only dashboard data).
	s.mux.Handle("/api/mesh", a.Require(auth.RoleViewer, s.handleMesh))
	s.mux.Handle("/api/history", a.Require(auth.RoleViewer, s.handleHistory))
	s.mux.Handle("/api/events", a.Require(auth.RoleViewer, s.handleEvents))
	s.mux.Handle("/api/uptime", a.Require(auth.RoleViewer, s.handleUptime))

	// Alias management (operator can register names).
	s.mux.Handle("/api/aliases", a.Require(auth.RoleOperator, s.handleAliases))

	// Operator endpoints.
	s.mux.Handle("/api/run", a.Require(auth.RoleOperator, s.handleRun))
	s.mux.Handle("/install", a.Require(auth.RoleOperator, s.handleInstall))
	s.mux.Handle("/bin/snitch", a.Require(auth.RoleOperator, s.handleBinary))

	// Admin endpoints.
	s.mux.Handle("/api/keys", a.Require(auth.RoleAdmin, s.handleKeys))

	// Satlink PoP tracker.
	s.registerSatlinkRoutes()

	return s
}

// ListenAndServe starts the HTTP server.
//
// Timeouts (Block 50E.1 — slowloris guard for the snitch admin surface):
//   - ReadHeaderTimeout: bounds slow-header attacks on the public-facing
//     dashboard + probe-policy endpoints.
//   - ReadTimeout: bounds slow-body POSTs on /api/report (probe reports
//     are small JSON; 30 s is generous).
//   - IdleTimeout: keep-alive ceiling.
//   - WriteTimeout is intentionally unset — /api/run runs a 60 s probe
//     and /bin/snitch streams binaries; both self-manage via per-handler
//     context.WithTimeout. A blanket WriteTimeout would prematurely kill
//     those long-tail handlers.
func (s *Server) ListenAndServe(addr string) error {
	log.Printf("snitch listening on %s", addr)
	srv := &http.Server{
		Addr:              addr,
		Handler:           s.mux,
		ReadHeaderTimeout: 10 * time.Second,
		ReadTimeout:       30 * time.Second,
		IdleTimeout:       2 * time.Minute,
	}
	return srv.ListenAndServe()
}

func (s *Server) handleDashboard(w http.ResponseWriter, r *http.Request) {
	if r.URL.Path != "/" {
		http.NotFound(w, r)
		return
	}
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
	_, _ = w.Write(dashboardHTML)
}

func (s *Server) handleGateStatus(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Content-Type", "application/json")
	data, err := os.ReadFile("/data/gate-status.json")
	if err != nil {
		fmt.Fprintf(w, `{"gate":"unknown"}`)
		return
	}
	//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
	_, _ = w.Write(data)
}

func (s *Server) handleHealth(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Content-Type", "application/json")
	//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
	_ = json.NewEncoder(w).Encode(map[string]any{
		"status": "ok",
		"probes": s.cfg.Collector.ProbeCount(),
		"time":   time.Now().UTC(),
	})
}

func (s *Server) handleMesh(w http.ResponseWriter, r *http.Request) {
	auth.LogAccess(r, "/api/mesh")
	state := s.cfg.Collector.MeshState()
	if aliases, err := s.cfg.Collector.Store().AliasMap(r.Context()); err == nil && len(aliases) > 0 {
		state.Aliases = aliases
	}
	w.Header().Set("Content-Type", "application/json")
	//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
	_ = json.NewEncoder(w).Encode(state)
}

func (s *Server) handleReport(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "POST only", http.StatusMethodNotAllowed)
		return
	}
	var report types.ProbeReport
	if err := json.NewDecoder(r.Body).Decode(&report); err != nil {
		http.Error(w, fmt.Sprintf("bad request: %v", err), http.StatusBadRequest)
		return
	}
	// Collector.Ingest does not take ctx today; the underlying store calls
	// thread r.Context() via the request-scoped methods they invoke. Adding
	// a ctx parameter to the public Ingest API is a separate API-shape
	// follow-up; for now the contextcheck false-positive is suppressed.
	if err := s.cfg.Collector.Ingest(report); err != nil { //nolint:contextcheck // see comment above
		log.Printf("ingest error: %v", err)
		http.Error(w, "internal error", http.StatusInternalServerError)
		return
	}
	log.Printf("report from %q (%q): %d peers", report.PeerName, report.PeerIP, len(report.Peers))
	w.Header().Set("Content-Type", "application/json")
	//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
	_ = json.NewEncoder(w).Encode(map[string]string{"status": "ok"})
}

func (s *Server) handleRun(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "POST only", http.StatusMethodNotAllowed)
		return
	}
	auth.LogAccess(r, "/api/run")
	ctx, cancel := context.WithTimeout(r.Context(), 60*time.Second)
	defer cancel()

	report, err := measure.RunProbe(ctx)
	if err != nil {
		http.Error(w, fmt.Sprintf("probe failed: %v", err), http.StatusInternalServerError)
		return
	}
	if err := s.cfg.Collector.Ingest(*report); err != nil { //nolint:contextcheck // Ingest takes no ctx today; see handleReport
		log.Printf("ingest error: %v", err)
	}
	w.Header().Set("Content-Type", "application/json")
	//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
	_ = json.NewEncoder(w).Encode(report)
}

func (s *Server) handleInstall(w http.ResponseWriter, r *http.Request) {
	auth.LogAccess(r, "/install")
	w.Header().Set("Content-Type", "text/plain; charset=utf-8")
	//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
	_, _ = w.Write(installScript)
}

// validBinaryTarget gates handleBinary's targetOS / targetArch query params
// against the Block 61H cross-compile matrix. Rejects everything else —
// closes the path-traversal class via the binDir concatenation in
// handleBinary (otherwise `?os=../etc&arch=passwd` resolves to a path
// outside /app/bin/).
func validBinaryTarget(goos, goarch string) bool {
	switch goos {
	case "linux", "darwin", "windows":
	default:
		return false
	}
	switch goarch {
	case "amd64", "arm64":
		return true
	default:
		return false
	}
}

func (s *Server) handleBinary(w http.ResponseWriter, r *http.Request) {
	auth.LogAccess(r, "/bin/snitch")
	// Allow client to request a specific OS/arch; default to server's own platform.
	// Block 61H multi-platform builds — windows variants ship as snitch.exe,
	// linux + darwin as snitch.
	targetOS := r.URL.Query().Get("os")
	if targetOS == "" {
		targetOS = runtime.GOOS
	}
	targetArch := r.URL.Query().Get("arch")
	if targetArch == "" {
		targetArch = runtime.GOARCH
	}
	// Validate against the Block 61H build matrix. Anything outside this
	// allowlist is rejected — closes the path-traversal class on the
	// targetOS / targetArch concat into binDir below (otherwise an
	// attacker could pass `os=../etc&arch=passwd`).
	if !validBinaryTarget(targetOS, targetArch) {
		http.Error(w, "unsupported os/arch", http.StatusBadRequest)
		return
	}
	filename := "snitch"
	if targetOS == "windows" {
		filename = "snitch.exe"
	}
	binDir := "/app/bin/" + targetOS + "-" + targetArch
	// gosec G703 false-positive: targetOS / targetArch are gated by
	// validBinaryTarget above (closed allowlist of GOOS×GOARCH); filename
	// is one of two literal values ("snitch" / "snitch.exe"). No taint.
	if f, err := os.Open(binDir + "/" + filename); err == nil { //nolint:gosec // see comment above

		defer f.Close()
		w.Header().Set("Content-Type", "application/octet-stream")
		w.Header().Set("Content-Disposition", "attachment; filename="+filename)
		http.ServeContent(w, r, filename, time.Time{}, f)
		return
	}
	// Fallback to the running binary (server's native platform only —
	// no cross-arch fallback). The fallback is only useful when a
	// developer is running `snitch serve` outside the production container.
	if targetOS != runtime.GOOS || targetArch != runtime.GOARCH {
		http.Error(w, "no binary for "+targetOS+"/"+targetArch, http.StatusNotFound)
		return
	}
	exe, err := os.Executable()
	if err != nil {
		http.Error(w, "cannot resolve executable path", http.StatusInternalServerError)
		return
	}
	w.Header().Set("Content-Type", "application/octet-stream")
	w.Header().Set("Content-Disposition", "attachment; filename="+filename)
	http.ServeFile(w, r, exe)
}

// --- Analytics endpoints ---

func (s *Server) handleHistory(w http.ResponseWriter, r *http.Request) {
	auth.LogAccess(r, "/api/history")
	rangeStr := r.URL.Query().Get("range")
	if rangeStr == "" {
		rangeStr = "24h"
	}
	from, to, bucket, err := store.ParseRange(rangeStr)
	if err != nil {
		http.Error(w, err.Error(), http.StatusBadRequest)
		return
	}

	st := s.cfg.Collector.Store()
	idm := s.cfg.Collector.Identity()

	fromToken := r.URL.Query().Get("from")
	toToken := r.URL.Query().Get("to")

	var points []store.TimePoint
	if fromToken != "" && toToken != "" {
		points, err = st.LinkTrend(r.Context(), fromToken, toToken, from, to, bucket)
	} else {
		points, err = st.MeshTrend(r.Context(), from, to, bucket)
	}
	if err != nil {
		http.Error(w, fmt.Sprintf("query: %v", err), http.StatusInternalServerError)
		return
	}

	_ = idm // available for future per-link name resolution
	w.Header().Set("Content-Type", "application/json")
	//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
	_ = json.NewEncoder(w).Encode(map[string]any{
		"range":  rangeStr,
		"from":   from,
		"to":     to,
		"points": points,
	})
}

func (s *Server) handleEvents(w http.ResponseWriter, r *http.Request) {
	auth.LogAccess(r, "/api/events")
	rangeStr := r.URL.Query().Get("range")
	if rangeStr == "" {
		rangeStr = "24h"
	}
	from, to, _, err := store.ParseRange(rangeStr)
	if err != nil {
		http.Error(w, err.Error(), http.StatusBadRequest)
		return
	}

	st := s.cfg.Collector.Store()
	idm := s.cfg.Collector.Identity()

	events, err := st.RecentEvents(r.Context(), from, to, 100)
	if err != nil {
		http.Error(w, fmt.Sprintf("query: %v", err), http.StatusInternalServerError)
		return
	}

	// Resolve token -> peer name where possible.
	identities, err := st.AllIdentities(r.Context())
	if err != nil {
		http.Error(w, fmt.Sprintf("load identities: %v", err), http.StatusInternalServerError)
		return
	}
	for i := range events {
		events[i].PeerName = decryptName(idm, identities, events[i].Token)
		if events[i].TargetToken != "" {
			events[i].TargetName = decryptName(idm, identities, events[i].TargetToken)
		}
	}

	w.Header().Set("Content-Type", "application/json")
	//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
	_ = json.NewEncoder(w).Encode(map[string]any{
		"range":  rangeStr,
		"events": events,
	})
}

func (s *Server) handleUptime(w http.ResponseWriter, r *http.Request) {
	auth.LogAccess(r, "/api/uptime")
	rangeStr := r.URL.Query().Get("range")
	if rangeStr == "" {
		rangeStr = "24h"
	}
	from, to, _, err := store.ParseRange(rangeStr)
	if err != nil {
		http.Error(w, err.Error(), http.StatusBadRequest)
		return
	}

	st := s.cfg.Collector.Store()
	idm := s.cfg.Collector.Identity()

	records, err := st.PeerUptime(r.Context(), from, to)
	if err != nil {
		http.Error(w, fmt.Sprintf("query: %v", err), http.StatusInternalServerError)
		return
	}

	identities, err := st.AllIdentities(r.Context())
	if err != nil {
		http.Error(w, fmt.Sprintf("load identities: %v", err), http.StatusInternalServerError)
		return
	}
	for i := range records {
		records[i].PeerName = decryptName(idm, identities, records[i].Token)
	}

	w.Header().Set("Content-Type", "application/json")
	//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
	_ = json.NewEncoder(w).Encode(map[string]any{
		"range":  rangeStr,
		"uptime": records,
	})
}

// --- Key management (admin only) ---

func (s *Server) handleKeys(w http.ResponseWriter, r *http.Request) {
	auth.LogAccess(r, "/api/keys")
	switch r.Method {
	case http.MethodGet:
		keys, err := s.cfg.Auth.ListKeys(r.Context())
		if err != nil {
			http.Error(w, fmt.Sprintf("list keys: %v", err), http.StatusInternalServerError)
			return
		}
		w.Header().Set("Content-Type", "application/json")
		//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
		_ = json.NewEncoder(w).Encode(keys)

	case http.MethodPost:
		var req struct {
			Role         auth.Role `json:"role"`
			Label        string    `json:"label"`
			PlaintextKey string    `json:"plaintext_key,omitempty"` // Block 61: caller-supplied (bundle-create path)
		}
		if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
			http.Error(w, "bad request", http.StatusBadRequest)
			return
		}
		if req.Label == "" {
			http.Error(w, "label required", http.StatusBadRequest)
			return
		}
		// Block 61 path — bundle-create generates the key as part of the
		// offline-CA-rooted bundle issuance ceremony and registers the
		// resulting bearer here. Idempotent on label, no plaintext echoed
		// back (caller already has it; the bundle is the source of truth).
		if req.PlaintextKey != "" {
			if err := s.cfg.Auth.CreateKeyWithPlaintext(r.Context(), req.Role, req.Label, req.PlaintextKey); err != nil {
				http.Error(w, fmt.Sprintf("register key: %v", err), http.StatusInternalServerError)
				return
			}
			w.Header().Set("Content-Type", "application/json")
			//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
			_ = json.NewEncoder(w).Encode(map[string]string{"role": string(req.Role), "label": req.Label, "registered": "true"})
			return
		}
		// Legacy path — collector generates the key and returns plaintext
		// once. Used by ad-hoc `snitch install` flow + operator-curated
		// dashboard buttons.
		key, err := s.cfg.Auth.CreateKey(r.Context(), req.Role, req.Label)
		if err != nil {
			http.Error(w, fmt.Sprintf("create key: %v", err), http.StatusInternalServerError)
			return
		}
		w.Header().Set("Content-Type", "application/json")
		//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
		_ = json.NewEncoder(w).Encode(map[string]string{"key": key, "role": string(req.Role)})

	case http.MethodDelete:
		idStr := r.URL.Query().Get("id")
		id, err := strconv.ParseInt(idStr, 10, 64)
		if err != nil {
			http.Error(w, "id required", http.StatusBadRequest)
			return
		}
		if err := s.cfg.Auth.RevokeKey(r.Context(), id); err != nil {
			http.Error(w, fmt.Sprintf("revoke: %v", err), http.StatusInternalServerError)
			return
		}
		w.Header().Set("Content-Type", "application/json")
		//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
		_ = json.NewEncoder(w).Encode(map[string]string{"status": "revoked"})

	default:
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
	}
}

func (s *Server) handleInventory(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
	_, _ = w.Write(inventoryHTML)
}

func (s *Server) handleAliases(w http.ResponseWriter, r *http.Request) {
	auth.LogAccess(r, "/api/aliases")
	st := s.cfg.Collector.Store()

	switch r.Method {
	case http.MethodGet:
		aliases, err := st.AllAliases(r.Context())
		if err != nil {
			http.Error(w, fmt.Sprintf("query: %v", err), http.StatusInternalServerError)
			return
		}
		w.Header().Set("Content-Type", "application/json")
		//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
		_ = json.NewEncoder(w).Encode(aliases)

	case http.MethodPut:
		var req struct {
			EntityType string `json:"entity_type"`
			EntityKey  string `json:"entity_key"`
			Alias      string `json:"alias"`
		}
		if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
			http.Error(w, "bad request", http.StatusBadRequest)
			return
		}
		if req.EntityType == "" || req.EntityKey == "" || req.Alias == "" {
			http.Error(w, "entity_type, entity_key, and alias required", http.StatusBadRequest)
			return
		}
		if req.EntityType != "peer" && req.EntityType != "gateway" {
			http.Error(w, "entity_type must be peer or gateway", http.StatusBadRequest)
			return
		}
		if err := st.UpsertAlias(r.Context(), req.EntityType, req.EntityKey, req.Alias); err != nil {
			http.Error(w, fmt.Sprintf("upsert: %v", err), http.StatusInternalServerError)
			return
		}
		w.Header().Set("Content-Type", "application/json")
		//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
		_ = json.NewEncoder(w).Encode(map[string]string{"status": "ok"})

	case http.MethodDelete:
		entityType := r.URL.Query().Get("type")
		entityKey := r.URL.Query().Get("key")
		if entityType == "" || entityKey == "" {
			http.Error(w, "type and key query params required", http.StatusBadRequest)
			return
		}
		if err := st.DeleteAlias(r.Context(), entityType, entityKey); err != nil {
			http.Error(w, fmt.Sprintf("delete: %v", err), http.StatusInternalServerError)
			return
		}
		w.Header().Set("Content-Type", "application/json")
		//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
		_ = json.NewEncoder(w).Encode(map[string]string{"status": "deleted"})

	default:
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
	}
}

// decryptName resolves a token to a peer name via the identity manager.
func decryptName(idm *identity.Manager, identities map[string][2][]byte, token string) string {
	enc, ok := identities[token]
	if !ok {
		return token[:8] + "..."
	}
	name, err := idm.Decrypt(enc[0])
	if err != nil {
		return token[:8] + "..."
	}
	return name
}
