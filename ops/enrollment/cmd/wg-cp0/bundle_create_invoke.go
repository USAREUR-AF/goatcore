package main

// bundle_create_invoke.go — thin wrapper that constructs and executes
// the `bundle-create` CLI for the metadata subcommands owned by
// Session C of Block 22R.2. The wrapper exists so update-endpoints +
// update-ttl share one well-tested integration point against
// bundle-create, and so unit tests can mock the exec path without
// re-implementing flag plumbing.
//
// Integration contract with Session B (per session-C prompt):
// bundle-create accepts `--reuse-keypair-from <peer-name>` to preserve
// the device's existing wg-cp0 Curve25519 keypair across re-mints. This
// wrapper passes that flag unconditionally for additive-metadata
// operations. When Session B's PR lands the flag on `main`, the apply
// path here works end-to-end. Until then, bundle-create fails fast on
// "unknown flag --reuse-keypair-from" — that's the integration catch.
//
// Operator-side environment (sourced via direnv per CLAUDE.md):
//   - GOAT_CA_MODE          — "dev" (default) or "prod"
//   - GOAT_CA_KEY           — path to age-encrypted CA private key (dev)
//   - GOAT_AGE_IDENTITY     — path to age identity file (dev)
//   - GOAT_CA_ID            — CA identifier matching the public key
//                             distribution (e.g. "dev-galaxy-
//                             ca-ecdsa-2026-05-09")
//   - GOAT_TLS_TRUST_ANCHOR — path to ADR 0407 internal-CA root cert
//                             PEM (optional; surfaces as
//                             tls_trust_anchor in the bundle)
//   - WG_CP0_BUNDLE_CREATE_BIN — override binary path (default: looks
//                             for `bundle-create` on PATH)
//
// The wrapper does NOT attempt to discover signing material itself;
// failing to set the required env vars is an operator-side error and
// surfaces with a clear pointer to docs/operations/wg-cp0-ceremony-v2.md.

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"time"

	"github.com/usareur-af/goatcore/ops/enrollment/peerset"
)

// invokeOpts carries everything bundle-create needs for an additive-
// metadata re-mint. The shape is the minimum surface that keeps the
// orchestrator-to-bundle-create boundary clean.
type invokeOpts struct {
	// Target peer that will receive a freshly-signed bundle.
	Peer peerset.Peer
	// ACL groups to embed in the bundle. update-endpoints + update-ttl
	// do not change ACL group membership; the caller passes through
	// whatever the peer's existing groups are. For now, callers infer
	// from the peer's site name as a "default ACL group" until the
	// peerset schema grows an explicit AclGroups field.
	ACLGroups []string
	// Relays that should appear as known endpoints in the new bundle.
	// Derived from peerset.Relays[] with --add-relay / --remove-relay
	// adjustments applied.
	Relays []peerset.Relay
	// TTLDays defaults to 90 (matches bundle-create's own default per
	// ADR 0308) when zero. update-ttl --extend overrides this with the
	// requested horizon.
	TTLDays int
	// OutPath is the operator-supplied target for the new bundle.cbor.
	OutPath string
}

// resolveBundleCreateBin returns the path to the bundle-create binary
// to exec. Operators override via WG_CP0_BUNDLE_CREATE_BIN; the
// default looks for `bundle-create` on PATH. Failing to find the
// binary returns a fatal error with the operator-actionable hint.
func resolveBundleCreateBin() (string, error) {
	if p := os.Getenv("WG_CP0_BUNDLE_CREATE_BIN"); p != "" {
		// #nosec G304,G703 — the env var is an operator-supplied path
		// to the bundle-create binary. Path-traversal is intentional:
		// the operator chooses where their bundle-create lives. The
		// stat is read-only; we just need to confirm the file exists.
		if _, err := os.Stat(p); err != nil {
			return "", fmt.Errorf("WG_CP0_BUNDLE_CREATE_BIN=%s: %w", p, err)
		}
		return p, nil
	}
	p, err := exec.LookPath("bundle-create")
	if err != nil {
		return "", fmt.Errorf("bundle-create not found on PATH; set WG_CP0_BUNDLE_CREATE_BIN or `go install ./cmd/bundle-create`: %w", err)
	}
	return p, nil
}

