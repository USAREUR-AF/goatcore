// Command snitch monitors network performance across the goat mesh.
//
// Three modes:
//   - serve: Start the HTTP dashboard and collector
//   - probe: Run one measurement cycle and POST results to the collector
//   - run:   Run one measurement cycle and print results locally
package main

import (
	"bytes"
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"log"
	"math/rand"
	"net/http"
	"os"
	"time"

	"github.com/usareur-af/goatcore/ops/snitch/internal/auth"
	"github.com/usareur-af/goatcore/ops/snitch/internal/collector"
	"github.com/usareur-af/goatcore/ops/snitch/internal/compat"
	"github.com/usareur-af/goatcore/ops/snitch/internal/identity"
	"github.com/usareur-af/goatcore/ops/snitch/internal/measure"
	"github.com/usareur-af/goatcore/ops/snitch/internal/server"
	"github.com/usareur-af/goatcore/ops/snitch/internal/store"
	"github.com/usareur-af/goatcore/ops/snitch/internal/types"
)

func main() {
	log.SetFlags(log.LstdFlags | log.Lshortfile)

	if len(os.Args) < 2 {
		usage()
		os.Exit(1)
	}

	// The earlier subcommand name runs the same command. Mapped before the
	// switch: a build in which both names are the same string still compiles.
	cmd := os.Args[1]
	if cmd == compat.SatlinkProbeCommand {
		cmd = "satlink-probe"
	}

	switch cmd {
	case "serve":
		cmdServe(os.Args[2:])
	case "probe":
		cmdProbe(os.Args[2:])
	case "run":
		cmdRun(os.Args[2:])
	case "satlink-probe":
		cmdSatlinkProbe(os.Args[2:])
	case "throughput-probe":
		cmdThroughputProbe(os.Args[2:])
	case "policy":
		cmdPolicy(os.Args[2:])
	case "self-test":
		cmdSelfTest(os.Args[2:])
	case "help", "-h", "--help":
		usage()
	default:
		fmt.Fprintf(os.Stderr, "unknown command: %s\n", os.Args[1])
		usage()
		os.Exit(1)
	}
}

func usage() {
	fmt.Fprintln(os.Stderr, `Usage: snitch <command> [flags]

Commands:
  serve              Start the HTTP dashboard and collector
  probe              Run one measurement cycle and POST to collector
  run                Run one measurement cycle and print results
  satlink-probe      Run one satellite-link PoP benchmark and store results
  throughput-probe   Run one daily-throughput sample (single + multi flow iperf3)
  policy             Get/set the fleet-wide probe policy (off|minimal|default|focused-research)
  self-test          Exercise every probe step once and emit per-step JSON (multi-platform install verification)

Serve flags:
  --addr                  Listen address (default: :8080)
  --db-url                PostgreSQL connection string
  --key-file              Identity encryption key file (default: identity.key)
  --satlink-interval      Satellite-link PoP probe interval (default: 24h, 0 = disabled)

Probe flags:
  --collector      Collector URL (required; or SNITCH_COLLECTOR)
  --interval       Repeat interval in seconds (0 = run once, default: 60)
  --probe-key      API key for authentication

Environment:
  SNITCH_DB_URL         PostgreSQL connection string
  SNITCH_COLLECTOR      Collector URL override
  SNITCH_PROBE_KEY      Probe API key
  SNITCH_IDENTITY_KEY   Hex-encoded master key (alternative to --key-file)`)
}

