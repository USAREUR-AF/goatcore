// Command ca-init bootstraps a dev-mode CA.
//
// Usage:
//
//	ca-init \
//	    --mode dev \
//	    --out ops/enrollment/ca/dev/ca.key.age \
//	    --recipients <age-pub-1>,<age-pub-2> \
//	    [--id dev-<goatnet>-ca-YYYY-MM-DD] \
//	    [--gen-root-cert ops/enrollment/ca/dev/root-cert.pem] \
//	    [--age-identity ~/.config/age/keys.txt] \
//	    [--root-cn "<goatnet> Root CA"] \
//	    [--public-key-dir ops/enrollment/public-keys] \
//	    [--allow-single-recipient]
//
//	ca-init --verify --out <ca.key.age> [--root-cert <root.pem>] [--public-key <ca.pem>]
//
// Dev mode refuses fewer than two age recipients (see minCARecipients) unless
// --allow-single-recipient is set. --verify checks an existing CA without
// minting anything; see verify.go.
//
// Writes an age-encrypted Ed25519 private key to --out, prints the PEM-
// encoded public key on stdout. With --public-key-dir it also writes
// <dir>/<ca_id>.pem, never overwriting; ops/enrollment/public-keys/ is the
// directory Ansible distributes to management hosts, and validators trust
// any public key present there.
//
// When --gen-root-cert is provided, also generates an X.509 v3 self-signed
// root certificate from the CA key and writes it to that path. The same
// cert ships in every offline-enrollment bundle as the tls_trust_anchor
// field (per ADR 0407 / docs/design/internal-tls-ca.md). --age-identity
// is required when --gen-root-cert is set so the freshly-encrypted key
// can be decrypted in the same ceremony.
//
// Production (YubiKey) is a separate ceremony performed on an air-gapped
// workstation; see docs/operations/enrollment-bundle-runbook.md.
package main

import (
	"crypto/x509"
	"encoding/pem"
	"flag"
	"fmt"
	"os"
	"strings"
	"time"

	"filippo.io/age"

	"github.com/usareur-af/goatcore/ops/enrollment/ca"
	"github.com/usareur-af/goatcore/ops/enrollment/x509ca"
)

