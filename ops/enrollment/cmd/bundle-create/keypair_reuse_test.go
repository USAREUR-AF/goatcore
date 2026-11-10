package main

import (
	"bytes"
	"encoding/base64"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/enrollment/bundle"
	"github.com/usareur-af/goatcore/ops/enrollment/peerset"
)

// seedPeerset writes a v2 envelope with one active peer so
// resolveKeypairIntent has something to find. Returns the path + the
// peer's stored pubkey bytes for later equality checks.
func seedPeerset(t *testing.T, deviceID, wgAddress string) (string, []byte) {
	t.Helper()
	dir := t.TempDir()
	path := filepath.Join(dir, "peerset.json")

	storedPubBytes := make([]byte, wgPubkeySize)
	for i := range storedPubBytes {
		storedPubBytes[i] = byte(i + 1) // not random — deterministic eq check
	}
	storedPubB64 := base64.StdEncoding.EncodeToString(storedPubBytes)

	ps, err := peerset.Load(path) // empty file → empty v2 envelope
	if err != nil {
		t.Fatalf("seed: load empty: %v", err)
	}
	peer := peerset.Peer{
		Name:          deviceID,
		WGCP0Address:  wgAddress,
		CurrentPubkey: storedPubB64,
		Role:          "mgmt-host",
		Site:          "lab",
	}
	if err := ps.Upsert(peer); err != nil {
		t.Fatalf("seed: upsert: %v", err)
	}
	if err := ps.Save(path); err != nil {
		t.Fatalf("seed: save: %v", err)
	}
	return path, storedPubBytes
}

// baseArgs returns a runArgs prefilled with the minimum valid fields a
// real bundle-create run needs. Tests override individual fields.
func baseArgs(t *testing.T, deviceID string) runArgs {
	t.Helper()
	caKey, ageID, _ := devCA(t)
	relaysPath := fixtureRelays(t)
	return runArgs{
		Site:            "lab",
		DeviceID:        deviceID,
		WGCP0Address:    "10.99.0.6/24",
		RelaysPath:      relaysPath,
		ACLGroupsRaw:    "LAB",
		CAID:            "dev-test",
		CAKeyPath:       caKey,
		AgeIdentityPath: ageID,
		OutPath:         filepath.Join(t.TempDir(), "bundle.cbor"),
		TTLDays:         90,
		ActivationDays:  14,
		Now:             fixedNow(),
	}
}

// TestRun_ReuseKeypairFrom_PreservesPubkey is the load-bearing
// acceptance test from the session-B prompt:
//
//	bundle-create --reuse-keypair-from produces a bundle with the exact
//	same pubkey as the prior bundle for that peer.
func TestRun_ReuseKeypairFrom_PreservesPubkey(t *testing.T) {
	const deviceID = "lab-a"
	peersetPath, storedPub := seedPeerset(t, deviceID, "10.99.0.6")

	args := baseArgs(t, deviceID)
	args.UpdateAllowlistPath = peersetPath
	args.ReuseKeypairFrom = deviceID

	if err := run(args); err != nil {
		t.Fatalf("run: %v", err)
	}

	wire, err := os.ReadFile(args.OutPath)
	if err != nil {
		t.Fatalf("read bundle: %v", err)
	}
	b, err := bundle.Unmarshal(wire)
	if err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if !bytes.Equal(b.CPDevicePubkey, storedPub) {
		t.Errorf("CPDevicePubkey = %x; want stored %x", b.CPDevicePubkey, storedPub)
	}
	// Privkey is intentionally omitted in the reuse path so the
	// device-side handler preserves its existing privkey.
	if len(b.CPDevicePrivkey) != 0 {
		t.Errorf("CPDevicePrivkey should be omitted in reuse path; got %d bytes", len(b.CPDevicePrivkey))
	}
	// Peerset is NOT mutated (no rotation, no history append).
	ps, err := peerset.Load(peersetPath)
	if err != nil {
		t.Fatalf("reload peerset: %v", err)
	}
	peer, ok := ps.Lookup(deviceID)
	if !ok {
		t.Fatal("peer disappeared from peerset")
	}
	if peer.CurrentPubkey != base64.StdEncoding.EncodeToString(storedPub) {
		t.Errorf("peer.CurrentPubkey changed after reuse: got %q", peer.CurrentPubkey)
	}
	if len(peer.PubkeyHistory) != 0 {
		t.Errorf("PubkeyHistory should be empty after reuse; got %d entries", len(peer.PubkeyHistory))
	}
}

