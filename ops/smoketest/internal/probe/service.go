package probe

import (
	"context"
	"crypto/tls"
	"crypto/x509"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/httptrace"
	"os"
	"strings"
	"time"
)

// ServiceCheck defines a single service health check.
type ServiceCheck struct {
	CAFile  string        `json:"ca_file,omitempty" yaml:"ca_file,omitempty"` // optional PEM trust roots for a private service
	Name    string        `json:"name" yaml:"name"`
	Type    string        `json:"type" yaml:"type"`       // "http", "https", or "tcp"
	URL     string        `json:"url" yaml:"url"`         // for http/https checks
	Host    string        `json:"host" yaml:"host"`       // for tcp checks
	Port    int           `json:"port" yaml:"port"`       // for tcp checks
	Timeout time.Duration `json:"timeout" yaml:"timeout"` // zero means 10s default
	Expect  ServiceExpect `json:"expect" yaml:"expect"`
}

// ServiceExpect defines what constitutes a passing service check.
type ServiceExpect struct {
	Status       int    `json:"status" yaml:"status"`               // expected HTTP status code (0 = any 2xx)
	BodyContains string `json:"body_contains" yaml:"body_contains"` // substring the body must contain
}

// ServiceProbe checks a catalogue of service endpoints.
type ServiceProbe struct {
	Checks []ServiceCheck
}

// globalServiceProbe is the singleton registered at init time.
var globalServiceProbe = &ServiceProbe{}

func init() {
	Register(globalServiceProbe)
}

// SetServiceChecks updates the global service probe's check list.
// Call this after loading the service catalogue.
func SetServiceChecks(checks []ServiceCheck) {
	globalServiceProbe.Checks = checks
}

func (p *ServiceProbe) Category() Category { return CatService }

func (p *ServiceProbe) Run(ctx context.Context, cfg *Config) []Result {
	if len(p.Checks) == 0 {
		return []Result{{
			Category:  CatService,
			Name:      "service-catalogue",
			Status:    Skip,
			Detail:    "no service checks configured",
			Timestamp: time.Now().UTC(),
		}}
	}

	var results []Result
	for _, check := range p.Checks {
		switch strings.ToLower(check.Type) {
		case "http", "https":
			results = append(results, p.checkHTTP(ctx, check))
		case "tcp":
			results = append(results, p.checkTCP(ctx, check))
		case "note":
			results = append(results, Result{
				Category:  CatService,
				Name:      fmt.Sprintf("svc-%s", check.Name),
				Status:    Pass,
				Detail:    check.URL, // URL field carries the display message for note checks
				Timestamp: time.Now().UTC(),
			})
		default:
			results = append(results, Result{
				Category:  CatService,
				Name:      fmt.Sprintf("svc-%s", check.Name),
				Status:    Fail,
				Detail:    fmt.Sprintf("unknown check type: %q", check.Type),
				Timestamp: time.Now().UTC(),
			})
		}
	}
	return results
}