func main() {
	mode := flag.String("mode", "dev", "CA mode: dev (age-encrypted file) or prod (YubiKey PIV slot 9c per ADR 0406)")
	out := flag.String("out", "", "Path to write the age-encrypted CA private key (dev mode only)")
	recipientsFlag := flag.String("recipients", "", "Comma-separated age X25519 recipient public keys (format: age1...)")
	idFlag := flag.String("id", "", "CA identifier; dev mode prefixes 'dev-', prod mode prefixes 'prod-' if absent. Defaults to <prefix>"+ca.DefaultCAIDStem+"-<date>")
	genRootCert := flag.String("gen-root-cert", "", "Optional. Path to write a self-signed X.509 v3 root cert generated from the CA key (Block 22J / ADR 0407). Distributes as bundle tls_trust_anchor.")
	ageIdentity := flag.String("age-identity", "", "Path to age identity file (dev mode only; required if --gen-root-cert is set, to decrypt the just-written key)")

	// Root subject identity. An empty --root-cn is derived from the CA id
	// ("<ca_id> Root CA"); already-minted roots keep their subject
	// because ca-init never re-issues an existing root. Goatnets must name
	// themselves: roots that share a byte-identical subject differ only by SPKI
	// and serial, so `openssl x509 -subject` on a leaf's issuer cannot tell an
	// operator WHICH goatnet's root signed it, and a router cert can chain to the
	// wrong root without anyone noticing at issuance time. A per-goatnet CN makes
	// the lineage readable in every trust store, chain dump and browser dialog.
	rootCN := flag.String("root-cn", "", "Optional. Subject CommonName for the generated root cert. Defaults to \"<ca_id> Root CA\". Prefer a readable name for a new goatnet — e.g. \"galaxy-prod Root CA\" — so the root is self-identifying in a chain dump.")
	allowSingleRecipient := flag.Bool("allow-single-recipient", false, "Dev mode. Accept a single age recipient. A CA key encrypted to one recipient is unrecoverable if that identity is lost; use only for a throwaway test CA.")
	verify := flag.Bool("verify", false, "Check an existing CA instead of minting: the --out key's recipient count, and with --root-cert / --public-key the root subject CN and that the public key is the root's. Exit 1 on any problem.")
	verifyRootCert := flag.String("root-cert", "", "With --verify: path to the existing root cert PEM.")
	verifyPublicKey := flag.String("public-key", "", "With --verify: path to the existing CA public key PEM (the <ca_id>.pem ca-init wrote).")
	pubKeyDir := flag.String("public-key-dir", "", "Directory to write <ca_id>.pem (the CA public key) into; for a goatnet CA, "+publicKeysRel+". Refuses to overwrite. Empty: stdout only, with a warning.")
	rootOU := flag.String("root-ou", "", "Optional. Subject OrganizationalUnit for the generated root cert. Defaults to \"offline-ca\".")
	org := flag.String("org", "", "Optional. Subject Organization (O) for the generated root and intermediate certs. Empty: the x509ca default. cert-sign takes the same flag for leaves.")
	intermediateCN := flag.String("intermediate-cn", "", "Optional. Subject CommonName for the generated intermediate cert. Empty: the x509ca default.")
	sharedRootCN := flag.String("shared-root-cn", x509ca.DefaultRootCommonName, "With --verify: a root CommonName that names no goatnet because several roots carry it; a root with this CN fails the check.")
	slotCertSubject := flag.String("slot-cert-subject", "", "Prod mode. CommonName of the self-signed certificate installed in the PIV slot. Empty: \""+ca.YubikeyCertSubjectPrefix+"<ca_id>\".")

	// Intermediate-cert tier. Produces an
	// ECDSA-P256 X.509 intermediate signed by the dev CA root. Operators
	// install the INTERMEDIATE in their macOS keychain (which accepts
	// ECDSA roots — Ed25519 roots fail-import). cert-sign uses
	// --intermediate-cert / --intermediate-key flags to sign leaves
	// chained through the intermediate; bundle signing remains Ed25519
	// signed by the root directly (decoupled, see F-090 §"constraint").
	genIntermediateCert := flag.String("gen-intermediate-cert", "", "Optional. Path to write an ECDSA-P256 X.509 intermediate cert signed by the CA root. Pair with --intermediate-out-key. Closes F-090 (macOS Keychain rejects Ed25519 roots).")
	intermediateOutKey := flag.String("intermediate-out-key", "", "Path to write the age-encrypted intermediate private key (mode 0600). Required when --gen-intermediate-cert is set.")
	intermediateOnly := flag.Bool("intermediate-only", false, "When true, skip root key + root cert generation and use the EXISTING root at --out + --gen-root-cert paths. Use this to add an intermediate to an already-provisioned root WITHOUT overwriting it. Requires --age-identity to decrypt the existing root key.")

	// prod-mode flags
	cardSelector := flag.String("card-selector", "", "Prod mode. Substring match against PCSC reader names; empty = first card found.")
	pivSlotName := flag.String("piv-slot", "9c", "Prod mode. PIV slot for the CA key. Default 9c (Digital Signature). Hex string per PIV spec.")
	mgmtKeyHex := flag.String("management-key", "", "Prod mode. PIV management key as hex (24 bytes). Empty = piv.DefaultManagementKey (only safe on a fresh / dedicated YubiKey).")
	pinFlag := flag.String("pin", "", "Prod mode. YubiKey PIV PIN. If empty, reads from GOAT_CA_PIN env var, else prompts on stdin.")

	flag.Parse()

	if *verify {
		runVerify(*out, *verifyRootCert, *verifyPublicKey, *sharedRootCN)
		return
	}

	switch *mode {
	case "dev":
		runDev(*out, *recipientsFlag, *idFlag, *genRootCert, *ageIdentity, *genIntermediateCert, *intermediateOutKey, *intermediateOnly, *rootCN, *rootOU, *org, *intermediateCN, *pubKeyDir, *allowSingleRecipient)
	case "prod":
		runProd(*idFlag, *cardSelector, *pivSlotName, *mgmtKeyHex, *pinFlag, *genRootCert, *rootCN, *rootOU, *org, *slotCertSubject, *pubKeyDir)
	default:
		fatal("unknown --mode %q (valid: dev, prod)", *mode)
	}
}

