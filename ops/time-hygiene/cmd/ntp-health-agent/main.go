package main

import (
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"log/slog"
	"math"
	mrand "math/rand/v2"
	"net"
	"net/http"
	"os"
	"os/signal"
	"sync"
	"syscall"
	"time"

	"github.com/usareur-af/goatcore/ops/time-hygiene/internal/attest"
	internalchrony "github.com/usareur-af/goatcore/ops/time-hygiene/internal/chrony"
	"github.com/usareur-af/goatcore/ops/time-hygiene/internal/otelinit"
	"github.com/usareur-af/goatcore/ops/time-hygiene/internal/safego"
	"github.com/usareur-af/goatcore/ops/time-hygiene/internal/signer"
	"github.com/usareur-af/goatcore/ops/time-hygiene/internal/state"
	"github.com/prometheus/client_golang/prometheus"
	"github.com/prometheus/client_golang/prometheus/promauto"
	"go.opentelemetry.io/otel/attribute"
)

const serviceName = "ntp-health-agent"

var (
	flagChronySocket = flag.String("chrony-socket", "/run/chrony/chronyd.sock", "chronyd control socket path")
	flagUnixSocket   = flag.String("unix-socket", "/run/ntp-health.sock", "Unix socket path to expose NTP state")
	flagHTTPAddr     = flag.String("http-addr", ":9753", "HTTP listen address for /health and /metrics")
	flagPollInterval = flag.Duration("poll-interval", 10*time.Second, "how often to query chronyd (±10% jitter applied — 50B.7)")
	flagHostID       = flag.String("host-id", "", "host identifier in state JSON (default: OS hostname)")
	flagAttestKey    = flag.String("attestation-key", "", "path to the Ed25519 attestation private key (raw-seed PEM). Empty = serve verdicts UNATTESTED (attested=false). Block 12D.")
	flagAttestValid  = flag.Duration("attestation-validity", 5*time.Minute, "signed validity window applied to each verdict attestation")
)

// 50B.7 — chrony pollLoop metrics. Surfaces the consecutive-error backoff
// state and the per-query latency distribution so an operator can detect a
// stuck chronyd before it cascades into ntp_class=unknown across the fleet.
var (
	chronyConsecutiveErrors = promauto.NewGauge(prometheus.GaugeOpts{
		Name: "ntp_health_chrony_consecutive_errors",
		Help: "Number of consecutive chrony.GetTracking failures (resets on success). Drives exponential back-off in pollLoop.",
	})
	chronyQueryDuration = promauto.NewHistogram(prometheus.HistogramOpts{
		Name:    "ntp_health_chrony_query_duration_seconds",
		Help:    "Latency of chrony.GetTracking queries.",
		Buckets: prometheus.DefBuckets,
	})
)