func cmdServe(args []string) {
	fs := flag.NewFlagSet("serve", flag.ExitOnError)
	addr := fs.String("addr", ":8080", "listen address")
	dbURL := fs.String("db-url", "", "PostgreSQL connection string")
	keyFile := fs.String("key-file", "identity.key", "identity encryption key file")
	satlinkInterval, compatInterval := satlinkIntervalFlags(fs)
	_ = fs.Parse(args) //nolint:errcheck // ExitOnError terminates instead of returning an error
	*satlinkInterval = resolveSatlinkInterval(fs, *satlinkInterval, *compatInterval)

	// Database URL: flag > env > default.
	if *dbURL == "" {
		*dbURL = os.Getenv("SNITCH_DB_URL")
	}
	if *dbURL == "" {
		*dbURL = "postgres://snitch:snitch@localhost:5432/snitch?sslmode=disable"
	}

	// Identity key: env (hex) > file (raw bytes).
	var masterKey []byte
	var err error
	if hexKey := os.Getenv("SNITCH_IDENTITY_KEY"); hexKey != "" {
		masterKey = []byte(hexKey)
		if len(masterKey) < 16 {
			log.Fatalf("SNITCH_IDENTITY_KEY must be at least 16 bytes")
		}
	} else {
		masterKey, err = identity.LoadOrGenerateKey(*keyFile)
		if err != nil {
			log.Fatalf("identity key: %v", err)
		}
	}
	idm, err := identity.NewManager(masterKey)
	if err != nil {
		log.Fatalf("identity manager: %v", err)
	}

	// PostgreSQL store.
	ctx := context.Background()
	st, err := store.Open(ctx, *dbURL)
	if err != nil {
		log.Fatalf("store: %v", err)
	}
	defer st.Close()

	// Auth (RBAC).
	au := auth.New(st.Pool())
	adminKey, probeKey, err := au.EnsureDefaults(ctx)
	if err != nil {
		log.Fatalf("auth: %v", err)
	}
	if adminKey != "" {
		log.Println("============================================")
		log.Println("  FIRST RUN — SAVE THESE API KEYS")
		log.Println("============================================")
		log.Printf("  ADMIN KEY: %s", adminKey)
		log.Printf("  PROBE KEY: %s", probeKey)
		log.Println("============================================")
		log.Println("  Admin key: dashboard + management access")
		log.Println("  Probe key: distribute to mesh peers")
		log.Println("  These are shown ONCE. Store them now.")
		log.Println("============================================")
	}

	// Collector + server.
	c := collector.New(st, idm)
	srv := server.New(server.Config{
		Collector:   c,
		Auth:        au,
		IsCollector: true,
	})

	srv.StartSatlinkProber(*satlinkInterval)

	if err := srv.ListenAndServe(*addr); err != nil {
		log.Fatalf("server: %v", err)
	}
}

