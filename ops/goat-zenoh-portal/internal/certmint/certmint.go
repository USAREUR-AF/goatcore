// Package certmint mints a per-vendor X.509 leaf cert via cert-sign and
// assembles the vendor onboarding tarball.
//
// The mint operation shells out to the cert-sign binary (see
// ops/enrollment/cmd/cert-sign/) with a per-goatnet --uri-san prefix. The
// --uri-san flag is the Track A substrate extension; until it lands the
// portal runs in fake-mode and produces a stub tarball for dev/test.
//
// The tarball contains:
//
//	<vendor>-cert.pem        — per-vendor X.509 leaf cert (1y validity)
//	<vendor>-key.pem         — leaf private key (mode 0400)
//	<root-cert-name>         — offline-CA root cert for trust-anchor install (per-goatnet filename)
//	<packet-name>            — community onboarding packet (per-goatnet filename)
//	NOTES.md                 — vendor-specific notes (prefix + portal URL)
//	first-publisher.py       — runnable publisher pre-wired with mTLS
//	first-subscriber.py      — companion subscriber for self-validation
//
// The tarball is dropped into VendorBundleDir and a one-time download URL
// is returned to the operator at vendor registration. Bundle path +
// token live in the registry; download is consumed once.
package certmint

import (
	"archive/tar"
	"compress/gzip"
	"context"
	"crypto/rand"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/config"
)

// Minter encapsulates a single cert-sign invocation shape.
type Minter struct {
	BinaryPath       string
	BundleCreatePath string // path to bundle-create; mints the goat-cli .cbor into the tarball. Empty (or PortalURL/RouterEndpoint unset) = omit the .cbor.
	Fake             bool
	CABundleDir      string
	AgeIdentityPath  string
	CAID             string
	SiteID           string
	Org              string // Subject O of every minted leaf, passed as cert-sign --org. Empty omits the flag (cert-sign's built-in default applies).
	URIPrefix        string // URI-SAN prefix; per-goatnet (e.g. "urn:goat:<site>:org="). Frozen into every minted cert — the integration-pinned identity grammar.
	CNSuffix         string // per-vendor-mode leaf CN suffix (legacy "-trial-peer"). Empty → "-trial-peer". Slot-pool mode overrides CN to the bare anchor via MintOptions, so this is unused there.
	DNSSuffix        string // DNS-SAN suffix; the SAN is "client.<prefix>.<DNSSuffix>" (per-goatnet). In the cert; not load-bearing for the ACL (keys on URI-SAN + CN) but per-goatnet for audit cleanliness. Empty → "example.peer".
	CARoot           string
	// BundleCAPubkey is the path to the PUBLIC key of the CA that signs the
	// .cbor — ops/enrollment/public-keys/<ca_id>.pem. Shipped in the tarball
	// beside the .cbor it verifies.
	//
	// goat-cli compiles in no CA root: a release used to carry one
	// goatnet's key, so a bundle from any other goatnet failed to install with
	// an error that reads like a corrupt file. Roots are operator-supplied now,
	// which is what makes one binary work everywhere — and it means the recipient
	// of a .cbor must be able to GET the matching root. This is that path: the
	// same authenticated download as the bundle, so it adds no new trust surface,
	// and NOTES.md tells the reader to confirm the fingerprint out of band anyway.
	//
	// Empty = omit (dev, and any goatnet that distributes roots another way).
	BundleCAPubkey   string
	OnboardingPacket string // path to the onboarding markdown template bundled into the tarball
	PacketName       string // in-tarball filename for the onboarding packet (legacy "zenoh-trial-onboarding.md"). Empty → legacy.
	TarballStem      string // filename stem for the minted tarball (per-goatnet). Empty → legacy.
	RootCertName     string // in-tarball filename for the CA root cert (per-goatnet). Empty → legacy.
	VendorBundleDir  string
	Validity         time.Duration

	// RecipientKeysDir, when set, receives the PUBLIC leaf cert of every mint as
	// <dir>/<prefix>.pem — the by-anchor recipient-cert store that
	// GET /api/access/recipient-cert serves and grant-keyd syncs into its
	// cert_dir (the U7 "publish my key" distribution step; grant-keyed-encryption
	// §"Recipient-key PKI"). The portal is the ISSUER of these leaves, so
	// republishing the public half it just minted adds no new authority — the
	// key service still validates chain + CN/URI-SAN binding at resolve time.
	// Overwritten on re-mint / rotation (ADR 1044 in-place re-issue) so the
	// store always carries the current leaf. The private key is never written
	// here. Empty = skip (pre-U7 behavior).
	RecipientKeysDir string

	// Lex is the display lexicon (vendor→org). Drives the org-noun + product
	// wording in the generated NOTES.md + first-publisher/subscriber.py packet
	// so the onboarding tarball matches the portal UI. Zero value → the legacy
	// defaults (vendor / Zenoh Trial). See docs/design/portal-lexicon.md.
	Lex config.Lexicon

	// Vendor-facing tarball-render parameters. Used by assembleTarball to
	// substitute placeholders in the in-tarball onboarding doc and to bake
	// the right endpoint into the worked first-publisher.py example.
	// Filled in at portal-deploy time by the Ansible role (defaults
	// match the legacy trial fabric).
	RouterEndpoint  string // e.g., "tls/zenoh.goat.example:7447" — what vendors connect their Zenoh client to
	PortalURL       string // e.g., "https://portal.goat.example" — vendor-facing portal base URL
	OnboardURL      string // OPTIONAL, e.g. "https://onboard.site.netbird.example.net" — the goat-id-console onboarding-engine URL baked into the bundle as onboard_endpoint (docs/design/onboarding-extraction-plan.md). Empty on goatnets without the onboard route (CLI portal-falls-back).
	TrialSunset     string // e.g., "2026-12-31" or "open-ended (trial-fabric)"
	FeedbackChannel string // e.g., "Matrix #goat-trial-feedback" — where vendors report friction
}

