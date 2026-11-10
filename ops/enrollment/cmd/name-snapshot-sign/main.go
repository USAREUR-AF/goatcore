// name-snapshot-sign — detached-signs a name-fallback snapshot with the
// per-goatnet offline-CA key (docs/design/name-fallback-resolver.md, ADR 1082).
//
// The signature is ASN.1 DER ECDSA-Sig-Value (P-256) over SHA-256 of the
// exact snapshot file bytes — the same scheme and the same AgeSigner as
// enrollment-bundle signing (ops/enrollment/bundle/bundle.go). Output is
// base64(DER) + "\n" at <snapshot>.sig.
//
//	go run ./ops/enrollment/cmd/name-snapshot-sign \
//	    --snapshot ops/dns/<site>/name-snapshot.json \
//	    --root-key <ca-state-dir>/ca.key.age \
//	    --age-identity ~/.config/age/keys.txt
//
// The CA-ID is read from the snapshot's ca_id field; the signer's public key
// is cross-checked against ops/enrollment/public-keys/<ca_id>.pem so a wrong
// --root-key fails loudly instead of committing an unverifiable signature.
package main

import (
	"crypto/ecdsa"
	"crypto/sha256"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"encoding/pem"
	"errors"
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"time"

	"filippo.io/age"

	"github.com/usareur-af/goatcore/ops/enrollment/ca"
)

func main() {
	if err := run(); err != nil {
		fmt.Fprintf(os.Stderr, "name-snapshot-sign: %v\n", err)
		os.Exit(1)
	}
}

func run() error {
	var (
		snapshotPath    = flag.String("snapshot", "", "Path to name-snapshot.json (required)")
		rootKeyPath     = flag.String("root-key", "", "Path to the age-encrypted CA private key (required)")
		ageIdentityPath = flag.String("age-identity", "", "Path to the age identity file (required)")
		pubkeyDir       = flag.String("pubkey-dir", "", "Dir holding <ca_id>.pem SPKI public keys (default: ops/enrollment/public-keys next to this repo's snapshot path)")
		auditLog        = flag.String("audit-log", ca.DefaultAuditLogPath(), "Path to append-only JSON audit log")
	)
	flag.Parse()
	switch {
	case *snapshotPath == "":
		return errors.New("--snapshot is required")
	case *rootKeyPath == "":
		return errors.New("--root-key is required")
	case *ageIdentityPath == "":
		return errors.New("--age-identity is required")
	}

	raw, err := os.ReadFile(*snapshotPath)
	if err != nil {
		return fmt.Errorf("read snapshot: %w", err)
	}
	var snap struct {
		Format string `json:"format"`
		SiteID string `json:"site_id"`
		Serial int64  `json:"serial"`
		CAID   string `json:"ca_id"`
	}
	if err := json.Unmarshal(raw, &snap); err != nil {
		return fmt.Errorf("parse snapshot: %w", err)
	}
	if snap.Format != "goat-name-snapshot-v1" {
		return fmt.Errorf("unexpected snapshot format %q", snap.Format)
	}
	if snap.CAID == "" {
		return errors.New("snapshot has no ca_id")
	}

	idFile, err := os.Open(*ageIdentityPath)
	if err != nil {
		return fmt.Errorf("open age identity: %w", err)
	}
	identities, err := age.ParseIdentities(idFile)
	idFile.Close()
	if err != nil {
		return fmt.Errorf("parse age identities: %w", err)
	}

	signer, err := ca.NewAgeSigner(snap.CAID, *rootKeyPath, identities)
	if err != nil {
		return fmt.Errorf("open ca signer: %w", err)
	}
	defer signer.Close() //nolint:errcheck // signing result is complete before signer cleanup

	// Cross-check signer pubkey against the committed SPKI for this CA-ID —
	// a wrong --root-key must fail here, not at every client.
	if declared, perr := loadDeclaredPubkey(*pubkeyDir, *snapshotPath, snap.CAID); perr != nil {
		return perr
	} else if declared != nil && !declared.Equal(signer.PublicKey()) {
		return fmt.Errorf("signer public key does not match public-keys/%s.pem — wrong --root-key for this snapshot's ca_id", snap.CAID)
	}

	sig, err := signer.Sign(raw)
	if err != nil {
		return fmt.Errorf("sign: %w", err)
	}

	// Verify what we are about to write, with the declared/derived pubkey.
	digest := sha256.Sum256(raw)
	if !ecdsa.VerifyASN1(signer.PublicKey(), digest[:], sig) {
		return errors.New("self-verify failed — refusing to write signature")
	}

	sigPath := *snapshotPath + ".sig"
	if err := os.WriteFile(sigPath, []byte(base64.StdEncoding.EncodeToString(sig)+"\n"), 0o644); err != nil {
		return fmt.Errorf("write signature: %w", err)
	}

	entry := map[string]any{
		"ts":       time.Now().UTC().Format(time.RFC3339),
		"action":   "name-snapshot-sign",
		"ca_id":    snap.CAID,
		"site_id":  snap.SiteID,
		"serial":   snap.Serial,
		"snapshot": *snapshotPath,
		"sig":      sigPath,
	}
	if err := appendAuditLog(*auditLog, entry); err != nil {
		return fmt.Errorf("append audit log: %w", err)
	}

	fmt.Printf("signed %s (site %s, serial %d, ca %s) -> %s\n",
		*snapshotPath, snap.SiteID, snap.Serial, snap.CAID, sigPath)
	return nil
}

// loadDeclaredPubkey finds ops/enrollment/public-keys/<caID>.pem, walking up
// from the snapshot path to the repo root when --pubkey-dir is not given.
// Returns (nil, nil) when the file genuinely cannot be located (out-of-repo
// invocation) — the self-verify above still guards signature validity.
func loadDeclaredPubkey(dir, snapshotPath, caID string) (*ecdsa.PublicKey, error) {
	var path string
	if dir != "" {
		path = filepath.Join(dir, caID+".pem")
	} else {
		abs, err := filepath.Abs(snapshotPath)
		if err != nil {
			return nil, err
		}
		for d := filepath.Dir(abs); d != filepath.Dir(d); d = filepath.Dir(d) {
			candidate := filepath.Join(d, "ops", "enrollment", "public-keys", caID+".pem")
			if _, err := os.Stat(candidate); err == nil {
				path = candidate
				break
			}
		}
		if path == "" {
			return nil, nil
		}
	}
	raw, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("read declared pubkey %s: %w", path, err)
	}
	block, _ := pem.Decode(raw)
	if block == nil || block.Type != "PUBLIC KEY" {
		return nil, fmt.Errorf("%s: not a PUBLIC KEY PEM", path)
	}
	pub, err := x509.ParsePKIXPublicKey(block.Bytes)
	if err != nil {
		return nil, fmt.Errorf("parse declared pubkey: %w", err)
	}
	ec, ok := pub.(*ecdsa.PublicKey)
	if !ok {
		return nil, fmt.Errorf("%s: not an ECDSA public key", path)
	}
	return ec, nil
}

func appendAuditLog(path string, entry map[string]any) (err error) {
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
	return json.NewEncoder(f).Encode(entry)
}
