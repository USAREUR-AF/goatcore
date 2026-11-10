// Command cert-sign issues an X.509 leaf certificate signed by the offline
// CA root, for service-host TLS termination on internal-only endpoints.
//
// Per ADR 0407 (internal X.509 CA for service TLS, single-tier, shares the
// ADR 0406 root) and docs/design/internal-tls-ca.md.
//
// Usage:
//
//	cert-sign \
//	    --root-cert ops/enrollment/ca/dev/root-cert.pem \
//	    --root-key  ops/enrollment/ca/dev/ca.key.age \
//	    --age-identity ~/.config/age/keys.txt \
//	    --site-id site-a \
//	    --cn mgmt.site-a.galaxy.example \
//	    --san 198.18.0.1 \
//	    --san 127.0.0.1 \
//	    --san mgmt.site-a.galaxy.example \
//	    --validity 8760h \
//	    --eku serverAuth \
//	    --out-cert ./out/mgmt-site-a-cert.pem \
//	    --out-key  ./out/mgmt-site-a-key.pem \
//	    [--audit-log <path>]
//
// On success, writes the leaf cert PEM (mode 0644) and the leaf private
// key PEM (mode 0400 root:root) and appends a JSON line to the audit log.
//
// Production (YubiKey) signing is a separate code path under ADR 0406 and
// is not yet implemented; cert-sign will fail with a clear message when
// --root-key points at a YubiKey URI.
package main

import (
	"bytes"
	"crypto/ecdsa"
	"crypto/x509"
	"encoding/json"
	"encoding/pem"
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"time"

	"filippo.io/age"

	"github.com/usareur-af/goatcore/ops/enrollment/ca"
	"github.com/usareur-af/goatcore/ops/enrollment/x509ca"
)

// stringList implements flag.Value for repeated --san flags.
type stringList []string

func (s *stringList) String() string     { return strings.Join(*s, ",") }
func (s *stringList) Set(v string) error { *s = append(*s, v); return nil }

func main() {
	var (
		rootCertPath = flag.String("root-cert", "", "Path to PEM-encoded CA root cert (produced by `ca-init --gen-root-cert`)")
		rootKeyPath  = flag.String("root-key", "", "Path to age-encrypted CA private key file")
		ageIdentity  = flag.String("age-identity", "", "Path to age identity file (decrypts --root-key OR --intermediate-key)")
		caID         = flag.String("ca-id", "", "CA identifier carried with audit-log entries; defaults to root-cert filename minus .pem")
		siteID       = flag.String("site-id", "", "Site identifier (Subject OU); e.g. site-a")
		org          = flag.String("org", "", "Subject Organization (O) of the leaf. Empty: the x509ca default. A CA whose earlier leaves carry another organisation passes it on every call.")
		cn           = flag.String("cn", "", "Subject CommonName (typically the canonical FQDN)")
		validity     = flag.Duration("validity", 365*24*time.Hour, "Leaf cert lifetime; default 1 year per ADR 0407")
		ekuFlag      = flag.String("eku", "serverAuth", "Comma-separated EKU set; values: serverAuth,clientAuth")
		outCert      = flag.String("out-cert", "", "Path to write the leaf cert PEM (mode 0644)")
		outKey       = flag.String("out-key", "", "Path to write the leaf key PEM (mode 0400)")
		auditLog     = flag.String("audit-log", ca.DefaultAuditLogPath(), "Path to append-only JSON audit log")
		keyType      = flag.String("key-type", "auto", "Leaf private-key algorithm; values: auto (default — ecdsa-p256 when EKU includes serverAuth, ed25519 otherwise), ed25519, ecdsa-p256. Auto closes F-071 + F-089: every operator-facing TLS leaf must be ECDSA-P256 because Chrome on macOS rejects Ed25519 leaves at TLS handshake.")
		// F-090 closure path B: when both flags are set, sign leaves
		// chained through an ECDSA-P256 intermediate. Operators install
		// the INTERMEDIATE in their macOS keychain (which accepts ECDSA
		// roots — Ed25519 fail-import). Chain validates without per-leaf
		// trust hacks. When unset, leaves are signed by the root directly
		// (legacy / non-macOS-browser path).
		intermediateCertPath = flag.String("intermediate-cert", "", "Optional. Path to ECDSA-P256 X.509 intermediate cert (produced by `ca-init --gen-intermediate-cert`). When set with --intermediate-key, leaves are signed by the intermediate instead of the root. F-090 closure.")
		intermediateKeyPath  = flag.String("intermediate-key", "", "Optional. Path to age-encrypted intermediate private key. Required when --intermediate-cert is set.")
		sans                 stringList
		uriSANs              stringList
	)
	flag.Var(&sans, "san", "Subject Alternative Name; repeat once per name/IP. IPs and DNS names are auto-classified.")
	flag.Var(&uriSANs, "uri-san", "Optional URI Subject Alternative Name; repeat once per URI. Must be an absolute URI (e.g. urn:goat-galaxy:vendor=example-org). Used by per-peer identity schemes (trial X.509, ADR 0844 production).")
	flag.Parse()

	if err := run(*rootCertPath, *rootKeyPath, *ageIdentity, *caID, *siteID, *org, *cn, *validity, *ekuFlag, *keyType, *outCert, *outKey, *auditLog, *intermediateCertPath, *intermediateKeyPath, sans, uriSANs); err != nil {
		fatal("%v", err)
	}
}