// MintOptions carries per-call overrides. The zero value preserves the
// per-vendor behavior (CN = "<prefix>-trial-peer").
type MintOptions struct {
	// CommonName overrides the leaf cert CN. Empty = the default
	// "<prefix>-trial-peer". In per-uuid-slot-pool mode (ADR 1025) the portal
	// sets this to the bare slot anchor so the leaf matches the router's
	// pre-baked cert_common_names subject and writes are admitted with no ACL
	// edit and no restart.
	CommonName string

	// ServerAuth adds id-kp-serverAuth to the leaf's EKU set alongside
	// clientAuth. Machine mints require it: pod first-boot wires the machine
	// bundle's leaf in as the pod router's LISTEN certificate as well as its
	// connect cert (the pod's host/first-boot.sh), and rustls refuses a
	// listener whose EKU lacks serverAuth — a clientAuth-only leaf fails every
	// local mTLS handshake. Person/org leaves stay clientAuth-only.
	ServerAuth bool
}

// Result is what callers get back from Mint.
type Result struct {
	CertPath    string
	KeyPath     string
	TarballPath string
	BundleToken string // hex-encoded one-time download token
	URISAN      string // the exact URI-SAN minted into the cert
}

// Mint produces the leaf cert + key for vendor <prefix> and assembles the
// onboarding tarball. Returns the paths + the one-time bundle token. The
// caller stores those in the registry; the consumer is the org/vendor
// bundle download endpoint (/api/org-bundle, legacy /api/vendor-bundle).
//
// Idempotency: each call mints a fresh cert + key + tarball under a
// timestamp suffix. Re-running for the same vendor is fine; the old
// tarball stays on disk for forensic reference and is rotated by the
// caller (or trial sunset).
func (m *Minter) Mint(ctx context.Context, prefix, apiKey string, opts MintOptions) (*Result, error) {
	if prefix == "" {
		return nil, errors.New("org prefix required")
	}
	// Per-goatnet values come from the role (PORTAL_* env / cert-sign flags)
	// on every goatnet; these fallbacks are example values.
	if m.CNSuffix == "" {
		m.CNSuffix = "-trial-peer"
	}
	if m.URIPrefix == "" {
		m.URIPrefix = "urn:goat:example:org="
	}
	if m.DNSSuffix == "" {
		m.DNSSuffix = "example.peer"
	}
	if m.PacketName == "" {
		m.PacketName = "zenoh-trial-onboarding.md"
	}
	if m.TarballStem == "" {
		m.TarballStem = "goat-onboarding"
	}
	if m.RootCertName == "" {
		m.RootCertName = defaultRootCertName
	}
	commonName := prefix + m.CNSuffix
	if opts.CommonName != "" {
		commonName = opts.CommonName
	}
	if m.Validity == 0 {
		m.Validity = 365 * 24 * time.Hour
	}
	if m.VendorBundleDir == "" {
		return nil, errors.New("vendor bundle dir required")
	}
	if err := os.MkdirAll(m.VendorBundleDir, 0o750); err != nil {
		return nil, fmt.Errorf("create vendor bundle dir: %w", err)
	}

	stamp := time.Now().UTC().Format("20060102T150405Z")
	stage := filepath.Join(m.VendorBundleDir, fmt.Sprintf(".stage-%s-%s", prefix, stamp))
	if err := os.MkdirAll(stage, 0o750); err != nil {
		return nil, fmt.Errorf("create stage dir: %w", err)
	}
	defer os.RemoveAll(stage)

	certPath := filepath.Join(stage, fmt.Sprintf("%s-cert.pem", prefix))
	keyPath := filepath.Join(stage, fmt.Sprintf("%s-key.pem", prefix))
	uriSAN := m.URIPrefix + prefix

	if m.Fake {
		if err := writeFakeCertKey(certPath, keyPath, prefix, uriSAN); err != nil {
			return nil, err
		}
	} else {
		eku := "clientAuth"
		if opts.ServerAuth {
			eku = "clientAuth,serverAuth"
		}
		if err := m.invokeCertSign(ctx, prefix, certPath, keyPath, commonName, uriSAN, eku); err != nil {
			return nil, err
		}
	}

	// Publish the PUBLIC leaf to the recipient-cert store (U7 distribution).
	// Fail the mint on error: a mint whose recipient cert silently failed to
	// persist would leave the member undecryptable-by-derivation (the exact
	// F-267 gap this closes) with nothing surfacing it.
	if err := m.persistRecipientCert(prefix, certPath); err != nil {
		return nil, err
	}

	// Mint the goat-cli profile bundle (.cbor) into the same stage dir so
	// it rides the vendor tarball — the vendor downloads ONE tarball and
	// runs `goat profile init <prefix>-goat-cli.cbor` from it, no separate
	// hand-delivered file. The .cbor is signed by the SAME CA the
	// portal already uses for cert-sign (ca.key.age under CABundleDir,
	// decrypted by AgeIdentityPath). Skipped (cborPath stays "") when the
	// portal isn't configured for it — assembleTarball omits empty files,
	// so the tarball degrades gracefully to cert+key only.
	cborPath := filepath.Join(stage, fmt.Sprintf("%s-goat-cli.cbor", prefix))
	switch {
	case m.Fake:
		if err := writeFakeCbor(cborPath, prefix); err != nil {
			return nil, err
		}
	case m.BundleCreatePath != "" && m.PortalURL != "" && m.RouterEndpoint != "":
		if err := m.invokeBundleCreate(ctx, prefix, certPath, keyPath, apiKey, cborPath); err != nil {
			return nil, fmt.Errorf("mint goat-cli bundle: %w", err)
		}
	default:
		// SAY SO. A bundle with no goat-cli profile contradicts the welcome email,
		// which tells every recipient to run `goat profile init <the .cbor>`, so the
		// mint must not omit the profile and report success as if nothing happened.
		//
		// Named individually rather than as "not configured": the gate is a
		// three-way AND, and a single "not configured" line cannot distinguish
		// missing two legs from missing one. Whoever reads this log should not have
		// to re-derive which knob is unset.
		var missing []string
		if m.BundleCreatePath == "" {
			missing = append(missing, "--bundle-create-bin")
		}
		if m.PortalURL == "" {
			missing = append(missing, "--portal-public-url")
		}
		if m.RouterEndpoint == "" {
			missing = append(missing, "--router-endpoint")
		}
		slog.Warn("minting a fabric bundle WITHOUT the goat-cli profile (.cbor)",
			"prefix", prefix,
			"missing", strings.Join(missing, " "),
			"consequence", "`goat profile init` has nothing to install; the welcome email names a file this tarball will not contain")
		cborPath = "" // not configured for .cbor minting; tarball omits it
	}

	// Assemble the tarball.
	tarballPath := filepath.Join(m.VendorBundleDir, fmt.Sprintf("%s-%s-%s.tar.gz", m.TarballStem, prefix, stamp))
	params := tarballParams{
		Prefix:          prefix,
		CertPath:        certPath,
		KeyPath:         keyPath,
		CborPath:        cborPath,
		BundleCAPubkey:  m.BundleCAPubkey,
		RootPath:        m.CARoot,
		RootCertName:    m.RootCertName,
		PacketName:      m.PacketName,
		OnboardingPath:  m.OnboardingPacket,
		URISAN:          uriSAN,
		RouterEndpoint:  m.RouterEndpoint,
		PortalURL:       m.PortalURL,
		TrialSunset:     m.TrialSunset,
		FeedbackChannel: m.FeedbackChannel,
		OrgNoun:         defaultStr(m.Lex.Org, "vendor"),
		OrgTitle:        defaultStr(m.Lex.OrgTitle, "Vendor"),
		OrgUpper:        defaultStr(m.Lex.OrgUpper, "VENDOR"),
		ProductName:     defaultStr(m.Lex.ProductName, "Zenoh Trial"),
	}
	if err := assembleTarball(tarballPath, params); err != nil {
		return nil, fmt.Errorf("assemble tarball: %w", err)
	}

	tokenBytes := make([]byte, 16)
	if _, err := rand.Read(tokenBytes); err != nil {
		return nil, fmt.Errorf("generate bundle token: %w", err)
	}
	return &Result{
		CertPath:    certPath,
		KeyPath:     keyPath,
		TarballPath: tarballPath,
		BundleToken: hex.EncodeToString(tokenBytes),
		URISAN:      uriSAN,
	}, nil
}

