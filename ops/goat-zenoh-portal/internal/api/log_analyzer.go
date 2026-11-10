package api

import (
	"net/http"
	"regexp"
	"strings"
)

// log_analyzer.go — Surface 1 of the live-troubleshooting set.
// Design-of-record: docs/design/portal-live-troubleshooting.md §1.
//
// Vendor pastes their zenohd / publisher stderr, backend pattern-matches
// against a curated catalog. Each match yields a structured finding the
// page renders in the same shape as the workbench's lint findings.
//
// Adding a new signature = appending one logSignature entry + one test
// fixture. The catalog is the public contract; the page renders it as a
// collapsible reference under the paste box so vendors see what we know
// about before they paste.

// logAnalyzerRequest is the POST body. content is the raw paste; we cap
// it at 512 KiB at the handler so a runaway paste can't OOM us.
type logAnalyzerRequest struct {
	Content string `json:"content"`
}

// logFinding is one signature match. MatchedLine is the literal line we
// hit on, truncated to 200 chars so the user can see we found their line
// and not a generic guess.
type logFinding struct {
	Severity    string `json:"severity"`
	SignatureID string `json:"signature_id"`
	Message     string `json:"message"`
	FixHint     string `json:"fix_hint"`
	DocAnchor   string `json:"doc_anchor,omitempty"`
	MatchedLine string `json:"matched_line,omitempty"`
}

type logAnalyzerResponse struct {
	Findings []logFinding `json:"findings"`
	// LinesScanned and SignaturesTried let the UI render an "we looked
	// at X lines, ran Y signatures" status line. Builds confidence that
	// the analyzer ran fully even when it finds nothing.
	LinesScanned     int `json:"lines_scanned"`
	SignaturesTried  int `json:"signatures_tried"`
	CatalogVersion   int `json:"catalog_version"`
}

// logSignature is a single pattern in the catalog. Composite patterns
// (require multiple regexes within a small line window) get a separate
// scan function rather than trying to express them in one regex.
type logSignature struct {
	ID        string
	Severity  string
	Message   string
	FixHint   string
	DocAnchor string
	// Either Regex (single-line match) or Compound (multi-line) is set.
	Regex    *regexp.Regexp
	Compound func(lines []string) (matchedLine string, ok bool)
}

// catalogVersion bumps every time the catalog meaningfully changes. The
// page surfaces this so a vendor reading old advice doesn't get confused
// about WHICH version of the catalog flagged them.
const catalogVersion = 1