// caSigningEnv reads the per-operator signing-material env vars and
// returns the flag-pairs to splice into the bundle-create command
// line. Missing required vars are reported as a single combined error
// so the operator fixes everything at once rather than one var per
// re-run.
func caSigningEnv() ([]string, error) {
	mode := os.Getenv("GOAT_CA_MODE")
	if mode == "" {
		mode = "dev"
	}
	caID := os.Getenv("GOAT_CA_ID")
	if caID == "" {
		return nil, fmt.Errorf("GOAT_CA_ID is required (e.g. dev-galaxy-ca-ecdsa-2026-05-09)")
	}
	flags := []string{"--mode", mode, "--ca-id", caID}

	switch mode {
	case "dev":
		caKey := os.Getenv("GOAT_CA_KEY")
		ageID := os.Getenv("GOAT_AGE_IDENTITY")
		if caKey == "" || ageID == "" {
			return nil, fmt.Errorf("dev mode requires GOAT_CA_KEY + GOAT_AGE_IDENTITY (see docs/operations/wg-cp0-ceremony-v2.md)")
		}
		flags = append(flags, "--ca-key", caKey, "--age-identity", ageID)
	case "prod":
		// Prod mode reads PIV slot via bundle-create's own defaults;
		// nothing else to splice here. Operator-side YubiKey PIN
		// handling is bundle-create's responsibility.
	default:
		return nil, fmt.Errorf("GOAT_CA_MODE=%q must be dev or prod", mode)
	}

	if anchor := os.Getenv("GOAT_TLS_TRUST_ANCHOR"); anchor != "" {
		flags = append(flags, "--tls-trust-anchor", anchor)
	}
	return flags, nil
}

// writeRelaysFile materializes peerset.Relay[] into the JSON shape
// bundle-create's --wg-cp0-relays flag expects. The file is created
// in os.TempDir() with a deterministic-per-call name; callers defer
// the cleanup via the returned closer.
func writeRelaysFile(relays []peerset.Relay) (path string, cleanup func(), err error) {
	type entry struct {
		Name     string `json:"name"`
		Endpoint string `json:"endpoint"`
		Address  string `json:"address"`
		Pubkey   string `json:"pubkey"`
	}
	out := make([]entry, 0, len(relays))
	for _, r := range relays {
		// Defensive: skip relays missing the load-bearing wire fields.
		// The peerset-side validator already forbids empty Name, but
		// Endpoint / Pubkey / MeshAddress are optional in the schema
		// (relay entries can be operator-pre-populated before a
		// genuine key-issuance ceremony).
		if r.Endpoint == "" || r.CurrentPubkey == "" || r.MeshAddress == "" {
			return "", nil, fmt.Errorf("relay %q is missing endpoint/pubkey/mesh-address — re-run `wg-cp0 add-relay --apply` first", r.Name)
		}
		// bundle-create expects the relay pubkey as base64; the
		// peerset stores it as the operator-supplied form. If it
		// already looks like base64 (44-char with trailing =) pass as
		// is; otherwise re-encode from raw 32 bytes if applicable.
		pubB64, err := normalizeRelayPubkey(r.CurrentPubkey)
		if err != nil {
			return "", nil, fmt.Errorf("relay %q pubkey: %w", r.Name, err)
		}
		out = append(out, entry{
			Name:     r.Name,
			Endpoint: r.Endpoint,
			Address:  r.MeshAddress,
			Pubkey:   pubB64,
		})
	}
	data, err := json.MarshalIndent(out, "", "  ")
	if err != nil {
		return "", nil, fmt.Errorf("marshal relays: %w", err)
	}
	f, err := os.CreateTemp("", "wg-cp0-relays-*.json")
	if err != nil {
		return "", nil, fmt.Errorf("create temp relays file: %w", err)
	}
	tmpPath := f.Name()
	if _, err := f.Write(data); err != nil {
		_ = f.Close()
		_ = os.Remove(tmpPath)
		return "", nil, fmt.Errorf("write temp relays file: %w", err)
	}
	if err := f.Close(); err != nil {
		_ = os.Remove(tmpPath)
		return "", nil, fmt.Errorf("close temp relays file: %w", err)
	}
	return tmpPath, func() { _ = os.Remove(tmpPath) }, nil
}

