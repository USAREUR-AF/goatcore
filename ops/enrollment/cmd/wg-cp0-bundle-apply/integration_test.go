// integration_test.go — end-to-end ceremony round-trip via real binaries.
//
// Builds both bundle-create and wg-cp0-bundle-apply via `go build`, runs
// them in sequence against a fresh CA + relays fixture, and verifies that
// the operator's actual command-line surface produces a valid wg-quick
// conf + privkey. Catches CLI-level regressions that struct-level Go tests
// can't see — flag parsing drift, CBOR-shape changes, default-value drift,
// allowlist file format drift across the two CLIs.
//
// Skipped when GOROOT's `go` binary isn't on PATH or when Linux-specific
// features the binaries need (curve25519 from x/crypto) aren't available
// — but in practice both prereqs hold anywhere `go test` works.

package main

import (
	"bytes"
	"crypto/ed25519"
	"crypto/rand"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"encoding/pem"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/enrollment/peerset"

	"filippo.io/age"
	"golang.org/x/crypto/curve25519"

	"github.com/usareur-af/goatcore/ops/enrollment/bundle"
	"github.com/usareur-af/goatcore/ops/enrollment/ca"
)

// TestCeremony_EndToEnd is the load-bearing operator-workflow test.
// Builds both CLI binaries with `go build` and runs them in the same
// sequence the operator runs in docs/operations/wg-cp0-bundle-ceremony.md.
// Asserts that:
//   - bundle-create + wg-cp0-bundle-apply share the same on-wire CBOR shape
//   - the rendered wg-cp0.conf has the expected [Interface] + [Peer] structure
//   - the PrivateKey line in the conf matches the privkey file content
//   - --update-allowlist produces a JSON entry the orchestrator can consume
//
// Skipped on non-amd64/arm64 (curve25519 portable but x/crypto/curve25519
// import is the canary for Go module sanity in unusual environments).
func TestCeremony_EndToEnd(t *testing.T) {
	if testing.Short() {
		t.Skip("skipping integration test in short mode")
	}
	if _, err := exec.LookPath("go"); err != nil {
		t.Skip("go not on PATH; can't build binaries for integration test")
	}

	work := t.TempDir()
	binDir := filepath.Join(work, "bin")
	if err := os.Mkdir(binDir, 0o755); err != nil {
		t.Fatalf("mkdir bin: %v", err)
	}

	// Find the enrollment module root so we can `go build` against it.
	// Walk up from the test's working directory (inside cmd/wg-cp0-bundle-apply)
	// until we find go.mod.
	moduleRoot := findModuleRoot(t)

	bundleCreatePath := filepath.Join(binDir, "bundle-create")
	applyPath := filepath.Join(binDir, "wg-cp0-bundle-apply")
	for _, b := range []struct{ cmd, out string }{
		{"./cmd/bundle-create", bundleCreatePath},
		{"./cmd/wg-cp0-bundle-apply", applyPath},
	} {
		cmd := exec.Command("go", "build", "-o", b.out, b.cmd)
		cmd.Dir = moduleRoot
		if out, err := cmd.CombinedOutput(); err != nil {
			t.Fatalf("go build %s: %v\n%s", b.cmd, err, out)
		}
	}

	// Build a dev-mode CA: age identity + age-encrypted key + PEM pubkey.
	identity, err := age.GenerateX25519Identity()
	if err != nil {
		t.Fatalf("age identity: %v", err)
	}
	ageIdentityPath := filepath.Join(work, "keys.txt")
	if err := os.WriteFile(ageIdentityPath, []byte(identity.String()+"\n"), 0o600); err != nil {
		t.Fatalf("write age identity: %v", err)
	}
	caKeyPath := filepath.Join(work, "ca.key.age")
	caPub, err := ca.GenerateAndWrite(caKeyPath, []age.Recipient{identity.Recipient()})
	if err != nil {
		t.Fatalf("ca generate: %v", err)
	}

	caPubPath := filepath.Join(work, "ca.pub")
	caPubSPKI, err := x509.MarshalPKIXPublicKey(caPub)
	if err != nil {
		t.Fatalf("marshal ca pub spki: %v", err)
	}
	if err := os.WriteFile(caPubPath, pem.EncodeToMemory(&pem.Block{Type: "PUBLIC KEY", Bytes: caPubSPKI}), 0o644); err != nil {
		t.Fatalf("write ca pub pem: %v", err)
	}

	// Build a relays-roster fixture matching the prod-shape JSON shape.
	relayPubBytes := make([][]byte, 3)
	for i := range relayPubBytes {
		priv := make([]byte, 32)
		if _, err := rand.Read(priv); err != nil {
			t.Fatalf("relay priv: %v", err)
		}
		priv[0] &= 248
		priv[31] &= 127
		priv[31] |= 64
		pub, err := curve25519.X25519(priv, curve25519.Basepoint)
		if err != nil {
			t.Fatalf("relay pub derive: %v", err)
		}
		relayPubBytes[i] = pub
	}
	type relayEntry struct {
		Name     string `json:"name"`
		Endpoint string `json:"endpoint"`
		Address  string `json:"address"`
		Pubkey   string `json:"pubkey"`
	}
	roster := []relayEntry{
		{Name: "relay-rla", Endpoint: "203.0.113.1:51821", Address: "10.99.0.2", Pubkey: base64.StdEncoding.EncodeToString(relayPubBytes[0])},
		{Name: "relay-rlb", Endpoint: "203.0.113.23:51821", Address: "10.99.0.3", Pubkey: base64.StdEncoding.EncodeToString(relayPubBytes[1])},
		{Name: "relay-rlc", Endpoint: "203.0.113.10:51821", Address: "10.99.0.4", Pubkey: base64.StdEncoding.EncodeToString(relayPubBytes[2])},
	}
	relaysPath := filepath.Join(work, "wg-cp0-relays.json")
	rosterBytes, err := json.MarshalIndent(roster, "", "  ")
	if err != nil {
		t.Fatalf("marshal roster: %v", err)
	}
	if err := os.WriteFile(relaysPath, rosterBytes, 0o600); err != nil {
		t.Fatalf("write roster: %v", err)
	}

	// === Operator step 1: issue bundle ===
	bundlePath := filepath.Join(work, "device.bundle.cbor")
	allowlistPath := filepath.Join(work, "wg-cp0-bundle-allowlist.json")
	createCmd := exec.Command(bundleCreatePath,
		"--site", "lab",
		"--device-id", "mgmt-prov-site3-01-orin",
		"--wg-cp0-address", "10.99.0.6/24",
		"--wg-cp0-relays", relaysPath,
		"--acl-groups", "LAB,LAB-MGMT",
		"--ca-id", "dev-test-ca",
		"--ca-key", caKeyPath,
		"--age-identity", ageIdentityPath,
		"--update-allowlist", allowlistPath,
		"--out", bundlePath,
		"--ttl-days", "90",
		"--activation-days", "14",
	)
	var createOut bytes.Buffer
	createCmd.Stdout = &createOut
	createCmd.Stderr = &createOut
	if err := createCmd.Run(); err != nil {
		t.Fatalf("bundle-create exec: %v\n%s", err, createOut.String())
	}

	// === Operator step 2: apply bundle on the device ===
	confPath := filepath.Join(work, "wg-cp0.conf")
	privkeyPath := filepath.Join(work, "wg-cp0-priv.key")
	applyCmd := exec.Command(applyPath,
		"--in", bundlePath,
		"--ca-pubkey", caPubPath,
		"--out-conf", confPath,
		"--out-privkey", privkeyPath,
		"--interface-name", "wg-cp0",
		"--persistent-keepalive", "25",
	)
	var applyOut bytes.Buffer
	applyCmd.Stdout = &applyOut
	applyCmd.Stderr = &applyOut
	if err := applyCmd.Run(); err != nil {
		t.Fatalf("wg-cp0-bundle-apply exec: %v\n%s", err, applyOut.String())
	}

	// === Verify ceremony output ===

	// Conf shape.
	confBytes, err := os.ReadFile(confPath)
	if err != nil {
		t.Fatalf("read conf: %v", err)
	}
	conf := string(confBytes)
	for _, want := range []string{
		"[Interface]",
		"PrivateKey = ",
		"Address    = 10.99.0.6/24",
		"[Peer]",
		"Endpoint   = 203.0.113.1:51821",
		// ADR 0321: bundle-create now always issues the canonical
		// pair [MeshAddr/32, WGCP0FleetSubnet] on every relay peer
		// block. Operator passed no extras, so the issuer-side
		// AllowedIPs = [<relay>/32, 198.18.0.0/24] flows through to
		// the rendered conf verbatim.
		"AllowedIPs = 10.99.0.2/32, 198.18.0.0/24",
		"Endpoint   = 203.0.113.23:51821",
		"AllowedIPs = 10.99.0.3/32, 198.18.0.0/24",
		"Endpoint   = 203.0.113.10:51821",
		"AllowedIPs = 10.99.0.4/32, 198.18.0.0/24",
		"PersistentKeepalive = 25",
		"# Site:        lab",
		"# Device:      mgmt-prov-site3-01-orin",
	} {
		if !strings.Contains(conf, want) {
			t.Errorf("conf missing %q\n--- conf ---\n%s\n--- bundle-create stderr ---\n%s\n--- apply stderr ---\n%s\n",
				want, conf, createOut.String(), applyOut.String())
		}
	}

	// Privkey file matches the conf's PrivateKey line.
	rawPriv, err := os.ReadFile(privkeyPath)
	if err != nil {
		t.Fatalf("read privkey: %v", err)
	}
	wantPriv := mustExtractPrivateKey(t, conf)
	if got := strings.TrimSpace(string(rawPriv)); got != wantPriv {
		t.Errorf("privkey file != conf PrivateKey\n  conf: %s\n  file: %s", wantPriv, got)
	}

	// File modes: privkey + conf are 0600.
	for _, p := range []string{confPath, privkeyPath} {
		st, err := os.Stat(p)
		if err != nil {
			t.Fatalf("stat %s: %v", p, err)
		}
		if st.Mode().Perm() != 0o600 {
			t.Errorf("%s mode = %o; want 0600", p, st.Mode().Perm())
		}
	}

	// Allowlist JSON was upserted with this device's entry. Block 22R.1
	// rewired bundle-create through ops/enrollment/peerset, so the file
	// is now a v2 envelope; route the test through the accessor library
	// rather than re-parsing the wire-shape directly.
	ps, err := peerset.Load(allowlistPath)
	if err != nil {
		raw, _ := os.ReadFile(allowlistPath)
		t.Fatalf("load allowlist: %v\n%s", err, raw)
	}
	if len(ps.Peers) != 1 {
		raw, _ := os.ReadFile(allowlistPath)
		t.Fatalf("allowlist peer count = %d; want 1\n%s", len(ps.Peers), raw)
	}
	got, ok := ps.Lookup("mgmt-prov-site3-01-orin")
	if !ok {
		t.Errorf("allowlist peer mgmt-prov-site3-01-orin missing")
	}
	if got.WGCP0Address != "10.99.0.6" {
		t.Errorf("allowlist peer address = %q; want 10.99.0.6 (no CIDR)", got.WGCP0Address)
	}

	// Bundle parses + verifies against the same CA pubkey, and the
	// CPDevicePubkey decoded from the bundle matches the pubkey
	// recorded in the allowlist (no field-shuffle drift).
	wire, err := os.ReadFile(bundlePath)
	if err != nil {
		t.Fatalf("read bundle: %v", err)
	}
	parsed, err := bundle.Unmarshal(wire)
	if err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if err := parsed.Verify(caPub); err != nil {
		t.Fatalf("verify: %v", err)
	}
	wantAllowPub := base64.StdEncoding.EncodeToString(parsed.CPDevicePubkey)
	if got.CurrentPubkey != wantAllowPub {
		t.Errorf("allowlist pubkey mismatch with bundle CPDevicePubkey\n  allowlist: %s\n  bundle:    %s", got.CurrentPubkey, wantAllowPub)
	}

	// Sanity: bundle round-trip + sign verify still works after the
	// real-binary path, including the CPDeviceAddress carrying CIDR.
	if parsed.CPDeviceAddress != "10.99.0.6/24" {
		t.Errorf("CPDeviceAddress = %q", parsed.CPDeviceAddress)
	}
	if err := parsed.CheckCPDeviceKeypair(); err != nil {
		t.Errorf("CheckCPDeviceKeypair: %v", err)
	}
}