func main() {
	flag.Parse()

	// Tag every safego panic-recovery counter with this service name.
	safego.SetService(serviceName)

	hostID := *flagHostID
	if hostID == "" {
		h, err := os.Hostname()
		if err != nil {
			slog.Error("hostname lookup failed", "err", err)
			os.Exit(1)
		}
		hostID = h
	}

	// Block 27C / 50B.4 — OTLP trace producer. Best-effort: init failure is
	// logged to stderr but never blocks daemon startup.
	otelShutdown, err := otelinit.Init(context.Background(), otelinit.Opts{
		ServiceName:    serviceName,
		ServiceVersion: version,
		Extra: []attribute.KeyValue{
			attribute.String("dbb.host_id", hostID),
		},
	})
	if err != nil {
		slog.Error("otel init failed; continuing without telemetry", "err", err)
	}

	// Attestation signer (Block 12D / increment 2a). An explicit --attestation-key
	// that fails to load is a fatal misconfig — refuse to run rather than
	// silently serve unattested under a key the operator believes is active.
	// No key at all is a supported posture: serve honestly unattested.
	var sgn *signer.Signer
	if *flagAttestKey != "" {
		sgn, err = signer.Load(*flagAttestKey)
		if err != nil {
			slog.Error("attestation key load failed; refusing to run unattested under an expected key", "path", *flagAttestKey, "err", err)
			os.Exit(1)
		}
		slog.Info("attestation signing enabled", "alg", attest.Alg, "validity", flagAttestValid.String())
	} else {
		slog.Warn("no --attestation-key set; serving verdicts UNATTESTED (attested=false)")
	}

	conn := internalchrony.New(*flagChronySocket)
	agent := &agent{
		hostID:       hostID,
		conn:         conn,
		state:        zeroState(hostID),
		signer:       sgn,
		validity:     *flagAttestValid,
		pollInterval: *flagPollInterval,
	}

	ctx, cancel := signal.NotifyContext(context.Background(), syscall.SIGTERM, syscall.SIGINT)
	defer cancel()

	// Bound the OTel exporter shutdown so a slow / unreachable collector
	// cannot stall the daemon's SIGTERM cleanup. Skip if init failed (nil).
	if otelShutdown != nil {
		defer func() {
			shutCtx, shutCancel := context.WithTimeout(context.Background(), 5*time.Second)
			defer shutCancel()
			if err := otelShutdown(shutCtx); err != nil {
				slog.Warn("otel shutdown returned error", "err", err)
			}
		}()
	}

	var wg sync.WaitGroup

	wg.Add(1)
	safego.Spawn("poll-loop", func() {
		defer wg.Done()
		agent.pollLoop(ctx, *flagPollInterval)
	})

	wg.Add(1)
	safego.Spawn("unix-server", func() {
		defer wg.Done()
		if err := agent.serveUnix(ctx, *flagUnixSocket); err != nil {
			slog.Error("unix socket server error", "err", err)
		}
	})

	wg.Add(1)
	safego.Spawn("http-server", func() {
		defer wg.Done()
		if err := agent.serveHTTP(ctx, *flagHTTPAddr); err != nil {
			slog.Error("http server error", "err", err)
		}
	})

	slog.Info("ntp-health-agent started",
		"host_id", hostID,
		"chrony_socket", *flagChronySocket,
		"unix_socket", *flagUnixSocket,
		"http_addr", *flagHTTPAddr,
		"poll_interval", flagPollInterval.String(),
	)

	wg.Wait()
	slog.Info("ntp-health-agent stopped")
}

type agent struct {
	hostID       string
	conn         *internalchrony.Conn
	signer       *signer.Signer // nil = serve unattested
	validity     time.Duration
	pollInterval time.Duration
	mu           sync.RWMutex
	state        state.NtpState
}

// minStaleAfter is the floor on how old a verdict may be before it is served
// as unknown, so a very short --poll-interval does not flap the verdict.
const minStaleAfter = 30 * time.Second

// staleAfter is how long after its last_updated_utc a verdict is still served
// as measured: three poll intervals (a healthy loop refreshes every interval
// ±10%; a failing one already serves unknown), at least minStaleAfter.
func (a *agent) staleAfter() time.Duration {
	d := 3 * a.pollInterval
	if d < minStaleAfter {
		d = minStaleAfter
	}
	return d
}

// servedState is the verdict every reader gets (Unix socket, /health,
// /metrics). When the stored verdict is past its valid_until, or older than
// staleAfter (the poll loop has stopped refreshing it: it returned after a
// recovered panic, or chronyd hangs), it is served as unknown, re-attested,
// with last_updated_utc left at the last real poll so readers can see its age.
func (a *agent) servedState() state.NtpState {
	s := a.currentState()
	if s.LastUpdatedUTC.IsZero() || s.NtpClass == state.ClassUnknown {
		return s
	}
	now := time.Now()
	expired := !s.ValidUntil.IsZero() && now.After(s.ValidUntil)
	if !expired && now.Sub(s.LastUpdatedUTC) <= a.staleAfter() {
		return s
	}
	s.NtpClass = state.ClassUnknown
	s.NtpClassStr = state.ClassUnknown.String()
	if err := attest.Sign(&s, a.signer, a.validity); err != nil {
		slog.Warn("attestation sign failed; serving stale verdict unsigned", "err", err)
		s.Attested, s.Signature, s.AttestationAlg, s.AttestationPubkey = false, "", "", ""
	}
	return s
}

