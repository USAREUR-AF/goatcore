// Package server implements the client-facing SignalExchange service and the
// server-to-server SignalFederation service. SignalExchange lives in
// exchange.go; SignalFederation in federation.go.
package server

import (
	"log/slog"
	"time"

	"github.com/prometheus/client_golang/prometheus"

	federationpb "github.com/usareur-af/goatcore/signal-server/gen/federation"
	signalpb "github.com/usareur-af/goatcore/signal-server/gen/signalexchange"
)

type Server struct {
	signalpb.UnimplementedSignalExchangeServer
	federationpb.UnimplementedSignalFederationServer

	Node       string
	Logger     *slog.Logger
	registry   *registry
	metrics    *metrics
	federation *federationClient
	limiter    *concurrencyLimiter

	// ADR 1081 L2 — control-plane storm shedding. Both default OFF (their
	// zero-value config disables them); enabled per-node via the
	// SIGNAL_SENDER_RATE_LIMIT / SIGNAL_NEGATIVE_CACHE_TTL env vars.
	rateLimiter *senderRateLimiter
	negCache    *negativeCache

	// Phase 3 federation transport security (fedauth.go). nil = auth off.
	fedAuth *federationAuth

	// ADR 1108 presence-sync (push model). Never nil after New(); disabled
	// (enabled=false) unless Config.PresenceSyncEnabled, in which case a pusher
	// goroutine runs until Close(). Off = pure broadcast (pre-1108 behavior).
	presence *presenceState
}

// DefaultMaxStreams is the default concurrent ConnectStream admission cap when
// Config.MaxInflight > 0 and Config.MaxStreams is left at 0.
const DefaultMaxStreams int64 = 8192

// Config carries optional construction parameters for New. Defaults applied
// in New() when fields are zero.
type Config struct {
	// MaxInflight caps concurrent unary gRPC handler invocations (Send,
	// Forward, PresenceUpdate). <=0 means unlimited (intended for tests).
	// Production default resolved by the caller (cmd/signal-server passes
	// the --max-inflight flag, default 1000).
	MaxInflight int64

	// MaxStreams caps concurrent streaming gRPC handler invocations
	// (ConnectStream). Zero falls back to DefaultMaxStreams when
	// MaxInflight > 0, or unlimited when MaxInflight <= 0; <0 means
	// unlimited even when MaxInflight > 0. Production default resolved by
	// cmd/signal-server (--max-streams, default 8192).
	MaxStreams int64

	// CircuitConsecutiveFailures is the number of consecutive Forward
	// failures to a single peer that trips its circuit open. Zero falls
	// back to the package default.
	//
	// (The per-Forward deadline is NOT here: it lives as
	// federationForwardTimeout = 200ms hardcoded in federation.go per
	// the 52A.4 RTT analysis. Don't reintroduce a flag without
	// updating that design doc first.)
	CircuitConsecutiveFailures uint32

	// CircuitOpenDuration is how long the circuit stays open before the
	// next probe attempt is allowed (half-open). Zero falls back to the
	// package default.
	CircuitOpenDuration time.Duration

	// SenderRateLimitPerMin caps offers/min per sender peer key on the unary
	// Send path (ADR 1081 L2, storm shedding). <=0 disables the limiter
	// entirely — that is the default, so the mechanism is off until an
	// operator sets SIGNAL_SENDER_RATE_LIMIT.
	SenderRateLimitPerMin int

	// SenderRateLimitBurst is the token-bucket burst capacity. <=0 falls back
	// to 2× SenderRateLimitPerMin when the limiter is enabled (rate 30 →
	// burst 60). Only meaningful when SenderRateLimitPerMin > 0.
	SenderRateLimitBurst int

	// NegativeCacheTTL caches recipient-offline so repeated misses to an
	// absent peer are shed cheaply before the federation fan-out (ADR 1081
	// L2). <=0 disables the cache entirely — the default, off until an
	// operator sets SIGNAL_NEGATIVE_CACHE_TTL.
	NegativeCacheTTL time.Duration

	// FederationToken is the shared bearer token for the federation
	// surface (fedauth.go; mesh-control-path Phase 3). When set, this
	// server rejects unauthenticated federation RPCs AND attaches the
	// token to its own outbound Forwards. May be a comma-separated
	// accept-list for rotation (server accepts all, client sends the
	// first). Empty = auth off (pre-Phase-3 mesh-only posture;
	// cmd/signal-server warns when peers are configured without a token).
	FederationToken string

	// FederationAuthority overrides the HTTP/2 :authority on outbound
	// federation dials (Phase 3 dual-path proxy — the peer's Traefik
	// federation router Host-matches this). Empty = gRPC default (the
	// dial target).
	FederationAuthority string

	// FederationForwardTimeout overrides the per-peer Forward deadline.
	// Zero = the 200 ms package default (52A.4 mesh-RTT analysis).
	// Dual-path goatnets set ~1 s to cover the public leg's RTT.
	FederationForwardTimeout time.Duration

	// PresenceSyncEnabled turns on ADR 1108 presence-sync: push local presence
	// to federation peers and forward cross-shard directly to the known owner
	// (broadcast fallback for unknowns). Default off = pure broadcast.
	PresenceSyncEnabled bool

	// PresenceSyncInterval is the full-snapshot push cadence. Zero = 2 s.
	PresenceSyncInterval time.Duration
}