// persistRecipientCert copies the minted PUBLIC leaf cert into
// RecipientKeysDir/<prefix>.pem (atomic: temp + rename, so a reader never sees
// a torn PEM). No-op when RecipientKeysDir is unset. The prefix doubles as the
// filename, so refuse anything path-shaped — mint prefixes are flat anchors.
func (m *Minter) persistRecipientCert(prefix, certPath string) error {
	if m.RecipientKeysDir == "" {
		return nil
	}
	if strings.ContainsAny(prefix, "/\\") || prefix == "." || prefix == ".." {
		return fmt.Errorf("recipient cert: unsafe anchor %q", prefix)
	}
	if err := os.MkdirAll(m.RecipientKeysDir, 0o755); err != nil {
		return fmt.Errorf("recipient-keys dir: %w", err)
	}
	pem, err := os.ReadFile(certPath)
	if err != nil {
		return fmt.Errorf("recipient cert: read minted leaf: %w", err)
	}
	tmp := filepath.Join(m.RecipientKeysDir, fmt.Sprintf(".tmp-%s-%d.pem", prefix, os.Getpid()))
	if err := os.WriteFile(tmp, pem, 0o644); err != nil {
		return fmt.Errorf("recipient cert: stage: %w", err)
	}
	final := filepath.Join(m.RecipientKeysDir, prefix+".pem")
	if err := os.Rename(tmp, final); err != nil {
		_ = os.Remove(tmp)
		return fmt.Errorf("recipient cert: publish: %w", err)
	}
	return nil
}

// invokeCertSign builds the cert-sign argv and runs the binary. Errors
// pass through stderr-truncated for actionability.
func (m *Minter) invokeCertSign(ctx context.Context, prefix, certPath, keyPath, commonName, uriSAN, eku string) error {
	bin := m.BinaryPath
	if bin == "" {
		var err error
		bin, err = exec.LookPath("cert-sign")
		if err != nil {
			return fmt.Errorf("cert-sign binary not found in PATH (set --cert-sign-bin or --cert-sign-fake): %w", err)
		}
	}
	if m.CABundleDir == "" || m.AgeIdentityPath == "" {
		return errors.New("real cert-sign requires --ca-bundle-dir and --age-identity")
	}
	rootCert := filepath.Join(m.CABundleDir, "root-cert.pem")
	rootKey := filepath.Join(m.CABundleDir, "ca.key.age")
	// cert-sign requires at least one --san (peers verify SAN, not CN).
	// The convention is `<system>.<prefix>.<DNSSuffix>`; the portal doesn't
	// have a per-mint "system" string, so default to `client.<prefix>.<DNSSuffix>`
	// — distinct enough for audit, not load-bearing for the ACL (which keys
	// on URI-SAN + CN). DNSSuffix is per-goatnet; defaulted in Mint before this
	// runs.
	dnsSAN := fmt.Sprintf("client.%s.%s", prefix, m.DNSSuffix)
	args := []string{
		"--root-cert", rootCert,
		"--root-key", rootKey,
		"--age-identity", m.AgeIdentityPath,
		"--ca-id", m.CAID,
		"--site-id", m.SiteID,
		"--cn", commonName,
		"--san", dnsSAN,
		"--uri-san", uriSAN,
		"--eku", eku,
		// Recipient contract: this leaf's public key is the grant-crypto
		// DEK wrap target (ECDH over P-256, envelope.RecipientFromCert). Ed25519
		// can sign but cannot ECDH, and cert-sign's "auto" maps clientAuth to
		// Ed25519 — so the key type must be pinned or every portal-minted member
		// is undecryptable. Gated by the portal-mint conformance job in
		// .github/workflows/grant-crypto.yml.
		"--key-type", "ecdsa-p256",
		"--validity", m.Validity.String(),
		"--out-cert", certPath,
		"--out-key", keyPath,
	}
	// The goatnet's Subject O. Without it a cert-sign built from this tree
	// stamps its built-in default, which is not the organisation a goatnet's
	// earlier leaves carry. Empty leaves cert-sign's default (and keeps a
	// cert-sign that predates --org working).
	if org := strings.TrimSpace(m.Org); org != "" {
		args = append(args, "--org", org)
	}
	cmd := exec.CommandContext(ctx, bin, args...)
	cmd.Stdout = nil
	stderrBuf := &strings.Builder{}
	cmd.Stderr = io.MultiWriter(os.Stderr, stderrBuf)
	if err := cmd.Run(); err != nil {
		errStr := stderrBuf.String()
		if strings.Contains(errStr, "flag provided but not defined: -org") {
			return fmt.Errorf("cert-sign rejected --org: this cert-sign build predates the flag; install a cert-sign built from ops/enrollment/cmd/cert-sign (the portal role rebuilds it) or clear the portal's --cert-org — underlying error: %w", err)
		}
		if strings.Contains(errStr, "--uri-san") {
			return fmt.Errorf("cert-sign rejected --uri-san: this cert-sign build lacks the URI-SAN extension; install one that has it before flipping --cert-sign-fake off — underlying error: %w", err)
		}
		return fmt.Errorf("cert-sign: %w; stderr: %s", err, strings.TrimSpace(errStr))
	}
	return nil
}