// TestRun_RotateKeypair_RequiresReason fails-loud when the operator
// passes --rotate-keypair without --reason. Rotations must carry an
// audit string.
func TestRun_RotateKeypair_RequiresReason(t *testing.T) {
	const deviceID = "lab-a"
	peersetPath, _ := seedPeerset(t, deviceID, "10.99.0.6")

	args := baseArgs(t, deviceID)
	args.UpdateAllowlistPath = peersetPath
	args.RotateKeypair = true
	// RotateReason intentionally empty.

	err := run(args)
	if err == nil {
		t.Fatal("expected error for missing --reason")
	}
	if !errors.Is(err, ErrRotateKeypairMissingReason) {
		t.Errorf("expected ErrRotateKeypairMissingReason; got %v", err)
	}
}

// TestRun_RotateKeypair_AppendsToHistory mints a fresh keypair, records
// the rotation in pubkey_history[], and updates current_pubkey to the
// new value.
func TestRun_RotateKeypair_AppendsToHistory(t *testing.T) {
	const deviceID = "lab-a"
	peersetPath, storedPub := seedPeerset(t, deviceID, "10.99.0.6")
	storedPubB64 := base64.StdEncoding.EncodeToString(storedPub)

	args := baseArgs(t, deviceID)
	args.UpdateAllowlistPath = peersetPath
	args.RotateKeypair = true
	args.RotateReason = "scheduled 90d rotation"

	if err := run(args); err != nil {
		t.Fatalf("run: %v", err)
	}

	wire, _ := os.ReadFile(args.OutPath)
	b, err := bundle.Unmarshal(wire)
	if err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if bytes.Equal(b.CPDevicePubkey, storedPub) {
		t.Error("rotation should produce a NEW pubkey; got the stored one")
	}
	if len(b.CPDevicePrivkey) != wgPubkeySize {
		t.Errorf("rotation must include privkey; got %d bytes", len(b.CPDevicePrivkey))
	}

	ps, err := peerset.Load(peersetPath)
	if err != nil {
		t.Fatalf("reload peerset: %v", err)
	}
	peer, ok := ps.Lookup(deviceID)
	if !ok {
		t.Fatal("peer disappeared from peerset")
	}
	// New current matches the bundle's freshly-minted pubkey.
	if peer.CurrentPubkey != base64.StdEncoding.EncodeToString(b.CPDevicePubkey) {
		t.Errorf("CurrentPubkey after rotate doesn't match bundle pubkey")
	}
	if len(peer.PubkeyHistory) != 1 {
		t.Fatalf("PubkeyHistory length = %d; want 1", len(peer.PubkeyHistory))
	}
	h := peer.PubkeyHistory[0]
	if h.Pubkey != storedPubB64 {
		t.Errorf("history pubkey = %q; want previous %q", h.Pubkey, storedPubB64)
	}
	if h.Reason != "scheduled 90d rotation" {
		t.Errorf("history reason = %q; want operator-supplied", h.Reason)
	}
	if h.RotatedAt.After(time.Now().UTC().Add(time.Minute)) {
		t.Errorf("history RotatedAt in the future: %v", h.RotatedAt)
	}
}

// TestRun_BareCallForExistingPeer_FailsLoud closes the F-114 trigger
// class: bare bundle-create against an existing peer without an explicit
// declared intent (reuse/rotate) is no longer silently re-minted.
func TestRun_BareCallForExistingPeer_FailsLoud(t *testing.T) {
	const deviceID = "lab-a"
	peersetPath, _ := seedPeerset(t, deviceID, "10.99.0.6")

	args := baseArgs(t, deviceID)
	args.UpdateAllowlistPath = peersetPath
	// Neither --reuse-keypair-from nor --rotate-keypair set.

	err := run(args)
	if err == nil {
		t.Fatal("expected fail-loud against existing peer")
	}
	if !errors.Is(err, ErrBareCreateForExistingPeer) {
		t.Errorf("expected ErrBareCreateForExistingPeer; got %v", err)
	}
	// Error message points at the two flags so an operator can fix.
	msg := err.Error()
	if !strings.Contains(msg, "--reuse-keypair-from") || !strings.Contains(msg, "--rotate-keypair") {
		t.Errorf("error doesn't point at remediation flags: %q", msg)
	}
}

