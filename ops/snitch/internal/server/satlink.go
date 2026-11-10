package server

import (
	"context"
	_ "embed"
	"encoding/json"
	"errors"
	"fmt"
	"log"
	"net/http"
	"sync"
	"time"

	"github.com/usareur-af/goatcore/ops/snitch/internal/auth"
	"github.com/usareur-af/goatcore/ops/snitch/internal/compat"
	"github.com/usareur-af/goatcore/ops/snitch/internal/measure"
	"github.com/usareur-af/goatcore/ops/snitch/internal/safego"
	"github.com/usareur-af/goatcore/ops/snitch/internal/store"
	"github.com/usareur-af/goatcore/ops/snitch/internal/types"
	"github.com/jackc/pgx/v5"
)

//go:embed satlink.html
var satlinkHTML []byte

var satlinkProbeMu sync.Mutex

// StartSatlinkProber runs the satellite-link PoP benchmark on startup and then
// at the given interval. The probe measures latency from behind the satellite link to
// each relay VPS, identifying the egress ground station / PoP.
func (s *Server) StartSatlinkProber(interval time.Duration) {
	if interval <= 0 {
		log.Println("satlink prober disabled")
		return
	}
	log.Printf("satlink prober enabled: interval=%s", interval)

	// Block 50E.4 (revised, do-no-harm follow-on) — safego.Spawn at the
	// outer goroutine guards against the catastrophic "process exit on
	// any panic" mode. Per-iteration `defer safego.Recover` inside the
	// runOnce closure preserves self-healing: a transient panic in one
	// iteration is logged + counted but the ticker loop continues. The
	// initial naive shape (single outer wrap, no per-iteration recover)
	// caused a permanent prober dead-end on any single panic.
	safego.Spawn("satlink-prober", func() {
		runOnce := func() {
			defer safego.Recover("satlink-iter")
			s.runSatlinkProbe()
		}
		runOnce() // immediate first run to get initial data
		ticker := time.NewTicker(interval)
		defer ticker.Stop()
		for range ticker.C {
			runOnce()
		}
	})
}

func (s *Server) runSatlinkProbe() {
	if !satlinkProbeMu.TryLock() {
		log.Println("satlink: probe already running, skipping")
		return
	}
	defer satlinkProbeMu.Unlock()

	log.Println("satlink: starting PoP benchmark...")
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Minute)
	defer cancel()

	result, err := measure.RunSatlinkProbe(ctx)
	if err != nil {
		log.Printf("satlink: probe failed: %v", err)
		return
	}

	st := s.cfg.Collector.Store()
	probeID, err := st.InsertSatlinkProbe(ctx, result)
	if err != nil {
		log.Printf("satlink: store probe: %v", err)
		return
	}
	for _, m := range result.Measurements {
		if err := st.InsertSatlinkMeasurement(ctx, probeID, &m); err != nil {
			log.Printf("satlink: store measurement %s: %v", m.Target, err)
		}
	}

	log.Printf("satlink: probe complete — egress=%s city=%s country=%s asn=%s satlink_range=%v",
		result.EgressIP, result.City, result.Country, result.ASN, result.InSatlinkRange)
	for _, m := range result.Measurements {
		log.Printf("satlink:   %s → ping=%.1fms(p95=%.1f jitter=%.1f) tcp=%.1fms hops=%d",
			m.Target, m.PingAvg, m.PingP95, m.PingJitter, m.TCPAvg, m.HopCount)
	}
}

// registerSatlinkRoutes adds satellite-link PoP tracker endpoints.
func (s *Server) registerSatlinkRoutes() {
	a := s.cfg.Auth

	// Each route is served under its current path and its earlier compat
	// path, so existing bookmarks and API clients keep working.
	// Where a compat path is the same string as the current one it is
	// registered once (registering a pattern twice panics).
	for _, page := range distinct("/satlink", compat.SatlinkPageRoute) {
		s.mux.HandleFunc(page, s.handleSatlinkPage)
	}
	for _, prefix := range distinct("/api/satlink/", compat.SatlinkAPIPrefix) {
		s.mux.Handle(prefix+"latest", a.Require(auth.RoleViewer, s.handleSatlinkLatest))
		s.mux.Handle(prefix+"trend", a.Require(auth.RoleViewer, s.handleSatlinkTrend))
		s.mux.Handle(prefix+"pops", a.Require(auth.RoleViewer, s.handleSatlinkPops))
		s.mux.Handle(prefix+"probe", a.Require(auth.RoleOperator, s.handleSatlinkRunProbe))
	}
}