// catalog is the v1 signature set, mirroring the table in the design doc.
// Ordering is intentional: more-specific signatures first so the first
// match wins when multiple could fire on the same line.
var catalog = []logSignature{
	{
		ID:        "peer-mode-listener-missing-cert",
		Severity:  sevError,
		Message:   "Process is in peer/router mode — Zenoh tried to stand up its OWN TLS listener, which needs server-side cert files this config doesn't have.",
		FixHint:   "Two valid paths depending on what you intended. (Flavor A — single publisher process, most common): set `mode: \"client\"` and remove any `listen.endpoints` block. Your bundle's connect_certificate / connect_private_key / root_ca_certificate are the only certs a client needs. (Flavor B — you actually want to run a Zenoh ROUTER that peers with our fabric): keep `mode: \"router\"` but ALSO configure `listen_certificate_file` + `listen_private_key_file` so your router can accept incoming connections from local publishers. The canonical Flavor B template is at /peering/zenohd-config.html (parameterized for your prefix).",
		DocAnchor: "/peering/guide.html",
		Compound: func(lines []string) (string, bool) {
			// Match if we see both "Cannot create a new TLS listener" AND
			// "No such file or directory" within 5 lines of each other.
			// That's the failure shape — the listener init looking for
			// listen_certificate_file / listen_private_key_file that the
			// publisher's bundle doesn't carry.
			for i, line := range lines {
				if !strings.Contains(line, "Cannot create a new TLS listener") {
					continue
				}
				lo := i
				hi := i + 5
				if hi > len(lines) {
					hi = len(lines)
				}
				for j := lo; j < hi; j++ {
					if strings.Contains(lines[j], "No such file or directory") ||
						strings.Contains(lines[j], "os error 2") {
						return line, true
					}
				}
			}
			return "", false
		},
	},
	{
		ID:        "scout-multicast-no-connect",
		Severity:  sevWarn,
		Message:   "Scout messages on the IPv4 multicast group suggest peer-mode default with no explicit connect target. On a remote fabric, the scout never finds the router.",
		FixHint:   "Either way you need an explicit `connect.endpoints` entry naming the router endpoint published at /api/fabric/config (`router.endpoint`) — the fabric is reachable only via TLS over the published endpoint, never multicast scout. For Flavor A (single publisher process) also set `mode: \"client\"`. For Flavor B (you run a Zenoh router that peers with us) keep `mode: \"router\"` and ALSO configure listen_certificate_file / listen_private_key_file for local connections. Canonical Flavor B template at /peering/zenohd-config.html.",
		DocAnchor: "/peering/guide.html",
		Regex:     regexp.MustCompile(`(?i)listening scout messages on (?:224\.0\.0\.224|\[ff)`),
	},
	{
		ID:        "tls-handshake-cert-unknown-ca",
		Severity:  sevError,
		Message:   "TLS handshake rejected the server's certificate because your trust store doesn't contain the fabric's root CA.",
		FixHint:   "Download the root CA from this portal at /peering/root-ca.pem and reference it via `root_ca_certificate_file` in your config (or `root_ca_certificate` in a Python `zenoh.Config()`).",
		DocAnchor: "/peering/our-config.html",
		Regex:     regexp.MustCompile(`(?i)unknown\s*issuer|certificate signed by unknown authority|self[- ]signed certificate in certificate chain`),
	},
	{
		ID:        "tls-handshake-hostname-mismatch",
		Severity:  sevError,
		Message:   "TLS handshake validated the cert chain but the server's hostname doesn't match the SAN in its certificate.",
		FixHint:   "Connect to the router's DNS hostname (`router.endpoint` at /api/fabric/config), not the raw mesh IP. Set `verify_name_on_connect: true`.",
		DocAnchor: "/peering/our-config.html",
		Regex:     regexp.MustCompile(`(?i)NotValidForName|hostname (?:does not match|mismatch)|certificate (?:is )?valid for [^ ]+, not`),
	},
	{
		ID:        "tls-handshake-client-cert-missing",
		Severity:  sevError,
		Message:   "Server demanded a client certificate but didn't get a valid one. Either the cert file is missing/unreadable, or its permissions block the user the publisher runs as.",
		FixHint:   "Verify `connect_certificate_file` and `connect_private_key_file` paths exist and are readable by the publisher's user. On Linux, `chmod 0644` is the right mode (`0600` makes the handshake fail this way).",
		DocAnchor: "/peering/our-config.html",
		Regex:     regexp.MustCompile(`(?i)peer did not return a certificate|bad certificate|no client certificate|client didn't provide a certificate`),
	},
	{
		ID:        "tls-handshake-uri-san-rejected",
		Severity:  sevError,
		Message:   "Server's ACL rejected your URI-SAN. The prefix you're trying to publish under doesn't match the URI-SAN in your client certificate.",
		FixHint:   "Confirm your certificate's URI-SAN matches the org prefix you intend to use. The operator who minted your bundle pinned this at mint time — if you changed your prefix, you need a re-mint, not just a config edit.",
		DocAnchor: "/peering/guide.html#identity",
		Regex:     regexp.MustCompile(`(?i)URI[- ]?SAN[^\n]*?(not authorized|prefix mismatch|denied|rejected)|access[_ ]control[^\n]*?denied[^\n]*?URI`),
	},
	{
		ID:        "endpoint-dns-resolution-failed",
		Severity:  sevError,
		Message:   "DNS could not resolve the fabric's router hostname.",
		FixHint:   "If you're on the mesh, the resolver pushed by netbird should answer for the router hostname. If you're NOT on the mesh, you can't reach the fabric — finish your peer enrollment first.",
		DocAnchor: "/peering/our-config.html",
		Regex:     regexp.MustCompile(`(?i)(failed to resolve|no such host|name (or service )?not known|dns lookup failed)[^\n]*\bzenoh\.[a-z0-9-]+\.`),
	},
	{
		ID:        "endpoint-tcp-refused",
		Severity:  sevError,
		Message:   "TCP connection to the router port was refused.",
		FixHint:   "DNS resolves but no listener is accepting connections on port 7447 from your host. Either a firewall is dropping the SYN, your host's route to the mesh isn't up, or you're connecting from the wrong network interface. Run the connectivity probe at /peering/probe.html for layer-by-layer diagnosis.",
		DocAnchor: "/peering/probe.html",
		Regex:     regexp.MustCompile(`(?i)connection refused[^\n]*?:7447|:7447[^\n]*?connection refused`),
	},
	{
		ID:        "endpoint-tcp-timeout",
		Severity:  sevError,
		Message:   "TCP connection to the router port timed out — packets are being silently dropped, not actively refused.",
		FixHint:   "Almost always a firewall between you and the fabric. Check that 7447/TCP is allowed outbound from your host AND that your netbird tunnel is up (`netbird status`). Run the connectivity probe at /peering/probe.html.",
		DocAnchor: "/peering/probe.html",
		Regex:     regexp.MustCompile(`(?i)(connection timed out|i/o timeout|connect.*timeout)[^\n]*?:7447|:7447[^\n]*?(connection timed out|i/o timeout)`),
	},
	{
		ID:        "python-eclipse-zenoh-import",
		Severity:  sevError,
		Message:   "Python can't import the `zenoh` module — you haven't installed the upstream eclipse-zenoh client.",
		FixHint:   "Run `pip install eclipse-zenoh==1.9.0 protobuf` in your virtualenv. The fabric is pinned to Zenoh 1.9.0; pinning the client to the same version avoids wire-format skew.",
		DocAnchor: "/peering/guide.html#flavor-a",
		Regex:     regexp.MustCompile(`(?i)(ImportError|ModuleNotFoundError)[^\n]*?['\x60"]?zenoh['\x60"]?`),
	},
	{
		ID:        "python-version-skew",
		Severity:  sevWarn,
		Message:   "Eclipse-zenoh Python client version doesn't match the fabric's pin.",
		FixHint:   "The fabric is pinned to Zenoh 1.9.0. Mismatched versions can skew the wire format silently. `pip install --force-reinstall eclipse-zenoh==1.9.0`.",
		DocAnchor: "/peering/our-config.html",
		Regex:     regexp.MustCompile(`(?i)zenoh[^\n]{0,60}(incompatible|version mismatch|protocol version)`),
	},
	{
		ID:        "subscribe-no-samples",
		Severity:  sevInfo,
		Message:   "Subscriber appears to be running but receiving zero samples. Classic S3-vs-channel mental-model trip.",
		FixHint:   "Zenoh topics are channels, not buckets. If your publisher writes to `vendor/system/uuid-XYZ/v1` (fresh UUID per message) and your subscriber listens on the literal `vendor/system/<some-uuid>/v1`, you'll never match. Use a wildcard: `vendor/system/*/v1` or `vendor/system/**`. See /peering/best-practices.html.",
		DocAnchor: "/peering/best-practices.html",
		Regex:     regexp.MustCompile(`(?i)subscribed[^\n]{0,40}(samples[^\n]*?[:= ]0|received 0)`),
	},
	{
		ID:        "acl-denied-publish",
		Severity:  sevError,
		Message:   "The router's ACL refused your publish — your `key_expr` falls outside your `<prefix>/**` allowance.",
		FixHint:   "Verify the topic key you're publishing on starts with your org prefix. If it does, double-check for typos (e.g. trailing whitespace, swapped segments). If it doesn't, that's the bug — every topic must live under your `<prefix>/**`.",
		DocAnchor: "/peering/guide.html#acl",
		Regex:     regexp.MustCompile(`(?i)access[_ ]control[^\n]*?(denied|forbidden|not authorized)[^\n]*?key[_ ]?expr|(?:denied|forbidden|rejected)[^\n]*?publish[^\n]*?key`),
	},
	{
		ID:        "cargo-nonexistent",
		Severity:  sevWarn,
		Message:   "`cargo install zenoh-cli` does not exist as a published crate.",
		FixHint:   "You're following an old guide. The current bundle ships a worked Python `first-publisher.py`. To run it: `pip install eclipse-zenoh==1.9.0 protobuf && python3 first-publisher.py`. The Zenoh CLI is `zenohd` (the router) and the in-process Python client.",
		DocAnchor: "/peering/guide.html#flavor-a",
		Regex:     regexp.MustCompile(`cargo install zenoh-cli`),
	},
}