// normalizeRelayPubkey accepts either a base64-encoded 32-byte
// Curve25519 pubkey or a raw 32-byte string and returns the canonical
// base64 form bundle-create expects.
func normalizeRelayPubkey(pub string) (string, error) {
	// First attempt: assume it's already base64. Validate that it
	// decodes to exactly 32 bytes; if so, return as-is.
	if raw, err := base64.StdEncoding.DecodeString(pub); err == nil && len(raw) == 32 {
		return pub, nil
	}
	// Second attempt: maybe it's raw 32 bytes interpreted as a string
	// (unlikely for JSON input, but defensive).
	if len(pub) == 32 {
		return base64.StdEncoding.EncodeToString([]byte(pub)), nil
	}
	return "", fmt.Errorf("not a valid base64-encoded 32-byte Curve25519 pubkey")
}

// inferACLGroups returns the ACL group set to embed in a re-minted
// bundle. The peerset schema does not (yet) carry an explicit
// AclGroups field, so we synthesize from the peer's site + role per
// the docs/design/mesh-acl.md taxonomy: every peer carries its site's
// default group, and operator-curated ACL membership lives in netbird
// API state (out-of-scope for this orchestrator pass).
//
// This is a deliberate, scoped trade-off: re-minting a bundle for
// metadata-only updates SHOULD preserve the device's mesh ACL stance.
// Until the peerset schema grows AclGroups, the safest default is the
// site-prefixed default group, which keeps the peer at "mesh member"
// without elevating or stripping any operator-curated bindings (those
// flow through netbird API state, which the bundle does not own).
func inferACLGroups(p peerset.Peer) []string {
	if p.Site == "" {
		// No site → no inferable group. Bundle-create will reject this
		// at the --acl-groups required-flag boundary, which is the
		// correct behavior: the operator must declare ACL membership
		// for any peer whose site is unset.
		return nil
	}
	// Site prefix uppercased to match the docs/design/mesh-acl.md
	// canonical group naming (e.g. SITE-A, SITE-B).
	prefix := upperKebab(p.Site)
	return []string{prefix}
}

// upperKebab uppercases a kebab-cased site identifier so "site-a"
// becomes "SITE-A". Replaces underscores with hyphens defensively
// (site identifiers in inventory occasionally use underscores).
func upperKebab(s string) string {
	out := make([]byte, 0, len(s))
	for i := 0; i < len(s); i++ {
		c := s[i]
		switch {
		case c >= 'a' && c <= 'z':
			out = append(out, c-32)
		case c == '_':
			out = append(out, '-')
		default:
			out = append(out, c)
		}
	}
	return string(out)
}

