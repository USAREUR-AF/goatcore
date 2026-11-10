// prod.go — `ca-init --mode prod` handler. One-time YubiKey-PIV CA
// provisioning ceremony per ADR 0406.
//
// Generates a fresh Ed25519 keypair INSIDE the YubiKey (slot 9c by
// default; touch-policy=always; pin-policy=once). The private key
// never leaves the secure element; even this code only sees the
// public key. Writes the pubkey PEM to stdout for distribution to
// management hosts (operator pipes to ops/enrollment/public-keys/).
//
// Sequencing during ceremony:
//   1. Operator inserts YubiKey on the air-gapped issuing workstation.
//   2. Run `ca-init --mode prod --id <ca-id> [--gen-root-cert <path>]`.
//   3. Tool prompts for PIN (or reads from GOAT_CA_PIN).
//   4. piv-go opens the card, generates the keypair (LED tap required),
//      installs a self-signed cert wrapping the pubkey in slot 9c.
//   5. Tool reads the pubkey back from the cert (sanity check) +
//      prints PEM to stdout.
//   6. Operator saves to ops/enrollment/public-keys/<ca-id>.pem,
//      commits to git, distributes via Ansible.

package main

import (
	"crypto/x509"
	"encoding/hex"
	"encoding/pem"
	"fmt"
	"os"
	"strings"
	"syscall"
	"time"

	"github.com/go-piv/piv-go/v2/piv"
	"golang.org/x/term"

	"github.com/usareur-af/goatcore/ops/enrollment/ca"
	"github.com/usareur-af/goatcore/ops/enrollment/x509ca"
)

