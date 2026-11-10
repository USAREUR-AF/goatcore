package main

// cmd_update_ttl_test.go — coverage for `wg-cp0 update-ttl`.

import (
	"encoding/base64"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/enrollment/peerset"
)

// fixtureWithOnePeer writes a v2 peerset carrying one peer + one
// well-formed relay. Used as the seed for update-ttl + update-
// endpoints tests.
func fixtureWithOnePeer(t *testing.T) (path, peerName string) {
	t.Helper()
	pub := base64.StdEncoding.EncodeToString(make([]byte, 32))
	relayPub := base64.StdEncoding.EncodeToString(make([]byte, 32))
	// Pubkey for the peer is distinct from the relay so peerset
	// invariants hold (no two entries share a pubkey).
	peerPub := make([]byte, 32)
	peerPub[31] = 0x42
	peerPubB64 := base64.StdEncoding.EncodeToString(peerPub)
	_ = pub
	prior := time.Now().UTC().Add(30 * 24 * time.Hour)
	ps := peerset.PeerSet{
		Peers: []peerset.Peer{{
			Name:          "lab-a",
			WGCP0Address:  "198.18.0.1",
			CurrentPubkey: peerPubB64,
			Site:          "lab",
			BundleExpiry:  &prior,
		}},
		Relays: []peerset.Relay{{
			Name:          "relay-rla",
			Endpoint:      "relay-rla.netbird-prod.goat.example:51821",
			MeshAddress:   "198.18.0.2",
			CurrentPubkey: relayPub,
			Site:          "fra",
			Tier:          "relay_servers",
		}},
	}
	path = filepath.Join(t.TempDir(), "allow.json")
	writeV2Fixture(t, path, ps)
	return path, "lab-a"
}

func TestCmdUpdateTTL_RequiresExtend(t *testing.T) {
	path, peer := fixtureWithOnePeer(t)
	code, _, stderr := runArgs(t, "update-ttl", peer, "--path", path)
	if code != 2 {
		t.Errorf("code = %d, want 2", code)
	}
	if !strings.Contains(stderr, "--extend") {
		t.Errorf("stderr = %q", stderr)
	}
}

func TestCmdUpdateTTL_RequiresPositionalOrAllPeers(t *testing.T) {
	path, _ := fixtureWithOnePeer(t)
	code, _, stderr := runArgs(t, "update-ttl", "--extend", "90d", "--path", path)
	if code != 1 {
		t.Errorf("code = %d, want 1", code)
	}
	if !strings.Contains(stderr, "--all-peers") && !strings.Contains(stderr, "positional") {
		t.Errorf("stderr = %q", stderr)
	}
}

func TestCmdUpdateTTL_PeerNotFound(t *testing.T) {
	path, _ := fixtureWithOnePeer(t)
	code, _, stderr := runArgs(t, "update-ttl", "ghost", "--extend", "90d", "--path", path)
	if code != 1 {
		t.Errorf("code = %d, want 1", code)
	}
	if !strings.Contains(stderr, "ghost") {
		t.Errorf("stderr = %q", stderr)
	}
}

func TestCmdUpdateTTL_BadDuration(t *testing.T) {
	path, peer := fixtureWithOnePeer(t)
	code, _, stderr := runArgs(t, "update-ttl", peer, "--extend", "potato", "--path", path)
	if code != 2 {
		t.Errorf("code = %d, want 2", code)
	}
	if !strings.Contains(stderr, "potato") {
		t.Errorf("stderr = %q", stderr)
	}
}

func TestCmdUpdateTTL_CapEnforced(t *testing.T) {
	path, peer := fixtureWithOnePeer(t)
	code, _, stderr := runArgs(t, "update-ttl", peer, "--extend", "400d", "--path", path)
	if code != 2 {
		t.Errorf("code = %d, want 2", code)
	}
	if !strings.Contains(stderr, "365") || !strings.Contains(stderr, "--force") {
		t.Errorf("stderr should mention cap + force: %q", stderr)
	}
}

