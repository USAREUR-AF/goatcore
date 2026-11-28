// interceptors.go — Block 50B.1 + 50B.2 production-readiness hardening.
//
// Two gRPC interceptors live here:
//
//   - Bounded-concurrency interceptor (50B.1). gRPC by default spawns one
//     goroutine per stream/unary call with no upper bound. A burst — or a
//     slow downstream — can drive the process into unbounded goroutine
//     and memory growth, escalating into FD exhaustion or oom-kill long
//     before anything user-visible signals the load. The semaphore-based
//     interceptor admits up to N concurrent in-flight handlers and rejects
//     the (N+1)th call with codes.ResourceExhausted, giving the caller a
//     meaningful retry signal and the operator a Prometheus surface
//     (signal_server_handler_rejected_total{reason="saturation"}) to alert on.
//
//   - Panic-recovery interceptor (50B.2). gRPC's default is to let a
//     handler panic crash the process. This is wrong for a control-plane
//     service: a single bad message shouldn't take the whole signal server
//     down for every other peer. The recovery interceptor returns
//     codes.Internal to the caller, increments panic_total{kind="grpc-handler"},
//     and lets the server keep handling traffic.
//
// Both interceptors are wired at New(), exposed via the Interceptors() method
// on Server so main.go can pass them to grpc.NewServer(grpc.ChainUnaryInterceptor,
// grpc.ChainStreamInterceptor).

package server

import (
	"context"
	"log/slog"
	"runtime/debug"
	"time"

	"github.com/prometheus/client_golang/prometheus"
	"golang.org/x/sync/semaphore"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/keepalive"
	"google.golang.org/grpc/status"
)

// concurrencyLimiter bounds concurrent unary and streaming gRPC handlers with
// independent weighted semaphores so long-lived ConnectStream sessions cannot
// exhaust the unary RPC admission budget. Emits gauges + rejection counters.
type concurrencyLimiter struct {
	unarySem  *semaphore.Weighted
	streamSem *semaphore.Weighted
	inflight  *prometheus.GaugeVec
	rejected  *prometheus.CounterVec
}

// newConcurrencyLimiter creates a limiter with identical unary and stream caps.
// Retained for unit tests that exercise symmetric saturation on a single cap.
func newConcurrencyLimiter(reg prometheus.Registerer, node string, maxInflight int64) *concurrencyLimiter {
	return newSplitConcurrencyLimiter(reg, node, maxInflight, maxInflight)
}

// newSplitConcurrencyLimiter creates a limiter with independent unary
// (maxUnary) and streaming (maxStreams) caps. Registers inflight gauge and
// rejection counter on reg. Either cap <= 0 means "unlimited" for that kind.
func newSplitConcurrencyLimiter(reg prometheus.Registerer, node string, maxUnary, maxStreams int64) *concurrencyLimiter {
	inflight := prometheus.NewGaugeVec(
		prometheus.GaugeOpts{
			Name:        "signal_server_inflight_handlers",
			Help:        "Number of gRPC handlers currently in flight, labeled by kind (unary or stream).",
			ConstLabels: prometheus.Labels{"node": node},
		},
		[]string{"kind"},
	)
	reg.MustRegister(inflight)
	// Pre-create both label series so they exist at zero even before traffic.
	inflight.WithLabelValues("unary")
	inflight.WithLabelValues("stream")

	rejected := prometheus.NewCounterVec(
		prometheus.CounterOpts{
			Name:        "signal_server_handler_rejected_total",
			Help:        "Total handler invocations rejected at admission, labeled by reason.",
			ConstLabels: prometheus.Labels{"node": node},
		},
		[]string{"reason"},
	)
	reg.MustRegister(rejected)
	rejected.WithLabelValues("saturation")

	cl := &concurrencyLimiter{
		inflight: inflight,
		rejected: rejected,
	}
	if maxUnary > 0 {
		cl.unarySem = semaphore.NewWeighted(maxUnary)
	}
	if maxStreams > 0 {
		cl.streamSem = semaphore.NewWeighted(maxStreams)
	}
	return cl
}

// tryAcquire attempts non-blocking acquisition of one semaphore slot.
// Returns true on success, false on saturation (caller should reject).
// nil semaphore = unbounded mode = always true.
func tryAcquire(sem *semaphore.Weighted) bool {
	if sem == nil {
		return true
	}
	return sem.TryAcquire(1)
}

// release returns one slot to sem. nil semaphore = no-op.
func release(sem *semaphore.Weighted) {
	if sem == nil {
		return
	}
	sem.Release(1)
}

// unaryConcurrencyInterceptor enforces the unary inflight cap for unary RPCs.
// On admission failure it returns codes.ResourceExhausted with a stable
// message; on admission success it tracks the inflight gauge and
// guarantees release on handler return (including panic — defer ordering
// matters; release fires before the panic-recovery defer).
func (cl *concurrencyLimiter) unaryInterceptor(
	ctx context.Context,
	req interface{},
	_ *grpc.UnaryServerInfo,
	handler grpc.UnaryHandler,
) (interface{}, error) {
	if !tryAcquire(cl.unarySem) {
		cl.rejected.WithLabelValues("saturation").Inc()
		return nil, status.Error(codes.ResourceExhausted, "signal server at concurrency limit")
	}
	defer release(cl.unarySem)
	cl.inflight.WithLabelValues("unary").Inc()
	defer cl.inflight.WithLabelValues("unary").Dec()
	return handler(ctx, req)
}

