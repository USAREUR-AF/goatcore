package main

import (
	"bufio"
	"bytes"
	"crypto/x509"
	"encoding/pem"
	"errors"
	"fmt"
	"io"
	"os"
	"strings"

	"filippo.io/age/armor"

	"github.com/usareur-af/goatcore/ops/enrollment/x509ca"
)

// legacyRootCN is the subject a root carries when it was issued with no
// CommonName (the x509ca default), which makes roots of different goatnets
// indistinguishable in a chain dump. --shared-root-cn names another such
// subject, for a CA whose early roots share one.
const legacyRootCN = x509ca.DefaultRootCommonName

// runVerify checks an existing CA without minting anything and exits 1 when
// any check fails. The round-trip decrypt is not repeated here: it already
// runs at mint time, and these are the static checks that catch a CA minted
// with too few recipients, a default or unexpanded subject, or a stale public
// key file. Recipient stanzas are randomized and cannot prove distinct identities.
func runVerify(keyPath, rootCertPath, publicKeyPath, sharedRootCN string) {
	if keyPath == "" && rootCertPath == "" && publicKeyPath == "" {
		fatal("--verify needs at least one of --out (CA key), --root-cert, --public-key")
	}
	problems := verifyCA(keyPath, rootCertPath, publicKeyPath, sharedRootCN)
	for _, p := range problems {
		fmt.Fprintf(os.Stderr, "ca-init: verify: %s\n", p)
	}
	if len(problems) > 0 {
		os.Exit(1)
	}
	if keyPath != "" {
		fmt.Fprintln(os.Stderr, "ca-init: verify: recipient stanza count does not prove distinct recovery identities; verify independent custody separately")
	}
	fmt.Fprintln(os.Stderr, "ca-init: verify: OK")
}

// verifyCA returns one line per failed check. Empty paths skip their checks.
func verifyCA(keyPath, rootCertPath, publicKeyPath, sharedRootCN string) []string {
	var problems []string
	if keyPath != "" {
		n, err := countRecipientsInFile(keyPath)
		switch {
		case err != nil:
			problems = append(problems, fmt.Sprintf("CA key %s: %v", keyPath, err))
		case n < minCARecipients:
			problems = append(problems, fmt.Sprintf("CA key %s is encrypted to %d age recipient(s); want at least %d", keyPath, n, minCARecipients))
		}
	}

	var root *x509.Certificate
	if rootCertPath != "" {
		cert, err := readCertificate(rootCertPath)
		if err != nil {
			problems = append(problems, fmt.Sprintf("root cert %s: %v", rootCertPath, err))
		} else {
			root = cert
			if p := rootCNProblem(cert.Subject.CommonName, sharedRootCN); p != "" {
				problems = append(problems, fmt.Sprintf("root cert %s: %s", rootCertPath, p))
			}
		}
	}

	if publicKeyPath != "" {
		spki, err := readPublicKeySPKI(publicKeyPath)
		switch {
		case err != nil:
			problems = append(problems, fmt.Sprintf("public key %s: %v", publicKeyPath, err))
		case root != nil && !bytes.Equal(spki, root.RawSubjectPublicKeyInfo):
			problems = append(problems, fmt.Sprintf("public key %s does not match the root cert's public key (stale or from another CA)", publicKeyPath))
		}
	}
	return problems
}

// rootCNProblem names what is wrong with a root subject CN, or returns "".
func rootCNProblem(cn, sharedRootCN string) string {
	switch {
	case strings.TrimSpace(cn) == "":
		return "root CN is empty"
	case cn == legacyRootCN || (sharedRootCN != "" && cn == sharedRootCN):
		return fmt.Sprintf("root CN is the legacy shared default %q; a root must name its goatnet", cn)
	case strings.Contains(cn, "$"):
		return fmt.Sprintf("root CN %q contains '$', an unexpanded shell variable", cn)
	}
	return ""
}

func countRecipientsInFile(path string) (int, error) {
	f, err := os.Open(path)
	if err != nil {
		return 0, err
	}
	defer f.Close()
	return countAgeRecipientStanzas(f)
}

// countAgeRecipientStanzas reads an age file header (binary or ASCII-armored)
// and counts its recipient stanzas. Grease stanzas, whose type ends in
// "-grease", carry no key and are not counted. Nothing is decrypted.
func countAgeRecipientStanzas(r io.Reader) (int, error) {
	br := bufio.NewReader(r)
	if peek, err := br.Peek(len(armor.Header)); err == nil && string(peek) == armor.Header {
		br = bufio.NewReader(armor.NewReader(br))
	}
	first, err := br.ReadString('\n')
	if err != nil || strings.TrimRight(first, "\n") != "age-encryption.org/v1" {
		return 0, errors.New("not an age v1 file")
	}
	n := 0
	for {
		line, err := br.ReadString('\n')
		if err != nil {
			return 0, errors.New("age header ends before its MAC line")
		}
		switch {
		case strings.HasPrefix(line, "---"):
			return n, nil
		case strings.HasPrefix(line, "-> "):
			fields := strings.Fields(strings.TrimPrefix(line, "-> "))
			if len(fields) > 0 && !strings.HasSuffix(fields[0], "-grease") {
				n++
			}
		}
	}
}

func readCertificate(path string) (*x509.Certificate, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	block, _ := pem.Decode(raw)
	if block == nil || block.Type != "CERTIFICATE" {
		return nil, errors.New("no CERTIFICATE PEM block")
	}
	return x509.ParseCertificate(block.Bytes)
}

func readPublicKeySPKI(path string) ([]byte, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	block, _ := pem.Decode(raw)
	if block == nil || block.Type != "PUBLIC KEY" {
		return nil, errors.New("no PUBLIC KEY PEM block")
	}
	if _, err := x509.ParsePKIXPublicKey(block.Bytes); err != nil {
		return nil, fmt.Errorf("parse public key: %w", err)
	}
	return block.Bytes, nil
}