// invokeBundleCreate shells out to bundle-create to mint the signed
// goat-cli profile bundle (.cbor) into outCbor. Mirrors invokeCertSign:
// same CA material (ca.key.age under CABundleDir, decrypted by
// AgeIdentityPath), same argv shape the operator orchestrator uses
// (Phase 4.5.5 of ops/ansible/operator-onboard.yml). The portal api_key
// (the .cbor's portal token) is written to a transient file because
// bundle-create takes --goat-cli-portal-token as a path.
func (m *Minter) invokeBundleCreate(ctx context.Context, prefix, certPath, keyPath, apiKey, outCbor string) error {
	bin := m.BundleCreatePath
	if bin == "" {
		var err error
		bin, err = exec.LookPath("bundle-create")
		if err != nil {
			return fmt.Errorf("bundle-create binary not found in PATH (set --bundle-create-bin): %w", err)
		}
	}
	if m.CABundleDir == "" || m.AgeIdentityPath == "" {
		return errors.New("bundle-create requires --ca-bundle-dir and --age-identity")
	}
	// The portal api_key is passed as a file path. Write it next to the
	// .cbor in the stage dir (RemoveAll'd by the caller's defer), 0600.
	tokenPath := filepath.Join(filepath.Dir(outCbor), "portal-api-key")
	if err := os.WriteFile(tokenPath, []byte(apiKey), 0o600); err != nil {
		return fmt.Errorf("write portal token file: %w", err)
	}
	defer os.Remove(tokenPath)

	caKey := filepath.Join(m.CABundleDir, "ca.key.age")
	args := []string{
		"--site", m.SiteID,
		"--device-id", "operator-" + prefix,
		"--acl-groups", "OPERATORS",
		"--ca-id", m.CAID,
		"--ca-key", caKey,
		"--age-identity", m.AgeIdentityPath,
		"--goat-cli-portal-endpoint", m.PortalURL,
		"--goat-cli-portal-token", tokenPath,
		"--goat-cli-router-endpoint", m.RouterEndpoint,
		"--goat-cli-mtls-cert", certPath,
		"--goat-cli-mtls-key", keyPath,
		"--goat-cli-ca-roots", m.CARoot,
		"--out", outCbor,
	}
	// OPTIONAL onboarding-engine endpoint — baked into the bundle as
	// onboard_endpoint so `goat profile init` writes it and the CLI targets the
	// console for /api/onboard/*. Omitted (portal-fallback) when unset.
	if m.OnboardURL != "" {
		args = append(args, "--goat-cli-onboard-endpoint", m.OnboardURL)
	}
	cmd := exec.CommandContext(ctx, bin, args...)
	cmd.Stdout = nil
	stderrBuf := &strings.Builder{}
	cmd.Stderr = io.MultiWriter(os.Stderr, stderrBuf)
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("bundle-create: %w; stderr: %s", err, strings.TrimSpace(stderrBuf.String()))
	}
	return nil
}

// writeFakeCbor writes a placeholder .cbor for --cert-sign-fake mode so
// the tarball-assembly flow can be exercised without a real signing key.
// Deliberately not valid CBOR — anything trying to `goat profile init`
// it fails fast, which is the point of fake mode.
func writeFakeCbor(path, prefix string) error {
	body := fmt.Sprintf("FAKE-GOAT-CLI-CBOR placeholder for %s (cert-sign-fake mode; not a real signed bundle)\n", prefix)
	return os.WriteFile(path, []byte(body), 0o400)
}

// writeFakeCertKey writes placeholder cert + key files for --cert-sign-fake.
//
// The placeholders are deliberately NOT PEM-shaped: anything trying to use
// them as real TLS material will fail at first byte (which is the point —
// fake mode is for exercising the tarball-assembly + bundle-download flow,
// not for TLS). Dropping the PEM markers also keeps the repo-wide
// secret-scanner happy — a literal `BEGIN ... KEY` header in tracked source
// would otherwise trip the credential-leak guard even though no key
// material is involved.
func writeFakeCertKey(certPath, keyPath, prefix, uriSAN string) error {
	cert := fmt.Sprintf(`FAKE CERTIFICATE PLACEHOLDER (not PEM; not a real X.509 leaf).
Generated by --cert-sign-fake.
Org prefix: %s
URI-SAN the real mint would carry: %s
Replace with output of cert-sign.
`, prefix, uriSAN)
	key := fmt.Sprintf(`FAKE KEY PLACEHOLDER (not PEM; no key material).
Generated by --cert-sign-fake for org %s.
Replace with output of cert-sign.
`, prefix)
	if err := os.WriteFile(certPath, []byte(cert), 0o644); err != nil {
		return fmt.Errorf("write fake cert: %w", err)
	}
	if err := os.WriteFile(keyPath, []byte(key), 0o400); err != nil {
		return fmt.Errorf("write fake key: %w", err)
	}
	return nil
}