func (a *agent) currentState() state.NtpState {
	a.mu.RLock()
	defer a.mu.RUnlock()
	return a.state
}

// store attests s (signing it when a key is configured, else marking it
// unattested) and swaps it in as the current state. Signing is done outside
// the lock — pollLoop is the only writer, so the build-then-swap is race-free.
func (a *agent) store(s state.NtpState) {
	if err := attest.Sign(&s, a.signer, a.validity); err != nil {
		slog.Warn("attestation sign failed; serving verdict unsigned", "err", err)
	}
	a.mu.Lock()
	a.state = s
	a.mu.Unlock()
}

// pollLoop queries chronyd at the configured interval, applying ±10% jitter
// per cycle (50B.7) so correlated chrony failures across a fleet do not
// thunder against `/run/chrony/chronyd.sock`. On consecutive failures, the
// effective sleep doubles up to 8× to back off from a clearly-stuck chronyd;
// it resets on success.
//
// Implemented as a sleep-loop rather than time.NewTicker because Ticker fires
// at fixed intervals — when we want jitter and per-cycle back-off, sleep is
// the cleaner shape.
func (a *agent) pollLoop(ctx context.Context, interval time.Duration) {
	consecutiveErrors := 0
	// Poll immediately on start so /health is meaningful before the first
	// jittered sleep elapses.
	if err := a.pollAndTrace(ctx, consecutiveErrors); err != nil {
		consecutiveErrors++
		chronyConsecutiveErrors.Set(float64(consecutiveErrors))
	} else {
		chronyConsecutiveErrors.Set(0)
	}

	for {
		sleep := jitteredInterval(interval)
		// Exponential back-off: double the per-cycle sleep on each consecutive
		// error, capped at 8× the configured interval. Reset is implicit (next
		// successful poll sets consecutiveErrors=0).
		if consecutiveErrors > 0 {
			multiplier := 1 << minInt(consecutiveErrors, 3) // 1, 2, 4, 8
			sleep = time.Duration(int64(sleep) * int64(multiplier))
		}

		select {
		case <-ctx.Done():
			return
		case <-time.After(sleep):
		}

		if err := a.pollAndTrace(ctx, consecutiveErrors); err != nil {
			consecutiveErrors++
		} else {
			consecutiveErrors = 0
		}
		chronyConsecutiveErrors.Set(float64(consecutiveErrors))
	}
}

// pollAndTrace runs one poll cycle wrapped in a root OTel span (Block 27C /
// 50B.4). The span's status + dbb.* attributes summarize the cycle outcome;
// per-cycle metrics + state mutation still live in poll() so the existing
// telemetry surface is untouched.
//
// consecutiveErrorsBefore is the consecutive-error count BEFORE this cycle —
// recorded as a span attribute so a query like
// `dbb.consecutive_errors > 3 AND dbb.poll_outcome=chrony_query_failed` flags
// the genuinely-stuck case.
func (a *agent) pollAndTrace(ctx context.Context, consecutiveErrorsBefore int) error {
	_, span := startPollSpan(ctx)
	defer span.End()

	err := a.poll()
	// Snapshot post-poll state for the span. poll() already took the write
	// lock; the read here is bounded.
	s := a.currentState()
	recordPollSpan(span, a.hostID, s, consecutiveErrorsBefore, err)
	return err
}

// jitteredInterval returns base ± up to 10% jitter (50B.7). Uses math/rand/v2
// (no manual seeding required — it self-seeds from the runtime).
//
// Defends against thundering-herd: if every site polls on a fixed 10s
// schedule and chronyd takes 200ms, every site stalls every cycle in lock-
// step. Spreading by ±1s decorrelates the sites enough to keep tail latency
// bounded.
func jitteredInterval(base time.Duration) time.Duration {
	if base <= 0 {
		return base
	}
	maxJitter := int64(base) / 10
	if maxJitter <= 0 {
		return base
	}
	// Range: [-maxJitter, +maxJitter)
	delta := mrand.Int64N(2*maxJitter) - maxJitter //nolint:gosec // non-security jitter; G404 already excluded
	return time.Duration(int64(base) + delta)
}

