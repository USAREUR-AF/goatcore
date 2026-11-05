// admit-audit — a small read-only sidecar that watches the local
// Netbird management Postgres for new `peers` rows (admits) and
// emits structured log lines + Prometheus metrics.
//
// C1 of docs/design/mgmt-plane-threat-mitigations.md §2.1
// ("Audit log on every admit operation"). C3 (IOC alert rules) is
// delivered as ops/compose/mgmt-host/monitoring/rules/admit-ioc.yml
// and consumes the metrics exposed here.
//
// Threat model — coincident-infiltration + partition
// ---------------------------------------------------
// An adversary who compromises a site AND severs its uplink can
// admit themselves silently during the partition; the peer row
// replicates cluster-wide on reconnect. The attack signature is a
// cluster of enrollments during a partition window from a single
// source-user with fresh WireGuard keys.
//
// This sidecar surfaces that signature by:
//
//  1. Emitting one JSON line per detected admit (source_user,
//     wg_key, ip, meta_hostname if present), shipped off-host by
//     the usual docker-logs / log-shipping path.
//  2. Exposing Prometheus metrics on :9124, scraped by the local
//     Prometheus from docker-compose.obs.yml. Metrics include a
//     rolling-window view of admit density, per-source-user burst,
//     and distinct-key counts — the inputs to the IOC alert rules.
//
// Design constraints
// ------------------
//   - Read-only. Role: admit_audit, SELECT-only on peers. Created
//     by ops/ansible/roles/admit-audit/ before this sidecar runs.
//   - In-memory state only. Process restart re-emits "detection"
//     for any peer row already in the table within the retention
//     window; that's the same-failure-mode as any tail-based audit,
//     and acceptable because C1 is a signal-generator not a
//     tamper-proof ledger.
//   - Poll-based, 30s default. Netbird's peer-admit path is a GORM
//     INSERT — no LISTEN/NOTIFY hook to latch onto without patching
//     the management server.
//
// Schema gap — no `created_at` on `peers`
// ----------------------------------------
// Netbird's peers table has `peer_status_last_seen` (updated on
// every heartbeat) but no row-insert timestamp. We use the
// sidecar's own detection time as a best-effort proxy. Worst case:
// peer_audit_detection_latency undercounts real admit→detection
// delay on cold-start (rows already present look instant). This is
// called out in the README and on the metric itself.
//
// Build:  go build ./...
// Run:    ADMIT_AUDIT_DSN=... SITE_NAME=site-a ./admit-audit
package main

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"log"
	"net/http"
	"os"
	"os/signal"
	"sort"
	"sync"
	"syscall"
	"time"

	_ "github.com/jackc/pgx/v5/stdlib"
	"github.com/prometheus/client_golang/prometheus"
	"github.com/prometheus/client_golang/prometheus/promhttp"
)

// ---------------------------------------------------------------------------
// Config.
// ---------------------------------------------------------------------------

type config struct {
	DSN            string
	Site           string
	PollInterval   time.Duration
	WindowDuration time.Duration
	ListenAddr     string
}

func parseFlags() config {
	var (
		dsn      = flag.String("dsn", envDefault("ADMIT_AUDIT_DSN", "postgres://admit_audit@127.0.0.1:5432/netbird?sslmode=disable"), "read-only Postgres DSN")
		site     = flag.String("site", envDefault("SITE_NAME", ""), "site label (required; e.g. alpha)")
		poll     = flag.Duration("poll", 30*time.Second, "poll interval")
		window   = flag.Duration("window", 5*time.Minute, "rolling window for burst/density metrics")
		listen   = flag.String("listen", "127.0.0.1:9124", "listen addr for /metrics")
	)
	flag.Parse()

	if *site == "" {
		fmt.Fprintln(os.Stderr, "-site / SITE_NAME required")
		flag.Usage()
		os.Exit(2)
	}
	return config{
		DSN:            *dsn,
		Site:           *site,
		PollInterval:   *poll,
		WindowDuration: *window,
		ListenAddr:     *listen,
	}
}

func envDefault(key, def string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return def
}

// ---------------------------------------------------------------------------
// Peer row + rolling window.
// ---------------------------------------------------------------------------

