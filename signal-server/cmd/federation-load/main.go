// federation-load drives Wave 3 §3.4 — a multi-server load test against a
// federated signal-server topology.
//
// It opens ConnectStream against each --peers server with peers-per-server
// deterministic fake peer keys, then fires Send RPCs at --rate/sec from a
// random non-owning server to a random target. That forces most calls through
// federation Forward. Every second it samples p50/p99/error rate and prints a
// compact JSON line; on SIGINT (or --duration) it writes a final summary JSON
// to --out.
//
// F-265 instrumentation (the reason a federation miss is otherwise undiagnosable): the
// target pool is ONLINE registered peers PLUS an OFFLINE cohort of keys that are
// never registered anywhere. A NotFound to an online peer sent from a non-owning
// server is PATHOLOGICAL — the cross-shard federation Forward failed for a peer
// that IS registered (the F-265 bug). A NotFound to an offline key is EXPECTED
// and models the baseline "offers to powered-off peers" noise
// that would mask the pathological misses. The summary reports the
// two separately; pathological_miss_rate is the headline pass/fail metric
// (must be ≈ 0 when federation forwarding is healthy).
//
// Kept deliberately simple — no metrics registry, no goroutine pools beyond
// one per open stream. Matches the style of cmd/federation-probe.
package main

import (
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"log"
	"math/rand"
	"os"
	"os/signal"
	"sort"
	"strings"
	"sync"
	"sync/atomic"
	"syscall"
	"time"

	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"

	signalpb "github.com/usareur-af/goatcore/signal-server/gen/signalexchange"
)

// config is one run's settings, from the command line.
type config struct {
	servers               []string
	rate                  float64
	duration              time.Duration
	peersPerServer        int
	offlinePeersPerServer int
	sendTimeout           time.Duration
	seed                  int64
}

func main() {
	peersFlag := flag.String("peers", "", "comma-separated host:port list of signal-server instances (3+ recommended)")
	rate := flag.Float64("rate", 100, "target Send RPCs per second")
	duration := flag.Duration("duration", 10*time.Minute, "total load duration; 0 means run until SIGINT")
	peersPerServer := flag.Int("peers-per-server", 50, "number of fake ConnectStream peer keys to register per server")
	offlinePeersPerServer := flag.Int("offline-peers-per-server", 50, "number of UNREGISTERED (offline) target keys per server. Sends to these SHOULD return NotFound — they model powered-off peers and separate normal misses from pathological cross-shard misses. 0 disables the offline cohort.")
	sendTimeout := flag.Duration("send-timeout", 5*time.Second, "per-Send RPC deadline")
	out := flag.String("out", "", "path to write final summary JSON (default stderr)")
	seed := flag.Int64("seed", 0, "RNG seed; 0 = time-based")
	flag.Parse()

	cfg := config{
		servers:               splitCSV(*peersFlag),
		rate:                  *rate,
		duration:              *duration,
		peersPerServer:        *peersPerServer,
		offlinePeersPerServer: *offlinePeersPerServer,
		sendTimeout:           *sendTimeout,
		seed:                  *seed,
	}
	rootCtx, stop := signal.NotifyContext(context.Background(), syscall.SIGINT, syscall.SIGTERM)
	defer stop()
	summary, err := run(rootCtx, cfg)
	if err != nil {
		log.Fatal(err)
	}
	writeSummary(summary, *out)
}