func minInt(a, b int) int {
	if a < b {
		return a
	}
	return b
}

// poll runs one chrony GetTracking query and updates a.state. Returns an
// error so the caller (pollLoop) can drive consecutive-error back-off.
func (a *agent) poll() error {
	start := time.Now()
	tracking, err := a.conn.GetTracking()
	chronyQueryDuration.Observe(time.Since(start).Seconds())
	if err != nil {
		slog.Warn("chrony tracking query failed", "err", err)
		// Degrade to unknown while preserving the last-known source/stratum
		// context, and re-attest the degraded verdict so consumers see a fresh
		// (signed) unknown rather than a stale healthy one.
		s := a.currentState()
		s.NtpClass = state.ClassUnknown
		s.NtpClassStr = state.ClassUnknown.String()
		s.LastUpdatedUTC = time.Now().UTC()
		a.store(s)
		return err
	}

	refID := internalchrony.RefidToString(tracking.RefID)
	class := state.Classify(tracking.Stratum, tracking.LeapStatus, math.Abs(tracking.RMSOffset))
	src := state.ClassifySource(refID, tracking.Stratum)

	s := state.NtpState{
		HostID:         a.hostID,
		NtpClass:       class,
		NtpClassStr:    class.String(),
		SelectedSource: src,
		Stratum:        tracking.Stratum,
		RefID:          refID,
		SystemOffsetNs: int64(tracking.LastOffset * 1e9),
		RmsOffsetNs:    int64(tracking.RMSOffset * 1e9),
		FreqPPM:        tracking.FreqPPM,
		LeapStatus:     tracking.LeapStatus,
		LastUpdatedUTC: time.Now().UTC(),
	}

	a.store(s)

	slog.Debug("poll",
		"class", class.String(),
		"stratum", tracking.Stratum,
		"ref_id", refID,
		"rms_offset_ns", s.RmsOffsetNs,
	)
	return nil
}

// serveUnix listens on a Unix stream socket and writes the current state JSON
// to each connecting client, then closes the connection.
func (a *agent) serveUnix(ctx context.Context, path string) error {
	_ = os.Remove(path)
	// Startup-time one-shot listen (Block 50A.3 noctx): the listener
	// lifecycle is bound to the agent process via the goroutine below that
	// closes l on ctx.Done(); plain net.Listen is appropriate here.
	l, err := net.Listen("unix", path) //nolint:noctx // startup-only; ctx cleanup below
	if err != nil {
		return fmt.Errorf("listen unix %s: %w", path, err)
	}
	if err := os.Chmod(path, 0660); err != nil {
		slog.Warn("chmod unix socket failed", "err", err)
	}
	safego.Spawn("unix-listener-closer", func() {
		<-ctx.Done()
		l.Close()
	})
	for {
		conn, err := l.Accept()
		if err != nil {
			select {
			case <-ctx.Done():
				return nil
			default:
				return fmt.Errorf("accept: %w", err)
			}
		}
		safego.Spawn("unix-handle", func() { a.handleUnix(conn) })
	}
}

func (a *agent) handleUnix(conn net.Conn) {
	defer conn.Close()
	s := a.servedState()
	if err := json.NewEncoder(conn).Encode(s); err != nil {
		slog.Warn("unix socket write error", "err", err)
	}
}