func runDev(out, recipientsFlag, idFlag, genRootCertPath, ageIdentityPath, genIntermediateCertPath, intermediateOutKeyPath string, intermediateOnly bool, rootCN, rootOU, org, intermediateCN, pubKeyDir string, allowSingleRecipient bool) {
	if out == "" {
		fatal("--out is required for dev mode (path to age-encrypted root key — existing or new)")
	}
	if !intermediateOnly && recipientsFlag == "" {
		fatal("--recipients is required for dev mode unless --intermediate-only is set")
	}
	if genRootCertPath != "" && ageIdentityPath == "" {
		fatal("--age-identity is required when --gen-root-cert is set (need to decrypt the just-written key for X.509 root signing)")
	}
	if genIntermediateCertPath != "" {
		if genRootCertPath == "" {
			fatal("--gen-intermediate-cert requires --gen-root-cert (intermediate is signed by the root)")
		}
		if intermediateOutKeyPath == "" {
			fatal("--gen-intermediate-cert requires --intermediate-out-key")
		}
		if ageIdentityPath == "" {
			fatal("--age-identity is required when --gen-intermediate-cert is set (need to decrypt the root key to sign the intermediate, and the same recipients encrypt the intermediate key)")
		}
	}
	if intermediateOnly {
		if genIntermediateCertPath == "" || intermediateOutKeyPath == "" {
			fatal("--intermediate-only requires --gen-intermediate-cert + --intermediate-out-key (those are the only artifacts this mode produces)")
		}
		if recipientsFlag == "" {
			fatal("--intermediate-only still requires --recipients (the intermediate KEY is age-encrypted to those recipients; reuse the same set as the existing root)")
		}
	}

	caID := idFlag
	if caID == "" {
		caID = "dev-" + ca.DefaultCAIDStem + "-" + time.Now().UTC().Format("2006-01-02")
	} else if !strings.HasPrefix(caID, "dev-") {
		caID = "dev-" + caID
	}

	recipients, err := parseRecipients(recipientsFlag)
	if err != nil {
		fatal("parse recipients: %v", err)
	}
	// Enforced here rather than in ca.GenerateAndWrite: the library is also
	// called by tests and tools with one throwaway recipient, and the floor is
	// a ceremony rule for this command, not a property of the key format.
	if err := checkRecipientFloor(len(recipients), allowSingleRecipient); err != nil {
		fatal("%v", err)
	}
	if len(recipients) < minCARecipients {
		fmt.Fprintf(os.Stderr, "ca-init: WARNING: --allow-single-recipient set; this CA key is encrypted to a single age recipient and is lost with that one identity\n")
	}

	pubPath := ""
	if !intermediateOnly {
		pubPath = publicKeyPath(pubKeyDir, caID)
	}
	if pubPath != "" {
		if err := refuseExisting(pubPath); err != nil {
			fatal("public key: %v", err)
		}
	}

	if !intermediateOnly {
		pub, err := ca.GenerateAndWrite(out, recipients)
		if err != nil {
			fatal("generate ca key: %v", err)
		}

		spki, err := x509.MarshalPKIXPublicKey(pub)
		if err != nil {
			fatal("marshal public key (SPKI): %v", err)
		}

		block := &pem.Block{
			Type: "PUBLIC KEY",
			Headers: map[string]string{
				"CA-ID":      caID,
				"Created-At": time.Now().UTC().Format(time.RFC3339),
				"Note":       "dev CA, not for production bundles",
			},
			Bytes: spki,
		}
		pemBytes := pem.EncodeToMemory(block)
		if _, err := os.Stdout.Write(pemBytes); err != nil {
			fatal("emit pem: %v", err)
		}
		persistPublicKey(pubPath, pemBytes)

		// Optional X.509 root cert generation (Block 22J / ADR 0407).
		if genRootCertPath != "" {
			cn, derived := rootSubjectCN(rootCN, caID)
			if derived {
				warnDerivedCN(cn)
			}
			emitRootCert(out, ageIdentityPath, caID, genRootCertPath, cn, rootOU, org)
		}
	} else {
		// Intermediate-only: don't touch the existing root key or the
		// existing root cert. Just emit the intermediate.
		fmt.Fprintf(os.Stderr, "ca-init: intermediate-only mode — preserving existing root at %s + %s\n", out, genRootCertPath)
	}

	// Optional ECDSA-P256 intermediate generation.
	if genIntermediateCertPath != "" {
		emitIntermediate(out, ageIdentityPath, recipients, caID, genRootCertPath, genIntermediateCertPath, intermediateOutKeyPath, org, intermediateCN)
	}

	fmt.Fprintf(os.Stderr,
		"\nCA initialized.\n"+
			"  ca_id:                 %s\n"+
			"  encrypted private key: %s\n",
		caID, out)
	if genRootCertPath != "" {
		fmt.Fprintf(os.Stderr, "  X.509 root cert:       %s\n", genRootCertPath)
	}
	if genIntermediateCertPath != "" {
		fmt.Fprintf(os.Stderr, "  X.509 intermediate:    %s\n", genIntermediateCertPath)
		fmt.Fprintf(os.Stderr, "  intermediate key:      %s (age-encrypted)\n", intermediateOutKeyPath)
	}
	fmt.Fprintf(os.Stderr,
		"\n"+
			"Next steps:\n"+
			"  1. %s\n"+
			"  2. Commit the public-key PEM so Ansible can distribute it.\n"+
			"  3. Do NOT commit %s — it belongs only on the issuing operator's\n"+
			"     workstation.\n",
		pubKeyStep(pubPath, caID), out)
	if genRootCertPath != "" {
		fmt.Fprintf(os.Stderr,
			"  4. Commit %s — peers add it as bundle tls_trust_anchor (ADR 0407).\n"+
				"  5. Issue leaf TLS certs via cert-sign for each service host.\n",
			genRootCertPath)
	}
	fmt.Fprintf(os.Stderr,
		"\n"+
			"Dev mode is not for production bundles. Gate to switch off dev mode:\n"+
			"  - YubiKey 5.7.0+ in hand\n"+
			"  - Firmware verified (`ykman info`)\n"+
			"  - Production ceremony dry-run complete\n"+
			"  - Production public key distributed\n"+
			"  See docs/operations/enrollment-bundle-runbook.md for details.\n")
}

