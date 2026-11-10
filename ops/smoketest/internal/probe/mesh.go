package probe

import (
	"context"
	"fmt"
	"os/exec"
	"regexp"
	"strconv"
	"strings"
	"time"
)

// MeshProbe checks Netbird mesh status and ICMP reachability to the mesh host.
// Only runs in "full" mode.
type MeshProbe struct{}

func init() {
	Register(&MeshProbe{})
}

func (p *MeshProbe) Category() Category { return CatMesh }

func (p *MeshProbe) Run(ctx context.Context, cfg *Config) []Result {
	if cfg.Mode != "full" {
		return []Result{{
			Category:  CatMesh,
			Name:      "mesh-status",
			Status:    Skip,
			Detail:    "skipped in quick mode",
			Timestamp: time.Now().UTC(),
		}}
	}

	var results []Result
	results = append(results, p.checkNetbirdStatus(ctx)...)
	results = append(results, p.checkICMP(ctx, cfg))
	return results
}

// checkNetbirdStatus runs `netbird status` and parses the output.
func (p *MeshProbe) checkNetbirdStatus(ctx context.Context) []Result {
	cmd := exec.CommandContext(ctx, "netbird", "status")
	out, err := cmd.CombinedOutput()
	output := string(out)

	if err != nil {
		return []Result{{
			Category:  CatMesh,
			Name:      "netbird-status",
			Status:    Fail,
			Detail:    fmt.Sprintf("netbird status failed: %v", err),
			Timestamp: time.Now().UTC(),
		}}
	}

	var results []Result

	// Parse connection state (e.g., "Status: Connected").
	state := "unknown"
	for _, line := range strings.Split(output, "\n") {
		trimmed := strings.TrimSpace(line)
		if strings.HasPrefix(trimmed, "Status:") {
			state = strings.TrimSpace(strings.TrimPrefix(trimmed, "Status:"))
			break
		}
	}

	if strings.EqualFold(state, "Connected") {
		results = append(results, Result{
			Category:  CatMesh,
			Name:      "netbird-status",
			Status:    Pass,
			Detail:    fmt.Sprintf("management status: %s", state),
			Timestamp: time.Now().UTC(),
		})
	} else {
		results = append(results, Result{
			Category:  CatMesh,
			Name:      "netbird-status",
			Status:    Fail,
			Detail:    fmt.Sprintf("management status: %s", state),
			Timestamp: time.Now().UTC(),
		})
	}

	// Count peers. Look for patterns like "Peers count: 5/5 Connected" or
	// individual peer entries. The format varies by version so we try a few.
	totalPeers, connectedPeers := parsePeerCounts(output)
	if totalPeers >= 0 {
		status := Pass
		detail := fmt.Sprintf("%d/%d peers connected", connectedPeers, totalPeers)
		if connectedPeers < totalPeers {
			status = Warn
		}
		if connectedPeers == 0 && totalPeers > 0 {
			status = Fail
		}
		results = append(results, Result{
			Category:  CatMesh,
			Name:      "netbird-peers",
			Status:    status,
			Detail:    detail,
			Timestamp: time.Now().UTC(),
			Metadata: map[string]any{
				"total":     totalPeers,
				"connected": connectedPeers,
			},
		})
	}

	return results
}

// parsePeerCounts extracts total and connected peer counts from netbird status output.
func parsePeerCounts(output string) (total, connected int) {
	// Try "Peers count: N/M Connected" format.
	re := regexp.MustCompile(`(?i)peers\s+count:\s*(\d+)/(\d+)`)
	if m := re.FindStringSubmatch(output); m != nil {
		connected, _ = strconv.Atoi(m[1])
		total, _ = strconv.Atoi(m[2])
		return total, connected
	}

	// Fall back: count individual "Connected" and "Disconnected" lines under Peers section.
	lines := strings.Split(output, "\n")
	inPeers := false
	total = 0
	connected = 0
	for _, line := range lines {
		trimmed := strings.TrimSpace(line)
		if strings.HasPrefix(trimmed, "Peers:") || strings.HasPrefix(trimmed, "Peers ") {
			inPeers = true
			continue
		}
		if !inPeers {
			continue
		}
		lower := strings.ToLower(trimmed)
		if strings.Contains(lower, "status:") {
			total++
			if strings.Contains(lower, "connected") && !strings.Contains(lower, "disconnected") {
				connected++
			}
		}
	}

	if total == 0 {
		return -1, 0
	}
	return total, connected
}

// checkICMP pings the mesh host's IP.
func (p *MeshProbe) checkICMP(ctx context.Context, cfg *Config) Result {
	if cfg.HostMeshIP == "" {
		return Result{
			Category:  CatMesh,
			Name:      "icmp-mesh-host",
			Status:    Skip,
			Detail:    "no mesh host IP configured",
			Timestamp: time.Now().UTC(),
		}
	}

	start := time.Now()
	cmd := exec.CommandContext(ctx, "ping", "-c", "1", "-W", "3", cfg.HostMeshIP)
	out, err := cmd.CombinedOutput()
	elapsed := time.Since(start)

	if err != nil {
		return Result{
			Category:  CatMesh,
			Name:      "icmp-mesh-host",
			Status:    Fail,
			Detail:    fmt.Sprintf("ping %s failed: %v", cfg.HostMeshIP, err),
			Timestamp: time.Now().UTC(),
		}
	}

	// Extract RTT from ping output (e.g., "time=12.3 ms").
	rttMs := elapsed.Seconds() * 1000
	output := string(out)
	re := regexp.MustCompile(`time[=<](\d+\.?\d*)\s*ms`)
	if m := re.FindStringSubmatch(output); m != nil {
		if parsed, err := strconv.ParseFloat(m[1], 64); err == nil {
			rttMs = parsed
		}
	}

	return Result{
		Category:  CatMesh,
		Name:      "icmp-mesh-host",
		Status:    Pass,
		Detail:    fmt.Sprintf("ping %s: %.1fms", cfg.HostMeshIP, rttMs),
		LatencyMs: rttMs,
		Timestamp: time.Now().UTC(),
	}
}