// peerRow holds the subset of the Netbird peers table we surface.
// Netbird's GORM model does NOT expose a row-insert timestamp on
// this table; see package doc for how we approximate.
type peerRow struct {
	ID           string
	AccountID    string
	Key          string // WG pubkey
	IP           string
	Name         string
	UserID       string // the "source_user" — who admitted this peer
	MetaHostname string // best-effort; falls back to empty if column missing
}

// admitEvent is one detection event, tagged with the detection time.
type admitEvent struct {
	DetectedAt time.Time
	Peer       peerRow
}

// rollingWindow keeps the last window-duration of admits for cheap
// metric computation. Not thread-safe on its own — caller uses a
// mutex around update + read.
type rollingWindow struct {
	events []admitEvent
	window time.Duration
}

func (w *rollingWindow) add(e admitEvent) {
	w.events = append(w.events, e)
	w.prune(e.DetectedAt)
}

func (w *rollingWindow) prune(now time.Time) {
	cutoff := now.Add(-w.window)
	// Events are appended in order, so a single forward scan suffices.
	i := 0
	for ; i < len(w.events); i++ {
		if !w.events[i].DetectedAt.Before(cutoff) {
			break
		}
	}
	if i > 0 {
		w.events = w.events[i:]
	}
}

// summary returns the three rolling-window metrics we expose as
// gauges to Prometheus: total admits, max per-source-user, and
// distinct WG-key count. All computed over events in the current
// window.
func (w *rollingWindow) summary(now time.Time) (total int, perUserMax int, distinctKeys int) {
	w.prune(now)
	perUser := make(map[string]int)
	keys := make(map[string]struct{})
	for _, e := range w.events {
		perUser[e.Peer.UserID]++
		keys[e.Peer.Key] = struct{}{}
	}
	total = len(w.events)
	for _, c := range perUser {
		if c > perUserMax {
			perUserMax = c
		}
	}
	distinctKeys = len(keys)
	return
}

// ---------------------------------------------------------------------------
// Prometheus metrics.
// ---------------------------------------------------------------------------

type metrics struct {
	admitsTotal         *prometheus.CounterVec
	peersWindowTotal    *prometheus.GaugeVec
	peersPerSourceMax   *prometheus.GaugeVec
	distinctKeysWindow  *prometheus.GaugeVec
	detectionLatencySec prometheus.Histogram // detection-vs-poll-tick delay (NOT admit→detection)
	pollErrors          prometheus.Counter
}

func newMetrics(reg *prometheus.Registry) *metrics {
	m := &metrics{
		admitsTotal: prometheus.NewCounterVec(prometheus.CounterOpts{
			Name: "admit_audit_admits_total",
			Help: "Count of admit (new peer row) events detected by the sidecar.",
		}, []string{"site", "account_id", "source_user"}),
		peersWindowTotal: prometheus.NewGaugeVec(prometheus.GaugeOpts{
			Name: "admit_audit_peers_total_bywindow",
			Help: "Count of admits in the rolling window.",
		}, []string{"site", "window"}),
		peersPerSourceMax: prometheus.NewGaugeVec(prometheus.GaugeOpts{
			Name: "admit_audit_peers_per_source_user_max",
			Help: "Highest per-source-user admit count in the rolling window.",
		}, []string{"site", "window"}),
		distinctKeysWindow: prometheus.NewGaugeVec(prometheus.GaugeOpts{
			Name: "admit_audit_distinct_wg_keys_bywindow",
			Help: "Count of distinct WireGuard keys observed across admits in the rolling window.",
		}, []string{"site", "window"}),
		detectionLatencySec: prometheus.NewHistogram(prometheus.HistogramOpts{
			Name:    "admit_audit_poll_tick_latency_seconds",
			Help:    "Wall-clock latency of each polling tick (SELECT + window update). NOT admit→detection latency; Netbird's peers table has no row-insert timestamp.",
			Buckets: prometheus.DefBuckets,
		}),
		pollErrors: prometheus.NewCounter(prometheus.CounterOpts{
			Name: "admit_audit_poll_errors_total",
			Help: "Polling errors (connection / query failures).",
		}),
	}
	reg.MustRegister(
		m.admitsTotal,
		m.peersWindowTotal,
		m.peersPerSourceMax,
		m.distinctKeysWindow,
		m.detectionLatencySec,
		m.pollErrors,
	)
	return m
}