// tarballParams carries everything assembleTarball needs to render the
// vendor-facing tarball — cert/key/root paths from the mint, plus the
// trial-fabric parameters (router endpoint, portal URL, sunset,
// feedback channel) that get substituted into the onboarding doc and
// baked into the worked first-publisher.py example.
type tarballParams struct {
	Prefix          string
	CertPath        string
	KeyPath         string
	CborPath        string // goat-cli profile bundle; "" omits it
	BundleCAPubkey  string // CA public key that verifies CborPath; "" omits it
	RootPath        string
	RootCertName    string // in-tarball filename for the CA root cert (per-goatnet); "" → defaultRootCertName
	PacketName      string // in-tarball filename for the onboarding packet (per-goatnet); "" → "zenoh-trial-onboarding.md"
	OnboardingPath  string
	URISAN          string
	RouterEndpoint  string
	PortalURL       string
	TrialSunset     string
	FeedbackChannel string

	// Display lexicon (vendor→org) for the packet wording; filled from
	// Minter.Lex with legacy fallbacks so an unset lexicon renders the old text.
	OrgNoun     string // singular lowercase: vendor / org
	OrgTitle    string // singular title-case: Vendor / Org
	OrgUpper    string // all-caps (the python example's prefix variable): VENDOR / ORG
	ProductName string // e.g. Zenoh Trial
}

// assembleTarball packs cert + key + root + onboarding packet + a worked
// first-publisher.py into a gz tarball at tarballPath.
func assembleTarball(tarballPath string, p tarballParams) error {
	f, err := os.Create(tarballPath)
	if err != nil {
		return err
	}
	defer f.Close()
	gz := gzip.NewWriter(f)
	defer gz.Close()
	tw := tar.NewWriter(gz)
	defer tw.Close()

	addFile := func(name, src string) error {
		if src == "" {
			return nil // skip; configured-empty
		}
		data, err := os.ReadFile(src)
		if err != nil {
			return fmt.Errorf("read %s: %w", src, err)
		}
		hdr := &tar.Header{
			Name:    name,
			Mode:    0o644,
			Size:    int64(len(data)),
			ModTime: time.Now(),
		}
		// -key.pem and the .cbor both carry the vendor's private key —
		// lock them to 0400.
		if strings.HasSuffix(name, "-key.pem") || strings.HasSuffix(name, ".cbor") {
			hdr.Mode = 0o400
		}
		if err := tw.WriteHeader(hdr); err != nil {
			return err
		}
		if _, err := tw.Write(data); err != nil {
			return err
		}
		return nil
	}

	// addBytes writes a synthesized (not on-disk) file into the tarball.
	addBytes := func(name string, body []byte, mode int64) error {
		hdr := &tar.Header{Name: name, Mode: mode, Size: int64(len(body)), ModTime: time.Now()}
		if err := tw.WriteHeader(hdr); err != nil {
			return err
		}
		_, err := tw.Write(body)
		return err
	}

	if err := addFile(fmt.Sprintf("%s-cert.pem", p.Prefix), p.CertPath); err != nil {
		return err
	}
	if err := addFile(fmt.Sprintf("%s-key.pem", p.Prefix), p.KeyPath); err != nil {
		return err
	}
	if err := addFile(defaultStr(p.RootCertName, defaultRootCertName), p.RootPath); err != nil {
		return err
	}

	// goat-cli profile bundle (signed cert+key+token+endpoints+ca-roots).
	// Empty CborPath is skipped by addFile. Vendor runs:
	//   goat profile init <prefix>-goat-cli.cbor
	if err := addFile(fmt.Sprintf("%s-goat-cli.cbor", p.Prefix), p.CborPath); err != nil {
		return err
	}

	// The CA public key that verifies the .cbor. Named as its sibling so the
	// pairing is visible in a directory listing — the two files are useless
	// apart. Only shipped when there is a .cbor to verify.
	if p.CborPath != "" {
		if err := addFile(fmt.Sprintf("%s-goat-cli-ca.pem", p.Prefix), p.BundleCAPubkey); err != nil {
			return err
		}
	}

	// Onboarding doc — render placeholders against the per-vendor params
	// before adding (F-DOG-5: prior behavior shipped the template verbatim).
	if p.OnboardingPath != "" {
		onboardingBytes, err := os.ReadFile(p.OnboardingPath)
		if err != nil {
			return fmt.Errorf("read %s: %w", p.OnboardingPath, err)
		}
		rendered := renderOnboardingDoc(string(onboardingBytes), p)
		if err := addBytes(defaultStr(p.PacketName, "zenoh-trial-onboarding.md"), []byte(rendered), 0o644); err != nil {
			return err
		}
	}

	// Worked Python publisher — vendor copy-pastes + runs, they're publishing.
	// Closes F-DOG-10/11/14 (cert SAN workaround, whole-block insert,
	// autogen example that lacks mTLS): this example has the full mTLS
	// config baked in and the right endpoint / cert paths pre-wired.
	publisherPy := renderFirstPublisher(p)
	if err := addBytes("first-publisher.py", []byte(publisherPy), 0o755); err != nil {
		return err
	}

	// Companion subscriber so the vendor can self-validate in two shells
	// (publisher in one, subscriber in another) without needing a
	// second cert or any code on our side.
	subscriberPy := renderFirstSubscriber(p)
	if err := addBytes("first-subscriber.py", []byte(subscriberPy), 0o755); err != nil {
		return err
	}

	notes := fmt.Sprintf(`# Vendor notes — %s

Your vendor prefix: %s
Your URI-SAN identity: %s
Router endpoint:     %s
Portal:              %s

This tarball was minted by goat-zenoh-portal. The cert + key authenticate
your peer to the Zenoh router at the wire layer. Your portal API key is a
SEPARATE credential — it authenticates your portal session for registering
topics / editing metadata — and it was delivered to you in your welcome
email. Paste that whole string into the Portal's "Sign in" box. Do NOT ask
for a rotation to get a "fresh" one: rotating re-issues the certificate
above and makes the key you were sent stop working.

Fastest path to "publishing in under a minute":

  python3 -m venv venv && . venv/bin/activate
  pip install eclipse-zenoh protobuf
  python3 first-publisher.py   # Windows: use "python" not "python3"

The first-publisher.py example has the right mTLS connection block
filled in (it points at the router endpoint above, presents your cert
+ key, trusts the fabric CA root). Copy it + modify for your real
payloads.

Self-validate in two shells:

  python3 first-subscriber.py     # shell A — subscribes to %s/**
  python3 first-publisher.py      # shell B — publishes %s/hello/v1

If you see shell A print "%s/hello/v1  text(...): 'hello from %s ...'"
within a second of running shell B, you're publishing AND subscribing
correctly. Same script, narrow subscription: pass a key expression
("%s/hello/v1") to first-subscriber.py as the first arg.

See %s in this tarball for the rest of the
onboarding workflow, including how to register topics + upload
protobuf schemas so the portal's schema viewer + inspector decode
light up for your topics.
`, p.Prefix, p.Prefix, p.URISAN, defaultStr(p.RouterEndpoint, "<router-endpoint, ask operator>"), defaultStr(p.PortalURL, "<portal-url, ask operator>"), p.Prefix, p.Prefix, p.Prefix, p.Prefix, p.Prefix, defaultStr(p.PacketName, "zenoh-trial-onboarding.md"))
	if p.CborPath != "" {
		notes += fmt.Sprintf("\n## goat-cli profile (optional power tool)\n\n"+
			"This tarball includes %s-goat-cli.cbor — a CA-signed profile bundle\n"+
			"(your cert + key + portal token + endpoints in one signed blob), and\n"+
			"%s-goat-cli-ca.pem, the CA public key that verifies it.\n\n"+
			"goat has no goatnet built into it, so you tell it which CA to trust\n"+
			"first. Install the goat binary from the mesh download surface, then:\n\n"+
			"  goat ca add %s-goat-cli-ca.pem\n"+
			"  goat profile init %s-goat-cli.cbor\n"+
			"  goat doctor\n\n"+
			"`goat ca add` prints a fingerprint. Check it against the one the\n"+
			"operator gives you before you rely on it — this tarball reached you\n"+
			"over a channel, and a channel can be wrong.\n\n"+
			"The same binary works against every goatnet whose root you have added;\n"+
			"`goat ca ls` shows them.\n\n"+
			"macOS / Linux only; no Windows build (use the web surfaces instead).\n\n"+
			"This file also holds a SECOND copy of the portal API key from your\n"+
			"welcome email. The .cbor is BINARY — do not copy the key out of it by\n"+
			"hand (stray bytes ride along and the portal rejects it). If the email\n"+
			"is gone, run `goat profile init <this .cbor>` and read\n"+
			"contexts/default/portal.token — same key, extracted correctly. Do not\n"+
			"ask for a new one (a rotation would invalidate this copy).\n",
			p.Prefix, p.Prefix, p.Prefix, p.Prefix)
	}
	if err := addBytes("NOTES.md", []byte(applyLexicon(notes, p)), 0o644); err != nil {
		return err
	}
	return nil
}