// run drives the load against cfg.servers until cfg.duration ends or ctx is
// cancelled, and returns the summary.
func run(rootCtx context.Context, cfg config) (finalSummary, error) {
	servers := cfg.servers
	if len(servers) < 2 {
		return finalSummary{}, errors.New("--peers requires at least 2 host:port entries (3+ recommended for §3.4)")
	}
	if cfg.peersPerServer < 1 {
		return finalSummary{}, errors.New("--peers-per-server must be >= 1")
	}
	if cfg.rate <= 0 {
		return finalSummary{}, errors.New("--rate must be > 0")
	}

	rnd := rand.New(rand.NewSource(nonZeroSeed(cfg.seed)))

	// Dial every server once for Send. Separate from the stream connections
	// so that closing a registration stream doesn't disturb the sender path.
	sendConns := make([]*grpc.ClientConn, len(servers))
	sendClients := make([]signalpb.SignalExchangeClient, len(servers))
	for i, addr := range servers {
		c, err := grpc.NewClient(dialTarget(addr), grpc.WithTransportCredentials(insecure.NewCredentials()))
		if err != nil {
			return finalSummary{}, fmt.Errorf("dial %s (sender): %w", addr, err)
		}
		sendConns[i] = c
		sendClients[i] = signalpb.NewSignalExchangeClient(c)
	}
	defer func() {
		for _, c := range sendConns {
			_ = c.Close()
		}
	}()

	// Register peers-per-server streams against each server in parallel.
	// Key format "lf-<peer_idx>-on-<server_idx>" makes ownership visible in
	// logs; the load loop uses it to pick a non-owning sender below.
	type registeredPeer struct {
		key      string
		serverIx int
	}
	allKeys := make([]registeredPeer, 0, len(servers)*(cfg.peersPerServer))
	var regMu sync.Mutex

	regCtx, regCancel := context.WithTimeout(rootCtx, 30*time.Second)
	defer regCancel()

	var regWG sync.WaitGroup
	errCh := make(chan error, len(servers)*(cfg.peersPerServer))
	streamConns := make([]*grpc.ClientConn, len(servers))
	for si, addr := range servers {
		c, err := grpc.NewClient(dialTarget(addr), grpc.WithTransportCredentials(insecure.NewCredentials()))
		if err != nil {
			return finalSummary{}, fmt.Errorf("dial %s (stream): %w", addr, err)
		}
		streamConns[si] = c
		sc := signalpb.NewSignalExchangeClient(c)

		for pi := 0; pi < cfg.peersPerServer; pi++ {
			key := fmt.Sprintf("lf-%d-on-%d", pi, si)
			regWG.Add(1)
			go func(serverIx int, key string) {
				// regWG.Done() is called EXPLICITLY (below), not deferred: this
				// goroutine goes on to drain the stream forever and never returns,
				// so a deferred Done would never fire and regWG.Wait() would hang.
				md := metadata.AppendToOutgoingContext(rootCtx, "x-wiretrustee-peer-id", key)
				stream, err := sc.ConnectStream(md)
				if err != nil {
					errCh <- fmt.Errorf("ConnectStream(%s, %s): %w", servers[serverIx], key, err)
					regWG.Done()
					return
				}
				// The server registers the peer when it accepts the ConnectStream
				// (observable as signal_server_connected_peers) but does NOT
				// eagerly flush the gRPC registration header — stream.Header()
				// blocks until the first forwarded message, which never arrives
				// for an idle peer, wedging the whole registration so
				// regWG.Wait() never returns. So treat a successful ConnectStream
				// as registered; a short settle window after all registrations
				// (below) covers the async server-side commit before Send traffic.
				regMu.Lock()
				allKeys = append(allKeys, registeredPeer{key: key, serverIx: serverIx})
				regMu.Unlock()
				// Registered — signal it NOW, before the never-returning drain
				// loop below, so regWG.Wait() can complete.
				regWG.Done()
				// Drain inbound forever; we don't validate bodies here because
				// the load test is measuring server-side behavior, not delivery
				// correctness (federation-probe covers the delivery check).
				for {
					if _, err := stream.Recv(); err != nil {
						if errors.Is(err, io.EOF) || errors.Is(regCtx.Err(), context.Canceled) {
							return
						}
						// Don't fail the test — a dropped stream after the
						// scenario kills a server is expected behavior.
						return
					}
				}
			}(si, key)
		}
	}
	regWG.Wait()
	close(errCh)
	for err := range errCh {
		// Report but don't abort — scenarios may partially tear things down
		// and we want the loader to keep measuring through the disruption.
		fmt.Fprintf(os.Stderr, "registration warning: %v\n", err)
	}
	defer func() {
		for _, c := range streamConns {
			_ = c.Close()
		}
	}()

	regMu.Lock()
	keyCount := len(allKeys)
	regMu.Unlock()
	if keyCount == 0 {
		return finalSummary{}, errors.New("no peers registered; aborting")
	}
	fmt.Fprintf(os.Stderr, "registered %d peers across %d servers\n", keyCount, len(servers))

	// Settle window: ConnectStream returns before the server has fully committed
	// the peer to its registry (and propagated presence). Give it a moment so the
	// first Sends don't race registration and count as spurious misses.
	settle := 2 * time.Second
	select {
	case <-time.After(settle):
	case <-rootCtx.Done():
	}

	// F-265: build the target pool = online (registered) peers + an OFFLINE
	// cohort of keys that were never registered anywhere. See the package doc.
	// allKeys is stable here (all registration goroutines joined at regWG.Wait).
	type target struct {
		key      string
		online   bool
		serverIx int // owning server for online targets; -1 for offline
	}
	targets := make([]target, 0, keyCount+len(servers)*(cfg.offlinePeersPerServer))
	for _, p := range allKeys {
		targets = append(targets, target{key: p.key, online: true, serverIx: p.serverIx})
	}
	for i := 0; i < len(servers)*(cfg.offlinePeersPerServer); i++ {
		targets = append(targets, target{key: fmt.Sprintf("off-%d-unreg", i), online: false, serverIx: -1})
	}
	targetCount := len(targets)
	fmt.Fprintf(os.Stderr, "targets: %d online + %d offline\n", keyCount, targetCount-keyCount)

	// Load loop. A single ticker at the target rate keeps timing precise; we
	// fire each Send in its own goroutine so slow RPCs don't starve the tick.
	interval := time.Duration(float64(time.Second) / cfg.rate)
	if interval <= 0 {
		interval = time.Microsecond
	}
	ticker := time.NewTicker(interval)
	defer ticker.Stop()

	runCtx := rootCtx
	if cfg.duration > 0 {
		var cancel context.CancelFunc
		runCtx, cancel = context.WithTimeout(rootCtx, cfg.duration)
		defer cancel()
	}

	var (
		sent                 atomic.Int64
		delivered            atomic.Int64
		notFound             atomic.Int64
		notFoundOffline      atomic.Int64 // F-265: expected miss (target offline)
		notFoundPathological atomic.Int64 // F-265: BUG — online peer unreachable cross-shard
		sentOnline           atomic.Int64
		sentOffline          atomic.Int64
		errored              atomic.Int64
		totalLatNS           atomic.Int64
		inFlight             atomic.Int64
		samplesMu            sync.Mutex
		perSec               []secondSample
		windowLat            []time.Duration
		windowErrs           int64
		windowSent           int64
		windowPath           int64 // pathological misses this second
		windowSentOnline     int64 // sends to online targets this second
	)

	done := make(chan struct{})
	go func() {
		sec := time.NewTicker(time.Second)
		defer sec.Stop()
		for {
			select {
			case <-runCtx.Done():
				close(done)
				return
			case <-sec.C:
				samplesMu.Lock()
				p50, p99 := percentiles(windowLat, 0.50, 0.99)
				var errRate float64
				if windowSent > 0 {
					errRate = float64(windowErrs) / float64(windowSent)
				}
				// F-265 headline: fraction of sends to ONLINE peers that missed
				// cross-shard. Denominator is online sends only (offline sends are
				// expected to miss and don't belong in this rate).
				var pathRate float64
				if windowSentOnline > 0 {
					pathRate = float64(windowPath) / float64(windowSentOnline)
				}
				s := secondSample{
					TS:           time.Now().UTC().Format(time.RFC3339Nano),
					P50ms:        float64(p50.Microseconds()) / 1000,
					P99ms:        float64(p99.Microseconds()) / 1000,
					ErrRate:      errRate,
					Sent:         windowSent,
					PathMisses:   windowPath,
					PathMissRate: pathRate,
					InFlight:     inFlight.Load(),
				}
				perSec = append(perSec, s)
				windowLat = windowLat[:0]
				windowErrs = 0
				windowSent = 0
				windowPath = 0
				windowSentOnline = 0
				samplesMu.Unlock()
				b, err := json.Marshal(s)
				if err != nil {
					log.Printf("encode interval sample: %v", err)
					continue
				}
				fmt.Println(string(b))
			}
		}
	}()

	for {
		select {
		case <-runCtx.Done():
			<-done
			goto summarize
		case <-ticker.C:
			// Pick a target (online or offline) and a sender server. For an
			// online target, send from a NON-owning server to force the
			// cross-shard federation Forward path (the F-265 surface). For an
			// offline target there is no owner, so any sender is fine.
			dst := targets[rnd.Intn(targetCount)]
			senderIx := rnd.Intn(len(servers))
			if dst.online && senderIx == dst.serverIx && len(servers) > 1 {
				senderIx = (senderIx + 1) % len(servers)
			}
			inFlight.Add(1)
			go func() {
				defer inFlight.Add(-1)
				ctx, cancel := context.WithTimeout(runCtx, cfg.sendTimeout)
				defer cancel()
				start := time.Now()
				_, err := sendClients[senderIx].Send(ctx, &signalpb.EncryptedMessage{
					Key:       fmt.Sprintf("loadgen-%d", senderIx),
					RemoteKey: dst.key,
					Body:      []byte("x"),
				})
				elapsed := time.Since(start)
				totalLatNS.Add(elapsed.Nanoseconds())
				if dst.online {
					sentOnline.Add(1)
				} else {
					sentOffline.Add(1)
				}
				sent.Add(1)
				samplesMu.Lock()
				windowSent++
				windowLat = append(windowLat, elapsed)
				if dst.online {
					windowSentOnline++
				}
				samplesMu.Unlock()
				switch {
				case err == nil:
					delivered.Add(1)
				case status.Code(err) == codes.NotFound:
					notFound.Add(1)
					if dst.online {
						// F-265: an ONLINE peer, reached from a non-owning server,
						// was NotFound — the cross-shard federation Forward failed.
						// This is the pathological miss (the bug).
						notFoundPathological.Add(1)
						samplesMu.Lock()
						windowPath++
						samplesMu.Unlock()
					} else {
						// Expected: target offline (never registered) — the
						// baseline miss noise, not a fault.
						notFoundOffline.Add(1)
					}
				default:
					errored.Add(1)
					samplesMu.Lock()
					windowErrs++
					samplesMu.Unlock()
				}
			}()
		}
	}

summarize:
	// Allow ~2×sendTimeout for in-flight Sends to settle before writing the
	// summary; otherwise the counts understate what actually shipped.
	drainDeadline := time.Now().Add(2 * cfg.sendTimeout)
	for inFlight.Load() > 0 && time.Now().Before(drainDeadline) {
		time.Sleep(50 * time.Millisecond)
	}

	// We didn't keep raw per-request latencies for the whole run (windowLat
	// was cleared each second), so derive overall p50/p99 from per-second
	// samples — a good-enough summary for scenario scoring. The max of
	// per-second p99s is the metric scenarios actually care about.
	samplesMu.Lock()
	overallP50, overallP99 := percentilesFromSamples(perSec)
	samplesMu.Unlock()

	totalSent := sent.Load()
	var avgMS float64
	if totalSent > 0 {
		avgMS = float64(totalLatNS.Load()) / float64(totalSent) / 1e6
	}
	so := sentOnline.Load()
	var pathRate float64
	if so > 0 {
		pathRate = float64(notFoundPathological.Load()) / float64(so)
	}
	summary := finalSummary{
		StartedAt:            time.Now().UTC().Format(time.RFC3339Nano),
		Servers:              servers,
		TargetRate:           cfg.rate,
		PeersPerServer:       cfg.peersPerServer,
		RegisteredPeers:      keyCount,
		TotalSent:            totalSent,
		SentOnline:           sentOnline.Load(),
		SentOffline:          sentOffline.Load(),
		TotalDelivered:       delivered.Load(),
		TotalNotFound:        notFound.Load(),
		NotFoundOffline:      notFoundOffline.Load(),
		NotFoundPathological: notFoundPathological.Load(),
		PathologicalMissRate: pathRate,
		TotalError:           errored.Load(),
		AvgLatencyMS:         avgMS,
		OverallP50MS:         overallP50,
		OverallP99MS:         overallP99,
		PerSecondSamples:     perSec,
	}
	return summary, nil
}

