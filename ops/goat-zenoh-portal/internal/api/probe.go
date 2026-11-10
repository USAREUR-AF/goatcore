package api

import (
	"fmt"
	"net/http"
	"strings"
)

// probe.go — Surface 2 of the live-troubleshooting set.
// Design-of-record: docs/design/portal-live-troubleshooting.md §2.
//
// The portal does NOT initiate any outbound probe. We hand the vendor a
// bash one-liner; the vendor runs it on the machine that's supposed to
// reach the router; the vendor pastes the JSON output back. We interpret
// it.
//
// Five layers, checked in dependency order. Layer N depends on the
// successful run of all previous layers; the JSON shape allows a partial
// result (lower-numbered layers tried, higher ones skipped).
//
//   1. DNS resolves
//   2. TCP reach on 7447
//   3. TLS handshake completes
//   4. Server cert chain validates against published root CA
//   5. Vendor's leaf URI-SAN matches the advertised prefix

// probeResult is the shape the vendor's script emits and pastes back.
// Each layer has its own object with the observed values; missing
// objects mean the layer didn't run (skipped because a prior layer
// failed, or the vendor ran with a flag that disabled it).
type probeResult struct {
	SchemaVersion int                `json:"schema_version"`
	Hostname      string             `json:"hostname,omitempty"`
	Port          int                `json:"port,omitempty"`
	IntendedPrefix string            `json:"intended_prefix,omitempty"`
	DNS           *probeLayerDNS     `json:"dns,omitempty"`
	TCP           *probeLayerTCP     `json:"tcp,omitempty"`
	TLS           *probeLayerTLS     `json:"tls,omitempty"`
	CertChain     *probeLayerChain   `json:"cert_chain,omitempty"`
	URISAN        *probeLayerURISAN  `json:"uri_san,omitempty"`
}

type probeLayerDNS struct {
	OK         bool     `json:"ok"`
	ResolvedIPs []string `json:"resolved_ips,omitempty"`
	Error      string   `json:"error,omitempty"`
}

type probeLayerTCP struct {
	OK    bool   `json:"ok"`
	Error string `json:"error,omitempty"`
}

type probeLayerTLS struct {
	OK              bool   `json:"ok"`
	NegotiatedCipher string `json:"negotiated_cipher,omitempty"`
	NegotiatedProto  string `json:"negotiated_proto,omitempty"`
	Error           string `json:"error,omitempty"`
}

type probeLayerChain struct {
	OK                       bool   `json:"ok"`
	ObservedFingerprintSHA256 string `json:"observed_fingerprint_sha256,omitempty"`
	Error                    string `json:"error,omitempty"`
}

type probeLayerURISAN struct {
	OK              bool     `json:"ok"`
	CertURISANs     []string `json:"cert_uri_sans,omitempty"`
	DeclaredPrefix  string   `json:"declared_prefix,omitempty"`
	Error           string   `json:"error,omitempty"`
}

// probeResultRequest is the POST /api/peering/probe-result body. Either
// the parsed result object OR a raw JSON string to parse (the script
// pipes through jq -c . so it's already a one-liner; the wrapping field
// keeps the API stable if a future probe shape changes).
type probeResultRequest struct {
	Result probeResult `json:"result"`
}

// probeFinding is the per-layer interpretation we return.
type probeFinding struct {
	Layer     string `json:"layer"`
	Severity  string `json:"severity"`
	Status    string `json:"status"` // "pass" | "fail" | "skip"
	Message   string `json:"message"`
	FixHint   string `json:"fix_hint,omitempty"`
	DocAnchor string `json:"doc_anchor,omitempty"`
}

type probeResultResponse struct {
	Hostname            string         `json:"hostname,omitempty"`
	Findings            []probeFinding `json:"findings"`
	OverallStatus       string         `json:"overall_status"` // "pass" | "fail" | "partial"
	FingerprintExpected string         `json:"fingerprint_expected,omitempty"`
	FingerprintObserved string         `json:"fingerprint_observed,omitempty"`
}

// handleProbeResult interprets the vendor-side probe output. Layer
// findings come out in dependency order; the page renders them top-down.
func (s *Server) handleProbeResult(w http.ResponseWriter, r *http.Request) {
	const maxBody = 64 * 1024
	r.Body = http.MaxBytesReader(w, r.Body, maxBody)

	var req probeResultRequest
	if err := decodeJSON(w, r, &req); err != nil {
		writeBodyError(w, err, "bad_request",
			"body must be JSON: {result: <probe-output>}")
		return
	}

	expectedFingerprint := s.rootCAFingerprintHex()
	findings := interpretProbe(req.Result, expectedFingerprint)
	overall := overallStatus(findings)
	observedFingerprint := ""
	if req.Result.CertChain != nil {
		observedFingerprint = strings.ToUpper(req.Result.CertChain.ObservedFingerprintSHA256)
	}

	writeJSON(w, http.StatusOK, probeResultResponse{
		Hostname:            req.Result.Hostname,
		Findings:            findings,
		OverallStatus:       overall,
		FingerprintExpected: expectedFingerprint,
		FingerprintObserved: observedFingerprint,
	})
}

