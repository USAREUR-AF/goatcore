package measure

import (
	"context"
	"fmt"
	"io"
	"net"
	"net/http"
	"os/exec"
	"regexp"
	"runtime"
	"strings"
	"time"

	"github.com/usareur-af/goatcore/ops/snitch/internal/types"
)

// DetectGateway discovers gateway hardware info: MAC, OUI manufacturer, subnet, HTTP banner.
func DetectGateway(ctx context.Context, gatewayIP string) types.GatewayInfo {
	if gatewayIP == "" {
		return types.GatewayInfo{}
	}

	info := types.GatewayInfo{}

	// MAC + OUI (passive — reads ARP cache)
	info.MAC = lookupARP(ctx, gatewayIP)
	if info.MAC != "" {
		info.Manufacturer = ouiLookup(info.MAC)
	}

	// HTTP banner (active — probes gateway web UI)
	info.HTTPBanner, info.Model = probeHTTPBanner(ctx, gatewayIP)

	// mDNS model discovery (active — queries local network)
	if info.Model == "" {
		info.Model = probeMDNS(ctx, gatewayIP)
	}

	return info
}

// DetectSubnet returns the local subnet CIDR for the given interface.
func DetectSubnet(ctx context.Context, ifaceName string) string {
	iface, err := net.InterfaceByName(ifaceName)
	if err != nil {
		return ""
	}
	addrs, err := iface.Addrs()
	if err != nil {
		return ""
	}
	for _, addr := range addrs {
		ipNet, ok := addr.(*net.IPNet)
		if !ok {
			continue
		}
		ip4 := ipNet.IP.To4()
		if ip4 == nil {
			continue
		}
		// Mask to network address
		network := ip4.Mask(ipNet.Mask)
		ones, _ := ipNet.Mask.Size()
		return fmt.Sprintf("%s/%d", network, ones)
	}
	return ""
}

// lookupARP reads the gateway MAC from the system ARP cache.
// This is passive — no packets sent; we already pinged the gateway earlier.
func lookupARP(ctx context.Context, ip string) string {
	// gosec G204 false-positive: `ip` is a string parsed out of the local
	// kernel routing table (default-route gateway) — never a network or
	// operator surface. Passed as a separate argv element to arp; no
	// shell metacharacter expansion possible.
	var cmd *exec.Cmd
	switch runtime.GOOS {
	case "darwin":
		cmd = exec.CommandContext(ctx, "arp", "-n", ip) //nolint:gosec // see comment above
	case "windows":
		// Windows arp -a <ip> emits MACs in dash-separated form
		// "00-1a-dd-e3-4f-50". The regex below handles both ":" and
		// "-" separators after normalization.
		cmd = exec.CommandContext(ctx, "arp", "-a", ip) //nolint:gosec // see comment above
	default:
		cmd = exec.CommandContext(ctx, "ip", "neigh", "show", ip) //nolint:gosec // see comment above
	}
	out, err := cmd.Output()
	if err != nil {
		return ""
	}
	return parseMACFromARP(string(out))
}

// normalizeMac pads single-digit hex octets and converts dash separators
// to colons: "0-1A-2:3:4:5" → "00:1a:02:03:04:05". Cross-platform: Linux  // opsec-allow: synthetic example MAC
// + Darwin emit colon-separated MACs; Windows arp -a emits dash-separated.
func normalizeMac(mac string) string {
	mac = strings.ReplaceAll(mac, "-", ":")
	parts := strings.Split(mac, ":")
	for i, p := range parts {
		if len(p) == 1 {
			parts[i] = "0" + p
		}
		parts[i] = strings.ToLower(parts[i])
	}
	return strings.Join(parts, ":")
}

// ouiLookup maps a MAC prefix to manufacturer using a built-in table
// of common networking equipment vendors seen in tactical/field environments.
func ouiLookup(mac string) string {
	if len(mac) < 8 {
		return ""
	}
	prefix := strings.ToLower(mac[:8]) // "aa:bb:cc"

	// Common OUI prefixes for networking gear likely encountered.
	// Format: first 3 octets → manufacturer name.
	oui := map[string]string{
		// Peplink / Pepwave
		"00:1a:dd": "Peplink",
		// GL.iNet
		"94:83:c4": "GL.iNet",
		"e4:95:6e": "GL.iNet",
		// Teltonika
		"00:1e:42": "Teltonika",
		// Cisco / Meraki
		"00:18:0a": "Cisco",
		"00:1b:0d": "Cisco",
		"00:1c:57": "Cisco",
		"00:23:04": "Cisco",
		"00:25:45": "Cisco",
		"00:26:0b": "Cisco",
		"d4:ad:71": "Cisco Meraki",
		"0c:8d:db": "Cisco Meraki",
		"88:15:44": "Cisco Meraki",
		// Ubiquiti
		"04:18:d6": "Ubiquiti",
		"24:5a:4c": "Ubiquiti",
		"68:72:51": "Ubiquiti",
		"78:8a:20": "Ubiquiti",
		"b4:fb:e4": "Ubiquiti",
		"f0:9f:c2": "Ubiquiti",
		"fc:ec:da": "Ubiquiti",
		// MikroTik
		"00:0c:42": "MikroTik",
		"48:8f:5a": "MikroTik",
		"64:d1:54": "MikroTik",
		"cc:2d:e0": "MikroTik",
		// Netgear
		"00:26:f2": "Netgear",
		"08:36:c9": "Netgear",
		"20:0d:b0": "Netgear",
		"44:94:fc": "Netgear",
		"6c:b0:ce": "Netgear",
		// TP-Link
		"50:c7:bf": "TP-Link",
		"60:32:b1": "TP-Link",
		"94:d9:b3": "TP-Link",
		"c0:06:c3": "TP-Link",
		// Cradlepoint
		"00:04:2d": "Cradlepoint",
		"00:09:18": "Cradlepoint",
		// Sierra Wireless
		"00:14:3e": "Sierra Wireless",
		// Intel (NETHOST mini PCs use Intel NICs)
		"a0:36:9f": "Intel",
		"3c:fd:fe": "Intel",
		"68:05:ca": "Intel",
		// Realtek (common in mini PCs)
		"00:e0:4c": "Realtek",
		"52:54:00": "QEMU/KVM",
		// SpaceX Starlink
		"74:24:9f": "SpaceX Starlink",
		"2c:d0:66": "SpaceX Starlink",
		// Apple (for dev/admin machines)
		"3c:22:fb": "Apple",
		"a4:83:e7": "Apple",
		"f0:18:98": "Apple",
	}

	if mfg, ok := oui[prefix]; ok {
		return mfg
	}
	return ""
}