type secondSample struct {
	TS           string  `json:"ts"`
	P50ms        float64 `json:"p50_ms"`
	P99ms        float64 `json:"p99_ms"`
	ErrRate      float64 `json:"err_rate"`
	Sent         int64   `json:"sent"`
	PathMisses   int64   `json:"path_misses"`    // F-265 pathological cross-shard misses this second
	PathMissRate float64 `json:"path_miss_rate"` // path_misses / sends-to-online-peers
	InFlight     int64   `json:"in_flight"`
}

type finalSummary struct {
	StartedAt            string         `json:"started_at"`
	Servers              []string       `json:"servers"`
	TargetRate           float64        `json:"target_rate_per_sec"`
	PeersPerServer       int            `json:"peers_per_server"`
	RegisteredPeers      int            `json:"registered_peers"`
	TotalSent            int64          `json:"total_sent"`
	SentOnline           int64          `json:"sent_online"`
	SentOffline          int64          `json:"sent_offline"`
	TotalDelivered       int64          `json:"total_delivered"`
	TotalNotFound        int64          `json:"total_not_found"`
	NotFoundOffline      int64          `json:"not_found_offline"`      // F-265: expected (target offline)
	NotFoundPathological int64          `json:"not_found_pathological"` // F-265: BUG (online peer, cross-shard forward failed)
	PathologicalMissRate float64        `json:"pathological_miss_rate"` // not_found_pathological / sent_online — HEADLINE pass/fail (≈0 = healthy)
	TotalError           int64          `json:"total_error"`
	AvgLatencyMS         float64        `json:"avg_latency_ms"`
	OverallP50MS         float64        `json:"overall_p50_ms"`
	OverallP99MS         float64        `json:"overall_p99_ms"`
	PerSecondSamples     []secondSample `json:"per_second_samples"`
}

