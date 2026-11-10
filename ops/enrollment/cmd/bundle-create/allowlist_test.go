package main

// Block 22R.1 — these tests verify that bundle-create's --update-allowlist
// adapter routes through ops/enrollment/peerset (rather than writing the
// legacy v1 flat array). They preserve the original test intents (append,
// replace-by-name, reject-incomplete, missing-file) against the new
// v2-envelope on-disk shape.

import (
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/enrollment/peerset"
)

func TestUpsertAllowlist_AppendsToEmpty(t *testing.T) {
	dir := t.TempDir()
	p := filepath.Join(dir, "allowlist.json")
	entry := allowlistEntry{Name: "mgmt-prov-site3-01-orin", Pubkey: "AAAA", Address: "198.18.0.6"}
	if err := upsertAllowlist(p, entry, "prov-rla", "mgmt-host", time.Time{}); err != nil {
		t.Fatalf("upsert: %v", err)
	}
	ps, err := peerset.Load(p)
	if err != nil {
		t.Fatalf("load: %v", err)
	}
	if len(ps.Peers) != 1 {
		t.Fatalf("count = %d, want 1", len(ps.Peers))
	}
	got, ok := ps.Lookup(entry.Name)
	if !ok || got.WGCP0Address != entry.Address || got.CurrentPubkey != entry.Pubkey {
		t.Errorf("got %+v; want %+v", got, entry)
	}
	if got.Site != "prov-rla" || got.Role != "mgmt-host" {
		t.Errorf("site/role drift: %+v", got)
	}
}

func TestUpsertAllowlist_AppendsWhenNonEmpty(t *testing.T) {
	dir := t.TempDir()
	p := filepath.Join(dir, "allowlist.json")
	first := allowlistEntry{Name: "orin-a", Pubkey: "AAAA", Address: "198.18.0.1"}
	second := allowlistEntry{Name: "orin-b", Pubkey: "BBBB", Address: "198.18.0.5"}
	if err := upsertAllowlist(p, first, "lab", "", time.Time{}); err != nil {
		t.Fatalf("first: %v", err)
	}
	if err := upsertAllowlist(p, second, "lab", "", time.Time{}); err != nil {
		t.Fatalf("second: %v", err)
	}
	ps, _ := peerset.Load(p)
	if len(ps.Peers) != 2 {
		t.Errorf("count = %d, want 2 (%+v)", len(ps.Peers), ps.Peers)
	}
	if ps.Peers[0].Name != "orin-a" || ps.Peers[1].Name != "orin-b" {
		t.Errorf("order lost: %+v", ps.Peers)
	}
}

func TestUpsertAllowlist_ReplacesByName(t *testing.T) {
	dir := t.TempDir()
	p := filepath.Join(dir, "allowlist.json")
	v1 := allowlistEntry{Name: "orin-a", Pubkey: "OLD=", Address: "198.18.0.1"}
	v2 := allowlistEntry{Name: "orin-a", Pubkey: "NEW=", Address: "198.18.0.1"}
	other := allowlistEntry{Name: "orin-b", Pubkey: "BBBB", Address: "198.18.0.5"}
	for _, e := range []allowlistEntry{v1, other, v2} {
		if err := upsertAllowlist(p, e, "lab", "", time.Time{}); err != nil {
			t.Fatalf("upsert %s: %v", e.Name, err)
		}
	}
	ps, _ := peerset.Load(p)
	if len(ps.Peers) != 2 {
		t.Fatalf("count = %d, want 2 (%+v)", len(ps.Peers), ps.Peers)
	}
	got, _ := ps.Lookup("orin-a")
	if got.CurrentPubkey != "NEW=" {
		t.Errorf("orin-a not replaced: %+v", got)
	}
}

func TestUpsertAllowlist_RejectsIncomplete(t *testing.T) {
	dir := t.TempDir()
	p := filepath.Join(dir, "allowlist.json")
	for _, tc := range []allowlistEntry{
		{Name: "", Pubkey: "AAAA", Address: "198.18.0.1"},
		{Name: "x", Pubkey: "", Address: "198.18.0.1"},
		{Name: "x", Pubkey: "AAAA", Address: ""},
	} {
		if err := upsertAllowlist(p, tc, "lab", "", time.Time{}); err == nil {
			t.Errorf("expected error for %+v", tc)
		}
	}
}

