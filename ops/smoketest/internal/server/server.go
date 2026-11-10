// Package server provides the HTTP API for the smoketest dashboard.
package server

import (
	"context"
	"encoding/json"
	"fmt"
	"log"
	"net/http"
	"os"
	pathpkg "path"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/usareur-af/goatcore/ops/smoketest/internal/netbird"
	"github.com/usareur-af/goatcore/ops/smoketest/internal/probe"
	"github.com/usareur-af/goatcore/ops/smoketest/internal/store"
)

// ServiceEntry describes one service in the catalogue.
type ServiceEntry struct {
	Name        string `json:"name"`
	Description string `json:"description"`
	URL         string `json:"url,omitempty"`
	Category    string `json:"category"`
}

// ServerConfig holds all dependencies for the HTTP server.
type ServerConfig struct {
	ProbeCfg     *probe.Config
	Store        *store.Store
	Netbird      *netbird.Client
	Catalogue    []ServiceEntry
	DashboardDir string // directory of the dashboard's static files (ops/smoketest/dashboard)
}

// Server is the HTTP API server.
type Server struct {
	cfg          *probe.Config
	store        *store.Store
	nb           *netbird.Client
	catalogue    []ServiceEntry
	dashboardDir string

	mu        sync.Mutex
	running   bool
	runID     string
	runStart  time.Time
	runCancel context.CancelFunc
}

// New creates a Server from the given config.
func New(cfg ServerConfig) *Server {
	return &Server{
		cfg:          cfg.ProbeCfg,
		store:        cfg.Store,
		nb:           cfg.Netbird,
		catalogue:    cfg.Catalogue,
		dashboardDir: cfg.DashboardDir,
	}
}

// ListenAndServe starts the HTTP server on the given address (e.g. ":8080").
func (s *Server) ListenAndServe(addr string) error {
	mux := http.NewServeMux()

	// API routes first, then dashboard static files as fallback.
	mux.HandleFunc("/api/run", s.handleRun)
	mux.HandleFunc("/api/run/status", s.handleRunStatus)
	mux.HandleFunc("/api/results", s.handleResults)
	mux.HandleFunc("/api/results/latest", s.handleResultsLatest)
	mux.HandleFunc("/api/peers", s.handlePeers)
	mux.HandleFunc("/api/services", s.handleServices)
	mux.HandleFunc("/api/catalogue", s.handleCatalogue)

	// Serve dashboard from dist dir if available, otherwise use embedded HTML.
	if s.dashboardDir != "" {
		fs := http.FileServer(http.Dir(s.dashboardDir))
		mux.Handle("/", spaHandler(fs, s.dashboardDir))
	} else {
		mux.HandleFunc("/", s.handleDashboard)
	}

	handler := corsMiddleware(mux)

	log.Printf("smoketest server listening on %s", addr)
	return http.ListenAndServe(addr, handler)
}

// corsMiddleware adds permissive CORS headers for local development.
func corsMiddleware(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Access-Control-Allow-Origin", "*")
		w.Header().Set("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
		w.Header().Set("Access-Control-Allow-Headers", "Content-Type, Authorization")
		if r.Method == http.MethodOptions {
			w.WriteHeader(http.StatusNoContent)
			return
		}
		next.ServeHTTP(w, r)
	})
}

// spaHandler wraps a file server to serve index.html for non-file paths (SPA routing).
func spaHandler(fs http.Handler, dir string) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		path := r.URL.Path
		// Try the file first. If it doesn't exist, serve index.html.
		if path != "/" {
			// Clean against a rooted path so ".." cannot climb out of dir; the
			// stat must not probe files the file server itself would refuse.
			fullPath := filepath.Join(dir, filepath.FromSlash(pathpkg.Clean("/"+path)))
			if _, err := os.Stat(fullPath); os.IsNotExist(err) {
				r.URL.Path = "/"
			}
		}
		fs.ServeHTTP(w, r)
	})
}