// TestRun_BareCallForNewPeer_StillWorks confirms the new fail-loud rule
// is scoped to existing peers only. A device-id absent from peerset is
// the new-onboard path and stays bare-callable.
func TestRun_BareCallForNewPeer_StillWorks(t *testing.T) {
	peersetPath, _ := seedPeerset(t, "some-other-peer", "10.99.0.99")

	args := baseArgs(t, "brand-new-orin")
	args.UpdateAllowlistPath = peersetPath

	if err := run(args); err != nil {
		t.Errorf("new-peer onboard should not fail-loud: %v", err)
	}
}

// TestRun_MutuallyExclusiveFlags fails-loud when both --reuse-keypair-from
// and --rotate-keypair are passed in the same invocation.
func TestRun_MutuallyExclusiveFlags(t *testing.T) {
	const deviceID = "lab-a"
	peersetPath, _ := seedPeerset(t, deviceID, "10.99.0.6")

	args := baseArgs(t, deviceID)
	args.UpdateAllowlistPath = peersetPath
	args.ReuseKeypairFrom = deviceID
	args.RotateKeypair = true
	args.RotateReason = "shouldn't matter"

	err := run(args)
	if err == nil {
		t.Fatal("expected mutual-exclusion error")
	}
	if !errors.Is(err, ErrMutuallyExclusiveKeypairFlags) {
		t.Errorf("expected ErrMutuallyExclusiveKeypairFlags; got %v", err)
	}
}

// TestRun_ReuseKeypair_RequiresPeerset fails-loud when --reuse-keypair-from
// is passed without --update-allowlist (the read path).
func TestRun_ReuseKeypair_RequiresPeerset(t *testing.T) {
	args := baseArgs(t, "lab-a")
	args.ReuseKeypairFrom = "lab-a"
	// UpdateAllowlistPath intentionally empty.

	err := run(args)
	if err == nil {
		t.Fatal("expected error when --update-allowlist not set")
	}
	if !errors.Is(err, ErrReuseKeypairWithoutPeerset) {
		t.Errorf("expected ErrReuseKeypairWithoutPeerset; got %v", err)
	}
}

// TestRun_ReuseKeypair_UnknownPeer fails-loud when --reuse-keypair-from
// names a peer not in the peerset.
func TestRun_ReuseKeypair_UnknownPeer(t *testing.T) {
	peersetPath, _ := seedPeerset(t, "existing-peer", "10.99.0.6")

	args := baseArgs(t, "brand-new-onboard")
	args.UpdateAllowlistPath = peersetPath
	args.ReuseKeypairFrom = "nonexistent-peer"

	err := run(args)
	if err == nil {
		t.Fatal("expected error when reuse peer not found")
	}
	if !errors.Is(err, ErrReuseKeypairFromUnknownPeer) {
		t.Errorf("expected ErrReuseKeypairFromUnknownPeer; got %v", err)
	}
}

// TestRun_RotateKeypair_NewPeerRejected — rotation only makes sense for
// an existing peer. Operator typo (meant onboard) is caught.
func TestRun_RotateKeypair_NewPeerRejected(t *testing.T) {
	peersetPath, _ := seedPeerset(t, "some-other-peer", "10.99.0.6")

	args := baseArgs(t, "brand-new-orin")
	args.UpdateAllowlistPath = peersetPath
	args.RotateKeypair = true
	args.RotateReason = "doesn't matter"

	err := run(args)
	if err == nil {
		t.Fatal("expected error when --rotate-keypair on new peer")
	}
	if !strings.Contains(err.Error(), "not in peerset") {
		t.Errorf("error %q doesn't mention 'not in peerset'", err)
	}
}

// TestResolveKeypairIntent_NoPeersetPath_FallsThrough confirms the
// legacy code path is preserved when --update-allowlist isn't set —
// the new intent surface is opt-in, today's ad-hoc dev callers don't
// regress.
func TestResolveKeypairIntent_NoPeersetPath_FallsThrough(t *testing.T) {
	args := runArgs{DeviceID: "anything", Now: fixedNow()}
	intent, err := resolveKeypairIntent(args)
	if err != nil {
		t.Fatalf("resolveKeypairIntent: %v", err)
	}
	if intent.Kind != "new" {
		t.Errorf("Kind = %q; want new", intent.Kind)
	}
	if intent.PeerExists {
		t.Error("PeerExists should be false when no peerset path")
	}
}
