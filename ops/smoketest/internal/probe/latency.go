package probe

import (
	"context"
	"fmt"
	"math"
	"net"
	"os/exec"
	"regexp"
	"sort"
	"strconv"
	"time"
)

const defaultIterations = 5

// LatencyProbe measures TCP connect RTT to each relay and optionally ICMP RTT
// to the mesh host's IP, computing percentile statistics.
type LatencyProbe struct{}

func init() {
	Register(&LatencyProbe{})
}

func (p *LatencyProbe) Category() Category { return CatLatency }

func (p *LatencyProbe) Run(ctx context.Context, cfg *Config) []Result {
	var results []Result

	for _, relay := range cfg.Relays {
		results = append(results, p.measureTCP(ctx, relay))
	}

	// ICMP to the mesh host only in full mode.
	if cfg.Mode == "full" && cfg.HostMeshIP != "" {
		results = append(results, p.measureICMP(ctx, cfg.HostMeshIP))
	}

	return results
}

// measureTCP opens N TCP connections to relay:443 and computes latency stats.
func (p *LatencyProbe) measureTCP(ctx context.Context, relay RelayInfo) Result {
	addr := net.JoinHostPort(relay.IP, "443")
	var samples []float64

	for i := 0; i < defaultIterations; i++ {
		start := time.Now()
		d := net.Dialer{Timeout: 5 * time.Second}
		conn, err := d.DialContext(ctx, "tcp", addr)
		elapsed := time.Since(start)

		if err != nil {
			// Record this iteration as a failure but keep going.
			continue
		}
		conn.Close()
		samples = append(samples, float64(elapsed.Microseconds())/1000.0)
	}

	if len(samples) == 0 {
		return Result{
			Category:  CatLatency,
			Name:      fmt.Sprintf("tcp-latency-%s", relay.Name),
			Status:    Fail,
			Detail:    fmt.Sprintf("all %d TCP connects to %s failed", defaultIterations, addr),
			Timestamp: time.Now().UTC(),
		}
	}

	stats := computeStats(samples)

	return Result{
		Category:  CatLatency,
		Name:      fmt.Sprintf("tcp-latency-%s", relay.Name),
		Status:    Pass,
		Detail:    fmt.Sprintf("avg=%.1fms p50=%.1fms p95=%.1fms (%d/%d ok)", stats["avg"], stats["p50"], stats["p95"], len(samples), defaultIterations),
		LatencyMs: stats["avg"],
		Timestamp: time.Now().UTC(),
		Metadata: map[string]any{
			"min":        stats["min"],
			"avg":        stats["avg"],
			"p50":        stats["p50"],
			"p95":        stats["p95"],
			"p99":        stats["p99"],
			"jitter":     stats["jitter"],
			"samples":    len(samples),
			"iterations": defaultIterations,
		},
	}
}

// measureICMP runs ping and extracts per-packet RTTs for statistics.
func (p *LatencyProbe) measureICMP(ctx context.Context, ip string) Result {
	cmd := exec.CommandContext(ctx, "ping", "-c", fmt.Sprintf("%d", defaultIterations), "-W", "3", ip)
	out, err := cmd.CombinedOutput()

	if err != nil {
		return Result{
			Category:  CatLatency,
			Name:      "icmp-latency-mesh-host",
			Status:    Fail,
			Detail:    fmt.Sprintf("ping %s failed: %v", ip, err),
			Timestamp: time.Now().UTC(),
		}
	}

	// Extract all "time=X.X ms" values from ping output.
	re := regexp.MustCompile(`time[=<](\d+\.?\d*)\s*ms`)
	matches := re.FindAllStringSubmatch(string(out), -1)

	var samples []float64
	for _, m := range matches {
		v, err := strconv.ParseFloat(m[1], 64)
		if err == nil {
			samples = append(samples, v)
		}
	}

	if len(samples) == 0 {
		return Result{
			Category:  CatLatency,
			Name:      "icmp-latency-mesh-host",
			Status:    Fail,
			Detail:    fmt.Sprintf("no RTT values from ping %s", ip),
			Timestamp: time.Now().UTC(),
		}
	}

	stats := computeStats(samples)

	return Result{
		Category:  CatLatency,
		Name:      "icmp-latency-mesh-host",
		Status:    Pass,
		Detail:    fmt.Sprintf("avg=%.1fms p50=%.1fms p95=%.1fms (%d samples)", stats["avg"], stats["p50"], stats["p95"], len(samples)),
		LatencyMs: stats["avg"],
		Timestamp: time.Now().UTC(),
		Metadata: map[string]any{
			"min":    stats["min"],
			"avg":    stats["avg"],
			"p50":    stats["p50"],
			"p95":    stats["p95"],
			"p99":    stats["p99"],
			"jitter": stats["jitter"],
		},
	}
}

// computeStats returns min, avg, p50, p95, p99, and jitter for a set of
// latency samples (in milliseconds).
func computeStats(samples []float64) map[string]float64 {
	sort.Float64s(samples)
	n := len(samples)

	sum := 0.0
	for _, v := range samples {
		sum += v
	}
	avg := sum / float64(n)

	// Jitter = average absolute deviation from the mean.
	jitterSum := 0.0
	for _, v := range samples {
		jitterSum += math.Abs(v - avg)
	}
	jitter := jitterSum / float64(n)

	return map[string]float64{
		"min":    samples[0],
		"avg":    avg,
		"p50":    percentile(samples, 50),
		"p95":    percentile(samples, 95),
		"p99":    percentile(samples, 99),
		"jitter": jitter,
	}
}

// percentile returns the p-th percentile from sorted samples using nearest-rank.
func percentile(sorted []float64, p float64) float64 {
	if len(sorted) == 0 {
		return 0
	}
	if len(sorted) == 1 {
		return sorted[0]
	}
	rank := (p / 100.0) * float64(len(sorted)-1)
	lower := int(math.Floor(rank))
	upper := int(math.Ceil(rank))
	if lower == upper {
		return sorted[lower]
	}
	// Linear interpolation between the two nearest ranks.
	frac := rank - float64(lower)
	return sorted[lower]*(1-frac) + sorted[upper]*frac
}