// defaultStr returns def when s is empty. Used so the vendor-facing
// tarball still renders cleanly when the operator hasn't configured
// every Minter field.
func defaultStr(s, def string) string {
	if s == "" {
		return def
	}
	return s
}

// applyLexicon rewrites the display-vocabulary words in the generated onboarding
// packet (NOTES.md + the worked first-publisher/subscriber.py) to match the
// goatnet's lexicon — "vendor"→"org" + product name where configured,
// byte-unchanged under the legacy lexicon (where every replacement value equals
// the matched text). Machine strings
// — the cert/key/CA-root filenames, the `urn:` URI-SAN, the `.cbor` profile name
// — contain no matched phrase and pass through untouched. The publisher's
// prefix variable (`VENDOR`/`ORG`) is renamed consistently across that script,
// so the generated example stays valid; the subscriber names its slots
// `CERT_SLOT_ID`/`TOPIC_SLOT_ID`, which no lexicon rewrites. See
// docs/design/portal-lexicon.md.
// defaultRootCertName is the in-tarball CA-root filename when the goatnet sets
// none (the role sets it on every goatnet).
const defaultRootCertName = "ca-root.pem"

// caRootCertNameToken is the placeholder the generated sample scripts and
// templates use for the per-goatnet CA-root filename.
const caRootCertNameToken = "<ca-root-cert-name>"

// The onboarding-doc template (docs/operations/zenoh-trial-onboarding.md,
// placed on the host by the role) names the CA root and the tarball row by
// these literal strings; renderOnboardingDoc rewrites them to the goatnet's
// values. They must match the template's text exactly.
const (
	docTemplateCARootName = "nova-ca-root.pem"
	docTemplateTarballRow = "`<filled-in-at-deploy>` — distributed OOB at vendor registration. Contains your per-vendor X.509 cert + key + the nova CA root cert."
)

func applyLexicon(s string, p tarballParams) string {
	return strings.NewReplacer(
		"VENDOR", p.OrgUpper,
		"# Vendor notes", "# "+p.OrgTitle+" notes",
		"Your vendor prefix:", "Your "+p.OrgNoun+" prefix:",
		"your vendor prefix", "your "+p.OrgNoun+" prefix",
		"your whole vendor subtree", "your whole "+p.OrgNoun+" subtree",
		`vendor "`, p.OrgNoun+` "`,
		"<fabric-product-name>", p.ProductName,
		// The sample scripts + NOTES.md reference the in-tarball CA root by name;
		// point it at the per-goatnet filename the tarball actually ships
		// (addFile uses the same RootCertName), or first-publisher.py /
		// first-subscriber.py point at a file not in the tarball and fail on
		// connect.
		caRootCertNameToken, defaultStr(p.RootCertName, defaultRootCertName),
	).Replace(s)
}