// distinct returns a and b, or a alone when they are the same string.
func distinct(a, b string) []string {
	if a == b {
		return []string{a}
	}
	return []string{a, b}
}

func (s *Server) handleSatlinkPage(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
	_, _ = w.Write(satlinkHTML)
}

func (s *Server) handleSatlinkLatest(w http.ResponseWriter, r *http.Request) {
	auth.LogAccess(r, "/api/satlink/latest")
	st := s.cfg.Collector.Store()

	latest, err := st.SatlinkLatest(r.Context())
	if err != nil {
		w.Header().Set("Content-Type", "application/json")
		//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
		_ = json.NewEncoder(w).Encode(map[string]any{"latest": nil, "previous": nil})
		return
	}

	previous, err := optionalSatlinkPrevious(st.SatlinkPrevious(r.Context()))
	if err != nil {
		http.Error(w, fmt.Sprintf("query previous sample: %v", err), http.StatusInternalServerError)
		return
	}

	w.Header().Set("Content-Type", "application/json")
	//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
	_ = json.NewEncoder(w).Encode(map[string]any{
		"latest":   latest,
		"previous": previous,
	})
}

// optionalSatlinkPrevious distinguishes the expected first-sample state
// from a database failure. A latest sample is useful even when there is no
// earlier sample to compare it with.
func optionalSatlinkPrevious(previous *types.SatlinkProbeResult, err error) (*types.SatlinkProbeResult, error) {
	if errors.Is(err, pgx.ErrNoRows) {
		return nil, nil
	}
	return previous, err
}

func (s *Server) handleSatlinkTrend(w http.ResponseWriter, r *http.Request) {
	auth.LogAccess(r, "/api/satlink/trend")
	rangeStr := r.URL.Query().Get("range")
	if rangeStr == "" {
		rangeStr = "30d"
	}
	from, to, _, err := store.ParseRange(rangeStr)
	if err != nil {
		http.Error(w, err.Error(), http.StatusBadRequest)
		return
	}

	points, err := s.cfg.Collector.Store().SatlinkTrend(r.Context(), from, to)
	if err != nil {
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}

	w.Header().Set("Content-Type", "application/json")
	//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
	_ = json.NewEncoder(w).Encode(map[string]any{
		"range":  rangeStr,
		"points": points,
	})
}

func (s *Server) handleSatlinkPops(w http.ResponseWriter, r *http.Request) {
	auth.LogAccess(r, "/api/satlink/pops")
	rangeStr := r.URL.Query().Get("range")
	if rangeStr == "" {
		rangeStr = "30d"
	}
	from, to, _, err := store.ParseRange(rangeStr)
	if err != nil {
		http.Error(w, err.Error(), http.StatusBadRequest)
		return
	}

	pops, err := s.cfg.Collector.Store().SatlinkPops(r.Context(), from, to)
	if err != nil {
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}

	w.Header().Set("Content-Type", "application/json")
	//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
	_ = json.NewEncoder(w).Encode(map[string]any{
		"range": rangeStr,
		"pops":  pops,
	})
}

func (s *Server) handleSatlinkRunProbe(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "POST only", http.StatusMethodNotAllowed)
		return
	}
	auth.LogAccess(r, "/api/satlink/probe")

	// Block 50E.4 — safego.Spawn so a panic in the admin-triggered
	// ad-hoc probe doesn't kill the listener. The probe is fire-and-
	// forget; deliberately decoupled from r.Context() so the operator
	// can disconnect without aborting an in-flight ~30s probe.
	safego.Spawn("satlink-adhoc-probe", func() { //nolint:contextcheck // intentional ctx detach; see comment
		s.runSatlinkProbe()
	})

	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(http.StatusAccepted)
	//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
	_ = json.NewEncoder(w).Encode(map[string]string{"status": "started"})
}
