package main

// cmd_update_endpoints_test.go — coverage for the
// `wg-cp0 update-endpoints` subcommand.

import (
	"encoding/base64"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/enrollment/peerset"
)

// fixtureWithRelayMix writes a v2 peerset with one peer + multiple
// relays so update-endpoints can exercise --add-relay /
// --remove-relay filtering.
func fixtureWithRelayMix(t *testing.T) (path, peerName string) {
	t.Helper()
	pub := func(b byte) string {
		raw := make([]byte, 32)
		raw[31] = b
		return base64.StdEncoding.EncodeToString(raw)
	}
	prior := time.Now().UTC().Add(30 * 24 * time.Hour)
	ps := peerset.PeerSet{
		Peers: []peerset.Peer{{
			Name: "lab-a", WGCP0Address: "198.18.0.1",
			CurrentPubkey: pub(0xA1), Site: "lab", BundleExpiry: &prior,
		}},
		Relays: []peerset.Relay{
			{Name: "relay-rla", Endpoint: "relay-rla:51821", MeshAddress: "198.18.0.2", CurrentPubkey: pub(0x02), Tier: "relay_servers"},
			{Name: "relay-rlb", Endpoint: "relay-rlb:51821", MeshAddress: "198.18.0.3", CurrentPubkey: pub(0x03), Tier: "relay_servers"},
			{Name: "relay-rlc", Endpoint: "relay-rlc:51821", MeshAddress: "198.18.0.4", CurrentPubkey: pub(0x04), Tier: "relay_servers"},
		},
	}
	path = filepath.Join(t.TempDir(), "allow.json")
	writeV2Fixture(t, path, ps)
	return path, "lab-a"
}

func TestCmdUpdateEndpoints_DryRunBasic(t *testing.T) {
	path, peer := fixtureWithRelayMix(t)
	code, stdout, _ := runArgs(t, "update-endpoints", peer, "--path", path)
	if code != 0 {
		t.Fatalf("code = %d", code)
	}
	for _, w := range []string{
		"PLAN — wg-cp0 update-endpoints lab-a",
		"* REUSING KEYPAIR",
		"lab-a",
		"relays=[relay-rla, relay-rlb, relay-rlc]",
		"REUSING KEYPAIR for 1 peer(s)",
		"Re-run with --apply",
	} {
		if !strings.Contains(stdout, w) {
			t.Errorf("stdout missing %q\nfull:\n%s", w, stdout)
		}
	}
}

func TestCmdUpdateEndpoints_AddRelayRequiresPeersetEntry(t *testing.T) {
	path, peer := fixtureWithRelayMix(t)
	code, _, stderr := runArgs(t, "update-endpoints", peer, "--add-relay", "relay-rld", "--path", path)
	if code != 1 {
		t.Errorf("code = %d, want 1", code)
	}
	if !strings.Contains(stderr, "relay-rld") || !strings.Contains(stderr, "wg-cp0 add-relay") {
		t.Errorf("stderr should suggest add-relay: %q", stderr)
	}
}

func TestCmdUpdateEndpoints_RemoveRelay_DryRun(t *testing.T) {
	path, peer := fixtureWithRelayMix(t)
	code, stdout, _ := runArgs(t, "update-endpoints", peer, "--remove-relay", "relay-rlc", "--path", path)
	if code != 0 {
		t.Fatalf("code = %d", code)
	}
	if !strings.Contains(stdout, "relays=[relay-rla, relay-rlb]") {
		t.Errorf("stdout = %q", stdout)
	}
	if !strings.Contains(stdout, "Removing relay(s) from bundles: relay-rlc") {
		t.Errorf("removal note missing: %q", stdout)
	}
}

func TestCmdUpdateEndpoints_RemoveAllRefuses(t *testing.T) {
	path, peer := fixtureWithRelayMix(t)
	code, _, stderr := runArgs(t, "update-endpoints", peer,
		"--remove-relay", "relay-rla",
		"--remove-relay", "relay-rlb",
		"--remove-relay", "relay-rlc",
		"--path", path)
	if code != 1 {
		t.Errorf("code = %d, want 1", code)
	}
	if !strings.Contains(stderr, "endpoint-less") {
		t.Errorf("stderr = %q", stderr)
	}
}

func TestCmdUpdateEndpoints_SetActiveForwarder_DryRun(t *testing.T) {
	path, peer := fixtureWithRelayMix(t)
	code, stdout, _ := runArgs(t, "update-endpoints", peer,
		"--set-active-forwarder", "relay-rla",
		"--path", path)
	if code != 0 {
		t.Fatalf("code = %d", code)
	}
	if !strings.Contains(stdout, "active_forwarder") || !strings.Contains(stdout, "host → relay-rla") {
		t.Errorf("active-forwarder line missing: %q", stdout)
	}
}