// interpretProbe is the pure layer-by-layer interpreter. Each layer
// emits exactly one finding (pass / fail / skip). Skips cascade — if
// DNS fails, TCP/TLS/chain/URI-SAN all skip.
func interpretProbe(res probeResult, expectedFingerprint string) []probeFinding {
	out := []probeFinding{}

	// Layer 1 — DNS.
	if res.DNS == nil {
		out = append(out, probeFinding{
			Layer:    "1-dns",
			Severity: sevWarn,
			Status:   "skip",
			Message:  "DNS layer not present in probe output. Re-run the probe with the script from /peering/probe.sh.",
		})
	} else if !res.DNS.OK {
		out = append(out, probeFinding{
			Layer:     "1-dns",
			Severity:  sevError,
			Status:    "fail",
			Message:   "DNS could not resolve the router hostname: " + res.DNS.Error,
			FixHint:   "Finish your netbird enrollment so the mesh resolver answers for the router hostname. Without DNS, none of the subsequent layers can run.",
			DocAnchor: "/peering/our-config.html",
		})
		out = appendSkips(out, []string{"2-tcp", "3-tls", "4-cert-chain", "5-uri-san"}, "DNS failed; can't proceed.")
		return out
	} else {
		ips := strings.Join(res.DNS.ResolvedIPs, ", ")
		out = append(out, probeFinding{
			Layer:    "1-dns",
			Severity: sevInfo,
			Status:   "pass",
			Message:  "DNS resolves to " + ips,
		})
	}

	// Layer 2 — TCP.
	if res.TCP == nil {
		out = appendSkip(out, "2-tcp", "TCP layer not present in probe output.")
		out = appendSkips(out, []string{"3-tls", "4-cert-chain", "5-uri-san"}, "TCP layer skipped.")
		return out
	}
	if !res.TCP.OK {
		out = append(out, probeFinding{
			Layer:     "2-tcp",
			Severity:  sevError,
			Status:    "fail",
			Message:   "TCP to the router port failed: " + res.TCP.Error,
			FixHint:   "Either a firewall is dropping packets between you and the fabric (`connection timed out`), or the listener actively refused (`connection refused`). Check that 7447/TCP outbound is allowed, AND that your netbird tunnel is up (`netbird status`).",
			DocAnchor: "/peering/our-config.html",
		})
		out = appendSkips(out, []string{"3-tls", "4-cert-chain", "5-uri-san"}, "TCP failed; can't proceed.")
		return out
	}
	out = append(out, probeFinding{
		Layer:    "2-tcp",
		Severity: sevInfo,
		Status:   "pass",
		Message:  "TCP reaches the router port.",
	})

	// Layer 3 — TLS.
	if res.TLS == nil {
		out = appendSkip(out, "3-tls", "TLS layer not present in probe output.")
		out = appendSkips(out, []string{"4-cert-chain", "5-uri-san"}, "TLS layer skipped.")
		return out
	}
	if !res.TLS.OK {
		out = append(out, probeFinding{
			Layer:     "3-tls",
			Severity:  sevError,
			Status:    "fail",
			Message:   "TLS handshake failed: " + res.TLS.Error,
			FixHint:   "The TCP socket opens but TLS doesn't complete. Common causes: stale openssl on your host that can't negotiate TLS 1.3, or the router cert is using an EC curve your client doesn't support. Re-run with a current openssl (>= 1.1.1).",
			DocAnchor: "/peering/our-config.html",
		})
		out = appendSkips(out, []string{"4-cert-chain", "5-uri-san"}, "TLS failed; can't proceed.")
		return out
	}
	out = append(out, probeFinding{
		Layer:    "3-tls",
		Severity: sevInfo,
		Status:   "pass",
		Message:  fmt.Sprintf("TLS handshake completes (%s, %s)", res.TLS.NegotiatedProto, res.TLS.NegotiatedCipher),
	})

	// Layer 4 — cert chain.
	if res.CertChain == nil {
		out = appendSkip(out, "4-cert-chain", "Cert-chain layer not present in probe output.")
		out = appendSkip(out, "5-uri-san", "Cert-chain layer skipped.")
		return out
	}
	if !res.CertChain.OK {
		out = append(out, probeFinding{
			Layer:     "4-cert-chain",
			Severity:  sevError,
			Status:    "fail",
			Message:   "Server cert chain did not validate against the published root CA: " + res.CertChain.Error,
			FixHint:   "Download a fresh copy of the root CA from `/peering/root-ca.pem` and re-run. If the chain still doesn't validate, the fabric may have rotated its CA; check `GET /api/fabric/config` for the current `tls.root_ca.fingerprint_sha256`.",
			DocAnchor: "/peering/our-config.html",
		})
		// Don't auto-fail URI-SAN; the URI-SAN is from the VENDOR's cert,
		// independent of whether the server's cert validates.
	} else {
		// Fingerprint compare — soft check; the cert chain validating is
		// the primary signal. Surface mismatch as a finding even when both
		// values exist (rotation in flight on either side will trip this).
		observed := strings.ToUpper(res.CertChain.ObservedFingerprintSHA256)
		if expectedFingerprint != "" && observed != "" && observed != expectedFingerprint {
			out = append(out, probeFinding{
				Layer:     "4-cert-chain",
				Severity:  sevError,
				Status:    "fail",
				Message:   "Server cert chain validates, BUT the observed root-CA fingerprint doesn't match the published one. observed=" + observed + " expected=" + expectedFingerprint,
				FixHint:   "Either you're trusting an old CA bundle (re-download from `/peering/root-ca.pem`), or the fabric has rotated. Cross-check by reading the current expected fingerprint at `GET /api/fabric/config`.",
				DocAnchor: "/peering/our-config.html",
			})
		} else {
			out = append(out, probeFinding{
				Layer:    "4-cert-chain",
				Severity: sevInfo,
				Status:   "pass",
				Message:  "Server cert chain validates against the published root CA.",
			})
		}
	}

	// Layer 5 — URI-SAN of the vendor's OWN cert against their declared
	// prefix. This is a local-only check; the portal can't verify the
	// vendor's cert because the portal never sees it. We just interpret
	// what the script reports.
	if res.URISAN == nil {
		out = appendSkip(out, "5-uri-san", "URI-SAN layer not present in probe output. Re-run with `--cert /path/to/connect_certificate.pem`.")
		return out
	}
	if !res.URISAN.OK {
		msg := "URI-SAN check failed"
		if res.URISAN.Error != "" {
			msg = msg + ": " + res.URISAN.Error
		}
		if len(res.URISAN.CertURISANs) > 0 {
			msg = msg + " (cert URI-SANs: " + strings.Join(res.URISAN.CertURISANs, ", ") + ")"
		}
		out = append(out, probeFinding{
			Layer:     "5-uri-san",
			Severity:  sevError,
			Status:    "fail",
			Message:   msg,
			FixHint:   "Your client cert's URI-SAN doesn't match the prefix you declared. If your prefix changed since you got the bundle, you need a re-mint from the operator — there's no way to retrofit URI-SAN without a fresh leaf.",
			DocAnchor: "/peering/guide.html#identity",
		})
	} else {
		msg := "Cert URI-SAN matches your declared prefix"
		if res.URISAN.DeclaredPrefix != "" {
			msg = msg + " (" + res.URISAN.DeclaredPrefix + ")"
		}
		out = append(out, probeFinding{
			Layer:    "5-uri-san",
			Severity: sevInfo,
			Status:   "pass",
			Message:  msg,
		})
	}

	return out
}