// renderOnboardingDoc replaces the documented placeholders in the
// in-tarball trial-onboarding markdown with the per-vendor + per-fabric
// values. Closes F-DOG-5 (prior behavior shipped the template verbatim
// so every vendor saw `<router-LAN-IP>` literals where their actual
// endpoint should have been).
//
// Placeholders that get substituted (only when the param is non-empty):
//
//	<router-endpoint>       → e.g. tls/zenoh.site.netbird.example.net:7447
//	<router-host>           → bare host, no scheme/port (REST row)
//	<router-LAN-IP>         → host:port from RouterEndpoint (legacy form)
//	<portal-url>            → full portal base URL (scheme + host)
//	<uri-san-prefix>        → per-goatnet URI-SAN scheme, e.g. urn:goat:<site>:org=
//	<ca-root-cert-name>     → per-goatnet CA-root filename (RootCertName); the
//	                          template's legacy literal filename is rewritten too
//	<vendor>                → vendor prefix
//	<portal-host>           → host of PortalURL
//	<trial-zone>            → DNS zone of PortalURL host
//	<filled-in-at-deploy>   → endpoint when followed by tarball/portal cues; sunset otherwise
//	<feedback-channel>      → e.g. Matrix #goat-trial-feedback
func renderOnboardingDoc(src string, p tarballParams) string {
	replacements := []string{
		"<vendor>", p.Prefix,
		"<router-endpoint>", defaultStr(p.RouterEndpoint, "<router-endpoint, ask operator>"),
		"<router-LAN-IP>", routerHostPort(p.RouterEndpoint),
		"<feedback-channel>", defaultStr(p.FeedbackChannel, "<feedback-channel, ask operator>"),
		// `<filled-in-at-deploy>` cells in the connection-details table —
		// replace with the corresponding actual value where we can; leave
		// alone otherwise (operator can fix in a follow-up sed if needed).
		docTemplateTarballRow,
		"Vendor cert tarball — distributed OOB at vendor registration. Contains your per-vendor X.509 cert + key + the fabric CA root cert.",
	}
	if p.TrialSunset != "" {
		replacements = append(replacements, "`<filled-in-at-deploy>`", p.TrialSunset)
	}
	if p.PortalURL != "" {
		// Strip scheme for host substitution.
		host := p.PortalURL
		host = strings.TrimPrefix(host, "https://")
		host = strings.TrimPrefix(host, "http://")
		dot := strings.Index(host, ".")
		if dot > 0 {
			replacements = append(replacements, "<portal-host>", host[:dot])
			replacements = append(replacements, "<trial-zone>", host[dot+1:])
		}
	}
	replacements = append(replacements,
		// Per-goatnet CA-root filename. Mirrors applyLexicon + addFile.
		caRootCertNameToken, defaultStr(p.RootCertName, defaultRootCertName),
		docTemplateCARootName, defaultStr(p.RootCertName, defaultRootCertName),
	)
	// <router-host> is the bare host (no scheme, no port). The REST endpoint
	// (:8000) wants host-only, NOT host:port — feeding host:port there
	// produced the doubled-port bug (`http://host:7447:8000`,
	// `tls/host:7447:7447`). Substitute only when we have an endpoint so an
	// unconfigured fabric shows the placeholder rather than an empty string.
	// (The template does not name the router as an NTP server: no goatnet
	// serves time — chrony renders `port 0` fleet-wide.)
	if h := routerHost(p.RouterEndpoint); h != "" {
		replacements = append(replacements, "<router-host>", h)
	}
	// <portal-url> is the full base URL (scheme + host) for the portal API
	// examples; distinct from <portal-host>/<trial-zone> (host parts) above.
	if p.PortalURL != "" {
		replacements = append(replacements, "<portal-url>", p.PortalURL)
	}
	// <uri-san-prefix> is the per-goatnet URI-SAN scheme. Derived from the
	// vendor's full URISAN minus its prefix so no new field is needed; keeps
	// the wide-open-reads section on the goatnet's own scheme.
	if p.URISAN != "" && p.Prefix != "" {
		replacements = append(replacements, "<uri-san-prefix>", strings.TrimSuffix(p.URISAN, p.Prefix))
	}
	return strings.NewReplacer(replacements...).Replace(src)
}

// routerHostPort extracts the host:port from a tls/<host>:<port>
// endpoint string. Returns the empty string when the input doesn't
// match the expected shape.
func routerHostPort(endpoint string) string {
	if endpoint == "" {
		return ""
	}
	// Endpoint shapes the bundle uses: "tls/zenoh.goat.example:7447"
	// or "tcp/127.0.0.1:7447". Strip the leading scheme prefix.
	for _, prefix := range []string{"tls/", "tcp/", "quic/", "udp/"} {
		if strings.HasPrefix(endpoint, prefix) {
			return endpoint[len(prefix):]
		}
	}
	return endpoint
}

// routerHost extracts the bare host (no scheme, no port) from a
// tls/<host>:<port> endpoint. Used for the REST (:8000) row, which must
// NOT carry the router's :7447 port.
func routerHost(endpoint string) string {
	hp := routerHostPort(endpoint)
	if i := strings.LastIndex(hp, ":"); i >= 0 {
		return hp[:i]
	}
	return hp
}