func runProd(idFlag, cardSelector, pivSlotName, mgmtKeyHex, pinFlag, genRootCertPath, rootCN, rootOU, org, slotCertSubject, pubKeyDir string) {
	caID := idFlag
	if caID == "" {
		caID = "prod-" + ca.DefaultCAIDStem + "-" + time.Now().UTC().Format("2006-01-02")
	} else if !strings.HasPrefix(caID, "prod-") && !strings.HasPrefix(caID, "test-") {
		caID = "prod-" + caID
	}

	pubPath := publicKeyPath(pubKeyDir, caID)
	if pubPath != "" {
		if err := refuseExisting(pubPath); err != nil {
			fatal("public key: %v", err)
		}
	}

	slot, err := parsePIVSlot(pivSlotName)
	if err != nil {
		fatal("--piv-slot: %v", err)
	}

	var mgmtKey []byte
	if mgmtKeyHex != "" {
		raw, err := hex.DecodeString(mgmtKeyHex)
		if err != nil {
			fatal("--management-key: hex decode: %v", err)
		}
		if len(raw) != 24 {
			fatal("--management-key: expected 24 bytes, got %d", len(raw))
		}
		mgmtKey = raw
	}

	pin, err := resolvePIN(pinFlag)
	if err != nil {
		fatal("PIN: %v", err)
	}

	fmt.Fprintf(os.Stderr, "ca-init: generating Ed25519 keypair on YubiKey (slot %X). Touch the LED when it blinks.\n", slot.Key)

	pub, cert, err := ca.YubikeyGenerate(ca.YubikeyGenerateOptions{
		CardSelector:  cardSelector,
		Slot:          slot,
		ManagementKey: mgmtKey,
		PIN:           pin,
		CAID:          caID,
		CertSubject:   slotCertSubject,
	})
	if err != nil {
		fatal("YubikeyGenerate: %v", err)
	}

	spki, err := x509.MarshalPKIXPublicKey(pub)
	if err != nil {
		fatal("marshal pubkey: %v", err)
	}
	block := &pem.Block{
		Type: "PUBLIC KEY",
		Headers: map[string]string{
			"CA-ID":      caID,
			"Created-At": time.Now().UTC().Format(time.RFC3339),
			"Backend":    "yubikey-piv-slot-" + pivSlotName,
		},
		Bytes: spki,
	}
	pemBytes := pem.EncodeToMemory(block)
	if _, err := os.Stdout.Write(pemBytes); err != nil {
		fatal("emit pem: %v", err)
	}
	persistPublicKey(pubPath, pemBytes)

	if genRootCertPath != "" {
		// Re-open the YubiKey through NewYubikeySigner so x509ca's
		// generic GenerateRootCert can sign with the just-provisioned
		// slot. Use a fresh PIN-gated session so cached state from
		// YubikeyGenerate doesn't leak into the cert-generation step.
		signer, err := ca.NewYubikeySigner(ca.YubikeyOptions{
			CardSelector: cardSelector,
			Slot:         slot,
			PIN:          pin,
			CAID:         caID,
		})
		if err != nil {
			fatal("re-open YubiKey for root-cert generation: %v", err)
		}
		defer signer.Close() //nolint:errcheck // generated certificate result is complete before signer cleanup
		fmt.Fprintf(os.Stderr, "ca-init: generating X.509 root cert (touch-policy=always; tap the YubiKey when prompted).\n")
		cn, derived := rootSubjectCN(rootCN, caID)
		if derived {
			warnDerivedCN(cn)
		}
		rootPEM, _, err := x509ca.GenerateRootCert(signer, x509ca.RootCertOptions{
			CommonName:   cn,
			OrgUnit:      rootOU,
			Organization: org,
		})
		if err != nil {
			fatal("generate root cert: %v", err)
		}
		if err := os.WriteFile(genRootCertPath, rootPEM, 0o644); err != nil { //nolint:gosec // root cert is public material.
			fatal("write root cert: %v", err)
		}
	}

	fmt.Fprintf(os.Stderr, "\nProduction CA initialized on YubiKey.\n")
	fmt.Fprintf(os.Stderr, "  ca_id:       %s\n", caID)
	fmt.Fprintf(os.Stderr, "  slot:        %s\n", pivSlotName)
	fmt.Fprintf(os.Stderr, "  cert subject: %s\n", cert.Subject.CommonName)
	fmt.Fprintf(os.Stderr, "  cert valid:  %s → %s\n", cert.NotBefore.Format("2006-01-02"), cert.NotAfter.Format("2006-01-02"))
	if genRootCertPath != "" {
		fmt.Fprintf(os.Stderr, "  X.509 root:  %s\n", genRootCertPath)
	}
	fmt.Fprintf(os.Stderr,
		"\nNext steps:\n"+
			"  1. %s\n"+
			"  2. Commit the public-key PEM (Ansible distributes it to mgmt hosts).\n"+
			"  3. Issue bundles via `bundle-create --mode prod --ca-id %s [--piv-slot %s]`.\n",
		pubKeyStep(pubPath, caID), caID, pivSlotName)
	if genRootCertPath != "" {
		fmt.Fprintf(os.Stderr, "  4. Commit %s — peers add it as bundle tls_trust_anchor (ADR 0407).\n", genRootCertPath)
	}
	fmt.Fprintf(os.Stderr,
		"\nKey custody reminders:\n"+
			"  - The CA private key is now on this YubiKey ONLY. Lose the YubiKey =\n"+
			"    lose the ability to sign new bundles. Existing bundles remain valid\n"+
			"    until their 90d TTL.\n"+
			"  - Provision a backup YubiKey via the same ceremony with a different\n"+
			"    --id (or rotate the existing CA on schedule per ADR 0406).\n")
}

func resolvePIN(flagVal string) (string, error) {
	if flagVal != "" {
		return flagVal, nil
	}
	if env := os.Getenv("GOAT_CA_PIN"); env != "" {
		return env, nil
	}
	if !term.IsTerminal(int(syscall.Stdin)) {
		return "", fmt.Errorf("PIN required: pass --pin, set GOAT_CA_PIN, or run on a terminal")
	}
	fmt.Fprint(os.Stderr, "Enter YubiKey PIV PIN: ")
	pin, err := term.ReadPassword(int(syscall.Stdin))
	fmt.Fprintln(os.Stderr)
	if err != nil {
		return "", fmt.Errorf("read PIN: %w", err)
	}
	return string(pin), nil
}

// parsePIVSlot accepts "9a" / "9c" / "9d" / "9e" / hex equivalents and
// returns the corresponding piv.Slot. Slot 9c (Digital Signature) is
// the canonical choice for the offline CA per ADR 0406.
func parsePIVSlot(name string) (piv.Slot, error) {
	switch strings.ToLower(strings.TrimPrefix(name, "0x")) {
	case "9a", "0x9a":
		return piv.SlotAuthentication, nil
	case "9c", "0x9c", "":
		return piv.SlotSignature, nil
	case "9d", "0x9d":
		return piv.SlotKeyManagement, nil
	case "9e", "0x9e":
		return piv.SlotCardAuthentication, nil
	}
	return piv.Slot{}, fmt.Errorf("unknown slot %q (valid: 9a, 9c, 9d, 9e)", name)
}