func cmdProbe(args []string) {
	fs := flag.NewFlagSet("probe", flag.ExitOnError)
	collectorURL := fs.String("collector", "", "collector URL")
	interval := fs.Int("interval", 60, "base interval in seconds (0 = once); actual sleep jitters [interval-5, interval+5]")
	probeKey := fs.String("probe-key", "", "probe API key")
	budget := fs.Duration("cycle-budget", 5*time.Second, "soft per-cycle budget; sub-steps that would start past this are skipped")
	disableRelay := fs.Bool("disable-relay-probe", false, "disable per-relay transport-assertion + reachability + TLS expiry probes (Block 61)")
	disableMTU := fs.Bool("disable-mtu-probe", false, "disable one-shot path-MTU probe (Block 61)")
	disableDialer := fs.Bool("disable-dialer-counters", false, "disable race-dialer outcome counters from journalctl (Block 61)")
	_ = fs.Parse(args) //nolint:errcheck // ExitOnError terminates instead of returning an error

	if *collectorURL == "" {
		*collectorURL = os.Getenv("SNITCH_COLLECTOR")
	}
	if *collectorURL == "" {
		fmt.Fprintln(os.Stderr, "snitch probe: no collector URL: pass --collector <url> or set SNITCH_COLLECTOR")
		os.Exit(2)
	}
	if *probeKey == "" {
		*probeKey = os.Getenv("SNITCH_PROBE_KEY")
	}

	// Block 61 probe options are wired here (they live in cmd because
	// they need OS access; the inner RunProbe is OS-free for testing).
	// The NetworkCache + RelayProbeConfig + MTU prober closures all
	// survive across cycles in this loop so cache-on-change sub-probes
	// honor their re-fetch triggers.
	netCache := measure.NewNetworkCache()
	relayCfg := measure.DefaultRelayProbeConfig()
	mtuProber := measure.MTUProberCachingByInterface("", "")

	// Probe-policy client. Polled once per cycle; controls which
	// sub-probes run + bypasses caches under focused-research mode.
	policyClient := measure.NewPolicyClient(*collectorURL)

	// Cumulative self-metrics. Reset on probe-process restart.
	var (
		cyclesCompleted int64
		cyclesSkipped   int64
		relayProbesRun  int64
		tlsHandshakes   int64
		tcpConnects     int64
	)

	rng := rand.New(rand.NewSource(time.Now().UnixNano()))
	for {
		cycleStart := time.Now()

		// Step 1: resolve current policy. Kill switch (SNITCH_PROBE_DISABLED)
		// supersedes everything; otherwise pull from collector with cached
		// fallback. ApplyPolicy handles ExpiresAt → revert-to-default.
		policyCtx, policyCancel := context.WithTimeout(context.Background(), 3*time.Second)
		rawPolicy, policySource := policyClient.CurrentPolicy(policyCtx)
		policyCancel()
		policy := measure.AppliedPolicy(rawPolicy, time.Now())

		// Step 2: build cycle-specific probe options based on policy.
		probeOpts := []measure.ProbeOption{
			measure.WithCycleBudget(*budget),
			measure.WithNetwork(netCache.CachingNetwork()),
		}

		if !*disableRelay && measure.PolicyAllowsRelayProbe(policy.Mode) {
			// Under focused-research, bypass the cert cache so we get a
			// fresh measurement every cycle. Otherwise honor the 1h TTL.
			cfgForCycle := relayCfg
			if measure.PolicyForcesUncached(policy.Mode) {
				cfgForCycle.CertProbeTTL = 0
			}
			runTLS := measure.PolicyAllowsTLSProbe(policy.Mode)
			probeOpts = append(probeOpts, measure.WithRelayProbe(func(ctx context.Context, url string) types.RelayHealth {
				rh := measure.ProbeRelay(ctx, url, cfgForCycle)
				relayProbesRun++
				if runTLS && rh.TLSDaysUntil >= 0 {
					tlsHandshakes++
				}
				if rh.TCPReachable {
					tcpConnects++
				}
				return rh
			}))
		}
		if !*disableMTU && measure.PolicyAllowsMTU(policy.Mode) {
			probeOpts = append(probeOpts, measure.WithMTUProbe(mtuProber))
		}
		if !*disableDialer && measure.PolicyAllowsDialerCounters(policy.Mode) {
			probeOpts = append(probeOpts, measure.WithDialerCounters(measure.CountDialerOutcomesJournalctl))
		}

		// Step 3: run probe (or skip if policy is "off") + emit report.
		var report *types.ProbeReport
		var err error
		if policy.Mode == types.PolicyOff {
			// Heartbeat-only report. Confirms peer is alive without
			// running any active probes.
			report = heartbeatReport()
			cyclesSkipped++
		} else {
			ctx, cancel := context.WithTimeout(context.Background(), 6*(*budget))
			report, err = measure.RunProbe(ctx, probeOpts...)
			cancel()
			if err == nil {
				cyclesCompleted++
			} else {
				cyclesSkipped++
			}
		}

		// Step 4: attach self-metrics + emit.
		if report != nil {
			killReason, killActive := envKillSwitchSummary()
			report.SelfMetrics = &types.ProbeSelfMetrics{
				PolicyMode:       policy.Mode,
				PolicyExpiresAt:  policy.ExpiresAt,
				PolicySource:     policySource,
				KillSwitchActive: killActive,
				BytesOutEstimate: estimateBytesOut(report),
				BytesInEstimate:  estimateBytesIn(report),
				CyclesCompleted:  cyclesCompleted,
				CyclesSkipped:    cyclesSkipped,
				RelayProbesRun:   relayProbesRun,
				TLSHandshakes:    tlsHandshakes,
				TCPConnects:      tcpConnects,
			}
			_ = killReason

			if err != nil {
				log.Printf("probe error: %v", err)
			} else if perr := postReport(*collectorURL, *probeKey, report); perr != nil {
				log.Printf("post report: %v", perr)
			} else {
				logCycleSummary(report, *collectorURL)
			}
		}

		if *interval <= 0 {
			break
		}
		// Jitter [interval-5, interval+5]. Default 60 → uniform[55s, 65s].
		jitterRange := 10
		base := *interval - 5
		if base < 1 {
			base = 1
		}
		sleep := time.Duration(base+rng.Intn(jitterRange+1)) * time.Second
		// If the cycle itself ran long, subtract from the sleep so we
		// don't drift below the configured cadence.
		elapsed := time.Since(cycleStart)
		if elapsed < sleep {
			sleep -= elapsed
		} else {
			sleep = 1 * time.Second
		}
		time.Sleep(sleep)
	}
}

