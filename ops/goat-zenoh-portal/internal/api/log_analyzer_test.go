package api

import (
	"strings"
	"testing"
)

// log_analyzer_test.go — one test fixture per signature in the catalog.
// CI guard: a signature landing without a fixture here fails the build.
// The fixtures are deliberately small — just enough Zenoh stderr context
// to make the match real, not a single keyword that would false-positive.

func TestAnalyzeLog_PeerModeListenerMissingCert(t *testing.T) {
	// Captured from a real Windows-WSL vendor publisher; the
	// failure shape that motivated this whole track.
	const sample = `INFO async_runtime starting
INFO async_runtime: connected
WARN net::runtime::orchestrator: Cannot create a new TLS listener tls/192.168.1.5:0
  caused by: No such file or directory (os error 2) at /usr/local/cargo/registry/src/index.crates.io-...
INFO close session zid=cf096241
`
	findings := analyzeLog(sample)
	if !hasFinding(findings, "peer-mode-listener-missing-cert") {
		t.Fatalf("expected peer-mode-listener-missing-cert finding, got %v", findingIDs(findings))
	}
}

func TestAnalyzeLog_ScoutMulticastNoConnect(t *testing.T) {
	const sample = `INFO listening scout messages on 224.0.0.224:7446
INFO starting net::runtime::orchestrator
`
	findings := analyzeLog(sample)
	if !hasFinding(findings, "scout-multicast-no-connect") {
		t.Fatalf("expected scout-multicast-no-connect, got %v", findingIDs(findings))
	}
}

func TestAnalyzeLog_TLSUnknownIssuer(t *testing.T) {
	const sample = `ERROR tls: handshake failed: invalid peer certificate: UnknownIssuer`
	findings := analyzeLog(sample)
	if !hasFinding(findings, "tls-handshake-cert-unknown-ca") {
		t.Fatalf("expected tls-handshake-cert-unknown-ca, got %v", findingIDs(findings))
	}
}

func TestAnalyzeLog_TLSHostnameMismatch(t *testing.T) {
	const sample = `ERROR tls: handshake failed: invalid peer certificate: NotValidForName`
	findings := analyzeLog(sample)
	if !hasFinding(findings, "tls-handshake-hostname-mismatch") {
		t.Fatalf("expected tls-handshake-hostname-mismatch, got %v", findingIDs(findings))
	}
}

func TestAnalyzeLog_ClientCertMissing(t *testing.T) {
	const sample = `ERROR transport handshake error: peer did not return a certificate`
	findings := analyzeLog(sample)
	if !hasFinding(findings, "tls-handshake-client-cert-missing") {
		t.Fatalf("expected tls-handshake-client-cert-missing, got %v", findingIDs(findings))
	}
}

func TestAnalyzeLog_URISANRejected(t *testing.T) {
	const sample = `ERROR access_control denied: URI-SAN prefix mismatch (cert says foo, attempted bar/**)`
	findings := analyzeLog(sample)
	if !hasFinding(findings, "tls-handshake-uri-san-rejected") {
		t.Fatalf("expected tls-handshake-uri-san-rejected, got %v", findingIDs(findings))
	}
}

func TestAnalyzeLog_DNSResolutionFailed(t *testing.T) {
	const sample = `ERROR failed to resolve zenoh.goat.example: name not known`
	findings := analyzeLog(sample)
	if !hasFinding(findings, "endpoint-dns-resolution-failed") {
		t.Fatalf("expected endpoint-dns-resolution-failed, got %v", findingIDs(findings))
	}
}

// The DNS signature names no one goatnet: it fires for any zenoh.<zone> host.
func TestAnalyzeLog_DNSResolutionFailed_AnyGoatnet(t *testing.T) {
	for _, sample := range []string{
		`ERROR failed to resolve zenoh.galaxy.goat.example:7447: no such host`,
		`WARN dns lookup failed for tls/zenoh.site-a.example.net:7447`,
	} {
		if !hasFinding(analyzeLog(sample), "endpoint-dns-resolution-failed") {
			t.Errorf("expected endpoint-dns-resolution-failed for %q", sample)
		}
	}
	if hasFinding(analyzeLog(`ERROR failed to resolve api.example.org: no such host`), "endpoint-dns-resolution-failed") {
		t.Error("DNS signature fired for a host that is not a zenoh router")
	}
}

func TestAnalyzeLog_TCPRefused(t *testing.T) {
	const sample = `ERROR dial tls/zenoh.goat.example:7447: connection refused`
	findings := analyzeLog(sample)
	if !hasFinding(findings, "endpoint-tcp-refused") {
		t.Fatalf("expected endpoint-tcp-refused, got %v", findingIDs(findings))
	}
}

func TestAnalyzeLog_TCPTimeout(t *testing.T) {
	const sample = `ERROR dial tls/zenoh.goat.example:7447: i/o timeout`
	findings := analyzeLog(sample)
	if !hasFinding(findings, "endpoint-tcp-timeout") {
		t.Fatalf("expected endpoint-tcp-timeout, got %v", findingIDs(findings))
	}
}