func TestCmdUpdateEndpoints_SetActiveForwarder_NotInRelaySet(t *testing.T) {
	path, peer := fixtureWithRelayMix(t)
	code, _, stderr := runArgs(t, "update-endpoints", peer,
		"--remove-relay", "relay-rla",
		"--set-active-forwarder", "relay-rla",
		"--path", path)
	if code != 1 {
		t.Errorf("code = %d, want 1", code)
	}
	if !strings.Contains(stderr, "not in the post-filter") {
		t.Errorf("stderr = %q", stderr)
	}
}

func TestCmdUpdateEndpoints_AllPeersConflictWithPositional(t *testing.T) {
	path, peer := fixtureWithRelayMix(t)
	code, _, stderr := runArgs(t, "update-endpoints", peer, "--all-peers", "--path", path)
	if code != 1 {
		t.Errorf("code = %d, want 1", code)
	}
	if !strings.Contains(stderr, "conflicts") {
		t.Errorf("stderr = %q", stderr)
	}
}

func TestCmdUpdateEndpoints_PeerNotFound(t *testing.T) {
	path, _ := fixtureWithRelayMix(t)
	code, _, stderr := runArgs(t, "update-endpoints", "ghost", "--path", path)
	if code != 1 {
		t.Errorf("code = %d", code)
	}
	if !strings.Contains(stderr, "ghost") {
		t.Errorf("stderr = %q", stderr)
	}
}

func TestCmdUpdateEndpoints_ApplySinglePeer(t *testing.T) {
	path, peer := fixtureWithRelayMix(t)
	withFakeBundleCreateEnv(t)
	outDir := t.TempDir()
	code, stdout, stderr := runArgs(t,
		"update-endpoints", peer,
		"--path", path,
		"--out-dir", outDir,
		"--apply",
	)
	if code != 0 {
		t.Fatalf("code = %d, stderr=%q stdout=%q", code, stderr, stdout)
	}
	if !strings.Contains(stdout, "Applied: update-endpoints (1 peer, 3 relays)") {
		t.Errorf("applied line missing: %q", stdout)
	}
	matches, _ := filepath.Glob(filepath.Join(outDir, "*.bundle.cbor"))
	if len(matches) != 1 {
		t.Errorf("expected one bundle file, got %v", matches)
	}
}

func TestCmdUpdateEndpoints_ApplyAllPeersAddNewRelay(t *testing.T) {
	path, _ := fixtureWithRelayMix(t)
	// Add a new relay (relay-rld) directly to the peerset so
	// --add-relay can reference it.
	ps, err := peerset.Load(path)
	if err != nil {
		t.Fatal(err)
	}
	pub := make([]byte, 32)
	pub[31] = 0xFE
	ps.Relays = append(ps.Relays, peerset.Relay{
		Name: "relay-rld", Endpoint: "relay-rld:51821", MeshAddress: "198.18.0.50",
		CurrentPubkey: base64.StdEncoding.EncodeToString(pub),
		Tier:          "relay_servers",
	})
	if err := ps.Save(path); err != nil {
		t.Fatal(err)
	}

	withFakeBundleCreateEnv(t)
	logPath := filepath.Join(t.TempDir(), "invocations.log")
	t.Setenv("GOAT_FAKE_BUNDLE_CREATE_LOG", logPath)

	outDir := t.TempDir()
	code, stdout, stderr := runArgs(t,
		"update-endpoints", "--all-peers",
		"--add-relay", "relay-rld",
		"--path", path,
		"--out-dir", outDir,
		"--apply",
	)
	if code != 0 {
		t.Fatalf("code = %d, stderr=%q stdout=%q", code, stderr, stdout)
	}
	if !strings.Contains(stdout, "Applied: update-endpoints (1 peer, 4 relays)") {
		t.Errorf("applied line missing: %q", stdout)
	}
	// Verify the invocation log captured the --add-relay's effect.
	logData, _ := os.ReadFile(logPath)
	if !strings.Contains(string(logData), "--reuse-keypair-from lab-a") {
		t.Errorf("invocation log missing reuse-keypair-from: %s", logData)
	}
}

func TestCmdUpdateEndpoints_ApplySetActiveForwarder(t *testing.T) {
	path, peer := fixtureWithRelayMix(t)
	withFakeBundleCreateEnv(t)
	outDir := t.TempDir()
	code, _, stderr := runArgs(t,
		"update-endpoints", peer,
		"--set-active-forwarder", "relay-rlb",
		"--path", path,
		"--out-dir", outDir,
		"--apply",
	)
	if code != 0 {
		t.Fatalf("code = %d, stderr = %q", code, stderr)
	}
	ps, _ := peerset.Load(path)
	if ps.ActiveForwarder == nil || ps.ActiveForwarder.Host != "relay-rlb" {
		t.Errorf("ActiveForwarder = %+v", ps.ActiveForwarder)
	}
	if ps.ActiveForwarder.Tier != "relay_servers" {
		t.Errorf("Tier = %q, want relay_servers", ps.ActiveForwarder.Tier)
	}
}