// TestCeremony_RotationOverwritesAllowlistEntry covers the operator
// re-issuing a bundle for the same device_id (key rotation, expiry
// approaching, role change). The allowlist file should end up with one
// entry, not two, and the entry's pubkey should match the second
// issuance — locking in the upsert-by-name semantic at CLI level.
//
// Block 22R.2 update: the second issuance now requires explicit
// intent via --rotate-keypair --reason "..." (silent re-mint is no
// longer reachable per ADR 0322 Property 4 / F-114 closure). The
// test asserts the upsert-by-name semantic AND that pubkey_history[]
// records the rotation.
func TestCeremony_RotationOverwritesAllowlistEntry(t *testing.T) {
	if testing.Short() {
		t.Skip("skipping integration test in short mode")
	}
	if _, err := exec.LookPath("go"); err != nil {
		t.Skip("go not on PATH")
	}

	work := t.TempDir()
	binDir := filepath.Join(work, "bin")
	_ = os.Mkdir(binDir, 0o755)
	moduleRoot := findModuleRoot(t)
	bundleCreatePath := filepath.Join(binDir, "bundle-create")
	cmd := exec.Command("go", "build", "-o", bundleCreatePath, "./cmd/bundle-create")
	cmd.Dir = moduleRoot
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("build: %v\n%s", err, out)
	}

	identity, _ := age.GenerateX25519Identity()
	ageIdentityPath := filepath.Join(work, "keys.txt")
	_ = os.WriteFile(ageIdentityPath, []byte(identity.String()+"\n"), 0o600)
	caKeyPath := filepath.Join(work, "ca.key.age")
	_, _ = ca.GenerateAndWrite(caKeyPath, []age.Recipient{identity.Recipient()})

	relays := []map[string]string{
		{"name": "r", "endpoint": "1.2.3.4:51821", "address": "10.99.0.2", "pubkey": base64.StdEncoding.EncodeToString(make([]byte, 32))},
	}
	relaysPath := filepath.Join(work, "relays.json")
	rb, _ := json.Marshal(relays)
	_ = os.WriteFile(relaysPath, rb, 0o600)

	allowlistPath := filepath.Join(work, "allowlist.json")

	// Helper: invoke bundle-create with the common flags + optional
	// extras (used to add --rotate-keypair --reason on second pass).
	issue := func(extras ...string) (string, error) {
		out := filepath.Join(work, "bundle.cbor")
		_ = os.Remove(out)
		args := []string{
			"--site", "lab",
			"--device-id", "orin-rotation-target",
			"--wg-cp0-address", "10.99.0.10/24",
			"--wg-cp0-relays", relaysPath,
			"--acl-groups", "G",
			"--ca-id", "dev",
			"--ca-key", caKeyPath,
			"--age-identity", ageIdentityPath,
			"--update-allowlist", allowlistPath,
			"--out", out,
		}
		args = append(args, extras...)
		c := exec.Command(bundleCreatePath, args...)
		var co bytes.Buffer
		c.Stdout = &co
		c.Stderr = &co
		err := c.Run()
		return co.String(), err
	}

	// First issuance — fresh onboard, no intent flag needed.
	if out, err := issue(); err != nil {
		t.Fatalf("issuance 1 (onboard): %v\n%s", err, out)
	}

	// Second issuance MUST declare explicit rotation intent. Without
	// the flag, bundle-create now fail-louds. With
	// --rotate-keypair --reason it re-issues, appends to
	// pubkey_history[], and keeps the peers[] row at count 1.
	if out, err := issue("--rotate-keypair", "--reason", "scheduled rotation"); err != nil {
		t.Fatalf("issuance 2 (rotate): %v\n%s", err, out)
	}

	// Block 22R.1: bundle-create's allowlist output is a v2 envelope;
	// load via the accessor library to assert the upsert-by-name semantic.
	ps, err := peerset.Load(allowlistPath)
	if err != nil {
		raw, _ := os.ReadFile(allowlistPath)
		t.Fatalf("load allowlist: %v\n%s", err, raw)
	}
	if len(ps.Peers) != 1 {
		raw, _ := os.ReadFile(allowlistPath)
		t.Errorf("peer count = %d; want 1 (upsert-by-name)\n%s", len(ps.Peers), raw)
	}
	// Block 22R.2: pubkey_history[] must carry exactly one entry
	// (the pre-rotation pubkey) so the audit trail flows.
	peer, ok := ps.Lookup("orin-rotation-target")
	if !ok {
		t.Fatal("orin-rotation-target missing from peerset")
	}
	if len(peer.PubkeyHistory) != 1 {
		t.Errorf("PubkeyHistory length = %d; want 1 (rotation should record prior pubkey)", len(peer.PubkeyHistory))
	}
	if len(peer.PubkeyHistory) > 0 && peer.PubkeyHistory[0].Reason != "scheduled rotation" {
		t.Errorf("PubkeyHistory[0].Reason = %q; want operator-supplied 'scheduled rotation'", peer.PubkeyHistory[0].Reason)
	}
}