// heartbeatReport is the minimal report the probe emits when policy is
// "off". Keeps the collector aware that the peer is alive without
// running any active probes.
func heartbeatReport() *types.ProbeReport {
	return &types.ProbeReport{
		Timestamp: time.Now().UTC(),
	}
}

// envKillSwitchSummary returns (reason, active) for the kill switch
// env var. Used for ProbeSelfMetrics.KillSwitchActive surface.
func envKillSwitchSummary() (string, bool) {
	v := os.Getenv("SNITCH_PROBE_DISABLED")
	return v, v != ""
}

// estimateBytesOut is a rough probe-egress estimator. The dominant
// terms: TCP connect probes (~80B per connect), TLS handshakes (~6KB
// out for client hello + key exchange), report POST itself.
//
// Intentionally rough — we're surfacing fleet-wide load trends to
// operators, not billing.
func estimateBytesOut(r *types.ProbeReport) int64 {
	var b int64
	// Each relay row ~ 1 TCP connect (80B). TLS probe is gated by the
	// cache so we don't count it per-cycle here; the cumulative
	// TLSHandshakes counter on SelfMetrics carries that signal.
	for _, rh := range r.Relays {
		_ = rh
		b += 80
	}
	// Report POST. Estimated from peer count × avg row size.
	b += int64(200 + 120*len(r.Peers))
	return b
}

// estimateBytesIn is the inbound side: TLS cert chain bytes (~3KB
// when the prober runs), report POST response (small).
func estimateBytesIn(r *types.ProbeReport) int64 {
	// Cert chain ~3KB per uncached probe. We can't tell from the report
	// alone how many cache misses occurred this cycle, so this is a
	// floor estimate (HTTP response only).
	return int64(120) // POST 200 OK with minimal body
}

// logCycleSummary emits a one-line summary of the cycle including
// Block 61 fields when present. Operators read this first; the full
// report is on the collector.
func logCycleSummary(r *types.ProbeReport, collectorURL string) {
	parts := []string{fmt.Sprintf("peers=%d", len(r.Peers))}
	if r.RaceDialer != nil {
		parts = append(parts, fmt.Sprintf("dialer=Q%d/W%d (fail Q%d/W%d)",
			r.RaceDialer.QUICSuccess, r.RaceDialer.WSSuccess,
			r.RaceDialer.QUICFailure, r.RaceDialer.WSFailure))
	}
	if r.MTU != nil {
		parts = append(parts, fmt.Sprintf("mtu=%d", r.MTU.PathMTU))
	}
	for _, rh := range r.Relays {
		parts = append(parts, fmt.Sprintf("relay[%s]=%s/u%v/t%v/cert%dd",
			rh.Endpoint, rh.Transport, rh.UDPReachable, rh.TCPReachable, rh.TLSDaysUntil))
	}
	if r.CycleBudget != nil {
		parts = append(parts, fmt.Sprintf("cycle=%dms (skipped=%d over=%v)",
			r.CycleBudget.ActualMs, r.CycleBudget.Skipped, r.CycleBudget.OverBudget))
	}
	log.Printf("reported to %s — %s", collectorURL, joinParts(parts))
}