// renderFirstPublisher returns the bytes of a runnable Python publisher
// for the trial fabric, pre-wired for vendor <prefix>'s cert + key. The
// mTLS connection block uses enable_mtls=true + verify_name_on_connect=true
// — the router endpoint is a DNS hostname matching a DNS SAN on the
// router leaf cert, so standard rustls hostname verification stays on.
func renderFirstPublisher(p tarballParams) string {
	endpoint := defaultStr(p.RouterEndpoint, "tls/zenoh.goat.example:7447")
	return applyLexicon(fmt.Sprintf(`#!/usr/bin/env python3
"""first-publisher.py — vendor "%s" → <fabric-product-name>.

A working starting point. Run this from the directory the tarball
unpacked into; the cert/key/root paths assume sibling files.

Pre-wired for the trial fabric's mTLS posture: enable_mtls=true,
verify_name_on_connect=true. The router endpoint is a DNS hostname
that resolves on the mesh CoreDNS resolver pushed by netbird-mgmt
and matches a DNS SAN on the router leaf cert, so standard TLS
hostname verification stays on (closes F-DOG-10).

Install:
    python3 -m venv venv && . venv/bin/activate
    pip install eclipse-zenoh protobuf
Run:
    python3 first-publisher.py            # 5 publishes
    python3 first-publisher.py 50 0.2     # 50 publishes at 200ms intervals

Windows note: use "python" instead of "python3" (the latter bypasses
the venv on Windows and hits the system Python, causing ModuleNotFoundError).
Activate with: venv\\Scripts\\activate  (not venv/bin/activate).
"""

import json
import os
import sys
import time

import zenoh

ROUTER = %q       # mesh-internal endpoint; from the OOB bundle
VENDOR = %q       # your vendor prefix
HERE = os.path.dirname(os.path.abspath(__file__))


def make_config() -> "zenoh.Config":
    conf = zenoh.Config()
    conf.insert_json5("mode", '"client"')
    conf.insert_json5("connect/endpoints", json.dumps([ROUTER]))
    # Insert the whole TLS block at once. Per-sub-key inserts
    # (transport/link/tls/connect_certificate, etc.) do NOT enable the
    # client-cert mTLS send path on at least eclipse-zenoh-python 1.x.
    conf.insert_json5("transport/link/tls", json.dumps({
        "root_ca_certificate": os.path.join(HERE, "<ca-root-cert-name>"),
        "connect_certificate": os.path.join(HERE, VENDOR + "-cert.pem"),
        "connect_private_key": os.path.join(HERE, VENDOR + "-key.pem"),
        "enable_mtls": True,
        "verify_name_on_connect": True,
    }))
    return conf


def main() -> None:
    iterations = int(sys.argv[1]) if len(sys.argv) > 1 else 5
    interval_s = float(sys.argv[2]) if len(sys.argv) > 2 else 1.0

    session = zenoh.open(make_config())
    key = VENDOR + "/hello/v1"
    pub = session.declare_publisher(key)
    try:
        for i in range(iterations):
            payload = "hello from {} at {:.3f}".format(VENDOR, time.time())
            pub.put(payload)
            print("PUT  i={:02d}  key={}  payload={}".format(i, key, payload), flush=True)
            time.sleep(interval_s)
    finally:
        pub.undeclare()
        session.close()


if __name__ == "__main__":
    main()
`, p.Prefix, endpoint, p.Prefix), p)
}

// renderFirstSubscriber returns the bytes of a runnable Python subscriber
// pre-wired for vendor <prefix>'s cert + key. Subscribes to "<prefix>/**"
// by default so a vendor can self-validate publishes in two shells (run
// this in one, first-publisher.py in another, see their own bytes come
// back). Same mTLS config shape as the publisher — the only difference
// is declare_subscriber instead of declare_publisher.
func renderFirstSubscriber(p tarballParams) string {
	endpoint := defaultStr(p.RouterEndpoint, "tls/zenoh.goat.example:7447")
	return applyLexicon(fmt.Sprintf(`#!/usr/bin/env python3
"""first-subscriber.py — vendor "%s" → <fabric-product-name>.

Self-validation companion to first-publisher.py. Run this in one shell
and first-publisher.py in another; you'll see your own publishes echo
back. By default subscribes to your whole vendor subtree ("%s/**");
pass a key expression as the first arg to narrow.

Install:
    python3 -m venv venv && . venv/bin/activate
    pip install eclipse-zenoh
Run:
    python3 first-subscriber.py                       # %s/**
    python3 first-subscriber.py "%s/hello/v1"         # one topic
    python3 first-subscriber.py "**"                  # everything on fabric

Windows note: use "python" instead of "python3"; activate the venv
with "venv\\Scripts\\activate" instead of ". venv/bin/activate".

Each sample prints as one line:
    HH:MM:SS  <key>  <text-or-hex-preview>

Press Ctrl-C to exit cleanly.
"""

import json
import os
import sys
import time

import zenoh

ROUTER = %q       # mesh-internal endpoint; from the OOB bundle
CERT_SLOT_ID = %q       # your slot ID; picks the <id>-cert.pem / <id>-key.pem files
# Point TOPIC_SLOT_ID at another slot to read its topics; the router's ACL
# for your cert still decides which of them reach you.
TOPIC_SLOT_ID = CERT_SLOT_ID       # slot whose topics to subscribe to; defaults to yours
HERE = os.path.dirname(os.path.abspath(__file__))


def make_config() -> "zenoh.Config":
    conf = zenoh.Config()
    conf.insert_json5("mode", '"client"')
    conf.insert_json5("connect/endpoints", json.dumps([ROUTER]))
    # Same whole-block insert as first-publisher.py — per-sub-key
    # inserts do NOT enable the client-cert send path on eclipse-zenoh-python 1.x.
    conf.insert_json5("transport/link/tls", json.dumps({
        "root_ca_certificate": os.path.join(HERE, "<ca-root-cert-name>"),
        "connect_certificate": os.path.join(HERE, CERT_SLOT_ID + "-cert.pem"),
        "connect_private_key": os.path.join(HERE, CERT_SLOT_ID + "-key.pem"),
        "enable_mtls": True,
        "verify_name_on_connect": True,
    }))
    return conf


def render_payload(raw: bytes) -> str:
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError:
        text = None
    if text is not None and all(c.isprintable() or c in "\r\n\t" for c in text):
        return "text({}B): {!r}".format(len(raw), text)
    return "bytes({}B) hex[:64]={}".format(len(raw), raw[:64].hex())


def on_sample(sample: "zenoh.Sample") -> None:
    ts = time.strftime("%%H:%%M:%%S")
    key = str(sample.key_expr)
    raw = bytes(sample.payload)
    print("{}  {}  {}".format(ts, key, render_payload(raw)), flush=True)


def main() -> None:
    key_expr = sys.argv[1] if len(sys.argv) > 1 else TOPIC_SLOT_ID + "/**"

    print("connecting to {} ...".format(ROUTER), flush=True)
    session = zenoh.open(make_config())
    sub = session.declare_subscriber(key_expr, on_sample)
    print("subscribed: {}".format(key_expr), flush=True)
    print("waiting for samples (Ctrl-C to exit)", flush=True)

    try:
        while True:
            time.sleep(3600)
    except KeyboardInterrupt:
        print("\nshutting down ...", flush=True)
    finally:
        sub.undeclare()
        session.close()


if __name__ == "__main__":
    main()
`, p.Prefix, p.Prefix, p.Prefix, p.Prefix, endpoint, p.Prefix), p)
}
