package measure

import (
	"context"
	"crypto/tls"
	"encoding/json"
	"fmt"
	"log"
	"math"
	"net"
	"net/http"
	"net/http/httptrace"
	"os/exec"
	"sort"
	"strings"
	"time"

	"github.com/usareur-af/goatcore/ops/snitch/internal/types"
)

const (
	satlinkPingCount = 50
	satlinkTCPCount  = 20
)

// Satellite-link provider egress ranges.
var satlinkCIDRs []*net.IPNet

func init() {
	for _, cidr := range []string{"216.255.176.0/20", "75.78.128.0/17"} { // oss-allow: Starlink's published egress ranges, a public fact
		_, n, err := net.ParseCIDR(cidr)
		if err != nil {
			panic("invalid built-in satellite-link CIDR: " + cidr)
		}
		satlinkCIDRs = append(satlinkCIDRs, n)
	}
}

// RunSatlinkProbe measures network performance from behind the satellite link
// to each relay VPS, identifying the egress PoP (ground station).
func RunSatlinkProbe(ctx context.Context) (*types.SatlinkProbeResult, error) {
	result := &types.SatlinkProbeResult{
		Timestamp: time.Now().Unix(),
	}

	log.Println("satlink: detecting egress PoP...")
	detectEgressGeo(ctx, result)
	log.Printf("satlink: egress=%s city=%s asn=%s satlink=%v",
		result.EgressIP, result.City, result.ASN, result.InSatlinkRange)

	for _, t := range types.DefaultSatlinkTargets {
		log.Printf("satlink: measuring %s (%s)...", t.Name, t.IP)
		m := measureSatlinkTarget(ctx, t)
		log.Printf("satlink: %s ping=%.1fms tcp=%.1fms hops=%d",
			t.Name, m.PingAvg, m.TCPAvg, m.HopCount)
		result.Measurements = append(result.Measurements, m)
	}

	return result, nil
}

// detectEgressGeo queries ipinfo.io for full egress identity and PoP location.
func detectEgressGeo(ctx context.Context, result *types.SatlinkProbeResult) {
	ctx, cancel := context.WithTimeout(ctx, 10*time.Second)
	defer cancel()

	req, err := http.NewRequestWithContext(ctx, "GET", "https://ipinfo.io/json", nil)
	if err != nil {
		detectEgressFallback(ctx, result)
		return
	}

	resp, err := (&http.Client{Timeout: 10 * time.Second}).Do(req)
	if err != nil {
		detectEgressFallback(ctx, result)
		return
	}
	defer resp.Body.Close()

	var info struct {
		IP      string `json:"ip"`
		City    string `json:"city"`
		Region  string `json:"region"`
		Country string `json:"country"`
		Org     string `json:"org"`
	}
	if err := json.NewDecoder(resp.Body).Decode(&info); err != nil {
		detectEgressFallback(ctx, result)
		return
	}

	result.EgressIP = info.IP
	result.City = info.City
	result.Region = info.Region
	result.Country = info.Country

	// Org field is "AS14593 SpaceX Services, Inc." — split ASN from name.
	if parts := strings.Fields(info.Org); len(parts) > 0 {
		result.ASN = parts[0]
		if len(parts) > 1 {
			result.Org = strings.Join(parts[1:], " ")
		}
	}

	checkSatlinkRange(result)
}

func detectEgressFallback(ctx context.Context, result *types.SatlinkProbeResult) {
	ip, asn, org := detectPublicIP(ctx)
	result.EgressIP = ip
	result.ASN = asn
	result.Org = org
	checkSatlinkRange(result)
}

func checkSatlinkRange(result *types.SatlinkProbeResult) {
	ip := net.ParseIP(result.EgressIP)
	if ip == nil {
		return
	}
	for _, cidr := range satlinkCIDRs {
		if cidr.Contains(ip) {
			result.InSatlinkRange = true
			return
		}
	}
}

func measureSatlinkTarget(ctx context.Context, t types.SatlinkTarget) types.SatlinkMeasurement {
	m := types.SatlinkMeasurement{
		Target:   t.Name,
		TargetIP: t.IP,
	}

	// ICMP ping (50 packets for meaningful jitter stats).
	if pr, err := Ping(ctx, t.IP, satlinkPingCount); err == nil {
		m.PingAvg = pr.AvgMs
		m.PingP50 = pr.P50Ms
		m.PingP95 = pr.P95Ms
		m.PingJitter = pr.JitterMs
		m.PingLoss = pr.LossPct
	} else {
		log.Printf("satlink: ping %s failed: %v", t.Name, err)
	}

	// TCP connect timing (20 iterations to port 443).
	if avg, p50, p95, jitter, err := tcpConnectStats(ctx, t.IP, 443, satlinkTCPCount); err == nil {
		m.TCPAvg = avg
		m.TCPP50 = p50
		m.TCPP95 = p95
		m.TCPJitter = jitter
	} else {
		log.Printf("satlink: tcp %s failed: %v", t.Name, err)
	}

	// HTTP timing (TLS handshake, first byte).
	m.HTTPConnect, m.HTTPTLS, m.HTTPFirstByte = httpTimingMeasure(ctx, t.IP)

	// Traceroute (optional, skipped if not installed).
	m.HopCount, m.Traceroute = runTraceroute(ctx, t.IP)

	return m
}