// probeHTTPBanner tries to grab the gateway's web UI title or server header.
// Quick, best-effort — 2s timeout, follows one redirect max.
func probeHTTPBanner(ctx context.Context, ip string) (banner, model string) {
	ctx, cancel := context.WithTimeout(ctx, 2*time.Second)
	defer cancel()

	for _, scheme := range []string{"http", "https"} {
		url := fmt.Sprintf("%s://%s/", scheme, ip)
		req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
		if err != nil {
			continue
		}
		client := &http.Client{
			Timeout: 2 * time.Second,
			CheckRedirect: func(req *http.Request, via []*http.Request) error {
				if len(via) >= 1 {
					return http.ErrUseLastResponse
				}
				return nil
			},
			Transport: &http.Transport{
				TLSHandshakeTimeout: 2 * time.Second,
				// Accept self-signed certs on gateway web UIs
				TLSClientConfig: nil, // will use default but we don't verify
			},
		}
		resp, err := client.Do(req)
		if err != nil {
			continue
		}
		defer resp.Body.Close()

		// Server header
		server := resp.Header.Get("Server")
		if server != "" {
			banner = server
		}

		// Try to extract <title> from HTML body (first 4KB only)
		body := make([]byte, 4096)
		n, _ := io.ReadFull(resp.Body, body) //nolint:errcheck // EOF is expected when the bounded banner is shorter than 4 KiB
		html := string(body[:n])

		titleRe := regexp.MustCompile(`(?i)<title[^>]*>([^<]+)</title>`)
		if m := titleRe.FindStringSubmatch(html); len(m) > 1 {
			title := strings.TrimSpace(m[1])
			if banner == "" {
				banner = title
			}
			model = inferModelFromTitle(title)
		}

		if banner != "" {
			return banner, model
		}
	}
	return "", ""
}

// inferModelFromTitle tries to extract a hardware model from gateway web UI title.
func inferModelFromTitle(title string) string {
	lower := strings.ToLower(title)

	// GL.iNet models: "GL-MT3000", "GL-AXT1800", etc.
	glRe := regexp.MustCompile(`(?i)(GL-[A-Z0-9]+)`)
	if m := glRe.FindString(title); m != "" {
		return m
	}

	// Peplink: "Peplink Balance 20X", "MAX BR1 Pro", etc.
	if strings.Contains(lower, "peplink") || strings.Contains(lower, "pepwave") {
		return strings.TrimSpace(title)
	}

	// Teltonika: "RUTX11", "RUT950", etc.
	telRe := regexp.MustCompile(`(?i)(RUT[A-Z]?\d+)`)
	if m := telRe.FindString(title); m != "" {
		return m
	}

	// Ubiquiti: "UniFi Dream Machine", "EdgeRouter X", etc.
	if strings.Contains(lower, "unifi") || strings.Contains(lower, "ubiquiti") || strings.Contains(lower, "edgerouter") {
		return strings.TrimSpace(title)
	}

	// Cradlepoint: "IBR900", "E3000", etc.
	cradRe := regexp.MustCompile(`(?i)(IBR\d+|E\d{4}|R\d{4})`)
	if m := cradRe.FindString(title); m != "" {
		return m
	}

	return ""
}

// probeMDNS queries for _http._tcp services and looks for the gateway.
// Best-effort: dns-sd on Darwin, avahi-browse on Linux, skipped on Windows
// (Bonjour is optional and rarely present on field laptops).
func probeMDNS(ctx context.Context, gatewayIP string) string {
	ctx, cancel := context.WithTimeout(ctx, 2*time.Second)
	defer cancel()

	var cmd *exec.Cmd
	switch runtime.GOOS {
	case "darwin":
		// dns-sd is macOS-native but blocks; use timeout context
		cmd = exec.CommandContext(ctx, "dns-sd", "-B", "_http._tcp", "local")
	case "windows":
		// Windows ships no built-in mDNS browser. Apple Bonjour Print
		// Services adds dns-sd.exe but is rarely on field machines.
		// Skip rather than guess.
		return ""
	default:
		cmd = exec.CommandContext(ctx, "avahi-browse", "-t", "-r", "_http._tcp", "-p")
	}
	out, _ := cmd.Output() //nolint:errcheck // optional discovery failure is represented by an empty result
	if len(out) == 0 {
		return ""
	}

	// Look for gateway-like service names
	output := string(out)
	for _, keyword := range []string{"router", "gateway", "gl-", "peplink", "teltonika", "ubiquiti", "cradlepoint"} {
		if idx := strings.Index(strings.ToLower(output), keyword); idx >= 0 {
			// Extract the line containing the keyword
			lines := strings.Split(output, "\n")
			for _, line := range lines {
				if strings.Contains(strings.ToLower(line), keyword) {
					return strings.TrimSpace(line)
				}
			}
		}
	}
	return ""
}
