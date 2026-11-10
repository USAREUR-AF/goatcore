package x509ca

import (
	"crypto/x509"
	"errors"
	"fmt"
	"net"
	"time"
)

// LeafTemplate carries the operator-controlled inputs to a leaf cert
// issuance. The cert template fields not exposed here (KeyUsage,
// BasicConstraints, signature algorithm) are set by IssueCert per the
// canonical leaf shape documented at docs/design/internal-tls-ca.md §3.
type LeafTemplate struct {
	// CommonName is the leaf cert's CN. Convention is the canonical FQDN;
	// TLS clients verify SAN, not CN, but CN aids human inspection.
	CommonName string

	// Organization defaults to DefaultOrganization if empty.
	Organization string

	// OrgUnit is typically the site_id or a service-class
	// label (e.g. "mgmt-plane"). Required.
	OrgUnit string

	// SANs is the exhaustive list of every address a peer might use to
	// reach this service. Mixed types — IPs and DNS names. IssueCert
	// classifies each via net.ParseIP.
	SANs []string

	// URISANs carries URI Subject Alternative Names (RFC 5280 §4.2.1.6),
	// distinct from DNS-name / IP-address SANs. Each entry must parse via
	// net/url.Parse and is set on the leaf's x509.Certificate.URIs.
	//
	// Used by per-peer identity schemes that key off a stable URN rather
	// than a hostname — a trial X.509 identity (`urn:goat-galaxy:vendor=...`)
	// and ADR 0844 production v1 (`urn:goat:peer:site=,role=,echelon=`).
	// Optional; empty slice / nil yields no URI SANs on the cert.
	URISANs []string

	// Validity is the leaf cert lifetime. Defaults to 365 * 24h if zero.
	Validity time.Duration

	// EKU is the Extended Key Usage list. Empty defaults to
	// {ExtKeyUsageServerAuth}. Add ExtKeyUsageClientAuth for mTLS-capable
	// services.
	EKU []x509.ExtKeyUsage

	// KeyType selects the leaf private-key algorithm. Zero value =
	// LeafKeyEd25519 (the original behavior). Set to LeafKeyECDSAP256
	// for operator-facing TLS endpoints that need to be reachable from
	// macOS browsers / LibreSSL clients per F-071.
	KeyType LeafKeyType
}

// validate returns nil iff t carries the minimum fields IssueCert needs.
func (t *LeafTemplate) validate() error {
	if t == nil {
		return errors.New("nil LeafTemplate")
	}
	if t.CommonName == "" {
		return errors.New("LeafTemplate.CommonName is required")
	}
	if t.OrgUnit == "" {
		return errors.New("LeafTemplate.OrgUnit is required (typically site_id)")
	}
	if len(t.SANs) == 0 {
		return errors.New("LeafTemplate.SANs is required (peers verify SAN, not CN)")
	}
	return nil
}

// withDefaults returns a copy of t with zero-value fields filled in.
func (t *LeafTemplate) withDefaults() *LeafTemplate {
	out := *t
	if out.Organization == "" {
		out.Organization = DefaultOrganization
	}
	if out.Validity == 0 {
		out.Validity = 365 * 24 * time.Hour
	}
	if len(out.EKU) == 0 {
		out.EKU = []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth}
	}
	return &out
}

// classifySANs splits the mixed SAN list into DNS names and IP addresses.
// Returns an error if any entry is empty.
func classifySANs(sans []string) (dns []string, ips []net.IP, err error) {
	for i, s := range sans {
		if s == "" {
			return nil, nil, fmt.Errorf("SAN[%d] is empty", i)
		}
		if ip := net.ParseIP(s); ip != nil {
			ips = append(ips, ip)
		} else {
			dns = append(dns, s)
		}
	}
	return dns, ips, nil
}