// emitRootCert generates a self-signed X.509 v3 root cert from the CA key
// (just written by GenerateAndWrite) and writes it to outPath. Aborts the
// process on error.
func emitRootCert(caKeyPath, ageIdentityPath, caID, outPath, rootCN, rootOU, org string) {
	identities, err := loadAgeIdentities(ageIdentityPath)
	if err != nil {
		fatal("load age identity: %v", err)
	}
	signer, err := ca.NewAgeSigner(caID, caKeyPath, identities)
	if err != nil {
		fatal("open ca signer: %v", err)
	}
	defer signer.Close() //nolint:errcheck // generated certificate result is complete before signer cleanup

	// Empty CommonName / OrgUnit / Organization fall through to x509ca's
	// defaults.
	rootPEM, _, err := x509ca.GenerateRootCert(signer, x509ca.RootCertOptions{
		CommonName:   rootCN,
		OrgUnit:      rootOU,
		Organization: org,
	})
	if err != nil {
		fatal("generate root cert: %v", err)
	}

	if err := os.WriteFile(outPath, rootPEM, 0o644); err != nil { //nolint:gosec // root cert is public material; mode 0644 is correct.
		fatal("write root cert: %v", err)
	}
}

// emitIntermediate generates an ECDSA-P256 X.509 intermediate cert
// signed by the offline-CA root, plus an age-encrypted intermediate
// private key. F-090 closure path B: macOS Keychain Services rejects
// Ed25519 root certs at import; ECDSA-P256 intermediate is importable
// and downstream leaves chain through it. Bundle signing remains the
// Ed25519 root path; only X.509 cert chains route through this
// intermediate.
//
// Reads the just-written root cert from rootCertPath (assumes
// emitRootCert already ran), opens the root signer via the same age
// identity, generates the intermediate, encrypts the intermediate's
// private key with the same age recipients (so any operator who can
// decrypt the root can also decrypt the intermediate), and writes the
// public cert + age-encrypted key to disk.
func emitIntermediate(caKeyPath, ageIdentityPath string, recipients []age.Recipient, caID, rootCertPath, intCertOutPath, intKeyOutPath, org, intermediateCN string) {
	identities, err := loadAgeIdentities(ageIdentityPath)
	if err != nil {
		fatal("load age identity: %v", err)
	}
	rootSigner, err := ca.NewAgeSigner(caID, caKeyPath, identities)
	if err != nil {
		fatal("open ca signer for intermediate: %v", err)
	}
	defer rootSigner.Close() //nolint:errcheck // generated certificate result is complete before signer cleanup

	rootBytes, err := os.ReadFile(rootCertPath)
	if err != nil {
		fatal("read root cert (for intermediate parent): %v", err)
	}
	rootCert, err := x509ca.LoadRootCert(rootBytes)
	if err != nil {
		fatal("parse root cert: %v", err)
	}

	intCertPEM, intKeyPEM, err := x509ca.GenerateIntermediate(rootSigner, rootCert, x509ca.IntermediateCertOptions{
		Organization: org,
		CommonName:   intermediateCN,
	})
	if err != nil {
		fatal("generate intermediate: %v", err)
	}

	// Public material — mode 0644.
	if err := os.WriteFile(intCertOutPath, intCertPEM, 0o644); err != nil { //nolint:gosec // intermediate cert is public material.
		fatal("write intermediate cert: %v", err)
	}

	// Private material — age-encrypt to the same recipients as the root.
	encryptedKey, err := encryptAgePEM(intKeyPEM, recipients)
	if err != nil {
		fatal("age-encrypt intermediate key: %v", err)
	}
	if err := os.WriteFile(intKeyOutPath, encryptedKey, 0o600); err != nil {
		fatal("write age-encrypted intermediate key: %v", err)
	}
}