// ---------------------------------------------------------------------------
// DB layer.
// ---------------------------------------------------------------------------

// queryPeers fetches the current peers snapshot. We run a COALESCE on
// columns that may or may not exist in a given Netbird version; the
// `meta_hostname` fallback specifically handles schema drift on the
// Netbird side without forcing a hard dep on that column.
//
// The retry on undefined_column (SQLSTATE 42703) lets us downgrade
// gracefully to a minimal column set if the target Netbird version
// has renamed or dropped meta_hostname.
func queryPeers(ctx context.Context, db *sql.DB) ([]peerRow, error) {
	full := `
		SELECT id,
		       account_id,
		       key,
		       COALESCE(ip::text, '')      AS ip,
		       COALESCE(name, '')          AS name,
		       COALESCE(user_id, '')       AS user_id,
		       COALESCE(meta_hostname, '') AS meta_hostname
		FROM peers
	`
	rows, err := db.QueryContext(ctx, full)
	if err != nil {
		// Fall back to the minimal set if meta_hostname doesn't exist.
		if isUndefinedColumn(err) {
			return queryPeersMinimal(ctx, db)
		}
		return nil, fmt.Errorf("query peers: %w", err)
	}
	defer rows.Close()

	var out []peerRow
	for rows.Next() {
		var p peerRow
		if err := rows.Scan(&p.ID, &p.AccountID, &p.Key, &p.IP, &p.Name, &p.UserID, &p.MetaHostname); err != nil {
			return nil, fmt.Errorf("scan peer: %w", err)
		}
		out = append(out, p)
	}
	return out, rows.Err()
}

func queryPeersMinimal(ctx context.Context, db *sql.DB) ([]peerRow, error) {
	const q = `
		SELECT id,
		       account_id,
		       key,
		       COALESCE(ip::text, '')      AS ip,
		       COALESCE(name, '')          AS name,
		       COALESCE(user_id, '')       AS user_id
		FROM peers
	`
	rows, err := db.QueryContext(ctx, q)
	if err != nil {
		return nil, fmt.Errorf("query peers (minimal): %w", err)
	}
	defer rows.Close()
	var out []peerRow
	for rows.Next() {
		var p peerRow
		if err := rows.Scan(&p.ID, &p.AccountID, &p.Key, &p.IP, &p.Name, &p.UserID); err != nil {
			return nil, fmt.Errorf("scan peer (minimal): %w", err)
		}
		out = append(out, p)
	}
	return out, rows.Err()
}

// isUndefinedColumn returns true for Postgres SQLSTATE 42703. We match on the
// error string rather than importing the driver's error type — keeps the dep
// surface small.
//
// NOTE: under lib/pq this check could never fire. lib/pq's
// Error() is `"pq: " + err.Message` — the SQLSTATE never appears in the
// string, so the substring match was dead code for as long as it existed.
// pgx's PgError.Error() is `Severity + ": " + Message + " (SQLSTATE " + Code
// + ")"`, so the code IS present and the check works as originally intended.
// Behavior therefore changes with the driver swap: this returns true where it
// used to always return false.
func isUndefinedColumn(err error) bool {
	if err == nil {
		return false
	}
	// pgx surfaces the SQLSTATE as part of the Error() string.
	return containsCode(err.Error(), "42703")
}

func containsCode(s, code string) bool {
	for i := 0; i+len(code) <= len(s); i++ {
		if s[i:i+len(code)] == code {
			return true
		}
	}
	return false
}

// ---------------------------------------------------------------------------
// Main loop.
// ---------------------------------------------------------------------------

// auditor is the full runtime state: DB, metrics, known-IDs set,
// rolling window. Single-goroutine; the polling loop owns the mutex
// and the HTTP /metrics handler never touches it directly — the
// metrics are published as Prometheus collectors which handle
// concurrency themselves.
type auditor struct {
	cfg     config
	db      *sql.DB
	m       *metrics
	logger  *log.Logger
	mu      sync.Mutex
	known   map[string]struct{}
	window  *rollingWindow
	started time.Time
}

func newAuditor(cfg config, db *sql.DB, m *metrics, logger *log.Logger) *auditor {
	return &auditor{
		cfg:     cfg,
		db:      db,
		m:       m,
		logger:  logger,
		known:   make(map[string]struct{}),
		window:  &rollingWindow{window: cfg.WindowDuration},
		started: time.Now(),
	}
}