// buildBundleCreateArgs constructs the full bundle-create command line
// for an additive-metadata re-mint. Returns the argv slice; caller
// constructs the *exec.Cmd. Exposed for unit-testing the flag
// construction without execing.
func buildBundleCreateArgs(opts invokeOpts, relaysFilePath, allowlistPath string, signingFlags []string) ([]string, error) {
	if opts.Peer.Name == "" {
		return nil, fmt.Errorf("peer name is required")
	}
	if opts.Peer.WGCP0Address == "" {
		return nil, fmt.Errorf("peer %q missing wg_cp0_address", opts.Peer.Name)
	}
	if opts.Peer.Site == "" {
		return nil, fmt.Errorf("peer %q has no site; bundle-create --acl-groups would be empty", opts.Peer.Name)
	}
	if opts.OutPath == "" {
		return nil, fmt.Errorf("OutPath is required")
	}
	if len(opts.Relays) == 0 {
		return nil, fmt.Errorf("at least one relay must be present in the relays list")
	}

	ttl := opts.TTLDays
	if ttl <= 0 {
		ttl = 90
	}
	acl := opts.ACLGroups
	if len(acl) == 0 {
		acl = inferACLGroups(opts.Peer)
	}
	if len(acl) == 0 {
		return nil, fmt.Errorf("ACL groups unresolved; pass --acl-groups or set peer.Site")
	}

	args := []string{
		"--site", opts.Peer.Site,
		"--device-id", opts.Peer.Name,
		"--wg-cp0-address", canonicalCIDR(opts.Peer.WGCP0Address),
		"--wg-cp0-relays", relaysFilePath,
		"--acl-groups", joinCSV(acl),
		"--out", opts.OutPath,
		"--ttl-days", strconv.Itoa(ttl),
		// Session B contract — preserve the existing keypair across
		// the re-mint. Required for the no-rotation guarantee of
		// additive-metadata operations.
		"--reuse-keypair-from", opts.Peer.Name,
	}
	args = append(args, signingFlags...)
	if allowlistPath != "" {
		args = append(args, "--update-allowlist", allowlistPath)
	}
	return args, nil
}

// canonicalCIDR ensures bundle-create receives a CIDR-shaped address.
// peerset stores wg_cp0_address as an IP-only string; bundle-create's
// --wg-cp0-address flag rejects bare IPs. The default /24 matches the
// fleet subnet per ADR 0319.
func canonicalCIDR(addr string) string {
	if addr == "" {
		return ""
	}
	for i := 0; i < len(addr); i++ {
		if addr[i] == '/' {
			return addr
		}
	}
	return addr + "/24"
}

// joinCSV produces the comma-separated form bundle-create's
// --acl-groups expects.
func joinCSV(parts []string) string {
	out := ""
	for i, p := range parts {
		if i > 0 {
			out += ","
		}
		out += p
	}
	return out
}

// invokeBundleCreate runs the full bundle-create cycle for one peer.
// Returns the resulting bundle.cbor path on success. On failure, the
// caller decides whether to continue (bulk paths fail-soft per-peer
// and aggregate; single-peer paths fail-loud).
func invokeBundleCreate(ctx context.Context, opts invokeOpts, allowlistPath string, stdout, stderr io.Writer) (string, error) {
	bin, err := resolveBundleCreateBin()
	if err != nil {
		return "", err
	}
	signing, err := caSigningEnv()
	if err != nil {
		return "", err
	}
	relaysFile, cleanup, err := writeRelaysFile(opts.Relays)
	if err != nil {
		return "", err
	}
	defer cleanup()

	args, err := buildBundleCreateArgs(opts, relaysFile, allowlistPath, signing)
	if err != nil {
		return "", err
	}

	// #nosec G204 — execing bundle-create with operator-curated args
	// is the entire point of this helper. The binary path comes from
	// resolveBundleCreateBin (env-var or LookPath); the flag values
	// come from peerset state + signing-material env vars, all
	// operator-controlled.
	cmd := exec.CommandContext(ctx, bin, args...)
	cmd.Stdout = stdout
	cmd.Stderr = stderr
	if err := cmd.Run(); err != nil {
		return "", fmt.Errorf("bundle-create %s: %w", opts.Peer.Name, err)
	}
	return opts.OutPath, nil
}

// computeDefaultOutPath returns a deterministic output filename for a
// peer's freshly-minted bundle. The naming matches the operator
// convention used in docs/operations/wg-cp0-bundle-ceremony.md:
// <site>-<device-id>-<UTC-date>.bundle.cbor.
func computeDefaultOutPath(p peerset.Peer, outDir string, t time.Time) string {
	if outDir == "" {
		outDir = "."
	}
	stamp := t.UTC().Format("2006-01-02")
	site := p.Site
	if site == "" {
		site = "unsited"
	}
	name := fmt.Sprintf("%s-%s-%s.bundle.cbor", site, p.Name, stamp)
	return filepath.Join(outDir, name)
}