// serveHTTP exposes /health (JSON state) and /metrics (Prometheus text format).
func (a *agent) serveHTTP(ctx context.Context, addr string) error {
	mux := http.NewServeMux()
	mux.HandleFunc("/health", a.healthHandler)
	mux.HandleFunc("/metrics", func(w http.ResponseWriter, r *http.Request) {
		s := a.servedState()
		w.Header().Set("Content-Type", "text/plain; version=0.0.4")
		fmt.Fprintf(w, "# HELP ntp_health_class NTP health class (0=healthy 1=warning 2=degraded 3=unknown)\n")
		fmt.Fprintf(w, "# TYPE ntp_health_class gauge\n")
		fmt.Fprintf(w, "ntp_health_class{host_id=%q,source=%q} %d\n", s.HostID, string(s.SelectedSource), s.NtpClass)

		fmt.Fprintf(w, "# HELP ntp_stratum Chrony reported stratum\n")
		fmt.Fprintf(w, "# TYPE ntp_stratum gauge\n")
		fmt.Fprintf(w, "ntp_stratum{host_id=%q} %d\n", s.HostID, s.Stratum)

		fmt.Fprintf(w, "# HELP ntp_rms_offset_seconds Chrony RMS offset (seconds)\n")
		fmt.Fprintf(w, "# TYPE ntp_rms_offset_seconds gauge\n")
		fmt.Fprintf(w, "ntp_rms_offset_seconds{host_id=%q} %g\n", s.HostID, float64(s.RmsOffsetNs)/1e9)

		fmt.Fprintf(w, "# HELP ntp_freq_ppm Chrony frequency error (ppm)\n")
		fmt.Fprintf(w, "# TYPE ntp_freq_ppm gauge\n")
		fmt.Fprintf(w, "ntp_freq_ppm{host_id=%q} %g\n", s.HostID, s.FreqPPM)

		fmt.Fprintf(w, "# HELP ntp_leap_status Chrony leap status (0=OK 1=insertLeap 2=deleteLeap 3=unsync)\n")
		fmt.Fprintf(w, "# TYPE ntp_leap_status gauge\n")
		fmt.Fprintf(w, "ntp_leap_status{host_id=%q} %d\n", s.HostID, s.LeapStatus)

		attested := 0
		if s.Attested {
			attested = 1
		}
		fmt.Fprintf(w, "# HELP ntp_health_attested Whether the current verdict carries a signed attestation (1) or is served unattested (0)\n")
		fmt.Fprintf(w, "# TYPE ntp_health_attested gauge\n")
		fmt.Fprintf(w, "ntp_health_attested{host_id=%q} %d\n", s.HostID, attested)
	})

	// Timeouts (Block 50A.1): plain HTTP/1.1 short-request listener (/health,
	// /metrics). Slowloris protection + bounded read/write so a stuck peer can't
	// hold a goroutine indefinitely.
	srv := &http.Server{
		Addr:              addr,
		Handler:           mux,
		ReadHeaderTimeout: 5 * time.Second,
		ReadTimeout:       10 * time.Second,
		WriteTimeout:      10 * time.Second,
		IdleTimeout:       60 * time.Second,
	}
	safego.Spawn("http-shutdown-watcher", func() {
		<-ctx.Done()
		// Fresh ctx for shutdown (intentional contextcheck violation): parent
		// ctx is already canceled by SIGTERM at this point, but Shutdown needs
		// a live ctx to drive the in-flight drain. No deadline = wait until all
		// /health and /metrics handlers complete; they are bounded by the
		// per-listener WriteTimeout above.
		_ = srv.Shutdown(context.Background()) //nolint:contextcheck // intentional shutdown decoupling
	})
	if err := srv.ListenAndServe(); err != nil && err != http.ErrServerClosed {
		return err
	}
	return nil
}

// healthHandler answers /health: the served verdict as JSON, 503 unless healthy.
func (a *agent) healthHandler(w http.ResponseWriter, _ *http.Request) {
	s := a.servedState()
	w.Header().Set("Content-Type", "application/json")
	if s.NtpClass != state.ClassHealthy {
		w.WriteHeader(http.StatusServiceUnavailable)
	}
	//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
	_ = json.NewEncoder(w).Encode(s)
}

func zeroState(hostID string) state.NtpState {
	return state.NtpState{
		HostID:         hostID,
		NtpClass:       state.ClassUnknown,
		NtpClassStr:    state.ClassUnknown.String(),
		SelectedSource: state.SourceNone,
	}
}