// run blocks until ctx is canceled. Emits an initial snapshot of
// every existing peer as "known" (seed) WITHOUT generating admit
// events — we only want to alert on new admits observed by this
// process, not on the entire existing peer population.
func (a *auditor) run(ctx context.Context) error {
	// Initial seed pass: populate known-IDs without generating events.
	seed, err := queryPeers(ctx, a.db)
	if err != nil {
		a.logger.Printf("seed query failed: %v", err)
		a.m.pollErrors.Inc()
		// Continue — the next tick will retry. We don't want to
		// die on a transient DB blip during compose up.
	} else {
		for _, p := range seed {
			a.known[p.ID] = struct{}{}
		}
		a.emitEvent("seed", len(seed), "")
	}

	ticker := time.NewTicker(a.cfg.PollInterval)
	defer ticker.Stop()

	for {
		select {
		case <-ctx.Done():
			return nil
		case <-ticker.C:
			if err := a.pollOnce(ctx); err != nil {
				a.logger.Printf("poll error: %v", err)
				a.m.pollErrors.Inc()
			}
		}
	}
}

// pollOnce runs one detection cycle: SELECT, diff vs known, emit
// events + update metrics for each new row.
//
// Each cycle is wrapped in an OTel root span (admit-audit.poll) per
// Block 27C. New admits are attached as span events on the cycle span;
// the cycle span carries summary attributes (peers_total, admits_detected,
// poll_outcome, window_*) regardless of error path.
func (a *auditor) pollOnce(ctx context.Context) error {
	cycleCtx, span := startPollSpan(ctx)
	defer span.End()

	t0 := time.Now()
	defer func() {
		a.m.detectionLatencySec.Observe(time.Since(t0).Seconds())
	}()

	peers, err := queryPeers(cycleCtx, a.db)
	if err != nil {
		recordPollSpan(span, a.cfg.Site, 0, 0, 0, 0, 0, err)
		return err
	}

	a.mu.Lock()
	defer a.mu.Unlock()

	now := time.Now()
	// Sort for deterministic JSON-line ordering — helps tests + log-replay.
	sort.Slice(peers, func(i, j int) bool { return peers[i].ID < peers[j].ID })

	admitsDetected := 0
	for _, p := range peers {
		if _, seen := a.known[p.ID]; seen {
			continue
		}
		a.known[p.ID] = struct{}{}
		a.window.add(admitEvent{DetectedAt: now, Peer: p})

		a.m.admitsTotal.With(prometheus.Labels{
			"site":        a.cfg.Site,
			"account_id":  p.AccountID,
			"source_user": p.UserID,
		}).Inc()

		a.emitAdmit(p, now)
		recordAdmitEvent(span, p, now)
		admitsDetected++
	}

	// Recompute rolling-window gauges every tick regardless of
	// whether new admits fired — the window decays even with no
	// new inserts, and Prometheus needs the gauge refreshed.
	total, perUserMax, distinctKeys := a.window.summary(now)
	windowLabel := a.cfg.WindowDuration.String()
	a.m.peersWindowTotal.WithLabelValues(a.cfg.Site, windowLabel).Set(float64(total))
	a.m.peersPerSourceMax.WithLabelValues(a.cfg.Site, windowLabel).Set(float64(perUserMax))
	a.m.distinctKeysWindow.WithLabelValues(a.cfg.Site, windowLabel).Set(float64(distinctKeys))

	recordPollSpan(span, a.cfg.Site, len(peers), admitsDetected, total, perUserMax, distinctKeys, nil)
	return nil
}

// emitAdmit writes one JSON line per detected admit. Field names
// match the event schema called out in the task spec.
func (a *auditor) emitAdmit(p peerRow, detectedAt time.Time) {
	rec := map[string]any{
		"ts":            detectedAt.UTC().Format(time.RFC3339Nano),
		"event":         "admit",
		"site":          a.cfg.Site,
		"peer_id":       p.ID,
		"account_id":    p.AccountID,
		"wg_key":        p.Key,
		"ip":            p.IP,
		"name":          p.Name,
		"source_user":   p.UserID,
		"meta_hostname": p.MetaHostname,
	}
	b, err := json.Marshal(rec)
	if err != nil {
		a.logger.Printf("marshal admit: %v", err)
		return
	}
	fmt.Println(string(b))
}