func TestCmdUpdateTTL_DryRunPreview(t *testing.T) {
	path, peer := fixtureWithOnePeer(t)
	code, stdout, _ := runArgs(t, "update-ttl", peer, "--extend", "90d", "--path", path)
	if code != 0 {
		t.Fatalf("code = %d", code)
	}
	for _, w := range []string{
		"PLAN — wg-cp0 update-ttl lab-a --extend 90d",
		"~ UPDATE",
		"peers[].bundle_expiry: lab-a",
		"* REUSING KEYPAIR",
		"REUSING KEYPAIR for 1 peer(s)",
		"Re-run with --apply",
	} {
		if !strings.Contains(stdout, w) {
			t.Errorf("stdout missing %q\nfull:\n%s", w, stdout)
		}
	}
	// State must NOT be mutated by dry-run.
	data, _ := os.ReadFile(path)
	if strings.Contains(string(data), `"last_operation"`) {
		// last_operation only gets stamped on Save; check that the
		// JSON-of-record didn't pick up our changes.
		t.Errorf("dry-run leaked state mutation: %s", data)
	}
}

func TestCmdUpdateTTL_AllPeersConflict(t *testing.T) {
	path, peer := fixtureWithOnePeer(t)
	code, _, stderr := runArgs(t, "update-ttl", peer, "--all-peers", "--extend", "90d", "--path", path)
	if code != 1 {
		t.Errorf("code = %d, want 1", code)
	}
	if !strings.Contains(stderr, "conflicts") {
		t.Errorf("stderr = %q", stderr)
	}
}

func TestCmdUpdateTTL_ApplySingleHappyPath(t *testing.T) {
	path, peer := fixtureWithOnePeer(t)
	withFakeBundleCreateEnv(t)

	outDir := t.TempDir()
	code, stdout, stderr := runArgs(t,
		"update-ttl", peer,
		"--extend", "120d",
		"--path", path,
		"--out-dir", outDir,
		"--apply",
	)
	if code != 0 {
		t.Fatalf("code = %d, stderr=%q stdout=%q", code, stderr, stdout)
	}
	if !strings.Contains(stdout, "Applied: update-ttl lab-a --extend 120d") {
		t.Errorf("applied line missing: %q", stdout)
	}
	// Verify bundle file landed in --out-dir.
	matches, _ := filepath.Glob(filepath.Join(outDir, "*.bundle.cbor"))
	if len(matches) != 1 {
		t.Errorf("expected one bundle file; got %v", matches)
	}
	// Verify peerset's BundleExpiry advanced.
	ps, err := peerset.Load(path)
	if err != nil {
		t.Fatal(err)
	}
	p, _ := ps.Lookup(peer)
	if p.BundleExpiry == nil {
		t.Fatal("BundleExpiry is nil after apply")
	}
	want := time.Now().UTC().Add(120 * 24 * time.Hour)
	if p.BundleExpiry.Before(want.Add(-25*time.Hour)) || p.BundleExpiry.After(want.Add(25*time.Hour)) {
		t.Errorf("BundleExpiry = %s, want within 25h of %s",
			p.BundleExpiry.Format(time.RFC3339), want.Format(time.RFC3339))
	}
}

func TestCmdUpdateTTL_ApplyAllPeers(t *testing.T) {
	path, _ := fixtureWithOnePeer(t)
	withFakeBundleCreateEnv(t)
	outDir := t.TempDir()
	code, stdout, stderr := runArgs(t,
		"update-ttl", "--all-peers",
		"--extend", "90d",
		"--path", path,
		"--out-dir", outDir,
		"--apply",
	)
	if code != 0 {
		t.Fatalf("code = %d, stderr=%q stdout=%q", code, stderr, stdout)
	}
	if !strings.Contains(stdout, "Applied: update-ttl --all-peers --extend 90d (1 peers)") {
		t.Errorf("applied line missing: %q", stdout)
	}
}