func TestUpsertAllowlist_MissingFileIsTreatedAsEmpty(t *testing.T) {
	dir := t.TempDir()
	p := filepath.Join(dir, "absent.json")
	entry := allowlistEntry{Name: "x", Pubkey: "P", Address: "198.18.0.1"}
	if err := upsertAllowlist(p, entry, "lab", "", time.Time{}); err != nil {
		t.Fatalf("upsert against missing file: %v", err)
	}
	ps, err := peerset.Load(p)
	if err != nil {
		t.Fatal(err)
	}
	if len(ps.Peers) != 1 {
		t.Errorf("want 1 peer after first upsert; got %+v", ps.Peers)
	}
}

func TestUpsertAllowlist_PubkeyCollisionSurfacesAsError(t *testing.T) {
	// Block 22R.1 invariant: the peerset library refuses to land two peers
	// sharing a pubkey. bundle-create's adapter must surface this as a
	// fatal error rather than continue silently.
	dir := t.TempDir()
	p := filepath.Join(dir, "allowlist.json")
	first := allowlistEntry{Name: "orin-a", Pubkey: "SHARED", Address: "198.18.0.1"}
	second := allowlistEntry{Name: "orin-b", Pubkey: "SHARED", Address: "198.18.0.2"}
	if err := upsertAllowlist(p, first, "lab", "", time.Time{}); err != nil {
		t.Fatal(err)
	}
	err := upsertAllowlist(p, second, "lab", "", time.Time{})
	if err == nil {
		t.Fatal("expected collision error")
	}
	if !errors.Is(err, peerset.ErrDuplicatePubkey) {
		t.Errorf("err = %v, want ErrDuplicatePubkey", err)
	}
}

func TestUpsertAllowlist_WritesV2EnvelopeOnDisk(t *testing.T) {
	dir := t.TempDir()
	p := filepath.Join(dir, "allowlist.json")
	entry := allowlistEntry{Name: "orin-a", Pubkey: "Y2", Address: "198.18.0.1"}
	if err := upsertAllowlist(p, entry, "lab", "mgmt-host", time.Time{}); err != nil {
		t.Fatalf("upsert: %v", err)
	}
	// Re-load via peerset and assert v2 schema_version on disk.
	ps, err := peerset.Load(p)
	if err != nil {
		t.Fatalf("load: %v", err)
	}
	if ps.SchemaVersion != peerset.SchemaVersion {
		t.Errorf("schema_version = %d, want %d", ps.SchemaVersion, peerset.SchemaVersion)
	}
}

func TestUpsertAllowlist_AutoMigratesLegacyV1FileOnFirstWrite(t *testing.T) {
	// Operator with a v1 local working copy invokes bundle-create
	// --update-allowlist; the adapter must auto-migrate (read v1, write
	// v2) without losing the prior entries.
	dir := t.TempDir()
	p := filepath.Join(dir, "allowlist.json")
	v1Bytes := []byte(strings.TrimSpace(`
[
  {"name": "preexisting-a", "pubkey": "AAA", "address": "198.18.0.1"},
  {"name": "preexisting-b", "pubkey": "BBB", "address": "198.18.0.2"}
]
`))
	if err := writeBytes(p, v1Bytes); err != nil {
		t.Fatal(err)
	}
	added := allowlistEntry{Name: "newly-added", Pubkey: "CCC", Address: "198.18.0.3"}
	if err := upsertAllowlist(p, added, "lab", "", time.Time{}); err != nil {
		t.Fatalf("upsert: %v", err)
	}
	ps, _ := peerset.Load(p)
	if len(ps.Peers) != 3 {
		t.Errorf("post-migrate count = %d, want 3 (%+v)", len(ps.Peers), ps.Peers)
	}
	for _, want := range []string{"preexisting-a", "preexisting-b", "newly-added"} {
		if _, ok := ps.Lookup(want); !ok {
			t.Errorf("missing peer %q after v1 → v2 migration", want)
		}
	}
}

// writeBytes is a tiny helper to make the migration test readable.
// It writes mode 0o644 because the migrated file ends up at that mode
// after Save; matching the eventual mode avoids cross-mode write churn.
func writeBytes(path string, data []byte) error {
	return os.WriteFile(path, data, 0o644)
}