// streamConcurrencyInterceptor enforces the stream inflight cap for streaming
// RPCs independently of unary RPCs. Streams are long-lived (ConnectStream
// often runs for hours), so maxStreams acts as a per-server peer-connection
// cap without starving unary Send / Forward / PresenceUpdate RPCs.
func (cl *concurrencyLimiter) streamInterceptor(
	srv interface{},
	ss grpc.ServerStream,
	_ *grpc.StreamServerInfo,
	handler grpc.StreamHandler,
) error {
	if !tryAcquire(cl.streamSem) {
		cl.rejected.WithLabelValues("saturation").Inc()
		return status.Error(codes.ResourceExhausted, "signal server at concurrency limit")
	}
	defer release(cl.streamSem)
	cl.inflight.WithLabelValues("stream").Inc()
	defer cl.inflight.WithLabelValues("stream").Dec()
	return handler(srv, ss)
}

// unaryPanicRecoveryInterceptor recovers panics escaping a unary handler,
// logs the stack, increments panic_total{kind="grpc-handler"}, and returns
// codes.Internal to the caller. Without this, a panic in any handler tears
// down the whole gRPC server's serve loop.
func unaryPanicRecoveryInterceptor(
	ctx context.Context,
	req interface{},
	info *grpc.UnaryServerInfo,
	handler grpc.UnaryHandler,
) (resp interface{}, err error) {
	defer func() {
		if r := recover(); r != nil {
			stack := debug.Stack()
			slog.Error("recovered panic in gRPC unary handler",
				"method", info.FullMethod,
				"panic", r,
				"stack", string(stack),
			)
			if pc := loadPanicCounter(); pc != nil {
				pc.WithLabelValues("grpc-handler", "grpc-handler").Inc()
			}
			err = status.Error(codes.Internal, "internal server error")
			resp = nil
		}
	}()
	return handler(ctx, req)
}

// streamPanicRecoveryInterceptor recovers panics escaping a stream handler.
// Same posture as the unary version: log, count, return Internal.
func streamPanicRecoveryInterceptor(
	srv interface{},
	ss grpc.ServerStream,
	info *grpc.StreamServerInfo,
	handler grpc.StreamHandler,
) (err error) {
	defer func() {
		if r := recover(); r != nil {
			stack := debug.Stack()
			slog.Error("recovered panic in gRPC stream handler",
				"method", info.FullMethod,
				"panic", r,
				"stack", string(stack),
			)
			if pc := loadPanicCounter(); pc != nil {
				pc.WithLabelValues("grpc-handler", "grpc-handler").Inc()
			}
			err = status.Error(codes.Internal, "internal server error")
		}
	}()
	return handler(srv, ss)
}

// ServerOptions returns gRPC server options that wire all production
// interceptors (panic recovery + federation auth + concurrency cap) for
// both unary and stream RPCs. Pass to grpc.NewServer in main.go.
//
// Interceptor order matters (outermost → innermost):
//   - panic-recovery FIRST: wraps everything below it, so a panic in any
//     interceptor or handler is contained, not fatal to the serve loop.
//   - federation-auth SECOND (fedauth.go, Phase 3): rejects an
//     unauthenticated federation call BEFORE it can take a concurrency
//     slot. This ordering is load-bearing for DDoS resistance — the
//     federation surface is public (relay-tunneled) since Phase 3, so an
//     unauthenticated flood must be cheap to reject. If the limiter ran
//     first, doomed calls would occupy MaxInflight admission slots
//     (shared with the peer-facing SignalExchange surface) until the
//     token check failed, and a public flood could starve legitimate
//     mesh clients into ResourceExhausted. Pass-through for
//     non-federation methods and when no token is configured, so
//     SignalExchange admission is unchanged.
//   - concurrency-limit THIRD: bounds everything that passed auth, so a
//     saturated server still sheds authenticated load at admission.
func (s *Server) ServerOptions() []grpc.ServerOption {
	return []grpc.ServerOption{
		// Permit the federation client's 30s keepalive pings (Phase 3 —
		// see the dial options in federation.go). MinTime 10s tolerates
		// clock skew below the client's interval; without-stream pings
		// are the whole point (the channel is idle between forwards).
		grpc.KeepaliveEnforcementPolicy(keepalive.EnforcementPolicy{
			MinTime:             10 * time.Second,
			PermitWithoutStream: true,
		}),
		// Actively probe client connections. Without this, gRPC's
		// defaults apply — a 2-HOUR ping interval and no max idle — so a
		// half-open transport (NAT rebind, silent middlebox drop, hard
		// client kill) keeps its registry entry alive for hours while
		// every sender is told delivery succeeded. Upstream netbird's
		// signal sets Time 5s / Timeout 2s / MaxConnectionIdle 15s; ours
		// inherited nothing when the server was rewritten. These values
		// are deliberately gentler than upstream's (a 95-peer hub does not
		// need 5-second probing) but bound detection to well under a
		// minute instead of two hours. MaxConnectionIdle is left unset:
		// peer streams are long-lived and legitimately idle between
		// offers, and the keepalive probe already proves liveness.
		grpc.KeepaliveParams(keepalive.ServerParameters{
			Time:    20 * time.Second,
			Timeout: 10 * time.Second,
		}),
		grpc.ChainUnaryInterceptor(
			unaryPanicRecoveryInterceptor,
			s.fedAuth.unaryInterceptor,
			s.limiter.unaryInterceptor,
		),
		grpc.ChainStreamInterceptor(
			streamPanicRecoveryInterceptor,
			s.fedAuth.streamInterceptor,
			s.limiter.streamInterceptor,
		),
	}
}