func TestCmdUpdateTTL_ForceOverridesCap(t *testing.T) {
	path, peer := fixtureWithOnePeer(t)
	withFakeBundleCreateEnv(t)
	outDir := t.TempDir()
	code, stdout, _ := runArgs(t,
		"update-ttl", peer,
		"--extend", "400d",
		"--path", path,
		"--out-dir", outDir,
		"--force",
		"--apply",
	)
	if code != 0 {
		t.Fatalf("code = %d", code)
	}
	if !strings.Contains(stdout, "WARNING") || !strings.Contains(stdout, "365") {
		t.Errorf("force warning missing: %q", stdout)
	}
}

func TestCmdUpdateTTL_ApplyFailsLoudWhenBinaryMissing(t *testing.T) {
	path, peer := fixtureWithOnePeer(t)
	// Point at a non-existent binary so the apply path errors.
	t.Setenv("WG_CP0_BUNDLE_CREATE_BIN", filepath.Join(t.TempDir(), "does-not-exist"))
	t.Setenv("GOAT_CA_MODE", "dev")
	t.Setenv("GOAT_CA_ID", "test-ca")
	t.Setenv("GOAT_CA_KEY", "/dev/null")
	t.Setenv("GOAT_AGE_IDENTITY", "/dev/null")
	code, _, stderr := runArgs(t,
		"update-ttl", peer,
		"--extend", "90d",
		"--path", path,
		"--apply",
	)
	if code != 1 {
		t.Errorf("code = %d, want 1", code)
	}
	if !strings.Contains(stderr, "WG_CP0_BUNDLE_CREATE_BIN") {
		t.Errorf("stderr should name the env var: %q", stderr)
	}
}

func TestParseExtendDuration_Cases(t *testing.T) {
	good := []struct {
		in   string
		want time.Duration
	}{
		{"1d", 24 * time.Hour},
		{"90d", 90 * 24 * time.Hour},
		{"24h", 24 * time.Hour},
		{"1h30m", 90 * time.Minute},
	}
	for _, c := range good {
		got, err := parseExtendDuration(c.in)
		if err != nil {
			t.Errorf("%q: %v", c.in, err)
		}
		if got != c.want {
			t.Errorf("%q: got %v, want %v", c.in, got, c.want)
		}
	}
}

func TestParseExtendDuration_Bad(t *testing.T) {
	for _, in := range []string{"", "0d", "-5d", "potato", "5x"} {
		if _, err := parseExtendDuration(in); err == nil {
			t.Errorf("%q: expected error", in)
		}
	}
}

func TestFormatExpiry_NilAndZero(t *testing.T) {
	if formatExpiry(nil) != "(unset)" {
		t.Error("nil should render (unset)")
	}
	zero := time.Time{}
	if formatExpiry(&zero) != "(unset)" {
		t.Error("zero should render (unset)")
	}
	tt := time.Date(2026, 8, 12, 0, 0, 0, 0, time.UTC)
	if !strings.HasPrefix(formatExpiry(&tt), "2026-08-12") {
		t.Errorf("formatExpiry rendered %q", formatExpiry(&tt))
	}
}

func TestResolveTTLTargets_AllPeersWithPositionalFails(t *testing.T) {
	ps := &peerset.PeerSet{Peers: []peerset.Peer{{Name: "a", CurrentPubkey: "x", WGCP0Address: "198.18.0.1"}}}
	if _, err := resolveTTLTargets(ps, []string{"a"}, true); err == nil {
		t.Error("expected error for --all-peers with positional")
	}
}

func TestParseFlagsUpdateTTL_UnknownFlag(t *testing.T) {
	accept := map[string]bool{"extend": true}
	if _, _, err := parseFlagsUpdateTTL([]string{"--bogus", "x"}, accept); err == nil {
		t.Error("expected error for unknown flag")
	}
}