// TestCeremony_BareReissueFailsLoud locks in the F-114 closure: a
// second bundle-create for the same device-id without --reuse-keypair-
// from / --rotate-keypair must fail-loud. The previous silent re-mint
// was the F-114 trigger class; this regression test prevents it from
// silently returning.
func TestCeremony_BareReissueFailsLoud(t *testing.T) {
	if testing.Short() {
		t.Skip("skipping integration test in short mode")
	}
	if _, err := exec.LookPath("go"); err != nil {
		t.Skip("go not on PATH")
	}

	work := t.TempDir()
	moduleRoot := findModuleRoot(t)
	bundleCreatePath := filepath.Join(work, "bundle-create")
	cmd := exec.Command("go", "build", "-o", bundleCreatePath, "./cmd/bundle-create")
	cmd.Dir = moduleRoot
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("build: %v\n%s", err, out)
	}

	identity, _ := age.GenerateX25519Identity()
	ageIdentityPath := filepath.Join(work, "keys.txt")
	_ = os.WriteFile(ageIdentityPath, []byte(identity.String()+"\n"), 0o600)
	caKeyPath := filepath.Join(work, "ca.key.age")
	_, _ = ca.GenerateAndWrite(caKeyPath, []age.Recipient{identity.Recipient()})

	relays := []map[string]string{
		{"name": "r", "endpoint": "1.2.3.4:51821", "address": "10.99.0.2", "pubkey": base64.StdEncoding.EncodeToString(make([]byte, 32))},
	}
	relaysPath := filepath.Join(work, "relays.json")
	rb, _ := json.Marshal(relays)
	_ = os.WriteFile(relaysPath, rb, 0o600)

	allowlistPath := filepath.Join(work, "allowlist.json")

	common := []string{
		"--site", "lab",
		"--device-id", "double-reissue-victim",
		"--wg-cp0-address", "10.99.0.11/24",
		"--wg-cp0-relays", relaysPath,
		"--acl-groups", "G",
		"--ca-id", "dev",
		"--ca-key", caKeyPath,
		"--age-identity", ageIdentityPath,
		"--update-allowlist", allowlistPath,
		"--out", filepath.Join(work, "bundle.cbor"),
	}
	// First issuance — succeeds.
	c := exec.Command(bundleCreatePath, common...)
	if out, err := c.CombinedOutput(); err != nil {
		t.Fatalf("issuance 1: %v\n%s", err, out)
	}

	// Second issuance with no intent flag — MUST fail.
	c2 := exec.Command(bundleCreatePath, common...)
	out, err := c2.CombinedOutput()
	if err == nil {
		t.Fatalf("issuance 2 should have failed-loud; got success:\n%s", out)
	}
	if !bytes.Contains(out, []byte("--reuse-keypair-from")) || !bytes.Contains(out, []byte("--rotate-keypair")) {
		t.Errorf("fail-loud message should point at remediation flags; got:\n%s", out)
	}
}