func (p *ServiceProbe) checkHTTP(ctx context.Context, check ServiceCheck) Result {
	timeout := check.Timeout
	if timeout == 0 {
		timeout = 10 * time.Second
	}

	tlsConfig := &tls.Config{MinVersion: tls.VersionTLS12}
	if check.CAFile != "" {
		roots, err := x509.SystemCertPool()
		if err != nil {
			roots = x509.NewCertPool()
		}
		pem, err := os.ReadFile(check.CAFile)
		if err != nil || !roots.AppendCertsFromPEM(pem) {
			return Result{Category: CatService, Name: "svc-" + check.Name, Status: Fail,
				Detail: "cannot load configured service CA", Timestamp: time.Now().UTC()}
		}
		tlsConfig.RootCAs = roots
	}
	client := &http.Client{
		Timeout: timeout,
		Transport: &http.Transport{
			// Health includes server identity; inspection probes are separate.
			TLSClientConfig: tlsConfig,
		},
	}

	req, err := http.NewRequestWithContext(ctx, http.MethodGet, check.URL, nil)
	if err != nil {
		return Result{
			Category:  CatService,
			Name:      fmt.Sprintf("svc-%s", check.Name),
			Status:    Fail,
			Detail:    fmt.Sprintf("invalid request: %v", err),
			Timestamp: time.Now().UTC(),
		}
	}

	// httptrace timing points
	var t0, tDNSStart, tDNSDone, tConnect, tTLSStart, tTLSDone, tFirstByte time.Time

	trace := &httptrace.ClientTrace{
		DNSStart:             func(_ httptrace.DNSStartInfo) { tDNSStart = time.Now() },
		DNSDone:              func(_ httptrace.DNSDoneInfo) { tDNSDone = time.Now() },
		ConnectStart:         func(_, _ string) { tConnect = time.Now() },
		ConnectDone:          func(_, _ string, _ error) {},
		TLSHandshakeStart:    func() { tTLSStart = time.Now() },
		TLSHandshakeDone:     func(_ tls.ConnectionState, _ error) { tTLSDone = time.Now() },
		GotFirstResponseByte: func() { tFirstByte = time.Now() },
	}
	req = req.WithContext(httptrace.WithClientTrace(req.Context(), trace))

	t0 = time.Now()
	resp, err := client.Do(req)
	elapsed := time.Since(t0)

	if err != nil {
		return Result{
			Category:  CatService,
			Name:      fmt.Sprintf("svc-%s", check.Name),
			Status:    Fail,
			Detail:    fmt.Sprintf("GET %s: %v", check.URL, err),
			Timestamp: time.Now().UTC(),
		}
	}
	defer func() {
		io.Copy(io.Discard, resp.Body)
		resp.Body.Close()
	}()

	// Build phase timing metadata.
	phases := map[string]any{}
	if !tDNSStart.IsZero() && !tDNSDone.IsZero() {
		phases["dns_ms"] = float64(tDNSDone.Sub(tDNSStart).Microseconds()) / 1000
	}
	if !tConnect.IsZero() && !tTLSStart.IsZero() {
		phases["tcp_ms"] = float64(tTLSStart.Sub(tConnect).Microseconds()) / 1000
	} else if !tConnect.IsZero() && !tFirstByte.IsZero() {
		// No TLS — TCP to first byte includes server processing.
		phases["tcp_ms"] = float64(tFirstByte.Sub(tConnect).Microseconds()) / 1000
	}
	if !tTLSStart.IsZero() && !tTLSDone.IsZero() {
		phases["tls_ms"] = float64(tTLSDone.Sub(tTLSStart).Microseconds()) / 1000
	}
	if !tTLSDone.IsZero() && !tFirstByte.IsZero() {
		phases["ttfb_ms"] = float64(tFirstByte.Sub(tTLSDone).Microseconds()) / 1000
	} else if !tConnect.IsZero() && !tFirstByte.IsZero() && tTLSStart.IsZero() {
		phases["ttfb_ms"] = float64(tFirstByte.Sub(tConnect).Microseconds()) / 1000
	}
	phases["total_ms"] = float64(elapsed.Microseconds()) / 1000

	mkResult := func(status Status, detail string) Result {
		return Result{
			Category:  CatService,
			Name:      fmt.Sprintf("svc-%s", check.Name),
			Status:    status,
			Detail:    detail,
			LatencyMs: float64(elapsed.Microseconds()) / 1000,
			Timestamp: time.Now().UTC(),
			Metadata:  phases,
		}
	}

	// Check status code.
	expectedStatus := check.Expect.Status
	if expectedStatus == 0 {
		if resp.StatusCode < 200 || resp.StatusCode >= 300 {
			return mkResult(Fail, fmt.Sprintf("GET %s: status %d (expected 2xx)", check.URL, resp.StatusCode))
		}
	} else if resp.StatusCode != expectedStatus {
		return mkResult(Fail, fmt.Sprintf("GET %s: status %d (expected %d)", check.URL, resp.StatusCode, expectedStatus))
	}

	// Check body contents if required.
	if check.Expect.BodyContains != "" {
		body, err := io.ReadAll(io.LimitReader(resp.Body, 1<<20))
		if err != nil {
			return mkResult(Fail, fmt.Sprintf("GET %s: read body: %v", check.URL, err))
		}
		if !strings.Contains(string(body), check.Expect.BodyContains) {
			return mkResult(Fail, fmt.Sprintf("GET %s: body missing %q", check.URL, check.Expect.BodyContains))
		}
	}

	return mkResult(Pass, fmt.Sprintf("GET %s: %d", check.URL, resp.StatusCode))
}

func (p *ServiceProbe) checkTCP(ctx context.Context, check ServiceCheck) Result {
	timeout := check.Timeout
	if timeout == 0 {
		timeout = 10 * time.Second
	}

	addr := net.JoinHostPort(check.Host, fmt.Sprintf("%d", check.Port))
	d := net.Dialer{Timeout: timeout}

	start := time.Now()
	conn, err := d.DialContext(ctx, "tcp", addr)
	elapsed := time.Since(start)

	if err != nil {
		return Result{
			Category:  CatService,
			Name:      fmt.Sprintf("svc-%s", check.Name),
			Status:    Fail,
			Detail:    fmt.Sprintf("TCP %s: %v", addr, err),
			Timestamp: time.Now().UTC(),
		}
	}
	conn.Close()

	return Result{
		Category:  CatService,
		Name:      fmt.Sprintf("svc-%s", check.Name),
		Status:    Pass,
		Detail:    fmt.Sprintf("TCP %s connected", addr),
		LatencyMs: float64(elapsed.Milliseconds()),
		Timestamp: time.Now().UTC(),
	}
}