// handleDashboard serves a simple placeholder HTML page.
func (s *Server) handleDashboard(w http.ResponseWriter, r *http.Request) {
	if r.URL.Path != "/" {
		http.NotFound(w, r)
		return
	}
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	fmt.Fprint(w, dashboardHTML)
}

// handleRun triggers a smoke test run (POST only).
func (s *Server) handleRun(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		writeError(w, http.StatusMethodNotAllowed, "POST required")
		return
	}

	s.mu.Lock()
	if s.running {
		s.mu.Unlock()
		writeError(w, http.StatusConflict, "a run is already in progress")
		return
	}

	// Build per-run config from query params.
	cfg := *s.cfg
	if mode := r.URL.Query().Get("mode"); mode == "quick" || mode == "full" {
		cfg.Mode = mode
	}
	if cats := r.URL.Query().Get("categories"); cats != "" {
		cfg.Categories = nil
		for _, c := range strings.Split(cats, ",") {
			c = strings.TrimSpace(c)
			if c != "" {
				cfg.Categories = append(cfg.Categories, probe.Category(c))
			}
		}
	}

	runner := probe.NewRunner(&cfg)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)

	runID := time.Now().UTC().Format("20060102T150405")
	s.running = true
	s.runID = runID
	s.runStart = time.Now()
	s.runCancel = cancel
	s.mu.Unlock()

	go func() {
		defer cancel()
		result := runner.Run(ctx)

		if err := s.store.Append(result); err != nil {
			log.Printf("failed to store run result: %v", err)
		}

		s.mu.Lock()
		s.running = false
		s.runCancel = nil
		s.mu.Unlock()
	}()

	writeJSON(w, http.StatusAccepted, map[string]any{
		"status": "started",
		"run_id": runID,
	})
}

// handleRunStatus returns whether a run is in progress.
func (s *Server) handleRunStatus(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		writeError(w, http.StatusMethodNotAllowed, "GET required")
		return
	}

	s.mu.Lock()
	running := s.running
	runID := s.runID
	start := s.runStart
	s.mu.Unlock()

	elapsed := 0.0
	if running {
		elapsed = time.Since(start).Seconds()
	}

	writeJSON(w, http.StatusOK, map[string]any{
		"running":   running,
		"run_id":    runID,
		"elapsed_s": elapsed,
	})
}

// handleResults returns the last N results from the store.
func (s *Server) handleResults(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		writeError(w, http.StatusMethodNotAllowed, "GET required")
		return
	}

	limit := 20
	if v := r.URL.Query().Get("limit"); v != "" {
		if n, err := strconv.Atoi(v); err == nil && n > 0 {
			limit = n
		}
	}

	results, err := s.store.List(limit)
	if err != nil {
		writeError(w, http.StatusInternalServerError, err.Error())
		return
	}

	writeJSON(w, http.StatusOK, results)
}

// handleResultsLatest returns the most recent result.
func (s *Server) handleResultsLatest(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		writeError(w, http.StatusMethodNotAllowed, "GET required")
		return
	}

	latest, err := s.store.Latest()
	if err != nil {
		writeError(w, http.StatusInternalServerError, err.Error())
		return
	}
	if latest == nil {
		writeJSON(w, http.StatusOK, nil)
		return
	}

	writeJSON(w, http.StatusOK, latest)
}

// handlePeers proxies to the Netbird API to list peers.
func (s *Server) handlePeers(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		writeError(w, http.StatusMethodNotAllowed, "GET required")
		return
	}

	if s.nb == nil {
		writeError(w, http.StatusServiceUnavailable, "netbird client not configured (missing NETBIRD_API_TOKEN)")
		return
	}

	peers, err := s.nb.ListPeers(r.Context())
	if err != nil {
		writeError(w, http.StatusBadGateway, err.Error())
		return
	}

	writeJSON(w, http.StatusOK, peers)
}

