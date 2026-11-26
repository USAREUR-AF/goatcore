// Package safego provides panic-safe goroutine spawning for load-bearing
// services in ops/time-hygiene.
//
// Block 50B.2 — Panic-recovery middleware.
//
// Rationale: a panic in any unrecovered goroutine kills the entire process,
// taking down the health agent and with it the site's local clock verdict
// (the Unix socket and the published health surface). For services
// that already swallow per-iteration faults at the protocol level (chrony
// poll error → mark unknown, accept loop dial error → log + continue), an
// unrecovered panic is a strictly worse failure mode than a logged-and-
// continued panic.
//
// Spawn wraps `go func(){ ... }()` with `defer recover()`. On panic it logs a
// structured slog event with the panic value + full stack trace + the caller-
// supplied name, then increments the Prometheus counter
// `time_hygiene_panic_total{service=<service>,goroutine=<name>}` so an
// operator can alert on panic rate. The wrapped function does NOT re-panic;
// the goroutine returns normally and the surrounding loop / caller is
// expected to either re-spawn (poll loops) or accept the loss (per-request
// goroutines). For long-running drivers the caller can layer its own restart
// policy on top of safego.
//
// SetService MUST be called once at process start so the counter carries the
// right service label. If unset, the counter is labeled service="unknown",
// which still alerts but yields a less actionable signal.
package safego

import (
	"log/slog"
	"runtime/debug"
	"sync"

	"github.com/prometheus/client_golang/prometheus"
	"github.com/prometheus/client_golang/prometheus/promauto"
)

// PanicTotal is the Prometheus counter incremented every time a goroutine
// spawned via Spawn — or a request handled by Middleware — recovers from a
// panic. Labels:
//   - service:   "ntp-health-agent" (set via SetService)
//   - goroutine: descriptive name supplied at Spawn() / Middleware() call site
//
// Exposed for /metrics scraping; do not increment directly outside this
// package.
var PanicTotal = promauto.NewCounterVec(
	prometheus.CounterOpts{
		Name: "time_hygiene_panic_total",
		Help: "Goroutine and handler panics recovered by safego (service-scoped).",
	},
	[]string{"service", "goroutine"},
)

var (
	serviceMu sync.RWMutex
	service   = "unknown"
)

// SetService sets the `service` label applied to every PanicTotal increment.
// Call once from main() before spawning any safego.Spawn'ed goroutine.
func SetService(name string) {
	serviceMu.Lock()
	defer serviceMu.Unlock()
	service = name
}

func currentService() string {
	serviceMu.RLock()
	defer serviceMu.RUnlock()
	return service
}

// Spawn launches fn in a new goroutine guarded by a defer-recover. On panic
// it logs the panic value + full stack trace + name via slog, increments
// PanicTotal{service,goroutine=name}, and returns from the goroutine
// without re-panicking. The caller is responsible for any restart logic.
//
// name should be a stable, low-cardinality descriptor of the goroutine's
// purpose (e.g. "poll-loop", "unix-accept", "shutdown-watcher"). It becomes
// a Prometheus label, so do not embed request IDs or per-call data.
func Spawn(name string, fn func()) {
	go func() {
		defer Recover(name)
		fn()
	}()
}

// Recover is the deferred-recover body used by Spawn and exposed for callers
// who already have their own go-statement (e.g. wrapped in a sync.WaitGroup
// or per-connection accept) but still want the same panic-handling shape.
//
// Usage:
//
//	go func() {
//	    defer wg.Done()
//	    defer safego.Recover("my-goroutine")
//	    // body
//	}()
//
// Note: when chaining defers, place safego.Recover INNERMOST (closest to the
// body) so any other defers (Done, Close) still fire after recovery.
func Recover(name string) {
	if r := recover(); r != nil {
		stack := debug.Stack()
		slog.Error("goroutine panic recovered",
			"goroutine", name,
			"service", currentService(),
			"panic", r,
			"stack", string(stack),
		)
		PanicTotal.WithLabelValues(currentService(), name).Inc()
	}
}