func TestCmdUpdateEndpoints_UnknownFlag(t *testing.T) {
	path, peer := fixtureWithRelayMix(t)
	code, _, stderr := runArgs(t, "update-endpoints", peer, "--bogus", "x", "--path", path)
	if code != 2 {
		t.Errorf("code = %d, want 2", code)
	}
	if !strings.Contains(stderr, "unknown flag") {
		t.Errorf("stderr = %q", stderr)
	}
}

func TestFilterRelays_AddRelayEnsurePresent(t *testing.T) {
	ps := &peerset.PeerSet{
		Relays: []peerset.Relay{
			{Name: "a"}, {Name: "b"}, {Name: "c"},
		},
	}
	out, err := filterRelays(ps, []string{"a"}, nil) // already present
	if err != nil {
		t.Fatal(err)
	}
	if len(out) != 3 {
		t.Errorf("expected 3 (no-op add); got %d", len(out))
	}
}

func TestFilterRelays_AddRelayPlusRemove(t *testing.T) {
	ps := &peerset.PeerSet{Relays: []peerset.Relay{
		{Name: "a"}, {Name: "b"}, {Name: "c"},
	}}
	// Remove b, add a (a is already present). Expected: [a, c].
	out, err := filterRelays(ps, []string{"a"}, []string{"b"})
	if err != nil {
		t.Fatal(err)
	}
	names := []string{}
	for _, r := range out {
		names = append(names, r.Name)
	}
	if strings.Join(names, ",") != "a,c" {
		t.Errorf("got %v", names)
	}
}

func TestFilterRelays_AddRelayConflictWithRemoveResolvesToAdd(t *testing.T) {
	// If --add-relay names a relay also in --remove-relay, the add
	// semantic wins (post-remove walk then ensure-present append).
	ps := &peerset.PeerSet{Relays: []peerset.Relay{
		{Name: "a"}, {Name: "b"},
	}}
	out, err := filterRelays(ps, []string{"b"}, []string{"b"})
	if err != nil {
		t.Fatal(err)
	}
	names := []string{}
	for _, r := range out {
		names = append(names, r.Name)
	}
	// b removed in pass 1, then re-added in pass 2.
	if strings.Join(names, ",") != "a,b" {
		t.Errorf("got %v", names)
	}
}

func TestFilterRelays_AddRelayUnknown(t *testing.T) {
	ps := &peerset.PeerSet{Relays: []peerset.Relay{{Name: "a"}}}
	if _, err := filterRelays(ps, []string{"ghost"}, nil); err == nil {
		t.Errorf("expected error for unknown --add-relay")
	}
}

func TestRelayByName(t *testing.T) {
	relays := []peerset.Relay{{Name: "a"}, {Name: "b"}}
	if _, ok := relayByName(relays, "b"); !ok {
		t.Error("missing positive lookup")
	}
	if _, ok := relayByName(relays, "ghost"); ok {
		t.Error("ghost should not be found")
	}
}

func TestPluralS(t *testing.T) {
	if pluralS(1) != "" || pluralS(0) != "s" || pluralS(2) != "s" {
		t.Errorf("plural cases failed: 0=%q 1=%q 2=%q", pluralS(0), pluralS(1), pluralS(2))
	}
}

func TestResolveEndpointTargets_AllPeers(t *testing.T) {
	ps := &peerset.PeerSet{Peers: []peerset.Peer{
		{Name: "a", CurrentPubkey: "p1", WGCP0Address: "198.18.0.1"},
		{Name: "b", CurrentPubkey: "p2", WGCP0Address: "198.18.0.2"},
	}}
	got, err := resolveEndpointTargets(ps, nil, true)
	if err != nil || len(got) != 2 {
		t.Errorf("got %d, err=%v", len(got), err)
	}
}

func TestResolveEndpointTargets_AllPeersWithPositionalConflicts(t *testing.T) {
	ps := &peerset.PeerSet{Peers: []peerset.Peer{{Name: "a", CurrentPubkey: "p", WGCP0Address: "198.18.0.1"}}}
	if _, err := resolveEndpointTargets(ps, []string{"a"}, true); err == nil {
		t.Error("expected conflict error")
	}
}

func TestParseFlagsUpdateEndpoints_RepeatedAddRelay(t *testing.T) {
	accept := map[string]bool{"add-relay": true, "remove-relay": true}
	f, _, err := parseFlagsUpdateEndpoints(
		[]string{"--add-relay", "x", "--add-relay", "y", "--remove-relay", "z"},
		accept,
	)
	if err != nil {
		t.Fatal(err)
	}
	if len(f.addRelays) != 2 || f.addRelays[0] != "x" || f.addRelays[1] != "y" {
		t.Errorf("addRelays = %v", f.addRelays)
	}
	if len(f.removeRelays) != 1 || f.removeRelays[0] != "z" {
		t.Errorf("removeRelays = %v", f.removeRelays)
	}
}

func TestParseFlagsUpdateEndpoints_UnknownFlag(t *testing.T) {
	accept := map[string]bool{"add-relay": true}
	if _, _, err := parseFlagsUpdateEndpoints([]string{"--bogus", "x"}, accept); err == nil {
		t.Error("expected error")
	}
}