// handleDiagnoseLog runs the catalog against the paste. Same posture as
// the workbench: no auth, no retention, in-memory parse + drop.
func (s *Server) handleDiagnoseLog(w http.ResponseWriter, r *http.Request) {
	// Cap the body at 512 KiB before we even parse. A vendor's debug log
	// can be large; an attacker's body is unbounded.
	const maxBody = 512 * 1024
	r.Body = http.MaxBytesReader(w, r.Body, maxBody)

	var req logAnalyzerRequest
	if err := decodeJSON(w, r, &req); err != nil {
		writeBodyError(w, err, "bad_request",
			"body must be JSON: {content: \"<paste>\"}; max 512 KiB")
		return
	}
	if strings.TrimSpace(req.Content) == "" {
		writeError(w, http.StatusBadRequest, "empty_content",
			"content is required and must be non-empty")
		return
	}

	findings := analyzeLog(req.Content)
	lines := splitLines(req.Content)
	writeJSON(w, http.StatusOK, logAnalyzerResponse{
		Findings:        findings,
		LinesScanned:    len(lines),
		SignaturesTried: len(catalog),
		CatalogVersion:  catalogVersion,
	})
}

// analyzeLog is the pure function under handleDiagnoseLog — exported in
// test scope (lowercase but in-package) so test fixtures can call it
// without standing up a Server. Returns findings in catalog declaration
// order; the page renders them flat.
func analyzeLog(content string) []logFinding {
	lines := splitLines(content)
	var out []logFinding
	// Track which signature IDs have fired so we don't emit duplicates
	// for the same logical failure on multiple matching lines.
	seen := map[string]bool{}

	for _, sig := range catalog {
		if seen[sig.ID] {
			continue
		}
		var matched string
		var hit bool
		switch {
		case sig.Compound != nil:
			matched, hit = sig.Compound(lines)
		case sig.Regex != nil:
			for _, line := range lines {
				if sig.Regex.MatchString(line) {
					matched = line
					hit = true
					break
				}
			}
		}
		if !hit {
			continue
		}
		seen[sig.ID] = true
		out = append(out, logFinding{
			Severity:    sig.Severity,
			SignatureID: sig.ID,
			Message:     sig.Message,
			FixHint:     sig.FixHint,
			DocAnchor:   sig.DocAnchor,
			MatchedLine: truncate(matched, 200),
		})
	}
	return out
}

func splitLines(s string) []string {
	// Tolerate \r\n (Windows pastes) by stripping trailing \r on each
	// line. strings.Split on "\n" preserves empty trailing entries which
	// we leave — they don't match anything but the line count reflects
	// what the user pasted.
	raw := strings.Split(s, "\n")
	for i, line := range raw {
		raw[i] = strings.TrimRight(line, "\r")
	}
	return raw
}

func truncate(s string, n int) string {
	if len(s) <= n {
		return s
	}
	// Trim to n-3 bytes so the trailing "…" (3 bytes UTF-8) keeps the
	// total under the cap. Without the -3 the resulting string can be
	// n+2 bytes — small but visible in the test guard.
	return s[:n-3] + "…"
}