func writeSummary(s finalSummary, path string) {
	b, err := json.MarshalIndent(s, "", "  ")
	if err != nil {
		fmt.Fprintf(os.Stderr, "summary marshal: %v\n", err)
		return
	}
	if path == "" {
		fmt.Fprintln(os.Stderr, string(b))
		return
	}
	if err := os.WriteFile(path, b, 0o644); err != nil {
		fmt.Fprintf(os.Stderr, "summary write %s: %v\n", path, err)
		return
	}
	fmt.Fprintf(os.Stderr, "summary written to %s\n", path)
}

// percentiles returns p50 and p99 of a duration slice. Returns zero for an
// empty slice so callers don't need a null check.
func percentiles(xs []time.Duration, qs ...float64) (p50, p99 time.Duration) {
	if len(xs) == 0 {
		return 0, 0
	}
	cp := append([]time.Duration(nil), xs...)
	sort.Slice(cp, func(i, j int) bool { return cp[i] < cp[j] })
	pick := func(q float64) time.Duration {
		idx := int(q * float64(len(cp)-1))
		return cp[idx]
	}
	return pick(0.50), pick(0.99)
}

func percentilesFromSamples(ss []secondSample) (p50, p99 float64) {
	if len(ss) == 0 {
		return 0, 0
	}
	p50s := make([]float64, 0, len(ss))
	p99s := make([]float64, 0, len(ss))
	for _, s := range ss {
		p50s = append(p50s, s.P50ms)
		p99s = append(p99s, s.P99ms)
	}
	sort.Float64s(p50s)
	sort.Float64s(p99s)
	// Median of per-second p50s, and max of per-second p99s as the overall
	// tail — that's the metric scenarios actually care about.
	mid := len(p50s) / 2
	overallP50 := p50s[mid]
	overallP99 := p99s[len(p99s)-1]
	return overallP50, overallP99
}

func splitCSV(s string) []string {
	var out []string
	for _, p := range strings.Split(s, ",") {
		if p = strings.TrimSpace(p); p != "" {
			out = append(out, p)
		}
	}
	return out
}

// dialTarget wraps a bare host:port in the passthrough scheme so grpc.NewClient
// uses the literal address instead of routing it through the default DNS
// resolver. With modern grpc-go, a bare "127.0.0.1:10000" (or any IP:port) is
// parsed as scheme "dns" with an empty host and the resolver returns an empty
// address set, leaving the channel stuck in CONNECTING and every RPC blocked
// until its deadline — the loader registered peers but sent 0 with no error.
// Targets that already carry a scheme (e.g. "dns:///host") are left untouched.
func dialTarget(addr string) string {
	if strings.Contains(addr, "://") {
		return addr
	}
	return "passthrough:///" + addr
}

func nonZeroSeed(s int64) int64 {
	if s != 0 {
		return s
	}
	return time.Now().UnixNano()
}