func (a *auditor) emitEvent(ev string, count int, extra string) {
	rec := map[string]any{
		"ts":    time.Now().UTC().Format(time.RFC3339Nano),
		"event": ev,
		"site":  a.cfg.Site,
		"count": count,
	}
	if extra != "" {
		rec["note"] = extra
	}
	b, err := json.Marshal(rec)
	if err != nil {
		a.logger.Printf("marshal %s event: %v", ev, err)
		return
	}
	fmt.Println(string(b))
}

// ---------------------------------------------------------------------------
// Entrypoint.
// ---------------------------------------------------------------------------

func main() {
	cfg := parseFlags()
	logger := log.New(os.Stderr, "admit-audit ", log.LstdFlags|log.Lmicroseconds)

	// OpenTelemetry — best-effort. Init failure logged to stderr; never
	// blocks the daemon. Shutdown is bounded so a slow / unreachable
	// endpoint cannot stall SIGTERM cleanup. Per Block 27C scoping.
	otelInitCtx, otelInitCancel := context.WithTimeout(context.Background(), 5*time.Second)
	otelShutdown, otelErr := initOTel(otelInitCtx)
	otelInitCancel()
	if otelErr != nil {
		fmt.Fprintln(os.Stderr, "otel init failed:", otelErr)
		// Continue — telemetry is best-effort.
	} else {
		defer func() {
			sCtx, sCancel := context.WithTimeout(context.Background(), 5*time.Second)
			defer sCancel()
			if err := otelShutdown(sCtx); err != nil {
				fmt.Fprintln(os.Stderr, "otel shutdown:", err)
			}
		}()
	}

	db, err := sql.Open("pgx", cfg.DSN)
	if err != nil {
		logger.Fatalf("sql.Open: %v", err)
	}
	defer db.Close()
	db.SetMaxOpenConns(2)
	db.SetConnMaxIdleTime(5 * time.Minute)

	// Retry-with-backoff ping so a cold-start behind postgres-a's
	// healthcheck doesn't crash-loop the container.
	if err := waitForDB(context.Background(), db, 30*time.Second); err != nil {
		logger.Fatalf("db unreachable: %v", err)
	}

	reg := prometheus.NewRegistry()
	m := newMetrics(reg)

	a := newAuditor(cfg, db, m, logger)

	ctx, cancel := signal.NotifyContext(context.Background(), syscall.SIGINT, syscall.SIGTERM)
	defer cancel()

	// /metrics server.
	mux := http.NewServeMux()
	mux.Handle("/metrics", promhttp.HandlerFor(reg, promhttp.HandlerOpts{}))
	mux.HandleFunc("/healthz", func(w http.ResponseWriter, r *http.Request) {
		if err := db.PingContext(r.Context()); err != nil {
			http.Error(w, err.Error(), http.StatusServiceUnavailable)
			return
		}
		w.WriteHeader(http.StatusOK)
		//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
		_, _ = w.Write([]byte("ok"))
	})
	srv := &http.Server{
		Addr:              cfg.ListenAddr,
		Handler:           mux,
		ReadHeaderTimeout: 5 * time.Second,
	}
	go func() {
		logger.Printf("listening on %s", cfg.ListenAddr)
		if err := srv.ListenAndServe(); err != nil && !errors.Is(err, http.ErrServerClosed) {
			logger.Printf("http: %v", err)
			cancel()
		}
	}()

	// Run the auditor until ctx is canceled.
	if err := a.run(ctx); err != nil {
		logger.Printf("auditor exited: %v", err)
	}

	shutdownCtx, shutdownCancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer shutdownCancel()
	_ = srv.Shutdown(shutdownCtx)
	logger.Print("exit")
}

func waitForDB(ctx context.Context, db *sql.DB, max time.Duration) error {
	deadline := time.Now().Add(max)
	var lastErr error
	for time.Now().Before(deadline) {
		if err := db.PingContext(ctx); err == nil {
			return nil
		} else {
			lastErr = err
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-time.After(1 * time.Second):
		}
	}
	if lastErr == nil {
		lastErr = errors.New("db unreachable (no error)")
	}
	return lastErr
}