// New constructs a Server, registers its Prometheus collectors with reg,
// and dials out to any configured federation peers.
//
// federationPeers is the list of peer signal-server gRPC addresses
// (host:port). Empty or nil means "standalone server, no federation" —
// local misses return codes.NotFound to the client rather than being
// fanned out.
//
// Backwards-compatible signature: cfg may be nil for callers that want
// defaults (tests, peripheral binaries). New always wires:
//   - the panic counter (used by safeGo and the panic-recovery interceptors)
//   - the concurrency limiter (uncapped if cfg.MaxInflight <= 0)
//   - the federation client with circuit-breaker + per-call timeout
func New(node string, logger *slog.Logger, reg prometheus.Registerer, federationPeers []string, cfg *Config) (*Server, error) {
	if cfg == nil {
		cfg = &Config{}
	}
	r := newRegistry()
	m := newMetrics(reg, node, r)
	registerPanicCounter(reg, node)

	maxStreams := cfg.MaxStreams
	if maxStreams == 0 && cfg.MaxInflight > 0 {
		maxStreams = DefaultMaxStreams
	}
	limiter := newSplitConcurrencyLimiter(reg, node, cfg.MaxInflight, maxStreams)

	fcCfg := federationClientConfig{
		CircuitConsecutiveFailures: cfg.CircuitConsecutiveFailures,
		CircuitOpenDuration:        cfg.CircuitOpenDuration,
		FederationToken:            cfg.FederationToken,
		FederationAuthority:        cfg.FederationAuthority,
		ForwardTimeout:             cfg.FederationForwardTimeout,
	}
	fc, err := newFederationClient(federationPeers, m, reg, node, fcCfg)
	if err != nil {
		return nil, err
	}

	rateLimiter := newSenderRateLimiter(reg, node, cfg.SenderRateLimitPerMin, cfg.SenderRateLimitBurst)
	negCache := newNegativeCache(cfg.NegativeCacheTTL)

	srv := &Server{
		Node:        node,
		Logger:      logger,
		registry:    r,
		metrics:     m,
		federation:  fc,
		limiter:     limiter,
		rateLimiter: rateLimiter,
		negCache:    negCache,
		fedAuth:     newFederationAuth(reg, node, cfg.FederationToken),
	}
	// ADR 1108 presence-sync. Always constructed (nil-safe methods); the pusher
	// only runs when enabled AND federation peers exist.
	srv.presence = newPresenceState(cfg.PresenceSyncEnabled, node, cfg.PresenceSyncInterval, fc, r, m, logger)
	safeGo("presence-sync", srv.presence.run)
	return srv, nil
}

// Close tears down outbound federation connections. Safe to call on a
// server with no federation configured.
func (s *Server) Close() error {
	s.presence.Stop()
	return s.federation.Close()
}

// RegisteredPeers returns the current count of registered peers.
func (s *Server) RegisteredPeers() int {
	return s.registry.count()
}