// findModuleRoot walks up from the current test's working dir until it
// finds a go.mod. The tests run from inside the module so the search
// always terminates within a few hops; falling out of the loop returns
// an absolute fallback path computed from the test source file.
func findModuleRoot(t *testing.T) string {
	t.Helper()
	cwd, err := os.Getwd()
	if err != nil {
		t.Fatalf("getwd: %v", err)
	}
	dir := cwd
	for {
		if _, err := os.Stat(filepath.Join(dir, "go.mod")); err == nil {
			return dir
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			break
		}
		dir = parent
	}
	// Fallback: if we couldn't find go.mod by walking up (unusual), use
	// the runtime source-file location to derive the module root —
	// integration_test.go lives at <module>/cmd/wg-cp0-bundle-apply/.
	_, src, _, ok := runtime.Caller(0)
	if !ok {
		t.Fatalf("can't locate module root")
	}
	return filepath.Dir(filepath.Dir(filepath.Dir(src)))
}

func mustExtractPrivateKey(t *testing.T, conf string) string {
	t.Helper()
	for _, line := range strings.Split(conf, "\n") {
		if strings.HasPrefix(line, "PrivateKey = ") {
			return strings.TrimPrefix(line, "PrivateKey = ")
		}
	}
	t.Fatal("no PrivateKey line in conf")
	return ""
}

// silence unused-import warnings if a future refactor drops one of the
// crypto imports — keeps this file robust against incremental edits.
var _ = ed25519.SignatureSize