// handleServices runs service catalogue probes and returns results.
func (s *Server) handleServices(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		writeError(w, http.StatusMethodNotAllowed, "GET required")
		return
	}

	cfg := *s.cfg
	cfg.Categories = []probe.Category{probe.CatService}
	runner := probe.NewRunner(&cfg)
	result := runner.Run(r.Context())

	writeJSON(w, http.StatusOK, result)
}

// handleCatalogue returns the raw service catalogue as JSON.
func (s *Server) handleCatalogue(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		writeError(w, http.StatusMethodNotAllowed, "GET required")
		return
	}

	writeJSON(w, http.StatusOK, s.catalogue)
}

// writeJSON marshals v as JSON and writes it with the given status code.
func writeJSON(w http.ResponseWriter, status int, v any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	if v == nil {
		_, _ = w.Write([]byte("null"))
		return
	}
	enc := json.NewEncoder(w)
	enc.SetIndent("", "  ")
	if err := enc.Encode(v); err != nil {
		log.Printf("json encode error: %v", err)
	}
}

// writeError writes a JSON error response.
func writeError(w http.ResponseWriter, status int, msg string) {
	writeJSON(w, status, map[string]string{"error": msg})
}

const dashboardHTML = `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>goat Smoke Test</title>
<style>
  body { font-family: system-ui, sans-serif; max-width: 960px; margin: 2rem auto; padding: 0 1rem; background: #0d1117; color: #c9d1d9; }
  h1 { color: #58a6ff; }
  pre { background: #161b22; padding: 1rem; border-radius: 6px; overflow-x: auto; }
  .pass { color: #3fb950; } .fail { color: #f85149; } .warn { color: #d29922; }
  button { background: #238636; color: #fff; border: none; padding: 0.5rem 1rem; border-radius: 6px; cursor: pointer; font-size: 1rem; }
  button:hover { background: #2ea043; }
  button:disabled { opacity: 0.5; cursor: not-allowed; }
  #status { margin: 1rem 0; }
  #results { white-space: pre-wrap; }
</style>
</head>
<body>
<h1>goat Smoke Test</h1>
<button id="run-btn" onclick="triggerRun()">Run Smoke Test</button>
<div id="status"></div>
<pre id="results">Loading latest results...</pre>
<script>
async function triggerRun() {
  const btn = document.getElementById('run-btn');
  btn.disabled = true;
  try {
    const resp = await fetch('/api/run', {method: 'POST'});
    const data = await resp.json();
    if (resp.status === 409) {
      document.getElementById('status').textContent = 'A run is already in progress.';
    } else {
      document.getElementById('status').textContent = 'Run started: ' + data.run_id;
      pollStatus();
    }
  } catch (e) {
    document.getElementById('status').textContent = 'Error: ' + e.message;
  } finally {
    btn.disabled = false;
  }
}

async function pollStatus() {
  const statusEl = document.getElementById('status');
  const poll = setInterval(async () => {
    try {
      const resp = await fetch('/api/run/status');
      const data = await resp.json();
      if (data.running) {
        statusEl.textContent = 'Running (' + data.elapsed_s.toFixed(1) + 's)...';
      } else {
        clearInterval(poll);
        statusEl.textContent = 'Complete.';
        loadLatest();
      }
    } catch (e) {
      clearInterval(poll);
    }
  }, 1000);
}

async function loadLatest() {
  try {
    const resp = await fetch('/api/results/latest');
    const data = await resp.json();
    if (!data) { document.getElementById('results').textContent = 'No results yet.'; return; }
    let out = 'Run: ' + data.run_id + '  Mode: ' + data.mode + '  Duration: ' + data.duration_s.toFixed(1) + 's\n';
    out += 'PASS: ' + data.pass + '  FAIL: ' + data.fail + '  WARN: ' + data.warn + '\n\n';
    for (const c of (data.checks || [])) {
      out += '[' + c.category + '] ' + c.name + '  ' + c.status + '  ' + c.detail + '\n';
    }
    document.getElementById('results').textContent = out;
  } catch (e) {
    document.getElementById('results').textContent = 'Failed to load results.';
  }
}

loadLatest();
</script>
</body>
</html>`