func run(rootCertPath, rootKeyPath, ageIdentityPath, caID, siteID, org, cn string, validity time.Duration, ekuFlag, keyTypeFlag, outCertPath, outKeyPath, auditLogPath, intermediateCertPath, intermediateKeyPath string, sans, uriSANs []string) error {
	switch {
	case rootCertPath == "":
		return errors.New("--root-cert is required")
	case rootKeyPath == "":
		return errors.New("--root-key is required")
	case ageIdentityPath == "":
		return errors.New("--age-identity is required")
	case siteID == "":
		return errors.New("--site-id is required (Subject OU)")
	case cn == "":
		return errors.New("--cn is required")
	case len(sans) == 0:
		return errors.New("--san is required (peers verify SAN, not CN); pass at least once")
	case outCertPath == "":
		return errors.New("--out-cert is required")
	case outKeyPath == "":
		return errors.New("--out-key is required")
	}
	if (intermediateCertPath == "") != (intermediateKeyPath == "") {
		return errors.New("--intermediate-cert and --intermediate-key must be set together (or both empty for root-signing)")
	}
	if caID == "" {
		caID = strings.TrimSuffix(filepath.Base(rootCertPath), ".pem")
	}

	// Load root cert.
	rootBytes, err := os.ReadFile(rootCertPath)
	if err != nil {
		return fmt.Errorf("read root cert: %w", err)
	}
	rootCert, err := x509ca.LoadRootCert(rootBytes)
	if err != nil {
		return fmt.Errorf("load root cert: %w", err)
	}

	// Open signer.
	identities, err := loadAgeIdentities(ageIdentityPath)
	if err != nil {
		return fmt.Errorf("load age identity: %w", err)
	}
	signer, err := ca.NewAgeSigner(caID, rootKeyPath, identities)
	if err != nil {
		return fmt.Errorf("open ca signer: %w", err)
	}
	defer signer.Close() //nolint:errcheck // signing result is complete before signer cleanup

	// Sanity: root cert pubkey matches the signer pubkey. If not, the cert
	// won't chain — fail loudly instead of producing garbage output.
	rootPubBytes, err := x509.MarshalPKIXPublicKey(rootCert.PublicKey)
	if err != nil {
		return fmt.Errorf("marshal root cert pubkey: %w", err)
	}
	signerPubBytes, err := x509.MarshalPKIXPublicKey(signer.PublicKey())
	if err != nil {
		return fmt.Errorf("marshal signer pubkey: %w", err)
	}
	if !bytesEqual(rootPubBytes, signerPubBytes) {
		return errors.New("root cert pubkey does not match signing key — wrong --root-cert / --root-key pair")
	}

	eku, err := parseEKU(ekuFlag)
	if err != nil {
		return err
	}

	keyType, err := parseKeyType(keyTypeFlag, eku)
	if err != nil {
		return err
	}

	tpl := &x509ca.LeafTemplate{
		CommonName:   cn,
		Organization: org, // empty: x509ca.DefaultOrganization
		OrgUnit:      siteID,
		SANs:         sans,
		URISANs:      uriSANs,
		Validity:     validity,
		EKU:          eku,
		KeyType:      keyType,
	}

	// Sign path: intermediate when both flags
	// are set; root direct otherwise.
	var issued *x509ca.Issued
	if intermediateCertPath != "" {
		intCert, intKey, ierr := loadIntermediate(intermediateCertPath, intermediateKeyPath, identities)
		if ierr != nil {
			return fmt.Errorf("load intermediate: %w", ierr)
		}
		issued, err = x509ca.IssueCertViaIntermediate(tpl, intKey, intCert)
		if err != nil {
			return fmt.Errorf("issue cert via intermediate: %w", err)
		}
	} else {
		issued, err = x509ca.IssueCert(tpl, signer, rootCert)
		if err != nil {
			return fmt.Errorf("issue cert: %w", err)
		}
	}

	if err := writeWithMode(outCertPath, issued.CertPEM, 0o644); err != nil {
		return fmt.Errorf("write cert: %w", err)
	}
	if err := writeWithMode(outKeyPath, issued.KeyPEM, 0o400); err != nil {
		return fmt.Errorf("write key: %w", err)
	}

	if err := appendAuditLog(auditLogPath, auditEntry{
		Timestamp:    time.Now().UTC().Format(time.RFC3339),
		CAID:         caID,
		Action:       "issue-leaf",
		SiteID:       siteID,
		CommonName:   cn,
		SANs:         sans,
		URISANs:      uriSANs,
		Validity:     validity.String(),
		EKU:          ekuFlag,
		Serial:       issued.SerialNumber.String(),
		NotAfter:     issued.NotAfter.Format(time.RFC3339),
		OutCertPath:  outCertPath,
		OutKeyPath:   outKeyPath,
		RootCertPath: rootCertPath,
	}); err != nil {
		return fmt.Errorf("append audit log: %w", err)
	}

	fmt.Fprintf(os.Stderr,
		"\nLeaf cert issued.\n"+
			"  ca_id:    %s\n"+
			"  cn:       %s\n"+
			"  serial:   %s\n"+
			"  not_after: %s\n"+
			"  cert:     %s (mode 0644)\n"+
			"  key:      %s (mode 0400)\n"+
			"  audit:    %s\n"+
			"\n"+
			"Next steps:\n"+
			"  1. Verify: openssl x509 -in %s -noout -text | grep -E \"Subject:|DNS:|IP:|Not After\"\n"+
			"  2. Push to host via Ansible (traefik role):\n"+
			"     ansible-playbook ... -t traefik -e traefik_cert_files='{\"<svc>\":{\"cert\":\"%s\",\"key\":\"%s\"}}'\n",
		caID, cn, issued.SerialNumber, issued.NotAfter.Format(time.RFC3339),
		outCertPath, outKeyPath, auditLogPath, outCertPath, outCertPath, outKeyPath)

	return nil
}

