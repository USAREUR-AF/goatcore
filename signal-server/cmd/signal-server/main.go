package main

import (
	"context"
	"errors"
	"flag"
	"log/slog"
	"net"
	"net/http"
	"os"
	"os/signal"
	"strconv"
	"strings"
	"syscall"
	"time"

	"github.com/prometheus/client_golang/prometheus"
	"github.com/prometheus/client_golang/prometheus/collectors"
	"github.com/prometheus/client_golang/prometheus/promhttp"
	"google.golang.org/grpc"

	federationpb "github.com/usareur-af/goatcore/signal-server/gen/federation"
	signalpb "github.com/usareur-af/goatcore/signal-server/gen/signalexchange"
	"github.com/usareur-af/goatcore/signal-server/internal/server"
)

var (
	grpcAddr        = flag.String("grpc-addr", ":10000", "gRPC listen address (serves both SignalExchange and SignalFederation)")
	metricsAddr     = flag.String("metrics-addr", ":9090", "HTTP listen address for /metrics (Prometheus)")
	nodeName        = flag.String("node", "unnamed", "signal-server node name; included in logs and metric labels")
	federationPeers = flag.String("federation-peers", "", "comma-separated list of peer signal-server addresses (host:port) for cross-site federation. Empty = standalone, no federation.")

	// Block 50B.1 — bounded concurrency on the gRPC accept loop. Unary RPCs
	// (Send, Forward, PresenceUpdate) and long-lived peer streams
	// (ConnectStream) use independent budgets so N connected peers cannot
	// starve unary signaling or liveness probes.
	maxInflight = flag.Int64("max-inflight", 1000, "maximum concurrent unary gRPC handlers (Send, Forward, PresenceUpdate); excess returns codes.ResourceExhausted")
	maxStreams  = flag.Int64("max-streams", server.DefaultMaxStreams, "maximum concurrent streaming gRPC handlers (ConnectStream); excess returns codes.ResourceExhausted")

	// Block 50B.6 — per-peer circuit breaker. The per-Forward deadline is
	// hardcoded at 200ms in federation.go per the 52A.4 RTT analysis (not
	// flag-configurable); only the breaker tunings are operator-tunable.
	circuitConsecutiveFailures = flag.Uint("federation-circuit-consecutive-failures", 5, "consecutive Forward failures to a peer that trip its breaker open")
	circuitOpenDuration        = flag.Duration("federation-circuit-open-duration", 30*time.Second, "how long the per-peer breaker stays open before allowing one half-open probe")

	presenceSync         = flag.Bool("presence-sync", false, "ADR 1108: push local presence to federation peers and forward cross-shard directly to the known owner (broadcast fallback). Default off = pure broadcast.")
	presenceSyncInterval = flag.Duration("presence-sync-interval", 2*time.Second, "presence full-snapshot push cadence (with --presence-sync)")
)