func joinParts(parts []string) string {
	out := ""
	for i, p := range parts {
		if i > 0 {
			out += " "
		}
		out += p
	}
	return out
}

func cmdRun(args []string) {
	ctx, cancel := context.WithTimeout(context.Background(), 90*time.Second)
	defer cancel()

	report, err := measure.RunProbe(ctx)
	if err != nil {
		log.Fatalf("probe failed: %v", err)
	}

	enc := json.NewEncoder(os.Stdout)
	enc.SetIndent("", "  ")
	if err := enc.Encode(report); err != nil {
		log.Fatalf("encode: %v", err)
	}
}

func postReport(collectorURL, probeKey string, report *types.ProbeReport) error {
	data, err := json.Marshal(report)
	if err != nil {
		return fmt.Errorf("marshal: %w", err)
	}
	// Bounded 30 s timeout — collector POST shouldn't outlive a probe
	// cycle. The probe loop above also wraps each cycle with a soft
	// budget; this is the network-level backstop.
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	req, err := http.NewRequestWithContext(ctx, "POST", collectorURL+"/api/report", bytes.NewReader(data))
	if err != nil {
		return fmt.Errorf("create request: %w", err)
	}
	req.Header.Set("Content-Type", "application/json")
	if probeKey != "" {
		req.Header.Set("Authorization", "Bearer "+probeKey)
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return fmt.Errorf("POST %s: %w", collectorURL, err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("POST %s: HTTP %d", collectorURL, resp.StatusCode)
	}
	return nil
}

// cmdPolicy is the operator-facing CLI for reading / setting the fleet
// probe policy. Two sub-modes: `policy get` and `policy set`. Reads
// the admin key from --admin-key or SNITCH_ADMIN_KEY env. Calls the
// collector's /api/probe-policy + /api/probe-policy/set endpoints.
//
// Examples:
//
//	snitch policy get
//
//	# 10-minute focused-research window:
//	snitch policy set --mode focused-research --duration 10m \
//	  --reason "weekly net-health snapshot" --set-by operator:alice
//
//	# Abort an in-progress focused-research window early:
//	snitch policy set --mode default --reason abort
//
//	# Stop probes fleet-wide (mission-critical disruption):
//	snitch policy set --mode off --reason "operational disruption"
func cmdPolicy(args []string) {
	if len(args) < 1 {
		fmt.Fprintln(os.Stderr, "usage: snitch policy {get|set} [flags]")
		os.Exit(1)
	}
	collectorURL := os.Getenv("SNITCH_COLLECTOR")
	if collectorURL == "" {
		fmt.Fprintln(os.Stderr, "snitch policy: no collector URL: set SNITCH_COLLECTOR")
		os.Exit(2)
	}
	switch args[0] {
	case "get":
		policyGet(collectorURL)
	case "set":
		policySet(collectorURL, args[1:])
	default:
		fmt.Fprintf(os.Stderr, "unknown policy sub-command: %s\n", args[0])
		os.Exit(1)
	}
}

func policyGet(collectorURL string) {
	// gosec G704 (SSRF) false-positive: collectorURL is operator-supplied
	// via --collector or $SNITCH_COLLECTOR. There's no untrusted-input
	// path here; this is the operator's own CLI talking to their own
	// fleet collector.
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	req, err := http.NewRequestWithContext(ctx, "GET", collectorURL+"/api/probe-policy", nil) //nolint:gosec // operator-supplied URL; see comment above
	if err != nil {
		log.Fatalf("create request: %v", err)
	}
	resp, err := http.DefaultClient.Do(req) //nolint:gosec // operator-supplied URL; see comment above
	if err != nil {
		log.Fatalf("GET /api/probe-policy: %v", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		// G706 (log injection) false-positive: resp.StatusCode is an int.
		log.Fatalf("GET /api/probe-policy: HTTP %d", resp.StatusCode) //nolint:gosec // see comment above
	}
	var p types.ProbePolicy
	if err := json.NewDecoder(resp.Body).Decode(&p); err != nil {
		log.Fatalf("decode: %v", err)
	}
	enc := json.NewEncoder(os.Stdout)
	enc.SetIndent("", "  ")
	if err := enc.Encode(p); err != nil {
		log.Fatalf("encode policy: %v", err)
	}
}

func policySet(collectorURL string, args []string) {
	fs := flag.NewFlagSet("policy set", flag.ExitOnError)
	mode := fs.String("mode", "", "policy mode: off | minimal | default | focused-research")
	duration := fs.Duration("duration", 0, "auto-revert after this duration (e.g. 10m); 0 = no expiry")
	reason := fs.String("reason", "", "operator note (recorded in policy)")
	setBy := fs.String("set-by", "", "operator identity (defaults to $USER)")
	adminKey := fs.String("admin-key", "", "snitch admin key (or set SNITCH_ADMIN_KEY)")
	_ = fs.Parse(args) //nolint:errcheck // ExitOnError terminates instead of returning an error

	if *mode == "" {
		log.Fatalf("--mode is required")
	}
	if *adminKey == "" {
		*adminKey = os.Getenv("SNITCH_ADMIN_KEY")
	}
	if *adminKey == "" {
		log.Fatalf("--admin-key or SNITCH_ADMIN_KEY required")
	}
	if *setBy == "" {
		*setBy = "operator:" + os.Getenv("USER")
	}

	policy := types.ProbePolicy{
		Mode:   *mode,
		Reason: *reason,
		SetBy:  *setBy,
		SetAt:  time.Now().UTC(),
	}
	if *duration > 0 {
		policy.ExpiresAt = policy.SetAt.Add(*duration)
	}

	body, err := json.Marshal(policy)
	if err != nil {
		log.Fatalf("marshal: %v", err)
	}
	// gosec G704 (SSRF) false-positive: collectorURL is operator-supplied
	// via --collector or $SNITCH_COLLECTOR. Same shape as policyGet above.
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	req, err := http.NewRequestWithContext(ctx, "POST", collectorURL+"/api/probe-policy/set", bytes.NewReader(body)) //nolint:gosec // operator-supplied URL; see policyGet comment
	if err != nil {
		log.Fatalf("create request: %v", err)
	}
	req.Header.Set("Authorization", "Bearer "+*adminKey)
	req.Header.Set("Content-Type", "application/json")

	resp, err := http.DefaultClient.Do(req) //nolint:gosec // operator-supplied URL; see policyGet comment
	if err != nil {
		log.Fatalf("POST /api/probe-policy/set: %v", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		// G706 (log injection) false-positive: resp.StatusCode is an int.
		log.Fatalf("POST /api/probe-policy/set: HTTP %d", resp.StatusCode) //nolint:gosec // see comment above
	}
	enc := json.NewEncoder(os.Stdout)
	enc.SetIndent("", "  ")
	var got types.ProbePolicy
	if err := json.NewDecoder(resp.Body).Decode(&got); err != nil {
		log.Fatalf("decode response: %v", err)
	}
	if err := enc.Encode(got); err != nil {
		log.Fatalf("encode policy: %v", err)
	}
	if !got.ExpiresAt.IsZero() {
		log.Printf("policy %q applied; auto-reverts at %s", got.Mode, got.ExpiresAt.Format(time.RFC3339))
	} else {
		log.Printf("policy %q applied; no expiry (set --mode default to revert)", got.Mode)
	}
}

// cmdThroughputProbe runs one daily-throughput sample against a configured
// sentinel target and emits one JSONL line on stdout. Designed to be called
// from a systemd timer once per day per region.
//
// Storage: today this just writes JSONL — caller pipes to journald or a
// log shipper. A follow-up adds a `throughput_samples` table to the snitch
// store + a dashboard row; the instrumentation discipline (one JSON object
// per probe run) is the load-bearing part.
func cmdThroughputProbe(args []string) {
	fs := flag.NewFlagSet("throughput-probe", flag.ExitOnError)
	target := fs.String("target", "", "iperf3 target host (defaults to $SNITCH_THROUGHPUT_TARGET)")
	port := fs.Int("port", 5201, "iperf3 server port")
	_ = fs.Parse(args) //nolint:errcheck // ExitOnError terminates instead of returning an error

	if *target == "" {
		*target = os.Getenv("SNITCH_THROUGHPUT_TARGET")
	}
	if *target == "" {
		log.Fatalf("--target or SNITCH_THROUGHPUT_TARGET must be set")
	}

	// 30s wall-clock cap covers 10s × 1 flow + 10s × 4 flow + 10s slack.
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()

	out := measure.RunThroughputProbe(ctx, *target, *port, measure.DefaultIperfRunner())
	enc := json.NewEncoder(os.Stdout)
	if err := enc.Encode(out); err != nil {
		log.Fatalf("encode: %v", err)
	}
}

// satlinkIntervalFlags defines --satlink-interval and its earlier name on fs.
// Where both names are the same string, one flag is defined and both results
// point at it (defining a flag twice panics).
func satlinkIntervalFlags(fs *flag.FlagSet) (current, earlier *time.Duration) {
	current = fs.Duration("satlink-interval", 24*time.Hour, "satellite-link PoP probe interval (0 = disabled)")
	if fs.Lookup(compat.SatlinkIntervalFlag) != nil {
		return current, current
	}
	earlier = fs.Duration(compat.SatlinkIntervalFlag, 24*time.Hour, "earlier name of --satlink-interval (wins when both are set)")
	return current, earlier
}

// resolveSatlinkInterval applies the earlier flag name when it was given on
// the command line; it wins over --satlink-interval when both are set.
func resolveSatlinkInterval(fs *flag.FlagSet, current, earlier time.Duration) time.Duration {
	set := false
	fs.Visit(func(f *flag.Flag) {
		if f.Name == compat.SatlinkIntervalFlag {
			set = true
		}
	})
	if set {
		return earlier
	}
	return current
}

func cmdSatlinkProbe(args []string) {
	fs := flag.NewFlagSet("satlink-probe", flag.ExitOnError)
	dbURL := fs.String("db-url", "", "PostgreSQL connection string")
	_ = fs.Parse(args) //nolint:errcheck // ExitOnError terminates instead of returning an error

	if *dbURL == "" {
		*dbURL = os.Getenv("SNITCH_DB_URL")
	}
	if *dbURL == "" {
		*dbURL = "postgres://snitch:snitch@localhost:5432/snitch?sslmode=disable"
	}

	ctx := context.Background()
	st, err := store.Open(ctx, *dbURL)
	if err != nil {
		log.Fatalf("store: %v", err)
	}
	defer st.Close()

	log.Println("running satellite-link PoP benchmark...")
	result, err := measure.RunSatlinkProbe(ctx)
	if err != nil {
		log.Fatalf("probe failed: %v", err)
	}

	probeID, err := st.InsertSatlinkProbe(ctx, result)
	if err != nil {
		log.Fatalf("store probe: %v", err)
	}
	for _, m := range result.Measurements {
		if err := st.InsertSatlinkMeasurement(ctx, probeID, &m); err != nil {
			log.Printf("store measurement %s: %v", m.Target, err)
		}
	}

	log.Printf("done — egress=%s city=%s country=%s asn=%s satlink_range=%v",
		result.EgressIP, result.City, result.Country, result.ASN, result.InSatlinkRange)
	for _, m := range result.Measurements {
		log.Printf("  %s → ping=%.1fms(p95=%.1f jitter=%.1f) tcp=%.1fms hops=%d",
			m.Target, m.PingAvg, m.PingP95, m.PingJitter, m.TCPAvg, m.HopCount)
	}
}