func appendSkip(out []probeFinding, layer, reason string) []probeFinding {
	return append(out, probeFinding{
		Layer:    layer,
		Severity: sevWarn,
		Status:   "skip",
		Message:  reason,
	})
}

func appendSkips(out []probeFinding, layers []string, reason string) []probeFinding {
	for _, l := range layers {
		out = appendSkip(out, l, reason)
	}
	return out
}

// overallStatus collapses the per-layer status list. Any fail = fail;
// any skip + at least one pass = partial; all pass = pass.
func overallStatus(findings []probeFinding) string {
	hasFail := false
	hasSkip := false
	hasPass := false
	for _, f := range findings {
		switch f.Status {
		case "fail":
			hasFail = true
		case "skip":
			hasSkip = true
		case "pass":
			hasPass = true
		}
	}
	switch {
	case hasFail:
		return "fail"
	case hasSkip && hasPass:
		return "partial"
	case hasPass:
		return "pass"
	default:
		return "unknown"
	}
}

// handleProbeScript renders the bash one-liner the vendor runs locally.
// text/plain; the page renders it inline FIRST so the vendor can read
// what they're about to run before any curl-pipe-bash button is offered.
//
// Parameters via query string:
//   ?prefix=<prefix>          — for layer 5 URI-SAN compare
//   ?cert=<path>              — local cert path; the script uses this as
//                                its --cert default if the vendor doesn't
//                                pass one explicitly. Optional.
func (s *Server) handleProbeScript(w http.ResponseWriter, r *http.Request) {
	hostAndPort := strings.TrimPrefix(s.cfg.RouterEndpoint, "tls/")
	if hostAndPort == "" {
		hostAndPort = "zenoh.goat.example:7447"
	}
	idx := strings.LastIndex(hostAndPort, ":")
	host := hostAndPort
	port := "7447"
	if idx > 0 {
		host = hostAndPort[:idx]
		port = hostAndPort[idx+1:]
	}
	rootCAURL := "/peering/root-ca.pem"
	if s.cfg.PortalPublicURL != "" {
		rootCAURL = s.cfg.PortalPublicURL + rootCAURL
	}

	prefix := strings.TrimSpace(r.URL.Query().Get("prefix"))
	defaultCert := strings.TrimSpace(r.URL.Query().Get("cert"))

	w.Header().Set("Content-Type", "text/plain; charset=utf-8")
	fmt.Fprintf(w, probeScriptTemplate, host, port, rootCAURL, prefix, defaultCert)
}