// tcpConnectStats opens N TCP connections and computes latency statistics.
func tcpConnectStats(ctx context.Context, ip string, port, count int) (avg, p50, p95, jitter float64, err error) {
	addr := fmt.Sprintf("%s:%d", ip, port)
	var samples []float64

	for i := 0; i < count; i++ {
		start := time.Now()
		d := net.Dialer{Timeout: 5 * time.Second}
		conn, dialErr := d.DialContext(ctx, "tcp", addr)
		elapsed := time.Since(start)

		if dialErr != nil {
			continue
		}
		conn.Close()
		samples = append(samples, float64(elapsed.Microseconds())/1000.0)
	}

	if len(samples) == 0 {
		return 0, 0, 0, 0, fmt.Errorf("all %d TCP connects to %s failed", count, addr)
	}

	sort.Float64s(samples)
	n := len(samples)
	sum := 0.0
	for _, v := range samples {
		sum += v
	}
	avg = sum / float64(n)

	jitterSum := 0.0
	for _, v := range samples {
		jitterSum += math.Abs(v - avg)
	}
	jitter = jitterSum / float64(n)

	p50 = percentile(samples, 50)
	p95 = percentile(samples, 95)
	return avg, p50, p95, jitter, nil
}

// httpTimingMeasure uses httptrace to measure TCP connect, TLS handshake,
// and time to first byte for a single HTTPS request.
func httpTimingMeasure(ctx context.Context, ip string) (connectMs, tlsMs, firstbyteMs float64) {
	ctx, cancel := context.WithTimeout(ctx, 15*time.Second)
	defer cancel()

	var connectStart, connectDone time.Time
	var tlsStart, tlsDone time.Time
	var gotFirstByte time.Time

	trace := &httptrace.ClientTrace{
		ConnectStart:         func(_, _ string) { connectStart = time.Now() },
		ConnectDone:          func(_, _ string, _ error) { connectDone = time.Now() },
		TLSHandshakeStart:    func() { tlsStart = time.Now() },
		TLSHandshakeDone:     func(_ tls.ConnectionState, _ error) { tlsDone = time.Now() },
		GotFirstResponseByte: func() { gotFirstByte = time.Now() },
	}

	url := fmt.Sprintf("https://%s/", ip)
	req, err := http.NewRequestWithContext(httptrace.WithClientTrace(ctx, trace), "GET", url, nil)
	if err != nil {
		return 0, 0, 0
	}

	// gosec G402 false-positive: this satellite-link characterization probe
	// dials the gateway IP directly (no DNS); cert verification is
	// off because the gateway terminal presents a self-signed cert and
	// we are NOT establishing a trusted channel — we're measuring whether
	// it answers + how fast. Pinned MinVersion 1.2 narrows the surface.
	client := &http.Client{
		Timeout: 15 * time.Second,
		Transport: &http.Transport{
			TLSClientConfig: &tls.Config{InsecureSkipVerify: true, MinVersion: tls.VersionTLS12}, //nolint:gosec // see comment above
		},
	}

	resp, err := client.Do(req)
	if err != nil {
		// Even on HTTP error, timing hooks may have fired.
		_ = err
	}
	if resp != nil {
		resp.Body.Close()
	}

	if !connectStart.IsZero() && !connectDone.IsZero() {
		connectMs = float64(connectDone.Sub(connectStart).Microseconds()) / 1000.0
	}
	if !tlsStart.IsZero() && !tlsDone.IsZero() {
		tlsMs = float64(tlsDone.Sub(tlsStart).Microseconds()) / 1000.0
	}
	if !connectStart.IsZero() && !gotFirstByte.IsZero() {
		firstbyteMs = float64(gotFirstByte.Sub(connectStart).Microseconds()) / 1000.0
	}
	return
}

// runTraceroute executes traceroute and returns hop count + raw output.
func runTraceroute(ctx context.Context, ip string) (int, string) {
	ctx, cancel := context.WithTimeout(ctx, 30*time.Second)
	defer cancel()

	// gosec G204 false-positive: `ip` is the gateway IP from the local
	// route table; bounded args; no shell.
	cmd := exec.CommandContext(ctx, "traceroute", "-m", "30", "-w", "3", ip) //nolint:gosec // see comment above
	out, err := cmd.CombinedOutput()
	if err != nil {
		// Traceroute not installed or failed — non-fatal.
		return 0, ""
	}

	output := string(out)
	hops := 0
	for _, line := range strings.Split(output, "\n") {
		trimmed := strings.TrimSpace(line)
		if trimmed != "" && !strings.HasPrefix(trimmed, "traceroute") {
			hops++
		}
	}
	return hops, output
}