// loadIntermediate reads + age-decrypts the intermediate keypair for
// the F-090 closure path. The cert is plaintext PEM (public material);
// the key is age-encrypted PKCS8 PEM (decryptable by the same age
// recipients that hold the root). Returns the parsed cert + the
// ECDSA-P256 private key ready to sign leaves.
func loadIntermediate(certPath, keyPath string, identities []age.Identity) (*x509.Certificate, *ecdsa.PrivateKey, error) {
	certBytes, err := os.ReadFile(certPath)
	if err != nil {
		return nil, nil, fmt.Errorf("read intermediate cert: %w", err)
	}
	cert, err := x509ca.LoadRootCert(certBytes) // accepts any IsCA=true cert; reuses root-loader.
	if err != nil {
		return nil, nil, fmt.Errorf("parse intermediate cert: %w", err)
	}

	encBytes, err := os.ReadFile(keyPath)
	if err != nil {
		return nil, nil, fmt.Errorf("read intermediate key: %w", err)
	}
	dec, err := age.Decrypt(bytes.NewReader(encBytes), identities...)
	if err != nil {
		return nil, nil, fmt.Errorf("age-decrypt intermediate key: %w", err)
	}
	pemBytes, err := io.ReadAll(dec)
	if err != nil {
		return nil, nil, fmt.Errorf("read decrypted intermediate key: %w", err)
	}
	block, _ := pem.Decode(pemBytes)
	if block == nil || block.Type != "PRIVATE KEY" {
		return nil, nil, errors.New("intermediate key PEM block missing or wrong type (want PRIVATE KEY)")
	}
	parsed, err := x509.ParsePKCS8PrivateKey(block.Bytes)
	if err != nil {
		return nil, nil, fmt.Errorf("parse intermediate PKCS8 key: %w", err)
	}
	ecKey, ok := parsed.(*ecdsa.PrivateKey)
	if !ok {
		return nil, nil, fmt.Errorf("intermediate key is not ECDSA-P256 (got %T)", parsed)
	}
	return cert, ecKey, nil
}