// probeScriptTemplate is a small bash script the vendor runs. It uses
// only standard tools (getent / bash /dev/tcp / openssl / jq). It echoes
// a single JSON object to stdout that the vendor pastes back.
//
// Deliberately simple: no `set -e` magic, no temp-file dance the vendor
// can't follow, no silent failure modes. Each layer's result is written
// to a shell variable, and a final printf assembles the JSON.
const probeScriptTemplate = `#!/usr/bin/env bash
# Zenoh trial fabric — vendor-side connectivity probe.
# Run this from the host that's supposed to reach the router; paste the
# JSON it emits back into /peering/probe.html.
#
# Read this script before running it. It does NOT modify your system; it
# only reads. No network listeners are opened. No data is transmitted to
# the portal — output goes to your stdout, and you decide what to paste.
#
# Usage:
#   ./probe.sh                          # run all layers using script defaults
#   ./probe.sh --prefix my-vendor       # set the prefix for layer 5
#   ./probe.sh --cert /path/to/cert.pem # set the client cert for layer 5

set -u

HOST="%[1]s"
PORT="%[2]s"
ROOT_CA_URL="%[3]s"
PREFIX="%[4]s"
CERT="%[5]s"

while [ $# -gt 0 ]; do
  case "$1" in
    --prefix) PREFIX="$2"; shift 2;;
    --cert)   CERT="$2"; shift 2;;
    -h|--help) sed -n '2,/^$/p' "$0"; exit 0;;
    *) echo "unknown flag: $1" >&2; exit 2;;
  esac
done

# Layer 1 — DNS.
DNS_OK="false"; DNS_IPS="[]"; DNS_ERR=""
if RESOLVED=$(getent ahosts "$HOST" 2>&1 | awk '{print $1}' | sort -u | head -16); then
  if [ -n "$RESOLVED" ]; then
    DNS_OK="true"
    DNS_IPS=$(echo "$RESOLVED" | jq -R . | jq -sc .)
  else
    DNS_ERR="getent returned no addresses for $HOST"
  fi
else
  DNS_ERR="getent failed for $HOST: $RESOLVED"
fi

# Layer 2 — TCP. Bash's /dev/tcp is universally present and needs no
# external tool; 5s timeout is enough on any sane network.
TCP_OK="false"; TCP_ERR=""
if [ "$DNS_OK" = "true" ]; then
  if (timeout 5 bash -c "</dev/tcp/$HOST/$PORT") 2>/dev/null; then
    TCP_OK="true"
  else
    TCP_ERR="TCP to $HOST:$PORT did not open within 5s (refused or timed out)"
  fi
else
  TCP_ERR="skipped: DNS failed"
fi

# Layer 3 — TLS handshake.
TLS_OK="false"; TLS_CIPHER=""; TLS_PROTO=""; TLS_ERR=""
if [ "$TCP_OK" = "true" ]; then
  HS=$(echo "Q" | timeout 8 openssl s_client -connect "$HOST:$PORT" -servername "$HOST" 2>&1 || true)
  if echo "$HS" | grep -q "BEGIN CERTIFICATE"; then
    TLS_OK="true"
    TLS_PROTO=$(echo "$HS" | grep -E "^\s*Protocol\s*:" | head -1 | awk -F: '{print $2}' | tr -d ' ' || true)
    TLS_CIPHER=$(echo "$HS" | grep -E "^\s*Cipher\s*:" | head -1 | awk -F: '{print $2}' | tr -d ' ' || true)
  else
    TLS_ERR=$(echo "$HS" | grep -E "(error|alert|verify error)" | head -1 || echo "TLS handshake did not return a certificate")
  fi
else
  TLS_ERR="skipped: TCP failed"
fi

# Layer 4 — cert chain validates against the published root CA.
CHAIN_OK="false"; CHAIN_FP=""; CHAIN_ERR=""
if [ "$TLS_OK" = "true" ]; then
  ROOT_CA_FILE=$(mktemp)
  trap 'rm -f "$ROOT_CA_FILE"' EXIT
  if curl -fsSL "$ROOT_CA_URL" -o "$ROOT_CA_FILE" 2>/dev/null; then
    CHAIN_OUT=$(echo "Q" | timeout 8 openssl s_client -connect "$HOST:$PORT" -servername "$HOST" -CAfile "$ROOT_CA_FILE" -verify_return_error 2>&1 || true)
    if echo "$CHAIN_OUT" | grep -q "Verify return code: 0"; then
      CHAIN_OK="true"
    else
      CHAIN_ERR=$(echo "$CHAIN_OUT" | grep -E "verify (error|return code)" | head -1 || echo "verify did not succeed")
    fi
    # Compute the published root-CA fingerprint so we can compare to the portal's published value.
    CHAIN_FP=$(openssl x509 -in "$ROOT_CA_FILE" -fingerprint -sha256 -noout 2>/dev/null | awk -F= '{gsub(":", "", $2); print toupper($2)}')
  else
    CHAIN_ERR="could not download root CA from $ROOT_CA_URL"
  fi
else
  CHAIN_ERR="skipped: TLS failed"
fi

# Layer 5 — URI-SAN compare on the VENDOR's own cert.
URISAN_OK="false"; URISAN_SANS="[]"; URISAN_ERR=""
if [ -n "$CERT" ] && [ -n "$PREFIX" ]; then
  if [ -r "$CERT" ]; then
    SANS=$(openssl x509 -in "$CERT" -text -noout 2>/dev/null | grep -E "URI:" | sed 's/.*URI://g' | awk '{print $1}' | sort -u)
    if [ -n "$SANS" ]; then
      URISAN_SANS=$(echo "$SANS" | jq -R . | jq -sc .)
      if echo "$SANS" | grep -qE "(/|=)$PREFIX(/|$)"; then
        URISAN_OK="true"
      else
        URISAN_ERR="no URI-SAN contains the declared prefix $PREFIX"
      fi
    else
      URISAN_ERR="no URI-SAN found in cert"
    fi
  else
    URISAN_ERR="cert path is not readable: $CERT"
  fi
else
  if [ -z "$CERT" ]; then
    URISAN_ERR="skipped: no --cert provided"
  else
    URISAN_ERR="skipped: no --prefix provided"
  fi
fi

# Final JSON. jq does the JSON assembly so escaping is correct even for
# error strings with quotes or newlines.
jq -c -n \
  --arg host "$HOST" --argjson port "$PORT" --arg prefix "$PREFIX" \
  --argjson dns_ok "$DNS_OK" --argjson dns_ips "$DNS_IPS" --arg dns_err "$DNS_ERR" \
  --argjson tcp_ok "$TCP_OK" --arg tcp_err "$TCP_ERR" \
  --argjson tls_ok "$TLS_OK" --arg tls_cipher "$TLS_CIPHER" --arg tls_proto "$TLS_PROTO" --arg tls_err "$TLS_ERR" \
  --argjson chain_ok "$CHAIN_OK" --arg chain_fp "$CHAIN_FP" --arg chain_err "$CHAIN_ERR" \
  --argjson urisan_ok "$URISAN_OK" --argjson urisan_sans "$URISAN_SANS" --arg urisan_err "$URISAN_ERR" \
  '{
    schema_version: 1,
    hostname: $host, port: $port, intended_prefix: $prefix,
    dns:        { ok: $dns_ok, resolved_ips: $dns_ips, error: $dns_err },
    tcp:        { ok: $tcp_ok, error: $tcp_err },
    tls:        { ok: $tls_ok, negotiated_cipher: $tls_cipher, negotiated_proto: $tls_proto, error: $tls_err },
    cert_chain: { ok: $chain_ok, observed_fingerprint_sha256: $chain_fp, error: $chain_err },
    uri_san:    { ok: $urisan_ok, cert_uri_sans: $urisan_sans, declared_prefix: $prefix, error: $urisan_err }
  }'
`