func main() {
	flag.Parse()

	logger := slog.New(slog.NewJSONHandler(os.Stderr, &slog.HandlerOptions{Level: slog.LevelInfo}))
	slog.SetDefault(logger)

	ctx, stop := signal.NotifyContext(context.Background(), syscall.SIGINT, syscall.SIGTERM)
	defer stop()

	// Dedicated registry — no global state, clean for multiple instances in
	// the same test process and easy to swap if we ever want to scope labels
	// differently.
	reg := prometheus.NewRegistry()
	reg.MustRegister(collectors.NewGoCollector())
	reg.MustRegister(collectors.NewProcessCollector(collectors.ProcessCollectorOpts{}))

	var peers []string
	if trimmed := strings.TrimSpace(*federationPeers); trimmed != "" {
		for _, p := range strings.Split(trimmed, ",") {
			if p = strings.TrimSpace(p); p != "" {
				peers = append(peers, p)
			}
		}
	}

	// ADR 1081 L2 — control-plane storm shedding. Both mechanisms are env-
	// gated and default OFF (unset/0 disables). See ratelimit.go / negcache.go
	// for the sizing rationale; the mgmt-host compose signal service documents
	// the recommended values (30 / 60 / 20) commented out.
	senderRateLimit := envInt("SIGNAL_SENDER_RATE_LIMIT")                           // offers/min per sender; 0/unset = off
	senderRateBurst := envInt("SIGNAL_SENDER_RATE_BURST")                           // burst; 0/unset = 2× rate when enabled
	negCacheTTL := time.Duration(envInt("SIGNAL_NEGATIVE_CACHE_TTL")) * time.Second // 0/unset = off

	// Phase 3 federation transport security (internal/server/fedauth.go).
	// Shared per-goatnet secret; same value on every federating server
	// (comma-separated accept-list during rotation). Empty = auth off —
	// acceptable only while federation rides the mesh exclusively, hence
	// the warning when peers are configured without it. The "enforced"
	// info line is load-bearing: deploy plays assert it to prove the
	// running image actually implements fedauth (an older binary
	// silently ignores the env var).
	federationToken := strings.TrimSpace(os.Getenv("SIGNAL_FEDERATION_TOKEN"))
	switch {
	case federationToken != "":
		logger.Info("federation auth enforced (bearer token, both transports)")
	case len(peers) > 0:
		logger.Warn("federation peers configured without SIGNAL_FEDERATION_TOKEN; " +
			"federation surface is UNAUTHENTICATED (mesh-only posture, pre-Phase-3)")
	}

	// Dual-path tunables (Phase 3): authority override for the peer's
	// Traefik Host-match, and a raised per-Forward deadline covering the
	// public leg's RTT (0/unset = the 200 ms mesh default).
	federationAuthority := strings.TrimSpace(os.Getenv("SIGNAL_FEDERATION_AUTHORITY"))
	federationForwardTimeout := time.Duration(envInt("SIGNAL_FEDERATION_FORWARD_TIMEOUT_MS")) * time.Millisecond

	srv, err := server.New(*nodeName, logger, reg, peers, &server.Config{
		MaxInflight:                *maxInflight,
		MaxStreams:                 *maxStreams,
		CircuitConsecutiveFailures: uint32(*circuitConsecutiveFailures), //nolint:gosec // operator-supplied flag, well below uint32 cap
		CircuitOpenDuration:        *circuitOpenDuration,
		SenderRateLimitPerMin:      senderRateLimit,
		SenderRateLimitBurst:       senderRateBurst,
		NegativeCacheTTL:           negCacheTTL,
		FederationToken:            federationToken,
		FederationAuthority:        federationAuthority,
		FederationForwardTimeout:   federationForwardTimeout,
		PresenceSyncEnabled:        *presenceSync,
		PresenceSyncInterval:       *presenceSyncInterval,
	})
	if err != nil {
		logger.Error("server setup", "err", err)
		os.Exit(1)
	}
	defer func() {
		if err := srv.Close(); err != nil {
			logger.Error("server close", "err", err)
		}
	}()

	// ServerOptions wires the bounded-concurrency interceptor (50B.1) and the
	// panic-recovery interceptors (50B.2) for both unary and stream RPCs.
	grpcSrv := grpc.NewServer(srv.ServerOptions()...)
	signalpb.RegisterSignalExchangeServer(grpcSrv, srv)
	federationpb.RegisterSignalFederationServer(grpcSrv, srv)

	// Startup-time one-shot listen (Block 50A.3 noctx): listener lifecycle is
	// bound to the gRPC server via grpcSrv.GracefulStop() on SIGTERM below.
	lis, err := net.Listen("tcp", *grpcAddr) //nolint:noctx // startup-only
	if err != nil {
		logger.Error("grpc listen", "addr", *grpcAddr, "err", err)
		os.Exit(1)
	}

	mux := http.NewServeMux()
	mux.Handle("/metrics", promhttp.HandlerFor(reg, promhttp.HandlerOpts{
		EnableOpenMetrics: true,
		Registry:          reg,
	}))
	mux.HandleFunc("/healthz", func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusOK)
		//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
		_, _ = w.Write([]byte("ok\n"))
	})
	metricsSrv := &http.Server{
		Addr:              *metricsAddr,
		Handler:           mux,
		ReadHeaderTimeout: 5 * time.Second,
	}

	// Block 50E.6 — server.SafeSpawn so a panic in either listener bubbles
	// up to the package's panic-recovery machinery (slog + signal_server_
	// panic_total metric) instead of killing the process. Without this,
	// a panic in the metrics path would also kill the grpc listener.
	server.SafeSpawn("grpc-listener", func() {
		logger.Info("grpc server listening", "addr", *grpcAddr, "node", *nodeName)
		if err := grpcSrv.Serve(lis); err != nil && !errors.Is(err, grpc.ErrServerStopped) {
			logger.Error("grpc serve", "err", err)
		}
	})
	server.SafeSpawn("metrics-listener", func() {
		logger.Info("metrics server listening", "addr", *metricsAddr)
		if err := metricsSrv.ListenAndServe(); err != nil && !errors.Is(err, http.ErrServerClosed) {
			logger.Error("metrics serve", "err", err)
		}
	})

	<-ctx.Done()
	logger.Info("shutdown signal received")

	grpcSrv.GracefulStop()
	shutCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	_ = metricsSrv.Shutdown(shutCtx)

	logger.Info("shutdown complete")
}

// envInt reads a non-negative integer from env var name. Unset, empty, or
// unparseable/negative all resolve to 0 — the "feature off" sentinel for the
// ADR 1081 L2 storm-shedding knobs. A malformed value is logged and treated
// as off rather than crashing the server on a typo.
func envInt(name string) int {
	raw := strings.TrimSpace(os.Getenv(name))
	if raw == "" {
		return 0
	}
	v, err := strconv.Atoi(raw)
	if err != nil || v < 0 {
		slog.Warn("ignoring malformed env var; treating as 0 (off)", "var", name, "value", raw)
		return 0
	}
	return v
}