type auditEntry struct {
	Timestamp    string   `json:"timestamp"`
	CAID         string   `json:"ca_id"`
	Action       string   `json:"action"`
	SiteID       string   `json:"site_id"`
	CommonName   string   `json:"cn"`
	SANs         []string `json:"sans"`
	URISANs      []string `json:"uri_sans,omitempty"`
	Validity     string   `json:"validity"`
	EKU          string   `json:"eku"`
	Serial       string   `json:"serial"`
	NotAfter     string   `json:"not_after"`
	OutCertPath  string   `json:"out_cert_path"`
	OutKeyPath   string   `json:"out_key_path"`
	RootCertPath string   `json:"root_cert_path"`
}

func appendAuditLog(path string, entry auditEntry) (err error) {
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return err
	}
	f, err := os.OpenFile(path, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o600)
	if err != nil {
		return err
	}
	defer func() {
		err = errors.Join(err, f.Close())
	}()
	enc := json.NewEncoder(f)
	return enc.Encode(entry)
}

func writeWithMode(path string, data []byte, mode os.FileMode) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return err
	}
	if err := os.WriteFile(path, data, mode); err != nil {
		return err
	}
	// WriteFile may not enforce mode when file pre-exists; make it explicit.
	return os.Chmod(path, mode)
}

func parseKeyType(s string, eku []x509.ExtKeyUsage) (x509ca.LeafKeyType, error) {
	switch strings.ToLower(strings.TrimSpace(s)) {
	case "auto", "":
		// Auto: ECDSA-P256 for serverAuth (operator-facing TLS, must
		// be browser-compatible — F-071 + F-089); Ed25519 otherwise
		// (clientAuth and similar non-browser EKUs are fine with the
		// smaller, faster Ed25519 keys). The default flipped from
		// "ed25519" to "auto" to close F-089 — operators
		// were forgetting to pass -key-type ecdsa-p256 explicitly,
		// producing leaves macOS browsers silently fail-close on.
		for _, u := range eku {
			if u == x509.ExtKeyUsageServerAuth {
				return x509ca.LeafKeyECDSAP256, nil
			}
		}
		return x509ca.LeafKeyEd25519, nil
	case "ed25519":
		return x509ca.LeafKeyEd25519, nil
	case "ecdsa-p256", "ecdsa", "p256":
		return x509ca.LeafKeyECDSAP256, nil
	default:
		return 0, fmt.Errorf("unknown --key-type %q (supported: auto, ed25519, ecdsa-p256)", s)
	}
}

func parseEKU(s string) ([]x509.ExtKeyUsage, error) {
	parts := strings.Split(s, ",")
	out := make([]x509.ExtKeyUsage, 0, len(parts))
	for _, p := range parts {
		switch strings.TrimSpace(p) {
		case "serverAuth":
			out = append(out, x509.ExtKeyUsageServerAuth)
		case "clientAuth":
			out = append(out, x509.ExtKeyUsageClientAuth)
		case "":
			continue
		default:
			return nil, fmt.Errorf("unknown EKU %q (supported: serverAuth, clientAuth)", p)
		}
	}
	if len(out) == 0 {
		return nil, errors.New("at least one EKU is required")
	}
	return out, nil
}

func loadAgeIdentities(path string) ([]age.Identity, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer f.Close()
	return age.ParseIdentities(f)
}

func bytesEqual(a, b []byte) bool {
	if len(a) != len(b) {
		return false
	}
	for i := range a {
		if a[i] != b[i] {
			return false
		}
	}
	return true
}

func fatal(format string, args ...any) {
	fmt.Fprintf(os.Stderr, "cert-sign: "+format+"\n", args...)
	os.Exit(1)
}