func TestAnalyzeLog_PythonImportError(t *testing.T) {
	const sample = `Traceback (most recent call last):
  File "first-publisher.py", line 3, in <module>
    import zenoh
ModuleNotFoundError: No module named 'zenoh'
`
	findings := analyzeLog(sample)
	if !hasFinding(findings, "python-eclipse-zenoh-import") {
		t.Fatalf("expected python-eclipse-zenoh-import, got %v", findingIDs(findings))
	}
}

func TestAnalyzeLog_PythonVersionSkew(t *testing.T) {
	const sample = `ERROR zenoh protocol version mismatch: peer expects 1.9, client speaks 0.10`
	findings := analyzeLog(sample)
	if !hasFinding(findings, "python-version-skew") {
		t.Fatalf("expected python-version-skew, got %v", findingIDs(findings))
	}
}

func TestAnalyzeLog_SubscribeNoSamples(t *testing.T) {
	const sample = `INFO subscribed on aardwolf/observations/uuid-XYZ/v1, samples: 0 after 30s`
	findings := analyzeLog(sample)
	if !hasFinding(findings, "subscribe-no-samples") {
		t.Fatalf("expected subscribe-no-samples, got %v", findingIDs(findings))
	}
}

func TestAnalyzeLog_ACLDeniedPublish(t *testing.T) {
	const sample = `ERROR access_control denied publish on key_expr "other-vendor/foo/v1"`
	findings := analyzeLog(sample)
	if !hasFinding(findings, "acl-denied-publish") {
		t.Fatalf("expected acl-denied-publish, got %v", findingIDs(findings))
	}
}

func TestAnalyzeLog_CargoNonexistent(t *testing.T) {
	const sample = `$ cargo install zenoh-cli
error: could not find ` + "`zenoh-cli`" + ` in registry`
	findings := analyzeLog(sample)
	if !hasFinding(findings, "cargo-nonexistent") {
		t.Fatalf("expected cargo-nonexistent, got %v", findingIDs(findings))
	}
}

// CleanLog returns no findings — guards against catalog signatures
// drifting into false-positive matches on benign output.
func TestAnalyzeLog_CleanLog(t *testing.T) {
	const sample = `INFO starting up
INFO connected to tls/zenoh.goat.example:7447
INFO sample 1 published successfully
INFO sample 2 published successfully
INFO shutdown clean
`
	findings := analyzeLog(sample)
	if len(findings) != 0 {
		t.Fatalf("expected zero findings on clean log, got %v", findingIDs(findings))
	}
}

// DuplicateSignature only fires once — same root cause may surface across
// multiple log lines; the user shouldn't see the same finding three times.
func TestAnalyzeLog_DuplicateSignature(t *testing.T) {
	const sample = `ERROR dial tls/foo:7447: connection refused
ERROR dial tls/foo:7447: connection refused
ERROR dial tls/foo:7447: connection refused
`
	findings := analyzeLog(sample)
	count := 0
	for _, f := range findings {
		if f.SignatureID == "endpoint-tcp-refused" {
			count++
		}
	}
	if count != 1 {
		t.Fatalf("expected exactly one endpoint-tcp-refused finding (duplicates suppressed), got %d", count)
	}
}

// WindowsCRLF paste — splitLines must strip \r so regexes match.
func TestAnalyzeLog_WindowsCRLF(t *testing.T) {
	sample := strings.ReplaceAll(`ERROR dial tls/zenoh.goat.example:7447: connection refused
`, "\n", "\r\n")
	findings := analyzeLog(sample)
	if !hasFinding(findings, "endpoint-tcp-refused") {
		t.Fatalf("expected endpoint-tcp-refused on Windows-CRLF paste, got %v", findingIDs(findings))
	}
}

// MatchedLineTruncation — a finding's matched_line is truncated to 200
// chars so a single absurdly-long log line can't blow up the response.
func TestAnalyzeLog_MatchedLineTruncation(t *testing.T) {
	long := strings.Repeat("x", 500)
	sample := "ERROR " + long + " connection refused :7447"
	findings := analyzeLog(sample)
	if len(findings) == 0 {
		t.Fatal("expected at least one finding")
	}
	for _, f := range findings {
		if len(f.MatchedLine) > 200 {
			t.Fatalf("matched_line not truncated: len=%d", len(f.MatchedLine))
		}
	}
}

// --- helpers ---

func hasFinding(findings []logFinding, id string) bool {
	for _, f := range findings {
		if f.SignatureID == id {
			return true
		}
	}
	return false
}

func findingIDs(findings []logFinding) []string {
	ids := make([]string, 0, len(findings))
	for _, f := range findings {
		ids = append(ids, f.SignatureID)
	}
	return ids
}