// encryptAgePEM age-encrypts plaintext to all recipients, returning
// the encrypted blob suitable for on-disk storage. Mirrors the shape
// `ca.GenerateAndWrite` uses for the root key.
func encryptAgePEM(plaintext []byte, recipients []age.Recipient) ([]byte, error) {
	var buf strings.Builder
	enc, err := age.Encrypt(stringWriter{&buf}, recipients...)
	if err != nil {
		return nil, fmt.Errorf("age encrypt init: %w", err)
	}
	if _, err := enc.Write(plaintext); err != nil {
		return nil, fmt.Errorf("age encrypt write: %w", err)
	}
	if err := enc.Close(); err != nil {
		return nil, fmt.Errorf("age encrypt close: %w", err)
	}
	return []byte(buf.String()), nil
}

// stringWriter adapts strings.Builder to io.Writer for age.Encrypt.
type stringWriter struct{ b *strings.Builder }

func (w stringWriter) Write(p []byte) (int, error) { return w.b.Write(p) }

func loadAgeIdentities(path string) ([]age.Identity, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, fmt.Errorf("open age identity: %w", err)
	}
	defer f.Close()
	return age.ParseIdentities(f)
}

func parseRecipients(s string) ([]age.Recipient, error) {
	parts := strings.Split(s, ",")
	out := make([]age.Recipient, 0, len(parts))
	seen := make(map[string]bool)
	for _, p := range parts {
		p = strings.TrimSpace(p)
		if p == "" {
			continue
		}
		r, err := age.ParseX25519Recipient(p)
		if err != nil {
			return nil, fmt.Errorf("%q: %w", p, err)
		}
		if seen[r.String()] {
			return nil, fmt.Errorf("duplicate age recipient: each recovery key must be distinct")
		}
		seen[r.String()] = true
		out = append(out, r)
	}
	if len(out) == 0 {
		return nil, fmt.Errorf("no valid age recipients parsed")
	}
	return out, nil
}

// minCARecipients is the fewest age recipients a CA key may be encrypted to.
// One recipient means the CA is lost with one identity, which is the failure
// an offline CA exists to prevent. ADR 0406 (interim dev-CA mode) encrypts the
// CA key to a named cohort plus a break-glass copy; two is the smallest set
// that keeps a recovery path.
const minCARecipients = 2

// checkRecipientFloor returns an error when n is below minCARecipients and the
// operator has not explicitly accepted a single recipient. Zero recipients is
// refused even with the override.
func checkRecipientFloor(n int, allowSingle bool) error {
	if n >= minCARecipients {
		return nil
	}
	if n == 1 && allowSingle {
		return nil
	}
	return fmt.Errorf("a CA key needs at least %d age recipients (got %d) so that losing one identity does not lose the CA; add a second recipient such as the break-glass key, or pass --allow-single-recipient for a throwaway test CA", minCARecipients, n)
}

func fatal(format string, args ...any) {
	fmt.Fprintf(os.Stderr, "ca-init: "+format+"\n", args...)
	os.Exit(1)
}
